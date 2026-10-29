import AVFoundation
import CoreMedia
import Network
import os
import SwiftUI
import UIKit
import VideoToolbox

// Videochamadas no iPhone (0.9.50), igual ao Video.kt do Android. Aqui so ha o que e do iPhone:
// a camara (AVCaptureSession), o codificador H.264 (VideoToolbox, em hardware) e as vistas.
// A cifra, os pacotes, a espera pelos atrasados, a sincronizacao com o audio, os pedidos de
// imagem completa e o debito estao no Rust (`CallAudio.video*`): esta parte so aplica os
// parametros que de la vem e passa imagens de um lado para o outro.
//
// Consentimento: o Rust so deixa sair video depois de o outro aceitar ver; antes disso a camara
// so alimenta a pre-visualizacao (e o codificador nem trabalha: `send` falso).

extension VideoFrameIn: @unchecked Sendable {}
extension VideoEncoderParams: @unchecked Sendable {}
extension VideoResponse: @unchecked Sendable {}
extension NetworkKind: @unchecked Sendable {}
extension FichasIssued: @unchecked Sendable {}

private let videoLog = "SealerVideo"

// ============================================================================ rede

/// Rede do iPhone para o teto do video em ligacao direta (Wi-Fi sem limite de dados: 1080p).
final class VideoNet: @unchecked Sendable {
    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "sealer-video-net")

    static func kind(_ p: NWPath) -> NetworkKind {
        guard p.status == .satisfied else { return .unknown }
        let wifi = p.usesInterfaceType(.wifi) || p.usesInterfaceType(.wiredEthernet)
        if wifi && !p.isExpensive && !p.isConstrained { return .wifi }
        // Dados moveis, ou Wi-Fi com limite (ex.: partilha de dados de outro telemovel).
        if p.usesInterfaceType(.cellular) || wifi { return .mobile }
        return .unknown
    }

    /// Avisa ja com a rede atual e depois sempre que mudar (Wi-Fi <-> dados moveis).
    func start(_ onChange: @escaping @Sendable (NetworkKind) -> Void) {
        monitor.pathUpdateHandler = { onChange(Self.kind($0)) }
        monitor.start(queue: queue)
    }

    func stop() { monitor.cancel() }
}

// ============================================================================ H.264

/// Annex B (00 00 00 01 + NAL) <-> o formato da Apple (comprimento + NAL).
enum H264 {
    /// As unidades NAL de uma imagem Annex B (sem os codigos de inicio).
    static func nals(_ data: Data) -> [Data] {
        var out: [Data] = []
        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            let b = raw.bindMemory(to: UInt8.self)
            let n = b.count
            var i = 0
            var start = -1
            while i + 2 < n {
                if b[i] == 0 && b[i + 1] == 0 && (b[i + 2] == 1 || (i + 3 < n && b[i + 2] == 0 && b[i + 3] == 1)) {
                    let codeLen = b[i + 2] == 1 ? 3 : 4
                    if start >= 0 {
                        var end = i
                        while end > start && b[end - 1] == 0 { end -= 1 }   // zeros a mais antes do codigo
                        if end > start { out.append(Data(b[start..<end])) }
                    }
                    i += codeLen
                    start = i
                } else {
                    i += 1
                }
            }
            if start >= 0 && start < n { out.append(Data(b[start..<n])) }
        }
        return out
    }

    static func nalType(_ nal: Data) -> UInt8 { nal.isEmpty ? 0 : nal[nal.startIndex] & 0x1F }

    private static let startCode: [UInt8] = [0, 0, 0, 1]

    /// Imagem do codificador da Apple -> Annex B (SPS e PPS a frente das imagens completas).
    static func annexB(_ sample: CMSampleBuffer, keyframe: Bool) -> Data? {
        guard let block = CMSampleBufferGetDataBuffer(sample) else { return nil }
        var out = Data()
        var headerLen: Int32 = 4
        if let fd = CMSampleBufferGetFormatDescription(sample) {
            var count = 0
            if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                fd, parameterSetIndex: 0, parameterSetPointerOut: nil, parameterSetSizeOut: nil,
                parameterSetCountOut: &count, nalUnitHeaderLengthOut: &headerLen
            ) != noErr { headerLen = 4 }
            if keyframe {
                for idx in 0..<count {
                    var ptr: UnsafePointer<UInt8>?
                    var size = 0
                    if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                        fd, parameterSetIndex: idx, parameterSetPointerOut: &ptr, parameterSetSizeOut: &size,
                        parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil
                    ) == noErr, let ptr {
                        out.append(contentsOf: startCode)
                        out.append(ptr, count: size)
                    }
                }
            }
        }
        let total = CMBlockBufferGetDataLength(block)
        guard total > 0 else { return nil }
        var bytes = [UInt8](repeating: 0, count: total)
        guard CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: total, destination: &bytes) == noErr else { return nil }
        let hl = Int(headerLen)
        var pos = 0
        while pos + hl <= total {
            var len = 0
            for k in 0..<hl { len = (len << 8) | Int(bytes[pos + k]) }
            pos += hl
            guard len > 0, pos + len <= total else { break }
            out.append(contentsOf: startCode)
            out.append(contentsOf: bytes[pos..<(pos + len)])
            pos += len
        }
        return out.isEmpty ? nil : out
    }
}

// ============================================================================ camara + codificador

/// Camara + codificador H.264. Segue `CallAudio.videoParams()` (debito, resolucao, imagens/s,
/// imagem completa) e entrega cada imagem codificada ao Rust (`videoPush`), com a rotacao.
/// A configuracao e as imagens correm todas na mesma fila ([queue]).
final class VideoSender: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
    /// A sessao da camara: a pre-visualizacao (`SelfVideoView`) liga-se a ela.
    let session = AVCaptureSession()
    private let audio: CallAudio
    private let queue = DispatchQueue(label: "sealer-video-cam", qos: .userInteractive)
    private let output = AVCaptureVideoDataOutput()
    private var input: AVCaptureDeviceInput?
    private(set) var front = true
    private var running = false
    private var encoder: VTCompressionSession?
    private var encW: Int32 = 0
    private var encH: Int32 = 0
    private var encFps: Int32 = 0
    private var preset: AVCaptureSession.Preset?
    private var lastBitrate = 0
    private var lastKeySeq: UInt32?
    private var sending = false
    private var forceKey = true
    private var lastEncodedAt = CMTime.invalid
    private var scaler: VTPixelTransferSession?
    private var scaledPool: CVPixelBufferPool?
    private var paramsTimer: DispatchSourceTimer?
    private var observers: [NSObjectProtocol] = []
    /// Orientacao do iPhone (lida na fila principal, usada na da camara).
    private let orientation = OSAllocatedUnfairLock(initialState: UIDeviceOrientation.portrait)
    /// A camara nao abriu ou falhou a meio (chamado na fila principal).
    var onError: (() -> Void)?

    init(audio: CallAudio) {
        self.audio = audio
        super.init()
    }

    /// Liga a camara (frontal ou traseira).
    @MainActor
    func start(front useFront: Bool) {
        front = useFront
        UIDevice.current.beginGeneratingDeviceOrientationNotifications()
        updateOrientation()
        observers.append(NotificationCenter.default.addObserver(
            forName: UIDevice.orientationDidChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in MainActor.assumeIsolated { self?.updateOrientation() } })
        observers.append(NotificationCenter.default.addObserver(
            forName: AVCaptureSession.runtimeErrorNotification, object: session, queue: .main
        ) { [weak self] n in
            let e = n.userInfo?[AVCaptureSessionErrorKey] as? NSError
            log(videoLog, "camara: erro \(e?.code ?? 0)")
            // O iOS para a camara quando a app sai do ecra (e isso nao e um erro a mostrar).
            if e?.code != AVError.Code.mediaServicesWereReset.rawValue { self?.onError?() }
        })
        queue.async { [self] in
            running = true
            // A sessao de audio e a das chamadas (CallKit): a camara nunca lhe mexe.
            session.automaticallyConfiguresApplicationAudioSession = false
            session.beginConfiguration()
            output.alwaysDiscardsLateVideoFrames = true
            output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
            output.setSampleBufferDelegate(self, queue: queue)
            if session.canAddOutput(output) { session.addOutput(output) }
            session.commitConfiguration()
            guard openCamera() else {
                fail("sem camara")
                return
            }
            applyParams(audio.videoParams(), force: true)
            session.startRunning()
            startParamsLoop()
        }
    }

    /// Frontal <-> traseira.
    func switchCamera() {
        queue.async { [self] in
            guard running else { return }
            front.toggle()
            if !openCamera() { fail("trocar camara") }
            forceKey = true
        }
    }

    @MainActor
    func stop() {
        for o in observers { NotificationCenter.default.removeObserver(o) }
        observers.removeAll()
        UIDevice.current.endGeneratingDeviceOrientationNotifications()
        onError = nil
        queue.async { [self] in
            running = false
            paramsTimer?.cancel()
            paramsTimer = nil
            output.setSampleBufferDelegate(nil, queue: nil)
            if session.isRunning { session.stopRunning() }
            releaseEncoder()
        }
    }

    private func fail(_ why: String) {
        log(videoLog, "camara: \(why)")
        DispatchQueue.main.async { [weak self] in self?.onError?() }
    }

    @MainActor
    private func updateOrientation() {
        let o = UIDevice.current.orientation
        // Deitado em cima da mesa (ou desconhecido): fica a ultima orientacao valida.
        guard o.isPortrait || o.isLandscape else { return }
        orientation.withLock { $0 = o }
    }

    /// Graus (sentido dos ponteiros) para mostrar direita a imagem da camara (que sai sempre
    /// deitada, como o sensor). A mesma tabela que o WebRTC usa no iPhone.
    private func rotationDegrees() -> UInt32 {
        switch orientation.withLock({ $0 }) {
        case .portraitUpsideDown: return 270
        case .landscapeLeft: return front ? 180 : 0
        case .landscapeRight: return front ? 0 : 180
        default: return 90
        }
    }

    /// (Re)liga a camara escolhida a sessao. Na [queue].
    private func openCamera() -> Bool {
        let position: AVCaptureDevice.Position = front ? .front : .back
        let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: position)
            ?? AVCaptureDevice.default(for: .video)
        guard let device, let newInput = try? AVCaptureDeviceInput(device: device) else { return false }
        session.beginConfiguration()
        defer { session.commitConfiguration() }
        if let old = input { session.removeInput(old) }
        guard session.canAddInput(newInput) else {
            if let old = input, session.canAddInput(old) { session.addInput(old) }
            return false
        }
        session.addInput(newInput)
        input = newInput
        // Imagens como o sensor (deitadas) e sem espelho: a rotacao vai com cada imagem e o
        // outro lado ve-me como sou (o espelho e so na minha pre-visualizacao).
        if let conn = output.connection(with: .video) {
            if conn.isVideoMirroringSupported {
                conn.automaticallyAdjustsVideoMirroring = false
                conn.isVideoMirrored = false
            }
            if conn.isVideoRotationAngleSupported(0) { conn.videoRotationAngle = 0 }
        }
        if let p = preset, session.canSetSessionPreset(p) { session.sessionPreset = p }
        return true
    }

    /// O formato da camara para o pedido (16:9; o VideoToolbox reduz ao tamanho do codificador).
    private func presetFor(height: Int32) -> AVCaptureSession.Preset {
        if height > 720 && session.canSetSessionPreset(.hd1920x1080) { return .hd1920x1080 }
        return .hd1280x720
    }

    // ------------------------------------------------------------------ parametros do Rust

    /// A cada 250 ms: o Rust pode ter mudado o debito, a resolucao ou pedido imagem completa.
    private func startParamsLoop() {
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + .milliseconds(250), repeating: .milliseconds(250))
        t.setEventHandler { [weak self] in
            guard let self, self.running else { return }
            self.applyParams(self.audio.videoParams(), force: false)
        }
        t.resume()
        paramsTimer = t
    }

    private func applyParams(_ p: VideoEncoderParams, force: Bool) {
        let w = Int32(p.width > 0 ? p.width : 640) & ~1
        let h = Int32(p.height > 0 ? p.height : 360) & ~1
        let fps = Int32(min(30, max(10, p.fps)))
        let bitrate = max(100_000, Int(p.bitrateBps))
        // Nada sai antes de o outro aceitar: ate la a camara so mostra a pre-visualizacao.
        if p.send && !sending { forceKey = true }
        sending = p.send
        if force || w != encW || h != encH || fps != encFps || encoder == nil {
            let want = presetFor(height: h)
            if want != preset {
                preset = want
                session.beginConfiguration()
                if session.canSetSessionPreset(want) { session.sessionPreset = want }
                session.commitConfiguration()
            }
            // Resolucao nova: codificador novo (a primeira imagem sai completa).
            configureEncoder(width: w, height: h, fps: fps, bitrate: bitrate)
            lastKeySeq = p.keyframeSeq
            return
        }
        if let enc = encoder, abs(bitrate - lastBitrate) > lastBitrate / 20 {
            setBitrate(enc, bitrate)
            lastBitrate = bitrate
        }
        if p.keyframeSeq != lastKeySeq {
            forceKey = true
            lastKeySeq = p.keyframeSeq
        }
    }

    private func setBitrate(_ enc: VTCompressionSession, _ bitrate: Int) {
        VTSessionSetProperty(enc, key: kVTCompressionPropertyKey_AverageBitRate, value: NSNumber(value: bitrate))
        // Teto por segundo (bytes): segura os picos das imagens completas.
        let limit = [NSNumber(value: bitrate * 3 / 16), NSNumber(value: 1)] as CFArray
        VTSessionSetProperty(enc, key: kVTCompressionPropertyKey_DataRateLimits, value: limit)
    }

    private func configureEncoder(width: Int32, height: Int32, fps: Int32, bitrate: Int) {
        releaseEncoder()
        var created: VTCompressionSession?
        // Primeiro o modo de baixa latencia (iOS 14.5+); se o codificador o recusar, o normal.
        let lowLatency = [kVTVideoEncoderSpecification_EnableLowLatencyRateControl: true] as CFDictionary
        var status = VTCompressionSessionCreate(
            allocator: nil, width: width, height: height, codecType: kCMVideoCodecType_H264,
            encoderSpecification: lowLatency, imageBufferAttributes: nil, compressedDataAllocator: nil,
            outputCallback: nil, refcon: nil, compressionSessionOut: &created
        )
        let low = status == noErr && created != nil
        if !low {
            status = VTCompressionSessionCreate(
                allocator: nil, width: width, height: height, codecType: kCMVideoCodecType_H264,
                encoderSpecification: nil, imageBufferAttributes: nil, compressedDataAllocator: nil,
                outputCallback: nil, refcon: nil, compressionSessionOut: &created
            )
        }
        guard status == noErr, let enc = created else {
            fail("codificador: \(status)")
            return
        }
        // Tempo real, perfil base (sem imagens B: todos os descodificadores, incluindo o Android).
        VTSessionSetProperty(enc, key: kVTCompressionPropertyKey_RealTime, value: kCFBooleanTrue)
        VTSessionSetProperty(enc, key: kVTCompressionPropertyKey_ProfileLevel,
                             value: low ? kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel : kVTProfileLevel_H264_Baseline_AutoLevel)
        VTSessionSetProperty(enc, key: kVTCompressionPropertyKey_AllowFrameReordering, value: kCFBooleanFalse)
        VTSessionSetProperty(enc, key: kVTCompressionPropertyKey_ExpectedFrameRate, value: NSNumber(value: fps))
        // Imagens completas so a pedido (o Rust pede quando o outro lado perde uma).
        VTSessionSetProperty(enc, key: kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, value: NSNumber(value: 10))
        VTSessionSetProperty(enc, key: kVTCompressionPropertyKey_MaxKeyFrameInterval, value: NSNumber(value: fps * 10))
        setBitrate(enc, bitrate)
        VTCompressionSessionPrepareToEncodeFrames(enc)
        encoder = enc
        encW = width
        encH = height
        encFps = fps
        lastBitrate = bitrate
        forceKey = true
        lastEncodedAt = .invalid
        log(videoLog, "codificador \(width)x\(height) \(fps)/s \(bitrate / 1000) kb/s\(low ? " (baixa latencia)" : "")")
    }

    private func releaseEncoder() {
        if let enc = encoder {
            VTCompressionSessionCompleteFrames(enc, untilPresentationTimeStamp: .invalid)
            VTCompressionSessionInvalidate(enc)
        }
        encoder = nil
        if let s = scaler { VTPixelTransferSessionInvalidate(s) }
        scaler = nil
        scaledPool = nil
    }

    /// A imagem da camara no tamanho do codificador (a camara so tem alguns tamanhos).
    private func scaled(_ pb: CVPixelBuffer) -> CVPixelBuffer? {
        let w = CVPixelBufferGetWidth(pb)
        let h = CVPixelBufferGetHeight(pb)
        if w == Int(encW) && h == Int(encH) { return pb }
        if scaledPool == nil {
            let attrs: [String: Any] = [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                kCVPixelBufferWidthKey as String: Int(encW),
                kCVPixelBufferHeightKey as String: Int(encH),
                kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any](),
            ]
            var pool: CVPixelBufferPool?
            CVPixelBufferPoolCreate(nil, nil, attrs as CFDictionary, &pool)
            scaledPool = pool
        }
        if scaler == nil {
            var s: VTPixelTransferSession?
            VTPixelTransferSessionCreate(allocator: nil, pixelTransferSessionOut: &s)
            if let s {
                VTSessionSetProperty(s, key: kVTPixelTransferPropertyKey_ScalingMode, value: kVTScalingMode_CropSourceToCleanAperture)
            }
            scaler = s
        }
        guard let pool = scaledPool, let s = scaler else { return nil }
        var out: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out) == kCVReturnSuccess, let out else { return nil }
        return VTPixelTransferSessionTransferImage(s, from: pb, to: out) == noErr ? out : nil
    }

    // ------------------------------------------------------------------ imagens

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard running, sending, let enc = encoder, let pb = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        // Menos imagens por segundo do que a camara da: salta as que vem cedo demais.
        if lastEncodedAt.isValid && encFps > 0 {
            let gap = CMTimeGetSeconds(CMTimeSubtract(pts, lastEncodedAt))
            if gap >= 0 && gap < 1.0 / Double(encFps) - 0.004 { return }
        }
        lastEncodedAt = pts
        guard let frame = scaled(pb) else { return }
        let key = forceKey
        forceKey = false
        let rotation = rotationDegrees()
        let props: CFDictionary? = key ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        let audio = self.audio
        let status = VTCompressionSessionEncodeFrame(
            enc, imageBuffer: frame, presentationTimeStamp: pts, duration: .invalid,
            frameProperties: props, infoFlagsOut: nil
        ) { status, _, sample in
            guard status == noErr, let sample, CMSampleBufferDataIsReady(sample) else { return }
            let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
            let notSync = (attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool) ?? false
            guard let data = H264.annexB(sample, keyframe: !notSync) else { return }
            // Ha quanto tempo foi captada (as marcas de tempo da camara sao do relogio da maquina).
            let now = CMClockGetTime(CMClockGetHostTimeClock())
            let age = CMTimeGetSeconds(CMTimeSubtract(now, CMSampleBufferGetPresentationTimeStamp(sample))) * 1000
            let ageMs = age >= 0 && age <= 500 ? UInt32(age) : 40
            // `false` = o outro ainda nao aceitou (ou pausa): a imagem nao sai.
            _ = audio.videoPush(frame: data, keyframe: !notSync, rotationDegrees: rotation, ageMs: ageMs)
        }
        if status == kVTInvalidSessionErr {
            // O codificador perdeu-se (ex.: a app voltou do segundo plano): um novo.
            log(videoLog, "codificar: \(status)")
            configureEncoder(width: encW, height: encH, fps: encFps, bitrate: lastBitrate)
        }
    }
}

// ============================================================================ descodificador

/// O video do outro lado: pede ao Rust a proxima imagem ja na hora certa (`videoPull`) e
/// entrega-a a camada de video do iOS (que descodifica o H.264 em hardware). Se a camada
/// falhar, recomeca e pede uma imagem completa ao outro lado.
final class VideoReceiver: @unchecked Sendable {
    /// A camada onde o video aparece (a vista `RemoteVideoView` mostra-a).
    let layer = AVSampleBufferDisplayLayer()
    private let audio: CallAudio
    private let state = OSAllocatedUnfairLock(initialState: (running: false, lastFrameAt: 0.0, rotation: UInt32(0)))
    private var thread: Thread?
    private var format: CMVideoFormatDescription?
    /// A rotacao das imagens mudou (chamado na fila principal).
    var onRotation: ((UInt32) -> Void)?

    init(audio: CallAudio) {
        self.audio = audio
        layer.videoGravity = .resizeAspectFill
        layer.backgroundColor = UIColor.black.cgColor
    }

    /// Quando chegou a ultima imagem do outro (relogio monotono, s); 0 = nenhuma ainda.
    var lastFrameAt: Double { state.withLock { $0.lastFrameAt } }

    func start() {
        let already = state.withLock { s -> Bool in
            if s.running { return true }
            s.running = true
            return false
        }
        if already { return }
        let t = Thread { [weak self] in self?.loop() }
        t.name = "sealer-video-dec"
        t.qualityOfService = .userInteractive
        thread = t
        t.start()
    }

    func stop() {
        state.withLock { $0.running = false }
        thread = nil
        let l = layer
        DispatchQueue.main.async { l.sampleBufferRenderer.flush(removingDisplayedImage: true, completionHandler: nil) }
    }

    private var isRunning: Bool { state.withLock { $0.running } }

    private func loop() {
        let renderer = layer.sampleBufferRenderer
        var needKey = true
        while isRunning {
            guard let f = audio.videoPull(timeoutMs: 50) else { continue }
            state.withLock { $0.lastFrameAt = monotonicSeconds() }
            let rot = f.rotationDegrees
            let changed = state.withLock { s -> Bool in
                if s.rotation == rot { return false }
                s.rotation = rot
                return true
            }
            if changed, let cb = onRotation { DispatchQueue.main.async { cb(rot) } }
            // A camada falhou ou foi interrompida (ex.: a app saiu do ecra): recomeca numa
            // imagem completa.
            if renderer.status == .failed || renderer.requiresFlushToResumeDecoding {
                renderer.flush()
                needKey = true
            }
            let nals = H264.nals(f.data)
            if let fd = formatFrom(nals) { format = fd }
            let key = f.keyframe || nals.contains { H264.nalType($0) == 5 }
            if needKey {
                guard key, format != nil else {
                    audio.videoRequestKeyframe()
                    continue
                }
                needKey = false
            }
            guard let fd = format, let sample = sampleBuffer(nals, fd) else {
                audio.videoRequestKeyframe()
                needKey = true
                continue
            }
            renderer.enqueue(sample)
        }
    }

    /// SPS e PPS de uma imagem completa -> descricao do formato.
    private func formatFrom(_ nals: [Data]) -> CMVideoFormatDescription? {
        guard let sps = nals.first(where: { H264.nalType($0) == 7 }), let pps = nals.first(where: { H264.nalType($0) == 8 }) else { return nil }
        var fd: CMVideoFormatDescription?
        let status = sps.withUnsafeBytes { (s: UnsafeRawBufferPointer) -> OSStatus in
            pps.withUnsafeBytes { (p: UnsafeRawBufferPointer) -> OSStatus in
                guard let sp = s.bindMemory(to: UInt8.self).baseAddress, let pp = p.bindMemory(to: UInt8.self).baseAddress else { return -1 }
                let ptrs: [UnsafePointer<UInt8>] = [sp, pp]
                let sizes: [Int] = [sps.count, pps.count]
                return CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    allocator: kCFAllocatorDefault, parameterSetCount: 2, parameterSetPointers: ptrs,
                    parameterSetSizes: sizes, nalUnitHeaderLength: 4, formatDescriptionOut: &fd
                )
            }
        }
        return status == noErr ? fd : nil
    }

    /// As NAL da imagem (sem SPS/PPS/AUD) no formato da Apple, prontas a mostrar ja.
    private func sampleBuffer(_ nals: [Data], _ fd: CMVideoFormatDescription) -> CMSampleBuffer? {
        var avcc = Data()
        for n in nals {
            let t = H264.nalType(n)
            if t == 7 || t == 8 || t == 9 || n.isEmpty { continue }
            var len = UInt32(n.count).bigEndian
            withUnsafeBytes(of: &len) { avcc.append(contentsOf: $0) }
            avcc.append(n)
        }
        guard !avcc.isEmpty else { return nil }
        var block: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(
            allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: avcc.count, blockAllocator: kCFAllocatorDefault,
            customBlockSource: nil, offsetToData: 0, dataLength: avcc.count, flags: 0, blockBufferOut: &block
        ) == kCMBlockBufferNoErr, let block else { return nil }
        let copied = avcc.withUnsafeBytes { raw -> OSStatus in
            guard let base = raw.baseAddress else { return -1 }
            return CMBlockBufferReplaceDataBytes(with: base, blockBuffer: block, offsetIntoDestination: 0, dataLength: avcc.count)
        }
        guard copied == kCMBlockBufferNoErr else { return nil }
        var sample: CMSampleBuffer?
        var size = avcc.count
        guard CMSampleBufferCreateReady(
            allocator: kCFAllocatorDefault, dataBuffer: block, formatDescription: fd, sampleCount: 1,
            sampleTimingEntryCount: 0, sampleTimingArray: nil, sampleSizeEntryCount: 1, sampleSizeArray: &size,
            sampleBufferOut: &sample
        ) == noErr, let sample else { return nil }
        if let arr = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true), CFArrayGetCount(arr) > 0 {
            let dict = unsafeBitCast(CFArrayGetValueAtIndex(arr, 0), to: CFMutableDictionary.self)
            CFDictionarySetValue(dict, Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
                                 Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
        }
        return sample
    }
}

// ============================================================================ vistas

/// O video do outro, direito (rodado pela rotacao que vem com as imagens) e a encher a vista.
struct RemoteVideoView: UIViewRepresentable {
    let layer: AVSampleBufferDisplayLayer
    let rotation: UInt32

    func makeUIView(context: Context) -> RemoteVideoUIView {
        let v = RemoteVideoUIView()
        v.backgroundColor = .black
        v.videoLayer = layer
        return v
    }

    func updateUIView(_ v: RemoteVideoUIView, context: Context) {
        v.videoLayer = layer
        v.rotation = rotation
    }
}

final class RemoteVideoUIView: UIView {
    var videoLayer: AVSampleBufferDisplayLayer? {
        didSet {
            guard videoLayer !== oldValue else { return }
            oldValue?.removeFromSuperlayer()
            if let d = videoLayer { layer.addSublayer(d) }
            setNeedsLayout()
        }
    }
    var rotation: UInt32 = 0 { didSet { if rotation != oldValue { setNeedsLayout() } } }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard let d = videoLayer else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let quarter = (rotation / 90) % 4
        let size = quarter % 2 == 0 ? bounds.size : CGSize(width: bounds.height, height: bounds.width)
        d.setAffineTransform(.identity)
        d.bounds = CGRect(origin: .zero, size: size)
        d.position = CGPoint(x: bounds.midX, y: bounds.midY)
        d.setAffineTransform(CGAffineTransform(rotationAngle: CGFloat(quarter) * .pi / 2))
        CATransaction.commit()
    }
}

/// A minha camara (pre-visualizacao do sistema: direita e, a da frente, em espelho).
struct SelfVideoView: UIViewRepresentable {
    let session: AVCaptureSession

    func makeUIView(context: Context) -> SelfVideoUIView {
        let v = SelfVideoUIView()
        v.backgroundColor = .black
        v.preview.session = session
        v.preview.videoGravity = .resizeAspectFill
        return v
    }

    func updateUIView(_ v: SelfVideoUIView, context: Context) {
        if v.preview.session !== session { v.preview.session = session }
        v.setNeedsLayout()
    }
}

final class SelfVideoUIView: UIView {
    override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
    var preview: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }

    override func layoutSubviews() {
        super.layoutSubviews()
        // A app so funciona ao alto: a pre-visualizacao tambem.
        if let c = preview.connection, c.isVideoRotationAngleSupported(90), c.videoRotationAngle != 90 {
            c.videoRotationAngle = 90
        }
    }
}
