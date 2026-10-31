package pt.qt.app

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.TimeoutCancellationException
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
 *   ligar -> handshake Noise -> (pinning) -> autenticar -> pedidos.
 * Qualquer erro: fecha, DESCARTA a NoiseSession e re-liga com backoff (nova sessao sempre).
 *
 * Desde a 0.9.45 a app usa UMA so ligacao para tudo (antes eram tres: pedidos, espera e anexos;
 * cada ligacao custa ~40 KB no servidor). Os pedidos vao em fila pela mesma ligacao, sem esperar
 * pela resposta do anterior, e o relay responde sempre pela mesma ordem: cada resposta vai para o
 * pedido mais antigo por responder ([Conn.pending]). A espera por mensagens (`WaitMessages`) e um
 * pedido como os outros: quando chega outro pedido a meio, o relay acaba a espera (responde com o
 * que houver) e trata o pedido a seguir. Assim um envio nunca fica parado atras da espera.
 */
class RelayLink(
    private val qt: QtClient,
    private val url: String,
    /** Chave publica estatica Noise do relay (hex), impressa no arranque do `relay_server`. Opcional. */
    private val pinnedRelayKeyHex: String?,
) {
    /** Para ligar e para escrever na ligacao: cifrar e enviar tem de ser pela mesma ordem. */
    private val sendLock = Mutex()
    @Volatile private var conn: Conn? = null
    /** Terminada de vez ([shutdown]): nao volta a ligar, os pedidos falham logo. */
    @Volatile private var shut = false

    private class Conn(val noise: NoiseSession, val inbox: Channel<ByteArray>) {
        lateinit var ws: WebSocket
        /** Respostas por chegar, pela ordem dos pedidos (guardado com `synchronized(pending)`). */
        val pending = ArrayDeque<CompletableDeferred<RelayResponse>>()
        /** Handshake e autenticacao acabados: as frames passam a ir para [pending]. */
        @Volatile var ready = false
        @Volatile var dead = false

        /** Uma frame do relay (thread do OkHttp, uma de cada vez e pela ordem de chegada). */
        fun onFrame(frame: ByteArray) {
            if (!ready) { inbox.trySend(frame); return }
            val d = synchronized(pending) { pending.removeFirstOrNull() }
            if (d == null) { fail(IOException("resposta do relay sem pedido")); return }
            try {
                // Decifrar aqui, pela ordem de chegada (os contadores do Noise exigem-no).
                d.complete(relayParseResponse(noise.decrypt(frame)))
            } catch (e: Exception) {
                d.completeExceptionally(e)
                fail(e)
            }
        }

        /** A ligacao acabou: todos os pedidos por responder falham (quem pode repetir, repete). */
        fun fail(e: Throwable) {
            val all = synchronized(pending) {
                dead = true
                pending.toList().also { pending.clear() }
            }
            for (d in all) d.completeExceptionally(e)
            inbox.close(e)
        }

        /** Junta pedidos a fila; falha se a ligacao ja acabou. */
        fun expect(n: Int): List<CompletableDeferred<RelayResponse>> = synchronized(pending) {
            if (dead) throw IOException("ligacao fechada")
            List(n) { CompletableDeferred<RelayResponse>() }.also { pending.addAll(it) }
        }
    }

    /**
     * Envia um pedido (ja construido com qt.relayRequest*) e devolve a resposta, com reconnect.
     * `timeoutMs`: tempo maximo pela resposta (maior para `relayRequestWait`, que so responde
     * quando chega uma mensagem ou outro pedido).
     */
    suspend fun request(bytes: ByteArray, timeoutMs: Long = REQUEST_MS): RelayResponse {
        var last: Exception? = null
        val repeatable = repeatable(bytes)
        for (attempt in 0..MAX_RETRIES) {
            if (attempt > 0) delay((BACKOFF_MS shl (attempt - 1)).coerceAtMost(8_000L))
            if (shut) throw IOException(SHUT_MSG)
            var c: Conn? = null
            try {
                val (sentOn, answers) = enqueue(listOf(bytes))
                c = sentOn
                return withTimeout(timeoutMs) { answers[0].await() }
            } catch (e: PermanentRelayException) {
                close(c); throw e
            } catch (e: Exception) {
                // Quem pediu desistiu (ecra fechado): a resposta, quando chegar, fica sem dono e
                // a ligacao continua boa para os outros pedidos.
                if (e is CancellationException && e !is TimeoutCancellationException) throw e
                close(c); last = e          // timeout, queda de rede, frame invalido...
                Log.w(TAG, "pedido falhou (tentativa ${attempt + 1}): ${e.javaClass.simpleName}: ${e.message?.take(120)}")
                // Ja tinha saido e a resposta perdeu-se: um envio ou um anexo novo nao se repete
                // aqui (podia chegar duas vezes). Quem chama tenta mais tarde (a outbox fica).
                if (c != null && !repeatable) throw IOException("sem resposta do relay", e)
            }
        }
        throw IOException("relay indisponivel: ${last?.message}", last)
    }

    /**
     * Pode repetir-se sem efeito duplicado (leituras, confirmacoes, registos)? Desde a 0.9.23
     * nao se repetem automaticamente, depois de enviados: mensagens (SendMessage), anexos
     * novos (BlobCreate), denuncias e a espera por mensagens.
     */
    private fun repeatable(bytes: ByteArray): Boolean {
        val head = String(bytes, 0, minOf(bytes.size, 24), Charsets.ISO_8859_1)
        return NOT_REPEATABLE.none { head.startsWith(it) }
    }

    /**
     * Varios pedidos seguidos e so depois as respostas, pela mesma ordem: para os
     * blocos dos anexos, sem esperar uma volta inteira a rede por cada bloco.
     * Sem repeticao automatica: se falhar, quem chama retoma (o relay diz onde ficou).
     * Um pedido de outra parte da app que chegue entretanto vai logo a seguir a estes blocos
     * (por isso os lotes sao pequenos: [MainViewModel] usa 4 blocos de 40 KB).
     */
    suspend fun requestMany(requests: List<ByteArray>): List<RelayResponse> {
        if (requests.isEmpty()) return emptyList()
        var c: Conn? = null
        try {
            val (sentOn, answers) = enqueue(requests)
            c = sentOn
            return answers.map { withTimeout(REQUEST_MS) { it.await() } }
        } catch (e: Exception) {
            if (e is CancellationException && e !is TimeoutCancellationException) throw e
            close(c); throw e
        }
    }

    /**
     * Cifra e envia os pedidos (ligando primeiro, se preciso) e devolve onde ficam as respostas.
     * Ligar e escrever acontecem com [sendLock]: os pedidos saem inteiros e pela ordem da fila.
     */
    private suspend fun enqueue(requests: List<ByteArray>): Pair<Conn, List<CompletableDeferred<RelayResponse>>> =
        sendLock.withLock {
            if (shut) throw IOException(SHUT_MSG)
            val c = conn?.takeUnless { it.dead } ?: run {
                close(conn)
                connect().also { conn = it; closeIfShut(it) }
            }
            try {
                val answers = c.expect(requests.size)
                for (r in requests) send(c.ws, c.noise.encrypt(r))
                c to answers
            } catch (e: Exception) {
                close(c); throw e
            }
        }

    fun close() = close(conn)

    /** Fecha esta ligacao (se ainda for a atual, deixa de o ser). Os pedidos por responder falham. */
    private fun close(c: Conn?) {
        if (c == null) return
        c.fail(IOException("ligacao fechada"))
        runCatching { c.ws.close(1000, null) }
        // Liberta ja a sessao Noise no Rust (e as chaves desta ligacao), sem esperar pelo GC.
        // Se outra thread ainda a estiver a usar, o UniFFI so a apaga quando essa acabar.
        runCatching { c.noise.close() }
        if (conn === c) conn = null
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
    private fun closeIfShut(c: Conn) {
        if (shut) {
            close(c)
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
        val c = Conn(noise, inbox)
        val listener = object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) { c.onFrame(bytes.toByteArray()) }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                Log.w(TAG, "ligacao fechada pelo relay ($code)")
                c.fail(IOException("ligacao fechada ($code)")); webSocket.close(1000, null)
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                Log.w(TAG, "ligacao caiu: ${t.javaClass.simpleName}: ${t.message?.take(120)}")
                c.fail(t)
            }
        }
        val ws = http.newWebSocket(Request.Builder().url(url).build(), listener)
        c.ws = ws
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
            // O pin e obrigatorio em todas as builds: sem ele nao se liga (0.9.23; tambem na debug desde a 0.9.53).
            if (pin.isEmpty()) throw PermanentRelayException("falta a chave do relay (pin)")
            if (pin.isNotEmpty() && noise.remoteStaticPublicKey().toHex() != pin) {
                throw PermanentRelayException("chave do relay diferente da esperada (possivel intermediario)")
            }
            // Autenticacao ligada a esta sessao Noise
            val r = roundtrip(c, qt.relayAuthMessage(noise.handshakeHash()))
            if (r !is RelayResponse.Ack) throw PermanentRelayException("autenticacao recusada pelo relay")
            c.ready = true   // daqui em diante as respostas vao para a fila dos pedidos
            Log.d(TAG, "ligado e autenticado em ${android.os.SystemClock.elapsedRealtime() - started} ms")
            return c
        } catch (e: Exception) {
            c.dead = true
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
         * Um so cliente HTTP para todas as ligacoes (a do ecra e a do segundo plano),
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
        private val NOT_REPEATABLE = listOf("{\"SendMessage\"", "{\"BlobCreate\"", "{\"SubmitReport\"", "{\"WaitMessages\"")
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
