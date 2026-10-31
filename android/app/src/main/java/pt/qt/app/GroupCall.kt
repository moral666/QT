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

/**
 * Chamadas de GRUPO no Android (0.9.63): ate 5 pessoas, voz e video. O nucleo (Rust,
 * `group_call.rs`) faz uma malha de chamadas a dois; aqui ha so o microfone e o altifalante (a
 * mistura vem pronta do Rust), a camara (uma so codificacao para todos), um descodificador por
 * participante com a camara ligada, o toque e o ecra.
 *
 * Antes da primeira chamada de grupo, a app mostra o aviso do IP com "Aceitar" (sem isso o Rust
 * recusa ligar ou entrar). Durante a chamada nao aparece nada sobre isto.
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

data class GroupCallUi(
    val callId: String,
    val groupId: String,
    val groupName: String,
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
    private val groupNameOf: (String) -> String,
    private val sounds: CallSounds,
    /** Ha uma chamada a dois no ecra (o convite e recusado em silencio). */
    private val busy: () -> Boolean,
    private val visible: () -> Boolean,
    private val onError: (String) -> Unit,
    /**
     * Cria as sessoes cifradas que faltam com os membros do grupo (0.9.67), como antes de enviar
     * uma mensagem ao grupo. Sem sessao, duas pessoas que nao sao contactos (e nunca escreveram no
     * grupo) nao tinham ligacao entre si: nao se viam nem se ouviam.
     */
    private val ensureGroupSessions: suspend (String) -> Unit = {},
    /** Esta nos meus contactos (entao mostra-se o nome que eu lhe dei, nao o da chamada). */
    private val isContact: (String) -> Boolean = { false },
) {
    private val prefs = app.getSharedPreferences("sealer_group_call", android.content.Context.MODE_PRIVATE)

    /** O nome que escolhi para as chamadas de grupo (0.9.67; so neste telemovel). */
    var myName: String
        get() = prefs.getString("name", "").orEmpty()
        set(v) {
            prefs.edit().putString("name", v.trim().take(32)).apply()
        }

    /** Envia o meu nome para esta chamada (quem nao me tem nos contactos ve-o). */
    private fun sendName(c: QtClient, callId: String) {
        val n = myName
        if (n.isNotBlank()) runCatching { c.groupCallSetName(callId, n) }
    }

    private val _ui = MutableStateFlow<GroupCallUi?>(null)
    val ui: StateFlow<GroupCallUi?> = _ui.asStateFlow()

    /** Chamadas a decorrer em grupos onde nao estou (recusei ou nao atendi): grupo -> chamada. */
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

    fun noticeAccepted(): Boolean = runCatching { client()?.groupCallNoticeAccepted() }.getOrNull() == true

    fun acceptNotice() {
        runCatching { client()?.groupCallAcceptNotice() }
    }

    // ------------------------------------------------------------ acoes

    /** LIGAR ao grupo (com o aviso ja aceite e as permissoes dadas). */
    fun start(groupId: String, video: Boolean) = launch {
        val c = client() ?: return@launch
        if (_ui.value != null || busy()) return@launch
        runCatching { ensureGroupSessions(groupId) }
        val st = c.groupCallStart(groupId, video)
        _ui.value = GroupCallUi(
            st.callId, groupId, groupNameOf(groupId), joined = true, video = video, camera = video,
            startedAt = SystemClock.elapsedRealtime(),
        )
        sendName(c, st.callId)
        flush()
        startMedia(c, st.callId)
        ensureTicker()
    }

    /** ENTRAR na chamada que esta a tocar. */
    fun join() = launch {
        val c = client() ?: return@launch
        val g = _ui.value?.takeIf { !it.joined } ?: return@launch
        sounds.stop()
        CallNotify.clear(app)
        try {
            runCatching { ensureGroupSessions(g.groupId) }
            c.groupCallJoin(g.callId)
        } catch (e: Exception) {
            // Cheia (5 pessoas) ou ja acabou.
            onError(s(R.string.gcall_full))
            end()
            return@launch
        }
        _ui.update { it?.copy(joined = true, camera = g.video, startedAt = SystemClock.elapsedRealtime()) }
        sendName(c, g.callId)
        flush()
        startMedia(c, g.callId)
        refresh(c)
    }

    /** ENTRAR DEPOIS numa chamada que recusei (ou nao atendi): o botao na conversa do grupo. */
    fun rejoin(groupId: String) = launch {
        val c = client() ?: return@launch
        val id = _ongoing.value[groupId] ?: return@launch
        if (_ui.value != null || busy()) return@launch
        try {
            runCatching { ensureGroupSessions(groupId) }
            c.groupCallRejoin(id)
        } catch (e: Exception) {
            onError(s(R.string.gcall_full))
            updateOngoing(c, groupId)
            return@launch
        }
        val video = runCatching { c.groupCallIsVideo(id) }.getOrDefault(false)
        _ui.value = GroupCallUi(
            id, groupId, groupNameOf(groupId), joined = true, video = video, camera = video,
            startedAt = SystemClock.elapsedRealtime(),
        )
        updateOngoing(c, groupId)
        sendName(c, id)
        flush()
        startMedia(c, id)
        refresh(c)
        ensureTicker()
    }

    private fun updateOngoing(c: QtClient, groupId: String) {
        val id = runCatching { c.groupCallOngoing(groupId) }.getOrNull()
        _ongoing.update { m -> if (id == null) m - groupId else m + (groupId to id) }
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
    }

    fun toggleSpeaker() {
        val on = !(_ui.value?.speakerOn ?: return)
        io?.setSpeaker(on)
        _ui.update { it?.copy(speakerOn = on) }
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
            c.groupCallEvents()
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
                    _ui.value = GroupCallUi(
                        ev.callId, ev.groupId, groupNameOf(ev.groupId), joined = false, video = ev.video,
                        fromName = nameOf(ev.from),
                    )
                    sounds.ringIncoming()
                    if (!visible()) CallNotify.ringing(app, groupNameOf(ev.groupId), sound = false, video = ev.video)
                    ensureTicker()
                }
                is GroupCallEvent.Changed -> refresh(c)
                is GroupCallEvent.Ended -> {
                    val cur = _ui.value
                    if (cur != null && cur.callId == ev.callId) {
                        if (ev.reason == GroupCallEndReason.MISSED && !cur.joined) CallNotify.missed(app, cur.groupName)
                        end()
                    }
                    updateOngoing(c, ev.groupId)
                }
                is GroupCallEvent.Ongoing -> updateOngoing(c, ev.groupId)
            }
        }
    }

    // ------------------------------------------------------------ por dentro

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
            io = CallAudioIO(app, a.pipe()).also {
                it.start()
                it.muted = g?.muted == true
                it.setSpeaker(g?.speakerOn != false)
            }
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
        _ui.value = null
    }
}
