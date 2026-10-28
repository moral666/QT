package pt.qt.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.util.Log

/**
 * Diagnostico das chamadas para os testers (fase 5c, `BuildConfig.CALL_DIAG`). Os numeros vem do
 * Rust (`CallAudio.diagnostic`): caminho direto/relay, perdas, debito, resolucao, pausas e
 * mudancas de rede, sem IPs, IDs nem conteudo. Nada sai sozinho para a rede: o tester copia o
 * texto e envia-o se quiser. Com a opcao desligada nada disto aparece nem vai para o registo.
 */
object CallDiag {
    val enabled: Boolean get() = BuildConfig.CALL_DIAG

    /** Registo discreto (`adb logcat`), so com o diagnostico ligado. */
    fun log(tag: String, msg: String) {
        if (enabled) Log.d(tag, msg)
    }

    /** Cabecalho com a versao da app (para saber de que build vem). */
    fun withHeader(text: String): String = "Sealer ${BuildConfig.VERSION_NAME} (Android ${android.os.Build.VERSION.RELEASE})\n$text"

    fun copy(ctx: Context, text: String) {
        val cm = ctx.getSystemService(ClipboardManager::class.java) ?: return
        runCatching { cm.setPrimaryClip(ClipData.newPlainText("Sealer", text)) }
    }
}
