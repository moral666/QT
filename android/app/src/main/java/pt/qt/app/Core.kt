package pt.qt.app

import android.content.Context
import android.os.SystemClock
import android.util.Log
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.qt_ffi.CallEvent
import uniffi.qt_ffi.QtClient
import uniffi.qt_ffi.QtException
import uniffi.qt_ffi.ReceiveResult
import uniffi.qt_ffi.RelayResponse
import uniffi.qt_ffi.relayIdOfPublicId

/**
 * O nucleo da app aberto UMA vez por processo (chamadas, fase 4b).
 *
 * Antes so o ecra (MainViewModel) abria a base de dados. Para tocar uma chamada com a app
 * fechada ou o telemovel bloqueado, a campainha do relay (FCM, ver Push.kt) tambem tem de
 * a abrir, decifrar o que chegou e ver se e uma chamada. Dois QtClient na mesma base de
 * dados estragavam o estado do Double Ratchet, por isso ha um so, aqui, partilhado pelo
 * ecra e pelo segundo plano.
 *
 * Em segundo plano a base de dados so abre sem o codigo: com o bloqueio da app ativo nao
 * se decifra nada. Nesse caso a campainha marcada como chamada ("call") toca "Chamada
 * SEALER" sem nome e o codigo e pedido ao atender (aprovado pelo Nuno a 19/10/2026).
 *
 * Quando o ecra volta, fica com a chamada a tocar ([takeRinging]).
 */
object SealerCore {
    private const val TAG = "SealerCore"
    /** Quanto tempo uma chamada toca sem o ecra (igual ao RING_TIMEOUT_MS do Rust). */
    const val RING_MS = 45_000L
    /** "Atender" na notificacao vale durante este tempo (desbloquear a app pode demorar). */
    private const val ANSWER_VALID_MS = 60_000L
    private const val MAX_PAGES = 10

    private val lock = Any()
    @Volatile var client: QtClient? = null
        private set
    @Volatile private var contactsKeyRef: ByteArray? = null

    /** Trabalho em segundo plano que nao pertence ao ecra (campainha, toque, recusar). */
    val scope = CoroutineScope(
        SupervisorJob() + Dispatchers.IO +
            // Um erro aqui (ex.: base de dados fechada a meio) nunca pode fechar a app.
            CoroutineExceptionHandler { _, e -> Log.w(TAG, "segundo plano: ${e.javaClass.simpleName}") },
    )
    /** Uma rececao de cada vez (ecra ou segundo plano): nunca a mesma mensagem duas vezes ao mesmo tempo. */
    val receiveLock = Mutex()
    /** Um envio da outbox de cada vez (ecra ou segundo plano), pela ordem. */
    val flushLock = Mutex()
    /** O ecra esta visivel e recebe tudo sozinho: o segundo plano nao mexe. */
    @Volatile var uiReceiving = false
    /**
     * O ecra (MainViewModel) que usa agora o nucleo. Um ecra antigo que acaba (a Activity foi
     * recriada) so fecha o nucleo se ainda for ele o dono: antes fechava a base de dados que o
     * ecra novo ja estava a usar e cortava a chamada (0.9.23).
     */
    @Volatile var uiOwner: Any? = null

    fun dbFile(ctx: Context): File = ctx.applicationContext.noBackupFilesDir.resolve("qt.db")
    private fun box() = KeystoreBox()
    private fun bgBox() = KeystoreBox(KeystoreBox.BACKGROUND_ALIAS, unlockedDeviceRequired = false)
    fun storageKeys(ctx: Context) = StorageKeyProvider(ctx.applicationContext, box(), bgBox(), dbFile(ctx))
    fun contactStore(ctx: Context) = ContactStore(ctx.applicationContext, box(), bgBox())

    /** Chave da lista de contactos (derivada da do armazenamento), enquanto a base estiver aberta. */
    fun contactsKey(): ByteArray? = contactsKeyRef

    /** Abre o QtClient com a chave (ou devolve o que ja esta aberto). Apaga `key` da memoria. */
    fun open(ctx: Context, key: ByteArray): QtClient = try {
        synchronized(lock) {
            client ?: run {
                val ck = contactsKeyFrom(key)
                val c = try {
                    QtClient.open(dbFile(ctx).path, key)
                } catch (e: Exception) {
                    ck.fill(0)
                    throw e
                }
                contactsKeyRef = ck
                client = c
                c
            }
        }
    } finally {
        key.fill(0)
    }

    /** Fecha a base de dados (bloquear a app, apagar tudo, ecra fechado). */
    fun close() {
        val old = synchronized(lock) {
            val c = client
            client = null
            contactsKeyRef?.fill(0)
            contactsKeyRef = null
            c
        }
        val r = synchronized(ringLock) {
            val cur = ringing
            ringJob?.cancel()
            ringJob = null
            ringing = null
            answerCallId = null
            answerRequestedAt = 0L
            cur
        }
        r?.let { SystemCalls.end(it.sysKey, CallEnd.MISSED) }
        old?.close()
    }

    /**
     * Abre sem o codigo, para o segundo plano. `null`: bloqueio da app ativo, ainda sem
     * base de dados, ou chave indisponivel (ex.: ficheiro ainda no formato antigo e
     * telemovel bloqueado; passa ao novo da proxima vez que a app abrir).
     */
    private fun openInBackground(ctx: Context): QtClient? {
        client?.let { return it }
        if (!dbFile(ctx).exists()) return null
        return try {
            val keys = storageKeys(ctx)
            if (keys.lockEnabled()) null else open(ctx, keys.storageKey())
        } catch (e: Exception) {
            Log.w(TAG, "segundo plano: base de dados indisponivel (${e.javaClass.simpleName})")
            null
        }
    }

    private fun prefs(ctx: Context) = ctx.applicationContext.getSharedPreferences("qt_prefs", Context.MODE_PRIVATE)

    /** O relay da conta (federacao: da lista fixa da build, `Relays`). Debug: o das definicoes. */
    fun relayUrl(ctx: Context): String = Relays.home(prefs(ctx)).url

    fun relayPinHex(ctx: Context): String = Relays.home(prefs(ctx)).pin

    private fun newLink(ctx: Context, c: QtClient) = RelayLink(c, relayUrl(ctx), relayPinHex(ctx).ifBlank { null })

    /** So leitura (segundo plano): uma lista que nao abre conta como vazia, mas nunca e gravada por cima. */
    fun loadContacts(ctx: Context): List<Contact> = contactsKeyRef?.let { contactStore(ctx).loadOrEmpty(it) } ?: emptyList()

    // ------------------------------------------------------------- eventos das chamadas

    /** Eventos que o segundo plano leu mas nao eram dele (ficam para o ecra). */
    private val pendingEvents = ArrayDeque<CallEvent>()

    /** Eventos das chamadas: os guardados primeiro, depois os novos do Rust. Ecra e segundo plano usam este. */
    fun callEvents(c: QtClient): List<CallEvent> = synchronized(pendingEvents) {
        val out = ArrayList<CallEvent>(pendingEvents)
        pendingEvents.clear()
        out.addAll(c.callEvents())
        out
    }

    private fun giveBack(events: List<CallEvent>) = synchronized(pendingEvents) {
        for (e in events.asReversed()) pendingEvents.addFirst(e)
        while (pendingEvents.size > 64) pendingEvents.removeLast()
    }

    // ----------------------------------------------------------- chamada a tocar sem o ecra

    /**
     * Chamada a tocar sem o ecra. `callId` null: com o bloqueio da app (so se sabe que e uma
     * chamada). `video`: videochamada ("Videochamada de [Nome]" no aviso); pode passar a true
     * depois de comecar a tocar, se a op do video chegar noutro lote ([upgradeToVideo]).
     */
    class Ringing(val callId: String?, val contactId: String?, val name: String?, video: Boolean = false) {
        @Volatile var video: Boolean = video
            internal set
        /** A mesma chamada no sistema de chamadas do Android (SystemCalls), tambem quando o ecra fica com ela. */
        val sysKey: String = SystemCalls.newKey()
        /** O telemovel estava noutra chamada ao comecar a tocar: sem o toque alto (canal sem som). */
        @Volatile var quiet: Boolean = false
            internal set
    }

    private val ringLock = Any()
    @Volatile var ringing: Ringing? = null
        private set
    private var ringJob: Job? = null
    /** Carregou-se em "Atender" na notificacao: o ecra atende assim que a chamada aparecer. */
    @Volatile var answerRequestedAt = 0L
    /** A chamada a que se carregou "Atender" (null: bloqueio da app, ainda nao se sabe qual). */
    @Volatile var answerCallId: String? = null

    /** Ha um "Atender" por usar para esta chamada? */
    fun answerRequested(callId: String?): Boolean {
        val t = answerRequestedAt
        if (t == 0L || SystemClock.elapsedRealtime() - t >= ANSWER_VALID_MS) return false
        val want = answerCallId
        return want == null || want == callId
    }

    fun clearAnswer() {
        answerRequestedAt = 0L
        answerCallId = null
    }

    /** Uma chave aleatoria por processo: so as notificacoes da app atendem (nao outra app, nem os recentes). */
    val intentToken: String = java.util.UUID.randomUUID().toString()

    private fun startRinging(ctx: Context, r: Ringing) {
        // Ja noutra chamada (GSM ou outra app): mostra-se sem o toque alto por cima da conversa.
        r.quiet = SystemCalls.phoneBusy(ctx)
        synchronized(ringLock) {
            ringJob?.cancel()
            ringing = r
            ringJob = scope.launch { watchRinging(ctx, r) }
        }
        SystemCalls.incoming(ctx, r.sysKey, r.callId, r.name, r.video)
        CallNotify.ringing(ctx, r.name, sound = !r.quiet, video = r.video, kindKnown = r.callId != null)
    }

    /** A chamada a tocar afinal e uma videochamada: troca o aviso (sem voltar a tocar do inicio). */
    private fun upgradeToVideo(ctx: Context, r: Ringing) {
        // Dentro do lock: um "parar de tocar" ao mesmo tempo nunca fica com o aviso de volta.
        synchronized(ringLock) {
            if (ringing !== r || r.video) return
            r.video = true
            CallNotify.ringing(ctx, r.name, sound = !r.quiet, video = true, update = true)
        }
    }

    /** Para de tocar se `r` ainda for a chamada a tocar. */
    private fun stopRinging(ctx: Context, r: Ringing, missed: Boolean) {
        synchronized(ringLock) {
            if (ringing !== r) return
            ringing = null
            ringJob = null
        }
        clearAnswer()
        CallNotify.clear(ctx)
        SystemCalls.end(r.sysKey, CallEnd.MISSED)
        if (missed) CallNotify.missed(ctx, r.name)
    }

    /** O ecra voltou: fica com a chamada a tocar (e para o toque do segundo plano). */
    fun takeRinging(ctx: Context): Ringing? {
        val r = synchronized(ringLock) {
            val cur = ringing ?: return null
            ringing = null
            ringJob?.cancel()
            ringJob = null
            cur
        }
        CallNotify.clear(ctx)
        return r
    }

    /** "Recusar" na notificacao, com a chamada a tocar sem o ecra. */
    suspend fun declineRinging(ctx: Context) {
        val r = synchronized(ringLock) {
            val cur = ringing ?: return
            ringing = null
            ringJob?.cancel()
            ringJob = null
            cur
        }
        clearAnswer()
        CallNotify.clear(ctx)
        SystemCalls.end(r.sysKey, CallEnd.REJECTED)
        // Com o bloqueio da app nao ha como avisar quem liga: para de tocar e ele fica "sem resposta".
        val id = r.callId ?: return
        val c = client ?: return
        withTimeoutOrNull(8_000L) {
            try {
                c.callHangup(id)
                val l = newLink(ctx, c)
                try { flush(ctx, c, l) } finally { l.shutdown() }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "recusar: ${e.javaClass.simpleName}")
            }
        }
    }

    /**
     * Enquanto toca sem o ecra: fica a espera no relay (para o "desligou" de quem liga
     * chegar logo) e trata os eventos da chamada. Pode parar a meio se o Android congelar
     * o processo: a campainha "call_end" do relay e o limite da notificacao resolvem isso.
     */
    private suspend fun watchRinging(ctx: Context, r: Ringing) {
        val until = SystemClock.elapsedRealtime() + RING_MS
        val c = client
        if (r.callId == null || c == null) {
            delay(RING_MS)
            stopRinging(ctx, r, missed = true)
            return
        }
        val l = newLink(ctx, c)
        try {
            while (ringing === r && SystemClock.elapsedRealtime() < until) {
                // Ja nao e a chamada do Rust (desligou, ou o ecra leu o "acabou"): para.
                if (runCatching { c.callCurrent() }.getOrNull() != r.callId) break
                try {
                    val got = l.request(c.relayRequestWait(2u), 8_000L)
                    if (got is RelayResponse.Messages && got.messages.isNotEmpty()) syncOnce(ctx, c, l)
                    else handleCallEvents(ctx, c)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    runCatching { handleCallEvents(ctx, c) }
                    delay(2_000L)
                }
            }
        } finally {
            l.shutdown()
            // Ainda a tocar (fim do tempo, chamada acabada ou erro): perdida.
            if (ringing === r) {
                runCatching { handleCallEvents(ctx, c) }
                stopRinging(ctx, r, missed = true)
            }
        }
    }

    /**
     * O ecra acabou a meio de uma chamada (ex.: a app foi fechada nos recentes): avisa o outro
     * lado ("desligou"), para ele nao ficar a espera, e so depois fecha o nucleo ([closeAfter]).
     */
    fun hangupInBackground(ctx: Context, c: QtClient, callId: String, closeAfter: Boolean) {
        val app = ctx.applicationContext
        SystemCalls.end(callId, CallEnd.LOCAL)
        scope.launch {
            withTimeoutOrNull(8_000L) {
                try {
                    c.callHangup(callId)
                    val l = newLink(app, c)
                    try { flush(app, c, l) } finally { l.shutdown() }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.w(TAG, "desligar: ${e.javaClass.simpleName}")
                }
            }
            // Entretanto abriu-se outro ecra: o nucleo fica para ele.
            if (closeAfter && uiOwner == null) close()
        }
    }

    /**
     * Atender vindo do sistema de chamadas (auricular, carro) sem o ecra com a chamada: como o
     * "Atender" da notificacao. `callId` null: bloqueio da app, ainda nao se sabe qual e.
     */
    fun answerFromSystem(ctx: Context, key: String, callId: String?) {
        val r = ringing
        answerCallId = callId ?: r?.takeIf { it.sysKey == key }?.callId
        answerRequestedAt = SystemClock.elapsedRealtime()
        // A tocar aqui: o toque para (a app abre e atende assim que tiver a chamada).
        if (r != null && r.sysKey == key) CallNotify.ringing(ctx, r.name, sound = false, video = r.video, kindKnown = r.callId != null, update = true)
        CallNotify.openToAnswer(ctx)
    }

    /** Recusar vindo do sistema de chamadas (auricular, carro) com a chamada a tocar aqui. */
    fun declineFromSystem(ctx: Context, key: String) {
        val r = ringing ?: return
        if (r.sysKey != key) return
        scope.launch { declineRinging(ctx.applicationContext) }
    }

    /** O Firebase deu um token novo: manda-o ao relay se a base de dados estiver aberta. */
    fun pushTokenChanged(ctx: Context, token: String) {
        val app = ctx.applicationContext
        val c = client ?: return
        if (!Push.isOn(app)) return
        scope.launch {
            withTimeoutOrNull(10_000L) {
                val l = newLink(app, c)
                try {
                    val r = l.request(c.relayRequestPushToken(token))
                    if (r !is RelayResponse.Ack) Log.w(TAG, "relay recusou o token novo")
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.w(TAG, "token novo: ${e.javaClass.simpleName}")
                } finally {
                    l.shutdown()
                }
            }
        }
    }

    // ------------------------------------------------------------------- a campainha

    /**
     * Campainha do relay (FCM): `kind` "1" (mensagem), "call" ou "call_end". Corre na thread
     * do Firebase, que da poucos segundos: o que for preciso depois disso (o toque) segue
     * em [scope].
     */
    fun onPush(ctx: Context, kind: String?) {
        val app = ctx.applicationContext
        if (uiReceiving) return   // o ecra esta aberto e ligado: recebe tudo sozinho
        val c = openInBackground(app)
        if (c == null) {
            val locked = runCatching { storageKeys(app).lockEnabled() }.getOrDefault(false)
            when (kind) {
                "call" -> if (locked) {
                    if (ringing == null) startRinging(app, Ringing(null, null, null))
                } else {
                    Push.showGeneric(app)
                }
                "call_end" -> ringing?.let { if (it.callId == null) stopRinging(app, it, missed = true) }
                else -> Push.showGeneric(app)
            }
            return
        }
        val ok = runBlocking {
            withTimeoutOrNull(9_000L) {
                val l = newLink(app, c)
                try {
                    syncOnce(app, c, l)
                    true
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.w(TAG, "segundo plano: ${e.javaClass.simpleName}")
                    false
                } finally {
                    l.shutdown()
                }
            } ?: false
        }
        // O tempo acabou a meio: as chamadas ja recebidas ainda tocam.
        if (!ok) runCatching { handleCallEvents(app, c) }
        // Sem rede ou relay em baixo: pelo menos o aviso de sempre.
        if (!ok && kind != "call_end") Push.showGeneric(app)
    }

    /** Busca, decifra, confirma e trata as chamadas; envia os recibos e o que estiver na outbox. */
    private suspend fun syncOnce(ctx: Context, c: QtClient, l: RelayLink) {
        val newMessage = receiveLock.withLock {
            var fresh = false
            var received = false
            for (page in 0 until MAX_PAGES) {
                val msgs = (l.request(c.relayRequestFetch()) as? RelayResponse.Messages)?.messages ?: break
                if (msgs.isEmpty()) break
                val ack = mutableListOf<ULong>()
                for (m in msgs) {
                    try {
                        when (c.receiveSealed(m.ciphertext)) {
                            is ReceiveResult.NewMessage -> { fresh = true; received = true }
                            is ReceiveResult.StatusChanged, is ReceiveResult.Updated -> received = true
                            is ReceiveResult.NoChange -> {}
                        }
                        ack += m.id
                    } catch (e: QtException.Storage) {
                        // Problema local: nao confirmar, o relay volta a entregar.
                    } catch (e: QtException) {
                        ack += m.id   // repetida ou invalida
                    }
                }
                if (ack.isNotEmpty()) l.request(c.relayRequestAck(ack))
            }
            if (received) runCatching { c.prepareDeliveryReceipts() }
            fresh
        }
        handleCallEvents(ctx, c)
        flush(ctx, c, l)
        if (newMessage) Push.showGeneric(ctx)
    }

    /** Envia a outbox (recibos, "a tocar", recusas e mensagens que tinham ficado por enviar). */
    private suspend fun flush(ctx: Context, c: QtClient, l: RelayLink) = flushLock.withLock {
        val contacts = loadContacts(ctx)
        val members by lazy { c.groups().flatMap { it.members }.associate { it.contactId to it.publicId } }
        for (item in c.outbox()) {
            val relayId = item.relayId
                ?: contacts.firstOrNull { it.contactId == item.contactId }?.relayId
                ?: members[item.contactId]?.let { runCatching { relayIdOfPublicId(it) }.getOrNull() }
                ?: continue
            val r = try {
                l.request(c.relayRequestSendItem(relayId, item))
            } catch (e: Exception) {
                break   // sem rede: fica para depois, pela mesma ordem
            }
            if (r is RelayResponse.Ack) c.outboxSent(item.outboxId)
        }
    }

    /** Eventos das chamadas no segundo plano: toca (contactos guardados) ou para de tocar. */
    private fun handleCallEvents(ctx: Context, c: QtClient) {
        val events = callEvents(c)
        if (events.isEmpty()) return
        if (uiReceiving) {
            giveBack(events)
            return
        }
        val contacts by lazy { loadContacts(ctx) }
        val blocked by lazy { c.blockedContacts().toSet() }
        // Com o bloqueio da app ativo o nome nunca aparece (a base pode estar aberta pelo ecra).
        val locked by lazy { runCatching { storageKeys(ctx).lockEnabled() }.getOrDefault(true) }
        val leftover = mutableListOf<CallEvent>()
        for (ev in events) {
            when (ev) {
                is CallEvent.Incoming -> {
                    val contact = contacts.firstOrNull { it.contactId == ev.contactId }
                    if (contact == null || ev.contactId in blocked) {
                        // Como no ecra: quem nao esta nos contactos e recusado em silencio.
                        runCatching { c.callHangup(ev.callId)?.let { c.outboxSent(it.outboxId) } }
                    } else {
                        // O Rust ja responde "ocupado" durante uma chamada: um toque antigo e lixo.
                        ringing?.let { stopRinging(ctx, it, missed = false) }
                        // A op "videochamada" chega com a oferta: ja foi tratada neste lote.
                        val video = runCatching { c.callIsVideo(ev.callId) }.getOrDefault(false)
                        startRinging(ctx, Ringing(ev.callId, ev.contactId, if (locked) null else contact.nickname, video))
                    }
                }
                is CallEvent.Ended -> {
                    val r = ringing
                    if (r != null && r.callId == ev.callId) stopRinging(ctx, r, missed = true) else leftover += ev
                }
                is CallEvent.Video -> {
                    // A op "videochamada" chegou depois da oferta: o aviso passa a "Videochamada de [Nome]".
                    val r = ringing
                    if (r != null && r.callId == ev.callId && !r.video &&
                        runCatching { c.callIsVideo(ev.callId) }.getOrDefault(false)
                    ) {
                        upgradeToVideo(ctx, r)
                    }
                    leftover += ev   // o ecra tambem precisa dele (estado do video)
                }
                else -> leftover += ev
            }
        }
        if (leftover.isNotEmpty()) giveBack(leftover)
    }
}
