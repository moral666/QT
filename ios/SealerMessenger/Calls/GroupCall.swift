import AVFoundation
import Combine
import Foundation
import UIKit

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

struct GroupCallUi: Equatable {
    let callId: String
    let groupId: String
    let groupName: String
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
/// participante com a camara ligada, o toque e o ecra. Sem CallKit (por agora): a chamada de
/// grupo toca com a app aberta.
@MainActor
final class GroupCallModel: ObservableObject {
    @Published private(set) var ui: GroupCallUi?
    /// Chamadas a decorrer em grupos onde nao estou (recusei ou nao atendi): grupo -> chamada.
    @Published private(set) var ongoing: [String: String] = [:]

    private var client: () -> QtClient? = { nil }
    private var flush: () async -> Void = {}
    private var nameOf: (String) -> String = { $0 }
    private var groupNameOf: (String) -> String = { _ in "" }
    private var busy: () -> Bool = { false }
    private var onError: (String) -> Void = { _ in }
    /// Cria as sessoes cifradas que faltam com os membros do grupo (0.9.67), como antes de enviar
    /// uma mensagem ao grupo: sem sessao, dois membros que nao sao contactos nao tinham ligacao.
    private var ensureGroupSessions: (String) async -> Void = { _ in }
    private var isContact: (String) -> Bool = { _ in false }

    /// O nome que escolhi para as chamadas de grupo (0.9.67; so neste iPhone).
    var myName: String {
        get { UserDefaults.standard.string(forKey: "gcall_name") ?? "" }
        set { UserDefaults.standard.set(String(newValue.trimmingCharacters(in: .whitespacesAndNewlines).prefix(32)), forKey: "gcall_name") }
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
    private var ring: AVAudioPlayer?

    func attach(
        client: @escaping () -> QtClient?,
        flush: @escaping () async -> Void,
        nameOf: @escaping (String) -> String,
        groupNameOf: @escaping (String) -> String,
        busy: @escaping () -> Bool,
        onError: @escaping (String) -> Void,
        ensureGroupSessions: @escaping (String) async -> Void = { _ in },
        isContact: @escaping (String) -> Bool = { _ in false }
    ) {
        self.ensureGroupSessions = ensureGroupSessions
        self.isContact = isContact
        self.client = client
        self.flush = flush
        self.nameOf = nameOf
        self.groupNameOf = groupNameOf
        self.busy = busy
        self.onError = onError
    }

    var selfSession: AVCaptureSession? { sender?.session }

    func layer(for contactId: String) -> AVSampleBufferDisplayLayer? { receivers[contactId]?.layer }

    func noticeAccepted() -> Bool { (try? client()?.groupCallNoticeAccepted()) == true }

    func acceptNotice() { try? client()?.groupCallAcceptNotice() }

    // ------------------------------------------------------------ acoes

    /// LIGAR ao grupo (com o aviso ja aceite e as permissoes dadas).
    func start(_ groupId: String, video: Bool) {
        guard let c = client(), ui == nil, !busy() else { return }
        Task {
            do {
                await ensureGroupSessions(groupId)
                let st = try await background { try c.groupCallStart(groupId: groupId, video: video) }
                ui = GroupCallUi(callId: st.callId, groupId: groupId, groupName: groupNameOf(groupId), joined: true,
                                 video: video, camera: video, startedAt: monotonicSeconds())
                sendName(c, st.callId)
                await flush()
                startMedia(c, st.callId)
                ensureTicker()
            } catch {
                onError(s("t_erro", safeMessage(error)))
            }
        }
    }

    /// ENTRAR na chamada que esta a tocar.
    func join() {
        guard let c = client(), let g = ui, !g.joined else { return }
        stopRing()
        let id = g.callId
        Task {
            do {
                await ensureGroupSessions(g.groupId)
                try await background { try c.groupCallJoin(callId: id) }
            } catch {
                onError(s("gcall_full"))
                end()
                return
            }
            ui?.joined = true
            ui?.camera = g.video
            ui?.startedAt = monotonicSeconds()
            sendName(c, id)
            await flush()
            startMedia(c, id)
            await refresh(c)
        }
    }

    /// ENTRAR DEPOIS numa chamada que recusei (ou nao atendi): o botao na conversa do grupo.
    func rejoin(_ groupId: String) {
        guard let c = client(), let id = ongoing[groupId], ui == nil, !busy() else { return }
        Task {
            do {
                await ensureGroupSessions(groupId)
                try await background { try c.groupCallRejoin(callId: id) }
            } catch {
                onError(s("gcall_full"))
                await updateOngoing(c, groupId)
                return
            }
            let video = (try? c.groupCallIsVideo(callId: id)) ?? false
            ui = GroupCallUi(callId: id, groupId: groupId, groupName: groupNameOf(groupId), joined: true,
                             video: video, camera: video, startedAt: monotonicSeconds())
            await updateOngoing(c, groupId)
            sendName(c, id)
            await flush()
            startMedia(c, id)
            await refresh(c)
            ensureTicker()
        }
    }

    private func updateOngoing(_ c: QtClient, _ groupId: String) async {
        let id = try? await background { try c.groupCallOngoing(groupId: groupId) }
        if let id = id ?? nil { ongoing[groupId] = id } else { ongoing[groupId] = nil }
    }

    /// SAIR (ou recusar o convite).
    func leave() {
        guard let g = ui else { return }
        let c = client()
        Task {
            _ = try? await background { () -> Void in try c?.groupCallLeave(callId: g.callId) }
            await flush()
            end()
        }
    }

    func toggleMute() {
        guard var g = ui else { return }
        g.muted.toggle()
        io?.muted = g.muted
        ui = g
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
            case .incoming(let callId, let groupId, let from, let video):
                if busy() || ui != nil {
                    _ = try? await background { () -> Void in try c.groupCallLeave(callId: callId) }
                    continue
                }
                ui = GroupCallUi(callId: callId, groupId: groupId, groupName: groupNameOf(groupId), joined: false,
                                 video: video, fromName: nameOf(from))
                startRing()
                ensureTicker()
            case .changed:
                await refresh(c)
            case .ended(let callId, let groupId, _):
                if ui?.callId == callId { end() }
                await updateOngoing(c, groupId)
            case .ongoing(let groupId):
                await updateOngoing(c, groupId)
            }
        }
    }

    // ------------------------------------------------------------ por dentro

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
        // Sem CallKit: a sessao de audio da chamada e ativada aqui.
        CallKitBridge.configureAudioSession()
        try? AVAudioSession.sharedInstance().setActive(true)
        let io = CallAudioIO(audio: a)
        do {
            try io.start()
            io.muted = ui?.muted == true
            io.setSpeaker(ui?.speakerOn != false)
            self.io = io
        } catch {
            onError(s("t_nao_foi_possivel_usar_o_microfone_2", safeMessage(error)))
        }
        syncCamera()
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

    private func startRing() {
        guard let url = Bundle.main.url(forResource: "sealer_toque", withExtension: "caf"),
              let p = try? AVAudioPlayer(contentsOf: url) else { return }
        p.numberOfLoops = -1
        p.play()
        ring = p
    }

    private func stopRing() {
        ring?.stop()
        ring = nil
    }

    /// Para tudo (saida, fim do toque ou chamada perdida).
    private func end() {
        stopRing()
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
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        ui = nil
    }
}
