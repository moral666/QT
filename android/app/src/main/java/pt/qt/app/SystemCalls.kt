package pt.qt.app

import android.content.Context
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.SystemClock
import android.telecom.DisconnectCause
import android.util.Log
import androidx.annotation.RequiresApi
import androidx.core.telecom.CallAttributesCompat
import androidx.core.telecom.CallControlResult
import androidx.core.telecom.CallControlScope
import androidx.core.telecom.CallEndpointCompat
import androidx.core.telecom.CallsManager
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

/**
 * As chamadas do Sealer no sistema de chamadas do Android (0.9.52), como o CallKit no iPhone:
 * auricular Bluetooth e carro (botoes de atender e desligar, o som vai para la), e uma chamada
 * normal ao mesmo tempo sem dois microfones em conflito.
 *
 * Usa o Jetpack Core-Telecom (chamadas "self-managed"): ConnectionService no Android 8 a 13 e a
 * API transacional no 14+. So no Android 8+ (API 26); no 7 tudo fica como antes. Se o sistema
 * recusar o registo, a app tambem segue como antes (sem Telecom).
 *
 * O audio, a cifra e a rede continuam no Rust (`CallAudio`): aqui so o ciclo da chamada e o
 * caminho do som. O toque proprio e a notificacao de ecra inteiro continuam (CallSounds, CallNotify).
 *
 * Privacidade (iguais ao iPhone):
 *  * Nada vai para o registo de chamadas do telemovel: as chamadas self-managed nao entram por
 *    omissao e a app nunca pede o contrario (EXTRA_LOG_SELF_MANAGED_CALLS).
 *  * O endereco dado ao sistema e sempre o mesmo, "sealer:SEALER". Nunca o ID do contacto.
 *  * O nome para o sistema, o carro e o Bluetooth e a alcunha do contacto; com o bloqueio da
 *    app ativo, ou sem nome, "Chamada SEALER" / "Videochamada SEALER".
 *  * Sem chamadas em espera: uma chamada GSM atendida por cima acaba a do Sealer.
 *
 * Cada chamada tem uma chave propria (`key`). A chamada a chegar com a app fechada (SealerCore)
 * e a mesma entrada quando o ecra fica com ela: uma so entrada no sistema por chamada. Com o
 * bloqueio da app nao se sabe o callId ao tocar: a entrada fica "sem callId" ate o ecra decifrar
 * a oferta e a ligar ([incoming]).
 */
object SystemCalls {
    private const val TAG = "SealerTelecom"
    /** O sistema tem este tempo para aceitar uma chamada nova (senao segue sem ele). */
    private const val ADD_TIMEOUT_MS = 5_000L
    private const val ACTION_TIMEOUT_MS = 3_000L
    /** O Telecom poe o modo de chamada pouco depois de atender: espera-se por ele antes do microfone. */
    private const val MODE_WAIT_MS = 600L
    /** Uma chamada a chegar que ninguem atendeu nem acabou sai do sistema ao fim disto. */
    private const val RING_LIMIT_MS = SealerCore.RING_MS + 15_000L

    /** O mesmo endereco para todas as chamadas (como o CXHandle "SEALER" do iPhone). */
    private val ADDRESS: Uri = Uri.fromParts("sealer", "SEALER", null)

    val supported: Boolean get() = Build.VERSION.SDK_INT >= 26

    /** O ecra (MainViewModel) com a chamada: atender, desligar, silenciar e saidas do som vindos do sistema. */
    interface Listener {
        /** O ecra mostra esta chamada (e ela ainda nao acabou). */
        fun owns(callId: String): Boolean
        fun onSystemAnswer(callId: String)
        fun onSystemHangup(callId: String)
        fun onSystemMuted(callId: String, muted: Boolean)
        fun onRoutes(callId: String, routes: List<AudioRoute>, current: AudioRoute?)
    }

    @Volatile var listener: Listener? = null

    private val scope = CoroutineScope(
        SupervisorJob() + Dispatchers.Default +
            CoroutineExceptionHandler { _, e -> Log.w(TAG, "sistema de chamadas: ${e.javaClass.simpleName}") },
    )

    private sealed class Action {
        class Answer(val done: CompletableDeferred<Boolean>) : Action()
        class Active(val done: CompletableDeferred<Boolean>) : Action()
        class Route(val id: String) : Action()
        /** `bySystem`: o sistema ja a desligou (nao se pede outra vez). */
        class End(val cause: Int, val bySystem: Boolean) : Action()
    }

    private class Entry(val key: String, val incoming: Boolean, val video: Boolean) {
        @Volatile var callId: String? = null
        /** O sistema aceitou a chamada (dentro do addCall). */
        @Volatile var live = false
        @Volatile var answered = false
        val ended = AtomicBoolean(false)
        /** Acabou antes de o sistema a aceitar: desliga logo que entrar. */
        @Volatile var endBeforeLive: Int? = null
        val added = CompletableDeferred<Boolean>()
        /** A chamada saiu do sistema ([finish]). */
        val over = CompletableDeferred<Unit>()
        val actions = Channel<Action>(Channel.UNLIMITED)
        @Volatile var endpoints: List<CallEndpointCompat> = emptyList()
        @Volatile var current: CallEndpointCompat? = null
        @Volatile var job: Job? = null
        /** O trabalho desta chamada (saidas do som, silencio, pedidos): acaba com ela. */
        val work = SupervisorJob(scope.coroutineContext[Job])
        val workScope = CoroutineScope(scope.coroutineContext + work)
    }

    private val entries = ConcurrentHashMap<String, Entry>()
    private val lock = Any()
    @Volatile private var manager: CallsManager? = null
    @Volatile private var failed = false
    /** Uma chamada a chegar sem callId (bloqueio da app) foi recusada no sistema (auricular, carro). */
    @Volatile private var declinedUnboundAt = 0L

    fun newKey(): String = UUID.randomUUID().toString()

    @RequiresApi(26)
    private fun manager(ctx: Context): CallsManager? {
        if (failed) return null
        manager?.let { return it }
        synchronized(lock) {
            manager?.let { return it }
            return try {
                CallsManager(ctx.applicationContext).also {
                    it.registerAppWithTelecom(
                        CallsManager.CAPABILITY_BASELINE or CallsManager.CAPABILITY_SUPPORTS_VIDEO_CALLING,
                    )
                    manager = it
                }
            } catch (e: Exception) {
                // Sem o sistema: as chamadas continuam como antes da 0.9.52.
                Log.w(TAG, "registo no sistema de chamadas: ${e.javaClass.simpleName}")
                failed = true
                null
            }
        }
    }

    private fun byCall(callId: String): Entry? = entries.values.firstOrNull { it.callId == callId && !it.ended.get() }

    /** O sistema tem esta chamada (o caminho do som e o modo de chamada sao dele). */
    fun manages(callId: String): Boolean = byCall(callId)?.live == true

    /**
     * O telemovel ja esta noutra chamada (GSM ou outra app de chamadas): a chamada do Sealer a
     * chegar toca sem o toque alto por cima da conversa.
     */
    fun phoneBusy(ctx: Context): Boolean {
        val am = ctx.getSystemService(Context.AUDIO_SERVICE) as? AudioManager ?: return false
        val mode = am.mode
        return mode == AudioManager.MODE_IN_CALL || mode == AudioManager.MODE_IN_COMMUNICATION
    }

    // ------------------------------------------------------------------ do Sealer para o sistema

    /**
     * Chamada a chegar. `key`: a do toque com a app fechada (SealerCore.Ringing), ou null (ecra).
     * `callId` null: bloqueio da app (so se sabe que e uma chamada). Ja existe: so liga o callId.
     */
    fun incoming(ctx: Context, key: String?, callId: String?, name: String?, video: Boolean) {
        if (Build.VERSION.SDK_INT < 26) return
        synchronized(lock) {
            if (callId != null && byCall(callId) != null) return
            val same = key?.let { entries[it] }?.takeIf { !it.ended.get() }
            if (same != null) {
                if (callId != null && same.callId == null) bind(same, callId)
                return
            }
            if (callId != null) {
                // O toque com o bloqueio da app (sem callId) e esta chamada: a mesma entrada.
                val unbound = entries.values.firstOrNull { it.incoming && it.callId == null && !it.ended.get() }
                if (unbound != null) {
                    bind(unbound, callId)
                    return
                }
            }
            val e = Entry(key ?: newKey(), incoming = true, video = video)
            e.callId = callId
            entries[e.key] = e
            start(ctx, e, name)
        }
    }

    /**
     * LIGAR: a chamada entra no sistema antes de sair. `true`: aceite. `false`: o sistema recusou
     * (o telemovel esta noutra chamada). `null`: sem o sistema (Android 7, registo recusado ou
     * sem resposta a tempo): segue como antes.
     */
    suspend fun outgoing(ctx: Context, key: String, name: String?, video: Boolean): Boolean? {
        if (Build.VERSION.SDK_INT < 26) return null
        if (manager(ctx) == null) return null
        val e = Entry(key, incoming = false, video = video)
        entries[key] = e
        start(ctx, e, name)
        val ok = withTimeoutOrNull(ADD_TIMEOUT_MS) { e.added.await() }
        if (ok == null) {
            Log.w(TAG, "o sistema nao respondeu a tempo: chamada sem ele")
            drop(e, DisconnectCause.LOCAL)
            return null
        }
        if (ok) return true
        // Recusada: so conta como "ocupado" se o telemovel estiver mesmo noutra chamada; outro
        // erro do sistema nunca impede de ligar (segue sem ele).
        return if (phoneBusy(ctx)) false else null
    }

    /** A chamada que saiu (ou chegou) ja tem callId do Rust. */
    fun bind(key: String, callId: String) {
        val e = entries[key] ?: return
        bind(e, callId)
    }

    private fun bind(e: Entry, callId: String) {
        if (e.callId == null) e.callId = callId
        publish(e)
    }

    /** Atendeu-se na app: diz ao sistema (o modo de chamada e o caminho do som passam a ser dele). */
    suspend fun answer(callId: String) {
        val e = byCall(callId) ?: return
        if (!e.live || e.answered) return
        val done = CompletableDeferred<Boolean>()
        e.actions.trySend(Action.Answer(done))
        withTimeoutOrNull(ACTION_TIMEOUT_MS) { done.await() }
    }

    /** O outro lado atendeu a chamada que fiz. */
    suspend fun setActive(callId: String) {
        val e = byCall(callId) ?: return
        if (!e.live) return
        val done = CompletableDeferred<Boolean>()
        e.actions.trySend(Action.Active(done))
        withTimeoutOrNull(ACTION_TIMEOUT_MS) { done.await() }
    }

    /** A chamada acabou (pelo callId ou pela chave). Ja acabada ou desconhecida: nada. */
    fun end(idOrKey: String?, how: CallEnd) {
        if (idOrKey == null) return
        val e = entries[idOrKey] ?: entries.values.firstOrNull { it.callId == idOrKey } ?: return
        if (!e.ended.compareAndSet(false, true)) return
        val cause = cause(how)
        if (e.live) {
            e.actions.trySend(Action.End(cause, bySystem = false))
        } else {
            e.endBeforeLive = cause
            drop(e, cause)
        }
    }

    /** Escolheu-se uma saida do som (o `id` de [AudioRoute]). */
    fun setRoute(callId: String, routeId: String) {
        val e = byCall(callId) ?: return
        if (e.live) e.actions.trySend(Action.Route(routeId))
    }

    /**
     * Silenciar na app tambem silencia o microfone do sistema (o carro e o auricular ficam
     * certos). O Telecom acompanha o silencio do microfone e repoe-no no fim da chamada.
     */
    fun setMuted(ctx: Context, callId: String, muted: Boolean) {
        if (!manages(callId)) return
        val am = ctx.getSystemService(Context.AUDIO_SERVICE) as? AudioManager ?: return
        runCatching { if (am.isMicrophoneMute != muted) am.isMicrophoneMute = muted }
    }

    /** Volta a mandar as saidas do som ao ecra (depois de ele ficar com a chamada). */
    fun refresh(callId: String) {
        byCall(callId)?.let { publish(it) }
    }

    /**
     * O ecra ja tem uma chamada a chegar sem callId que o sistema recusou (auricular, carro):
     * a oferta que chegar agora e essa e tambem e recusada. Le e limpa.
     */
    fun consumeDeclinedUnbound(): Boolean {
        val t = declinedUnboundAt
        if (t == 0L) return false
        declinedUnboundAt = 0L
        return SystemClock.elapsedRealtime() - t < SealerCore.RING_MS
    }

    /**
     * Antes de ligar o microfone de uma chamada do sistema: o modo de chamada e do Telecom e chega
     * pouco depois de atender. Criar o microfone antes disso para-o ~0,5 s em muitos telemoveis.
     */
    fun awaitCommMode(ctx: Context) {
        val am = ctx.getSystemService(Context.AUDIO_SERVICE) as? AudioManager ?: return
        val until = SystemClock.elapsedRealtime() + MODE_WAIT_MS
        while (am.mode != AudioManager.MODE_IN_COMMUNICATION && SystemClock.elapsedRealtime() < until) {
            SystemClock.sleep(20)
        }
    }

    // ------------------------------------------------------------------ interno

    private fun cause(how: CallEnd): Int = when (how) {
        CallEnd.LOCAL -> DisconnectCause.LOCAL
        CallEnd.REMOTE -> DisconnectCause.REMOTE
        CallEnd.REJECTED -> DisconnectCause.REJECTED
        CallEnd.BUSY -> DisconnectCause.BUSY
        CallEnd.MISSED -> DisconnectCause.MISSED
        CallEnd.CANCELED -> DisconnectCause.CANCELED
        CallEnd.ERROR -> DisconnectCause.ERROR
    }

    private fun displayName(name: String?, video: Boolean): String =
        name?.takeIf { it.isNotBlank() } ?: s(if (video) R.string.video_call_sealer else R.string.t_chamada_sealer)

    @RequiresApi(26)
    private fun start(ctx: Context, e: Entry, name: String?) {
        val cm = manager(ctx)
        if (cm == null) {
            e.ended.set(true)
            entries.remove(e.key)
            e.added.complete(false)
            return
        }
        val app = ctx.applicationContext
        val attrs = CallAttributesCompat(
            displayName = displayName(name, e.video),
            address = ADDRESS,
            direction = if (e.incoming) CallAttributesCompat.DIRECTION_INCOMING else CallAttributesCompat.DIRECTION_OUTGOING,
            callType = if (e.video) CallAttributesCompat.CALL_TYPE_VIDEO_CALL else CallAttributesCompat.CALL_TYPE_AUDIO_CALL,
            // Sem SUPPORTS_SET_INACTIVE: sem chamadas em espera (como o supportsHolding = false do iPhone).
            callCapabilities = 0,
        )
        e.job = scope.launch {
            try {
                cm.addCall(
                    attrs,
                    { _ -> systemAnswer(app, e) },
                    { _ -> systemEnd(app, e) },
                    { },
                    // Por em espera (ex.: atendeu-se uma chamada GSM por cima): sem espera, acaba.
                    { systemEnd(app, e) },
                ) {
                    control(this, e)
                }
                // Fica vivo ate a chamada acabar: o sistema nunca a da por terminada so porque o
                // bloco acima ja devolveu.
                e.over.await()
            } catch (c: CancellationException) {
                throw c
            } catch (x: Exception) {
                Log.w(TAG, "o sistema recusou a chamada: ${x.javaClass.simpleName}")
                e.ended.set(true)
                entries.remove(e.key)
                e.actions.close()
                e.work.cancel()
                e.added.complete(false)
            }
        }
        if (e.incoming) {
            scope.launch {
                delay(RING_LIMIT_MS)
                if (!e.answered) end(e.key, CallEnd.MISSED)
            }
        }
    }

    /** Dentro do sistema: as saidas do som, o silencio e os pedidos da app, pela ordem. */
    @RequiresApi(26)
    private fun control(cs: CallControlScope, e: Entry) {
        e.live = true
        e.added.complete(true)
        val w = e.workScope
        e.endBeforeLive?.let { cause ->
            // Acabou enquanto o sistema a aceitava: desliga ja.
            scope.launch { runCatching { cs.disconnect(DisconnectCause(cause)) } }
            finish(e)
            return
        }
        w.launch { cs.availableEndpoints.collect { e.endpoints = it; publish(e) } }
        w.launch { cs.currentCallEndpoint.collect { e.current = it; publish(e) } }
        w.launch {
            cs.isMuted.collect { m ->
                val id = e.callId ?: return@collect
                listener?.takeIf { it.owns(id) }?.onSystemMuted(id, m)
            }
        }
        // Os pedidos fora do trabalho da chamada: o ultimo (desligar) nunca e cancelado a meio.
        scope.launch {
            for (a in e.actions) {
                when (a) {
                    is Action.Answer -> {
                        val type = if (e.video) CallAttributesCompat.CALL_TYPE_VIDEO_CALL else CallAttributesCompat.CALL_TYPE_AUDIO_CALL
                        val ok = runCatching { cs.answer(type) is CallControlResult.Success }.getOrDefault(false)
                        if (ok) e.answered = true
                        a.done.complete(ok)
                    }
                    is Action.Active -> {
                        val ok = runCatching { cs.setActive() is CallControlResult.Success }.getOrDefault(false)
                        a.done.complete(ok)
                    }
                    is Action.Route -> {
                        val ep = e.endpoints.firstOrNull { it.identifier.toString() == a.id }
                        if (ep != null) runCatching { cs.requestEndpointChange(ep) }
                    }
                    is Action.End -> {
                        if (!a.bySystem) runCatching { cs.disconnect(DisconnectCause(a.cause)) }
                        finish(e)
                        break
                    }
                }
            }
        }
    }

    private fun finish(e: Entry) {
        entries.remove(e.key)
        e.actions.close()
        e.work.cancel()
        e.over.complete(Unit)
    }

    /** Cancela uma chamada que o sistema ainda nao tinha aceitado. */
    private fun drop(e: Entry, cause: Int) {
        e.ended.set(true)
        if (e.live) {
            e.actions.trySend(Action.End(cause, bySystem = false))
        } else {
            e.job?.cancel()
            finish(e)
        }
    }

    /** Atender vindo do sistema (auricular, carro, relogio). */
    private fun systemAnswer(app: Context, e: Entry) {
        e.answered = true
        val id = e.callId
        val l = listener
        if (id != null && l != null && l.owns(id)) {
            l.onSystemAnswer(id)
        } else {
            SealerCore.answerFromSystem(app, e.key, id)
        }
    }

    /** Recusar/desligar vindo do sistema (auricular, carro, ou uma chamada GSM atendida por cima). */
    private fun systemEnd(app: Context, e: Entry) {
        if (!e.ended.compareAndSet(false, true)) return
        e.actions.trySend(Action.End(DisconnectCause.LOCAL, bySystem = true))
        val id = e.callId
        val l = listener
        when {
            id != null && l != null && l.owns(id) -> l.onSystemHangup(id)
            else -> {
                if (id == null && !e.answered) declinedUnboundAt = SystemClock.elapsedRealtime()
                SealerCore.declineFromSystem(app, e.key)
            }
        }
    }

    private fun publish(e: Entry) {
        if (Build.VERSION.SDK_INT < 26) return
        val id = e.callId ?: return
        val l = listener ?: return
        if (!l.owns(id)) return
        l.onRoutes(id, e.endpoints.map(::route), e.current?.let(::route))
    }

    @RequiresApi(26)
    private fun route(ep: CallEndpointCompat): AudioRoute {
        val kind = when (ep.type) {
            CallEndpointCompat.TYPE_EARPIECE -> RouteKind.PHONE
            CallEndpointCompat.TYPE_SPEAKER -> RouteKind.SPEAKER
            CallEndpointCompat.TYPE_BLUETOOTH -> RouteKind.BLUETOOTH
            CallEndpointCompat.TYPE_WIRED_HEADSET -> RouteKind.WIRED
            else -> RouteKind.OTHER
        }
        val name = when (kind) {
            RouteKind.PHONE -> s(R.string.call_route_phone)
            RouteKind.SPEAKER -> s(R.string.t_altifalante)
            RouteKind.WIRED -> s(R.string.call_route_headphones)
            else -> ep.name.toString().ifBlank { "Bluetooth" }
        }
        return AudioRoute(ep.identifier.toString(), kind, name)
    }
}
