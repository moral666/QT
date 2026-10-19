package pt.qt.app

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.Ringtone
import android.media.RingtoneManager
import android.media.ToneGenerator
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.os.IBinder
import android.os.Process
import android.os.VibrationEffect
import android.os.Vibrator
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.content.ContextCompat
import uniffi.qt_ffi.CallAudio
import uniffi.qt_ffi.callFrameBytes
import kotlinx.coroutines.launch

/**
 * Chamadas de voz (fase 4a: com a app aberta). Aqui so ha o que e do Android:
 * microfone, altifalante, toque, notificacao e o servico em primeiro plano. O Opus, a
 * cifra, o jitter buffer e a ligacao ao relay das chamadas estao no Rust (CallAudio).
 */

private const val TAG = "SealerCall"

/**
 * Microfone e altifalante de uma chamada: 48 kHz, mono, 16 bits, 20 ms de cada vez.
 *
 * A fonte VOICE_COMMUNICATION liga o cancelamento de eco e de ruido do proprio telemovel
 * (e ainda se pedem os efeitos explicitamente, quando existem). A saida e de voz
 * (USAGE_VOICE_COMMUNICATION): auricular por omissao, altifalante a pedido.
 */
class CallAudioIO(ctx: Context, private val audio: CallAudio) {
    private val am = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    @Volatile private var running = false
    @Volatile var muted = false
    private var record: AudioRecord? = null
    private var track: AudioTrack? = null
    private var aec: AcousticEchoCanceler? = null
    private var ns: NoiseSuppressor? = null
    private var focus: AudioFocusRequest? = null
    private val threads = mutableListOf<Thread>()

    /** Liga o microfone e o altifalante. A permissao do microfone e pedida antes (MainActivity). */
    @SuppressLint("MissingPermission")
    @Synchronized
    fun start() {
        if (running) return
        val frame = callFrameBytes().toInt()
        // Modo de chamada e foco ANTES de criar o microfone e o altifalante: mudar o modo
        // depois reencaminha o audio a meio e em muitos telemoveis o microfone para ~0,5 s.
        am.mode = AudioManager.MODE_IN_COMMUNICATION
        requestFocus()
        val minRec = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        // 200 ms de folga: se a thread do microfone se atrasar, as amostras ficam a espera
        // em vez de se perderem (nao acrescenta atraso, le-se assim que ha 20 ms).
        val rec = try {
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minRec, frame * 10),
            )
        } catch (e: Exception) {
            am.mode = AudioManager.MODE_NORMAL
            abandonFocus()
            throw e
        }
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            am.mode = AudioManager.MODE_NORMAL
            abandonFocus()
            throw IllegalStateException("microfone indisponivel")
        }
        val minTrack = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val tr = try {
            AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(RATE)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .build(),
                )
                .setBufferSizeInBytes(maxOf(minTrack, frame * 3))
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build()
        } catch (e: Exception) {
            rec.release()
            am.mode = AudioManager.MODE_NORMAL
            abandonFocus()
            throw e
        }
        if (AcousticEchoCanceler.isAvailable()) aec = AcousticEchoCanceler.create(rec.audioSessionId)?.apply { setEnabled(true) }
        if (NoiseSuppressor.isAvailable()) ns = NoiseSuppressor.create(rec.audioSessionId)?.apply { setEnabled(true) }
        record = rec
        track = tr
        running = true
        rec.startRecording()
        tr.play()
        threads += Thread({ captureLoop(rec, frame) }, "sealer-call-mic").apply { start() }
        threads += Thread({ playbackLoop(tr) }, "sealer-call-spk").apply { start() }
    }

    private fun captureLoop(rec: AudioRecord, frame: Int) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        val buf = ByteArray(frame)
        val silence = ByteArray(frame)
        while (running) {
            var off = 0
            while (off < frame && running) {
                val n = rec.read(buf, off, frame - off)
                if (n <= 0) break
                off += n
            }
            if (off < frame) continue
            try {
                audio.pushCapture(if (muted) silence else buf)
            } catch (e: Exception) {
                Log.w(TAG, "microfone: ${e.javaClass.simpleName}")
            }
        }
    }

    private fun playbackLoop(tr: AudioTrack) {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        while (running) {
            val pcm = try {
                audio.pullPlayback()
            } catch (e: Exception) {
                ByteArray(callFrameBytes().toInt())
            }
            // Bloqueia ate haver espaco: e o relogio do altifalante que marca o ritmo.
            tr.write(pcm, 0, pcm.size)
        }
    }

    /** Altifalante (true) ou auricular (false). */
    fun setSpeaker(on: Boolean) {
        if (Build.VERSION.SDK_INT >= 31) {
            if (on) {
                am.availableCommunicationDevices.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
                    ?.let { am.setCommunicationDevice(it) }
            } else {
                am.clearCommunicationDevice()
            }
        } else {
            @Suppress("DEPRECATION")
            am.isSpeakerphoneOn = on
        }
    }

    private fun requestFocus() {
        if (Build.VERSION.SDK_INT >= 26) {
            val f = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
                .setAudioAttributes(
                    AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION).build(),
                )
                .build()
            am.requestAudioFocus(f)
            focus = f
        } else {
            @Suppress("DEPRECATION")
            am.requestAudioFocus(null, AudioManager.STREAM_VOICE_CALL, AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
        }
    }

    /** Desliga o microfone e o altifalante e devolve o audio do telemovel ao normal. */
    @Synchronized
    fun stop() {
        if (!running && record == null) return
        running = false
        for (t in threads) runCatching { t.join(500) }
        threads.clear()
        record?.let { runCatching { it.stop() }; it.release() }
        record = null
        track?.let { runCatching { it.pause(); it.flush(); it.stop() }; it.release() }
        track = null
        aec?.release()
        aec = null
        ns?.release()
        ns = null
        runCatching { setSpeaker(false) }
        abandonFocus()
        am.mode = AudioManager.MODE_NORMAL
    }

    private fun abandonFocus() {
        if (Build.VERSION.SDK_INT >= 26) {
            focus?.let { am.abandonAudioFocusRequest(it) }
        } else {
            @Suppress("DEPRECATION")
            am.abandonAudioFocus(null)
        }
        focus = null
    }

    companion object {
        const val RATE = 48_000
    }
}

/** Toque de chamada a chegar (toque + vibracao) e o "tuu... tuu" de quem liga. */
class CallSounds(private val ctx: Context) {
    private var ringtone: Ringtone? = null
    private var tone: ToneGenerator? = null

    fun ringIncoming() {
        stop()
        runCatching {
            val uri = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE)
            ringtone = RingtoneManager.getRingtone(ctx, uri)?.apply {
                audioAttributes = AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build()
                if (Build.VERSION.SDK_INT >= 28) isLooping = true
                play()
            }
        }
        runCatching {
            val v = ContextCompat.getSystemService(ctx, Vibrator::class.java)
            val pattern = longArrayOf(0, 800, 800)
            if (Build.VERSION.SDK_INT >= 26) {
                v?.vibrate(VibrationEffect.createWaveform(pattern, 0))
            } else {
                @Suppress("DEPRECATION")
                v?.vibrate(pattern, 0)
            }
        }
    }

    fun ringback() {
        stop()
        runCatching {
            tone = ToneGenerator(AudioManager.STREAM_VOICE_CALL, 60).apply { startTone(ToneGenerator.TONE_SUP_RINGTONE) }
        }
    }

    fun stop() {
        ringtone?.let { runCatching { it.stop() } }
        ringtone = null
        tone?.let { runCatching { it.stopTone(); it.release() } }
        tone = null
        runCatching { ContextCompat.getSystemService(ctx, Vibrator::class.java)?.cancel() }
    }
}

/** O botao "Desligar" da notificacao chega aqui (o ViewModel regista-se). */
object CallControl {
    @Volatile var hangup: (() -> Unit)? = null
}

/**
 * Servico em primeiro plano durante a chamada: o Android so deixa usar o microfone com a
 * app fora do ecra se houver um (tipo "microphone"). Mostra a notificacao "Chamada em
 * curso", sem o nome de quem esta do outro lado.
 */
class CallService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_HANGUP) {
            CallControl.hangup?.invoke()
            return START_NOT_STICKY
        }
        val n = notification(this)
        try {
            if (Build.VERSION.SDK_INT >= 30) {
                startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
            } else {
                startForeground(NOTIFICATION_ID, n)
            }
        } catch (e: Exception) {
            // Ex.: sem permissao do microfone. A chamada continua enquanto a app estiver aberta.
            Log.w(TAG, "servico da chamada: ${e.javaClass.simpleName}")
            stopSelf()
        }
        return START_NOT_STICKY
    }

    companion object {
        private const val ACTION_HANGUP = "pt.qt.app.HANGUP"
        private const val NOTIFICATION_ID = 7
        const val CHANNEL = "chamadas"

        fun ensureChannel(ctx: Context) {
            if (Build.VERSION.SDK_INT < 26) return
            val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
            if (nm.getNotificationChannel(CHANNEL) != null) return
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, "Chamadas", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "Chamadas de voz em curso e a chegar com a app aberta."
                    setSound(null, null)
                },
            )
        }

        private fun notification(ctx: Context): Notification {
            ensureChannel(ctx)
            val open = Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            val pi = PendingIntent.getActivity(ctx, 1, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            val hang = PendingIntent.getService(
                ctx, 2, Intent(ctx, CallService::class.java).setAction(ACTION_HANGUP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            return NotificationCompat.Builder(ctx, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_sealer)
                .setContentTitle("SEALER")
                .setContentText("Chamada de voz em curso")
                .setOngoing(true)
                .setCategory(NotificationCompat.CATEGORY_CALL)
                .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
                .setContentIntent(pi)
                .addAction(0, "Desligar", hang)
                .build()
        }

        fun start(ctx: Context) {
            runCatching { ContextCompat.startForegroundService(ctx, Intent(ctx, CallService::class.java)) }
                .onFailure { Log.w(TAG, "servico da chamada: ${it.javaClass.simpleName}") }
        }

        fun stop(ctx: Context) {
            ctx.stopService(Intent(ctx, CallService::class.java))
        }
    }
}

/**
 * Notificacoes das chamadas a chegar (fase 4b): ecra inteiro por cima do bloqueio do
 * telemovel, com "Atender" e "Recusar" (estilo de chamada do Android).
 *
 *  - Com a app fechada ou em segundo plano ([SealerCore]): toca pelo canal "chamadas_toque"
 *    (toque do telemovel, repetido ate atender, recusar ou passarem 45 s).
 *  - Com a app aberta mas fora do ecra (o ecra ja toca com [CallSounds]): canal sem som.
 *
 * O nome so aparece se a app o souber (sem o bloqueio da app). Com o bloqueio ativo toca
 * "Chamada SEALER" e o codigo e pedido ao atender.
 */
object CallNotify {
    private const val ID = 8
    private const val MISSED_ID = 9
    const val RING_CHANNEL = "chamadas_toque"
    /** A Activity abre por cima do bloqueio do telemovel so com estas acoes. */
    const val ACTION_SHOW = "pt.qt.app.CHAMADA"
    const val ACTION_ANSWER = "pt.qt.app.ATENDER"
    const val ACTION_DECLINE = "pt.qt.app.RECUSAR"
    const val EXTRA_TOKEN = "pt.qt.app.CHAVE"

    private fun ensureRingChannel(ctx: Context) {
        if (Build.VERSION.SDK_INT < 26) return
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        if (nm.getNotificationChannel(RING_CHANNEL) != null) return
        nm.createNotificationChannel(
            NotificationChannel(RING_CHANNEL, "Chamadas a tocar", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "Toque das chamadas de voz com a app fechada."
                setSound(
                    RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                        .build(),
                )
                enableVibration(true)
                vibrationPattern = longArrayOf(0, 800, 800)
                lockscreenVisibility = Notification.VISIBILITY_PUBLIC
            },
        )
    }

    private fun activityIntent(ctx: Context, action: String, code: Int): PendingIntent {
        val i = Intent(ctx, MainActivity::class.java)
            .setAction(action)
            .putExtra(EXTRA_TOKEN, SealerCore.intentToken)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        return PendingIntent.getActivity(ctx, code, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** Chamada a chegar: ecra inteiro (telemovel bloqueado) ou aviso no topo. `sound`: toca pelo canal. */
    @SuppressLint("MissingPermission")   // verificada em Push.permissionGranted
    fun ringing(ctx: Context, name: String?, sound: Boolean) {
        if (!Push.permissionGranted(ctx)) return
        if (sound) ensureRingChannel(ctx) else CallService.ensureChannel(ctx)
        val show = activityIntent(ctx, ACTION_SHOW, 4)
        val answer = activityIntent(ctx, ACTION_ANSWER, 5)
        val decline = PendingIntent.getBroadcast(
            ctx, 6, Intent(ctx, CallActionReceiver::class.java).setAction(ACTION_DECLINE),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val title = name?.takeIf { it.isNotBlank() } ?: "Chamada SEALER"
        val channel = if (sound) RING_CHANNEL else CallService.CHANNEL
        fun base() = NotificationCompat.Builder(ctx, channel)
            .setSmallIcon(R.drawable.ic_stat_sealer)
            .setContentTitle(title)
            .setContentText("Chamada de voz SEALER")
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setOngoing(true)
            .setContentIntent(show)
            .setFullScreenIntent(show, true)
            .setTimeoutAfter(SealerCore.RING_MS)
        val n = runCatching {
            val person = Person.Builder().setName(title).setImportant(true).build()
            base().setStyle(NotificationCompat.CallStyle.forIncomingCall(person, decline, answer)).build()
        }.getOrElse {
            // Sem o estilo de chamada: botoes normais.
            base().addAction(0, "Recusar", decline).addAction(0, "Atender", answer).build()
        }
        if (sound) n.flags = n.flags or Notification.FLAG_INSISTENT
        try {
            NotificationManagerCompat.from(ctx).notify(ID, n)
        } catch (e: Exception) {
            Log.w(TAG, "notificacao da chamada: ${e.javaClass.simpleName}")
        }
    }

    /** Chamada perdida (so com a app fechada). No ecra bloqueado aparece sem o nome. */
    @SuppressLint("MissingPermission")   // verificada em Push.permissionGranted
    fun missed(ctx: Context, name: String?) {
        if (!Push.permissionGranted(ctx) || !Push.isOn(ctx)) return
        CallService.ensureChannel(ctx)
        val open = Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val pi = PendingIntent.getActivity(ctx, 7, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        fun base() = NotificationCompat.Builder(ctx, CallService.CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_sealer)
            .setContentTitle("SEALER")
            .setCategory("missed_call")   // Notification.CATEGORY_MISSED_CALL (Android 11+)
            .setContentIntent(pi)
            .setAutoCancel(true)
        val hidden = base().setContentText("Chamada perdida").build()
        val n = base()
            .setContentText(if (name.isNullOrBlank()) "Chamada perdida" else "Chamada perdida de $name")
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setPublicVersion(hidden)
            .build()
        try {
            NotificationManagerCompat.from(ctx).notify(MISSED_ID, n)
        } catch (e: Exception) {
            // Permissao retirada entretanto.
        }
    }

    fun clear(ctx: Context) {
        NotificationManagerCompat.from(ctx).cancel(ID)
    }

    fun clearMissed(ctx: Context) {
        NotificationManagerCompat.from(ctx).cancel(MISSED_ID)
    }
}

/** "Recusar" na notificacao da chamada a chegar. */
class CallActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != CallNotify.ACTION_DECLINE) return
        val ctx = context.applicationContext
        // O ecra tem a chamada (app aberta fora do ecra): recusa por la.
        if (SealerCore.ringing == null) {
            CallControl.hangup?.invoke()
            CallNotify.clear(ctx)
            return
        }
        val pending = goAsync()
        SealerCore.scope.launch {
            try {
                SealerCore.declineRinging(ctx)
            } finally {
                pending.finish()
            }
        }
    }
}
