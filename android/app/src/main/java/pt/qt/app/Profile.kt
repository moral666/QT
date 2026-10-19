package pt.qt.app

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.graphics.Matrix
import android.net.Uri
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.exifinterface.media.ExifInterface
import java.io.ByteArrayInputStream
import java.io.IOException
import java.nio.ByteBuffer
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/** Fotos de perfil ja decifradas e prontas a mostrar, por contactId (os Avatar leem daqui). */
val LocalPhotos = compositionLocalOf<Map<String, ImageBitmap>> { emptyMap() }

class PhotoTooBigException : Exception("foto maior que 3 MB")

/**
 * Foto de perfil: a original pode ter ate 3 MB; vai para os contactos um quadrado pequeno
 * em WebP que caiba no limite do Rust (`MAX_PHOTO_BYTES`, 10 000 bytes), dentro de um
 * envelope do tamanho normal. Tudo em memoria: nada em claro no disco.
 */
object ProfilePhoto {
    const val MAX_INPUT_BYTES = 3 * 1024 * 1024
    private const val MAX_OUT_BYTES = 10_000
    /** Fotos recebidas maiores do que isto nao se mostram (defesa contra imagens estranhas). */
    private const val MAX_SHOWN_SIDE = 1024

    /** Le a foto escolhida (ate 3 MB), corta ao centro em quadrado e comprime. */
    fun prepare(ctx: Context, uri: Uri): ByteArray {
        val raw = readLimited(ctx, uri)
        val bmp = decodeSquare(raw, 512)
        try {
            for (side in intArrayOf(256, 224, 192, 160, 128)) {
                val s = if (bmp.width == side) bmp else Bitmap.createScaledBitmap(bmp, side, side, true)
                try {
                    for (q in intArrayOf(80, 70, 60, 50, 40, 30)) {
                        val out = java.io.ByteArrayOutputStream()
                        s.compress(webp(), q, out)
                        if (out.size() <= MAX_OUT_BYTES) return out.toByteArray()
                    }
                } finally {
                    if (s !== bmp) s.recycle()
                }
            }
        } finally {
            bmp.recycle()
        }
        throw IOException("nao foi possivel reduzir a foto")
    }

    /** Foto recebida (bytes do Rust) para mostrar; null se nao for uma imagem aceitavel. */
    fun decode(bytes: ByteArray): ImageBitmap? = runCatching {
        val o = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, o)
        if (o.outWidth <= 0 || o.outHeight <= 0 || max(o.outWidth, o.outHeight) > MAX_SHOWN_SIDE) return null
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
    }.getOrNull()

    @Suppress("DEPRECATION")
    private fun webp(): Bitmap.CompressFormat =
        if (Build.VERSION.SDK_INT >= 30) Bitmap.CompressFormat.WEBP_LOSSY else Bitmap.CompressFormat.WEBP

    private fun readLimited(ctx: Context, uri: Uri): ByteArray {
        val input = ctx.contentResolver.openInputStream(uri) ?: throw IOException("foto ilegivel")
        input.use { s ->
            val out = java.io.ByteArrayOutputStream()
            val buf = ByteArray(64 * 1024)
            while (true) {
                val n = s.read(buf)
                if (n < 0) break
                out.write(buf, 0, n)
                if (out.size() > MAX_INPUT_BYTES) throw PhotoTooBigException()
            }
            return out.toByteArray()
        }
    }

    /** Descodifica (com a rotacao da camara) e corta ao centro num quadrado de `side`. */
    private fun decodeSquare(raw: ByteArray, side: Int): Bitmap {
        val full: Bitmap = if (Build.VERSION.SDK_INT >= 28) {
            ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(raw))) { dec, info, _ ->
                dec.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                val w = info.size.width
                val h = info.size.height
                val scale = min(1f, side.toFloat() / min(w, h))
                if (scale < 1f) dec.setTargetSize(max(1, (w * scale).roundToInt()), max(1, (h * scale).roundToInt()))
            }
        } else {
            decodeLegacy(raw, side)
        }
        val crop = min(full.width, full.height)
        val sq = Bitmap.createBitmap(full, (full.width - crop) / 2, (full.height - crop) / 2, crop, crop)
        if (sq !== full) full.recycle()
        val out = if (crop == side) sq else Bitmap.createScaledBitmap(sq, side, side, true)
        if (out !== sq) sq.recycle()
        return out
    }

    private fun decodeLegacy(raw: ByteArray, side: Int): Bitmap {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(raw, 0, raw.size, bounds)
        var sample = 1
        while (min(bounds.outWidth, bounds.outHeight) / (sample * 2) >= side) sample *= 2
        val bmp = BitmapFactory.decodeByteArray(raw, 0, raw.size, BitmapFactory.Options().apply { inSampleSize = sample })
            ?: throw IOException("foto ilegivel")
        val orientation = runCatching {
            ExifInterface(ByteArrayInputStream(raw)).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
        }.getOrDefault(ExifInterface.ORIENTATION_NORMAL)
        val degrees = when (orientation) {
            ExifInterface.ORIENTATION_ROTATE_90 -> 90f
            ExifInterface.ORIENTATION_ROTATE_180 -> 180f
            ExifInterface.ORIENTATION_ROTATE_270 -> 270f
            else -> 0f
        }
        if (degrees == 0f) return bmp
        val rotated = Bitmap.createBitmap(bmp, 0, 0, bmp.width, bmp.height, Matrix().apply { postRotate(degrees) }, true)
        if (rotated !== bmp) bmp.recycle()
        return rotated
    }
}

/** "Daqui a 2 d e 5 h" / "daqui a 3 h" / "daqui a menos de 1 h". */
fun waitText(ms: Long): String {
    val h = (ms + 3_599_999) / 3_600_000
    return when {
        h <= 1 -> "daqui a menos de 1 h"
        h < 24 -> "daqui a $h h"
        h % 24 == 0L -> "daqui a ${h / 24} d"
        else -> "daqui a ${h / 24} d e ${h % 24} h"
    }
}

/** Menu "Foto de perfil": ver, escolher (de 72 em 72 h) ou tirar a foto. */
@Composable
fun ProfilePhotoDialog(ui: UiState, onPick: (Uri) -> Unit, onRemove: () -> Unit, onDismiss: () -> Unit) {
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri != null) onPick(uri)
    }
    val canChange = ui.photoNextChangeMs <= 0L
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Foto de perfil") },
        text = {
            Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                val mine = ui.myPhoto
                if (mine != null) {
                    Image(mine, contentDescription = "A tua foto", modifier = Modifier.size(140.dp).clip(CircleShape), contentScale = ContentScale.Crop)
                } else {
                    Avatar("Eu", "eu", sizeDp = 140)
                }
                Text(
                    "Só os teus contactos a veem, cifrada de ponta a ponta. Máximo 3 MB. Podes mudar de 72 em 72 horas.",
                    style = MaterialTheme.typography.bodySmall, textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (!canChange) {
                    Text(
                        "Próxima mudança ${waitText(ui.photoNextChangeMs)}.",
                        style = MaterialTheme.typography.bodySmall, textAlign = TextAlign.Center,
                    )
                }
            }
        },
        confirmButton = {
            Row {
                if (ui.myPhoto != null) TextButton(onClick = onRemove) { Text("Tirar") }
                TextButton(
                    enabled = canChange,
                    onClick = {
                        AutoLockPause.active = true   // escolher a foto nao conta como sair da app
                        pick.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                    },
                ) { Text("Escolher foto") }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Fechar") } },
    )
}
