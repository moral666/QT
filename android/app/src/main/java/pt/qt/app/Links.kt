package pt.qt.app

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.unit.dp

/**
 * Links nas mensagens (0.9.29): ficam clicaveis e copiaveis. Nada sai da app sem a pessoa
 * confirmar o site, e nao ha pre-visualizacao (quem recebe nunca contacta o site sozinho).
 * So `http(s)://` e `www.`; outros esquemas (intent:, file:, javascript:...) ficam texto.
 */
object Links {
    private val RE = Regex("""(?i)\b(?:https?://|www\.)[^\s<>"]+""")
    private const val TRAIL = ".,;:!?)]}'\"…»"
    private const val MAX = 20

    /** Posicoes dos links no texto (sem a pontuacao do fim da frase). */
    fun find(text: String): List<IntRange> = RE.findAll(text).take(MAX).mapNotNull { m ->
        var end = m.range.last
        while (end > m.range.first && text[end] in TRAIL) {
            // ")" fica se o link tiver "(" (ex.: wikipedia/Foo_(bar)).
            if (text[end] == ')' && text.substring(m.range.first, end).contains('(')) break
            end--
        }
        val r = m.range.first..end
        if (open(text.substring(r)) != null) r else null
    }.toList()

    fun urls(text: String): List<String> = find(text).map { text.substring(it) }.distinct()

    /** Endereco para abrir (com `https://` se comecar por `www.`), ou `null` se nao for web. */
    fun open(raw: String): String? {
        val u = if (raw.startsWith("www.", ignoreCase = true)) "https://$raw" else raw
        val uri = runCatching { Uri.parse(u) }.getOrNull() ?: return null
        val scheme = uri.scheme?.lowercase()
        if (scheme != "http" && scheme != "https") return null
        if (uri.host.isNullOrBlank()) return null
        return u
    }

    /**
     * O nome do site, como o navegador o vai mostrar. Nomes com letras de outros alfabetos
     * aparecem tambem em `xn--` (os links falsos usam letras parecidas, ex.: um "а" cirilico).
     */
    fun host(url: String): String {
        val h = runCatching { Uri.parse(url).host }.getOrNull().orEmpty().lowercase()
        val ascii = runCatching { java.net.IDN.toASCII(h) }.getOrDefault(h)
        return if (ascii != h) "$h ($ascii)" else h
    }
}

/** Texto da mensagem com os links sublinhados. Tocar num link pede confirmacao antes de abrir. */
@Composable
fun LinkedText(text: String, style: TextStyle, color: Color) {
    val ranges = remember(text) { Links.find(text) }
    var confirm by remember { mutableStateOf<String?>(null) }
    if (ranges.isEmpty()) {
        Text(text, style = style, color = color)
    } else {
        val linkStyle = TextLinkStyles(SpanStyle(color = color, textDecoration = TextDecoration.Underline))
        val annotated = remember(text, color) {
            buildAnnotatedString {
                var at = 0
                for (r in ranges) {
                    append(text.substring(at, r.first))
                    val raw = text.substring(r)
                    withLink(LinkAnnotation.Clickable(raw, linkStyle) { confirm = Links.open(raw) }) { append(raw) }
                    at = r.last + 1
                }
                append(text.substring(at))
            }
        }
        Text(annotated, style = style, color = color)
    }
    confirm?.let { url -> OpenLinkDialog(url, onDismiss = { confirm = null }) }
}

/** "Abrir este link?": o nome do site em destaque e o endereco completo por baixo. */
@Composable
private fun OpenLinkDialog(url: String, onDismiss: () -> Unit) {
    val ctx = LocalContext.current
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.link_open_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(Links.host(url), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                Text(url, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, maxLines = 4)
                Text(s(R.string.link_open_warn), style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = {
            TextButton(onClick = {
                onDismiss()
                runCatching {
                    ctx.startActivity(
                        Intent(Intent.ACTION_VIEW, Uri.parse(url))
                            .addCategory(Intent.CATEGORY_BROWSABLE)
                            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    )
                }
            }) { Text(s(R.string.t_abrir)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
    )
}
