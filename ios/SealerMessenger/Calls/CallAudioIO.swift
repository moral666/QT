import AVFoundation
import Foundation
import os

/// Microfone e altifalante de uma chamada: 48 kHz, mono, 16 bits, 20 ms de cada vez (o que o
/// Rust espera em `pushCapture` / `pullPlayback`). Igual ao CallAudioIO do Android.
///
/// O modo de voz do iPhone (`setVoiceProcessingEnabled`) liga o cancelamento de eco, a
/// reducao de ruido e o controlo de volume do proprio sistema. O Opus, a cifra, o jitter
/// buffer e a rede estao no Rust: aqui so se movem amostras.
///
/// Desempenho: as callbacks de audio (tempo real) so copiam amostras para aneis com trinco;
/// a codificacao (pushCapture) e o pedido ao jitter buffer (pullPlayback) correm em duas
/// threads proprias, de alta prioridade, como no Android.
final class CallAudioIO: @unchecked Sendable {
    static let rate = 48_000.0
    private static let frame = 960   // 20 ms a 48 kHz

    private let audio: CallAudio
    private let engine = AVAudioEngine()
    private let capture = SampleRing(capacity: 48_000)      // 1 s de folga
    private let playback = SampleRing(capacity: 48_000 / 5)  // 200 ms
    private var inputRate = 48_000.0
    private let stateLock = OSAllocatedUnfairLock(initialState: false)
    private var running: Bool {
        get { stateLock.withLock { $0 } }
        set { stateLock.withLock { $0 = newValue } }
    }
    private var threads: [Thread] = []
    private let wake = DispatchSemaphore(value: 0)

    /// Silencia o microfone (envia silencio, como no Android).
    var muted: Bool {
        get { flags.withLock { $0.muted } }
        set { flags.withLock { $0.muted = newValue } }
    }
    /// "tuu... tuu" de quem liga, enquanto o outro lado nao atende (sem o pedir ao Rust).
    var ringback: Bool {
        get { flags.withLock { $0.ringback } }
        set { flags.withLock { $0.ringback = newValue } }
    }
    private struct Flags { var muted = false; var ringback = false }
    private let flags = OSAllocatedUnfairLock(initialState: Flags())

    init(audio: CallAudio) {
        self.audio = audio
    }

    /// Liga o microfone e o altifalante. A sessao de audio ja foi ativada pelo CallKit.
    func start() throws {
        guard !running else { return }
        let input = engine.inputNode
        try input.setVoiceProcessingEnabled(true)
        let inFormat = input.outputFormat(forBus: 0)
        guard inFormat.sampleRate > 0, inFormat.channelCount > 0 else {
            throw SecureStoreError(message: "microfone indisponivel")
        }
        inputRate = inFormat.sampleRate
        let ring = capture
        let sem = wake
        let sink = AVAudioSinkNode { _, frameCount, list -> OSStatus in
            // Tempo real: so copia o canal 0 para o anel.
            let abl = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: list))
            if let buf = abl.first, let p = buf.mData?.assumingMemoryBound(to: Float.self) {
                ring.write(p, Int(frameCount))
                sem.signal()
            }
            return noErr
        }
        let outFormat = AVAudioFormat(standardFormatWithSampleRate: Self.rate, channels: 1)!
        let out = playback
        let source = AVAudioSourceNode(format: outFormat) { _, _, frameCount, list -> OSStatus in
            let abl = UnsafeMutableAudioBufferListPointer(list)
            guard let buf = abl.first, let p = buf.mData?.assumingMemoryBound(to: Float.self) else { return noErr }
            let n = Int(frameCount)
            let got = out.read(p, n)
            if got < n { (p + got).update(repeating: 0, count: n - got) }   // falta: silencio
            return noErr
        }
        engine.attach(sink)
        engine.attach(source)
        engine.connect(input, to: sink, format: inFormat)
        engine.connect(source, to: engine.mainMixerNode, format: outFormat)
        engine.prepare()
        try engine.start()
        running = true
        let mic = Thread { [weak self] in self?.captureLoop() }
        mic.name = "sealer-call-mic"
        mic.qualityOfService = .userInteractive
        let spk = Thread { [weak self] in self?.playbackLoop() }
        spk.name = "sealer-call-spk"
        spk.qualityOfService = .userInteractive
        threads = [mic, spk]
        mic.start()
        spk.start()
    }

    /// Junta as amostras do microfone em blocos de 20 ms a 48 kHz e entrega-os ao Rust.
    private func captureLoop() {
        let step = inputRate / Self.rate   // amostras de entrada por amostra de saida
        var src = [Float](repeating: 0, count: 4096)
        var pending: [Float] = []
        pending.reserveCapacity(8192)
        var pos = 0.0   // posicao fracionaria em `pending`
        var frame = [Int16](repeating: 0, count: Self.frame)
        var filled = 0
        while running {
            _ = wake.wait(timeout: .now() + .milliseconds(50))
            let n = src.withUnsafeMutableBufferPointer { capture.read($0.baseAddress!, $0.count) }
            if n == 0 { continue }
            pending.append(contentsOf: src[0..<n])
            // Reamostragem linear (a voz do modo de chamada vem muitas vezes a 48 kHz: passo 1).
            while pos + 1 < Double(pending.count) {
                let i = Int(pos)
                let f = Float(pos - Double(i))
                let v = pending[i] + (pending[i + 1] - pending[i]) * f
                frame[filled] = Int16(max(-1, min(1, v)) * 32767)
                filled += 1
                pos += step
                if filled == Self.frame {
                    filled = 0
                    let pcm: Data
                    if muted {
                        pcm = Data(count: Self.frame * 2)
                    } else {
                        pcm = frame.withUnsafeBufferPointer { Data(buffer: $0) }
                    }
                    do { try audio.pushCapture(pcm: pcm) } catch {
                        log("SealerCall", "microfone: \(safeMessage(error))")
                    }
                }
            }
            let used = Int(pos)
            if used > 0 {
                pending.removeFirst(min(used, pending.count))
                pos -= Double(used)
            }
        }
    }

    /// Mantem ~40 ms prontos para o altifalante: e o ritmo do altifalante que manda.
    private func playbackLoop() {
        var tonePhase = 0.0
        var toneSample = 0
        var floats = [Float](repeating: 0, count: Self.frame)
        while running {
            if playback.count >= Self.frame * 2 {
                usleep(5_000)
                continue
            }
            if ringback {
                // Toque europeu de chamada: 425 Hz, 1 s ligado e 4 s desligado.
                for k in 0..<Self.frame {
                    let on = (toneSample % 240_000) < 48_000
                    floats[k] = on ? Float(sin(tonePhase) * 0.25) : 0
                    tonePhase += 2 * .pi * 425 / Self.rate
                    if tonePhase > 2 * .pi { tonePhase -= 2 * .pi }
                    toneSample += 1
                }
                _ = audio.pullPlayback()   // mantem o relogio do jitter buffer a andar
            } else {
                toneSample = 0
                let pcm = audio.pullPlayback()
                pcm.withUnsafeBytes { raw in
                    let s16 = raw.bindMemory(to: Int16.self)
                    let n = min(s16.count, Self.frame)
                    for k in 0..<n { floats[k] = Float(s16[k]) / 32768 }
                    for k in n..<Self.frame { floats[k] = 0 }
                }
            }
            floats.withUnsafeBufferPointer { playback.write($0.baseAddress!, $0.count) }
        }
    }

    /// Altifalante (true) ou auricular/Bluetooth (false).
    func setSpeaker(_ on: Bool) {
        try? AVAudioSession.sharedInstance().overrideOutputAudioPort(on ? .speaker : .none)
    }

    /// Desliga o microfone e o altifalante.
    func stop() {
        guard running else { return }
        running = false
        wake.signal()
        engine.stop()
        try? engine.inputNode.setVoiceProcessingEnabled(false)
        threads.removeAll()
        setSpeaker(false)
    }
}

/// Anel de amostras com trinco curto (uma thread escreve, outra le). Memoria fixa, sem
/// alocacoes nem contagem de referencias no caminho de tempo real.
final class SampleRing: @unchecked Sendable {
    private let buf: UnsafeMutablePointer<Float>
    private let cap: Int
    private var head = 0
    private var size = 0
    private let lock: UnsafeMutablePointer<os_unfair_lock>

    init(capacity: Int) {
        cap = capacity
        buf = .allocate(capacity: capacity)
        buf.initialize(repeating: 0, count: capacity)
        lock = .allocate(capacity: 1)
        lock.initialize(to: os_unfair_lock())
    }

    deinit {
        buf.deallocate()
        lock.deallocate()
    }

    var count: Int {
        os_unfair_lock_lock(lock); defer { os_unfair_lock_unlock(lock) }
        return size
    }

    /// Escreve; se nao couber, deita fora as mais antigas.
    func write(_ p: UnsafePointer<Float>, _ n: Int) {
        os_unfair_lock_lock(lock); defer { os_unfair_lock_unlock(lock) }
        for k in 0..<n {
            buf[(head + size) % cap] = p[k]
            if size < cap { size += 1 } else { head = (head + 1) % cap }
        }
    }

    /// Le ate `n`; devolve quantas leu.
    func read(_ p: UnsafeMutablePointer<Float>, _ n: Int) -> Int {
        os_unfair_lock_lock(lock); defer { os_unfair_lock_unlock(lock) }
        let m = min(n, size)
        for k in 0..<m { p[k] = buf[(head + k) % cap] }
        head = (head + m) % cap
        size -= m
        return m
    }
}
