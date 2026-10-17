package pt.qt.app

import android.graphics.Bitmap
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/**
 * Codigo QR com o ID publico (o mesmo texto "qt1:..." que se partilha ou copia).
 * Preto sobre branco, para qualquer camara o ler. Gerado no telemovel, sem rede.
 */
fun qrCode(text: String, sizePx: Int = 600): ImageBitmap {
    val hints = mapOf(EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M, EncodeHintType.MARGIN to 2)
    val m = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, sizePx, sizePx, hints)
    val w = m.width
    val px = IntArray(w * m.height) { i -> if (m.get(i % w, i / w)) 0xFF000000.toInt() else 0xFFFFFFFF.toInt() }
    return Bitmap.createBitmap(px, w, m.height, Bitmap.Config.ARGB_8888).asImageBitmap()
}

/**
 * O texto lido de um QR e um ID SEALER? Devolve-o limpo (em minusculas), ou null. O QR da
 * app vai em maiusculas (modo alfanumerico, menos denso); o Rust valida-o a seguir.
 */
fun idFromQr(contents: String?): String? =
    contents?.trim()?.takeIf { it.startsWith("qt1:", ignoreCase = true) && it.length in 68..400 }?.lowercase()

/** "qt1:66543d…bc5a39": o ID abreviado, so para mostrar. */
fun shortId(id: String): String = if (id.length > 20) id.take(10) + "…" + id.takeLast(6) else id

/**
 * O leitor de QR e outro ecra, por isso abri-lo conta como sair da app. Sem isto,
 * com "Bloquear imediatamente", voltar do leitor bloqueava a app a meio de adicionar
 * o contacto. Em vez de desligar o bloqueio, o leitor ganha GRACE_MS a mais: quem
 * deixar o telemovel no leitor encontra a app bloqueada na mesma.
 */
object AutoLockPause {
    @Volatile var active = false
    const val GRACE_MS = 2 * 60 * 1000L
}
