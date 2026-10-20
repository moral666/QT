package pt.qt.app

import androidx.compose.foundation.Image
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.res.painterResource

/**
 * Badges (aro colorido a volta da foto). O certificado e verificado no nucleo Rust
 * (`ffi/src/badges.rs`); aqui so se desenha. Um so desenho (`badge_ring`, o aro dourado em
 * tons de cinza) pintado com duas cores, sombra e brilho, para manter o aspeto metalico.
 * Numeros iguais aos do Rust: 0 nenhum, 1 Founder, 2 Co-founder, 3 Dev, 4 Tester, 5 Contributor.
 */
object Badges {
    const val NONE = 0

    /** (sombra, brilho) de cada badge; null = sem aro. */
    fun colors(badge: Int): Pair<Color, Color>? = when (badge) {
        1 -> Color(0xFF8A5F12) to Color(0xFFFFE48C)   // Founder: dourado
        2 -> Color(0xFF9E2E66) to Color(0xFFFFC2DD)   // Co-founder: rosa
        3 -> Color(0xFF141414) to Color(0xFF8A8A8A)   // Dev: preto
        4 -> Color(0xFF0A6F9C) to Color(0xFFB5F1FF)   // Tester: ciano
        5 -> Color(0xFF0D5E33) to Color(0xFF7FE3A8)   // Contributor: verde
        else -> null
    }

    fun name(badge: Int): String? = when (badge) {
        1 -> s(R.string.badge_founder)
        2 -> s(R.string.badge_cofounder)
        3 -> s(R.string.badge_dev)
        4 -> s(R.string.badge_tester)
        5 -> s(R.string.badge_contributor)
        else -> null
    }

    /** Parte do aro ocupada pela foto (o buraco do desenho tem 83% do diametro). */
    const val INNER = 0.84f

    /**
     * "Gradient map": o cinzento L (0..255) do desenho passa a sombra + L * (brilho - sombra),
     * em cada canal. O alfa fica igual.
     */
    fun filter(shadow: Color, light: Color): ColorFilter {
        fun row(s: Float, l: Float) = floatArrayOf(l - s, 0f, 0f, 0f, s * 255f)
        val r = row(shadow.red, light.red)
        val g = row(shadow.green, light.green)
        val b = row(shadow.blue, light.blue)
        return ColorFilter.colorMatrix(ColorMatrix(r + g + b + floatArrayOf(0f, 0f, 0f, 1f, 0f)))
    }
}

/** Badges dos contactos (contactId -> numero), para os Avatar. */
val LocalBadges = compositionLocalOf<Map<String, Int>> { emptyMap() }

/** O aro do badge, do tamanho do `modifier`. Nada se o badge for 0 ou desconhecido. */
@Composable
fun BadgeRing(badge: Int, modifier: Modifier) {
    val (shadow, light) = Badges.colors(badge) ?: return
    val filter = remember(badge) { Badges.filter(shadow, light) }
    Image(painterResource(R.drawable.badge_ring), contentDescription = Badges.name(badge), modifier = modifier, colorFilter = filter)
}
