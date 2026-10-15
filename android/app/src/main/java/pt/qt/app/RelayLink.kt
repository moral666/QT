package pt.qt.app

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout
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
    private val http = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .pingInterval(25, TimeUnit.SECONDS)   // mantem a ligacao viva e deteta quedas
        .build()
    private val lock = Mutex()
    private var conn: Conn? = null

    private class Conn(val ws: WebSocket, val noise: NoiseSession, val inbox: Channel<ByteArray>)

    /** Envia um pedido (ja construido com qt.relayRequest*) e devolve a resposta, com reconnect. */
    suspend fun request(bytes: ByteArray): RelayResponse = lock.withLock { requestLocked(bytes) }

    private suspend fun requestLocked(bytes: ByteArray): RelayResponse {
        var last: Exception? = null
        for (attempt in 0..MAX_RETRIES) {
            if (attempt > 0) delay((BACKOFF_MS shl (attempt - 1)).coerceAtMost(8_000L))
            try {
                val c = conn ?: connect().also { conn = it }
                return roundtrip(c, bytes)
            } catch (e: PermanentRelayException) {
                close(); throw e
            } catch (e: Exception) {
                close(); last = e          // timeout, queda de rede, frame invalido...
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
        val c = try {
            conn ?: connect().also { conn = it }
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
        }
        conn = null
    }

    private suspend fun connect(): Conn {
        // Defesa em profundidade alem do network_security_config: release so aceita wss://.
        val scheme = url.trim().lowercase()
        if (!scheme.startsWith("wss://") && !(BuildConfig.ALLOW_CLEARTEXT && scheme.startsWith("ws://"))) {
            throw PermanentRelayException("URL do relay recusado: usa wss:// (ws:// so na build debug)")
        }
        val noise = qt.relayNoiseSession()
        val inbox = Channel<ByteArray>(Channel.UNLIMITED)
        val listener = object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) { inbox.trySend(bytes.toByteArray()) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                inbox.close(IOException("ligacao fechada ($code)")); webSocket.close(1000, null)
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) { inbox.close(t) }
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
            return c
        } catch (e: Exception) {
            runCatching { ws.cancel() }; inbox.close()
            throw e
        }
    }

    private suspend fun roundtrip(c: Conn, bytes: ByteArray): RelayResponse {
        send(c.ws, c.noise.encrypt(bytes))
        val frame = withTimeout(REQUEST_MS) { c.inbox.receive() }
        return relayParseResponse(c.noise.decrypt(frame))
    }

    private fun send(ws: WebSocket, data: ByteArray) {
        if (!ws.send(data.toByteString())) throw IOException("nao foi possivel enviar (ligacao fechada)")
    }

    companion object {
        private const val MAX_RETRIES = 3
        private const val BACKOFF_MS = 300L
        private const val HANDSHAKE_MS = 10_000L
        private const val REQUEST_MS = 15_000L
    }
}

fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
