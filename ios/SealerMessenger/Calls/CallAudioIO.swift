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

    /// Diagnostico (so para o registo): blocos de 20 ms entregues ao Rust e pedidos ao Rust.
    private let counters = OSAllocatedUnfairLock(initialState: (sent: 0, pulled: 0))
    var stats: (sent: Int, pulled: Int) { counters.withLock { $0 } }
    /// Formato do microfone (para o registo).
    private(set) var inputDescription = ""
    /// Chamadas das callbacks de tempo real (microfone e altifalante). So contadores: escrita
    /// sem trinco, lidos pelo vigia (um valor atrasado nao faz mal).
    private let rtCounts: UnsafeMutablePointer<Int> = {
        let p = UnsafeMutablePointer<Int>.allocate(capacity: 2)
        p.initialize(repeating: 0, count: 2)
        return p
    }()
    private var source: AVAudioSourceNode?
    private var voiceProcessing = true
    private var restarts = 0
    private var configObserver: NSObjectProtocol?

    deinit {
        if let o = configObserver { NotificationCenter.default.removeObserver(o) }
        rtCounts.deallocate()
    }

    /// Linha para o registo: o motor de audio do iPhone esta mesmo a trabalhar?
    var debugLine: String {
        "motor \(engine.isRunning ? "sim" : "nao"), voz \(voiceProcessing ? "sim" : "nao"), "
            + "callbacks micro \(rtCounts[0]) / altifalante \(rtCounts[1]), reinicios \(restarts)"
    }

    /// Liga o microfone e o altifalante. A sessao de audio ja foi ativada (CallKit).
    func start() throws {
        guard !running else { return }
        try buildGraph(voiceProcessing: true)
        do {
            try engine.start()
        } catch {
            // Sem o modo de voz do iPhone (sem cancelamento de eco), mas com som.
            log("SealerCall", "audio com modo de voz falhou (\(safeMessage(error))): sem modo de voz")
            teardownGraph()
            try buildGraph(voiceProcessing: false)
            try engine.start()
        }
        running = true
        // O iOS para o motor quando a configuracao do audio muda (CallKit, Bluetooth, auricular):
        // volta a arrancar.
        configObserver = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: engine, queue: nil
        ) { [weak self] _ in
            // Fora da notificacao (pode chegar durante um reinicio) e so se o motor parou mesmo.
            self?.restartQueue.asyncAfter(deadline: .now() + 0.2) { [weak self] in
                guard let self, self.running, !self.engine.isRunning else { return }
                self.restart(reason: "configuracao do audio mudou")
            }
        }
        let mic = Thread { [weak self] in self?.captureLoop() }
        mic.name = "sealer-call-mic"
        mic.qualityOfService = .userInteractive
        let spk = Thread { [weak self] in self?.playbackLoop() }
        spk.name = "sealer-call-spk"
        spk.qualityOfService = .userInteractive
        let dog = Thread { [weak self] in self?.watchdog() }
        dog.name = "sealer-call-vigia"
        threads = [mic, spk, dog]
        mic.start()
        spk.start()
        dog.start()
    }

    private func buildGraph(voiceProcessing vp: Bool) throws {
        let input = engine.inputNode
        let output = engine.outputNode
        // O modo de voz liga-se nos dois lados (entrada e saida) antes de ligar os nos.
        try input.setVoiceProcessingEnabled(vp)
        voiceProcessing = vp
        let inFormat = input.outputFormat(forBus: 0)
        guard inFormat.sampleRate > 0, inFormat.channelCount > 0 else {
            throw SecureStoreError(message: "microfone indisponivel")
        }
        inputRate = inFormat.sampleRate
        inputDescription = "\(Int(inFormat.sampleRate)) Hz, \(inFormat.channelCount) canal(is), saida \(Int(output.outputFormat(forBus: 0).sampleRate)) Hz, voz \(vp ? "sim" : "nao")"
        let ring = capture
        let sem = wake
        let counts = rtCounts
        input.installTap(onBus: 0, bufferSize: 480, format: inFormat) { buffer, _ in
            counts[0] &+= 1
            guard let ch = buffer.floatChannelData else { return }
            ring.write(ch[0], Int(buffer.frameLength))
            sem.signal()
        }
        let outFormat = AVAudioFormat(standardFormatWithSampleRate: Self.rate, channels: 1)!
        let out = playback
        let node = AVAudioSourceNode(format: outFormat) { _, _, frameCount, list -> OSStatus in
            counts[1] &+= 1
            let abl = UnsafeMutableAudioBufferListPointer(list)
            let n = Int(frameCount)
            for buf in abl {
                guard let p = buf.mData?.assumingMemoryBound(to: Float.self) else { continue }
                let got = out.read(p, n)
                if got < n { (p + got).update(repeating: 0, count: n - got) }   // falta: silencio
            }
            return noErr
        }
        engine.attach(node)
        engine.connect(node, to: engine.mainMixerNode, format: outFormat)
        engine.connect(engine.mainMixerNode, to: output, format: nil)
        source = node
        engine.prepare()
    }

    private func teardownGraph() {
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        if let s = source {
            engine.disconnectNodeOutput(s)
            engine.detach(s)
        }
        source = nil
    }

    private let restartLock = NSLock()
    private let restartQueue = DispatchQueue(label: "sealer-call-audio-restart")

    /// Volta a montar o audio. Ao 2.o reinicio sem som, desiste do modo de voz do iPhone.
    private func restart(reason: String) {
        restartLock.lock(); defer { restartLock.unlock() }
        guard running, restarts < 6 else { return }
        restarts += 1
        let vp = voiceProcessing && restarts < 2
        log("SealerCall", "a reiniciar o audio (\(reason)), modo de voz \(vp ? "sim" : "nao")")
        teardownGraph()
        do {
            try buildGraph(voiceProcessing: vp)
            try engine.start()
        } catch {
            log("SealerCall", "reinicio falhou: \(safeMessage(error))")
            if vp {
                teardownGraph()
                try? buildGraph(voiceProcessing: false)
                try? engine.start()
            }
        }
    }

    /// Vigia: se o altifalante ou o microfone deixarem de ser chamados pelo iOS durante 1,5 s,
    /// o audio esta parado (motor parado ou preso): reinicia (ate 3 vezes).
    private func watchdog() {
        var last = (rtCounts[0], rtCounts[1])
        var stuckFor = 0
        var micStuckFor = 0
        var micLostSent = false
        while running {
            usleep(500_000)
            let now = (rtCounts[0], rtCounts[1])
            let stuck = now.0 == last.0 || now.1 == last.1 || !engine.isRunning
            let micStuck = now.0 == last.0
            last = now
            stuckFor = stuck ? stuckFor + 1 : 0
            micStuckFor = micStuck ? micStuckFor + 1 : 0
            if stuckFor >= 3 && restarts < 3 {
                stuckFor = 0
                restartQueue.sync { restart(reason: "sem callbacks (micro \(now.0), altifalante \(now.1), motor \(engine.isRunning ? "sim" : "nao"))") }
            } else if restarts >= 3 && micStuckFor >= 6 && !micLostSent {
                // Os reinicios acabaram e o microfone continua sem dar nada ha 3 s (outra app ficou
                // com ele): avisa uma vez (igual ao `onMicLost` do Android). O altifalante continua.
                micLostSent = true
                log("SealerCall", "microfone perdido")
                if let cb = onMicLost { DispatchQueue.main.async { cb() } }
            }
            if !micStuck { micLostSent = false }
        }
    }

    /// O microfone deixou de dar som e os reinicios nao o recuperaram (chamado na fila principal).
    var onMicLost: (() -> Void)?

    /// Junta as amostras do microfone em blocos de 20 ms a 48 kHz e entrega-os ao Rust.
    private func captureLoop() {
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
            // Amostras de entrada por amostra de saida (o formato pode mudar num reinicio).
            let step = inputRate / Self.rate
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
                    counters.withLock { $0.sent += 1 }
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
                counters.withLock { $0.pulled += 1 }
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
        if let o = configObserver { NotificationCenter.default.removeObserver(o) }
        configObserver = nil
        restartLock.lock()
        teardownGraph()
        try? engine.inputNode.setVoiceProcessingEnabled(false)
        restartLock.unlock()
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
