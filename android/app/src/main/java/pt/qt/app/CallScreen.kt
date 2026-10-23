package pt.qt.app

import android.graphics.SurfaceTexture
import android.os.SystemClock
import android.view.Surface
import android.view.TextureView
import android.view.WindowManager
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.runtime.mutableLongStateOf
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
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

/**
 * Ecra de chamada (a chamar, a tocar, a chegar, atendida, terminada). Com video (fase 5b): o
 * video do outro enche o ecra e a minha camara fica num canto; enquanto o outro nao aceita,
 * a minha camara enche o ecra com "A espera de [Nome]...". Numa videochamada minha, a camara
 * ja aparece enquanto chama. FLAG_SECURE: nada deste ecra sai em capturas, gravacoes do ecra
 * nem na lista de apps recentes.
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
    /** Video (`null` = sem video neste ecra, ex.: por cima do bloqueio do telemovel). */
    onCamera: (() -> Unit)? = null,
    onSwitchCamera: () -> Unit = {},
    onVideoRespond: ((VideoResponse) -> Unit)? = null,
    onSelfPreview: (SurfaceTexture?) -> Unit = {},
    onRemoteSurface: (Surface?) -> Unit = {},
) {
    val colors = LocalChatColors.current
    val status = callStatusText(call)
    val activity = LocalContext.current.findActivity()
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
    Box(
        Modifier
            .fillMaxSize()
            .background(if (videoOn) Brush.verticalGradient(listOf(Color.Black, Color.Black)) else Brush.verticalGradient(colors.topBarGradient)),
    ) {
        // Camadas do video (por baixo de tudo, ate as bordas do ecra).
        if (showRemote) {
            RemoteVideo(v, onRemoteSurface, Modifier.fillMaxSize())
            // O video dele parou (pausa, rede, sem fichas): so a foto ou a inicial, sem texto.
            if (v.remoteStalled) RemotePaused(call, colors.topBarGradient)
        }
        if (showSelf) {
            val pip = Modifier
                .align(Alignment.TopEnd)
                .safeDrawingPadding()
                .padding(top = 56.dp, end = 12.dp)
                .size(108.dp, 160.dp)
                .clip(RoundedCornerShape(12.dp))
            SelfVideo(v, onSelfPreview, if (showRemote) pip else Modifier.fillMaxSize())
        }
        Box(Modifier.fillMaxSize().safeDrawingPadding()) {
            if (onMinimize != null) {
                IconButton(onClick = onMinimize, modifier = Modifier.align(Alignment.TopStart).padding(8.dp)) {
                    Icon(Icons.Filled.KeyboardArrowDown, contentDescription = s(R.string.call_minimize), tint = colors.onTopBar, modifier = Modifier.size(32.dp))
                }
            }
            Column(
                Modifier.fillMaxSize().padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                if (videoOn) {
                    // Com video: so o nome e o tempo, por cima da imagem.
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "${call.name} · $status", style = MaterialTheme.typography.titleMedium, color = Color.White,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.clip(RoundedCornerShape(16.dp)).background(SCRIM).padding(horizontal = 12.dp, vertical = 6.dp),
                    )
                } else {
                    Spacer(Modifier.height(48.dp))
                    Avatar(call.name, call.contactId, sizeDp = 112)
                    Spacer(Modifier.height(16.dp))
                    Text(
                        call.name, style = MaterialTheme.typography.headlineMedium, color = colors.onTopBar,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.height(8.dp))
                    Text(status, style = MaterialTheme.typography.titleMedium, color = colors.onTopBar.copy(alpha = 0.85f))
                    Spacer(Modifier.height(4.dp))
                    Text(s(R.string.t_cifrada_ponta_a_ponta), style = MaterialTheme.typography.labelSmall, color = colors.onTopBar.copy(alpha = 0.6f))
                }
                if (videoAllowed) {
                    Spacer(Modifier.height(12.dp))
                    videoMessage(call)?.let { VideoBanner(it) }
                }
                Spacer(Modifier.weight(1f))
                if (videoAllowed && v.request && onVideoRespond != null) {
                    VideoRequest(call.name, onVideoRespond)
                    Spacer(Modifier.height(16.dp))
                }
                when (call.phase) {
                    CallPhase.INCOMING -> Row(horizontalArrangement = Arrangement.spacedBy(56.dp)) {
                        RoundAction(s(R.string.t_recusar), RED, Icons.Filled.Call, hangup = true, onClick = onHangup)
                        RoundAction(s(R.string.t_atender), GREEN, Icons.Filled.Call, hangup = false, onClick = onAccept)
                    }
                    CallPhase.ENDED -> Spacer(Modifier.height(1.dp))
                    else -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        val content = if (videoOn) Color.White else colors.onTopBar
                        val btn = ButtonDefaults.outlinedButtonColors(contentColor = content, containerColor = if (videoOn) SCRIM else Color.Transparent)
                        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                            OutlinedButton(onClick = onMute, colors = btn) { Text(if (call.muted) s(R.string.t_ligar_o_microfone) else s(R.string.t_silenciar)) }
                            OutlinedButton(onClick = onSpeaker, colors = btn) { Text(if (call.speaker) s(R.string.t_auricular) else s(R.string.t_altifalante)) }
                        }
                        // Botao da camara so se os dois lados tiverem video (app antiga: so voz).
                        if (videoAllowed && v.supported && onCamera != null) {
                            Spacer(Modifier.height(12.dp))
                            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                                OutlinedButton(onClick = onCamera, colors = btn) {
                                    Text(if (v.camera) s(R.string.video_camera_off) else s(R.string.video_camera_on))
                                }
                                if (v.camera) {
                                    OutlinedButton(onClick = onSwitchCamera, colors = btn) { Text(s(R.string.video_switch)) }
                                }
                            }
                        }
                        Spacer(Modifier.height(28.dp))
                        RoundAction(s(R.string.t_desligar), RED, Icons.Filled.Call, hangup = true, onClick = onHangup)
                    }
                }
                Spacer(Modifier.height(32.dp))
            }
        }
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
private fun VideoBanner(text: String) {
    Text(
        text, color = Color.White, style = MaterialTheme.typography.labelLarge, textAlign = TextAlign.Center,
        modifier = Modifier.clip(RoundedCornerShape(16.dp)).background(SCRIM).padding(horizontal = 14.dp, vertical = 8.dp),
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
