package pt.qt.app

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.MediaPlayer
import android.net.Uri
import android.media.ToneGenerator
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.os.Process
import android.os.SystemClock
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
import java.lang.ref.WeakReference

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
 *
 * `managed` (0.9.52): a chamada esta no sistema de chamadas do Android (SystemCalls). O modo de
 * chamada, o foco do audio e o caminho do som (auricular, altifalante, Bluetooth) sao do Telecom:
 * aqui nao se mexe neles, so se espera pelo modo de chamada antes de criar o microfone.
 */
class CallAudioIO(private val ctx: Context, private val audio: CallAudio, private val managed: Boolean = false) {
    private val am = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    @Volatile private var running = false
    @Volatile var muted = false
    /** O microfone deixou de funcionar a meio da chamada (outra app ficou com ele). */
    @Volatile var onMicLost: (() -> Unit)? = null
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
        // No sistema de chamadas e o Telecom que os poe: so se espera pelo modo.
        if (managed) {
            SystemCalls.awaitCommMode(ctx)
        } else {
            am.mode = AudioManager.MODE_IN_COMMUNICATION
            requestFocus()
        }
        val minRec = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        // 200 ms de folga: se a thread do microfone se atrasar, as amostras ficam a espera
        // em vez de se perderem (nao acrescenta atraso, le-se assim que ha 20 ms).
        val rec = try {
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minRec, frame * 10),
            )
        } catch (e: Exception) {
            leaveMode()
            throw e
        }
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            leaveMode()
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
            leaveMode()
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
            var err = 0
            while (off < frame && running) {
                val n = rec.read(buf, off, frame - off)
                if (n <= 0) { err = n; break }
                off += n
            }
            if (off < frame) {
                // O microfone morreu (outra app ficou com ele, mudanca de saida): para de ler.
                if (err == AudioRecord.ERROR_DEAD_OBJECT) {
                    Log.w(TAG, "microfone: perdido")
                    onMicLost?.invoke()   // avisa (0.9.23; antes o outro lado so deixava de ouvir)
                    break
                }
                // Outro erro: espera um pouco em vez de repetir sem pausa (antes gastava 100% de um nucleo).
                if (running) SystemClock.sleep(10)
                continue
            }
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

    /** Altifalante (true) ou auricular (false). No sistema de chamadas a saida e escolhida por la. */
    fun setSpeaker(on: Boolean) {
        if (!managed) CallRoute.speaker(am, on)
    }

    /** Fora do modo de chamada (so sem o sistema de chamadas: com ele e o Telecom que o faz). */
    private fun leaveMode() {
        if (managed) return
        am.mode = AudioManager.MODE_NORMAL
        abandonFocus()
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
        leaveMode()
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

/** Saida do som das chamadas: altifalante ou auricular (tambem antes de atender, 0.9.30). */
object CallRoute {
    fun speaker(am: AudioManager, on: Boolean) {
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
}

/** Toque de chamada a chegar (toque + vibracao) e o "tuu... tuu" de quem liga. */
class CallSounds(private val ctx: Context) {
    private var player: MediaPlayer? = null
    private var tone: ToneGenerator? = null
    private val am = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    /** O "tuu... tuu" pos o telemovel em modo de chamada (e e este que o tira, se ninguem atender). */
    private var routed = false
    private var focus: AudioFocusRequest? = null

    fun ringIncoming() {
        stop()
        runCatching {
            // Toque proprio da SEALER (res/raw/sealer_toque.ogg), nao o toque do telemovel.
            player = MediaPlayer().apply {
                setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                        .build(),
                )
                setDataSource(ctx, CallNotify.ringUri(ctx))
                isLooping = true
                prepare()
                start()
            }
        }.onFailure { player?.release(); player = null }
        // Modo silencio: nem som nem vibracao (0.9.23; antes vibrava na mesma). O som ja segue
        // o volume do toque do telemovel (fica mudo em silencio e em vibracao).
        val am = ctx.getSystemService(Context.AUDIO_SERVICE) as? AudioManager
        if (am?.ringerMode == AudioManager.RINGER_MODE_SILENT) return
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

    /**
     * "tuu... tuu" de quem liga. Desde a 0.9.30 o telemovel entra logo em modo de chamada: o toque
     * sai pelo auricular (ou pelo altifalante, se `speaker`) e o botao do altifalante ja funciona
     * antes de atender. As outras apps baixam o som, como numa chamada normal.
     *
     * `managed` (0.9.52): a chamada esta no sistema de chamadas, que ja pos o modo de chamada e
     * escolheu a saida (ex.: o auricular Bluetooth). Aqui so toca o "tuu... tuu".
     */
    fun ringback(speaker: Boolean = false, managed: Boolean = false) {
        stop(keepRoute = true)
        if (!managed) runCatching {
            am.mode = AudioManager.MODE_IN_COMMUNICATION
            routed = true
            requestFocus()
            CallRoute.speaker(am, speaker)
        }
        runCatching {
            // Volume 90 (antes 60, mal se ouvia nos Galaxy).
            tone = ToneGenerator(AudioManager.STREAM_VOICE_CALL, 90).apply { startTone(ToneGenerator.TONE_SUP_RINGTONE) }
        }
    }

    /** Altifalante durante o "tuu... tuu" (antes de atender). */
    fun setSpeaker(on: Boolean) {
        if (routed) runCatching { CallRoute.speaker(am, on) }
    }

    /**
     * Para o toque. `keepRoute`: a chamada foi atendida e o microfone/altifalante (CallAudioIO)
     * fica com o modo de chamada e a saida escolhida; sem isso, o som do telemovel volta ao normal.
     */
    fun stop(keepRoute: Boolean = false) {
        player?.let { runCatching { it.stop() }; runCatching { it.release() } }
        player = null
        tone?.let { runCatching { it.stopTone(); it.release() } }
        tone = null
        runCatching { ContextCompat.getSystemService(ctx, Vibrator::class.java)?.cancel() }
        abandonFocus()
        if (routed && !keepRoute) leaveRoute()
        routed = false
    }

    /** Som do telemovel de volta ao normal (auricular desligado, fora do modo de chamada). */
    fun leaveRoute() {
        // Numa chamada GSM o modo e dela: mexer-lhe cortava o som dessa chamada (0.9.52).
        if (am.mode == AudioManager.MODE_IN_CALL) return
        runCatching { CallRoute.speaker(am, false) }
        runCatching { am.mode = AudioManager.MODE_NORMAL }
    }

    private fun requestFocus() {
        if (Build.VERSION.SDK_INT >= 26) {
            val f = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
                .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION).build())
                .build()
            am.requestAudioFocus(f)
            focus = f
        }
    }

    private fun abandonFocus() {
        if (Build.VERSION.SDK_INT >= 26) focus?.let { runCatching { am.abandonAudioFocusRequest(it) } }
        focus = null
    }
}

/** O botao "Desligar" da notificacao chega aqui (o ViewModel regista-se). */
object CallControl {
    @Volatile var hangup: (() -> Unit)? = null
}

/**
 * Servico em primeiro plano durante a chamada: o Android so deixa usar o microfone (e a camara)
 * com a app fora do ecra se houver um. Mostra a notificacao "Chamada em curso", sem o nome de
 * quem esta do outro lado.
 *
 * Desde a 0.9.42 (0010):
 *  * Tipos (Android 11+): `phoneCall` sempre; `microphone` com a permissao do microfone;
 *    `camera` so enquanto a camara esta ligada (em segundo plano ela pausa, ver MainViewModel).
 *  * So ARRANCA com a app no ecra (ligar, atender): o Android 12+ nao deixa arrancar um servico
 *    destes em segundo plano e o 14+ so da microfone e camara a um servico arrancado no ecra.
 *    Depois, mudar de tipo vai direto ao servico que ja corre (`startForeground`), sem outro
 *    `startForegroundService` (que em segundo plano dava ForegroundServiceStartNotAllowedException).
 *  * Um tipo recusado (SecurityException: permissao retirada, por exemplo) tenta com menos:
 *    sem camara e, so ao arrancar, so `phoneCall`. Nunca para sem ter chamado `startForeground`
 *    (o Android fechava a app) e nunca com `stopService` de fora: so o proprio servico para,
 *    quando ja nao ha chamada.
 */
class CallService : Service() {
    /** O ultimo pedido entregue: `stopSelfResult` so para se nao houver um mais novo a caminho. */
    private var lastStartId = 0
    /** Tipos com que esta em primeiro plano agora; -1 = ainda nao esta. */
    private var current = -1

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ref = WeakReference(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        lastStartId = startId
        ref = WeakReference(this)
        if (intent?.action == ACTION_HANGUP) {
            CallControl.hangup?.invoke()
            if (!wanted) finish()
            return START_NOT_STICKY
        }
        promote()
        // A chamada acabou antes de o servico arrancar: ja chamou startForeground, pode parar.
        if (!wanted) finish()
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        if (ref?.get() === this) ref = null
        super.onDestroy()
    }

    private fun granted(permission: String) =
        ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED

    /** Primeiro plano com os tipos que a chamada quer agora (thread principal). */
    private fun promote() {
        if (!wanted && current >= 0) return
        val n = notification(this, videoText)
        if (Build.VERSION.SDK_INT >= 30) {
            val phone = ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL
            val mic = if (granted(Manifest.permission.RECORD_AUDIO)) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
            val cam = if (wantCamera && granted(Manifest.permission.CAMERA)) ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA else 0
            val full = phone or mic or cam
            // Ja em primeiro plano: nunca descer abaixo do microfone (a voz cortava em segundo plano).
            val tries = if (current < 0) listOf(full, phone or mic, phone) else listOf(full, phone or mic)
            for (types in tries.distinct()) {
                try {
                    startForeground(NOTIFICATION_ID, n, types)
                    current = types
                    return
                } catch (e: Exception) {
                    Log.w(TAG, "servico da chamada: ${e.javaClass.simpleName} (tipos $types)")
                }
            }
        } else {
            // Android 7 a 10: sem tipos de microfone/camara (ficam os do manifesto).
            try {
                startForeground(NOTIFICATION_ID, n)
                current = 0
            } catch (e: Exception) {
                Log.w(TAG, "servico da chamada: ${e.javaClass.simpleName}")
            }
        }
    }

    /** Sem chamada: sai do primeiro plano e para (so se nao houver outro pedido a caminho). */
    private fun finish() {
        if (wanted) return
        if (ref?.get() === this) ref = null
        if (current >= 0) runCatching { stopForeground(STOP_FOREGROUND_REMOVE) }
        current = -1
        stopSelfResult(lastStartId)
    }

    companion object {
        private const val ACTION_HANGUP = "pt.qt.app.HANGUP"
        private const val NOTIFICATION_ID = 7
        const val CHANNEL = "chamadas"
        private val main = Handler(Looper.getMainLooper())
        /** O servico a correr (para lhe mudar os tipos sem novo startForegroundService). */
        @Volatile private var ref: WeakReference<CallService>? = null
        /** Ha uma chamada que quer o servico. `false`: o servico para assim que puder. */
        @Volatile private var wanted = false
        @Volatile private var wantCamera = false
        /** "Videochamada em curso" (fica ate a chamada acabar, mesmo com a camara em pausa). */
        @Volatile private var videoText = false

        fun ensureChannel(ctx: Context) {
            if (Build.VERSION.SDK_INT < 26) return
            val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
            if (nm.getNotificationChannel(CHANNEL) != null) return
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, s(R.string.t_chamadas), NotificationManager.IMPORTANCE_HIGH).apply {
                    description = s(R.string.t_chamadas_de_voz_em_curso_e)
                    setSound(null, null)
                },
            )
        }

        private fun notification(ctx: Context, video: Boolean): Notification {
            ensureChannel(ctx)
            val open = Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            val pi = PendingIntent.getActivity(ctx, 1, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            val hang = PendingIntent.getService(
                ctx, 2, Intent(ctx, CallService::class.java).setAction(ACTION_HANGUP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            return NotificationCompat.Builder(ctx, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_sealer)
                .setContentTitle("SEALER")
                .setContentText(s(if (video) R.string.t_videochamada_em_curso else R.string.t_chamada_de_voz_em_curso))
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setCategory(NotificationCompat.CATEGORY_CALL)
                .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
                .setContentIntent(pi)
                .addAction(0, s(R.string.t_desligar), hang)
                .build()
        }

        /**
         * A chamada comecou (ligar ou atender, sempre com a app no ecra). `camera`: a minha camara
         * ligada; `video`: videochamada (o texto da notificacao). Ja a correr: so muda os tipos.
         */
        fun start(ctx: Context, camera: Boolean = false, video: Boolean = camera) {
            wanted = true
            wantCamera = camera
            videoText = video || camera
            val svc = ref?.get()
            if (svc != null) {
                main.post { svc.promote() }
                return
            }
            runCatching { ContextCompat.startForegroundService(ctx, Intent(ctx, CallService::class.java)) }
                .onFailure { Log.w(TAG, "servico da chamada: ${it.javaClass.simpleName}") }
        }

        /**
         * A app voltou ao ecra com uma chamada: se o servico nao chegou a arrancar (o Android pode
         * recusar o arranque fora do ecra), tenta outra vez agora.
         */
        fun ensure(ctx: Context) {
            if (!wanted || ref?.get() != null) return
            runCatching { ContextCompat.startForegroundService(ctx, Intent(ctx, CallService::class.java)) }
                .onFailure { Log.w(TAG, "servico da chamada: ${it.javaClass.simpleName}") }
        }

        /**
         * A camara ligou ou desligou (ou pausou): o servico passa a ser (ou deixa de ser) do tipo
         * camara. Ligar so com a app no ecra. Ainda a arrancar: o arranque ja usa o valor novo.
         */
        @Suppress("UNUSED_PARAMETER")
        fun update(ctx: Context, camera: Boolean) {
            if (!wanted) return
            wantCamera = camera
            if (camera) videoText = true
            val svc = ref?.get() ?: return
            main.post { svc.promote() }
        }

        /** A chamada acabou: o servico sai do primeiro plano e para (ele proprio, sem stopService). */
        @Suppress("UNUSED_PARAMETER")
        fun stop(ctx: Context) {
            wanted = false
            wantCamera = false
            videoText = false
            val svc = ref?.get() ?: return
            main.post { svc.finish() }
        }
    }
}

/** Ecra apagado ao encostar o telemovel ao ouvido (so nas chamadas de voz, ver CallScreen). */
object CallProximity {
    /** `null`: o telemovel nao tem sensor de proximidade (ou o Android nao deixa). */
    @SuppressLint("WakelockTimeout")   // largado quando o ecra da chamada sai ou passa a video
    fun acquire(ctx: Context): PowerManager.WakeLock? {
        val pm = ctx.getSystemService(Context.POWER_SERVICE) as? PowerManager ?: return null
        if (!pm.isWakeLockLevelSupported(PowerManager.PROXIMITY_SCREEN_OFF_WAKE_LOCK)) return null
        return runCatching {
            pm.newWakeLock(PowerManager.PROXIMITY_SCREEN_OFF_WAKE_LOCK, "sealer:chamada").apply {
                setReferenceCounted(false)
                acquire()
            }
        }.getOrNull()
    }

    /** Larga; se o telemovel ainda estiver encostado, o ecra so acende quando o afastar. */
    fun release(lock: PowerManager.WakeLock) {
        runCatching { if (lock.isHeld) lock.release(PowerManager.RELEASE_FLAG_WAIT_FOR_NO_PROXIMITY) }
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
    /** v2 (0.9.8): toque proprio. O Android nao deixa mudar o som de um canal ja criado. */
    const val RING_CHANNEL = "chamadas_toque_v2"
    private const val OLD_RING_CHANNEL = "chamadas_toque"

    fun ringUri(ctx: Context): Uri = Uri.parse("android.resource://${ctx.packageName}/${R.raw.sealer_toque}")
    /** A Activity abre por cima do bloqueio do telemovel so com estas acoes. */
    const val ACTION_SHOW = "pt.qt.app.CHAMADA"
    const val ACTION_ANSWER = "pt.qt.app.ATENDER"
    const val ACTION_DECLINE = "pt.qt.app.RECUSAR"
    const val EXTRA_TOKEN = "pt.qt.app.CHAVE"

    private fun ensureRingChannel(ctx: Context) {
        if (Build.VERSION.SDK_INT < 26) return
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        if (nm.getNotificationChannel(OLD_RING_CHANNEL) != null) nm.deleteNotificationChannel(OLD_RING_CHANNEL)
        if (nm.getNotificationChannel(RING_CHANNEL) != null) return
        nm.createNotificationChannel(
            NotificationChannel(RING_CHANNEL, s(R.string.t_chamadas_a_tocar), NotificationManager.IMPORTANCE_HIGH).apply {
                description = s(R.string.t_toque_das_chamadas_de_voz_com)
                setSound(
                    ringUri(ctx),
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

    /** Abre a app com uma acao da chamada (so as notificacoes e o sistema de chamadas da app). */
    fun activityLaunch(ctx: Context, action: String): Intent =
        Intent(ctx, MainActivity::class.java)
            .setAction(action)
            .putExtra(EXTRA_TOKEN, SealerCore.intentToken)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)

    private fun activityIntent(ctx: Context, action: String, code: Int): PendingIntent =
        PendingIntent.getActivity(ctx, code, activityLaunch(ctx, action), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    /**
     * Atendeu-se no sistema (auricular, carro) com a app fora do ecra: abre a app para atender,
     * como o "Atender" da notificacao (o microfone e o servico da chamada precisam dela no ecra).
     * O Android pode nao deixar abrir de fora do ecra; o "Atender" fica guardado e a app atende
     * assim que for aberta (a notificacao de ecra inteiro continua la).
     */
    fun openToAnswer(ctx: Context) {
        runCatching { ctx.startActivity(activityLaunch(ctx, ACTION_ANSWER)) }
            .onFailure { Log.w(TAG, "abrir para atender: ${it.javaClass.simpleName}") }
    }

    /**
     * Chamada a chegar: ecra inteiro (telemovel bloqueado) ou aviso no topo. `sound`: toca pelo canal.
     * Nao depende do interruptor "Notificacoes" (de proposito): e a forma de atender. Com ele
     * desligado o relay nao acorda o telemovel, por isso so toca com a app ainda a correr.
     * A chamada perdida e um aviso como os outros e segue o interruptor.
     *
     * `video`: videochamada, "Videochamada de [Nome]" (0010; com o botao de atender com a camara).
     * `kindKnown` false: com o bloqueio da app nada se decifra e nao se sabe se e de voz ou de
     * video (fica so "Chamada SEALER"). `update`: a mesma chamada passou a videochamada (a op do
     * video chegou depois da oferta): troca o texto sem voltar a tocar nem a abrir o ecra.
     */
    @SuppressLint("MissingPermission")   // verificada em Push.permissionGranted
    fun ringing(
        ctx: Context,
        name: String?,
        sound: Boolean,
        video: Boolean = false,
        kindKnown: Boolean = true,
        update: Boolean = false,
    ) {
        if (!Push.permissionGranted(ctx)) return
        if (sound) ensureRingChannel(ctx) else CallService.ensureChannel(ctx)
        val show = activityIntent(ctx, ACTION_SHOW, 4)
        val answer = activityIntent(ctx, ACTION_ANSWER, 5)
        val decline = PendingIntent.getBroadcast(
            ctx, 6, Intent(ctx, CallActionReceiver::class.java).setAction(ACTION_DECLINE),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val known = name?.takeIf { it.isNotBlank() }
        val title = known ?: s(R.string.t_chamada_sealer)
        val text = when {
            !kindKnown -> null
            video && known != null -> s(R.string.video_call_from, known)
            video -> s(R.string.video_call_sealer)
            else -> s(R.string.t_chamada_de_voz_sealer)
        }
        val channel = if (sound) RING_CHANNEL else CallService.CHANNEL
        fun base() = NotificationCompat.Builder(ctx, channel)
            .setSmallIcon(R.drawable.ic_stat_sealer)
            .setContentTitle(title)
            .apply { if (text != null) setContentText(text) }
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setOngoing(true)
            // Atualizacao: o toque continua (nao recomeca) e o ecra inteiro nao volta a abrir.
            .setOnlyAlertOnce(update)
            .setContentIntent(show)
            .setFullScreenIntent(show, true)
            .setTimeoutAfter(SealerCore.RING_MS)
        val n = runCatching {
            val person = Person.Builder().setName(title).setImportant(true).build()
            base().setStyle(NotificationCompat.CallStyle.forIncomingCall(person, decline, answer).setIsVideo(video)).build()
        }.getOrElse {
            // Sem o estilo de chamada: botoes normais.
            base().addAction(0, s(R.string.t_recusar), decline).addAction(0, s(R.string.t_atender), answer).build()
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
        val open = Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val pi = PendingIntent.getActivity(ctx, 7, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        fun base() = NotificationCompat.Builder(ctx, CallService.CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_sealer)
            .setContentTitle("SEALER")
            .setCategory("missed_call")   // Notification.CATEGORY_MISSED_CALL (Android 11+)
            .setContentIntent(pi)
            .setAutoCancel(true)
        val hidden = base().setContentText(s(R.string.t_chamada_perdida)).build()
        val n = base()
            .setContentText(if (name.isNullOrBlank()) s(R.string.t_chamada_perdida) else s(R.string.t_chamada_perdida_de, name))
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

    /**
     * Android 14+: a permissao de ecra inteiro pode estar desligada (vem desligada se a Play
     * Console nao tiver a declaracao). Sem ela, a chamada so aparece como um aviso pequeno.
     */
    fun fullScreenAllowed(ctx: Context): Boolean =
        if (Build.VERSION.SDK_INT >= 34) {
            ctx.getSystemService(NotificationManager::class.java)?.canUseFullScreenIntent() ?: true
        } else {
            true
        }

    /** Ecra das definicoes do Android onde se liga o ecra inteiro desta app (Android 14+). */
    fun fullScreenSettings(ctx: Context): Intent =
        if (Build.VERSION.SDK_INT >= 34) {
            Intent(android.provider.Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT, Uri.parse("package:${ctx.packageName}"))
        } else {
            Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${ctx.packageName}"))
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
