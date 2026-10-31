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
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.materialIcon
import androidx.compose.material.icons.materialPath
import androidx.compose.material3.AlertDialog
import kotlinx.coroutines.delay
import androidx.compose.ui.draw.alpha
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
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

/**
 * Ecra da chamada de GRUPO (0.9.63). Layout do esboco do Nuno (so como referencia; cores e logo
 * da app): a minha camara no quadrado grande em cima (desde a 0.9.66) e os outros numa grelha de
 * 2 por baixo, cada um com o nome e o icone do microfone ou da camara. Com video, o topo e os
 * botoes desaparecem ao fim de uns segundos sem toques (como na videochamada a dois); um toque
 * mostra-os outra vez.
 */
@Composable
fun GroupCallScreen(
    g: GroupCallUi,
    myName: String,
    myId: String,
    onJoin: () -> Unit,
    onLeave: () -> Unit,
    onMute: () -> Unit,
    onSpeaker: () -> Unit,
    onCamera: () -> Unit,
    onSwitchCamera: () -> Unit,
    onSelfPreview: (SurfaceTexture?) -> Unit,
    onSurface: (String, Surface?) -> Unit,
    onClearSurface: (String, Surface) -> Unit,
    /** Testers (`CallDiag`): premir o tempo da chamada abre os numeros. `null` = desligado. */
    diagnostic: (() -> String?)? = null,
) {
    val colors = LocalChatColors.current
    val context = LocalContext.current
    var showDiag by remember { mutableStateOf(false) }
    if (showDiag && diagnostic != null) CallDiagDialog(diagnostic) { showDiag = false }
    val activity = context.findActivity()
    DisposableEffect(activity) {
        activity?.window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        onDispose { activity?.window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE) }
    }
    val anyVideo = g.joined && (g.camera || g.tiles.any { it.camera && it.connected })
    CallScreenPower(proximity = false, keepOn = true)
    val touchExploration = remember(context) {
        (context.getSystemService(Context.ACCESSIBILITY_SERVICE) as? AccessibilityManager)?.isTouchExplorationEnabled == true
    }
    val autoHide = anyVideo && !touchExploration
    var shown by remember { mutableStateOf(true) }
    var touches by remember { mutableIntStateOf(0) }
    LaunchedEffect(autoHide, shown, touches) {
        if (!autoHide) {
            shown = true
        } else if (shown) {
            delay(3_500L)
            shown = false
        }
    }
    val visible = shown || !autoHide
    val touched: (() -> Unit) -> (() -> Unit) = { f -> { touches++; f() } }

    Box(
        Modifier
            .fillMaxSize()
            .background(if (anyVideo) Brush.verticalGradient(listOf(Color.Black, Color.Black)) else Brush.verticalGradient(colors.topBarGradient))
            .pointerInput(autoHide) {
                detectTapGestures(onTap = {
                    if (autoHide) {
                        shown = !shown
                        touches++
                    }
                })
            },
    ) {
        Box(Modifier.fillMaxSize().safeDrawingPadding()) {
            if (!g.joined) {
                // A tocar: o grupo, quem chama, Recusar / Atender.
                Column(
                    Modifier.align(Alignment.TopCenter).fillMaxWidth().padding(top = 112.dp, start = 24.dp, end = 24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Avatar(g.groupName, g.groupId, sizeDp = 120)
                    Spacer(Modifier.height(20.dp))
                    Text(g.groupName, style = MaterialTheme.typography.headlineMedium, color = colors.onTopBar, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        s(if (g.video) R.string.gcall_video_title else R.string.gcall_title) + " · " + s(R.string.gcall_from, g.fromName),
                        style = MaterialTheme.typography.titleMedium, color = colors.onTopBar.copy(alpha = 0.85f), textAlign = TextAlign.Center,
                    )
                }
                Row(
                    Modifier.align(Alignment.BottomCenter).padding(bottom = 40.dp),
                    horizontalArrangement = Arrangement.spacedBy(64.dp),
                ) {
                    RoundAction(s(R.string.t_recusar), CALL_RED, Icons.Filled.Call, hangup = true, onClick = onLeave)
                    RoundAction(s(R.string.t_atender), CALL_GREEN, Icons.Filled.Call, hangup = false, onClick = onJoin)
                }
                return@Box
            }
            // 0.9.66: o quadrado grande e SEMPRE o meu (a minha camara) e os outros ficam na grelha,
            // sempre pela mesma ordem. Antes ia para cima quem falava, e cada troca refazia os
            // videos (flashes constantes). Quem fala fica so com a moldura na cor da app.
            Column(
                Modifier.fillMaxSize().padding(horizontal = 16.dp).padding(top = 56.dp, bottom = 12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                SelfTile(g, myName, myId, onSelfPreview, Modifier.fillMaxWidth(0.72f).aspectRatio(1f))
                if (g.tiles.isEmpty()) {
                    Spacer(Modifier.height(12.dp))
                    Text(s(R.string.gcall_waiting), style = MaterialTheme.typography.titleMedium, color = Color.White.copy(alpha = 0.85f), textAlign = TextAlign.Center)
                } else {
                    for (row in g.tiles.chunked(2)) {
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            for (t in row) {
                                key(t.contactId) {
                                    PeerTile(t, speaking = t.contactId == g.speaker, onSurface, onClearSurface, Modifier.weight(1f).aspectRatio(1.1f))
                                }
                            }
                            if (row.size == 1) Spacer(Modifier.weight(1f))
                        }
                    }
                }
            }
            AnimatedVisibility(visible, Modifier.align(Alignment.TopCenter), enter = fadeIn(), exit = fadeOut()) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(g.groupName, style = MaterialTheme.typography.titleMedium, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    val press = if (diagnostic != null) Modifier.pointerInput(Unit) { detectTapGestures(onLongPress = { showDiag = true }) } else Modifier
                    Box(press) { GroupCallClock(g.startedAt) }
                }
            }
            AnimatedVisibility(visible, Modifier.align(Alignment.BottomCenter), enter = fadeIn(), exit = fadeOut()) {
                Column(Modifier.padding(bottom = 20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Row(
                        Modifier
                            .clip(RoundedCornerShape(44.dp))
                            .background(Color(0xA6000000))
                            .padding(horizontal = 12.dp, vertical = 10.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        ToggleRound(if (g.muted) MicOffIcon else MicIcon, s(R.string.t_silenciar), on = g.muted, onClick = touched(onMute))
                        ToggleRound(SpeakerIcon, s(R.string.t_altifalante), on = g.speakerOn, onClick = touched(onSpeaker))
                        ToggleRound(if (g.camera) VideoCallIcon else VideoOffIcon, s(R.string.call_camera), on = g.camera, onClick = touched(onCamera))
                        ActionRound(SwitchCameraIcon, s(R.string.video_switch), enabled = g.camera, onClick = touched(onSwitchCamera))
                    }
                    Spacer(Modifier.height(16.dp))
                    HangupButton(touched(onLeave))
                }
            }
        }
    }
}

/** Moldura de um quadrado: a cor da app em quem fala, um fio claro nos outros. */
@Composable
private fun tileFrame(speaking: Boolean): Modifier {
    val colors = LocalChatColors.current
    val shape = RoundedCornerShape(18.dp)
    val border = if (speaking) {
        Modifier.border(2.dp, Brush.linearGradient(colors.topBarGradient.drop(1)), shape)
    } else {
        Modifier.border(1.dp, Color.White.copy(alpha = 0.18f), shape)
    }
    return Modifier.clip(shape).then(border).background(Brush.verticalGradient(colors.topBarGradient.take(2)))
}

@Composable
private fun PeerTile(
    t: GroupTile,
    speaking: Boolean,
    onSurface: (String, Surface?) -> Unit,
    onClearSurface: (String, Surface) -> Unit,
    modifier: Modifier,
) {
    Box(modifier.then(tileFrame(speaking))) {
        if (t.camera && t.connected) {
            val set by rememberUpdatedState(onSurface)
            val clear by rememberUpdatedState(onClearSurface)
            // O ecra deste quadrado: so ele o pode apagar (ver `clearSurface`).
            val mine = remember { arrayOfNulls<Surface>(1) }
            AndroidView(
                factory = { ctx ->
                    VideoTextureView(ctx).apply {
                        surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                            override fun onSurfaceTextureAvailable(st: SurfaceTexture, w: Int, h: Int) {
                                val sf = Surface(st)
                                mine[0] = sf
                                set(t.contactId, sf)
                            }
                            override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, w: Int, h: Int) {}
                            override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
                                mine[0]?.let { clear(t.contactId, it) }
                                mine[0] = null
                                return true
                            }
                            override fun onSurfaceTextureUpdated(st: SurfaceTexture) {}
                        }
                    }
                },
                update = { it.setFrame(t.videoW, t.videoH, t.rotation, mirrored = false) },
                modifier = Modifier.fillMaxSize(),
            )
            DisposableEffect(t.contactId) { onDispose { mine[0]?.let { clear(t.contactId, it) } } }
        } else {
            Box(Modifier.align(Alignment.Center).alpha(if (t.connected) 1f else 0.5f)) { Avatar(t.name, t.contactId, sizeDp = 64) }
        }
        TileLabel(t.name, camera = t.camera, Modifier.align(Alignment.BottomCenter))
    }
}

@Composable
private fun SelfTile(g: GroupCallUi, myName: String, myId: String, onPreview: (SurfaceTexture?) -> Unit, modifier: Modifier) {
    Box(modifier.then(tileFrame(g.meSpeaking))) {
        if (g.camera) {
            val current by rememberUpdatedState(onPreview)
            val view = LocalView.current
            val displayRotation = (view.display?.rotation ?: Surface.ROTATION_0) * 90
            AndroidView(
                factory = { ctx -> VideoTextureView(ctx).apply { surfaceTextureListener = textureListener { st -> current(st) } } },
                update = { it.setFrame(g.previewH, g.previewW, (360 - displayRotation) % 360, mirrored = false) },
                modifier = Modifier.fillMaxSize(),
            )
        } else {
            Box(Modifier.align(Alignment.Center)) { Avatar(myName, myId, sizeDp = 64) }
        }
        TileLabel(myName, camera = g.camera, Modifier.align(Alignment.BottomCenter), muted = g.muted)
    }
}

@Composable
private fun TileLabel(name: String, camera: Boolean, modifier: Modifier, muted: Boolean = false) {
    Row(
        modifier.fillMaxWidth().background(Brush.verticalGradient(listOf(Color.Transparent, Color(0x99000000)))).padding(horizontal = 10.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (camera) VideoCallIcon else if (muted) MicOffIcon else MicIcon,
            contentDescription = null, tint = Color.White, modifier = Modifier.size(16.dp),
        )
        Spacer(Modifier.width(6.dp))
        Text(name.uppercase(), style = MaterialTheme.typography.labelLarge, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun GroupCallClock(since: Long) {
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(since) {
        while (true) {
            now = SystemClock.elapsedRealtime()
            delay(1_000L)
        }
    }
    val sec = ((now - since).coerceAtLeast(0L) / 1000L)
    Text(String.format(java.util.Locale.ROOT, "%d:%02d", sec / 60, sec % 60), style = MaterialTheme.typography.titleMedium, color = Color.White)
}

/** O aviso antes da primeira chamada de grupo: so "Aceitar" deixa ligar ou entrar. */
@Composable
fun GroupCallNotice(onAccept: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.gcall_title)) },
        text = { Text(s(R.string.gcall_notice)) },
        confirmButton = { TextButton(onClick = onAccept) { Text(s(R.string.t_aceitar)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
    )
}
