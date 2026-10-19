package pt.qt.app

import android.os.SystemClock
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
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
) {
    val colors = LocalChatColors.current
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(call.phase, call.answeredAt) {
        while (call.phase == CallPhase.ACTIVE) {
            now = SystemClock.elapsedRealtime()
            delay(1_000L)
        }
    }
    val status = when (call.phase) {
        CallPhase.CALLING -> "A chamar…"
        CallPhase.RINGING_OUT -> "A tocar…"
        CallPhase.INCOMING -> "Chamada de voz"
        CallPhase.ACTIVE -> {
            val s = ((now - call.answeredAt).coerceAtLeast(0L) / 1000L)
            "%d:%02d".format(s / 60, s % 60)
        }
        CallPhase.ENDED -> call.endText
    }
    Box(
        Modifier
            .fillMaxSize()
            .background(Brush.verticalGradient(colors.topBarGradient))
            .safeDrawingPadding(),
    ) {
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
            Text("Cifrada ponta a ponta", style = MaterialTheme.typography.labelSmall, color = colors.onTopBar.copy(alpha = 0.6f))
            Spacer(Modifier.weight(1f))
            when (call.phase) {
                CallPhase.INCOMING -> Row(horizontalArrangement = Arrangement.spacedBy(56.dp)) {
                    RoundAction("Recusar", RED, Icons.Filled.Call, hangup = true, onClick = onHangup)
                    RoundAction("Atender", GREEN, Icons.Filled.Call, hangup = false, onClick = onAccept)
                }
                CallPhase.ENDED -> Spacer(Modifier.height(1.dp))
                else -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                        val btn = ButtonDefaults.outlinedButtonColors(contentColor = colors.onTopBar)
                        OutlinedButton(onClick = onMute, colors = btn) { Text(if (call.muted) "Ligar o microfone" else "Silenciar") }
                        OutlinedButton(onClick = onSpeaker, colors = btn) { Text(if (call.speaker) "Auricular" else "Altifalante") }
                    }
                    Spacer(Modifier.height(28.dp))
                    RoundAction("Desligar", RED, Icons.Filled.Call, hangup = true, onClick = onHangup)
                }
            }
            Spacer(Modifier.height(32.dp))
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
