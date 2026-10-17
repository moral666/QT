package pt.qt.app

import android.content.Context
import android.media.MediaDataSource
import android.media.MediaPlayer
import android.media.MediaRecorder
import android.os.Build
import android.os.SystemClock
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import uniffi.qt_ffi.AttachmentInfo
import uniffi.qt_ffi.AttachmentState
import uniffi.qt_ffi.ChatMessage
import java.io.File

/** Mensagens de voz: AAC em MP4 (o Rust aceita `audio/mp4` ate 10 s; ver attachments.rs). */
const val VOICE_MIME = "audio/mp4"
/** Duracao maxima de uma gravacao. */
const val VOICE_MAX_MS = 10_000L
/** Gravacoes mais curtas do que isto sao descartadas (toque sem querer). */
private const val VOICE_MIN_MS = 600L

/**
 * Grava do microfone para um ficheiro temporario da app (cache). O ficheiro so existe ate
 * ser cifrado pelo Rust (`attachmentCreate`) e e apagado logo a seguir, ou ao cancelar.
 */
class VoiceRecorder(private val ctx: Context) {
    private var recorder: MediaRecorder? = null
    private var file: File? = null
    private var startedAt = 0L

    val isRecording: Boolean get() = recorder != null

    /** Comeca a gravar. Falha (excecao) se o microfone nao estiver disponivel. */
    fun start() {
        cancel()
        val dir = File(ctx.cacheDir, "voz").apply { mkdirs() }
        val f = File.createTempFile("voz", ".m4a", dir)
        val r = if (Build.VERSION.SDK_INT >= 31) MediaRecorder(ctx) else @Suppress("DEPRECATION") MediaRecorder()
        try {
            r.setAudioSource(MediaRecorder.AudioSource.MIC)
            r.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
            r.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
            r.setAudioChannels(1)
            r.setAudioSamplingRate(44_100)
            r.setAudioEncodingBitRate(48_000)
            // Rede de seguranca: quem para aos 10 s e o ecra (VoiceRecordBar).
            r.setMaxDuration((VOICE_MAX_MS + 400).toInt())
            r.setOutputFile(f.absolutePath)
            r.prepare()
            r.start()
        } catch (e: Exception) {
            r.release()
            f.delete()
            throw e
        }
        recorder = r
        file = f
        startedAt = SystemClock.elapsedRealtime()
    }

    /** Ms desde o inicio da gravacao. */
    fun elapsedMs(): Long = if (recorder == null) 0L else SystemClock.elapsedRealtime() - startedAt

    /** Para e devolve o ficheiro e a duracao, ou null se foi curta demais ou falhou. */
    fun stop(): Pair<File, Long>? {
        val r = recorder ?: return null
        val f = file
        val ms = elapsedMs().coerceAtMost(VOICE_MAX_MS)
        recorder = null
        file = null
        val ok = runCatching { r.stop() }.isSuccess   // stop() falha se quase nao gravou nada
        r.release()
        if (!ok || f == null || ms < VOICE_MIN_MS || f.length() == 0L) {
            f?.delete()
            return null
        }
        return f to ms
    }

    /** Para e deita fora a gravacao. */
    fun cancel() {
        stop()?.first?.delete()
    }
}

/** Barra de gravacao: tempo a contar ate 10 s, Cancelar e Enviar. Para sozinha aos 10 s. */
@Composable
fun VoiceRecordBar(recorder: VoiceRecorder, onCancel: () -> Unit, onSend: () -> Unit) {
    var elapsed by remember { mutableStateOf(0L) }
    LaunchedEffect(recorder) {
        while (recorder.isRecording) {
            elapsed = recorder.elapsedMs()
            if (elapsed >= VOICE_MAX_MS) {
                onSend()
                break
            }
            delay(100)
        }
    }
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(Modifier.size(12.dp).clip(CircleShape).background(Color(0xFFE53935)))
        Text("${formatVoice(elapsed)} / ${formatVoice(VOICE_MAX_MS)}", style = MaterialTheme.typography.bodyLarge)
        LinearProgressIndicator(progress = { (elapsed.toFloat() / VOICE_MAX_MS).coerceIn(0f, 1f) }, modifier = Modifier.weight(1f))
        TextButton(onClick = onCancel) { Text("Cancelar") }
        TextButton(onClick = onSend) { Text("Enviar") }
    }
}

/** "0:07". */
fun formatVoice(ms: Long): String {
    val s = (ms / 1000).coerceAtLeast(0)
    return "${s / 60}:${(s % 60).toString().padStart(2, '0')}"
}

/** Le o audio diretamente da memoria (decifrado pelo Rust), sem o escrever em disco. */
private class BytesDataSource(private val data: ByteArray) : MediaDataSource() {
    override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
        if (position >= data.size) return -1
        val n = minOf(size, data.size - position.toInt())
        System.arraycopy(data, position.toInt(), buffer, offset, n)
        return n
    }

    override fun getSize(): Long = data.size.toLong()
    override fun close() {}
}

/** Mensagem de voz na conversa: tocar / pausar, progresso e duracao. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun VoiceContent(
    m: ChatMessage,
    att: AttachmentInfo,
    progress: Float?,
    load: suspend (String, String) -> ByteArray?,
    onLongPress: () -> Unit,
) {
    val ready = m.outgoing || att.state == AttachmentState.READY
    val bytes = rememberAttachmentBytes(m, ready, load)
    val total = att.width.toLong()   // duracao em ms (ver VOICE_MIME no Rust)
    var playing by remember { mutableStateOf(false) }
    var position by remember { mutableIntStateOf(0) }
    val player = remember { mutableStateOf<MediaPlayer?>(null) }
    DisposableEffect(m.messageId) {
        onDispose {
            player.value?.release()
            player.value = null
        }
    }
    LaunchedEffect(playing) {
        while (playing) {
            position = runCatching { player.value?.currentPosition ?: 0 }.getOrDefault(0)
            delay(100)
        }
    }
    fun toggle() {
        val data = bytes ?: return
        val p = player.value ?: runCatching {
            MediaPlayer().apply {
                setDataSource(BytesDataSource(data))
                setOnCompletionListener {
                    playing = false
                    position = 0
                    runCatching { seekTo(0) }
                }
                prepare()
            }
        }.getOrNull()?.also { player.value = it } ?: return
        if (playing) {
            p.pause()
            playing = false
        } else {
            p.start()
            playing = true
        }
    }
    val busy = att.state == AttachmentState.UPLOADING || att.state == AttachmentState.DOWNLOADING
    val problem = when (att.state) {
        AttachmentState.EXPIRED -> "Já não está disponível (passaram mais de 24 horas)."
        AttachmentState.FAILED -> if (m.outgoing) "Não foi possível enviar." else "Não foi possível receber."
        else -> null
    }
    Column(Modifier.width(230.dp).combinedClickable(onClick = { toggle() }, onLongClick = onLongPress).padding(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier.size(40.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary),
                contentAlignment = Alignment.Center,
            ) {
                when {
                    busy && bytes == null -> if (progress != null && progress > 0f) {
                        CircularProgressIndicator(progress = { progress }, color = Color.White, modifier = Modifier.size(28.dp))
                    } else {
                        CircularProgressIndicator(color = Color.White, modifier = Modifier.size(28.dp))
                    }
                    playing -> Text("❚❚", color = Color.White, fontSize = 14.sp)
                    else -> Icon(Icons.Filled.PlayArrow, contentDescription = "Ouvir", tint = Color.White)
                }
            }
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                LinearProgressIndicator(
                    progress = { if (total > 0) (position.toFloat() / total).coerceIn(0f, 1f) else 0f },
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.size(4.dp))
                Text(
                    if (playing || position > 0) formatVoice(position.toLong()) else formatVoice(total),
                    style = MaterialTheme.typography.labelSmall,
                )
            }
        }
        if (problem != null) Text(problem, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.error)
    }
}
