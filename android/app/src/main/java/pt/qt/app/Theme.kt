package pt.qt.app

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** Nome da app (titulo, ecra inicial). O nome no telemovel vem do AndroidManifest. */
const val APP_NAME = "SEALER"

/** Roxo "galatico" da marca: mais escuro e mais vivo que o roxo Material original. */
private val Purple = Color(0xFF5A2FD8)
private val PurpleDark = Color(0xFF4A24B8)

private val LightScheme = lightColorScheme(
    primary = Purple,
    onPrimary = Color.White,
    primaryContainer = Color(0xFFE6DCFF),
    onPrimaryContainer = Color(0xFF1E0B5C),
    secondary = Color(0xFF5F5873),
    background = Color(0xFFFCFAFF),
    surface = Color(0xFFFCFAFF),
)

private val DarkScheme = darkColorScheme(
    primary = Color(0xFFB79CFF),
    onPrimary = Color(0xFF2A0C7A),
    primaryContainer = PurpleDark,
    onPrimaryContainer = Color(0xFFEADDFF),
    secondary = Color(0xFFCBC2E0),
    background = Color(0xFF120E1F),
    surface = Color(0xFF120E1F),
)

/** Cores especificas do mensageiro (barra, fundo do chat, bolhas, visto). */
@Immutable
data class ChatColors(
    val topBar: Color,
    /** Fundo da barra de topo: gradiente "galaxia" (do indigo noturno ao violeta), com estrelas. */
    val topBarGradient: List<Color>,
    val onTopBar: Color,
    val chatBackground: Color,
    val outgoingBubble: Color,
    val incomingBubble: Color,
    val bubbleText: Color,
    val bubbleMeta: Color,
    val readTick: Color,
    val avatarPalette: List<Color>,
)

private val Avatars = listOf(
    Color(0xFF7E57C2), Color(0xFF5C6BC0), Color(0xFF26A69A), Color(0xFFEF6C00),
    Color(0xFFD81B60), Color(0xFF00897B), Color(0xFF8D6E63), Color(0xFF3949AB),
)

private val LightChat = ChatColors(
    topBar = Color(0xFF4321B0),
    topBarGradient = listOf(Color(0xFF1E1052), Color(0xFF4321B0), Color(0xFF7038F0)),
    onTopBar = Color.White,
    chatBackground = Color(0xFFEFEAFB),
    outgoingBubble = Color(0xFFDCCDFF),
    incomingBubble = Color.White,
    bubbleText = Color(0xFF1C1B1F),
    bubbleMeta = Color(0xFF6E6A75),
    readTick = Color(0xFF34B7F1),
    avatarPalette = Avatars,
)

private val DarkChat = ChatColors(
    topBar = Color(0xFF22105E),
    topBarGradient = listOf(Color(0xFF0E0828), Color(0xFF22105E), Color(0xFF3A1C96)),
    onTopBar = Color(0xFFF1EAFF),
    chatBackground = Color(0xFF0F0B1C),
    outgoingBubble = PurpleDark,
    incomingBubble = Color(0xFF231C36),
    bubbleText = Color(0xFFEDE6F7),
    bubbleMeta = Color(0xFFB9B2C4),
    readTick = Color(0xFF53BDEB),
    avatarPalette = Avatars,
)

val LocalChatColors = staticCompositionLocalOf { LightChat }

@Composable
fun SealerTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    androidx.compose.runtime.CompositionLocalProvider(LocalChatColors provides if (dark) DarkChat else LightChat) {
        MaterialTheme(colorScheme = if (dark) DarkScheme else LightScheme, content = content)
    }
}
