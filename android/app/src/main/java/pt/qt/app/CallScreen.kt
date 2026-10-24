package pt.qt.app

import android.content.Context
import android.graphics.SurfaceTexture
import android.os.SystemClock
import android.view.Surface
import android.view.TextureView
import android.view.WindowManager
import android.view.accessibility.AccessibilityManager
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.materialIcon
import androidx.compose.material.icons.materialPath
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalAccessibilityManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.delay
import uniffi.qt_ffi.VideoResponse

private val GREEN = Color(0xFF2E9E5B)
private val RED = Color(0xFFD64545)
private val SCRIM = Color(0x99000000)

/**
 * Icone da videochamada (camara de video, o do Material "Videocam", Apache 2.0). A app so
 * tem os icones base do Material, por isso fica desenhado aqui.
 */
val VideoCallIcon: ImageVector by lazy {
    materialIcon(name = "Filled.Videocam") {
        materialPath {
            moveTo(17.0f, 10.5f)
            verticalLineTo(7.0f)
            curveToRelative(0.0f, -0.55f, -0.45f, -1.0f, -1.0f, -1.0f)
            horizontalLineTo(4.0f)
            curveToRelative(-0.55f, 0.0f, -1.0f, 0.45f, -1.0f, 1.0f)
            verticalLineToRelative(10.0f)
            curveToRelative(0.0f, 0.55f, 0.45f, 1.0f, 1.0f, 1.0f)
            horizontalLineToRelative(12.0f)
            curveToRelative(0.55f, 0.0f, 1.0f, -0.45f, 1.0f, -1.0f)
            verticalLineToRelative(-3.5f)
            lineToRelative(4.0f, 4.0f)
            verticalLineToRelative(-11.0f)
            lineToRelative(-4.0f, 4.0f)
            close()
        }
    }
}

/** Fundo dos botoes por cima do video: preto a 65% (o branco por cima tem contraste mesmo com video claro). */
private val ON_VIDEO = Color(0xA6000000)
/** Botao redondo normal (claro e escuro: o fundo da chamada e sempre o gradiente escuro ou o video). */
private val ROUND_IDLE = Color(0x29FFFFFF)
/** Icone de um botao ligado (fundo branco). */
private val ICON_ON = Color(0xFF1C1B1F)
/** Com video, o topo e os controlos desaparecem ao fim disto sem toques (o Android pode pedir mais). */
private const val CONTROLS_HIDE_MS = 3_500L

/**
 * Ecra de chamada (a chamar, a tocar, a chegar, atendida, terminada). Desenho da 0.9.38: o esboco
 * do Nuno so serviu de referencia de layout; logo e cores sao os da app (tema 0.9.11, claro e
 * escuro), com o "Sealer" em Manrope Bold, parado.
 *
 *  * Com video: o video do outro enche o ecra e a minha camara fica num canto, com cantos
 *    arredondados. Enquanto o outro nao aceita, a minha camara enche o ecra com "A espera de
 *    [Nome]..." (o unico texto de estado). Video do outro parado: so a foto ou a inicial dele.
 *  * Topo: "Sealer" a esquerda, o tempo da chamada a direita.
 *  * Barra de controlos flutuante (microfone, altifalante, camara, trocar camara) e, por baixo,
 *    o desligar vermelho com o auscultador.
 *  * Com video, o topo e os controlos desaparecem ao fim de 3-4 s sem toques e voltam com um
 *    toque no ecra. So de voz (ou com o TalkBack ligado): sempre visiveis.
 *
 * FLAG_SECURE: nada deste ecra sai em capturas, gravacoes do ecra nem na lista de apps recentes.
 *
 * Ecra (0010): com video fica sempre aceso; so de voz, apaga-se ao encostar ao ouvido (sensor de
 * proximidade), nunca com video. A app fora do ecra: a minha camara pausa (MainViewModel).
 */
@Composable
fun CallScreen(
    call: CallUi,
    onAccept: () -> Unit,
    onHangup: () -> Unit,
    onMute: () -> Unit,
    onSpeaker: () -> Unit,
    /** Minimizar (volta-se pela barra no topo da app); `null` = sem botao (a chegar, terminada). */
    onMinimize: (() -> Unit)? = null,
    /** Video (`null` = sem video neste ecra). Desde a 0.9.42 tambem por cima do bloqueio do telemovel. */
    onCamera: (() -> Unit)? = null,
    onSwitchCamera: () -> Unit = {},
    onVideoRespond: ((VideoResponse) -> Unit)? = null,
    onSelfPreview: (SurfaceTexture?) -> Unit = {},
    onRemoteSurface: (Surface?) -> Unit = {},
) {
    val colors = LocalChatColors.current
    val status = callStatusText(call)
    val context = LocalContext.current
    val activity = context.findActivity()
    DisposableEffect(activity) {
        activity?.window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        onDispose { activity?.window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE) }
    }
    val v = call.video
    val active = call.phase == CallPhase.ACTIVE
    val videoAllowed = onCamera != null && active
    val showRemote = videoAllowed && v.showRemote
    // Videochamada minha a chamar ou a tocar: a minha camara ja esta no ecra (nada sai ainda).
    val calling = call.phase == CallPhase.CALLING || call.phase == CallPhase.RINGING_OUT
    val showSelf = onCamera != null && v.camera && (active || calling)
    val videoOn = showRemote || showSelf
    val request = videoAllowed && v.request && onVideoRespond != null
    // Ecra apagado ao ouvido so de voz (0010): nunca com video no ecra, com um pedido de video por
    // responder ou em altifalante. Com video, o ecra fica sempre aceso.
    CallScreenPower(proximity = (active || calling) && !videoOn && !request && !call.speaker, keepOn = videoOn)

    // Esconder o topo e os controlos: so com video e atendida, sem um pedido de video por
    // responder, e nunca com o TalkBack (o leitor de ecra tem de encontrar sempre os botoes).
    val touchExploration = remember(context) {
        (context.getSystemService(Context.ACCESSIBILITY_SERVICE) as? AccessibilityManager)?.isTouchExplorationEnabled == true
    }
    val autoHide = videoOn && active && !request && !touchExploration
    var controlsShown by remember { mutableStateOf(true) }
    var touches by remember { mutableIntStateOf(0) }
    val a11y = LocalAccessibilityManager.current
    val hideAfter = remember(a11y) {
        a11y?.calculateRecommendedTimeoutMillis(CONTROLS_HIDE_MS, containsIcons = true, containsControls = true) ?: CONTROLS_HIDE_MS
    }
    LaunchedEffect(autoHide, controlsShown, touches) {
        if (!autoHide) {
            controlsShown = true
        } else if (controlsShown) {
            delay(hideAfter)
            controlsShown = false
        }
    }
    val visible = controlsShown || !autoHide
    // Tocar num botao tambem conta como toque (os 3-4 s recomecam).
    val touched: (() -> Unit) -> (() -> Unit) = { f -> { touches++; f() } }
    val ink = if (videoOn) Color.White else colors.onTopBar

    Box(
        Modifier
            .fillMaxSize()
            .background(if (videoOn) Brush.verticalGradient(listOf(Color.Black, Color.Black)) else Brush.verticalGradient(colors.topBarGradient))
            .pointerInput(autoHide) {
                detectTapGestures(onTap = {
                    if (autoHide) {
                        controlsShown = !controlsShown
                        touches++
                    }
                })
            },
    ) {
        // Camadas do video (por baixo de tudo, ate as bordas do ecra).
        if (showRemote) {
            RemoteVideo(v, onRemoteSurface, Modifier.fillMaxSize())
            // O video dele parou (pausa, rede, sem fichas): so a foto ou a inicial, sem texto.
            if (v.remoteStalled) RemotePaused(call, colors.topBarGradient)
        }
        if (showSelf) {
            val shape = RoundedCornerShape(16.dp)
            val pip = Modifier
                .align(Alignment.TopEnd)
                .safeDrawingPadding()
                .padding(top = 68.dp, end = 16.dp)
                .size(104.dp, 156.dp)
                .clip(shape)
                .border(1.dp, Color.White.copy(alpha = 0.35f), shape)
            SelfVideo(v, onSelfPreview, if (showRemote) pip else Modifier.fillMaxSize())
        }
        // Sombras suaves por cima do video, para o topo e os controlos se lerem sempre.
        if (videoOn) {
            AnimatedVisibility(visible, Modifier.align(Alignment.TopCenter), enter = fadeIn(), exit = fadeOut()) {
                Box(Modifier.fillMaxWidth().height(150.dp).background(Brush.verticalGradient(listOf(Color(0x99000000), Color.Transparent))))
            }
            AnimatedVisibility(visible, Modifier.align(Alignment.BottomCenter), enter = fadeIn(), exit = fadeOut()) {
                Box(Modifier.fillMaxWidth().height(280.dp).background(Brush.verticalGradient(listOf(Color.Transparent, Color(0x99000000)))))
            }
        }
        Box(Modifier.fillMaxSize().safeDrawingPadding()) {
            // Topo: "Sealer" a esquerda, o tempo da chamada a direita.
            AnimatedVisibility(visible, Modifier.align(Alignment.TopCenter), enter = fadeIn(), exit = fadeOut()) {
                CallTopBar(onMinimize = onMinimize?.let(touched), time = if (active) status else null, tint = ink)
            }
            if (!videoOn) {
                // So de voz (ou a chegar): a foto, o nome e o estado, no meio.
                Column(
                    Modifier.align(Alignment.TopCenter).fillMaxWidth().padding(top = 112.dp, start = 24.dp, end = 24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Avatar(call.name, call.contactId, sizeDp = 120)
                    Spacer(Modifier.height(20.dp))
                    Text(
                        call.name, style = MaterialTheme.typography.headlineMedium, color = colors.onTopBar,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                    if (!active) {
                        Spacer(Modifier.height(8.dp))
                        Text(status, style = MaterialTheme.typography.titleMedium, color = colors.onTopBar.copy(alpha = 0.85f), textAlign = TextAlign.Center)
                    }
                    Spacer(Modifier.height(6.dp))
                    Text(s(R.string.t_cifrada_ponta_a_ponta), style = MaterialTheme.typography.labelMedium, color = colors.onTopBar.copy(alpha = 0.7f))
                }
            } else {
                // Com video: "A espera de [Nome]..." enquanto o outro nao aceita; a chamar, o nome e o estado.
                val banner = if (active) videoMessage(call) else "${call.name} · $status"
                banner?.let { VideoBanner(it, Modifier.align(Alignment.TopCenter).padding(top = 64.dp, start = 24.dp, end = 24.dp)) }
            }
            Column(
                Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 28.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                if (request) {
                    VideoRequest(call.name, onVideoRespond!!)
                    Spacer(Modifier.height(16.dp))
                }
                when (call.phase) {
                    CallPhase.INCOMING -> Row(horizontalArrangement = Arrangement.spacedBy(64.dp)) {
                        RoundAction(s(R.string.t_recusar), RED, Icons.Filled.Call, hangup = true, onClick = onHangup)
                        RoundAction(s(R.string.t_atender), GREEN, Icons.Filled.Call, hangup = false, onClick = onAccept)
                    }
                    CallPhase.ENDED -> Spacer(Modifier.height(1.dp))
                    else -> AnimatedVisibility(visible, enter = fadeIn(), exit = fadeOut()) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            ControlBar(
                                call = call,
                                onVideo = videoOn,
                                // Camara so se os dois lados tiverem video (app antiga: so voz).
                                camera = if (videoAllowed && v.supported && onCamera != null) touched(onCamera) else null,
                                onMute = touched(onMute),
                                onSpeaker = touched(onSpeaker),
                                onSwitchCamera = touched(onSwitchCamera),
                            )
                            Spacer(Modifier.height(20.dp))
                            HangupButton(touched(onHangup))
                        }
                    }
                }
            }
        }
    }
}

/**
 * Aceso com video (sem isto o Android apagava o ecra pelo tempo de inatividade) e apagado ao
 * encostar ao ouvido so de voz (sensor de proximidade). O sensor so conta com a app no ecra:
 * em segundo plano nunca apaga o ecra das outras apps.
 */
@Composable
private fun CallScreenPower(proximity: Boolean, keepOn: Boolean) {
    val view = LocalView.current
    DisposableEffect(view, keepOn) {
        if (keepOn) view.keepScreenOn = true
        onDispose { if (keepOn) view.keepScreenOn = false }
    }
    val context = LocalContext.current
    val owner = context.findActivity() as? LifecycleOwner
    var started by remember(owner) { mutableStateOf(owner?.lifecycle?.currentState?.isAtLeast(Lifecycle.State.STARTED) ?: true) }
    DisposableEffect(owner) {
        val obs = LifecycleEventObserver { _, e ->
            if (e == Lifecycle.Event.ON_START) started = true
            if (e == Lifecycle.Event.ON_STOP) started = false
        }
        owner?.lifecycle?.addObserver(obs)
        onDispose { owner?.lifecycle?.removeObserver(obs) }
    }
    val on = proximity && started
    DisposableEffect(on) {
        val lock = if (on) CallProximity.acquire(context.applicationContext) else null
        onDispose { lock?.let { CallProximity.release(it) } }
    }
}

/** Topo da chamada: (minimizar) logo e "Sealer" a esquerda, o tempo a direita. */
@Composable
private fun CallTopBar(onMinimize: (() -> Unit)?, time: String?, tint: Color) {
    Row(
        Modifier.fillMaxWidth().height(56.dp).padding(start = 4.dp, end = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onMinimize != null) {
            IconButton(onClick = onMinimize) {
                Icon(Icons.Filled.KeyboardArrowDown, contentDescription = s(R.string.call_minimize), tint = tint, modifier = Modifier.size(30.dp))
            }
        } else {
            Spacer(Modifier.width(16.dp))
        }
        BrandLogo(28)
        Spacer(Modifier.width(8.dp))
        Text("Sealer", fontFamily = ManropeBold, fontWeight = FontWeight.Bold, fontSize = 22.sp, color = tint, maxLines = 1)
        Spacer(Modifier.weight(1f))
        if (time != null) {
            Text(time, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = tint, maxLines = 1)
        }
    }
}

/**
 * Barra flutuante com os botoes redondos: microfone (silenciar), altifalante (ouvido ou
 * altifalante, nunca os auscultadores), camara (ligar/desligar) e trocar camara. `camera` nulo:
 * sem os dois botoes da camara (o outro lado nao tem video).
 */
@Composable
private fun ControlBar(
    call: CallUi,
    onVideo: Boolean,
    camera: (() -> Unit)?,
    onMute: () -> Unit,
    onSpeaker: () -> Unit,
    onSwitchCamera: () -> Unit,
) {
    val v = call.video
    Row(
        Modifier
            .clip(RoundedCornerShape(44.dp))
            .background(if (onVideo) ON_VIDEO else Color(0x1AFFFFFF))
            .padding(horizontal = 12.dp, vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ToggleRound(if (call.muted) MicOffIcon else MicIcon, s(R.string.t_silenciar), on = call.muted, onClick = onMute)
        ToggleRound(SpeakerIcon, s(R.string.t_altifalante), on = call.speaker, onClick = onSpeaker)
        if (camera != null) {
            ToggleRound(if (v.camera) VideoCallIcon else VideoOffIcon, s(R.string.call_camera), on = v.camera, onClick = camera)
            ActionRound(SwitchCameraIcon, s(R.string.video_switch), enabled = v.camera, onClick = onSwitchCamera)
        }
    }
}

/** Botao redondo de ligar/desligar: ligado = fundo branco e icone escuro. O TalkBack diz "ligado/desligado". */
@Composable
private fun ToggleRound(icon: ImageVector, label: String, on: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .size(56.dp)
            .clip(CircleShape)
            .background(if (on) Color.White else ROUND_IDLE)
            .toggleable(value = on, role = Role.Switch, onValueChange = { onClick() })
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = if (on) ICON_ON else Color.White, modifier = Modifier.size(26.dp))
    }
}

/** Botao redondo de uma acao (trocar camara). Desativado: mais transparente. */
@Composable
private fun ActionRound(icon: ImageVector, label: String, enabled: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .size(56.dp)
            .clip(CircleShape)
            .background(if (enabled) ROUND_IDLE else Color(0x14FFFFFF))
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = Color.White.copy(alpha = if (enabled) 1f else 0.45f), modifier = Modifier.size(26.dp))
    }
}

/** Desligar: botao redondo vermelho com o auscultador (o padrao que toda a gente reconhece). */
@Composable
private fun HangupButton(onClick: () -> Unit) {
    val label = s(R.string.t_desligar)
    Box(
        Modifier
            .size(72.dp)
            .clip(CircleShape)
            .background(RED)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Filled.Call, contentDescription = null, tint = Color.White, modifier = Modifier.size(34.dp).rotate(135f))
    }
}

/**
 * A mensagem do video no topo, se houver: so "A espera de [Nome]..." (o pedido da camara).
 * Nunca nada sobre rede, limites ou o caminho da chamada.
 */
@Composable
private fun videoMessage(call: CallUi): String? {
    val v = call.video
    return if (v.waiting) s(R.string.video_waiting, call.name) else null
}

/** O video do outro parou: tapa a ultima imagem com a foto ou a inicial dele, sem texto. */
@Composable
private fun RemotePaused(call: CallUi, gradient: List<Color>) {
    Box(Modifier.fillMaxSize().background(Brush.verticalGradient(gradient)), contentAlignment = Alignment.Center) {
        Avatar(call.name, call.contactId, sizeDp = 112)
    }
}

@Composable
private fun VideoBanner(text: String, modifier: Modifier = Modifier) {
    Text(
        text, color = Color.White, style = MaterialTheme.typography.labelLarge, textAlign = TextAlign.Center,
        maxLines = 2, overflow = TextOverflow.Ellipsis,
        modifier = modifier.clip(RoundedCornerShape(16.dp)).background(SCRIM).padding(horizontal = 14.dp, vertical = 8.dp),
    )
}

/** "[Nome] quer mostrar video": [Ver] [Ligar a minha camara] [Agora nao]. */
@Composable
private fun VideoRequest(name: String, onRespond: (VideoResponse) -> Unit) {
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(Color(0xCC000000)).padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(s(R.string.video_request_title, name), color = Color.White, style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
        Spacer(Modifier.height(4.dp))
        Button(onClick = { onRespond(VideoResponse.VIEW) }, modifier = Modifier.fillMaxWidth()) { Text(s(R.string.video_view)) }
        Button(onClick = { onRespond(VideoResponse.VIEW_AND_CAMERA) }, modifier = Modifier.fillMaxWidth()) { Text(s(R.string.video_view_and_camera)) }
        OutlinedButton(
            onClick = { onRespond(VideoResponse.NOT_NOW) }, modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.outlinedButtonColors(contentColor = Color.White),
        ) { Text(s(R.string.video_not_now)) }
    }
}

/** Ouvinte de uma TextureView: avisa quando a superficie aparece e desaparece. */
private fun textureListener(onReady: (SurfaceTexture?) -> Unit) = object : TextureView.SurfaceTextureListener {
    override fun onSurfaceTextureAvailable(st: SurfaceTexture, width: Int, height: Int) = onReady(st)
    override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, width: Int, height: Int) {}
    override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
        onReady(null)
        return true
    }
    override fun onSurfaceTextureUpdated(st: SurfaceTexture) {}
}

/** O video do outro (o descodificador desenha aqui), direito e a encher a vista. */
@Composable
private fun RemoteVideo(v: VideoUi, onSurface: (Surface?) -> Unit, modifier: Modifier) {
    val current by rememberUpdatedState(onSurface)
    AndroidView(
        factory = { ctx ->
            VideoTextureView(ctx).apply {
                surfaceTextureListener = textureListener { st -> current(st?.let { Surface(it) }) }
            }
        },
        update = { it.setFrame(v.remoteW, v.remoteH, v.remoteRotation, mirrored = false) },
        modifier = modifier,
    )
}

/**
 * A minha camara. A Camera2 ja roda a pre-visualizacao para o telemovel direito (e espelha a
 * da frente); aqui so se corrige o ecra deitado e o tamanho.
 */
@Composable
private fun SelfVideo(v: VideoUi, onPreview: (SurfaceTexture?) -> Unit, modifier: Modifier) {
    val current by rememberUpdatedState(onPreview)
    val view = LocalView.current
    val displayRotation = (view.display?.rotation ?: Surface.ROTATION_0) * 90
    AndroidView(
        factory = { ctx ->
            VideoTextureView(ctx).apply { surfaceTextureListener = textureListener { st -> current(st) } }
        },
        update = { it.setFrame(v.previewH, v.previewW, (360 - displayRotation) % 360, mirrored = false) },
        modifier = modifier,
    )
}

/** "A chamar", "A tocar", o tempo da chamada (atualizado de segundo a segundo) ou como acabou. */
@Composable
private fun callStatusText(call: CallUi): String {
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(call.phase, call.answeredAt) {
        while (call.phase == CallPhase.ACTIVE) {
            now = SystemClock.elapsedRealtime()
            delay(1_000L)
        }
    }
    return when (call.phase) {
        CallPhase.CALLING -> s(R.string.t_a_chamar)
        CallPhase.RINGING_OUT -> s(R.string.t_a_tocar)
        CallPhase.INCOMING -> if (call.videoCall) s(R.string.video_call_from, call.name) else s(R.string.t_chamada_de_voz)
        CallPhase.ACTIVE -> {
            val s = ((now - call.answeredAt).coerceAtLeast(0L) / 1000L)
            String.format(java.util.Locale.ROOT, "%d:%02d", s / 60, s % 60)
        }
        CallPhase.ENDED -> call.endText
    }
}

/**
 * Barra verde no topo da app com a chamada minimizada (0.9.29): nome, estado/tempo e
 * "toca para voltar". Ocupa tambem a barra de estado do telemovel.
 */
@Composable
fun CallBar(call: CallUi, onClick: () -> Unit) {
    val status = callStatusText(call)
    Row(
        Modifier
            .fillMaxWidth()
            .background(GREEN)
            .clickable(onClick = onClick)
            .windowInsetsPadding(WindowInsets.statusBars)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Filled.Call, contentDescription = null, tint = Color.White, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(
                "${call.name} · $status", color = Color.White, style = MaterialTheme.typography.labelLarge,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            Text(s(R.string.call_banner_back), color = Color.White.copy(alpha = 0.85f), style = MaterialTheme.typography.labelSmall, maxLines = 1)
        }
    }
}

@Composable
private fun RoundAction(label: String, color: Color, icon: ImageVector, hangup: Boolean, onClick: () -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier.size(72.dp).clip(CircleShape).background(color).clickable(onClick = onClick),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                icon, contentDescription = label, tint = Color.White,
                modifier = Modifier.size(32.dp).rotate(if (hangup) 135f else 0f),
            )
        }
        Spacer(Modifier.height(8.dp))
        Text(label, style = MaterialTheme.typography.labelLarge, color = Color.White)
    }
}
