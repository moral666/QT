package pt.qt.app

import android.app.Application
import android.app.LocaleManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.content.res.Resources
import android.os.Build
import android.net.Uri
import android.os.LocaleList
import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import java.text.Normalizer
import java.util.Locale

/**
 * Idiomas da app: PT (base, `values/`), EN, FR, ES, DE, IT, JA, ZH (chines simplificado,
 * `values-zh-rCN/`), HI e AR (`values-xx/`). O arabe escreve-se da direita para a esquerda:
 * `android:supportsRtl` no manifesto e, no Compose, `start`/`end` em vez de esquerda/direita.
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
        "ja" to "日本語",
        "zh-CN" to "简体中文",
        "hi" to "हिन्दी",
        "ar" to "العربية",
    )
    /** Para onde vao os avisos de erros de traducao (Definicoes > Reportar erro de traducao). */
    const val REPORT_EMAIL = "privacidade@sealerapp.com"
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
    fun locale(): Locale {
        // Sem app iniciada (testes unitarios na JVM): o locale do sistema.
        if (res == null && !::app.isInitialized) return Locale.getDefault()
        val l = (res ?: app.resources).configuration.locales[0]
        // Arabe: algarismos ocidentais 0-9 nas datas e tamanhos (decisao do Nuno, 22/10/2026).
        return if (l.language == "ar") Locale.Builder().setLocale(l).setUnicodeLocaleKeyword("nu", "latn").build() else l
    }

    /**
     * Abre a app de email com um aviso de erro de traducao para [REPORT_EMAIL]. Vai so a lingua
     * e a versao da app (nada do utilizador: nem o ID, nem contactos).
     */
    fun reportTranslation(ctx: Context) {
        val subject = "SEALER: erro de tradução (${locale().toLanguageTag()}, ${BuildConfig.VERSION_NAME})"
        val uri = Uri.parse(
            "mailto:$REPORT_EMAIL?subject=" + Uri.encode(subject) + "&body=" + Uri.encode(str(R.string.lang_report_body)),
        )
        try {
            ctx.startActivity(Intent(Intent.ACTION_SENDTO, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        } catch (e: ActivityNotFoundException) {
            android.widget.Toast.makeText(ctx, str(R.string.lang_report_no_app, REPORT_EMAIL), android.widget.Toast.LENGTH_LONG).show()
        }
    }

    /**
     * A palavra escrita para confirmar "Destruir perfil e dados" (APAGAR, 削除, हटाएं...) esta
     * certa? Aceita tambem as outras grafias de `confirm_word_alt` (separadas por virgulas: em
     * japones sem converter, em hindi as grafias comuns e DELETE de quem so tem teclado latino).
     * Ignora maiusculas, espacos nas pontas, as duas formas do nasal em hindi (ँ e ं) e, em arabe,
     * o tatweel, os sinais de vogal e as marcas de direcao que alguns teclados acrescentam.
     */
    fun confirmWordOk(typed: String): Boolean {
        fun norm(t: String) = Normalizer.normalize(t.trim(), Normalizer.Form.NFC).replace('\u0901', '\u0902')
            .replace(Regex("[\u0640\u064B-\u0652\u200E\u200F]"), "")
        val words = listOf(str(R.string.confirm_word)) + str(R.string.confirm_word_alt).split(',').filter { it.isNotBlank() }
        return words.any { norm(typed).equals(norm(it), ignoreCase = true) }
    }

    /**
     * Codigo de acesso: so algarismos e sempre 0-9. Teclados em arabe ou hindi enviam ٠-٩ ou
     * ०-९; o mesmo codigo tem de funcionar seja qual for o teclado (senao a pessoa fica fechada
     * fora e so pode apagar tudo).
     */
    fun asciiDigits(v: String, max: Int = 32): String =
        v.mapNotNull { c -> Character.digit(c, 10).takeIf { it >= 0 }?.let { '0' + it } }.joinToString("").take(max)

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

/** Menu "Idioma": a lingua do telemovel ou uma das dez. */
@Composable
fun LanguageDialog(onPick: (String) -> Unit, onDismiss: () -> Unit) {
    val current = Lang.chosen()
    val options = listOf("" to s(R.string.lang_system)) + Lang.SUPPORTED.toList()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.lang_menu)) },
        text = {
            // Com dez linguas a lista nao cabe em ecras mais baixos: desliza (0.9.31; antes o hindi
            // e o arabe ficavam esmagados um em cima do outro e o arabe sem nome).
            Column(Modifier.verticalScroll(rememberScrollState())) {
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
