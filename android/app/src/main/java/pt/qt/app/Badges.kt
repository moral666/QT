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
import androidx.compose.ui.draw.shadow
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.unit.dp

/**
 * Badges (aro colorido a volta da foto). O certificado e verificado no nucleo Rust
 * (`ffi/src/badges.rs`); aqui so se desenha. Um so desenho (`badge_ring`, o aro dourado em
 * tons de cinza) pintado com duas cores, sombra e brilho, para manter o aspeto metalico.
 * A Legacy e diferente: um escudo prateado a volta da foto (`badge_shield_legacy`), sem aro
 * nem medalha no canto.
 * Numeros iguais aos do Rust: 0 nenhum, 1 Founder, 2 Co-founder, 3 Dev, 4 Tester, 5 Contributor,
 * 6 Bug Hunter, 7 Support, 8 Troll, 9 Legacy.
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
        6 -> Color(0xFF8E0E1A) to Color(0xFFFFB0B0)   // Bug Hunter: vermelho
        7 -> Color(0xFF9A4208) to Color(0xFFFFD2A0)   // Support: laranja
        8 -> Color(0xFF14287A) to Color(0xFFA9C0FF)   // Troll: azul
        else -> null
    }

    fun name(badge: Int): String? = when (badge) {
        1 -> s(R.string.badge_founder)
        2 -> s(R.string.badge_cofounder)
        3 -> s(R.string.badge_dev)
        4 -> s(R.string.badge_tester)
        5 -> s(R.string.badge_contributor)
        6 -> s(R.string.badge_bughunter)
        7 -> s(R.string.badge_support)
        8 -> s(R.string.badge_troll)
        LEGACY -> s(R.string.badge_legacy)
        else -> null
    }

    /** A medalha do badge (canto da foto); null = nenhuma. */
    fun coin(badge: Int): Int? = when (badge) {
        1 -> R.drawable.badge_coin_founder
        2 -> R.drawable.badge_coin_cofounder
        3 -> R.drawable.badge_coin_dev
        4 -> R.drawable.badge_coin_tester
        5 -> R.drawable.badge_coin_contributor
        6 -> R.drawable.badge_coin_bughunter
        7 -> R.drawable.badge_coin_support
        8 -> R.drawable.badge_coin_troll
        else -> null
    }

    /** Legacy: escudo a volta da foto, em vez do aro e da medalha. */
    const val LEGACY = 9

    /** Ha alguma coisa a desenhar (aro + medalha, ou o escudo). */
    fun shows(badge: Int): Boolean = colors(badge) != null || badge == LEGACY

    /**
     * Escudo: lado do desenho em relacao ao avatar (passa um pouco para fora, sobretudo em
     * baixo, para a foto nao ficar muito mais pequena do que com o aro).
     */
    const val SHIELD = 1.25f

    /** Foto dentro do escudo, em relacao ao avatar (o buraco do desenho tem 59,2% do lado). */
    const val SHIELD_PHOTO = SHIELD * 0.60f

    /** O centro do buraco esta a 45,2% da altura do desenho: o escudo desce 4,8% do seu lado. */
    const val SHIELD_DOWN = SHIELD * 0.048f

    /** Medalha: 40% do avatar (45% na minha foto do cabecalho). */
    const val COIN = 0.40f

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

/**
 * O escudo da Legacy, centrado no avatar de lado `sizeDp`. Fica maior do que o avatar (passa para
 * fora sem mudar o tamanho do avatar): `requiredSize` ignora o tamanho que o pai da.
 */
@Composable
fun BadgeShield(sizeDp: Int, modifier: Modifier = Modifier) {
    Image(
        painterResource(R.drawable.badge_shield_legacy),
        contentDescription = Badges.name(Badges.LEGACY),
        modifier = modifier.offset(y = (sizeDp * Badges.SHIELD_DOWN).dp).requiredSize((sizeDp * Badges.SHIELD).dp),
    )
}

/** A medalha do badge (moeda com o simbolo), do tamanho do `modifier`. */
@Composable
fun BadgeCoin(badge: Int, modifier: Modifier) {
    val res = Badges.coin(badge) ?: return
    Image(painterResource(res), contentDescription = null, modifier = modifier.shadow(3.dp, CircleShape))
}
