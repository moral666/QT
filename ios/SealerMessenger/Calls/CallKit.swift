import AVFoundation
import CallKit
import Foundation
import UIKit

/// Chamadas de voz e videochamadas no ecra do iPhone (CallKit): tocar com a app fechada ou o iPhone
/// bloqueado, atender, recusar, silenciar, auricular/Bluetooth. O audio, a cifra e a rede
/// continuam no Rust (`CallAudio`); aqui so se fala com o sistema.
///
/// Privacidade (decisao a partir das regras do Nuno):
///  * `includesCallsInRecents = false`: as chamadas NAO aparecem no historico do telefone
///    (que o iOS sincroniza com o iCloud).
///  * O "numero" mostrado ao sistema e sempre o mesmo ("SEALER"); o nome do contacto so vai
///    para o ecra da chamada, e nunca com o bloqueio da app ativo.
final class CallKitBridge: NSObject, CXProviderDelegate {
    static let shared = CallKitBridge()

    private let provider: CXProvider
    private let controller = CXCallController()
    /// Chamadas ja atendidas (para nao pedir "atender" duas vezes ao sistema).
    private var answered = Set<UUID>()

    private override init() {
        let cfg = CXProviderConfiguration()
        cfg.supportsVideo = true   // videochamadas desde a 0.9.50
        cfg.maximumCallGroups = 1
        cfg.maximumCallsPerCallGroup = 1
        cfg.supportedHandleTypes = [.generic]
        cfg.includesCallsInRecents = false
        cfg.ringtoneSound = "sealer_toque.caf"
        if let logo = UIImage(named: "Logo") { cfg.iconTemplateImageData = logo.pngData() }
        provider = CXProvider(configuration: cfg)
        super.init()
        provider.setDelegate(self, queue: nil)   // nil = fila principal
    }

    private static let handle = CXHandle(type: .generic, value: "SEALER")

    private static func update(name: String, video: Bool) -> CXCallUpdate {
        let u = CXCallUpdate()
        u.remoteHandle = handle
        u.localizedCallerName = name.isEmpty ? s(video ? "video_call_sealer" : "t_chamada_sealer") : name
        u.hasVideo = video
        u.supportsHolding = false
        u.supportsGrouping = false
        u.supportsUngrouping = false
        u.supportsDTMF = false
        return u
    }

    /// Chamada a chegar. `done(false)`: o iOS recusou (ex.: "Nao incomodar" ou outra chamada).
    func reportIncoming(_ uuid: UUID, name: String, video: Bool = false, done: @escaping (Bool) -> Void = { _ in }) {
        provider.reportNewIncomingCall(with: uuid, update: Self.update(name: name, video: video)) { error in
            if let error { log("SealerCall", "CallKit recusou a chamada: \(error.localizedDescription)") }
            done(error == nil)
        }
    }

    func updateName(_ uuid: UUID, name: String, video: Bool = false) {
        provider.reportCall(with: uuid, updated: Self.update(name: name, video: video))
    }

    /// LIGAR: o sistema mostra a chamada a sair e ativa o audio.
    func startOutgoing(_ uuid: UUID, name: String, video: Bool = false) {
        let action = CXStartCallAction(call: uuid, handle: Self.handle)
        action.contactIdentifier = nil
        action.isVideo = video
        controller.request(CXTransaction(action: action)) { [weak self] error in
            guard let self else { return }
            if let error { log("SealerCall", "CallKit: ligar falhou: \(error.localizedDescription)"); return }
            self.provider.reportCall(with: uuid, updated: Self.update(name: name, video: video))
            self.provider.reportOutgoingCall(with: uuid, startedConnectingAt: Date())
        }
    }

    func reportConnected(_ uuid: UUID) {
        provider.reportOutgoingCall(with: uuid, connectedAt: Date())
    }

    /// Atendeu-se no ecra da app: diz ao sistema (para ativar o audio).
    func answerFromApp(_ uuid: UUID) {
        guard !answered.contains(uuid) else { return }
        answered.insert(uuid)
        controller.request(CXTransaction(action: CXAnswerCallAction(call: uuid))) { error in
            if let error { log("SealerCall", "CallKit: atender falhou: \(error.localizedDescription)") }
        }
    }

    func setMuted(_ uuid: UUID, _ muted: Bool) {
        controller.request(CXTransaction(action: CXSetMutedCallAction(call: uuid, muted: muted))) { _ in }
    }

    /// Desligou-se na app.
    func requestEnd(_ uuid: UUID) {
        answered.remove(uuid)
        controller.request(CXTransaction(action: CXEndCallAction(call: uuid))) { [weak self] error in
            if error != nil { self?.provider.reportCall(with: uuid, endedAt: Date(), reason: .remoteEnded) }
        }
    }

    /// Acabou do outro lado (ou nunca chegou).
    func reportEnded(_ uuid: UUID, reason: CXCallEndedReason) {
        answered.remove(uuid)
        provider.reportCall(with: uuid, endedAt: Date(), reason: reason)
    }

    /// Modo de audio de chamada (cancelamento de eco do iPhone), antes de o sistema o ativar.
    static func configureAudioSession() {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetooth, .allowBluetoothA2DP])
        try? session.setPreferredSampleRate(48_000)
        try? session.setPreferredIOBufferDuration(0.01)
    }

    // ------------------------------------------------------------------ CXProviderDelegate
    // A fila do provider e a principal (setDelegate com queue nil).

    func providerDidReset(_ provider: CXProvider) {
        answered.removeAll()
        MainActor.assumeIsolated { AppModel.shared.callAudioSessionDeactivated() }
    }

    func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
        Self.configureAudioSession()
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        Self.configureAudioSession()
        answered.insert(action.callUUID)
        let uuid = action.callUUID
        MainActor.assumeIsolated { AppModel.shared.answerFromSystem(uuid) }
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        answered.remove(action.callUUID)
        let uuid = action.callUUID
        MainActor.assumeIsolated { AppModel.shared.endFromSystem(uuid) }
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        let muted = action.isMuted
        MainActor.assumeIsolated { AppModel.shared.setMutedFromSystem(muted) }
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXSetHeldCallAction) {
        action.fail()   // sem chamadas em espera
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        MainActor.assumeIsolated { AppModel.shared.callAudioSessionActivated() }
    }

    func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        MainActor.assumeIsolated { AppModel.shared.callAudioSessionDeactivated() }
    }
}
