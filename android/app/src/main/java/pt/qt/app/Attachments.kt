package pt.qt.app

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.ImageDecoder
import android.graphics.Matrix
import android.net.Uri
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.exifinterface.media.ExifInterface
import coil.ImageLoader
import coil.compose.AsyncImage
import coil.decode.GifDecoder
import coil.decode.ImageDecoderDecoder
import coil.request.ImageRequest
import uniffi.qt_ffi.AttachmentInfo
import uniffi.qt_ffi.AttachmentState
import uniffi.qt_ffi.ChatMessage
import java.io.File
import java.io.IOException
import kotlin.math.max
import kotlin.math.roundToInt

// ======================================================================= preparar

class ImageTooBigException : Exception("imagem maior que 25 MB")

/** Ficheiro temporario pronto a cifrar pelo Rust (apagar depois de `attachmentCreate`). */
data class PreparedImage(val file: File, val mime: String, val width: Int, val height: Int)

/**
 * Prepara uma imagem para enviar, tudo no telemovel:
 *  - GIF: vai como esta (para continuar animado);
 *  - fotos: reduzidas para 1600 px no lado maior e JPEG 80 (~300 KB), ou, com
 *    "qualidade original", ate 4096 px e JPEG 92;
 *  - PNG/WebP do teclado (autocolantes) ou em qualidade original: como estao.
 * Voltar a gravar a foto apaga os metadados (incluindo a localizacao GPS).
 */
object ImagePrep {
    const val MAX_BYTES = 25L * 1024 * 1024
    private const val COMPRESSED_SIDE = 1600
    private const val ORIGINAL_SIDE = 4096

    fun prepare(ctx: Context, uri: Uri, original: Boolean, keepAsIs: Boolean): PreparedImage {
        val mime = sniffMime(ctx, uri) ?: throw IOException("nao e uma imagem")
        val asIs = mime == "image/gif" || ((original || keepAsIs) && (mime == "image/png" || mime == "image/webp"))
        val out = File(ctx.cacheDir, "envio").apply { mkdirs() }.let { File.createTempFile("img", ".tmp", it) }
        try {
            if (asIs) {
                copyLimited(ctx, uri, out)
                val o = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(out.path, o)
                return PreparedImage(out, mime, max(o.outWidth, 1), max(o.outHeight, 1))
            }
            val bmp = decodeScaled(ctx, uri, if (original) ORIGINAL_SIDE else COMPRESSED_SIDE)
            try {
                out.outputStream().use { s -> bmp.compress(Bitmap.CompressFormat.JPEG, if (original) 92 else 80, s) }
                if (out.length() > MAX_BYTES) throw ImageTooBigException()
                return PreparedImage(out, "image/jpeg", bmp.width, bmp.height)
            } finally {
                bmp.recycle()
            }
        } catch (e: Exception) {
            out.delete()
            throw e
        }
    }

    /** Copia o que o teclado mandou (enquanto a permissao de leitura e valida). */
    fun copyToCache(ctx: Context, uri: Uri): File? = try {
        val out = File(ctx.cacheDir, "envio").apply { mkdirs() }.let { File.createTempFile("kbd", ".tmp", it) }
        try {
            copyLimited(ctx, uri, out)
            out
        } catch (e: Exception) {
            out.delete()
            null
        }
    } catch (e: Exception) {
        null
    }

    /** Ficheiros temporarios de envios anteriores (ex.: a app fechou a meio). */
    fun cleanCache(ctx: Context) {
        File(ctx.cacheDir, "envio").listFiles()?.forEach { it.delete() }
    }

    private fun copyLimited(ctx: Context, uri: Uri, out: File) {
        val input = ctx.contentResolver.openInputStream(uri) ?: throw IOException("sem acesso")
        input.use { inp ->
            out.outputStream().use { o ->
                val buf = ByteArray(64 * 1024)
                var total = 0L
                while (true) {
                    val n = inp.read(buf)
                    if (n < 0) break
                    total += n
                    if (total > MAX_BYTES) throw ImageTooBigException()
                    o.write(buf, 0, n)
                }
            }
        }
    }

    /** O tipo verdadeiro, pelos primeiros bytes (nao pela extensao nem pelo que a app de origem diz). */
    fun sniffMime(ctx: Context, uri: Uri): String? {
        val h = ByteArray(16)
        val n = ctx.contentResolver.openInputStream(uri)?.use { it.read(h) } ?: return null
        if (n < 12) return null
        fun at(i: Int, s: String) = s.indices.all { h[i + it] == s[it].code.toByte() }
        return when {
            at(0, "GIF87a") || at(0, "GIF89a") -> "image/gif"
            h[0] == 0x89.toByte() && at(1, "PNG") -> "image/png"
            h[0] == 0xFF.toByte() && h[1] == 0xD8.toByte() && h[2] == 0xFF.toByte() -> "image/jpeg"
            at(0, "RIFF") && at(8, "WEBP") -> "image/webp"
            at(4, "ftyp") -> "image/heic"   // HEIC/HEIF/AVIF: convertido para JPEG
            else -> null
        }
    }

    private fun decodeScaled(ctx: Context, uri: Uri, maxSide: Int): Bitmap {
        val decoded: Bitmap = if (Build.VERSION.SDK_INT >= 28) {
            // O ImageDecoder ja aplica a rotacao da camara (EXIF) e reduz enquanto le.
            ImageDecoder.decodeBitmap(ImageDecoder.createSource(ctx.contentResolver, uri)) { dec, info, _ ->
                dec.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                val w = info.size.width
                val h = info.size.height
                val scale = minOf(1f, maxSide.toFloat() / max(w, h))
                if (scale < 1f) dec.setTargetSize(max(1, (w * scale).roundToInt()), max(1, (h * scale).roundToInt()))
            }
        } else {
            decodeLegacy(ctx, uri, maxSide)
        }
        val scaled = fit(decoded, maxSide)
        // JPEG nao tem transparencia: fundo branco em vez de preto.
        if (!scaled.hasAlpha()) return scaled
        val flat = Bitmap.createBitmap(scaled.width, scaled.height, Bitmap.Config.ARGB_8888)
        Canvas(flat).apply { drawColor(android.graphics.Color.WHITE); drawBitmap(scaled, 0f, 0f, null) }
        scaled.recycle()
        return flat
    }

    private fun decodeLegacy(ctx: Context, uri: Uri, maxSide: Int): Bitmap {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        ctx.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, bounds) }
        var sample = 1
        while (max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= maxSide) sample *= 2
        val opts = BitmapFactory.Options().apply { inSampleSize = sample }
        val bmp = ctx.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, opts) }
            ?: throw IOException("imagem ilegivel")
        val orientation = ctx.contentResolver.openInputStream(uri)?.use {
            ExifInterface(it).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
        } ?: ExifInterface.ORIENTATION_NORMAL
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

    private fun fit(bmp: Bitmap, maxSide: Int): Bitmap {
        val side = max(bmp.width, bmp.height)
        if (side <= maxSide) return bmp
        val s = maxSide.toFloat() / side
        val out = Bitmap.createScaledBitmap(bmp, max(1, (bmp.width * s).roundToInt()), max(1, (bmp.height * s).roundToInt()), true)
        if (out !== bmp) bmp.recycle()
        return out
    }
}

// ================================================================== mostrar

/**
 * Carregador de imagens (Coil) so para os anexos: sem cache em disco (as imagens
 * em claro nunca sao escritas), GIFs animados. A cache em memoria e limpa ao
 * bloquear a app.
 */
object SealerImages {
    @Volatile private var loader: ImageLoader? = null

    fun loader(ctx: Context): ImageLoader = loader ?: synchronized(this) {
        loader ?: ImageLoader.Builder(ctx.applicationContext)
            .components {
                if (Build.VERSION.SDK_INT >= 28) add(ImageDecoderDecoder.Factory()) else add(GifDecoder.Factory())
            }
            .diskCache(null)
            .crossfade(true)
            .build()
            .also { loader = it }
    }

    fun clearMemory() {
        loader?.memoryCache?.clear()
    }
}

/** A imagem decifrada (so em memoria) para `m`, ou null enquanto carrega/se falhar. */
@Composable
fun rememberAttachmentBytes(m: ChatMessage, ready: Boolean, load: suspend (String, String) -> ByteArray?): ByteArray? {
    val bytes by produceState<ByteArray?>(null, m.messageId, ready) {
        value = if (ready) load(m.contactId, m.messageId) else null
    }
    return bytes
}

@Composable
fun AttachmentImage(bytes: ByteArray, key: String, modifier: Modifier, contentScale: ContentScale) {
    val ctx = LocalContext.current
    val request = remember(key, bytes) {
        ImageRequest.Builder(ctx).data(bytes).memoryCacheKey("att:$key").build()
    }
    AsyncImage(
        model = request,
        contentDescription = "Imagem",
        imageLoader = SealerImages.loader(ctx),
        modifier = modifier,
        contentScale = contentScale,
    )
}

/** Conteudo de uma bolha com anexo: a imagem (ou voz, ou ficheiro), ou o estado da descarga. */
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
fun AttachmentContent(
    m: ChatMessage,
    att: AttachmentInfo,
    progress: Float?,
    load: suspend (String, String) -> ByteArray?,
    onOpen: () -> Unit,
    onLongPress: () -> Unit = {},
) {
    // Ficheiros (PDF, documentos...): nunca mostrados aqui, so abertos ou guardados a pedido.
    if (att.fileName != null) {
        FileContent(m, att, progress, onOpen, onLongPress)
        return
    }
    if (att.mime == VOICE_MIME) {
        VoiceContent(m, att, progress, load, onLongPress)
        return
    }
    val ready = m.outgoing || att.state == AttachmentState.READY
    val bytes = rememberAttachmentBytes(m, ready, load)
    // Proporcao da imagem, limitada para nao haver bolhas gigantes nem tiras finas.
    val ratio = if (att.width > 0u && att.height > 0u) (att.width.toFloat() / att.height.toFloat()).coerceIn(0.5f, 2f) else 1f
    val w = 240.dp
    Box(
        Modifier.width(w).height(w / ratio).clip(RoundedCornerShape(12.dp))
            .background(Color.Black.copy(alpha = 0.08f))
            .combinedClickable(onClick = { if (bytes != null) onOpen() }, onLongClick = onLongPress),
        contentAlignment = Alignment.Center,
    ) {
        if (bytes != null) {
            AttachmentImage(bytes, m.messageId, Modifier.fillMaxSize(), ContentScale.Crop)
        }
        val busy = att.state == AttachmentState.UPLOADING || att.state == AttachmentState.DOWNLOADING
        if (busy) {
            Box(Modifier.size(56.dp).clip(RoundedCornerShape(28.dp)).background(Color.Black.copy(alpha = 0.45f)), contentAlignment = Alignment.Center) {
                if (progress != null && progress > 0f) {
                    CircularProgressIndicator(progress = { progress }, color = Color.White, modifier = Modifier.size(40.dp))
                } else {
                    CircularProgressIndicator(color = Color.White, modifier = Modifier.size(40.dp))
                }
            }
        }
        val problem = when (att.state) {
            AttachmentState.EXPIRED -> "A imagem já não está disponível (passaram mais de 24 horas)."
            AttachmentState.FAILED -> if (m.outgoing) "Não foi possível enviar." else "Não foi possível receber a imagem."
            else -> null
        }
        if (problem != null) {
            Text(
                problem,
                Modifier.align(if (bytes != null) Alignment.BottomCenter else Alignment.Center).fillMaxWidth()
                    .background(Color.Black.copy(alpha = 0.55f)).padding(8.dp),
                color = Color.White, fontSize = 13.sp, textAlign = TextAlign.Center,
            )
        }
    }
}

/** Imagem em ecra inteiro (toque para fechar). */
@Composable
fun AttachmentViewer(m: ChatMessage, load: suspend (String, String) -> ByteArray?, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        val bytes = rememberAttachmentBytes(m, true, load)
        Box(
            Modifier.fillMaxSize().background(Color.Black).clickable(onClick = onDismiss),
            contentAlignment = Alignment.Center,
        ) {
            if (bytes != null) {
                AttachmentImage(bytes, m.messageId + ":full", Modifier.fillMaxSize(), ContentScale.Fit)
            } else {
                CircularProgressIndicator(color = Color.White)
            }
        }
    }
}

/** Pre-visualizacao antes de enviar, com a opcao de qualidade original. */
@Composable
fun SendImageDialog(uri: Uri, onSend: (original: Boolean) -> Unit, onDismiss: () -> Unit) {
    val ctx = LocalContext.current
    var original by rememberSaveable { mutableStateOf(false) }
    val isGif = remember(uri) { runCatching { ImagePrep.sniffMime(ctx, uri) == "image/gif" }.getOrDefault(false) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (isGif) "Enviar GIF" else "Enviar imagem") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                AsyncImage(
                    model = uri,
                    contentDescription = "Pré-visualização",
                    imageLoader = SealerImages.loader(ctx),
                    modifier = Modifier.fillMaxWidth().heightIn(max = 320.dp).clip(RoundedCornerShape(12.dp)),
                    contentScale = ContentScale.Fit,
                )
                if (!isGif) {
                    Row(
                        Modifier.fillMaxWidth().clickable { original = !original },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Checkbox(checked = original, onCheckedChange = { original = it })
                        Column {
                            Text("Qualidade original", style = MaterialTheme.typography.bodyMedium)
                            Text(
                                if (original) "Mais nítida, mas gasta mais dados." else "Comprimida: envia mais depressa e gasta menos dados.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
                Text(
                    "Cifrada no teu telemóvel antes de sair. A localização da foto é removida.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = { TextButton(onClick = { onSend(original) }) { Text("Enviar") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
}
