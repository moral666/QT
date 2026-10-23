package pt.qt.app

import android.os.SystemClock
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

private val GREEN = Color(0xFF2E9E5B)
private val RED = Color(0xFFD64545)

/** Ecra de chamada (a chamar, a tocar, a chegar, atendida, terminada). */
@Composable
fun CallScreen(
    call: CallUi,
    onAccept: () -> Unit,
    onHangup: () -> Unit,
    onMute: () -> Unit,
    onSpeaker: () -> Unit,
    /** Minimizar (volta-se pela barra no topo da app); `null` = sem botao (a chegar, terminada). */
    onMinimize: (() -> Unit)? = null,
) {
    val colors = LocalChatColors.current
    val status = callStatusText(call)
    Box(
        Modifier
            .fillMaxSize()
            .background(Brush.verticalGradient(colors.topBarGradient))
            .safeDrawingPadding(),
    ) {
        if (onMinimize != null) {
            IconButton(onClick = onMinimize, modifier = Modifier.align(Alignment.TopStart).padding(8.dp)) {
                Icon(Icons.Filled.KeyboardArrowDown, contentDescription = s(R.string.call_minimize), tint = colors.onTopBar, modifier = Modifier.size(32.dp))
            }
        }
        Column(
            Modifier.fillMaxSize().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
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
            Spacer(Modifier.weight(1f))
            when (call.phase) {
                CallPhase.INCOMING -> Row(horizontalArrangement = Arrangement.spacedBy(56.dp)) {
                    RoundAction(s(R.string.t_recusar), RED, Icons.Filled.Call, hangup = true, onClick = onHangup)
                    RoundAction(s(R.string.t_atender), GREEN, Icons.Filled.Call, hangup = false, onClick = onAccept)
                }
                CallPhase.ENDED -> Spacer(Modifier.height(1.dp))
                else -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        val btn = ButtonDefaults.outlinedButtonColors(contentColor = colors.onTopBar)
                        OutlinedButton(onClick = onMute, colors = btn) { Text(if (call.muted) s(R.string.t_ligar_o_microfone) else s(R.string.t_silenciar)) }
                        OutlinedButton(onClick = onSpeaker, colors = btn) { Text(if (call.speaker) s(R.string.t_auricular) else s(R.string.t_altifalante)) }
                    }
                    Spacer(Modifier.height(28.dp))
                    RoundAction(s(R.string.t_desligar), RED, Icons.Filled.Call, hangup = true, onClick = onHangup)
                }
            }
            Spacer(Modifier.height(32.dp))
        }
    }
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
        CallPhase.INCOMING -> s(R.string.t_chamada_de_voz)
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
