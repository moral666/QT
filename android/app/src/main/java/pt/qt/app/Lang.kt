package pt.qt.app

import android.app.Application
import android.app.LocaleManager
import android.content.Context
import android.content.res.Configuration
import android.content.res.Resources
import android.os.Build
import android.os.LocaleList
import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import java.util.Locale

/**
 * Idiomas da app: PT (base, `values/`), EN, FR, ES, DE e IT (`values-xx/`).
 * Por omissao segue a lingua do telemovel (PT se nao for nenhuma destas); no menu
 * "Idioma" pode escolher-se uma a mao. No Android 13+ a escolha fica no sistema
 * (tambem aparece nas definicoes da app); antes disso aplica-se aqui.
 */
object Lang {
    /** Codigo -> nome na propria lingua (como aparece no menu). */
    val SUPPORTED = linkedMapOf(
        "pt" to "Português",
        "en" to "English",
        "fr" to "Français",
        "es" to "Español",
        "de" to "Deutsch",
        "it" to "Italiano",
    )
    private const val PREFS = "idioma"
    private const val KEY = "lang"

    private lateinit var app: Context
    @Volatile private var res: Resources? = null

    fun init(ctx: Context) {
        app = ctx.applicationContext
        refresh()
    }

    /** Lingua escolhida no menu ("" = a do telemovel). */
    fun chosen(): String = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY, "") ?: ""

    /** Escolhe a lingua ("" = a do telemovel). A Activity tem de ser recriada a seguir. */
    fun choose(tag: String) {
        app.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(KEY, tag).apply()
        if (Build.VERSION.SDK_INT >= 33) {
            app.getSystemService(LocaleManager::class.java)?.applicationLocales =
                if (tag.isEmpty()) LocaleList.getEmptyLocaleList() else LocaleList.forLanguageTags(tag)
        }
        refresh()
    }

    /** Volta a ler os textos (lingua escolhida ou do telemovel mudou). */
    fun refresh() {
        res = localized(app).resources
    }

    /** `ctx` com a lingua escolhida (antes do Android 13; no 13+ o sistema ja a aplica). */
    fun localized(ctx: Context): Context {
        val tag = chosen()
        if (tag.isEmpty() || Build.VERSION.SDK_INT >= 33) return ctx
        val conf = Configuration(ctx.resources.configuration)
        conf.setLocale(Locale.forLanguageTag(tag))
        return ctx.createConfigurationContext(conf)
    }

    /** A lingua em que a app esta a mostrar os textos (para datas e numeros). */
    fun locale(): Locale = (res ?: app.resources).configuration.locales[0]

    fun str(@StringRes id: Int, vararg args: Any?): String {
        val r = res ?: app.resources
        return if (args.isEmpty()) r.getString(id) else r.getString(id, *args)
    }
}

/** Texto traduzido (strings.xml), em qualquer sitio: ecras, ViewModel, notificacoes. */
fun s(@StringRes id: Int, vararg args: Any?): String = Lang.str(id, *args)

/** Textos que vem do nucleo Rust (sempre em portugues) para a lingua da app. */
fun coreText(t: String): String = when {
    t == "📷 Foto" -> "📷 " + s(R.string.t_foto)
    t == "🎤 Mensagem de voz" -> "🎤 " + s(R.string.core_voice_message)
    t.startsWith("⚠️ Esta mensagem usa uma funcionalidade") -> "⚠️ " + s(R.string.core_unsupported)
    else -> t
}

/** A Application: prepara os textos antes de qualquer ecra, notificacao ou servico. */
class SealerApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Lang.init(this)
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        Lang.refresh()
    }
}

/** Menu "Idioma": a lingua do telemovel ou uma das seis. */
@Composable
fun LanguageDialog(onPick: (String) -> Unit, onDismiss: () -> Unit) {
    val current = Lang.chosen()
    val options = listOf("" to s(R.string.lang_system)) + Lang.SUPPORTED.toList()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.lang_menu)) },
        text = {
            Column {
                options.forEach { (tag, label) ->
                    Row(
                        Modifier.fillMaxWidth().clickable { onPick(tag) }.padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = tag == current, onClick = { onPick(tag) })
                        Text(label)
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_fechar)) } },
    )
}
