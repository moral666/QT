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

/** Roxo da marca (o mesmo tom que a app ja usava). */
private val Purple = Color(0xFF6750A4)
private val PurpleDark = Color(0xFF4F378B)

private val LightScheme = lightColorScheme(
    primary = Purple,
    onPrimary = Color.White,
    primaryContainer = Color(0xFFEADDFF),
    onPrimaryContainer = Color(0xFF21005D),
    secondary = Color(0xFF625B71),
    background = Color(0xFFFFFBFE),
    surface = Color(0xFFFFFBFE),
)

private val DarkScheme = darkColorScheme(
    primary = Color(0xFFD0BCFF),
    onPrimary = Color(0xFF381E72),
    primaryContainer = PurpleDark,
    onPrimaryContainer = Color(0xFFEADDFF),
    secondary = Color(0xFFCCC2DC),
    background = Color(0xFF141218),
    surface = Color(0xFF141218),
)

/** Cores especificas do mensageiro (barra, fundo do chat, bolhas, visto). */
@Immutable
data class ChatColors(
    val topBar: Color,
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
    topBar = Purple,
    onTopBar = Color.White,
    chatBackground = Color(0xFFF1ECF8),
    outgoingBubble = Color(0xFFE3D7FF),
    incomingBubble = Color.White,
    bubbleText = Color(0xFF1C1B1F),
    bubbleMeta = Color(0xFF6E6A75),
    readTick = Color(0xFF34B7F1),
    avatarPalette = Avatars,
)

private val DarkChat = ChatColors(
    topBar = Color(0xFF2B2534),
    onTopBar = Color(0xFFEDE6F7),
    chatBackground = Color(0xFF17141C),
    outgoingBubble = PurpleDark,
    incomingBubble = Color(0xFF2B2930),
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
