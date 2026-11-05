package pt.qt.app

import android.app.Application
import android.graphics.SurfaceTexture
import android.os.SystemClock
import android.util.Log
import android.view.Surface
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import uniffi.qt_ffi.CallRelayInfo
import uniffi.qt_ffi.GroupAudio
import uniffi.qt_ffi.GroupCallEndReason
import uniffi.qt_ffi.GroupCallEvent
import uniffi.qt_ffi.QtClient
import uniffi.qt_ffi.QtException

/**
 * Chamadas de GRUPO no Android (0.9.63): ate 5 pessoas, voz e video. O nucleo (Rust,
 * `group_call.rs`) faz uma malha de chamadas a dois; aqui ha so o microfone e o altifalante (a
 * mistura vem pronta do Rust), a camara (uma so codificacao para todos), um descodificador por
 * participante com a camara ligada, o toque e o ecra.
 *
 * Antes da primeira chamada de grupo, a app mostra o aviso do IP com "Aceitar" (sem isso o Rust
 * recusa ligar ou entrar). Durante a chamada nao aparece nada sobre isto.
 *
 * 0.9.71: a chamada de grupo comeca numa chamada a dois, com "Adicionar a ligacao" ([add]). Os
 * grupos de conversa nao tem chamadas (0.9.70).
 */
private const val TAG = "SealerGroupCall"

/** Um participante no ecra (eu nao conto). */
data class GroupTile(
    val contactId: String,
    val name: String,
    val connected: Boolean,
    val camera: Boolean,
    val videoW: Int = 0,
    val videoH: Int = 0,
    val rotation: Int = 0,
)

/**
 * O que a chamada a dois deixa para a de grupo: o microfone e a saida do som ficam como estavam,
 * e a chamada no sistema de chamadas (`systemId`) continua, agora para o grupo.
 */
data class CallHandover(
    val muted: Boolean,
    val speaker: Boolean,
    val systemId: String?,
    val routes: List<AudioRoute> = emptyList(),
    val route: AudioRoute? = null,
)

data class GroupCallUi(
    val callId: String,
    /** Quem me convidou (ou com quem eu estava na chamada a dois): a foto e o nome no topo. */
    val fromId: String,
    val title: String,
    /** Ja estou dentro (false: esta a tocar). */
    val joined: Boolean,
    /** Videochamada de grupo (a camara de quem liga comecou ligada). */
    val video: Boolean,
    /** Quem me convidou (so a tocar). */
    val fromName: String = "",
    val tiles: List<GroupTile> = emptyList(),
    /** Quem esta a falar (contactId), para o quadrado grande. */
    val speaker: String? = null,
    val meSpeaking: Boolean = false,
    val muted: Boolean = false,
    val speakerOn: Boolean = true,
    val camera: Boolean = false,
    val front: Boolean = true,
    val previewW: Int = 0,
    val previewH: Int = 0,
    /** Quando entrei (SystemClock.elapsedRealtime), para o contador. */
    val startedAt: Long = 0L,
)

class GroupCallController(
    private val app: Application,
    private val scope: CoroutineScope,
    private val client: () -> QtClient?,
    private val flush: suspend () -> Unit,
    /** Nome a mostrar de um contactId (alcunha, ou o nome no grupo). */
    private val nameOf: (String) -> String,
    private val sounds: CallSounds,
    /** Ha uma chamada a dois no ecra (o convite e recusado em silencio). */
    private val busy: () -> Boolean,
    private val visible: () -> Boolean,
    private val onError: (String) -> Unit,
    /**
     * Cria as sessoes cifradas que faltam com quem esta na lista da chamada (public IDs), como
     * nos grupos. Sem sessao, duas pessoas que nao sao contactos nao se viam nem se ouviam.
     */
    private val ensureSessions: suspend (List<String>) -> Unit = {},
    /** Esta nos meus contactos (entao mostra-se o nome que eu lhe dei, nao o da chamada). */
    private val isContact: (String) -> Boolean = { false },
    /** O que a chamada a dois deixou, uma so vez. `null`: nao vim de uma chamada a dois. */
    private val takeHandover: () -> CallHandover? = { null },
) {
    private val prefs = app.getSharedPreferences("sealer_group_call", android.content.Context.MODE_PRIVATE)

    /** O nome que escolhi para as chamadas de grupo (0.9.67; na base cifrada desde a 0.9.74). */
    var myName: String
        get() = client()?.let { c -> runCatching { c.groupCallMyName() }.getOrNull() }.orEmpty()
        set(v) {
            client()?.let { c -> runCatching { c.groupCallSaveMyName(v.trim().take(32)) } }
        }

    /** Passa o nome das versoes antigas para a base cifrada. */
    fun moveNameToCore(c: QtClient) {
        val old = prefs.getString("name", null) ?: return
        if (runCatching { c.groupCallSaveMyName(old) }.isSuccess) prefs.edit().remove("name").apply()
    }

    /** Envia o meu nome para esta chamada (quem nao me tem nos contactos ve-o). */
    private fun sendName(c: QtClient, callId: String) {
        val n = myName
        if (n.isNotBlank()) runCatching { c.groupCallSetName(callId, n) }
    }

    private val _ui = MutableStateFlow<GroupCallUi?>(null)
    val ui: StateFlow<GroupCallUi?> = _ui.asStateFlow()

    /** Chamadas a decorrer em que nao estou (recusei ou nao atendi): quem convidou -> chamada. */
    private val _ongoing = MutableStateFlow<Map<String, String>>(emptyMap())
    val ongoing: StateFlow<Map<String, String>> = _ongoing.asStateFlow()

    @Volatile private var audio: GroupAudio? = null
    private var io: CallAudioIO? = null
    private var sender: VideoSender? = null
    private val receivers = HashMap<String, VideoReceiver>()
    private val surfaces = HashMap<String, Surface?>()
    private var preview: SurfaceTexture? = null
    private var ticker: Job? = null
    private var stopNet: (() -> Unit)? = null
    /** Esta chamada no sistema de chamadas (auricular, carro): herdada ou vinda do toque. */
    @Volatile private var systemId: String? = null
    private var routes: List<AudioRoute> = emptyList()
    private var route: AudioRoute? = null
    /** O toque veio do segundo plano: ainda esta no sistema como chamada a chegar. */
    @Volatile private var ringingSystemId: String? = null

    /** A chamada do sistema `id` e a desta chamada de grupo? */
    fun ownsSystem(id: String): Boolean = _ui.value != null && (systemId == id || ringingSystemId == id)

    fun noticeAccepted(): Boolean = runCatching { client()?.groupCallNoticeAccepted() }.getOrNull() == true

    fun acceptNotice() {
        runCatching { client()?.groupCallAcceptNotice() }
    }

    // ------------------------------------------------------------ acoes

    /** As sessoes que faltam com a lista da chamada (antes de entrar). */
    private suspend fun ensureCallSessions(c: QtClient, callId: String) {
        val ids = runCatching { c.groupCallNeedingSession(callId) }.getOrDefault(emptyList())
        if (ids.isNotEmpty()) runCatching { ensureSessions(ids) }
    }

    /**
     * ADICIONAR A LIGACAO (0.9.71), com o aviso ja aceite. Numa chamada de grupo: os novos entram
     * na lista de todos. Numa chamada a dois atendida (`peer` = esse contacto): ela passa a
     * chamada de grupo; `detach` para o microfone e o ecra da chamada a dois antes de o audio do
     * grupo arrancar.
     */
    fun add(publicIds: List<String>, peer: Contact?, video: Boolean, detach: () -> Unit) = launch {
        val c = client() ?: return@launch
        val cur = _ui.value
        if (cur != null) {
            if (!cur.joined) return@launch
            try {
                c.groupCallAdd(publicIds, cur.camera)
            } catch (e: Exception) {
                onError(joinError(e))
                return@launch
            }
            flush()
            refresh(c)
            return@launch
        }
        val p = peer ?: return@launch
        val st = try {
            c.groupCallAdd(publicIds + p.publicId, video)
        } catch (e: Exception) {
            onError(joinError(e))
            return@launch
        }
        detach()
        val h = inherit()
        _ui.value = GroupCallUi(
            st.callId, p.contactId, nameOf(p.contactId), joined = true, video = video, camera = video,
            muted = h?.muted == true, speakerOn = h?.speaker ?: true,
            startedAt = SystemClock.elapsedRealtime(),
        )
        sendName(c, st.callId)
        flush()
        startMedia(c, st.callId)
        refresh(c)
        ensureTicker()
    }

    /** ENTRAR na chamada que esta a tocar. */
    fun join() = launch {
        val c = client() ?: return@launch
        val g = _ui.value?.takeIf { !it.joined } ?: return@launch
        sounds.stop()
        CallNotify.clear(app)
        try {
            ensureCallSessions(c, g.callId)
            c.groupCallJoin(g.callId)
        } catch (e: Exception) {
            // Cheia (5 pessoas) ou ja acabou.
            onError(joinError(e))
            end()
            return@launch
        }
        // O toque veio do segundo plano: o sistema de chamadas fica com o som antes do microfone.
        ringingSystemId?.let { id ->
            ringingSystemId = null
            runCatching { SystemCalls.answer(id) }
            systemId = id
        }
        // Vinda de uma chamada a dois, a camara fica como estava nela (decide o nucleo).
        val camera = runCatching { c.groupCallCamera(g.callId) }.getOrDefault(false)
        _ui.update { it?.copy(joined = true, camera = camera, startedAt = SystemClock.elapsedRealtime()) }
        sendName(c, g.callId)
        flush()
        startMedia(c, g.callId)
        refresh(c)
    }

    /** ENTRAR DEPOIS numa chamada que recusei (ou nao atendi): o botao na conversa com quem convidou. */
    fun rejoin(fromId: String) = launch {
        val c = client() ?: return@launch
        val id = _ongoing.value[fromId] ?: return@launch
        if (_ui.value != null || busy()) return@launch
        try {
            ensureCallSessions(c, id)
            c.groupCallRejoin(id)
        } catch (e: Exception) {
            onError(joinError(e))
            updateOngoing(c, fromId)
            return@launch
        }
        val video = runCatching { c.groupCallIsVideo(id) }.getOrDefault(false)
        _ui.value = GroupCallUi(
            id, fromId, nameOf(fromId), joined = true, video = video, camera = video,
            startedAt = SystemClock.elapsedRealtime(),
        )
        updateOngoing(c, fromId)
        sendName(c, id)
        flush()
        startMedia(c, id)
        refresh(c)
        ensureTicker()
    }

    private fun updateOngoing(c: QtClient, fromId: String) {
        val id = runCatching { c.groupCallOngoing(fromId) }.getOrNull()
        _ongoing.update { m -> if (id == null) m - fromId else m + (fromId to id) }
    }

    /** SAIR (ou recusar o convite). */
    fun leave() = launch {
        val g = _ui.value ?: return@launch
        runCatching { client()?.groupCallLeave(g.callId) }
        flush()
        end()
    }

    /** "Desligar" na notificacao da chamada (o servico chama [CallControl.hangup]). */
    private val hangupFromNotification: () -> Unit = { leave() }

    /**
     * O ecra (ViewModel) vai fechar a meio da chamada de grupo (ex.: a app foi fechada nos
     * recentes): o microfone, a camara, os descodificadores e o servico param ja. Devolve a
     * chamada, para o "sai" ir aos outros em segundo plano (`SealerCore.leaveGroupCallInBackground`).
     * `null`: nao havia chamada de grupo.
     */
    fun closeForScreenGone(): String? {
        val g = _ui.value ?: return null
        ticker?.cancel()
        ticker = null
        end()
        return g.callId
    }

    fun toggleMute() {
        val m = !(_ui.value?.muted ?: return)
        io?.muted = m
        _ui.update { it?.copy(muted = m) }
        // O carro e o auricular ficam a saber (so com a chamada no sistema de chamadas).
        systemId?.let { SystemCalls.setMuted(app, it, m) }
    }

    fun toggleSpeaker() {
        val on = !(_ui.value?.speakerOn ?: return)
        // No sistema de chamadas a saida e escolhida por la (a resposta vem em [onSystemRoutes]).
        val sys = systemId
        if (sys != null && routes.isNotEmpty() && SystemCalls.manages(sys)) {
            routes.toggleTarget(route)?.let { SystemCalls.setRoute(sys, it.id) }
            return
        }
        io?.setSpeaker(on)
        _ui.update { it?.copy(speakerOn = on) }
    }

    /** O sistema de chamadas desligou (auricular, carro, chamada GSM atendida por cima). */
    fun onSystemHangup() = leave()

    fun onSystemMuted(m: Boolean) {
        if (_ui.value?.muted == m) return
        io?.muted = m
        _ui.update { it?.copy(muted = m) }
    }

    fun onSystemRoutes(list: List<AudioRoute>, current: AudioRoute?) {
        routes = list
        route = current
        _ui.update { it?.copy(speakerOn = current?.kind == RouteKind.SPEAKER) }
    }

    /** "Atender" na notificacao com esta chamada a tocar no ecra. `false`: nao e nada comigo. */
    fun answerFromNotification(): Boolean {
        if (_ui.value?.joined != false) return false
        join()
        return true
    }

    /**
     * O ecra voltou com um convite a tocar no segundo plano: o toque passa para aqui. Quem ja
     * carregou em "Atender" (notificacao, auricular, carro) entra logo.
     */
    fun adoptRinging(c: QtClient, r: SealerCore.Ringing) {
        val id = r.callId ?: return
        // Entretanto acabou (desligou, ou passou o tempo): fica so como perdida.
        if (runCatching { c.groupCallCurrent() }.getOrNull() != id || _ui.value != null) {
            SealerCore.clearAnswer()
            SystemCalls.end(r.sysKey, CallEnd.MISSED)
            return
        }
        val from = r.contactId.orEmpty()
        ringingSystemId = id
        _ui.value = GroupCallUi(id, from, nameOf(from), joined = false, video = r.video, fromName = nameOf(from))
        CallControl.hangup = hangupFromNotification
        ensureTicker()
        if (SealerCore.answerRequested(id) && noticeAccepted()) {
            SealerCore.clearAnswer()
            join()
        } else if (!r.quiet) {
            sounds.ringIncoming()
        }
    }

    fun toggleCamera() = launch {
        val g = _ui.value?.takeIf { it.joined } ?: return@launch
        val on = !g.camera
        runCatching { client()?.groupCallSetCamera(g.callId, on) }
        _ui.update { it?.copy(camera = on) }
        flush()
        syncCamera()
    }

    fun switchCamera() {
        sender?.switchCamera()
        _ui.update { u -> u?.let { it.copy(front = !it.front) } }
    }

    fun setPreview(st: SurfaceTexture?) {
        preview = st
        sender?.setPreview(st)
    }

    /** O ecra do video de `contactId` (o quadrado novo, quando muda de sitio). */
    fun setSurface(contactId: String, surface: Surface?) {
        synchronized(receivers) {
            surfaces[contactId] = surface
            receivers[contactId]?.setSurface(surface)
        }
    }

    /**
     * O quadrado de `contactId` saiu do ecra. So conta se for o ecra ATUAL: quando alguem passa
     * para o quadrado grande, o novo pode chegar antes de o antigo sair (antes, o antigo apagava
     * o novo e o video dessa pessoa ficava preto).
     */
    fun clearSurface(contactId: String, surface: Surface) {
        synchronized(receivers) {
            if (surfaces[contactId] !== surface) return
            surfaces[contactId] = null
            receivers[contactId]?.setSurface(null)
        }
    }

    /**
     * DIAGNOSTICO para os testers (premir o tempo da chamada, como nas chamadas a dois): os numeros
     * de cada participante, com o nome. Sem IPs nem ids.
     */
    fun diagnostic(): String? {
        val a = audio ?: return null
        val g = _ui.value ?: return null
        if (g.tiles.isEmpty()) return CallDiag.withHeader(s(R.string.gcall_waiting))
        return CallDiag.withHeader(g.tiles.joinToString("\n\n") { t -> "== ${t.name} ==\n" + runCatching { a.diagnosticOf(t.contactId) }.getOrDefault("") })
    }

    /** A app saiu do ecra (ou voltou): a minha camara pausa sem avisar ninguem. */
    fun hold(on: Boolean) {
        audio?.videoHold(on)
    }

    // ------------------------------------------------------------ eventos

    /** Chamar depois de cada lote recebido e de segundo a segundo durante a chamada. */
    suspend fun onEvents(c: QtClient) {
        val events = try {
            SealerCore.groupCallEvents(c)
        } catch (e: Exception) {
            return
        }
        for (ev in events) {
            when (ev) {
                is GroupCallEvent.Incoming -> {
                    if (busy() || _ui.value != null) {
                        runCatching { c.groupCallLeave(ev.callId) }
                        continue
                    }
                    val h = if (ev.fromCall) inherit() else null
                    _ui.value = GroupCallUi(
                        ev.callId, ev.from, nameOf(ev.from), joined = false, video = ev.video,
                        fromName = nameOf(ev.from),
                        muted = h?.muted == true, speakerOn = h?.speaker ?: true,
                    )
                    CallControl.hangup = hangupFromNotification
                    ensureTicker()
                    // Vinha da chamada a dois com quem me adicionou: entro logo, sem tocar (com o
                    // aviso ja aceite; senao fica a tocar para o aceitar).
                    if (ev.fromCall && noticeAccepted()) {
                        join()
                        continue
                    }
                    sounds.ringIncoming()
                    if (!visible()) CallNotify.ringing(app, nameOf(ev.from), sound = false, video = ev.video)
                }
                is GroupCallEvent.Changed -> refresh(c)
                is GroupCallEvent.Ended -> {
                    val cur = _ui.value
                    if (cur != null && cur.callId == ev.callId) {
                        if (ev.reason == GroupCallEndReason.MISSED && !cur.joined) CallNotify.missed(app, cur.title)
                        end()
                    }
                }
                is GroupCallEvent.Ongoing -> updateOngoing(c, ev.contactId)
            }
        }
    }

    // ------------------------------------------------------------ por dentro

    /** Fica com o que a chamada a dois deixou (o servico e a chamada no sistema continuam). */
    private fun inherit(): CallHandover? = takeHandover()?.also { h ->
        systemId = h.systemId
        routes = h.routes
        route = h.route
    }

    /** O que dizer quando nao deu para entrar ou adicionar. */
    private fun joinError(e: Exception): String = when ((e as? QtException.InvalidInput)?.field) {
        "group_call_full" -> s(R.string.gcall_full)
        "call_id" -> s(R.string.t_a_chamada_ja_nao_esta_disponivel)
        "call_busy" -> s(R.string.t_ocupado_noutra_chamada)
        else -> s(R.string.t_erro, e.safeMsg())
    }

    private fun launch(block: suspend () -> Unit) {
        scope.launch(Dispatchers.IO) {
            try {
                block()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "chamada de grupo: ${e.javaClass.simpleName}")
                onError(s(R.string.t_erro, e.safeMsg()))
            }
        }
    }

    private fun pin(hex: String): ByteArray? {
        val h = hex.trim()
        if (h.length != 64) return null
        return ByteArray(32) { i -> h.substring(i * 2, i * 2 + 2).toInt(16).toByte() }
    }

    /** O microfone, o altifalante, a rede e (com a camara ligada) a camara. */
    private fun startMedia(c: QtClient, callId: String) {
        if (audio != null) return
        val main = Relays.main
        val mainPin = pin(main.pin) ?: run { Log.w(TAG, "sem a chave do relay"); return }
        val others = Relays.all.drop(1).mapNotNull { r -> pin(r.pin)?.let { CallRelayInfo(r.name, r.url, it) } }
        val a = c.groupCallAudioStart(callId, main.url, mainPin, others)
        audio = a
        a.setNetwork(VideoNet.current(app))
        stopNet = VideoNet.watch(app) { k -> audio?.setNetwork(k) }
        val g = _ui.value
        CallService.start(app, camera = g?.camera == true, video = g?.video == true)
        // O "Desligar" da notificacao tambem sai da chamada de grupo (antes nao fazia nada).
        CallControl.hangup = hangupFromNotification
        try {
            // Na passagem de uma chamada a dois, o sistema de chamadas continua com o som.
            val managed = systemId?.let { SystemCalls.manages(it) } == true
            io = CallAudioIO(app, a.pipe(), managed).also {
                it.start()
                it.muted = g?.muted == true
                it.setSpeaker(g?.speakerOn != false)
            }
            if (managed) systemId?.let { SystemCalls.refresh(it) }
        } catch (e: Exception) {
            onError(s(R.string.t_nao_foi_possivel_usar_o_microfone_2, e.safeMsg()))
        }
        syncCamera()
    }

    private fun syncCamera() {
        val a = audio ?: return
        val on = _ui.value?.camera == true
        if (on && sender == null) {
            sender = VideoSender(app, a.videoOut()).also { vs ->
                // Nunca um aviso no ecra sobre a camara (decisao do Nuno, 0.9.67): so o registo.
                vs.onError = { Log.w(TAG, "camara: falhou") }
                vs.onPreview = {
                    vs.previewSize?.let { size -> _ui.update { it?.copy(previewW = size.width, previewH = size.height) } }
                }
                vs.setPreview(preview)
                vs.start(_ui.value?.front != false)
            }
            CallService.update(app, camera = true)
        } else if (!on && sender != null) {
            sender?.stop()
            sender = null
            CallService.update(app, camera = false)
        }
    }

    /** Quem esta, quem fala e os descodificadores (um por camara ligada). */
    private fun refresh(c: QtClient) {
        val g = _ui.value ?: return
        val parts = runCatching { c.groupCallParticipants(g.callId) }.getOrDefault(emptyList())
        val a = audio
        val old = g.tiles.associateBy { it.contactId }
        val tiles = parts.map { p ->
            val prev = old[p.contactId]
            GroupTile(
                // Nos meus contactos: o nome que lhe dei. Senao: o nome que escolheu para a chamada.
                p.contactId, if (isContact(p.contactId)) nameOf(p.contactId) else p.name ?: nameOf(p.contactId), p.connected, p.camera,
                prev?.videoW ?: 0, prev?.videoH ?: 0, prev?.rotation ?: 0,
            )
        }
        _ui.update { it?.copy(tiles = tiles, speaker = a?.speaker(), meSpeaking = a?.meSpeaking() == true) }
        if (a == null) return
        synchronized(receivers) {
            val want = tiles.filter { it.camera && it.connected }.map { it.contactId }.toSet()
            for (id in receivers.keys - want) receivers.remove(id)?.stop()
            for (id in want - receivers.keys) {
                receivers[id] = VideoReceiver(a.videoIn(id)) { w, h, rot ->
                    _ui.update { u ->
                        u?.let { cur -> cur.copy(tiles = cur.tiles.map { t -> if (t.contactId == id) t.copy(videoW = w, videoH = h, rotation = rot) else t }) }
                    }
                }.also {
                    it.setSurface(surfaces[id])
                    it.start()
                }
            }
        }
    }

    private fun ensureTicker() {
        if (ticker?.isActive == true) return
        ticker = scope.launch(Dispatchers.IO) {
            while (isActive && _ui.value != null) {
                delay(500L)
                val c = client() ?: break
                try {
                    onEvents(c)
                    flush()
                    refresh(c)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.w(TAG, "ticker: ${e.javaClass.simpleName}")
                }
            }
        }
    }

    /** Para tudo (saida, fim do toque ou chamada perdida). */
    private fun end() {
        sounds.stop()
        CallNotify.clear(app)
        sender?.stop()
        sender = null
        synchronized(receivers) {
            receivers.values.forEach { it.stop() }
            receivers.clear()
            surfaces.clear()
        }
        io?.stop()
        io = null
        stopNet?.invoke()
        stopNet = null
        audio?.let { a ->
            if (CallDiag.enabled) diagnostic()?.let { CallDiag.log(TAG, "chamada de grupo terminada: " + it.replace("\n", " | ")) }
            a.destroy()
        }
        audio = null
        if (CallControl.hangup === hangupFromNotification) CallControl.hangup = null
        CallService.stop(app)
        // O microfone do sistema nunca fica silenciado depois da chamada.
        systemId?.let { id ->
            if (_ui.value?.muted == true) SystemCalls.setMuted(app, id, false)
            SystemCalls.end(id, CallEnd.LOCAL)
        }
        systemId = null
        ringingSystemId?.let { SystemCalls.end(it, CallEnd.REJECTED) }
        ringingSystemId = null
        routes = emptyList()
        route = null
        _ui.value = null
    }
}
