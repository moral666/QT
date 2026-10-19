package pt.qt.app

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
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
import androidx.core.content.ContextCompat
import uniffi.qt_ffi.CallAudio
import uniffi.qt_ffi.callFrameBytes

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
        val rec = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE,
            AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minRec, frame * 10),
        )
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
                    description = "Chamadas de voz a chegar e em curso (sem o nome de quem liga)."
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

/** Aviso "Chamada a chegar" quando a app esta aberta mas fora do ecra (sem o nome). */
object CallNotify {
    private const val ID = 8

    @SuppressLint("MissingPermission")   // verificada em Push.permissionGranted
    fun incoming(ctx: Context) {
        if (!Push.permissionGranted(ctx)) return
        CallService.ensureChannel(ctx)
        val open = Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val pi = PendingIntent.getActivity(ctx, 3, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(ctx, CallService.CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_sealer)
            .setContentTitle("SEALER")
            .setContentText("Chamada de voz a chegar")
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setContentIntent(pi)
            .setAutoCancel(true)
            .build()
        NotificationManagerCompat.from(ctx).notify(ID, n)
    }

    fun clear(ctx: Context) {
        NotificationManagerCompat.from(ctx).cancel(ID)
    }
}
