package pt.qt.app

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout
import okhttp3.Dispatcher
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import uniffi.qt_ffi.NoiseSession
import uniffi.qt_ffi.QtClient
import uniffi.qt_ffi.RelayResponse
import uniffi.qt_ffi.relayParseResponse
import android.util.Log
import java.io.IOException
import java.util.concurrent.TimeUnit

/** Erro que NAO deve ser repetido (possivel MITM / autenticacao recusada). */
class PermanentRelayException(msg: String) : Exception(msg)

/**
 * WebSocket (OkHttp) + Noise (NoiseSession, no Rust). O Kotlin so move bytes: a chave estatica
 * Noise, a assinatura de autenticacao e o codec das mensagens vivem no Rust.
 *
 * Segue a maquina de estados de docs/android-integration.md §4:
 *   ligar -> handshake Noise -> (pinning) -> autenticar -> pedidos sequenciais.
 * Qualquer erro: fecha, DESCARTA a NoiseSession e re-liga com backoff (nova sessao sempre).
 */
class RelayLink(
    private val qt: QtClient,
    private val url: String,
    /** Chave publica estatica Noise do relay (hex), impressa no arranque do `relay_server`. Opcional. */
    private val pinnedRelayKeyHex: String?,
) {
    private val lock = Mutex()
    private var conn: Conn? = null
    /** Terminada de vez ([shutdown]): nao volta a ligar, os pedidos falham logo. */
    @Volatile private var shut = false

    private class Conn(val ws: WebSocket, val noise: NoiseSession, val inbox: Channel<ByteArray>)

    /**
     * Envia um pedido (ja construido com qt.relayRequest*) e devolve a resposta, com reconnect.
     * `timeoutMs`: tempo maximo pela resposta (maior para `relayRequestWait`, que so responde
     * quando chega uma mensagem).
     */
    suspend fun request(bytes: ByteArray, timeoutMs: Long = REQUEST_MS): RelayResponse =
        lock.withLock { requestLocked(bytes, timeoutMs) }

    private suspend fun requestLocked(bytes: ByteArray, timeoutMs: Long): RelayResponse {
        var last: Exception? = null
        for (attempt in 0..MAX_RETRIES) {
            if (attempt > 0) delay((BACKOFF_MS shl (attempt - 1)).coerceAtMost(8_000L))
            if (shut) throw IOException(SHUT_MSG)
            try {
                val c = conn ?: connect().also { conn = it; closeIfShut() }
                return roundtrip(c, bytes, timeoutMs)
            } catch (e: PermanentRelayException) {
                close(); throw e
            } catch (e: Exception) {
                close(); last = e          // timeout, queda de rede, frame invalido...
                Log.w(TAG, "pedido falhou (tentativa ${attempt + 1}): ${e.javaClass.simpleName}: ${e.message?.take(120)}")
            }
        }
        throw IOException("relay indisponivel: ${last?.message}", last)
    }

    /**
     * Varios pedidos seguidos e so depois as respostas, pela mesma ordem: para os
     * blocos dos anexos, sem esperar uma volta inteira a rede por cada bloco.
     * Sem repeticao automatica: se falhar, quem chama retoma (o relay diz onde ficou).
     */
    suspend fun requestMany(requests: List<ByteArray>): List<RelayResponse> = lock.withLock {
        if (shut) throw IOException(SHUT_MSG)
        val c = try {
            conn ?: connect().also { conn = it; closeIfShut() }
        } catch (e: Exception) {
            close(); throw e
        }
        try {
            for (r in requests) send(c.ws, c.noise.encrypt(r))
            requests.map {
                val frame = withTimeout(REQUEST_MS) { c.inbox.receive() }
                relayParseResponse(c.noise.decrypt(frame))
            }
        } catch (e: Exception) {
            close(); throw e
        }
    }

    fun close() {
        conn?.let {
            runCatching { it.ws.close(1000, null) }
            it.inbox.close()
            // Liberta ja a sessao Noise no Rust (e as chaves desta ligacao), sem esperar pelo GC.
            // Se outro pedido ainda a estiver a usar, o UniFFI so a apaga quando esse acabar.
            runCatching { it.noise.close() }
        }
        conn = null
    }

    /**
     * Fecha de vez. Ao contrario de [close], um pedido que ja estivesse a meio (ou que chegue
     * depois) NAO volta a ligar: falha logo. Sem isto, esse pedido abria uma ligacao nova que
     * ninguem fechava e que ficava viva em segundo plano (com ping de 25 em 25 s).
     */
    fun shutdown() {
        shut = true
        close()
    }

    /** Uma ligacao que acabou de abrir depois de [shutdown]: fecha-a ja. */
    private fun closeIfShut() {
        if (shut) {
            close()
            throw IOException(SHUT_MSG)
        }
    }

    private suspend fun connect(): Conn {
        // Defesa em profundidade alem do network_security_config: release so aceita wss://.
        val scheme = url.trim().lowercase()
        if (!scheme.startsWith("wss://") && !(BuildConfig.ALLOW_CLEARTEXT && scheme.startsWith("ws://"))) {
            throw PermanentRelayException("URL do relay recusado: usa wss:// (ws:// so na build debug)")
        }
        val started = android.os.SystemClock.elapsedRealtime()
        val noise = qt.relayNoiseSession()
        val inbox = Channel<ByteArray>(Channel.UNLIMITED)
        val listener = object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) { inbox.trySend(bytes.toByteArray()) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                Log.w(TAG, "ligacao fechada pelo relay ($code)")
                inbox.close(IOException("ligacao fechada ($code)")); webSocket.close(1000, null)
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                Log.w(TAG, "ligacao caiu: ${t.javaClass.simpleName}: ${t.message?.take(120)}")
                inbox.close(t)
            }
        }
        val ws = http.newWebSocket(Request.Builder().url(url).build(), listener)
        val c = Conn(ws, noise, inbox)
        try {
            // Handshake Noise XX: write, read, write
            withTimeout(HANDSHAKE_MS) {
                while (!noise.isFinished()) {
                    send(ws, noise.writeStep())
                    if (!noise.isFinished()) noise.readStep(inbox.receive())
                }
            }
            // Pinning: se a chave do relay nao for a esperada, aborta SEM repetir (possivel MITM)
            val pin = pinnedRelayKeyHex?.trim()?.lowercase().orEmpty()
            if (pin.isNotEmpty() && noise.remoteStaticPublicKey().toHex() != pin) {
                throw PermanentRelayException("chave do relay diferente da esperada (possivel intermediario)")
            }
            // Autenticacao ligada a esta sessao Noise
            val r = roundtrip(c, qt.relayAuthMessage(noise.handshakeHash()))
            if (r !is RelayResponse.Ack) throw PermanentRelayException("autenticacao recusada pelo relay")
            Log.d(TAG, "ligado e autenticado em ${android.os.SystemClock.elapsedRealtime() - started} ms")
            return c
        } catch (e: Exception) {
            runCatching { ws.cancel() }; inbox.close()
            runCatching { noise.close() }   // ligacao falhada: liberta ja a sessao Noise
            throw e
        }
    }

    private suspend fun roundtrip(c: Conn, bytes: ByteArray, timeoutMs: Long = REQUEST_MS): RelayResponse {
        send(c.ws, c.noise.encrypt(bytes))
        val frame = withTimeout(timeoutMs) { c.inbox.receive() }
        return relayParseResponse(c.noise.decrypt(frame))
    }

    private fun send(ws: WebSocket, data: ByteArray) {
        if (!ws.send(data.toByteString())) throw IOException("nao foi possivel enviar (ligacao fechada)")
    }

    companion object {
        /**
         * Um so cliente HTTP para todas as ligacoes (a dos pedidos, a de espera e a dos anexos),
         * como recomenda o OkHttp: os certificados do sistema so sao preparados uma vez, as
         * threads sao partilhadas e as ligacoes novas reaproveitam a sessao TLS das anteriores.
         * Antes cada RelayLink criava o seu (3 por arranque e 3 por cada regresso a app).
         */
        private val http: OkHttpClient by lazy {
            // Cada WebSocket ocupa um lugar do Dispatcher enquanto esta aberto (o OkHttp le-o
            // dentro da chamada). O limite normal e 5 por servidor: ao mudar de rede, as 3
            // ligacoes antigas podem demorar a fechar enquanto abrem 3 novas, e uma ficaria
            // em fila. Com 16 isso nunca acontece.
            val dispatcher = Dispatcher().apply { maxRequestsPerHost = 16 }
            OkHttpClient.Builder()
                .dispatcher(dispatcher)
                .connectTimeout(10, TimeUnit.SECONDS)
                .pingInterval(25, TimeUnit.SECONDS)   // mantem a ligacao viva e deteta quedas
                .build()
        }
        private const val MAX_RETRIES = 3
        private const val BACKOFF_MS = 300L
        private const val HANDSHAKE_MS = 10_000L
        private const val REQUEST_MS = 15_000L
        private const val SHUT_MSG = "ligacao terminada"
        /** Etiqueta dos registos (adb logcat -s SealerRelay). Nunca regista conteudo nem chaves. */
        private const val TAG = "SealerRelay"
    }
}

fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
