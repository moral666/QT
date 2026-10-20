import AVFoundation
import CallKit
import Foundation
import LocalAuthentication
import SwiftUI
import UIKit
import UserNotifications

struct UiState {
    var loading = true
    var hasIdentity = false
    var publicId = ""
    var relayUrl = Config.relayURL
    var relayKeyHex = Config.relayPin
    /// Estado da ligacao: nunca mostrado na app (decisao do Nuno), so no registo.
    var status = ""
    var error: String?
    var contacts: [Contact] = []
    /// Conversas com historico (vem do SQLCipher, no Rust), a mais recente primeiro.
    var conversations: [ConversationSummary] = []
    /// Conversa aberta (contactId ou "g:...") e a janela de mensagens, da MAIS RECENTE para a mais antiga.
    var openChatId: String?
    var chatMessages: [ChatMessage] = []
    var chatHasMore = false
    /// Enviar recibos "Visto" (o utilizador pode desligar).
    var readReceipts = true
    /// Notificacoes com a app fechada (interruptor "Notificações").
    var pushOn = true
    /// Ja se pediu a permissao de notificacoes uma vez.
    var pushAsked = false
    /// Safety number a mostrar (nome + 60 digitos), ou nil.
    var safetyNumber: String?
    // ----- bloqueio da app -----
    var locked = false
    var lockEnabled = false
    var bioEnabled = false
    var bioAvailable = false
    var lockTimeoutMs: Int64 = defaultLockTimeoutMs
    var unlocking = false
    var lockMessage: String?
    /// Progresso (0 a 1) dos anexos a enviar/descarregar, por message_id.
    var transfers: [String: Double] = [:]
    /// Nao foi possivel apagar a conta no servidor (motivo). Nada foi apagado.
    var wipeServerFailed: String?
    /// Contactos bloqueados (contactId).
    var blocked: Set<String> = []
    /// Convites: quem me escreveu sem estar nos contactos (contactId).
    var invites: Set<String> = []
    /// Aceitou os termos de utilizacao em vigor (termsVersion).
    var termsAccepted = true
    /// Grupos (ate 10 pessoas). A conversa de cada um tem o id `conversationId` ("g:...").
    var groups: [GroupInfo] = []
    /// Mostrar aos contactos quando estou online (e ver o deles).
    var presenceOn = true
    /// Contactos com a app aberta agora (contactId). So em memoria.
    var online: Set<String> = []
    /// Chamada de voz a decorrer.
    var call: CallUi?
    /// Fotos de perfil dos contactos (contactId). So em memoria.
    var photos: [String: UIImage] = [:]
    var myPhoto: UIImage?
    /// Badges dos contactos (contactId -> numero, so os que tem). Verificados no Rust.
    var badges: [String: Int] = [:]
    /// O meu badge (0 = nenhum).
    var myBadge: Int = 0
    /// Daqui a quantos ms posso trocar de foto (0 = ja).
    var photoNextChangeMs: Int64 = 0
    /// Foto escolhida e ja reduzida, a espera de "Aplicar". So em memoria.
    var photoPreview: UIImage?
    /// Aviso curto a mostrar uma vez (ex.: "Foto atualizada").
    var notice: String?

    func groupOf(_ conversationId: String) -> GroupInfo? {
        conversationId.hasPrefix("g:") ? groups.first { $0.conversationId == conversationId } : nil
    }

    /// Nome de um membro de um grupo: "Tu", o nome nos contactos, ou um nome curto.
    func memberName(_ contactId: String) -> String {
        if publicId.count >= 68, contactIdOf(publicId) == contactId { return s("t_tu") }
        return contacts.first { $0.contactId == contactId }?.nickname ?? s("t_membro", String(contactId.prefix(6)))
    }

    func contact(_ id: String) -> Contact? { contacts.first { $0.contactId == id } }
}

enum CallPhase { case calling, ringingOut, incoming, active, ended }

struct CallUi: Equatable {
    let callId: String
    let contactId: String
    var name: String
    var phase: CallPhase
    /// Quando foi atendida (relogio monotono, s), para o contador.
    var answeredAt: Double = 0
    var muted = false
    var speaker = false
    var endText = ""
}

/// Versao dos termos de utilizacao: subir quando mudarem, para voltar a pedir a aceitacao.
let termsVersion = 1
let defaultLockTimeoutMs: Int64 = 60_000

/// Relogio que conta tambem com o iPhone a dormir (como o elapsedRealtime do Android).
func monotonicSeconds() -> Double { Double(clock_gettime_nsec_np(CLOCK_MONOTONIC)) / 1e9 }

/// Mesma logica que o MainViewModel.kt (e o SealerCore.kt) do Android. Um so por processo
/// (`AppModel.shared`): o ecra e o segundo plano (chamadas acordadas pelo PushKit) usam o
/// mesmo QtClient, porque dois na mesma base de dados estragavam o Double Ratchet.
/// O estado vive na thread principal; tudo o que toca no SQLCipher/criptografia corre em
/// `background { }`.
@MainActor
final class AppModel: ObservableObject {
    static let shared = AppModel()

    @Published private(set) var ui = UiState()

    private let prefs = UserDefaults.standard   // so URL do relay, chave PUBLICA e opcoes (nao secretos)
    private var qt: QtClient?
    /// Chave da lista de contactos; so existe em memoria com a base de dados aberta.
    private var contactsKey: Data?
    private var link: RelayLink?
    private var poller: Task<Void, Never>?
    /// Segunda ligacao, so para anexos: um envio grande nao atrasa as mensagens.
    private var fileLink: RelayLink?
    /// Terceira ligacao, so para ESPERAR por mensagens (o relay responde quando chega uma).
    private var waitLink: RelayLink?
    private var transferTask: Task<Void, Never>?
    private var transferKick: AsyncStream<Void>.Continuation?
    private let flushLock = AsyncLock()
    private let receiveLock = AsyncLock()
    /// Quando a app foi para segundo plano (relogio monotono).
    private var hiddenAt: Double?
    private var pauseTask: Task<Void, Never>?
    private var linksPaused = false
    /// As chaves publicas ainda nao foram republicadas desde que a app abriu.
    private var publishOnResume = false
    /// A app esta no ecra (cena ativa).
    private(set) var visible = false
    /// Bloquear quando a chamada acabar (o tempo do bloqueio passou durante ela).
    private var lockAfterCall = false
    private var presenceTask: Task<Void, Never>?
    private var deletedNotified = Set<String>()
    private var photoCache: [String: (Int, UIImage)] = [:]
    private var pendingPhoto: Data?
    private var bgTask: UIBackgroundTaskIdentifier = .invalid
    /// Anexos decifrados em memoria (imagens e voz; ficheiros nunca).
    private let attachmentCache = AttachmentCache()
    // ------------------------------------------------------------- chamadas
    private var callAudio: CallAudio?
    private var callIO: CallAudioIO?
    /// O CallKit ja ativou o audio desta chamada (so entao se liga o microfone).
    private var audioSessionActive = false
    private var callTicker: Task<Void, Never>?
    /// Eventos de chamada lidos mas ainda por tratar.
    private var callStatsTick = 0
    /// Chamada mostrada pelo CallKit (callId -> UUID).
    private var callKitIds: [String: UUID] = [:]
    /// Push VoIP que ainda nao se sabe de que chamada e (bloqueio da app, ou a oferta ainda a chegar).
    private var pendingRing: (uuid: UUID, at: Double)?
    /// Carregou-se em "Atender" no CallKit antes de a app saber qual era a chamada.
    private var answerRequestedAt: Double = 0

    private static let pollSeconds: UInt64 = 3
    private static let backgroundGraceSeconds: UInt64 = 25
    private static let waitSeconds: UInt32 = 25
    private static let waitTimeoutSeconds: Double = 35
    private static let waitRetrySeconds: Double = 300
    private static let relayUnknownRequest = "mensagem invalida"
    private static let maxPages = 20
    private static let maxTextBytes = 9_000
    private static let page = 50
    private static let refreshMax = 200
    private static let chunk: UInt64 = 40 * 1024
    private static let window = 8
    private static let presenceTickSeconds: UInt64 = 60
    private static let endedShowSeconds: Double = 2
    /// Quanto tempo uma chamada toca (igual ao Rust).
    static let ringSeconds: Double = 45
    private static let answerValidSeconds: Double = 60
    static let myPhotoKey = "\u{0}eu"

    private init() {
        // Aberta pelo utilizador (inactive -> active) ou acordada em segundo plano por uma chamada.
        visible = UIApplication.shared.applicationState != .background
        AppFiles.cleanTemporary()
        Task { await openStorage() }
        presenceTask = Task { [weak self] in await self?.presenceLoop() }
    }

    // ================================================================ armazenamento

    private func dbFile() throws -> URL { try AppFiles.url("qt.db") }

    /// Abre a base de dados, ou mostra o ecra de bloqueio se o codigo estiver ativo.
    private func openStorage() async {
        do {
            if await background({ StorageKeyProvider.lockEnabled() }) {
                showLocked()
                return
            }
            let db = try dbFile()
            let key = try await background { try StorageKeyProvider.storageKey(dbFile: db) }
            try await openClient(key)
        } catch is AppLockedError {
            showLocked()
        } catch let e as SecureStoreError {
            // Nunca se cria uma chave nova por cima: o utilizador decide se apaga os dados locais.
            ui.loading = false
            ui.error = s("t_chave_do_armazenamento_seguro_indisponivel", e.message)
        } catch {
            ui.loading = false
            ui.error = s("t_nao_foi_possivel_abrir_o_armazenamento", safeMessage(error))
        }
    }

    /// Abre o QtClient com a chave do armazenamento. Apaga `key` da memoria no fim.
    private func openClient(_ key: Data) async throws {
        let db = try dbFile()
        let ck = contactsKeyFrom(key)
        let client: QtClient = try await background {
            var k = key
            defer { k.resetBytes(in: 0..<k.count) }
            AppFiles.applyProtection()
            return try QtClient.open(path: db.path, storageKey: k)
        }
        contactsKey = ck
        try await setupClient(client)
    }

    private func setupClient(_ client: QtClient) async throws {
        qt = client
        guard let ck = contactsKey else { throw SecureStoreError(message: "sem a chave dos contactos") }
        let url = Config.relayEditable ? (prefs.string(forKey: "relay_url") ?? Config.relayURL) : Config.relayURL
        let pin = Config.relayEditable ? (prefs.string(forKey: "relay_key") ?? Config.relayPin) : Config.relayPin
        struct Loaded: Sendable {
            let has: Bool; let publicId: String; let contacts: [Contact]; let convs: [ConversationSummary]
            let blocked: [String]; let invites: [String]; let lockEnabled: Bool
        }
        let l = try await background { () -> Loaded in
            if ContactStore.isLegacy() {
                let old = ContactStore.load(key: ck)
                if !old.isEmpty { try? ContactStore.save(old, key: ck) }
            }
            let has = try client.hasIdentity()
            let pid = try has ? client.exportPublicIdentity().publicId : ""
            return Loaded(
                has: has, publicId: pid,
                contacts: ContactStore.load(key: ck), convs: try client.conversations(),
                blocked: try client.blockedContacts(), invites: try client.pendingIntros(),
                lockEnabled: StorageKeyProvider.lockEnabled()
            )
        }
        var u = ui
        u.loading = false
        u.hasIdentity = l.has
        u.relayUrl = url
        u.relayKeyHex = pin
        u.publicId = l.publicId
        u.contacts = l.contacts
        u.conversations = l.convs
        u.readReceipts = prefs.object(forKey: "read_receipts") as? Bool ?? true
        u.pushOn = prefs.object(forKey: "push_on") as? Bool ?? true
        u.pushAsked = prefs.bool(forKey: "push_asked")
        u.locked = false
        u.unlocking = false
        u.lockMessage = nil
        u.lockEnabled = l.lockEnabled
        u.bioEnabled = BioUnlock.isEnabled()
        u.bioAvailable = BioUnlock.isAvailable()
        u.lockTimeoutMs = (prefs.object(forKey: "lock_timeout_ms") as? Int64) ?? defaultLockTimeoutMs
        u.blocked = Set(l.blocked)
        u.invites = Set(l.invites)
        u.termsAccepted = prefs.integer(forKey: "terms_version") >= termsVersion
        u.presenceOn = prefs.object(forKey: "presence") as? Bool ?? true
        u.online = []
        ui = u
        await syncPresence(client)
        if l.has {
            // Ecra visivel, ou uma chamada acordou a app (PushKit): liga ja. Senao, ao voltar.
            if visible || pendingRing != nil { await startLink() } else { linksPaused = true; publishOnResume = true }
        }
        await refreshViews(client)
    }

    /// Ecra de bloqueio, sem nenhum dado das conversas no estado.
    private func showLocked(_ message: String? = nil) {
        photoCache.removeAll()
        var u = UiState()
        u.loading = false
        u.locked = true
        u.lockEnabled = true
        u.bioEnabled = BioUnlock.isEnabled()
        u.bioAvailable = BioUnlock.isAvailable()
        u.lockTimeoutMs = (prefs.object(forKey: "lock_timeout_ms") as? Int64) ?? defaultLockTimeoutMs
        u.lockMessage = message
        u.call = ui.call.flatMap { $0.phase == .ended ? nil : $0 }
        ui = u
    }

    /// Fecha a base de dados e as ligacoes ao relay; apaga as chaves da memoria.
    private func closeSession() {
        stopCallMedia()
        for (_, uuid) in callKitIds { CallKitBridge.shared.reportEnded(uuid, reason: .remoteEnded) }
        callKitIds.removeAll()
        ui.call = nil
        closeLinks()
        attachmentCache.clear()
        FilePreview.clear()
        pendingPhoto = nil
        qt = nil
        if let n = contactsKey?.count { contactsKey?.resetBytes(in: 0..<n) }
        contactsKey = nil
    }

    private func closeLinks() {
        poller?.cancel(); poller = nil
        transferTask?.cancel(); transferTask = nil
        let ls = [link, fileLink, waitLink]
        link = nil; fileLink = nil; waitLink = nil
        Task { for l in ls { await l?.close() } }
    }

    // ================================================================ bloqueio da app

    /// Bloqueia ja (so se o bloqueio estiver ativo).
    func lockNow() {
        guard ui.lockEnabled, !ui.locked, ui.call == nil else { return }
        closeSession()
        showLocked()
    }

    /// A app saiu do ecra (cena em segundo plano). Conta o tempo para o bloqueio e, ao fim de
    /// alguns segundos, fecha as ligacoes ao relay (o iOS corta-as de qualquer forma).
    func onAppHidden() {
        guard visible else { return }
        visible = false
        hiddenAt = monotonicSeconds()
        Task { await presenceAnnounce(false) }
        pauseTask?.cancel()
        if ui.call != nil { return }   // chamada: o CallKit mantem a app viva, as ligacoes ficam
        // O iOS da ~30 s para acabar o que estava a meio (envios, "offline", recibos).
        if bgTask == .invalid {
            bgTask = UIApplication.shared.beginBackgroundTask(withName: "sealer-envios") { [weak self] in
                MainActor.assumeIsolated { self?.pauseLinks(); self?.endBackgroundTask() }
            }
        }
        pauseTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.backgroundGraceSeconds * 1_000_000_000)
            guard let self, !Task.isCancelled else { return }
            self.pauseLinks()
            self.endBackgroundTask()
        }
    }

    private func endBackgroundTask() {
        if bgTask != .invalid {
            UIApplication.shared.endBackgroundTask(bgTask)
            bgTask = .invalid
        }
    }

    /// A app voltou ao ecra: bloqueia se passou o tempo escolhido; senao, religa ao relay.
    func onAppVisible() {
        let wasVisible = visible
        visible = true
        pauseTask?.cancel(); pauseTask = nil
        endBackgroundTask()
        UNUserNotificationCenter.current().removeAllDeliveredNotifications()
        if let t = hiddenAt {
            hiddenAt = nil
            if ui.lockEnabled, !ui.locked, monotonicSeconds() - t >= Double(ui.lockTimeoutMs) / 1000 {
                if ui.call == nil && pendingRing == nil { lockNow() } else { lockAfterCall = true }
            }
        }
        if ui.locked || ui.loading { return }
        if !wasVisible, let w = waitLink {
            // Em segundo plano o iOS corta as ligacoes: a espera pendente estaria num socket morto.
            Task { await w.close() }
        }
        resumeLinks()
        Task {
            guard let c = qt else { return }
            if link != nil { await presenceAnnounce(true) }
            await pollOnce()
            await refreshViews(c)
        }
        kickTransfers()
    }

    private func pauseLinks() {
        if visible || qt == nil || link == nil || ui.call != nil || pendingRing != nil { return }
        closeLinks()
        linksPaused = true
        log("SealerRelay", "segundo plano: ligacoes ao relay fechadas")
    }

    private func resumeLinks() {
        guard linksPaused else { return }
        linksPaused = false
        if ui.locked { return }
        Task {
            guard let c = qt, (try? await background({ try c.hasIdentity() })) == true, link == nil else { return }
            let publish = publishOnResume
            publishOnResume = false
            await startLink(publishBundle: publish)
        }
    }

    func unlockWithCode(_ code: String) {
        let wait = lockWaitMs()
        if wait > 0 { ui.lockMessage = waitMessage(wait); return }
        ui.unlocking = true
        ui.lockMessage = nil
        Task {
            do {
                let key = try await background { try StorageKeyProvider.storageKeyWithCode(code) }
                resetFails()
                try await openClient(key)
                afterUnlock()
            } catch QtError.Crypto {
                let w = registerFail()
                ui.unlocking = false
                ui.lockMessage = s("t_codigo_errado") + (w > 0 ? " " + waitMessage(w) : "")
            } catch QtError.InvalidInput {
                ui.unlocking = false
                ui.lockMessage = s("t_o_codigo_tem_pelo_menos_digitos", Self.minCodeLen)
            } catch {
                ui.unlocking = false
                ui.lockMessage = s("t_nao_foi_possivel_desbloquear", safeMessage(error))
            }
        }
    }

    /// Face ID / Touch ID / codigo do iPhone.
    func requestBioUnlock() {
        guard BioUnlock.isEnabled(), !ui.unlocking else {
            if !BioUnlock.isEnabled() { ui.bioEnabled = false }
            return
        }
        ui.unlocking = true
        ui.lockMessage = nil
        let reason = s("t_desbloquear", Config.appName)
        Task {
            guard let key = await background({ BioUnlock.open(reason: reason) }) else {
                ui.unlocking = false
                ui.bioEnabled = BioUnlock.isEnabled()
                if !ui.bioEnabled { ui.lockMessage = s("ios_bio_changed") }
                return
            }
            do {
                resetFails()
                try await openClient(key)
                afterUnlock()
            } catch {
                ui.unlocking = false
                ui.lockMessage = s("t_nao_foi_possivel_desbloquear_assim_usa")
            }
        }
    }

    /// Desbloqueada: uma chamada que tocou com a app bloqueada passa para o ecra.
    private func afterUnlock() {
        if pendingRing != nil || answerRequested() { Task { await pollOnce() } }
    }

    func enableLock(_ code: String) {
        Task {
            do {
                let db = try dbFile()
                try await background { try StorageKeyProvider.enableLock(code: code, dbFile: db) }
                resetFails()
                ui.lockEnabled = true
                ui.bioAvailable = BioUnlock.isAvailable()
            } catch QtError.InvalidInput {
                showError(s("t_o_codigo_tem_de_ter_pelo", Self.minCodeLen))
            } catch {
                showError(s("t_nao_foi_possivel_ativar_o_bloqueio", safeMessage(error)))
            }
        }
    }

    func disableLock(_ code: String) {
        if lockWaitMs() > 0 { showError(waitMessage(lockWaitMs())); return }
        Task {
            do {
                try await background { try StorageKeyProvider.disableLock(code: code) }
                BioUnlock.disable()
                resetFails()
                ui.lockEnabled = false
                ui.bioEnabled = false
            } catch QtError.Crypto {
                _ = registerFail()
                showError(s("t_codigo_errado"))
            } catch {
                showError(s("t_nao_foi_possivel_desativar_o_bloqueio", safeMessage(error)))
            }
        }
    }

    func changeCode(_ old: String, _ new: String) {
        if lockWaitMs() > 0 { showError(waitMessage(lockWaitMs())); return }
        Task {
            do {
                try await background { try StorageKeyProvider.changeCode(old: old, new: new) }
                resetFails()
                showError(s("t_codigo_alterado"))
            } catch QtError.Crypto {
                _ = registerFail()
                showError(s("t_o_codigo_atual_esta_errado"))
            } catch QtError.InvalidInput {
                showError(s("t_o_codigo_tem_de_ter_pelo", Self.minCodeLen))
            } catch {
                showError(s("t_nao_foi_possivel_mudar_o_codigo", safeMessage(error)))
            }
        }
    }

    /// Ativar o Face ID: confirma o codigo e pede a autenticacao do iPhone.
    func startBioEnable(_ code: String) {
        guard BioUnlock.isAvailable() else { showError(s("ios_bio_unavailable")); return }
        if lockWaitMs() > 0 { showError(waitMessage(lockWaitMs())); return }
        let reason = s("t_ativar_desbloqueio_rapido")
        Task {
            do {
                try await background {
                    var key = try StorageKeyProvider.storageKeyWithCode(code)
                    defer { key.resetBytes(in: 0..<key.count) }
                    try BioUnlock.save(key, reason: reason)
                }
                resetFails()
                ui.bioEnabled = true
            } catch QtError.Crypto {
                _ = registerFail()
                showError(s("t_codigo_errado"))
            } catch let e as LAError where [.userCancel, .appCancel, .systemCancel].contains(e.code) {
                // Cancelou o Face ID: nada a dizer.
            } catch {
                showError(s("t_nao_foi_possivel_ativar", safeMessage(error)))
            }
        }
    }

    func disableBio() {
        BioUnlock.disable()
        ui.bioEnabled = false
    }

    func setLockTimeout(_ ms: Int64) {
        prefs.set(ms, forKey: "lock_timeout_ms")
        ui.lockTimeoutMs = ms
    }

    /// Esqueci-me do codigo: so resta apagar tudo e comecar de novo.
    func wipeLockedData() { Task { await resetStorage() } }

    private func resetStorage() async {
        closeSession()
        BioUnlock.disable()
        StorageKeyProvider.deleteKeyFile()
        if let db = try? dbFile() {
            for suffix in ["", "-wal", "-shm", "-journal"] { try? FileManager.default.removeItem(atPath: db.path + suffix) }
            try? FileManager.default.removeItem(atPath: db.path + ".anexos")
        }
        ContactStore.wipe()
        for k in ["relay_url", "relay_key", "read_receipts", "push_on", "push_asked", "lock_timeout_ms", "terms_version",
                  "presence", "lock_fails", "lock_until"] {
            prefs.removeObject(forKey: k)
        }
        ui = UiState()
        await openStorage()
    }

    private func lockWaitMs() -> Int64 {
        max(0, Int64(prefs.double(forKey: "lock_until") * 1000) - Int64(Date().timeIntervalSince1970 * 1000))
    }

    /// Tentativas erradas: depois de 5, espera 30 s, 1 min, 2 min... ate 15 min.
    private func registerFail() -> Int64 {
        let n = prefs.integer(forKey: "lock_fails") + 1
        let wait: Int64 = n < 5 ? 0 : min(30_000 << Int64(min(n - 5, 5)), 900_000)
        prefs.set(n, forKey: "lock_fails")
        prefs.set(Date().timeIntervalSince1970 + Double(wait) / 1000, forKey: "lock_until")
        return wait
    }

    private func resetFails() {
        prefs.removeObject(forKey: "lock_fails")
        prefs.removeObject(forKey: "lock_until")
    }

    private func waitMessage(_ ms: Int64) -> String { s("t_demasiadas_tentativas_tenta_de_novo_daqui", (ms + 999) / 1000) }

    static var minCodeLen: Int { Int(appLockMinCodeLen()) }

    // ================================================================ configuracao

    func createIdentity(relayUrl: String, relayKeyHex: String) {
        Task {
            guard let client = qt else { return }
            if Config.relayEditable && !applyRelay(relayUrl, relayKeyHex) { return }
            do {
                let me = try await background { try client.createIdentity() }
                ui.hasIdentity = true
                ui.publicId = me.publicId
                await startLink()
            } catch {
                showError(s("t_erro", safeMessage(error)))
            }
        }
    }

    func saveRelay(relayUrl: String, relayKeyHex: String) {
        guard Config.relayEditable, applyRelay(relayUrl, relayKeyHex) else { return }
        Task {
            guard let client = qt, (try? await background({ try client.hasIdentity() })) == true else { return }
            await startLink()
        }
    }

    private func applyRelay(_ relayUrl: String, _ relayKeyHex: String) -> Bool {
        let u = relayUrl.trimmingCharacters(in: .whitespaces)
        let lower = u.lowercased()
        guard lower.hasPrefix("wss://") || (Config.allowCleartext && lower.hasPrefix("ws://")) else {
            showError(s("t_o_relay_tem_de_usar_wss"))
            return false
        }
        let k = relayKeyHex.trimmingCharacters(in: .whitespaces)
        prefs.set(u, forKey: "relay_url")
        prefs.set(k, forKey: "relay_key")
        ui.relayUrl = u
        ui.relayKeyHex = k
        return true
    }

    private func newLink(_ client: QtClient) -> RelayLink {
        RelayLink(qt: client, url: ui.relayUrl, pinnedRelayKeyHex: ui.relayKeyHex.isEmpty ? nil : ui.relayKeyHex)
    }

    /// Abre as ligacoes ao relay e comeca a receber. Com `publishBundle`, republica tambem as
    /// chaves publicas (renova os 90 dias no relay).
    private func startLink(publishBundle: Bool = true) async {
        guard let client = qt else { return }
        closeLinks()
        linksPaused = false
        let l = newLink(client)
        link = l
        let f = newLink(client)
        fileLink = f
        waitLink = newLink(client)
        status("A ligar ao relay…")
        poller = Task { [weak self] in await self?.receiveLoop(client) }
        startTransfers()
        if publishBundle {
            do {
                let req = try await background { try client.relayRequestRegisterBundle(bundleBytes: try client.publishableBundle()) }
                let r = try await f.request(req)
                if case .ack = r {} else { log("SealerRelay", "relay recusou o bundle: \(r.describe)") }
            } catch {
                log("SealerRelay", "bundle: \(safeMessage(error))")
            }
        }
        await registerPush(client, f)
        if visible { await presenceAnnounce(true) }
    }

    // ================================================================ notificacoes

    /// Envia ao relay o token da Apple deste iPhone (ou vazio, com o interruptor desligado).
    func registerPush(_ client: QtClient, _ l: RelayLink) async {
        let token: String
        if ui.pushOn {
            guard let t = PushTokens.shared.relayToken() else { return }
            token = t
        } else {
            token = ""
        }
        do {
            let req = try await background { try client.relayRequestPushToken(token: token) }
            let r = try await l.request(req)
            if case .ack = r {} else { log("SealerPush", "relay recusou o token: \(r.describe)") }
        } catch {
            log("SealerPush", "token: \(safeMessage(error))")
        }
    }

    /// O iOS deu um token novo (alertas ou VoIP): manda-o ao relay se estiver ligado.
    func pushTokensChanged() {
        guard let client = qt, let f = fileLink, ui.hasIdentity else { return }
        Task { await registerPush(client, f) }
    }

    /// Interruptor "Notificações". Desligar avisa o relay (token vazio).
    func setPush(_ on: Bool) {
        prefs.set(on, forKey: "push_on")
        ui.pushOn = on
        if on { PushTokens.shared.askPermission() }
        guard let client = qt, let f = fileLink else { return }
        Task { await registerPush(client, f) }
    }

    func markPushAsked() {
        prefs.set(true, forKey: "push_asked")
        ui.pushAsked = true
    }

    /// Com a app fora do ecra mas ainda ligada: o aviso de sempre, sem nome nem texto.
    private func showGenericNotice() {
        guard !visible, ui.pushOn else { return }
        let content = UNMutableNotificationContent()
        content.title = Config.appName
        content.body = s("t_tens_mensagens_novas")
        content.sound = .default
        content.threadIdentifier = "qt"
        let req = UNNotificationRequest(identifier: "qt-mensagens", content: content, trigger: nil)
        UNUserNotificationCenter.current().add(req)
    }

    // ================================================================ presenca

    private func presenceAnnounce(_ onScreen: Bool) async {
        guard let client = qt else { return }
        if let n = try? await background({ try client.presenceAnnounce(visible: onScreen) }), n > 0 { await flushOutbox() }
    }

    /// Diz ao Rust quem sao os contactos (so eles veem o meu estado) e se a presenca esta ligada.
    private func syncPresence(_ client: QtClient) async {
        let on = ui.presenceOn
        let ids = ui.contacts.map(\.contactId)
        if let n = try? await background({ try client.presenceConfigure(share: on, contactIds: ids) }), n > 0 { await flushOutbox() }
        await syncPhotos(client)
    }

    func setPresence(_ on: Bool) {
        prefs.set(on, forKey: "presence")
        ui.presenceOn = on
        if !on { ui.online = [] }
        Task {
            guard let client = qt else { return }
            await syncPresence(client)
            if on && visible { await presenceAnnounce(true) }
        }
    }

    /// Com a app aberta: de minuto a minuto atualiza quem esta online e manda o batimento.
    private func presenceLoop() async {
        var lastBeat = monotonicSeconds()
        while !Task.isCancelled {
            try? await Task.sleep(nanoseconds: Self.presenceTickSeconds * 1_000_000_000)
            guard let client = qt, visible, link != nil, !ui.locked else { continue }
            if monotonicSeconds() - lastBeat >= Double(presenceHeartbeatMs()) / 1000 {
                lastBeat = monotonicSeconds()
                if let n = try? await background({ try client.presenceHeartbeat() }), n > 0 { await flushOutbox() }
            }
            let on = await background { Set(client.presenceOnline()) }
            if on != ui.online { ui.online = on }
        }
    }

    // ================================================================ foto de perfil

    private func syncPhotos(_ client: QtClient) async {
        let ids = ui.contacts.map(\.contactId)
        if let n = try? await background({ try client.profilePhotoSync(contactIds: ids) }), n > 0 { await flushOutbox() }
        if let n = try? await background({ try client.badgeSync(contactIds: ids) }), n > 0 { await flushOutbox() }
        await refreshPhotos(client)
        await refreshBadges(client)
    }

    /// Badges (o meu e os dos contactos), ja verificados pelo nucleo.
    private func refreshBadges(_ client: QtClient) async {
        let ids = ui.contacts.map(\.contactId)
        struct Raw: Sendable { let theirs: [String: Int]; let mine: Int }
        guard let raw = try? await background({ () -> Raw in
            var m: [String: Int] = [:]
            for id in ids {
                let b = Int(try client.badgeOf(contactId: id))
                if b != Badges.none { m[id] = b }
            }
            return Raw(theirs: m, mine: Int(try client.badgeMine()))
        }) else { return }
        if raw.theirs != ui.badges { ui.badges = raw.theirs }
        if raw.mine != ui.myBadge { ui.myBadge = raw.mine }
    }

    private func refreshPhotos(_ client: QtClient) async {
        let ids = ui.contacts.map(\.contactId)
        struct Raw: Sendable { let theirs: [String: Data]; let mine: Data?; let wait: Int64 }
        guard let raw = try? await background({ () -> Raw in
            var m: [String: Data] = [:]
            for id in ids { if let d = try client.profilePhotoOf(contactId: id) { m[id] = d } }
            return Raw(theirs: m, mine: try client.profilePhotoMine(), wait: try client.profilePhotoNextChangeMs())
        }) else { return }
        func show(_ key: String, _ bytes: Data?) -> UIImage? {
            guard let bytes else { return nil }
            let h = bytes.hashValue
            if let c = photoCache[key], c.0 == h { return c.1 }
            guard let img = ProfilePhoto.decode(bytes) else { return nil }
            photoCache[key] = (h, img)
            return img
        }
        var map: [String: UIImage] = [:]
        for (id, d) in raw.theirs { if let i = show(id, d) { map[id] = i } }
        let mine = show(Self.myPhotoKey, raw.mine)
        let keep = Set(map.keys).union(mine != nil ? [Self.myPhotoKey] : [])
        photoCache = photoCache.filter { keep.contains($0.key) }
        ui.photos = map
        ui.myPhoto = mine
        ui.photoNextChangeMs = raw.wait
    }

    /// Foto escolhida (ate 20 MB): reduzida aqui e mostrada para confirmar. So "Aplicar" a envia.
    func previewProfilePhoto(_ data: Data) {
        guard qt != nil else { return }
        Task {
            do {
                let bytes = try await background { try ProfilePhoto.prepare(data) }
                guard let img = ProfilePhoto.decode(bytes) else { throw ImagePrepError.unreadable }
                pendingPhoto = bytes
                ui.photoPreview = img
            } catch ImagePrepError.tooBig {
                showError(s("t_a_foto_tem_mais_de_20"))
            } catch {
                showError(s("t_nao_foi_possivel_ler_essa_imagem"))
            }
        }
    }

    func applyProfilePhoto() {
        guard let client = qt, let bytes = pendingPhoto else { return }
        let ids = ui.contacts.map(\.contactId)
        Task {
            do {
                let n = try await background { try client.profilePhotoSet(photo: bytes, contactIds: ids) }
                pendingPhoto = nil
                ui.photoPreview = nil
                ui.notice = s("photo_updated")
                if n > 0 { await flushOutbox() }
            } catch QtError.InvalidInput(let field) {
                pendingPhoto = nil
                ui.photoPreview = nil
                showError(field == "photo_wait" ? s("photo_wait") : s("photo_invalid"))
            } catch {
                pendingPhoto = nil
                ui.photoPreview = nil
                showError(s("photo_invalid"))
            }
            await refreshPhotos(client)
        }
    }

    func cancelProfilePhoto() {
        pendingPhoto = nil
        if ui.photoPreview != nil { ui.photoPreview = nil }
    }

    func dismissNotice() { ui.notice = nil }

    func removeProfilePhoto() {
        guard let client = qt else { return }
        let ids = ui.contacts.map(\.contactId)
        Task {
            if let n = try? await background({ try client.profilePhotoRemove(contactIds: ids) }), n > 0 { await flushOutbox() }
            await refreshPhotos(client)
        }
    }

    // ================================================================ contactos

    private func saveContacts(_ list: [Contact]) async throws {
        guard let ck = contactsKey else { throw SecureStoreError(message: "sem a chave dos contactos") }
        try await background { try ContactStore.save(list, key: ck) }
        ui.contacts = list
    }

    /// Guarda o contacto. NAO inicia sessao (evita sessoes cruzadas): so ao enviar a 1.a mensagem.
    func addContact(nickname: String, publicId: String) {
        Task {
            let id = publicId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let relayId = try? relayIdOfPublicId(publicId: id) else { showError(s("t_public_id_invalido")); return }
            let name = nickname.trimmingCharacters(in: .whitespaces)
            let c = Contact(nickname: name.isEmpty ? s("t_contacto") : name, publicId: id, relayId: relayId)
            if c.publicId == ui.publicId { showError(s("t_esse_e_o_teu_proprio_id")); return }
            do {
                try await saveContacts(ui.contacts.filter { $0.contactId != c.contactId } + [c])
            } catch {
                showError(s("t_erro", safeMessage(error))); return
            }
            ui.error = nil
            if let q = qt { await syncPresence(q) }
            await flushOutbox()   // recibos que estavam a espera de saber o endereco deste contacto
        }
    }

    func showSafetyNumber(_ contact: Contact) {
        guard let client = qt else { return }
        Task {
            do {
                let n = try await background { try client.safetyNumber(peerPublicId: contact.publicId) }
                ui.safetyNumber = "\(contact.nickname)\n\n\(n)"
            } catch {
                showError(s("t_nao_foi_possivel_calcular_o_safety", safeMessage(error)))
            }
        }
    }

    func dismissSafetyNumber() { ui.safetyNumber = nil }

    // ================================================================ conversas

    func openChat(_ id: String) {
        guard let client = qt else { return }
        let limit = UInt32(Self.page)
        Task {
            guard let page = try? await background({ try client.messagesPage(contactId: id, beforeSeq: nil, limit: limit) }) else { return }
            ui.openChatId = id
            ui.chatMessages = page
            ui.chatHasMore = page.count == Self.page
            await markRead(client, id)
        }
    }

    func closeChat() {
        ui.openChatId = nil
        ui.chatMessages = []
        ui.chatHasMore = false
    }

    private var loadingOlder = false

    func loadOlder() {
        guard !loadingOlder, let client = qt, let cid = ui.openChatId, ui.chatHasMore,
              let oldest = ui.chatMessages.last?.seq else { return }
        loadingOlder = true
        let limit = UInt32(Self.page)
        Task {
            defer { loadingOlder = false }
            guard let older = try? await background({ try client.messagesPage(contactId: cid, beforeSeq: oldest, limit: limit) }) else { return }
            guard ui.openChatId == cid else { return }
            ui.chatMessages += older
            ui.chatHasMore = older.count == Self.page
        }
    }

    /// ELIMINAR CONVERSA: apaga para sempre as mensagens desta conversa neste iPhone.
    func deleteConversation(_ id: String) {
        guard let client = qt else { return }
        attachmentCache.clear()
        Task {
            do {
                try await background { try client.deleteConversation(contactId: id) }
                if ui.openChatId == id { ui.chatMessages = []; ui.chatHasMore = false }
                await refreshViews(client)
            } catch {
                showError(s("t_nao_foi_possivel_eliminar_a_conversa", safeMessage(error)))
            }
        }
    }

    /// BLOQUEAR: as mensagens deste contacto passam a ser descartadas ao chegar (no Rust).
    func blockContact(_ id: String) {
        guard let client = qt else { return }
        Task {
            do {
                try await background { try client.blockContact(contactId: id) }
                ui.blocked.insert(id)
                await refreshViews(client)
            } catch {
                showError(s("t_nao_foi_possivel_bloquear", safeMessage(error)))
            }
        }
    }

    func unblockContact(_ id: String) {
        guard let client = qt else { return }
        Task {
            do {
                try await background { try client.unblockContact(contactId: id) }
                ui.blocked.remove(id)
            } catch {
                showError(s("t_nao_foi_possivel_desbloquear_2", safeMessage(error)))
            }
        }
    }

    /// ACEITAR CONVITE: o Public ID veio cifrado e verificado no Rust; fica nos contactos.
    func acceptInvite(_ id: String, nickname: String) {
        guard let client = qt else { return }
        Task {
            do {
                guard let pid = try await background({ try client.pendingIntro(contactId: id) }) else {
                    showError(s("t_este_convite_ja_nao_esta_disponivel")); return
                }
                let n = nickname.trimmingCharacters(in: .whitespaces)
                let c = Contact(nickname: n.isEmpty ? s("t_contacto_2", String(id.prefix(4))) : n,
                                publicId: pid, relayId: try relayIdOfPublicId(publicId: pid))
                try await saveContacts(ui.contacts.filter { $0.contactId != c.contactId } + [c])
                try await background { try client.dismissIntro(contactId: id) }
                ui.invites.remove(id)
                await syncPresence(client)
                await refreshViews(client)
                await flushOutbox()
            } catch {
                showError(s("t_nao_foi_possivel_aceitar_o_convite", safeMessage(error)))
            }
        }
    }

    func declineInvite(_ id: String) { deleteContact(id) }

    func acceptTerms() {
        prefs.set(termsVersion, forKey: "terms_version")
        ui.termsAccepted = true
    }

    /// APAGAR CONTACTO: mensagens, envios pendentes, chaves da sessao e o contacto. Avisa o
    /// outro lado ("sessao terminada"), para a proxima mensagem dele abrir uma sessao nova.
    func deleteContact(_ id: String) {
        guard let client = qt else { return }
        attachmentCache.clear()
        let known = ui.contacts.first { $0.contactId == id }?.relayId
        Task {
            do {
                var relayId = known
                if relayId == nil, let pid = try? await background({ try client.pendingIntro(contactId: id) }) {
                    relayId = try? relayIdOfPublicId(publicId: pid)
                }
                let r = relayId
                try await background {
                    if let r { try client.deleteContactNotify(contactId: id, relayId: r) } else { try client.deleteContact(contactId: id) }
                }
                try await saveContacts(ui.contacts.filter { $0.contactId != id })
                if ui.openChatId == id { closeChat() }
                await syncPresence(client)
                await refreshViews(client)
                await flushOutbox()
            } catch {
                showError(s("t_nao_foi_possivel_apagar_o_contacto", safeMessage(error)))
            }
        }
    }

    /// Apaga a conta: primeiro no relay (bundle e mensagens em espera), depois tudo deste iPhone.
    func wipeAllData(localOnly: Bool = false) {
        guard let client = qt else { return }
        ui.wipeServerFailed = nil
        Task {
            if !localOnly, let problem = await deleteOnRelay(client) {
                ui.wipeServerFailed = problem
                return
            }
            do {
                closeLinks()
                try await background { try client.deleteIdentity() }
                await resetStorage()
            } catch {
                showError(s("t_nao_foi_possivel_apagar_os_dados", safeMessage(error)))
            }
        }
    }

    func dismissWipeServerFailed() { ui.wipeServerFailed = nil }

    private func deleteOnRelay(_ client: QtClient) async -> String? {
        guard let l = link else { return s("t_sem_ligacao_ao_servidor") }
        do {
            switch try await l.request(try await background { try client.relayRequestDeleteAccount() }) {
            case .ack: return nil
            case .error(let reason):
                return reason == Self.relayUnknownRequest ? s("t_o_servidor_ainda_nao_tem_esta") : s("t_o_servidor_respondeu", reason)
            default: return s("t_resposta_inesperada_do_servidor")
            }
        } catch {
            return s("t_sem_ligacao_ao_servidor")
        }
    }

    func setReadReceipts(_ on: Bool) {
        prefs.set(on, forKey: "read_receipts")
        ui.readReceipts = on
    }

    private func markRead(_ client: QtClient, _ id: String) async {
        // "Visto" so para contactos: a quem ainda nao aceitei (convite) nao se diz que li.
        let send = ui.readReceipts && (ui.contacts.contains { $0.contactId == id } || id.hasPrefix("g:"))
        if let n = try? await background({ try client.markConversationRead(contactId: id, sendReadReceipt: send) }), n > 0 {
            await flushOutbox()
        }
        await refreshViews(client)
    }

    /// Recarrega a lista de conversas e as mensagens mais recentes da conversa aberta.
    private func refreshViews(_ client: QtClient) async {
        let cid = ui.openChatId
        let n = min(max(ui.chatMessages.count, Self.page), Self.refreshMax)
        struct R: Sendable {
            let window: [ChatMessage]?; let invites: [String]; let groups: [GroupInfo]; let online: [String]; let convs: [ConversationSummary]
        }
        guard let r = try? await background({ () -> R in
            R(window: try cid.map { try client.messagesPage(contactId: $0, beforeSeq: nil, limit: UInt32(n)) },
              invites: try client.pendingIntros(), groups: try client.groups(), online: client.presenceOnline(),
              convs: try client.conversations())
        }) else { return }
        ui.invites = Set(r.invites)
        ui.groups = r.groups
        let on = Set(r.online)
        if on != ui.online { ui.online = on }
        ui.conversations = r.convs
        if let w = r.window, ui.openChatId == cid { ui.chatMessages = mergeWindow(w, ui.chatMessages, n) }
    }

    private func mergeWindow(_ fresh: [ChatMessage], _ current: [ChatMessage], _ n: Int) -> [ChatMessage] {
        guard fresh.count >= n, let oldest = fresh.last?.seq else { return fresh }
        return fresh + current.filter { $0.seq < oldest }
    }

    /// Rele mensagens concretas da conversa aberta (ex.: reacao ou recibo numa mensagem antiga).
    private func refreshMessages(_ client: QtClient, _ conv: String, _ ids: [String]) async {
        guard !ids.isEmpty, ui.openChatId == conv else { return }
        let loaded = Set(ui.chatMessages.map(\.messageId))
        let want = ids.filter { loaded.contains($0) }
        guard !want.isEmpty else { return }
        let updated = await background { () -> [String: ChatMessage] in
            var m: [String: ChatMessage] = [:]
            for id in want { if let x = try? client.message(contactId: conv, messageId: id) { m[id] = x } }
            return m
        }
        guard !updated.isEmpty, ui.openChatId == conv else { return }
        ui.chatMessages = ui.chatMessages.map { updated[$0.messageId] ?? $0 }
    }

    // ================================================================ enviar

    func send(_ contact: Contact, _ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed.utf8.count <= Self.maxTextBytes else {
            showError(s("t_mensagem_vazia_ou_maior_que_bytes", Self.maxTextBytes)); return
        }
        guard let client = qt else { return }
        Task {
            do {
                try await ensureSession(client, contact)
                _ = try await background { try client.sendText(contactId: contact.contactId, text: trimmed) }
                await refreshViews(client)
                await flushOutbox()
            } catch {
                showError(s("t_nao_foi_possivel_enviar_2", safeMessage(error)))
            }
        }
    }

    /// Mensagem de voz (o ficheiro e apagado no fim, cifrado ou nao).
    func sendVoice(to conv: String, file: URL, durationMs: Int64) {
        guard let client = qt else { try? FileManager.default.removeItem(at: file); return }
        Task {
            defer { try? FileManager.default.removeItem(at: file) }
            do {
                try await prepareTarget(client, conv)
                let ms = UInt32(min(max(durationMs, 1), Voice.maxMs))
                _ = try await background {
                    try client.attachmentCreate(contactId: conv, sourcePath: file.path, mime: Voice.mime, width: ms, height: 0)
                }
                await refreshViews(client)
                kickTransfers()
            } catch {
                showError(s("t_nao_foi_possivel_enviar_a_mensagem", safeMessage(error)))
            }
        }
    }

    /// Reagir com um emoji (vazio = retirar a minha reacao). A dois ou num grupo.
    func react(_ conv: String, _ messageId: String, _ emoji: String) {
        guard let client = qt else { return }
        Task {
            do {
                if let gid = groupIdOfConversation(conversationId: conv) {
                    try await ensureSessions(client, try await background { try client.groupMembersNeedingSession(groupId: gid) })
                    try await background { try client.groupReact(groupId: gid, messageId: messageId, emoji: emoji) }
                } else {
                    try await background { try client.react(contactId: conv, messageId: messageId, emoji: emoji) }
                }
                await refreshViews(client)
                await refreshMessages(client, conv, [messageId])
                await flushOutbox()
            } catch {
                showError(s("t_nao_foi_possivel_reagir", safeMessage(error)))
            }
        }
    }

    // ================================================================ grupos

    /// Abre sessao com quem ainda nao a tem. Quem falhar fica para depois.
    private func ensureSessions(_ client: QtClient, _ publicIds: [String]) async {
        guard let l = link else { return }
        for pid in publicIds {
            let cid = contactIdOf(pid)
            if ui.blocked.contains(cid) { continue }
            if (try? await background({ try client.hasSession(contactId: cid) })) == true { continue }
            do {
                let req = try await background { try client.relayRequestFetchBundle(relayId: try relayIdOfPublicId(publicId: pid)) }
                if case .bundle(let bytes) = try await l.request(req) {
                    _ = try await background { try client.startSession(publicId: pid, bundle: try peerBundleFromRelayBytes(bytes: bytes)) }
                }
            } catch {
                log("SealerRelay", "sessao com membro do grupo: \(safeMessage(error))")
            }
        }
    }

    func createGroup(name: String, members: [Contact]) {
        guard let client = qt else { return }
        Task {
            do {
                await ensureSessions(client, members.map(\.publicId))
                let n = name.trimmingCharacters(in: .whitespaces)
                let g = try await background { try client.groupCreate(name: n, memberPublicIds: members.map(\.publicId)) }
                await refreshViews(client)
                await flushOutbox()
                openChat(g.conversationId)
            } catch {
                showError(s("t_nao_foi_possivel_criar_o_grupo", safeMessage(error)))
            }
        }
    }

    func sendGroup(_ groupId: String, _ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed.utf8.count <= Self.maxTextBytes else {
            showError(s("t_mensagem_vazia_ou_maior_que_bytes", Self.maxTextBytes)); return
        }
        guard let client = qt else { return }
        Task {
            do {
                await ensureSessions(client, try await background { try client.groupMembersNeedingSession(groupId: groupId) })
                _ = try await background { try client.groupSendText(groupId: groupId, text: trimmed) }
                await refreshViews(client)
                await flushOutbox()
            } catch {
                showError(s("t_nao_foi_possivel_enviar_2", safeMessage(error)))
            }
        }
    }

    /// MUDAR O GRUPO (so administradores): nome, membros e administradores (ate 3).
    func updateGroup(_ groupId: String, name: String, members: [GroupMemberInput]) {
        guard let client = qt else { return }
        Task {
            do {
                await ensureSessions(client, members.map(\.publicId))
                let n = name.trimmingCharacters(in: .whitespaces)
                _ = try await background { try client.groupUpdate(groupId: groupId, name: n, members: members) }
                await refreshViews(client)
                await flushOutbox()
            } catch {
                showError(s("t_nao_foi_possivel_mudar_o_grupo", safeMessage(error)))
            }
        }
    }

    func leaveGroup(_ groupId: String) {
        guard let client = qt else { return }
        Task {
            do {
                try await background { try client.groupLeave(groupId: groupId) }
                await refreshViews(client)
                await flushOutbox()
            } catch {
                showError(s("t_nao_foi_possivel_sair_do_grupo", safeMessage(error)))
            }
        }
    }

    func deleteGroup(_ groupId: String) {
        guard let client = qt else { return }
        attachmentCache.clear()
        Task {
            do {
                let conv = try await background { try client.group(groupId: groupId)?.conversationId }
                try await background { try client.groupDelete(groupId: groupId) }
                if let conv, ui.openChatId == conv { closeChat() }
                await refreshViews(client)
                await flushOutbox()
            } catch {
                showError(s("t_nao_foi_possivel_apagar_o_grupo", safeMessage(error)))
            }
        }
    }

    // ================================================================ anexos

    /// Imagem/GIF (galeria ou colada) para uma conversa a dois ou de grupo.
    func sendImage(to conv: String, data: Data, original: Bool, pasted: Bool) {
        guard let client = qt else { return }
        Task {
            let prepared: PreparedImage
            do {
                prepared = try await background { try ImagePrep.prepare(data, original: original, keepAsIs: pasted) }
            } catch ImagePrepError.tooBig {
                showError(s("t_a_imagem_tem_mais_de_25")); return
            } catch {
                showError(s("t_nao_foi_possivel_ler_esta_imagem")); return
            }
            defer { try? FileManager.default.removeItem(at: prepared.url) }
            do {
                try await prepareTarget(client, conv)
                _ = try await background {
                    try client.attachmentCreate(contactId: conv, sourcePath: prepared.url.path,
                                                mime: prepared.mime, width: prepared.width, height: prepared.height)
                }
                await refreshViews(client)
                kickTransfers()
            } catch {
                showError(s("t_nao_foi_possivel_enviar_a_imagem", safeMessage(error)))
            }
        }
    }

    /// Ficheiro (PDF, documento...) ate 25 MB, tal como esta.
    func sendFile(to conv: String, url: URL) {
        guard let client = qt else { return }
        Task {
            let prepared: PreparedFile
            // A copia que o seletor de ficheiros pos na pasta temporaria da app ja nao faz falta.
            defer {
                if url.resolvingSymlinksInPath().path.hasPrefix(FileManager.default.temporaryDirectory.resolvingSymlinksInPath().path) {
                    try? FileManager.default.removeItem(at: url)
                }
            }
            do {
                prepared = try await background { try FilePrep.prepare(url) }
            } catch FilePrepError.tooBig {
                showError(s("t_o_ficheiro_tem_mais_de_25_2")); return
            } catch {
                showError(s("t_nao_foi_possivel_ler_este_ficheiro")); return
            }
            defer { try? FileManager.default.removeItem(at: prepared.url) }
            do {
                try await prepareTarget(client, conv)
                _ = try await background {
                    try client.attachmentCreateFile(contactId: conv, sourcePath: prepared.url.path, mime: prepared.mime, fileName: prepared.name)
                }
                await refreshViews(client)
                kickTransfers()
            } catch {
                showError(s("t_nao_foi_possivel_enviar_o_ficheiro", safeMessage(error)))
            }
        }
    }

    private struct PlainError: LocalizedError {
        let errorDescription: String?
        init(_ m: String) { errorDescription = m }
    }

    /// A dois: abre a sessao com o contacto; num grupo: as sessoes que faltam com os membros.
    private func prepareTarget(_ client: QtClient, _ conv: String) async throws {
        if let gid = groupIdOfConversation(conversationId: conv) {
            await ensureSessions(client, try await background { try client.groupMembersNeedingSession(groupId: gid) })
        } else {
            guard let c = ui.contact(conv) else { throw PlainError("contacto desconhecido") }
            try await ensureSession(client, c)
        }
    }

    /// O anexo decifrado, so em memoria (imagem, voz, ou ficheiro para ver/guardar).
    func attachmentBytes(_ m: ChatMessage) async -> Data? {
        if let d = attachmentCache.get(m.contactId, m.messageId) { return d }
        guard let client = qt else { return nil }
        let cid = m.contactId, mid = m.messageId
        guard let d = try? await background({ try client.attachmentRead(contactId: cid, messageId: mid) }) else { return nil }
        // Ficheiros nao ficam em cache: sao vistos ou guardados e depois apagados da memoria.
        if m.attachment?.fileName == nil { attachmentCache.put(cid, mid, d) }
        return d
    }

    private func kickTransfers() { transferKick?.yield() }

    private func setProgress(_ id: String, _ p: Double?) { ui.transfers[id] = p }

    private func startTransfers() {
        transferTask?.cancel()
        let (stream, cont) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        transferKick = cont
        transferTask = Task { [weak self] in
            for await _ in stream {
                guard let self, !Task.isCancelled else { return }
                if await self.runTransfers() {
                    // Falha de rede: tenta de novo daqui a 15 s (ou antes, se houver novidades).
                    Task { [weak self] in
                        try? await Task.sleep(nanoseconds: 15_000_000_000)
                        self?.kickTransfers()
                    }
                }
            }
        }
        cont.yield()
    }

    /// Envia e descarrega os anexos pendentes. Devolve true se algo falhou por rede.
    private func runTransfers() async -> Bool {
        guard let client = qt, let l = fileLink else { return false }
        let ups = (try? await background { try client.attachmentsPendingUpload() }) ?? []
        for r in ups {
            let gid = groupIdOfConversation(conversationId: r.contactId)
            if let gid {
                // Grupo de que ja sai (ou apagado): o anexo nao chega a sair.
                if (try? await background({ try client.group(groupId: gid)?.active })) != true {
                    _ = try? await background { try client.attachmentFailed(contactId: r.contactId, messageId: r.messageId, expired: false) }
                    await refreshViews(client)
                    continue
                }
            } else {
                if ui.blocked.contains(r.contactId) { continue }
                if ui.contact(r.contactId) == nil { continue }
            }
            do { try await upload(client, l, r, gid) } catch { setProgress(r.messageId, nil); return true }
        }
        let downs = (try? await background { try client.attachmentsPendingDownload() }) ?? []
        for r in downs {
            do { try await download(client, l, r) } catch { setProgress(r.messageId, nil); return true }
        }
        return false
    }

    private func upload(_ client: QtClient, _ l: RelayLink, _ r: AttachmentRef, _ gid: String?) async throws {
        let cid = r.contactId, mid = r.messageId
        guard let total = try await background({ try client.message(contactId: cid, messageId: mid)?.attachment?.transferSize }) else { return }
        var start: UInt64 = 0
        if try await background({ try client.attachmentBlobId(contactId: cid, messageId: mid) }) == nil {
            switch try await l.request(try await background { try client.blobCreateRequest(contactId: cid, messageId: mid) }) {
            case .blobCreated(let blobId, let token):
                try await background { try client.attachmentSetBlob(contactId: cid, messageId: mid, blobId: blobId, token: token) }
            case .error(let reason):
                _ = try? await background { try client.attachmentFailed(contactId: cid, messageId: mid, expired: false) }
                await refreshViews(client)
                showError(relayRefusal(reason))
                return
            default:
                throw PlainError("resposta inesperada do relay")
            }
        } else {
            switch try await l.request(try await background { try client.blobInfoRequest(contactId: cid, messageId: mid) }) {
            case .blobStatus(_, let received, _):
                start = received - received % Self.chunk
            case .blobNotFound:
                _ = try? await background { try client.attachmentFailed(contactId: cid, messageId: mid, expired: false) }
                await refreshViews(client)
                return
            default:
                throw PlainError("resposta inesperada do relay")
            }
        }
        var offset = start
        while offset < total {
            let batch = Array(stride(from: offset, to: total, by: Int(Self.chunk)).prefix(Self.window))
            let reqs = try await background { try batch.map { try client.blobPutRequest(contactId: cid, messageId: mid, offset: $0) } }
            let answers = try await l.requestMany(reqs)
            guard answers.allSatisfy({ if case .blobStatus = $0 { return true } else { return false } }) else {
                throw PlainError("bloco recusado")
            }
            offset = (batch.last ?? offset) + Self.chunk
            setProgress(mid, Double(min(offset, total)) / Double(total))
        }
        if let gid {
            await ensureSessions(client, try await background { try client.groupMembersNeedingSession(groupId: gid) })
            _ = try await background { try client.groupAttachmentUploaded(groupId: gid, messageId: mid) }
        } else {
            guard let c = ui.contact(cid) else { return }
            try await ensureSession(client, c)
            _ = try await background { try client.attachmentUploaded(contactId: cid, messageId: mid) }
        }
        setProgress(mid, nil)
        await refreshViews(client)
        await flushOutbox()
    }

    private func download(_ client: QtClient, _ l: RelayLink, _ r: AttachmentRef) async throws {
        let cid = r.contactId, mid = r.messageId
        var have = try await background { try client.attachmentDownloadedBytes(contactId: cid, messageId: mid) }
        var total: UInt64?
        while true {
            if let t = total, have >= t { break }
            let offsets: [UInt64] = total.map { Array(stride(from: have, to: $0, by: Int(Self.chunk)).prefix(Self.window)) } ?? [have]
            let reqs = try await background { try offsets.map { try client.blobGetRequest(contactId: cid, messageId: mid, offset: $0) } }
            for resp in try await l.requestMany(reqs) {
                switch resp {
                case .blobData(let off, let size, let data):
                    total = size
                    have = try await background { try client.attachmentWriteChunk(contactId: cid, messageId: mid, offset: off, data: data) }
                case .blobNotFound:
                    _ = try? await background { try client.attachmentFailed(contactId: cid, messageId: mid, expired: true) }
                    setProgress(mid, nil)
                    await refreshViews(client)
                    return
                default:
                    throw PlainError("resposta inesperada do relay")
                }
            }
            if let t = total, t > 0 { setProgress(mid, Double(have) / Double(t)) }
        }
        let done = try await background { try client.attachmentDownloadFinished(contactId: cid, messageId: mid) }
        setProgress(mid, nil)
        await refreshViews(client)
        // Ja esta no iPhone: o relay pode apagar a copia. Nos grupos nao (os outros ainda descarregam).
        if done.attachment?.state == .ready, groupIdOfConversation(conversationId: cid) == nil,
           let req = try? await background({ try client.blobDeleteRequest(contactId: cid, messageId: mid) }) {
            _ = try? await l.request(req)
        }
    }

    private func relayRefusal(_ reason: String) -> String {
        if reason.contains("invalida") || reason.contains("desativados") { return s("t_o_servidor_ainda_nao_aceita_anexos") }
        if reason.contains("limite") { return s("att_daily_limit") }
        if reason.contains("espaco") { return s("att_no_space") }
        return s("t_nao_foi_possivel_enviar_o_anexo", reason)
    }

    // ================================================================ sessoes e outbox

    /// Um contacto apagou-me e as chaves dele ("sessao terminada"): abre ja uma sessao nova.
    private func restartSessions(_ client: QtClient) async {
        guard let ids = try? await background({ try client.sessionsToRestart() }), !ids.isEmpty else { return }
        var started = false
        for cid in ids {
            guard let c = ui.contact(cid), !ui.blocked.contains(cid) else { continue }
            do { try await ensureSession(client, c); started = true } catch {
                log("SealerRelay", "sessao nova depois de terminada: \(safeMessage(error))")
            }
        }
        if started {
            await refreshViews(client)
            await flushOutbox()
        }
    }

    private func ensureSession(_ client: QtClient, _ c: Contact) async throws {
        if try await background({ try client.hasSession(contactId: c.contactId) }) { return }
        guard let l = link else { throw PlainError(s("t_sem_ligacao_ao_servidor")) }
        let req = try await background { try client.relayRequestFetchBundle(relayId: c.relayId) }
        switch try await l.request(req) {
        case .bundle(let bytes):
            _ = try await background { try client.startSession(publicId: c.publicId, bundle: try peerBundleFromRelayBytes(bytes: bytes)) }
        case .bundleNotFound:
            throw PlainError("o contacto ainda nao publicou as chaves (tem de abrir a app uma vez)")
        case let other:
            throw PlainError("resposta inesperada do relay: \(other.describe)")
        }
    }

    /// Envia ao relay tudo o que esta na outbox, pela ordem. So sai o que o relay confirmar.
    private func flushOutbox() async {
        await flushLock.lock()
        await flushOutboxLocked()
        await flushLock.unlock()
    }

    private func flushOutboxLocked() async {
        guard let client = qt, let l = link else { return }
        guard let items = try? await background({ try client.outbox() }) else { return }
        var members: [String: String]?
        var changed = false
        for item in items {
            var relayId = item.relayId ?? ui.contact(item.contactId)?.relayId
            if relayId == nil {
                if members == nil {
                    members = await background {
                        var m: [String: String] = [:]
                        for g in (try? client.groups()) ?? [] { for x in g.members { m[x.contactId] = x.publicId } }
                        return m
                    }
                }
                if let pid = members?[item.contactId] { relayId = try? relayIdOfPublicId(publicId: pid) }
            }
            guard let rid = relayId else { continue }
            let r: RelayResponse
            do {
                let req = try await background { try client.relayRequestSendItem(toRelayId: rid, item: item) }
                r = try await l.request(req)
            } catch {
                break   // sem rede: tenta no proximo ciclo, mantendo a ordem
            }
            switch r {
            case .ack:
                _ = try? await background { try client.outboxSent(outboxId: item.outboxId) }
                changed = true
            case .error(let reason) where reason == relayRecipientDeletedReason():
                if deletedNotified.insert(item.contactId).inserted {
                    showError(s("t_apagou_a_conta_sealer_as_mensagens", ui.contact(item.contactId)?.nickname ?? s("t_este_contacto")))
                }
            default:
                break   // ex.: fila cheia: fica na outbox
            }
        }
        if changed { await refreshViews(client) }
    }

    // ================================================================ receber

    private func receiveLoop(_ client: QtClient) async {
        var waitBlockedUntil: Double = 0
        var first: [RelayDelivered]?
        while !Task.isCancelled {
            await pollOnce(first: first)
            first = nil
            guard let w = waitLink, monotonicSeconds() >= waitBlockedUntil else {
                try? await Task.sleep(nanoseconds: Self.pollSeconds * 1_000_000_000)
                continue
            }
            do {
                let secs = Self.waitSeconds
                let req = try await background { try client.relayRequestWait(timeoutSecs: secs) }
                switch try await w.request(req, timeout: Self.waitTimeoutSeconds) {
                case .messages(let msgs):
                    first = msgs.isEmpty ? nil : msgs
                case .error(let reason):
                    if reason == Self.relayUnknownRequest {
                        waitBlockedUntil = monotonicSeconds() + Self.waitRetrySeconds
                    } else {
                        try? await Task.sleep(nanoseconds: Self.pollSeconds * 1_000_000_000)
                    }
                    log("SealerRelay", "espera recusada pelo relay: \(reason)")
                default:
                    try? await Task.sleep(nanoseconds: Self.pollSeconds * 1_000_000_000)
                }
            } catch {
                if Task.isCancelled { return }
                log("SealerRelay", "espera falhou: \(safeMessage(error))")
                try? await Task.sleep(nanoseconds: Self.pollSeconds * 1_000_000_000)
            }
        }
    }

    private func pollOnce(first: [RelayDelivered]? = nil) async {
        await receiveLock.lock()
        await pollOnceLocked(first)
        await receiveLock.unlock()
    }

    private func pollOnceLocked(_ first: [RelayDelivered]?) async {
        guard let client = qt, let l = link else { return }
        do {
            if first == nil { await flushOutbox() }
            var received = false
            var pending = first
            for _ in 0..<Self.maxPages {
                let msgs: [RelayDelivered]
                if let p = pending {
                    msgs = p
                    pending = nil
                } else {
                    guard case .messages(let m) = try await l.request(try await background { try client.relayRequestFetch() }) else { break }
                    msgs = m
                }
                if msgs.isEmpty { break }
                struct Page: Sendable { var ack: [UInt64] = []; var got = false; var fresh = false; var status: [String: [String]] = [:] }
                let page = try await background { () -> Page in
                    var p = Page()
                    for m in msgs {
                        do {
                            switch try client.receiveSealed(sealedEnvelope: m.ciphertext) {
                            case .statusChanged(let cid, let ids, _):
                                p.got = true
                                p.status[cid, default: []] += ids
                            case .newMessage: p.got = true; p.fresh = true
                            case .updated: p.got = true
                            case .noChange: break
                            }
                            p.ack.append(m.id)
                        } catch QtError.Storage {
                            // Problema local: NAO confirmar, o relay volta a entregar mais tarde.
                        } catch {
                            p.ack.append(m.id)   // repetida ou invalida
                        }
                    }
                    return p
                }
                if page.got {
                    received = true
                    // Fora do ecra (ainda ligada): o relay nao toca a campainha porque a espera estava aberta.
                    if page.fresh { showGenericNotice() }
                    await refreshViews(client)
                    for (conv, ids) in page.status { await refreshMessages(client, conv, ids) }
                }
                if !page.ack.isEmpty {
                    let ids = page.ack
                    _ = try await l.request(try await background { try client.relayRequestAck(ids: ids) })
                }
            }
            await restartSessions(client)
            await handleCallEvents(client)
            if received {
                await syncPhotos(client)
                kickTransfers()
                _ = try? await background { try client.prepareDeliveryReceipts() }
                if let open = ui.openChatId, visible { await markRead(client, open) } else { await refreshViews(client) }
                await flushOutbox()
            }
            status("Ligado ao relay")
        } catch {
            status("Sem ligacao ao relay: \(safeMessage(error))")
        }
    }

    // ================================================================ chamadas de voz

    private func relayPin() -> Data? {
        let hex = ui.relayKeyHex.trimmingCharacters(in: .whitespaces)
        guard hex.count == 64 else { return nil }
        var out = Data(capacity: 32)
        var i = hex.startIndex
        while i < hex.endIndex {
            let j = hex.index(i, offsetBy: 2)
            guard let b = UInt8(hex[i..<j], radix: 16) else { return nil }
            out.append(b)
            i = j
        }
        return out
    }

    /// Arranca o motor de audio e entra ja na sala do relay das chamadas.
    private func startCallAudio(_ client: QtClient, _ callId: String) async {
        guard callAudio == nil else { return }
        guard let pin = relayPin() else { log("SealerCall", "chamada: sem a chave do relay"); return }
        let url = ui.relayUrl
        do {
            callAudio = try await background { try client.callAudioStart(callId: callId, relayUrl: url, relayPublicKey: pin) }
            log("SealerCall", "motor de audio a correr (\(url))")
        } catch {
            log("SealerCall", "motor de audio nao arrancou: \(safeMessage(error))")
        }
    }

    /// O CallKit ativou o audio: liga o microfone e o altifalante.
    func callAudioSessionActivated() {
        log("SealerCall", "CallKit ativou o audio")
        audioSessionActive = true
        startCallIOIfReady()
    }

    /// Ha casos em que o CallKit nao chega a avisar que ativou o audio (por exemplo, a sessao
    /// de audio ja estava ativa por uma mensagem de voz). Ao fim de 1,5 s ativa-se aqui.
    private func activateAudioFallback() {
        let id = ui.call?.callId
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            guard let self, !self.audioSessionActive, self.callIO == nil, let c = self.ui.call, c.callId == id,
                  c.phase != .ended, c.phase != .incoming else { return }
            log("SealerCall", "CallKit nao ativou o audio: a ativar na app")
            CallKitBridge.configureAudioSession()
            do {
                try AVAudioSession.sharedInstance().setActive(true)
                self.callAudioSessionActivated()
            } catch {
                log("SealerCall", "sessao de audio: \(safeMessage(error))")
            }
        }
    }

    private func startCallIOIfReady() {
        guard callIO == nil, let a = callAudio, let c = ui.call, c.phase != .ended, c.phase != .incoming else { return }
        guard audioSessionActive else { activateAudioFallback(); return }
        let io = CallAudioIO(audio: a)
        do {
            try io.start()
            log("SealerCall", "microfone e altifalante ligados (\(io.inputDescription))")
            io.muted = c.muted
            io.ringback = c.phase == .calling || c.phase == .ringingOut
            if c.speaker { io.setSpeaker(true) }
            callIO = io
        } catch {
            log("SealerCall", "microfone: \(safeMessage(error))")
            showError(s("t_nao_foi_possivel_usar_o_microfone_2", safeMessage(error)))
        }
    }

    func callAudioSessionDeactivated() {
        log("SealerCall", "CallKit desativou o audio")
        audioSessionActive = false
        callIO?.stop()
        callIO = nil
    }

    private func stopCallMedia() {
        audioSessionActive = false
        callIO?.stop()
        callIO = nil
        callAudio?.stop()
        callAudio = nil
        callTicker?.cancel()
        callTicker = nil
    }

    private func ensureCallTicker() {
        guard callTicker == nil else { return }
        callTicker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard let self, self.ui.call != nil || self.pendingRing != nil, let client = self.qt else { break }
                await self.handleCallEvents(client)
                self.expirePendingRing()
                if let st = self.callAudio?.stats() {
                    self.callStatsTick += 1
                    if self.callStatsTick % 5 == 0 {
                        let rtt = st.relayRttMs.map { " · \($0) ms ao servidor" } ?? ""
                        let io = self.callIO.map { " · micro \($0.stats.sent) / altifalante \($0.stats.pulled) blocos" } ?? " · sem microfone/altifalante"
                        log("SealerCall", "chamada: \(st.transport), sala \(st.joined ? "sim" : "nao"), outro lado \(st.peerPresent ? "sim" : "nao")\(rtt) · recebidos \(st.received) · \(st.lost) perdidos (\(st.dredFrames) recuperados) · \(st.late) atrasados\(io)")
                    }
                }
            }
            self?.callTicker = nil
        }
    }

    /// LIGAR a um contacto (o microfone e pedido pelo ecra antes).
    func startCall(_ contact: Contact) {
        guard let client = qt, ui.call == nil, !ui.blocked.contains(contact.contactId) else { return }
        Task {
            do {
                try await ensureSession(client, contact)
                let start = try await background { try client.callStart(contactId: contact.contactId) }
                ui.call = CallUi(callId: start.callId, contactId: contact.contactId, name: contact.nickname, phase: .calling)
                let uuid = UUID()
                callKitIds[start.callId] = uuid
                CallKitBridge.shared.startOutgoing(uuid, name: contact.nickname)
                await flushOutbox()
                await startCallAudio(client, start.callId)
                startCallIOIfReady()
                ensureCallTicker()
            } catch {
                stopCallMedia()
                ui.call = nil
                showError(s("t_nao_foi_possivel_ligar", safeMessage(error)))
            }
        }
    }

    /// ATENDER (no ecra da app ou no CallKit).
    func acceptCall() {
        guard let client = qt, let c = ui.call, c.phase == .incoming else { return }
        Task {
            do {
                _ = try await background { try client.callAccept(callId: c.callId) }
                await flushOutbox()
                await startCallAudio(client, c.callId)
                var a = c
                a.phase = .active
                a.answeredAt = monotonicSeconds()
                ui.call = a
                if let uuid = callKitIds[c.callId] { CallKitBridge.shared.answerFromApp(uuid) }
                startCallIOIfReady()   // se o CallKit ja ativou o audio
                await handleCallEvents(client)
            } catch {
                endCallLocal(s("t_a_chamada_ja_nao_esta_disponivel"))
            }
        }
    }

    /// RECUSAR (a tocar), CANCELAR (a chamar) ou DESLIGAR (atendida).
    func hangupCall() {
        guard let c = ui.call, c.phase != .ended else { return }
        let client = qt
        Task {
            if let client { _ = try? await background { try client.callHangup(callId: c.callId) } }
            await flushOutbox()
        }
        endCallLocal(c.phase == .incoming ? s("t_chamada_recusada") : s("t_chamada_terminada"), fromUser: true)
    }

    func toggleMute() {
        guard var c = ui.call else { return }
        c.muted.toggle()
        callIO?.muted = c.muted
        ui.call = c
        if let uuid = callKitIds[c.callId] { CallKitBridge.shared.setMuted(uuid, c.muted) }
    }

    /// O CallKit mudou o "silenciar" (botao do ecra de chamada do iPhone).
    func setMutedFromSystem(_ muted: Bool) {
        guard var c = ui.call, c.muted != muted else { return }
        c.muted = muted
        callIO?.muted = muted
        ui.call = c
    }

    func toggleSpeaker() {
        guard var c = ui.call else { return }
        c.speaker.toggle()
        callIO?.setSpeaker(c.speaker)
        ui.call = c
    }

    /// A chamada acabou: para o audio e mostra o motivo durante uns segundos.
    private func endCallLocal(_ text: String, fromUser: Bool = false, reason: CXCallEndedReason = .remoteEnded) {
        guard var c = ui.call else { return }
        stopCallMedia()
        if let uuid = callKitIds.removeValue(forKey: c.callId) {
            if fromUser { CallKitBridge.shared.requestEnd(uuid) } else { CallKitBridge.shared.reportEnded(uuid, reason: reason) }
        }
        c.phase = .ended
        c.endText = text
        ui.call = c
        let id = c.callId
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(Self.endedShowSeconds * 1_000_000_000))
            guard let self else { return }
            if let cur = self.ui.call, cur.callId == id, cur.phase == .ended { self.ui.call = nil }
            if self.lockAfterCall, self.ui.call == nil {
                self.lockAfterCall = false
                self.lockNow()
            }
            if !self.visible { self.onAppHiddenAfterCall() }
        }
    }

    /// A chamada acabou com a app fora do ecra: volta ao comportamento normal (fecha as ligacoes).
    private func onAppHiddenAfterCall() {
        guard !visible, ui.call == nil, pendingRing == nil else { return }
        pauseTask?.cancel()
        pauseTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 10_000_000_000)
            self?.pauseLinks()
        }
    }

    private func endText(_ reason: CallEndReason, incoming: Bool) -> String {
        switch reason {
        case .hangup: return s("t_chamada_terminada")
        case .rejected: return s("t_chamada_recusada")
        case .busy: return s("t_ocupado_noutra_chamada")
        case .noAnswer: return incoming ? s("t_chamada_perdida") : s("t_sem_resposta")
        case .cancelled: return incoming ? s("t_chamada_perdida") : s("t_chamada_cancelada")
        case .expired: return s("t_chamada_perdida")
        }
    }

    private func callKitReason(_ reason: CallEndReason, incoming: Bool) -> CXCallEndedReason {
        switch reason {
        case .hangup, .rejected, .busy: return .remoteEnded
        case .noAnswer, .cancelled, .expired: return incoming ? .unanswered : .remoteEnded
        }
    }

    private func answerRequested() -> Bool {
        answerRequestedAt > 0 && monotonicSeconds() - answerRequestedAt < Self.answerValidSeconds
    }

    /// Eventos das chamadas (vem do Rust, depois de receber mensagens e de segundo a segundo).
    private func handleCallEvents(_ client: QtClient) async {
        guard let events = try? await background({ try client.callEvents() }), !events.isEmpty else { return }
        for ev in events {
            switch ev {
            case .incoming(let callId, let contactId):
                // So tocam os contactos guardados (e nao bloqueados). Os outros: recusa em silencio.
                guard let contact = ui.contact(contactId), !ui.blocked.contains(contactId) else {
                    _ = try? await background { () -> Void in
                        if let item = try client.callHangup(callId: callId) { try client.outboxSent(outboxId: item.outboxId) }
                    }
                    continue
                }
                if let shown = ui.call, shown.phase != .ended {
                    _ = try? await background { try client.callHangup(callId: callId) }   // ja ha outra: ocupado
                    continue
                }
                ui.call = CallUi(callId: callId, contactId: contactId, name: contact.nickname, phase: .incoming)
                if let pr = pendingRing {
                    // A chamada que o PushKit ja mostrou no CallKit (sem nome): passa a ter o nome.
                    pendingRing = nil
                    callKitIds[callId] = pr.uuid
                    CallKitBridge.shared.updateName(pr.uuid, name: ui.lockEnabled ? s("t_chamada_sealer") : contact.nickname)
                } else {
                    let uuid = UUID()
                    callKitIds[callId] = uuid
                    CallKitBridge.shared.reportIncoming(uuid, name: contact.nickname) { [weak self] ok in
                        if !ok { Task { @MainActor in self?.hangupCall() } }
                    }
                }
                await startCallAudio(client, callId)
                ensureCallTicker()
                if answerRequested() {
                    answerRequestedAt = 0
                    acceptCall()
                }
            case .ringing(let callId):
                if var c = ui.call, c.callId == callId, c.phase == .calling {
                    c.phase = .ringingOut
                    ui.call = c
                    callIO?.ringback = true
                }
            case .answered(let callId):
                if var c = ui.call, c.callId == callId, c.phase != .active, c.phase != .incoming {
                    c.phase = .active
                    c.answeredAt = monotonicSeconds()
                    ui.call = c
                    callIO?.ringback = false
                    if let uuid = callKitIds[callId] { CallKitBridge.shared.reportConnected(uuid) }
                }
            case .ended(let callId, _, let reason):
                if let c = ui.call, c.callId == callId, c.phase != .ended {
                    let incoming = c.phase == .incoming
                    endCallLocal(endText(reason, incoming: incoming), reason: callKitReason(reason, incoming: incoming))
                }
            }
        }
        await flushOutbox()
    }

    // ---------------------------------------------------------------- PushKit / CallKit

    /// Push VoIP (uma chamada a chegar com a app fechada ou em segundo plano). O CallKit ja
    /// mostra "Chamada SEALER" (o iOS obriga); aqui descobre-se de quem e e, se ninguem estiver
    /// a ligar (chamada ja acabada), termina-se.
    func onVoipPush(_ uuid: UUID) {
        if let c = ui.call, c.phase != .ended {
            CallKitBridge.shared.reportEnded(uuid, reason: .failed)   // ocupado: o Rust responde ao outro
            return
        }
        pendingRing = (uuid, monotonicSeconds())
        ensureCallTicker()
        if ui.locked || ui.loading { return }   // bloqueio da app: o codigo e pedido ao atender
        Task {
            if link == nil {
                let publish = publishOnResume
                publishOnResume = false
                linksPaused = false
                await startLink(publishBundle: publish)
            }
            await pollOnce()
        }
    }

    /// O PushKit acordou a app mas a chamada nao apareceu (ou passou o tempo de tocar).
    private func expirePendingRing() {
        guard let pr = pendingRing else { return }
        let age = monotonicSeconds() - pr.at
        // Com a base aberta, a oferta chega em segundos; com o bloqueio da app, toca ate ao fim.
        let limit = (ui.locked || ui.loading) ? Self.ringSeconds : 20
        if age > limit {
            pendingRing = nil
            answerRequestedAt = 0
            CallKitBridge.shared.reportEnded(pr.uuid, reason: .unanswered)
            onAppHiddenAfterCall()
        }
    }

    /// "Atender" no ecra do iPhone (CallKit).
    func answerFromSystem(_ uuid: UUID) {
        if let callId = callKitIds.first(where: { $0.value == uuid })?.key, ui.call?.callId == callId {
            if ui.call?.phase == .incoming { acceptCall() }
            return
        }
        // Ainda nao se sabe qual e (bloqueio da app, ou a oferta a chegar): atende quando aparecer.
        answerRequestedAt = monotonicSeconds()
    }

    /// "Desligar" / "Recusar" no ecra do iPhone (CallKit).
    func endFromSystem(_ uuid: UUID) {
        if let pr = pendingRing, pr.uuid == uuid {
            pendingRing = nil
            answerRequestedAt = 0
            onAppHiddenAfterCall()
            return
        }
        guard let callId = callKitIds.first(where: { $0.value == uuid })?.key else { return }
        callKitIds.removeValue(forKey: callId)
        if ui.call?.callId == callId { hangupCall() }
    }

    // ================================================================ utilitarios

    func dismissError() { ui.error = nil }
    private func status(_ s: String) {
        if ui.status != s {
            ui.status = s
            log("SealerRelay", s)
        }
    }
    private func showError(_ m: String) { ui.error = m }
}

/// Anexos decifrados em memoria (imagens e voz), ate ~24 MB; os mais antigos saem primeiro.
final class AttachmentCache: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [(key: String, data: Data)] = []
    private var total = 0
    private let max = 24 * 1024 * 1024

    func get(_ cid: String, _ mid: String) -> Data? {
        lock.lock(); defer { lock.unlock() }
        let k = cid + "/" + mid
        guard let i = items.firstIndex(where: { $0.key == k }) else { return nil }
        let it = items.remove(at: i)
        items.append(it)
        return it.data
    }

    func put(_ cid: String, _ mid: String, _ d: Data) {
        guard d.count <= max / 2 else { return }
        lock.lock(); defer { lock.unlock() }
        let k = cid + "/" + mid
        if let i = items.firstIndex(where: { $0.key == k }) { total -= items[i].data.count; items.remove(at: i) }
        items.append((k, d))
        total += d.count
        while total > max, !items.isEmpty { total -= items.removeFirst().data.count }
    }

    func clear() {
        lock.lock(); defer { lock.unlock() }
        items.removeAll()
        total = 0
    }
}
