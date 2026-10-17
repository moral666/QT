package pt.qt.app

import android.content.ActivityNotFoundException
import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelFileDescriptor
import android.os.ProxyFileDescriptorCallback
import android.os.SystemClock
import android.os.storage.StorageManager
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.qt_ffi.AttachmentInfo
import uniffi.qt_ffi.AttachmentState
import uniffi.qt_ffi.ChatMessage
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException
import java.security.SecureRandom
import java.util.concurrent.ConcurrentHashMap

// ===================================================================== preparar

class FileTooBigException : Exception("ficheiro maior que 25 MB")

/** Ficheiro temporario pronto a cifrar pelo Rust (apagar depois de `attachmentCreateFile`). */
data class PreparedFile(val file: File, val name: String, val mime: String)

/**
 * Ficheiros (PDF, documentos, etc.): vao tal como estao, sem compressao, ate 25 MB.
 * O nome e o tipo vem do Android; sao limpos aqui e validados outra vez no Rust.
 */
object FilePrep {
    const val MAX_BYTES = 25L * 1024 * 1024
    private const val MAX_NAME_BYTES = 200
    private val MIME_RE = Regex("^[a-z0-9!#\$&^_.+-]+/[a-z0-9!#\$&^_.+-]+\$")

    /** Nome e tamanho que o Android indica (o tamanho pode faltar). */
    fun describe(ctx: Context, uri: Uri): Pair<String, Long?> {
        var name: String? = null
        var size: Long? = null
        try {
            ctx.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    val n = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    val s = c.getColumnIndex(OpenableColumns.SIZE)
                    if (n >= 0 && !c.isNull(n)) name = c.getString(n)
                    if (s >= 0 && !c.isNull(s)) size = c.getLong(s)
                }
            }
        } catch (e: Exception) {
            // Sem nome: fica "ficheiro".
        }
        return cleanName(name) to size
    }

    fun prepare(ctx: Context, uri: Uri): PreparedFile {
        val (name, _) = describe(ctx, uri)
        val mime = cleanMime(ctx.contentResolver.getType(uri))
        val out = File(ctx.cacheDir, "envio").apply { mkdirs() }.let { File.createTempFile("fich", ".tmp", it) }
        try {
            val input = ctx.contentResolver.openInputStream(uri) ?: throw IOException("sem acesso")
            input.use { inp ->
                out.outputStream().use { o ->
                    val buf = ByteArray(64 * 1024)
                    var total = 0L
                    while (true) {
                        val n = inp.read(buf)
                        if (n < 0) break
                        total += n
                        if (total > MAX_BYTES) throw FileTooBigException()
                        o.write(buf, 0, n)
                    }
                }
            }
            if (out.length() == 0L) throw IOException("ficheiro vazio")
            return PreparedFile(out, name, mime)
        } catch (e: Exception) {
            out.delete()
            throw e
        }
    }

    /** Sem caminhos nem caracteres de controlo, ate 200 bytes (mantendo a extensao). */
    fun cleanName(raw: String?): String {
        val s = (raw ?: "").map { c -> if (c == '/' || c == '\\' || c.isISOControl()) '_' else c }.joinToString("").trim()
        if (s.isEmpty() || s == "." || s == "..") return "ficheiro"
        if (s.toByteArray(Charsets.UTF_8).size <= MAX_NAME_BYTES) return s
        val dot = s.lastIndexOf('.')
        val ext = if (dot > 0 && s.length - dot <= 16) s.substring(dot) else ""
        val base = s.substring(0, s.length - ext.length)
        val room = MAX_NAME_BYTES - ext.toByteArray(Charsets.UTF_8).size
        val sb = StringBuilder()
        var used = 0
        var i = 0
        while (i < base.length) {
            val cp = base.codePointAt(i)
            val chunk = String(Character.toChars(cp))
            val len = chunk.toByteArray(Charsets.UTF_8).size
            if (used + len > room) break
            sb.append(chunk)
            used += len
            i += Character.charCount(cp)
        }
        return sb.toString().trimEnd() + ext
    }

    /** "tipo/subtipo" em minusculas, sem parametros; senao `application/octet-stream`. */
    fun cleanMime(raw: String?): String {
        val m = (raw ?: "").substringBefore(';').trim().lowercase()
        return if (m.length <= 100 && MIME_RE.matches(m)) m else "application/octet-stream"
    }
}

/** "350 KB", "2,4 MB". */
fun formatSize(bytes: Long): String = when {
    bytes < 1024 -> "$bytes B"
    bytes < 1024 * 1024 -> "${(bytes + 512) / 1024} KB"
    else -> String.format(java.util.Locale("pt", "PT"), "%.1f MB", bytes / (1024.0 * 1024.0))
}

/** "PDF", "DOCX"... (para o icone); vazio se nao houver extensao curta. */
fun fileExtension(name: String): String {
    val dot = name.lastIndexOf('.')
    if (dot <= 0 || dot == name.length - 1) return ""
    val ext = name.substring(dot + 1)
    return if (ext.length <= 5) ext.uppercase() else ""
}

// ============================================================ abrir noutra app

/**
 * Entrega um ficheiro decifrado a outra app (um leitor de PDF, por exemplo) SEM o
 * escrever em disco: a outra app le-o a partir da memoria desta, por um descritor
 * de ficheiro "virtual" (Android 8+) ou por um pipe (Android 7). Cada ficheiro fica
 * disponivel no maximo 10 minutos, ou ate a app bloquear. A outra app pode, claro,
 * guardar uma copia sua.
 */
class SealerFileProvider : ContentProvider() {
    companion object {
        private const val TTL_MS = 10 * 60 * 1000L
        private class Entry(val bytes: ByteArray, val name: String, val mime: String, val at: Long)
        private val entries = ConcurrentHashMap<String, Entry>()
        private val random = SecureRandom()
        @Volatile private var thread: HandlerThread? = null

        private fun authority(ctx: Context) = "${ctx.packageName}.ficheiros"

        /** Torna `bytes` legivel por outra app (com permissao dada no Intent). */
        fun share(ctx: Context, bytes: ByteArray, name: String, mime: String): Uri {
            purge()
            val token = ByteArray(16).also { random.nextBytes(it) }.joinToString("") { "%02x".format(it) }
            entries[token] = Entry(bytes, name, mime, SystemClock.elapsedRealtime())
            return Uri.Builder().scheme("content").authority(authority(ctx)).appendPath(token).appendPath(name).build()
        }

        /** Apaga da memoria todos os ficheiros partilhados (ao bloquear ou fechar a sessao). */
        fun clear() {
            for (e in entries.values) e.bytes.fill(0)
            entries.clear()
        }

        private fun purge() {
            val now = SystemClock.elapsedRealtime()
            val old = entries.filterValues { now - it.at > TTL_MS }.keys
            for (k in old) entries.remove(k)?.bytes?.fill(0)
        }

        private fun handler(): Handler {
            val t = thread ?: synchronized(this) {
                thread ?: HandlerThread("sealer-ficheiros").also { it.start(); thread = it }
            }
            return Handler(t.looper)
        }
    }

    override fun onCreate(): Boolean = true

    private fun entry(uri: Uri): Entry? {
        purge()
        return uri.pathSegments.firstOrNull()?.let { entries[it] }
    }

    override fun getType(uri: Uri): String? = entry(uri)?.mime

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? {
        val e = entry(uri) ?: return null
        val cols = projection?.filter { it == OpenableColumns.DISPLAY_NAME || it == OpenableColumns.SIZE }?.toTypedArray()
            ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        val row = cols.map<String, Any> { if (it == OpenableColumns.DISPLAY_NAME) e.name else e.bytes.size.toLong() }
        return MatrixCursor(cols, 1).apply { addRow(row.toTypedArray()) }
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw SecurityException("so leitura")
        val bytes = entry(uri)?.bytes ?: throw FileNotFoundException()
        if (Build.VERSION.SDK_INT >= 26) {
            val sm = context!!.getSystemService(StorageManager::class.java)
            return sm.openProxyFileDescriptor(ParcelFileDescriptor.MODE_READ_ONLY, object : ProxyFileDescriptorCallback() {
                override fun onGetSize(): Long = bytes.size.toLong()
                override fun onRead(offset: Long, size: Int, data: ByteArray): Int {
                    if (offset >= bytes.size) return 0
                    val n = minOf(size.toLong(), bytes.size - offset).toInt()
                    System.arraycopy(bytes, offset.toInt(), data, 0, n)
                    return n
                }
                override fun onRelease() {}
            }, handler())
        }
        val pipe = ParcelFileDescriptor.createReliablePipe()
        Thread {
            try {
                ParcelFileDescriptor.AutoCloseOutputStream(pipe[1]).use { it.write(bytes) }
            } catch (e: IOException) {
                // A outra app fechou antes do fim.
            }
        }.start()
        return pipe[0]
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = 0
}

// ===================================================================== mostrar

/** Bolha de um ficheiro: icone com a extensao, nome, tamanho e estado. Tocar abre as opcoes. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun FileContent(m: ChatMessage, att: AttachmentInfo, progress: Float?, onOpen: () -> Unit, onLongPress: () -> Unit) {
    val colors = LocalChatColors.current
    val name = att.fileName ?: "ficheiro"
    val ready = m.outgoing || att.state == AttachmentState.READY
    val busy = att.state == AttachmentState.UPLOADING || att.state == AttachmentState.DOWNLOADING
    val ext = fileExtension(name)
    val subtitle = when (att.state) {
        AttachmentState.EXPIRED -> "Já não está disponível (passaram mais de 24 horas)."
        AttachmentState.FAILED -> if (m.outgoing) "Não foi possível enviar." else "Não foi possível receber."
        AttachmentState.UPLOADING -> "A enviar… ${formatSize(att.size.toLong())}"
        AttachmentState.DOWNLOADING -> "A receber… ${formatSize(att.size.toLong())}"
        else -> if (ext.isNotEmpty()) "${formatSize(att.size.toLong())} · $ext" else formatSize(att.size.toLong())
    }
    Row(
        Modifier.width(250.dp).clip(RoundedCornerShape(12.dp))
            .background(Color.Black.copy(alpha = 0.06f))
            .combinedClickable(onClick = { if (ready) onOpen() }, onLongClick = onLongPress)
            .padding(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier.size(44.dp).clip(RoundedCornerShape(10.dp)).background(MaterialTheme.colorScheme.primary.copy(alpha = 0.15f)),
            contentAlignment = Alignment.Center,
        ) {
            if (busy) {
                if (progress != null && progress > 0f) {
                    CircularProgressIndicator(progress = { progress }, modifier = Modifier.size(30.dp))
                } else {
                    CircularProgressIndicator(modifier = Modifier.size(30.dp))
                }
            } else {
                Text(
                    if (ext.isNotEmpty()) ext else "📄",
                    fontSize = if (ext.isNotEmpty()) 11.sp else 20.sp, fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.primary, maxLines = 1,
                )
            }
        }
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(name, style = MaterialTheme.typography.bodyMedium, color = colors.bubbleText, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(subtitle, fontSize = 12.sp, color = colors.bubbleMeta, maxLines = 2)
        }
    }
}

/** Opcoes de um ficheiro: abrir noutra app ou guardar no telemovel. */
@Composable
fun FileActionsDialog(m: ChatMessage, load: suspend (String, String) -> ByteArray?, onDismiss: () -> Unit) {
    val att = m.attachment ?: return
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val name = att.fileName ?: "ficheiro"
    var busy by remember { mutableStateOf(false) }
    fun toast(s: String) = Toast.makeText(ctx, s, Toast.LENGTH_LONG).show()

    // Guardar: o utilizador escolhe onde (Transferencias, Drive...). Fica la em claro, por escolha dele.
    val saver = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument(att.mime)) { uri ->
        if (uri == null) {
            busy = false
        } else {
            scope.launch {
                val ok = withContext(Dispatchers.IO) {
                    val bytes = load(m.contactId, m.messageId)
                    if (bytes == null) {
                        false
                    } else {
                        try {
                            ctx.contentResolver.openOutputStream(uri)?.use { it.write(bytes) } != null
                        } catch (e: Exception) {
                            false
                        } finally {
                            bytes.fill(0)
                        }
                    }
                }
                toast(if (ok) "Ficheiro guardado." else "Não foi possível guardar o ficheiro.")
                onDismiss()
            }
        }
    }

    fun open() {
        busy = true
        scope.launch {
            val bytes = load(m.contactId, m.messageId)
            if (bytes == null) {
                toast("Não foi possível abrir o ficheiro.")
                busy = false
                return@launch
            }
            val uri = SealerFileProvider.share(ctx, bytes, name, att.mime)
            val intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, att.mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            try {
                AutoLockPause.active = true   // abrir noutra app nao conta como sair (bloqueio)
                ctx.startActivity(intent)
                onDismiss()
            } catch (e: ActivityNotFoundException) {
                AutoLockPause.active = false
                toast("Não há nenhuma app para abrir este tipo de ficheiro. Usa \"Guardar\".")
                busy = false
            }
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(name, maxLines = 3, overflow = TextOverflow.Ellipsis) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(formatSize(att.size.toLong()), style = MaterialTheme.typography.bodyMedium)
                Text(
                    "Fica cifrado no teu telemóvel. Ao abrir, é decifrado só na memória e entregue à app que escolheres, que pode guardar uma cópia.",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (busy) CircularProgressIndicator(Modifier.size(24.dp))
            }
        },
        confirmButton = { TextButton(onClick = { open() }, enabled = !busy) { Text("Abrir") } },
        dismissButton = {
            Row {
                TextButton(onClick = onDismiss) { Text("Fechar") }
                TextButton(
                    onClick = {
                        busy = true
                        AutoLockPause.active = true   // o seletor de pastas e outro ecra
                        saver.launch(name)
                    },
                    enabled = !busy,
                ) { Text("Guardar") }
            }
        },
    )
}

/** O que mostrar ao tocar num anexo: imagem em ecra inteiro, ou as opcoes de um ficheiro. */
@Composable
fun AttachmentOverlay(m: ChatMessage, load: suspend (String, String) -> ByteArray?, onDismiss: () -> Unit) {
    if (m.attachment?.fileName != null) FileActionsDialog(m, load, onDismiss) else AttachmentViewer(m, load, onDismiss)
}

/** Confirmacao antes de enviar um ficheiro escolhido. */
@Composable
fun SendFileDialog(uri: Uri, onSend: () -> Unit, onDismiss: () -> Unit) {
    val ctx = LocalContext.current
    val info = remember(uri) { FilePrep.describe(ctx, uri) }
    val tooBig = (info.second ?: 0L) > FilePrep.MAX_BYTES
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Enviar ficheiro") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(info.first, style = MaterialTheme.typography.bodyLarge, maxLines = 3, overflow = TextOverflow.Ellipsis)
                info.second?.let { Text(formatSize(it), style = MaterialTheme.typography.bodyMedium) }
                Text(
                    if (tooBig) "O ficheiro tem mais de 25 MB e não pode ser enviado."
                    else "Vai tal como está, cifrado no teu telemóvel antes de sair.",
                    style = MaterialTheme.typography.bodySmall,
                    color = if (tooBig) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = { TextButton(onClick = onSend, enabled = !tooBig) { Text("Enviar") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
}
