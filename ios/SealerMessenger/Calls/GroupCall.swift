import AVFoundation
import CallKit
import Combine
import Foundation
import UIKit
import UserNotifications

// ============================================================================ canais do som e do video

/// De onde vem e para onde vai o som: a chamada a dois (`CallAudio`) ou a de grupo (`GroupAudio`,
/// a mistura de todos; 0.9.64). As assinaturas sao as das classes geradas pelo UniFFI.
protocol AudioPipe: AnyObject, Sendable {
    func pushCapture(pcm: Data) throws
    func pullPlayback() -> Data
}

extension CallAudio: AudioPipe {}
extension GroupAudio: AudioPipe {}

/// Para onde vai o video da minha camara (uma so codificacao para todos, no grupo).
protocol VideoOut: AnyObject, Sendable {
    func videoParams() -> VideoEncoderParams
    func videoPush(frame: Data, keyframe: Bool, rotationDegrees: UInt32, ageMs: UInt32) -> Bool
    func videoSetEncodeLimit(maxHeight: UInt32)
}

extension CallAudio: VideoOut {}
extension GroupAudio: VideoOut {}

/// De onde vem o video de uma pessoa.
protocol VideoIn: AnyObject, Sendable {
    func videoPull(timeoutMs: UInt32) -> VideoFrameIn?
    func videoRequestKeyframe()
    func videoSetDecodeLimit(maxHeight: UInt32)
}

extension CallAudio: VideoIn {}

/// O video de um participante de uma chamada de grupo.
final class GroupVideoIn: VideoIn, @unchecked Sendable {
    private let audio: GroupAudio
    private let contactId: String

    init(audio: GroupAudio, contactId: String) {
        self.audio = audio
        self.contactId = contactId
    }

    func videoPull(timeoutMs: UInt32) -> VideoFrameIn? { audio.videoPull(contactId: contactId, timeoutMs: timeoutMs) }
    func videoRequestKeyframe() { audio.videoRequestKeyframe(contactId: contactId) }
    func videoSetDecodeLimit(maxHeight: UInt32) { audio.videoSetDecodeLimit(maxHeight: maxHeight) }
}

// ============================================================================ chamada de grupo

/// Um participante no ecra (eu nao conto).
struct GroupTile: Equatable {
    let contactId: String
    let name: String
    let connected: Bool
    let camera: Bool
    var rotation: UInt32 = 0
}

/// O que a chamada a dois deixa para a de grupo: o microfone e a saida do som ficam como estavam,
/// e a chamada no CallKit continua, agora para o grupo (a sessao de audio nunca muda de dono).
struct CallHandover {
    let muted: Bool
    let speaker: Bool
    let callKitId: UUID?
    let audioActive: Bool
}

struct GroupCallUi: Equatable {
    let callId: String
    /// Quem me convidou (ou com quem eu estava na chamada a dois): a foto e o nome no topo.
    let fromId: String
    let title: String
    /// Ja estou dentro (false: esta a tocar).
    var joined: Bool
    let video: Bool
    var fromName = ""
    var tiles: [GroupTile] = []
    /// Quem esta a falar (contactId), para o quadrado grande.
    var speaker: String?
    var meSpeaking = false
    var muted = false
    var speakerOn = true
    var camera = false
    var front = true
    /// Quando entrei (relogio monotono, s), para o contador.
    var startedAt: Double = 0
}

/// Chamadas de GRUPO no iPhone (0.9.64): ate 5 pessoas, voz e video. O nucleo (Rust,
/// `group_call.rs`) faz uma malha de chamadas a dois; aqui ha so o microfone e o altifalante (a
/// mistura vem pronta do Rust), a camara (uma so codificacao para todos), um descodificador por
/// participante com a camara ligada e o ecra. Desde a 0.9.74 usa o CallKit como as chamadas a
/// dois: toca com a app fechada, e a chamada de grupo que nasce de uma chamada a dois continua a
/// chamada que ja estava no sistema.
@MainActor
final class GroupCallModel: ObservableObject {
    @Published private(set) var ui: GroupCallUi?
    /// Chamadas a decorrer em que nao estou (recusei ou nao atendi): quem convidou -> chamada.
    @Published private(set) var ongoing: [String: String] = [:]

    private var client: () -> QtClient? = { nil }
    private var flush: () async -> Void = {}
    private var nameOf: (String) -> String = { $0 }
    private var busy: () -> Bool = { false }
    private var onError: (String) -> Void = { _ in }
    /// Cria as sessoes cifradas que faltam com quem esta na lista da chamada (public IDs), como
    /// nos grupos: sem sessao, duas pessoas que nao sao contactos nao tinham ligacao.
    private var ensureSessions: ([String]) async -> Void = { _ in }
    private var isContact: (String) -> Bool = { _ in false }
    /// O que a chamada a dois deixou, uma so vez. `nil`: nao vim de uma chamada a dois.
    private var takeHandover: () -> CallHandover? = { nil }
    /// O toque que o PushKit ja mostrou no CallKit, sem se saber de que chamada era.
    private var takePendingRing: () -> UUID? = { nil }
    /// Ja se carregou em "Atender" no CallKit? (Responde uma so vez.)
    private var consumeAnswer: () -> Bool = { false }
    /// O nome a mostrar no CallKit (com o bloqueio da app, nenhum).
    private var callKitName: (String) -> String = { _ in "" }

    /// O nome que escolhi para as chamadas de grupo (0.9.67; na base cifrada desde a 0.9.74).
    var myName: String {
        get { (try? client()?.groupCallMyName()) ?? "" }
        set { try? client()?.groupCallSaveMyName(name: String(newValue.trimmingCharacters(in: .whitespacesAndNewlines).prefix(32))) }
    }

    /// Passa o nome das versoes antigas para a base cifrada.
    func moveNameToCore(_ c: QtClient) {
        guard let old = UserDefaults.standard.string(forKey: "gcall_name") else { return }
        if (try? c.groupCallSaveMyName(name: old)) != nil { UserDefaults.standard.removeObject(forKey: "gcall_name") }
    }

    /// Envia o meu nome para esta chamada (quem nao me tem nos contactos ve-o).
    private func sendName(_ c: QtClient, _ callId: String) {
        let n = myName
        if !n.isEmpty { try? c.groupCallSetName(callId: callId, name: n) }
    }

    private var audio: GroupAudio?
    private var io: CallAudioIO?
    private var sender: VideoSender?
    private var receivers: [String: VideoReceiver] = [:]
    private var net: VideoNet?
    private var ticker: Task<Void, Never>?
    /// Esta chamada no CallKit (herdada da chamada a dois, ou o toque).
    private var systemCallId: UUID?
    /// O toque foi pelo CallKit: ao atender na app, o sistema fica a saber.
    private var rangViaCallKit = false
    /// O sistema ja acabou esta chamada (nao se pede outra vez).
    private var systemEnded = false
    private var audioActive = false
    private var joining = false
    private var interruption: NSObjectProtocol?

    func attach(
        client: @escaping () -> QtClient?,
        flush: @escaping () async -> Void,
        nameOf: @escaping (String) -> String,
        busy: @escaping () -> Bool,
        onError: @escaping (String) -> Void,
        ensureSessions: @escaping ([String]) async -> Void = { _ in },
        isContact: @escaping (String) -> Bool = { _ in false },
        takeHandover: @escaping () -> CallHandover? = { nil },
        takePendingRing: @escaping () -> UUID? = { nil },
        consumeAnswer: @escaping () -> Bool = { false },
        callKitName: @escaping (String) -> String = { _ in "" }
    ) {
        self.ensureSessions = ensureSessions
        self.isContact = isContact
        self.takeHandover = takeHandover
        self.takePendingRing = takePendingRing
        self.consumeAnswer = consumeAnswer
        self.callKitName = callKitName
        self.client = client
        self.flush = flush
        self.nameOf = nameOf
        self.busy = busy
        self.onError = onError
    }

    var selfSession: AVCaptureSession? { sender?.session }

    func layer(for contactId: String) -> AVSampleBufferDisplayLayer? { receivers[contactId]?.layer }

    func noticeAccepted() -> Bool { (try? client()?.groupCallNoticeAccepted()) == true }

    func acceptNotice() { try? client()?.groupCallAcceptNotice() }

    // ------------------------------------------------------------ acoes

    /// As sessoes que faltam com a lista da chamada (antes de entrar).
    private func ensureCallSessions(_ c: QtClient, _ callId: String) async {
        let ids = (try? await background { try c.groupCallNeedingSession(callId: callId) }) ?? []
        if !ids.isEmpty { await ensureSessions(ids) }
    }

    /// ADICIONAR A LIGACAO (0.9.71), com o aviso ja aceite. Numa chamada de grupo: os novos entram
    /// na lista de todos. Numa chamada a dois atendida (`peer` = esse contacto): ela passa a
    /// chamada de grupo; `detach` para o microfone e o ecra da chamada a dois antes do audio do grupo.
    func add(_ publicIds: [String], peer: Contact?, video: Bool, detach: @escaping () -> Void) {
        guard let c = client() else { return }
        if let cur = ui {
            guard cur.joined else { return }
            let cam = cur.camera
            Task {
                do {
                    _ = try await background { try c.groupCallAdd(publicIds: publicIds, video: cam) }
                } catch {
                    onError(joinError(error))
                    return
                }
                await flush()
                await refresh(c)
            }
            return
        }
        guard let p = peer else { return }
        Task {
            let st: GroupCallStart
            do {
                st = try await background { try c.groupCallAdd(publicIds: publicIds + [p.publicId], video: video) }
            } catch {
                onError(joinError(error))
                return
            }
            detach()
            let h = inherit()
            ui = GroupCallUi(callId: st.callId, fromId: p.contactId, title: nameOf(p.contactId), joined: true,
                             video: video, muted: h?.muted ?? false, speakerOn: h?.speaker ?? true,
                             camera: video, startedAt: monotonicSeconds())
            sendName(c, st.callId)
            await flush()
            startMedia(c, st.callId)
            await refresh(c)
            ensureTicker()
        }
    }

    /// ENTRAR na chamada que esta a tocar.
    func join() {
        guard let c = client(), let g = ui, !g.joined, !joining else { return }
        joining = true
        let id = g.callId
        // Tocou pelo CallKit: o sistema fica a saber que se atendeu e ativa o audio.
        if rangViaCallKit, let uuid = systemCallId { CallKitBridge.shared.answerFromApp(uuid) }
        Task {
            defer { joining = false }
            do {
                await ensureCallSessions(c, id)
                try await background { try c.groupCallJoin(callId: id) }
            } catch {
                onError(joinError(error))
                end()
                return
            }
            // Vinda de uma chamada a dois, a camara fica como estava nela (decide o nucleo).
            let camera = (try? await background { try c.groupCallCamera(callId: id) }) ?? false
            ui?.joined = true
            ui?.camera = camera
            ui?.startedAt = monotonicSeconds()
            sendName(c, id)
            await flush()
            startMedia(c, id)
            await refresh(c)
        }
    }

    /// ENTRAR DEPOIS numa chamada que recusei (ou nao atendi): o botao na conversa com quem convidou.
    func rejoin(_ fromId: String) {
        guard let c = client(), let id = ongoing[fromId], ui == nil, !busy() else { return }
        Task {
            do {
                await ensureCallSessions(c, id)
                try await background { try c.groupCallRejoin(callId: id) }
            } catch {
                onError(joinError(error))
                await updateOngoing(c, fromId)
                return
            }
            let video = (try? c.groupCallIsVideo(callId: id)) ?? false
            ui = GroupCallUi(callId: id, fromId: fromId, title: nameOf(fromId), joined: true,
                             video: video, camera: video, startedAt: monotonicSeconds())
            await updateOngoing(c, fromId)
            sendName(c, id)
            await flush()
            startMedia(c, id)
            await refresh(c)
            ensureTicker()
        }
    }

    private func updateOngoing(_ c: QtClient, _ fromId: String) async {
        let id = try? await background { try c.groupCallOngoing(contactId: fromId) }
        if let id = id ?? nil { ongoing[fromId] = id } else { ongoing[fromId] = nil }
    }

    /// SAIR (ou recusar o convite).
    func leave() {
        guard let g = ui else { return }
        let c = client()
        Task {
            _ = try? await background { () -> Void in try c?.groupCallLeave(callId: g.callId) }
            await flush()
            end(fromUser: true)
        }
    }

    func toggleMute() {
        guard var g = ui else { return }
        g.muted.toggle()
        io?.muted = g.muted
        ui = g
        if let uuid = systemCallId { CallKitBridge.shared.setMuted(uuid, g.muted) }
    }

    // ------------------------------------------------------------ CallKit

    /// Esta chamada do CallKit e a desta chamada de grupo?
    func ownsSystem(_ uuid: UUID) -> Bool { ui != nil && systemCallId == uuid }

    /// "Desligar" ou "Recusar" no ecra do iPhone.
    func leaveFromSystem() {
        systemEnded = true
        leave()
    }

    /// O botao "silenciar" do ecra do iPhone.
    func setMutedFromSystem(_ muted: Bool) {
        guard var g = ui, g.muted != muted else { return }
        g.muted = muted
        io?.muted = muted
        ui = g
    }

    /// O CallKit ativou o audio.
    func audioSessionActivated() {
        audioActive = true
        startIO()
    }

    /// O CallKit desativou o audio (outra chamada por cima, por exemplo).
    func audioSessionDeactivated() {
        audioActive = false
        io?.stop()
        io = nil
    }

    func toggleSpeaker() {
        guard var g = ui else { return }
        g.speakerOn.toggle()
        io?.setSpeaker(g.speakerOn)
        ui = g
    }

    func toggleCamera() {
        guard let c = client(), var g = ui, g.joined else { return }
        g.camera.toggle()
        ui = g
        let (id, on) = (g.callId, g.camera)
        Task {
            _ = try? await background { () -> Void in try c.groupCallSetCamera(callId: id, on: on) }
            await flush()
            syncCamera()
        }
    }

    func switchCamera() {
        sender?.switchCamera()
        ui?.front.toggle()
    }

    /// DIAGNOSTICO (premir o tempo da chamada, como nas chamadas a dois): os numeros de cada
    /// participante, com o nome. Sem IPs nem ids.
    func diagnostic() -> String? {
        guard let a = audio, let g = ui else { return nil }
        let body = g.tiles.isEmpty ? s("gcall_waiting")
            : g.tiles.map { "== \($0.name) ==\n" + a.diagnosticOf(contactId: $0.contactId) }.joined(separator: "\n\n")
        return "Sealer \(Config.version) (iOS \(UIDevice.current.systemVersion))\n\(body)"
    }

    /// A app saiu do ecra (ou voltou): a minha camara pausa sem avisar ninguem.
    func hold(_ on: Bool) { audio?.videoHold(on: on) }

    // ------------------------------------------------------------ eventos

    /// Chamar depois de cada lote recebido e de segundo a segundo durante a chamada.
    func onEvents(_ c: QtClient) async {
        guard let events = try? await background({ try c.groupCallEvents() }), !events.isEmpty else { return }
        for ev in events {
            switch ev {
            case .incoming(let callId, let from, let video, let fromCall):
                if busy() || ui != nil {
                    _ = try? await background { () -> Void in try c.groupCallLeave(callId: callId) }
                    if let uuid = takePendingRing() { CallKitBridge.shared.reportEnded(uuid, reason: .failed) }
                    continue
                }
                let h = fromCall ? inherit() : nil
                ui = GroupCallUi(callId: callId, fromId: from, title: nameOf(from), joined: false,
                                 video: video, fromName: nameOf(from),
                                 muted: h?.muted ?? false, speakerOn: h?.speaker ?? true)
                ensureTicker()
                // Vinha da chamada a dois com quem me adicionou: entro logo, sem tocar (com o aviso
                // ja aceite; senao fica a tocar para o aceitar).
                if fromCall && noticeAccepted() {
                    join()
                    continue
                }
                ringViaCallKit(from: from, video: video)
                if consumeAnswer() && noticeAccepted() { join() }
            case .changed:
                await refresh(c)
            case .ended(let callId, let reason):
                guard let g = ui, g.callId == callId else { continue }
                let missed = reason == .missed && !g.joined
                if missed { showMissed(g.fromName) }
                end(reason: missed ? .unanswered : .remoteEnded)
            case .ongoing(let contactId):
                await updateOngoing(c, contactId)
            }
        }
    }

    // ------------------------------------------------------------ por dentro

    /// Fica com o que a chamada a dois deixou.
    private func inherit() -> CallHandover? {
        guard let h = takeHandover() else { return nil }
        systemCallId = h.callKitId
        audioActive = h.audioActive
        return h
    }

    /// Toca como uma chamada a dois: pelo CallKit, com a app aberta, em segundo plano ou fechada.
    private func ringViaCallKit(from: String, video: Bool) {
        rangViaCallKit = true
        if let uuid = takePendingRing() {
            // O toque que o PushKit ja mostrou (sem nome) passa a ter o nome.
            systemCallId = uuid
            CallKitBridge.shared.updateName(uuid, name: callKitName(from), video: video)
            return
        }
        if systemCallId != nil { return }
        let uuid = UUID()
        systemCallId = uuid
        CallKitBridge.shared.reportIncoming(uuid, name: callKitName(from), video: video) { [weak self] ok in
            if !ok { Task { @MainActor in self?.leaveFromSystem() } }
        }
    }

    /// Chamada perdida: o aviso de sempre, com o nome (sem ele com o bloqueio da app).
    private func showMissed(_ name: String) {
        let content = UNMutableNotificationContent()
        content.title = Config.appName
        content.body = s("t_chamada_perdida")
        if !callKitName(ui?.fromId ?? "").isEmpty, !name.isEmpty { content.body = s("t_chamada_perdida_de", name) }
        content.sound = .default
        content.threadIdentifier = "qt"
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
    }

    /// O que dizer quando nao deu para entrar ou adicionar.
    private func joinError(_ error: Error) -> String {
        if case QtError.InvalidInput(let field) = error {
            switch field {
            case "group_call_full": return s("gcall_full")
            case "call_id": return s("t_a_chamada_ja_nao_esta_disponivel")
            case "call_busy": return s("t_ocupado_noutra_chamada")
            default: break
            }
        }
        return s("t_erro", safeMessage(error))
    }

    private func pin(_ hex: String) -> Data? {
        let h = hex.trimmingCharacters(in: .whitespaces)
        guard h.count == 64 else { return nil }
        var out = Data(capacity: 32)
        var i = h.startIndex
        while i < h.endIndex {
            let j = h.index(i, offsetBy: 2)
            guard let b = UInt8(h[i..<j], radix: 16) else { return nil }
            out.append(b)
            i = j
        }
        return out
    }

    /// O microfone, o altifalante, a rede e (com a camara ligada) a camara.
    private func startMedia(_ c: QtClient, _ callId: String) {
        guard audio == nil else { return }
        let main = Relays.main
        guard let mainPin = pin(main.pin) else { return }
        let others = Relays.all.dropFirst().compactMap { r in pin(r.pin).map { CallRelayInfo(name: r.name, url: r.url, publicKey: $0) } }
        guard let a = try? c.groupCallAudioStart(callId: callId, relayUrl: main.url, relayPublicKey: mainPin, otherRelays: Array(others)) else {
            onError(s("t_erro", "audio"))
            return
        }
        audio = a
        let n = VideoNet()
        n.start { [weak a] k in a?.setNetwork(kind: k) }
        net = n
        // Uma interrupcao (alarme, Siri) para o motor de audio: volta a arrancar quando acaba.
        interruption = NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] n in
            guard let raw = n.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
            MainActor.assumeIsolated { self?.onInterruption(type) }
        }
        // Sem CallKit: a sessao de audio da chamada e ativada aqui. Com ele, e o sistema que a ativa.
        if systemCallId == nil {
            CallKitBridge.configureAudioSession()
            try? AVAudioSession.sharedInstance().setActive(true)
            audioActive = true
        }
        startIO()
        syncCamera()
    }

    private func onInterruption(_ type: AVAudioSession.InterruptionType) {
        guard ui?.joined == true else { return }
        switch type {
        case .began:
            io?.stop()
            io = nil
        case .ended:
            try? AVAudioSession.sharedInstance().setActive(true)
            startIO()
        @unknown default:
            break
        }
    }

    /// Liga o microfone e o altifalante, quando a sessao de audio esta ativa.
    private func startIO() {
        guard self.io == nil, let a = audio, ui != nil else { return }
        guard audioActive else {
            activateAudioFallback()
            return
        }
        let io = CallAudioIO(audio: a)
        do {
            try io.start()
            io.muted = ui?.muted == true
            io.setSpeaker(ui?.speakerOn != false)
            self.io = io
        } catch {
            onError(s("t_nao_foi_possivel_usar_o_microfone_2", safeMessage(error)))
        }
    }

    /// Ha casos em que o CallKit nao avisa que ativou o audio: ao fim de 1,5 s ativa-se aqui.
    private func activateAudioFallback() {
        let id = ui?.callId
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            guard let self, !self.audioActive, self.io == nil, self.ui?.callId == id else { return }
            CallKitBridge.configureAudioSession()
            guard (try? AVAudioSession.sharedInstance().setActive(true)) != nil else { return }
            self.audioActive = true
            self.startIO()
        }
    }

    private func syncCamera() {
        guard let a = audio else { return }
        let on = ui?.camera == true
        if on, sender == nil {
            let vs = VideoSender(audio: a)
            // Nunca um aviso no ecra sobre a camara (decisao do Nuno, 0.9.67): so o registo.
            vs.onError = { log("SealerVideo", "camara: falhou (grupo)") }
            vs.start(front: ui?.front != false)
            sender = vs
            objectWillChange.send()
        } else if !on, let vs = sender {
            vs.stop()
            sender = nil
            objectWillChange.send()
        }
    }

    /// Quem esta, quem fala e os descodificadores (um por camara ligada).
    private func refresh(_ c: QtClient) async {
        guard let g = ui else { return }
        let id = g.callId
        let parts = (try? await background { try c.groupCallParticipants(callId: id) }) ?? []
        let old = Dictionary(uniqueKeysWithValues: g.tiles.map { ($0.contactId, $0) })
        let tiles = parts.map { p in
            // Nos meus contactos: o nome que lhe dei. Senao: o nome que escolheu para a chamada.
            GroupTile(contactId: p.contactId, name: isContact(p.contactId) ? nameOf(p.contactId) : (p.name ?? nameOf(p.contactId)),
                      connected: p.connected, camera: p.camera,
                      rotation: old[p.contactId]?.rotation ?? 0)
        }
        ui?.tiles = tiles
        ui?.speaker = audio?.speaker()
        ui?.meSpeaking = audio?.meSpeaking() == true
        guard let a = audio else { return }
        let want = Set(tiles.filter { $0.camera && $0.connected }.map(\.contactId))
        for (cid, r) in receivers where !want.contains(cid) {
            r.stop()
            receivers[cid] = nil
        }
        for cid in want where receivers[cid] == nil {
            let r = VideoReceiver(audio: GroupVideoIn(audio: a, contactId: cid))
            r.onRotation = { [weak self] rot in
                guard let self, var u = self.ui else { return }
                u.tiles = u.tiles.map { t in
                    guard t.contactId == cid else { return t }
                    var t = t
                    t.rotation = rot
                    return t
                }
                self.ui = u
            }
            r.start()
            receivers[cid] = r
        }
    }

    private func ensureTicker() {
        guard ticker == nil else { return }
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 500_000_000)
                guard let self, self.ui != nil, let c = self.client() else { break }
                await self.onEvents(c)
                await self.flush()
                await self.refresh(c)
            }
            self?.ticker = nil
        }
    }

    /// Para tudo (saida, fim do toque ou chamada perdida).
    private func end(reason: CXCallEndedReason = .remoteEnded, fromUser: Bool = false) {
        sender?.stop()
        sender = nil
        for r in receivers.values { r.stop() }
        receivers.removeAll()
        io?.stop()
        io = nil
        net?.stop()
        net = nil
        audio = nil
        ticker?.cancel()
        ticker = nil
        if let o = interruption { NotificationCenter.default.removeObserver(o) }
        interruption = nil
        // Com o CallKit, a sessao de audio e dele: desativa-a ao acabar a chamada.
        if let uuid = systemCallId {
            if !systemEnded {
                if fromUser { CallKitBridge.shared.requestEnd(uuid) } else { CallKitBridge.shared.reportEnded(uuid, reason: reason) }
            }
        } else {
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
        systemCallId = nil
        systemEnded = false
        rangViaCallKit = false
        audioActive = false
        ui = nil
    }
}
