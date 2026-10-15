import Foundation
import SwiftUI

struct UiState {
    var loading = true
    var hasIdentity = false
    var publicId = ""
    var relayUrl = Config.relayURL
    var relayKeyHex = Config.relayPin
    var status = ""
    var error: String?
    var contacts: [Contact] = []
    /// Conversas com historico (vem do SQLCipher, no Rust), a mais recente primeiro.
    var conversations: [ConversationSummary] = []
    /// Conversa aberta (contactId) e a janela de mensagens carregada, da MAIS RECENTE para a mais antiga.
    var openChatId: String?
    var chatMessages: [ChatMessage] = []
    /// Ainda ha mensagens mais antigas por carregar na conversa aberta.
    var chatHasMore = false
    /// Enviar recibos "Visto" (o utilizador pode desligar).
    var readReceipts = true
    /// Safety number a mostrar (nome + 60 digitos), ou nil.
    var safetyNumber: String?
}

/// Mesma logica que o MainViewModel.kt do Android. O estado vive na thread principal;
/// tudo o que toca no SQLCipher/criptografia corre em `background { }`.
@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var ui = UiState()

    private let prefs = UserDefaults.standard   // so URL do relay e chave PUBLICA (nao secretos)
    private var qt: QtClient?
    private var link: RelayLink?
    private var poller: Task<Void, Never>?
    /// Um so envio da outbox de cada vez.
    private let flushLock = AsyncLock()

    private static let pollSeconds: UInt64 = 3
    private static let maxPages = 20
    private static let maxTextBytes = 9_000
    private static let page = 50

    init() {
        Task { await start() }
    }

    private func start() async {
        do {
            let dbFile = try AppFiles.url("qt.db")
            let client: QtClient = try await background {
                var key = try StorageKeyProvider.storageKey(dbFile: dbFile)
                defer { key.resetBytes(in: 0..<key.count) }
                return try QtClient.open(path: dbFile.path, storageKey: key)
            }
            qt = client
            // Release: relay fixo da build, ignora preferencias antigas. Debug: editavel.
            let url = Config.relayEditable ? (prefs.string(forKey: "relay_url") ?? Config.relayURL) : Config.relayURL
            let pin = Config.relayEditable ? (prefs.string(forKey: "relay_key") ?? Config.relayPin) : Config.relayPin
            let (has, publicId, convs) = try await background { () -> (Bool, String, [ConversationSummary]) in
                let has = try client.hasIdentity()
                var publicId = ""
                if has { publicId = try client.exportPublicIdentity().publicId }
                return (has, publicId, try client.conversations())
            }
            let contacts = (try? await background { ContactStore.load() }) ?? []
            ui.loading = false
            ui.hasIdentity = has
            ui.relayUrl = url
            ui.relayKeyHex = pin
            ui.publicId = publicId
            ui.contacts = contacts
            ui.conversations = convs
            ui.readReceipts = prefs.object(forKey: "read_receipts") as? Bool ?? true
            if has { await startLink() }
        } catch let e as SecureStoreError {
            // Nunca se cria uma chave nova por cima: o utilizador decide se apaga os dados locais.
            ui.loading = false
            ui.error = "Chave do armazenamento seguro indisponivel: \(e.message)"
        } catch {
            ui.loading = false
            ui.error = "Nao foi possivel abrir o armazenamento seguro: \(safeMessage(error))"
        }
    }

    // ---------------------------------------------------------------- configuracao

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
                showError("Nao foi possivel criar a identidade: \(safeMessage(error))")
            }
        }
    }

    func saveRelay(relayUrl: String, relayKeyHex: String) {
        guard Config.relayEditable else { return }   // release: relay fixo, nunca alteravel
        guard applyRelay(relayUrl, relayKeyHex) else { return }
        Task {
            guard let client = qt, (try? await background({ try client.hasIdentity() })) == true else { return }
            await startLink()
        }
    }

    private func applyRelay(_ relayUrl: String, _ relayKeyHex: String) -> Bool {
        let u = relayUrl.trimmingCharacters(in: .whitespaces)
        let lower = u.lowercased()
        guard lower.hasPrefix("wss://") || (Config.allowCleartext && lower.hasPrefix("ws://")) else {
            showError("O relay tem de usar wss:// (ws:// so na build debug)")
            return false
        }
        let k = relayKeyHex.trimmingCharacters(in: .whitespaces)
        prefs.set(u, forKey: "relay_url")
        prefs.set(k, forKey: "relay_key")
        ui.relayUrl = u
        ui.relayKeyHex = k
        return true
    }

    private func startLink() async {
        guard let client = qt else { return }
        poller?.cancel()
        if let old = link { await old.close() }
        let l = RelayLink(qt: client, url: ui.relayUrl, pinnedRelayKeyHex: ui.relayKeyHex.isEmpty ? nil : ui.relayKeyHex)
        link = l
        status("A ligar ao relay…")
        do {
            // Publicar (ou re-publicar) o bundle publico para que outros nos possam contactar.
            let req = try await background { try client.relayRequestRegisterBundle(bundleBytes: try client.publishableBundle()) }
            let r = try await l.request(req)
            if case .ack = r { status("Ligado ao relay") } else { status("Relay recusou o bundle: \(r.describe)") }
        } catch {
            status("Sem ligacao ao relay: \(safeMessage(error))")
        }
        poller = Task { [weak self] in
            while !Task.isCancelled {
                await self?.pollOnce()
                try? await Task.sleep(nanoseconds: AppModel.pollSeconds * 1_000_000_000)
            }
        }
    }

    /// A app voltou ao primeiro plano: verifica ja se ha mensagens (sem esperar pelo proximo ciclo).
    func becameActive() {
        Task { await pollOnce() }
    }

    // ------------------------------------------------------------------- contactos

    /// Guarda o contacto. NAO inicia sessao (evita sessoes cruzadas): so ao enviar a 1.a mensagem.
    func addContact(nickname: String, publicId: String) {
        Task {
            let id = publicId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let relayId = try? relayIdOfPublicId(publicId: id) else {   // valida o formato no Rust
                showError("Public ID invalido"); return
            }
            let name = nickname.trimmingCharacters(in: .whitespaces)
            let c = Contact(nickname: name.isEmpty ? "Contacto" : name, publicId: id, relayId: relayId)
            if c.publicId == ui.publicId { showError("Esse e o teu proprio ID"); return }
            let list = ui.contacts.filter { $0.contactId != c.contactId } + [c]
            do {
                try await background { try ContactStore.save(list) }
            } catch {
                showError("Nao foi possivel guardar o contacto: \(safeMessage(error))"); return
            }
            ui.contacts = list
            ui.error = nil
            await flushOutbox()   // recibos que estavam a espera de saber o endereco deste contacto
        }
    }

    // ---------------------------------------------------------- safety numbers

    /// Mostra o safety number com este contacto (calculado no Rust a partir dos dois Public IDs).
    func showSafetyNumber(_ contact: Contact) {
        Task {
            guard let client = qt else { return }
            do {
                let n = try await background { try client.safetyNumber(peerPublicId: contact.publicId) }
                ui.safetyNumber = "\(contact.nickname)\n\n\(n)"
            } catch {
                showError("Nao foi possivel calcular o safety number: \(safeMessage(error))")
            }
        }
    }

    func dismissSafetyNumber() { ui.safetyNumber = nil }

    // ------------------------------------------------------------------ conversas

    /// Abre uma conversa: carrega as mensagens mais recentes e marca-as como lidas.
    func openChat(_ contactId: String) {
        ui.openChatId = contactId
        ui.chatMessages = []
        ui.chatHasMore = false
        let limit = UInt32(Self.page)
        Task {
            guard let client = qt else { return }
            guard let page = try? await background({ try client.messagesPage(contactId: contactId, beforeSeq: nil, limit: limit) }) else { return }
            guard ui.openChatId == contactId else { return }
            ui.chatMessages = page
            ui.chatHasMore = page.count == Self.page
            await markRead(client, contactId)
        }
    }

    func closeChat() {
        ui.openChatId = nil
        ui.chatMessages = []
        ui.chatHasMore = false
    }

    private var loadingOlder = false

    /// Scroll ate ao topo: carrega mais uma pagina de mensagens antigas (do disco, cifrado).
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

    func setReadReceipts(_ on: Bool) {
        prefs.set(on, forKey: "read_receipts")
        ui.readReceipts = on
    }

    private func markRead(_ client: QtClient, _ contactId: String) async {
        let send = ui.readReceipts
        if let n = try? await background({ try client.markConversationRead(contactId: contactId, sendReadReceipt: send) }), n > 0 {
            await flushOutbox()
        }
        await refreshViews(client)
    }

    /// Recarrega a lista de conversas e a janela da conversa aberta (estados ✓/✓✓ e mensagens novas).
    private func refreshViews(_ client: QtClient) async {
        let cid = ui.openChatId
        let size = UInt32(max(Self.page, ui.chatMessages.count))
        guard let res = try? await background({ () -> ([ConversationSummary], [ChatMessage]?) in
            let w = try cid.map { try client.messagesPage(contactId: $0, beforeSeq: nil, limit: size) }
            return (try client.conversations(), w)
        }) else { return }
        ui.conversations = res.0
        if let window = res.1, ui.openChatId == cid { ui.chatMessages = window }
    }

    // ---------------------------------------------------------------------- enviar

    func send(_ contact: Contact, _ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let size = trimmed.utf8.count
        guard size > 0, size <= Self.maxTextBytes else {
            showError("Mensagem vazia ou maior que \(Self.maxTextBytes) bytes"); return
        }
        Task {
            guard let client = qt else { return }
            do {
                try await ensureSession(client, contact)
                // Numa so transacao no Rust: ratchet avanca + mensagem guardada + envelope na outbox.
                _ = try await background { try client.sendText(contactId: contact.contactId, text: trimmed) }
                await refreshViews(client)
                await flushOutbox()
            } catch {
                showError("Nao foi possivel enviar: \(safeMessage(error))")
            }
        }
    }

    private struct SessionError: LocalizedError {
        let errorDescription: String?
        init(_ m: String) { errorDescription = m }
    }

    private func ensureSession(_ client: QtClient, _ c: Contact) async throws {
        if try await background({ try client.hasSession(contactId: c.contactId) }) { return }
        guard let l = link else { throw SessionError("sem ligacao ao relay") }
        let req = try await background { try client.relayRequestFetchBundle(relayId: c.relayId) }
        switch try await l.request(req) {
        case .bundle(let bytes):
            _ = try await background { try client.startSession(publicId: c.publicId, bundle: try peerBundleFromRelayBytes(bytes: bytes)) }
        case .bundleNotFound:
            throw SessionError("o contacto ainda nao publicou as chaves (tem de abrir a app uma vez)")
        case let other:
            throw SessionError("resposta inesperada do relay: \(other.describe)")
        }
    }

    /// Envia ao relay tudo o que esta na outbox (mensagens e recibos), pela ordem
    /// de criacao. So sai da outbox o que o relay confirmar; o resto fica guardado
    /// (cifrado) e volta a ser tentado no proximo ciclo - mesmo depois de reiniciar a app.
    private func flushOutbox() async {
        await flushLock.lock()
        await flushOutboxLocked()
        await flushLock.unlock()
    }

    private func flushOutboxLocked() async {
        guard let client = qt, let l = link else { return }
        guard let items = try? await background({ try client.outbox() }) else { return }
        var changed = false
        for item in items {
            // O relay identifica caixas pela chave de assinatura (do Public ID do contacto).
            guard let relayId = ui.contacts.first(where: { $0.contactId == item.contactId })?.relayId else { continue }
            let r: RelayResponse
            do {
                let req = try await background { try client.relayRequestSend(toRelayId: relayId, blob: item.blob) }
                r = try await l.request(req)
            } catch {
                break   // sem rede: tenta no proximo ciclo, mantendo a ordem
            }
            if case .ack = r {
                _ = try? await background { try client.outboxSent(outboxId: item.outboxId) }
                changed = true
            }
            // Outra resposta (ex.: fila do destinatario cheia): fica na outbox e segue para o proximo.
        }
        if changed { await refreshViews(client) }
    }

    // --------------------------------------------------------------------- receber

    private func pollOnce() async {
        guard let client = qt, let l = link else { return }
        do {
            await flushOutbox()
            var received = false
            for _ in 0..<Self.maxPages {
                let r = try await l.request(try await background { try client.relayRequestFetch() })
                guard case .messages(let msgs) = r, !msgs.isEmpty else { break }
                // Decifrar + guardar no historico + agendar o recibo, tudo numa transacao (no Rust).
                let (ackIds, got) = try await background { () -> ([UInt64], Bool) in
                    var ids: [UInt64] = []
                    var got = false
                    for m in msgs {
                        do {
                            switch try client.receiveSealed(sealedEnvelope: m.ciphertext) {
                            case .newMessage, .statusChanged: got = true
                            case .noChange: break
                            }
                            ids.append(m.id)
                        } catch QtError.Storage {
                            // Problema local: NAO confirmar, o relay volta a entregar mais tarde.
                        } catch {
                            // Replay (ja processada) ou envelope invalido: nada a recuperar -> confirmar.
                            ids.append(m.id)
                        }
                    }
                    return (ids, got)
                }
                if got { received = true }
                if !ackIds.isEmpty {   // so DEPOIS de guardar
                    _ = try await l.request(try await background { try client.relayRequestAck(ids: ackIds) })
                }
            }
            if received {
                _ = try? await background { try client.prepareDeliveryReceipts() }
                if let open = ui.openChatId { await markRead(client, open) } else { await refreshViews(client) }
                await flushOutbox()
            }
            status("Ligado ao relay")
        } catch {
            status("Sem ligacao ao relay: \(safeMessage(error))")
        }
    }

    // ------------------------------------------------------------------ utilitarios

    func dismissError() { ui.error = nil }
    private func status(_ s: String) { if ui.status != s { ui.status = s } }
    private func showError(_ s: String) { ui.error = s }
}
