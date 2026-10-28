package pt.qt.app

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.graphics.Matrix
import android.graphics.SurfaceTexture
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CameraMetadata
import android.hardware.camera2.CaptureRequest
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Log
import android.util.Range
import android.util.Size
import android.view.OrientationEventListener
import android.view.Surface
import android.view.TextureView
import uniffi.qt_ffi.CallAudio
import uniffi.qt_ffi.NetworkKind

/**
 * Videochamadas (fase 5b, com a app aberta). Aqui so ha o que e do Android: a camara
 * (Camera2), o codificador e o descodificador H.264 (MediaCodec, em hardware) e as vistas.
 * A cifra, os pacotes, a espera pelos atrasados, a sincronizacao com o audio, os pedidos de
 * imagem completa e o debito estao no Rust (`CallAudio.video*`): esta parte so aplica os
 * parametros que de la vem e passa imagens de um lado para o outro.
 *
 * Consentimento: o Rust so deixa sair video depois de o outro aceitar ver; antes disso a
 * camara so alimenta a pre-visualizacao (o que o codificador produz e recusado).
 */

private const val TAG = "SealerVideo"
private const val AVC = MediaFormat.MIMETYPE_VIDEO_AVC

/** A Activity por tras de um Context do Compose. */
tailrec fun Context.findActivity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.findActivity()
    else -> null
}

/** Rede do telemovel para o teto do video em ligacao direta (Wi-Fi sem limite de dados: 1080p). */
object VideoNet {
    fun current(ctx: Context): NetworkKind {
        val cm = ctx.getSystemService(ConnectivityManager::class.java) ?: return NetworkKind.UNKNOWN
        val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return NetworkKind.UNKNOWN
        return kind(caps)
    }

    private fun kind(caps: NetworkCapabilities): NetworkKind {
        val unmetered = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        val wifi = caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        return when {
            wifi && unmetered -> NetworkKind.WIFI
            // Dados moveis, ou Wi-Fi com limite (ex.: partilha de dados de outro telemovel).
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) || wifi -> NetworkKind.MOBILE
            else -> NetworkKind.UNKNOWN
        }
    }

    /** Avisa quando a rede muda (Wi-Fi <-> dados moveis). Devolve o que e preciso para parar. */
    fun watch(ctx: Context, onChange: (NetworkKind) -> Unit): (() -> Unit)? {
        val cm = ctx.getSystemService(ConnectivityManager::class.java) ?: return null
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) = onChange(kind(caps))
            override fun onLost(network: Network) = onChange(current(ctx))
        }
        return try {
            cm.registerDefaultNetworkCallback(cb)
            ({ runCatching { cm.unregisterNetworkCallback(cb) } })
        } catch (e: Exception) {
            null
        }
    }
}

/**
 * TextureView que mostra uma imagem a encher a vista (corta o que sobra), rodada e, se for
 * a camara da frente na pre-visualizacao, em espelho.
 */
class VideoTextureView(ctx: Context) : TextureView(ctx) {
    private var frameW = 0
    private var frameH = 0
    private var rotation = 0
    private var mirror = false

    /** Tamanho da imagem (antes de rodar), quanto rodar (graus, sentido dos ponteiros) e espelho. */
    fun setFrame(width: Int, height: Int, rotationDegrees: Int, mirrored: Boolean) {
        if (width == frameW && height == frameH && rotationDegrees == rotation && mirrored == mirror) return
        frameW = width
        frameH = height
        rotation = rotationDegrees
        mirror = mirrored
        applyTransform()
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        applyTransform()
    }

    private fun applyTransform() {
        val vw = width.toFloat()
        val vh = height.toFloat()
        if (vw <= 0f || vh <= 0f || frameW <= 0 || frameH <= 0) return
        val cx = vw / 2f
        val cy = vh / 2f
        val m = Matrix()
        // A TextureView estica a imagem ate a vista: desfaz, roda e enche (cortando).
        m.setScale(frameW / vw, frameH / vh, cx, cy)
        m.postRotate(rotation.toFloat(), cx, cy)
        val (cw, ch) = if (rotation % 180 == 0) frameW.toFloat() to frameH.toFloat() else frameH.toFloat() to frameW.toFloat()
        val scale = maxOf(vw / cw, vh / ch)
        m.postScale(if (mirror) -scale else scale, scale, cx, cy)
        setTransform(m)
    }
}

/**
 * Camara + codificador H.264. Segue `CallAudio.videoParams()` (debito, resolucao, imagens/s,
 * imagem completa) e entrega cada imagem codificada ao Rust (`videoPush`), com a rotacao.
 * Toda a configuracao corre numa thread propria ([handler]).
 */
class VideoSender(private val ctx: Context, private val audio: CallAudio) {
    private val cm = ctx.getSystemService(Context.CAMERA_SERVICE) as CameraManager
    private val thread = HandlerThread("sealer-video-cam").apply { start() }
    private val handler = Handler(thread.looper)
    @Volatile var front = true
        private set
    @Volatile private var running = false
    private var camera: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    @Volatile private var encoder: MediaCodec? = null
    private var encoderSurface: Surface? = null
    private var preview: SurfaceTexture? = null
    private var previewSurface: Surface? = null
    private var encSize: Size? = null
    private var encFps = 0
    private var lastBitrate = 0
    private var lastKeySeq: UInt? = null
    private var sensorOrientation = 90
    private var timestampRealtime = false
    @Volatile private var csd: ByteArray? = null
    @Volatile private var deviceOrientation = 0
    private var outThread: Thread? = null
    private var paramsThread: Thread? = null
    /** Tamanho da pre-visualizacao (para a vista a por direita). */
    @Volatile var previewSize: Size? = null
        private set
    /** A camara nao abriu ou caiu (outra app ficou com ela). */
    @Volatile var onError: (() -> Unit)? = null
    /** A pre-visualizacao mudou de tamanho ou de camara. */
    @Volatile var onPreview: (() -> Unit)? = null

    private val orientation = object : OrientationEventListener(ctx) {
        override fun onOrientationChanged(o: Int) {
            if (o != OrientationEventListener.ORIENTATION_UNKNOWN) deviceOrientation = (o + 45) / 90 * 90 % 360
        }
    }

    fun start(useFront: Boolean = true) {
        front = useFront
        running = true
        runCatching { orientation.enable() }
        handler.post { openCamera() }
        paramsThread = Thread({ paramsLoop() }, "sealer-video-params").apply { start() }
    }

    /** Frontal <-> traseira. */
    fun switchCamera() {
        handler.post {
            front = !front
            closeCamera()
            openCamera()
        }
    }

    /** A vista da pre-visualizacao (pode chegar antes ou depois da camara abrir). */
    fun setPreview(st: SurfaceTexture?) {
        handler.post {
            if (st === preview && (st == null || previewSurface != null)) return@post
            preview = st
            previewSurface?.release()
            previewSurface = null
            if (camera != null) startSession()
        }
    }

    fun stop() {
        running = false
        runCatching { orientation.disable() }
        paramsThread?.interrupt()
        handler.post {
            closeCamera()
            previewSurface?.release()
            previewSurface = null
            thread.quitSafely()
        }
    }

    /** Graus (sentido dos ponteiros) para mostrar a imagem da camara direita. */
    private fun rotationDegrees(): Int {
        val d = if (front) -deviceOrientation else deviceOrientation
        return (sensorOrientation + d + 360) % 360
    }

    private fun pickCamera(wantFront: Boolean): String? {
        val facing = if (wantFront) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK
        return cm.cameraIdList.firstOrNull { cm.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == facing }
            ?: cm.cameraIdList.firstOrNull()
    }

    @SuppressLint("MissingPermission")   // pedida antes (MainActivity), ao carregar no botao
    private fun openCamera() {
        if (!running) return
        val id = runCatching { pickCamera(front) }.getOrNull() ?: return fail("sem camara")
        val ch = cm.getCameraCharacteristics(id)
        sensorOrientation = ch.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
        timestampRealtime = ch.get(CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE) ==
            CameraMetadata.SENSOR_INFO_TIMESTAMP_SOURCE_REALTIME
        try {
            cm.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(c: CameraDevice) {
                    if (!running) {
                        c.close()
                        return
                    }
                    camera = c
                    configureEncoder()
                    startSession()
                }

                override fun onDisconnected(c: CameraDevice) {
                    c.close()
                    if (camera === c) {
                        camera = null
                        fail("camara desligada")
                    }
                }

                override fun onError(c: CameraDevice, error: Int) {
                    c.close()
                    if (camera === c) camera = null
                    fail("erro da camara $error")
                }
            }, handler)
        } catch (e: Exception) {
            fail("abrir: ${e.javaClass.simpleName}")
        }
    }

    private fun fail(why: String) {
        Log.w(TAG, "camara: $why")
        onError?.invoke()
    }

    private fun closeCamera() {
        runCatching { session?.close() }
        session = null
        runCatching { camera?.close() }
        camera = null
        releaseEncoder()
    }

    /** O tamanho suportado mais proximo do pedido (16:9 primeiro, nunca maior do que o pedido). */
    private fun chooseSize(sizes: Array<Size>, w: Int, h: Int): Size {
        val wide = sizes.filter { kotlin.math.abs(it.width * 9 - it.height * 16) <= it.height }
        val pool = wide.ifEmpty { sizes.toList() }
        return pool.filter { it.height <= h && it.width <= maxOf(w, h * 16 / 9 + 16) }.maxByOrNull { it.width * it.height }
            ?: pool.minByOrNull { it.width * it.height }
            ?: Size(640, 360)
    }

    private fun fpsRange(ch: CameraCharacteristics, fps: Int): Range<Int>? {
        val ranges = ch.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES) ?: return null
        return ranges.filter { it.upper >= fps }.minByOrNull { (it.upper - fps) * 100 + (it.upper - it.lower) } ?: ranges.lastOrNull()
    }

    private fun avcFormat(size: Size, bitrate: Int, fps: Int, strict: Boolean, prepend: Boolean = true): MediaFormat =
        MediaFormat.createVideoFormat(AVC, size.width, size.height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            // Imagens completas so a pedido (o Rust pede quando o outro lado perde uma).
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 10)
            if (Build.VERSION.SDK_INT >= 23) setInteger(MediaFormat.KEY_PRIORITY, 0)
            // Sem esta chave (codificadores por software, ex. c2.android.avc.encoder) o SPS/PPS
            // vai a frente de cada imagem completa a partir do `csd` (ver `outputLoop`).
            if (prepend && Build.VERSION.SDK_INT >= 29) setInteger(MediaFormat.KEY_PREPEND_HEADER_TO_SYNC_FRAMES, 1)
            if (strict) {
                // Tempo real: debito constante e perfil base (sem imagens B, todos os descodificadores).
                setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR)
                setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline)
                setInteger(
                    MediaFormat.KEY_LEVEL,
                    if (size.height > 720) MediaCodecInfo.CodecProfileLevel.AVCLevel4 else MediaCodecInfo.CodecProfileLevel.AVCLevel31,
                )
            }
        }

    /**
     * Abre um codificador H.264 que aceite o formato. Tenta cada codificador do telemovel
     * (hardware primeiro) com tres formatos, do mais exigente ao mais simples: tempo real
     * (CBR, perfil base), so o essencial, e sem `KEY_PREPEND_HEADER_TO_SYNC_FRAMES` (que
     * alguns codificadores por software recusam com BAD_VALUE).
     */
    private fun openEncoder(size: Size, bitrate: Int, fps: Int): MediaCodec? {
        val names = runCatching {
            MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
                .filter { it.isEncoder && it.supportedTypes.any { t -> t.equals(AVC, ignoreCase = true) } }
                .sortedBy { if (Build.VERSION.SDK_INT >= 29 && it.isSoftwareOnly) 1 else 0 }
                .map { it.name }
        }.getOrDefault(emptyList())
        val attempts = listOf(true to true, false to true, false to false)
        for (name in names.ifEmpty { listOf<String?>(null) }) {
            val enc = try {
                if (name != null) MediaCodec.createByCodecName(name) else MediaCodec.createEncoderByType(AVC)
            } catch (e: Exception) {
                CallDiag.log(TAG, "codificador $name: ${e.javaClass.simpleName}")
                continue
            }
            for ((strict, prepend) in attempts) {
                try {
                    enc.configure(avcFormat(size, bitrate, fps, strict, prepend), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                    CallDiag.log(TAG, "codificador ${enc.name} ${size.width}x${size.height} strict=$strict prepend=$prepend")
                    return enc
                } catch (e: Exception) {
                    CallDiag.log(TAG, "codificador ${enc.name} strict=$strict prepend=$prepend: ${e.javaClass.simpleName}")
                    runCatching { enc.reset() }
                }
            }
            runCatching { enc.release() }
        }
        return null
    }

    /** (Re)cria o codificador para os parametros atuais. */
    private fun configureEncoder() {
        val cam = camera ?: return
        releaseEncoder()
        val p = audio.videoParams()
        val map = cm.getCameraCharacteristics(cam.id).get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return fail("sem tamanhos")
        val w = p.width.toInt().takeIf { it > 0 } ?: 640
        val h = p.height.toInt().takeIf { it > 0 } ?: 360
        val size = chooseSize(map.getOutputSizes(MediaCodec::class.java), w, h)
        val fps = p.fps.toInt().coerceIn(10, 30)
        val bitrate = p.bitrateBps.toInt().coerceAtLeast(100_000)
        val enc = openEncoder(size, bitrate, fps) ?: return fail("codificador: nenhum aceitou")
        val surface = enc.createInputSurface()
        enc.start()
        csd = null
        encoder = enc
        encoderSurface = surface
        encSize = size
        encFps = fps
        lastBitrate = bitrate
        lastKeySeq = p.keyframeSeq
        val prevSize = chooseSize(map.getOutputSizes(SurfaceTexture::class.java), 640, 360)
        previewSize = prevSize
        onPreview?.invoke()
        outThread = Thread({ outputLoop(enc) }, "sealer-video-enc").apply { start() }
    }

    private fun releaseEncoder() {
        val enc = encoder
        encoder = null
        if (enc != null) {
            runCatching { enc.stop() }
            runCatching { enc.release() }
        }
        outThread?.let { runCatching { it.join(300) } }
        outThread = null
        encoderSurface?.release()
        encoderSurface = null
    }

    /** Sessao da camara com a pre-visualizacao e o codificador. */
    private fun startSession() {
        val cam = camera ?: return
        runCatching { session?.close() }
        session = null
        val targets = ArrayList<Surface>()
        encoderSurface?.let { targets += it }
        val st = preview
        if (st != null) {
            previewSize?.let { st.setDefaultBufferSize(it.width, it.height) }
            val ps = previewSurface ?: Surface(st).also { previewSurface = it }
            targets += ps
        }
        if (targets.isEmpty()) return
        val ch = cm.getCameraCharacteristics(cam.id)
        try {
            @Suppress("DEPRECATION")
            cam.createCaptureSession(targets, object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(s: CameraCaptureSession) {
                    if (camera !== cam) {
                        s.close()
                        return
                    }
                    session = s
                    try {
                        val req = cam.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                            for (t in targets) addTarget(t)
                            set(CaptureRequest.CONTROL_MODE, CameraMetadata.CONTROL_MODE_AUTO)
                            fpsRange(ch, encFps.takeIf { it > 0 } ?: 30)?.let { set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it) }
                        }.build()
                        s.setRepeatingRequest(req, null, handler)
                    } catch (e: Exception) {
                        fail("pedido: ${e.javaClass.simpleName}")
                    }
                }

                override fun onConfigureFailed(s: CameraCaptureSession) = fail("sessao")
            }, handler)
        } catch (e: Exception) {
            fail("sessao: ${e.javaClass.simpleName}")
        }
    }

    /** A cada 250 ms: o Rust pode ter mudado o debito, a resolucao ou pedido imagem completa. */
    private fun paramsLoop() {
        while (running) {
            try {
                Thread.sleep(250)
            } catch (e: InterruptedException) {
                return
            }
            val p = runCatching { audio.videoParams() }.getOrNull() ?: continue
            handler.post { applyParams(p) }
        }
    }

    private fun applyParams(p: uniffi.qt_ffi.VideoEncoderParams) {
        val enc = encoder ?: return
        val cam = camera ?: return
        val map = cm.getCameraCharacteristics(cam.id).get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return
        val want = chooseSize(map.getOutputSizes(MediaCodec::class.java), p.width.toInt().coerceAtLeast(1), p.height.toInt().coerceAtLeast(1))
        val fps = p.fps.toInt().coerceIn(10, 30)
        if (want != encSize || fps != encFps) {
            // Resolucao nova: codificador novo (a primeira imagem sai completa).
            configureEncoder()
            startSession()
            return
        }
        val bitrate = p.bitrateBps.toInt().coerceAtLeast(100_000)
        val params = Bundle()
        if (kotlin.math.abs(bitrate - lastBitrate) > lastBitrate / 20) {
            params.putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, bitrate)
            lastBitrate = bitrate
        }
        if (p.keyframeSeq != lastKeySeq) {
            params.putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0)
            lastKeySeq = p.keyframeSeq
        }
        if (!params.isEmpty) runCatching { enc.setParameters(params) }
    }

    /** Ha quanto tempo a imagem foi captada (o relogio das marcas de tempo da camara). */
    private fun ageMs(ptsUs: Long): Int {
        val nowUs = (if (timestampRealtime) SystemClock.elapsedRealtimeNanos() else System.nanoTime()) / 1000
        val age = (nowUs - ptsUs) / 1000
        return if (age in 0..500) age.toInt() else 40
    }

    /** A imagem completa ja traz SPS/PPS? (Annex B: primeiro NAL do tipo 7) */
    private fun startsWithSps(b: ByteArray): Boolean {
        var i = 0
        while (i + 3 < b.size) {
            if (b[i].toInt() == 0 && b[i + 1].toInt() == 0) {
                val start = when {
                    b[i + 2].toInt() == 1 -> i + 3
                    b[i + 2].toInt() == 0 && b[i + 3].toInt() == 1 -> i + 4
                    else -> -1
                }
                if (start in 1 until b.size) return (b[start].toInt() and 0x1F) == 7
            }
            i++
        }
        return false
    }

    private fun outputLoop(enc: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        while (running && encoder === enc) {
            val idx = try {
                enc.dequeueOutputBuffer(info, 10_000)
            } catch (e: Exception) {
                break
            }
            if (idx < 0) continue
            try {
                val buf = enc.getOutputBuffer(idx)
                if (buf != null && info.size > 0) {
                    val bytes = ByteArray(info.size)
                    buf.position(info.offset)
                    buf.limit(info.offset + info.size)
                    buf.get(bytes)
                    if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                        csd = bytes
                    } else {
                        val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                        val c = csd
                        val data = if (key && c != null && !startsWithSps(bytes)) c + bytes else bytes
                        // `false` = o outro ainda nao aceitou (ou pausa): a imagem nao sai.
                        audio.videoPush(data, key, rotationDegrees().toUInt(), ageMs(info.presentationTimeUs).toUInt())
                    }
                }
                enc.releaseOutputBuffer(idx, false)
            } catch (e: Exception) {
                if (encoder !== enc) break
            }
        }
    }
}

/**
 * Descodificador H.264 do video do outro lado. Pede ao Rust a proxima imagem ja na hora
 * certa (`videoPull`) e mostra-a logo. Se o descodificador falhar, recomeca e pede uma
 * imagem completa ao outro lado.
 */
class VideoReceiver(private val audio: CallAudio, private val onFrame: (width: Int, height: Int, rotation: Int) -> Unit) {
    /** Quando chegou a ultima imagem do outro (`SystemClock.elapsedRealtime`); 0 = nenhuma ainda. */
    @Volatile var lastFrameAt = 0L
        private set
    @Volatile private var surface: Surface? = null
    @Volatile private var running = false
    private var codec: MediaCodec? = null
    private var thread: Thread? = null
    private val lock = Any()
    private var width = 0
    private var height = 0
    private var rotation = -1
    @Volatile private var surfaceChanged = false

    fun start() {
        if (running) return
        running = true
        thread = Thread({ loop() }, "sealer-video-dec").apply { start() }
    }

    /** O ecra do video (null: escondido). Muda o ecra: o descodificador recomeca. */
    fun setSurface(s: Surface?) {
        synchronized(lock) {
            if (s === surface) return
            surface = s
            surfaceChanged = true
        }
    }

    fun stop() {
        running = false
        thread?.let { runCatching { it.join(500) } }
        thread = null
    }

    private fun createDecoder(s: Surface): MediaCodec {
        val fmt = MediaFormat.createVideoFormat(AVC, 1280, 720).apply {
            setInteger(MediaFormat.KEY_MAX_WIDTH, 1920)
            setInteger(MediaFormat.KEY_MAX_HEIGHT, 1088)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 1 shl 20)
            if (Build.VERSION.SDK_INT >= 23) setInteger(MediaFormat.KEY_PRIORITY, 0)
            if (Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
        }
        val c = MediaCodec.createDecoderByType(AVC)
        c.configure(fmt, s, null, 0)
        c.start()
        return c
    }

    private fun release() {
        codec?.let { c ->
            runCatching { c.stop() }
            runCatching { c.release() }
        }
        codec = null
    }

    private fun loop() {
        val info = MediaCodec.BufferInfo()
        var pts = 0L
        while (running) {
            val f = runCatching { audio.videoPull(50u) }.getOrNull() ?: continue
            lastFrameAt = SystemClock.elapsedRealtime()
            val s = synchronized(lock) {
                if (surfaceChanged) {
                    surfaceChanged = false
                    release()
                }
                surface
            } ?: continue
            var c = codec
            if (c == null) {
                // So se recomeca numa imagem completa; ate la pede-se uma.
                if (!f.keyframe) {
                    audio.videoRequestKeyframe()
                    continue
                }
                c = try {
                    createDecoder(s)
                } catch (e: Exception) {
                    Log.w(TAG, "descodificador: ${e.javaClass.simpleName}")
                    continue
                }
                codec = c
            }
            try {
                val i = c.dequeueInputBuffer(20_000)
                if (i >= 0) {
                    val b = c.getInputBuffer(i) ?: continue
                    b.clear()
                    b.put(f.data)
                    c.queueInputBuffer(i, 0, f.data.size, pts, if (f.keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
                    pts += 33_333
                } else {
                    // Sem lugar: esta imagem perdeu-se e as seguintes dependem dela.
                    audio.videoRequestKeyframe()
                }
                drain(c, info, f.rotationDegrees.toInt())
            } catch (e: Exception) {
                Log.w(TAG, "descodificar: ${e.javaClass.simpleName}")
                release()
                audio.videoRequestKeyframe()
            }
        }
        synchronized(lock) { release() }
    }

    private fun drain(c: MediaCodec, info: MediaCodec.BufferInfo, rot: Int) {
        while (true) {
            val o = c.dequeueOutputBuffer(info, 0)
            when {
                o >= 0 -> c.releaseOutputBuffer(o, true)
                o == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    val fmt = c.outputFormat
                    val w = if (fmt.containsKey("crop-right")) fmt.getInteger("crop-right") - fmt.getInteger("crop-left") + 1 else fmt.getInteger(MediaFormat.KEY_WIDTH)
                    val h = if (fmt.containsKey("crop-bottom")) fmt.getInteger("crop-bottom") - fmt.getInteger("crop-top") + 1 else fmt.getInteger(MediaFormat.KEY_HEIGHT)
                    width = w
                    height = h
                    onFrame(w, h, if (rot >= 0) rot else 0)
                }
                else -> break
            }
        }
        if (rot != rotation && width > 0) {
            rotation = rot
            onFrame(width, height, rot)
        }
    }
}
