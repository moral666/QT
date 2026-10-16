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
    /// Progresso (0 a 1) dos anexos a enviar/descarregar, por message_id.
    var transfers: [String: Double] = [:]
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
    /// Segunda ligacao ao relay, so para anexos: um envio grande nao atrasa as mensagens.
    private var fileLink: RelayLink?
    /// Terceira ligacao, so para ESPERAR por mensagens (relayRequestWait): o relay responde no
    /// instante em que chega uma, em vez de a app perguntar de `pollSeconds` em `pollSeconds`.
    /// Fica ocupada durante a espera, por isso nao partilha a ligacao dos envios.
    private var waitLink: RelayLink?
    private var transferTask: Task<Void, Never>?
    private var transferKick: AsyncStream<Void>.Continuation?
    /// Um so envio da outbox de cada vez.
    private let flushLock = AsyncLock()

    /// Sem entrega imediata (relay antigo ou sem rede): perguntar de 3 em 3 s.
    private static let pollSeconds: UInt64 = 3
    /// Espera maxima de cada pedido de espera (o relay limita a 25 s).
    private static let waitSeconds: UInt32 = 25
    /// Tempo pela resposta a um pedido de espera: a espera mais margem para a rede.
    private static let waitTimeoutSeconds: Double = 35
    /// Resposta de um relay a um pedido que nao conhece (relay sem entrega imediata).
    private static let relayUnknownRequest = "mensagem invalida"
    private static let maxPages = 20
    private static let maxTextBytes = 9_000
    private static let page = 50
    /// Bytes cifrados por bloco de anexo (igual ao Rust e ao relay).
    private static let chunk: UInt64 = 40 * 1024
    /// Blocos em transito ao mesmo tempo (~320 KB).
    private static let window = 8

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
        if let old = fileLink { await old.close() }
        fileLink = RelayLink(qt: client, url: ui.relayUrl, pinnedRelayKeyHex: ui.relayKeyHex.isEmpty ? nil : ui.relayKeyHex)
        if let old = waitLink { await old.close() }
        waitLink = RelayLink(qt: client, url: ui.relayUrl, pinnedRelayKeyHex: ui.relayKeyHex.isEmpty ? nil : ui.relayKeyHex)
        status("A ligar ao relay…")
        do {
            // Publicar (ou re-publicar) o bundle publico para que outros nos possam contactar.
            let req = try await background { try client.relayRequestRegisterBundle(bundleBytes: try client.publishableBundle()) }
            let r = try await l.request(req)
            if case .ack = r { status("Ligado ao relay") } else { status("Relay recusou o bundle: \(r.describe)") }
        } catch {
            status("Sem ligacao ao relay: \(safeMessage(error))")
        }
        poller = Task { [weak self] in await self?.receiveLoop() }
        startTransfers()
    }

    /// A app voltou ao primeiro plano: verifica ja se ha mensagens (sem esperar pelo proximo ciclo).
    func becameActive() {
        Task {
            // Em segundo plano o iOS corta as ligacoes: a espera pendente estaria num socket morto.
            // Fechar faz essa espera falhar ja e ser repetida numa ligacao nova.
            if let w = waitLink { await w.close() }
            await pollOnce()
        }
        kickTransfers()   // envios/descargas que o iOS interrompeu em segundo plano
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

    // ------------------------------------------------- eliminacao (permanente)

    /// ELIMINAR CONVERSA: apaga para sempre as mensagens desta conversa neste
    /// aparelho. O contacto e a sessao cifrada mantem-se.
    func deleteConversation(_ contactId: String) {
        Task {
            guard let client = qt else { return }
            do {
                try await background { try client.deleteConversation(contactId: contactId) }
                if ui.openChatId == contactId { ui.chatMessages = []; ui.chatHasMore = false }
                await refreshViews(client)
            } catch {
                showError("Nao foi possivel eliminar a conversa: \(safeMessage(error))")
            }
        }
    }

    /// APAGAR CONTACTO: mensagens, envios pendentes, chaves da sessao e o contacto.
    func deleteContact(_ contactId: String) {
        Task {
            guard let client = qt else { return }
            let list = ui.contacts.filter { $0.contactId != contactId }
            do {
                try await background { try client.deleteContact(contactId: contactId) }
                try await background { try ContactStore.save(list) }
                ui.contacts = list
                if ui.openChatId == contactId { closeChat() }
                await refreshViews(client)
            } catch {
                showError("Nao foi possivel apagar o contacto: \(safeMessage(error))")
            }
        }
    }

    /// APAGAR TODOS OS DADOS deste aparelho: identidade, chaves, contactos, conversas, definicoes.
    func wipeAllData() {
        Task {
            guard let client = qt else { return }
            poller?.cancel()
            transferTask?.cancel()
            if let l = link { await l.close() }
            link = nil
            if let f = fileLink { await f.close() }
            fileLink = nil
            if let w = waitLink { await w.close() }
            waitLink = nil
            do {
                try await background { try client.deleteIdentity() }   // tambem os ficheiros dos anexos
                try? await background { try ContactStore.wipe() }
                for k in ["relay_url", "relay_key", "read_receipts"] { prefs.removeObject(forKey: k) }
                ui = UiState()
                ui.loading = false
                ui.hasIdentity = false
            } catch {
                showError("Nao foi possivel apagar os dados: \(safeMessage(error))")
            }
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


    // ---------------------------------------------------------------- imagens/GIFs

    /// Envia uma imagem/GIF (da galeria ou colada). Preparada no iPhone, cifrada pelo
    /// Rust e enviada em segundo plano (ver `runTransfers`).
    func sendImage(_ contact: Contact, data: Data, original: Bool, pasted: Bool) {
        Task {
            guard let client = qt else { return }
            let prepared: PreparedImage
            do {
                prepared = try await background { try ImagePrep.prepare(data, original: original, keepAsIs: pasted) }
            } catch ImagePrepError.tooBig {
                showError("A imagem tem mais de 25 MB."); return
            } catch {
                showError("Não foi possível ler esta imagem."); return
            }
            defer { try? FileManager.default.removeItem(at: prepared.url) }
            do {
                try await ensureSession(client, contact)
                _ = try await background {
                    try client.attachmentCreate(contactId: contact.contactId, sourcePath: prepared.url.path,
                                                mime: prepared.mime, width: prepared.width, height: prepared.height)
                }
                await refreshViews(client)
                kickTransfers()
            } catch {
                showError("Não foi possível enviar a imagem: \(safeMessage(error))")
            }
        }
    }

    /// A imagem decifrada, so em memoria, para mostrar no ecra.
    func attachmentBytes(_ m: ChatMessage) async -> Data? {
        guard let client = qt else { return nil }
        let cid = m.contactId, mid = m.messageId
        return try? await background { try client.attachmentRead(contactId: cid, messageId: mid) }
    }

    private func kickTransfers() { transferKick?.yield() }

    private func setProgress(_ messageId: String, _ p: Double?) {
        ui.transfers[messageId] = p
    }

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

    /// Envia e descarrega os anexos pendentes (tambem os que ficaram a meio antes de a app
    /// fechar). Devolve true se algo falhou por rede e deve ser tentado de novo.
    private func runTransfers() async -> Bool {
        guard let client = qt, let l = fileLink else { return false }
        let ups = (try? await background { try client.attachmentsPendingUpload() }) ?? []
        for r in ups {
            guard let contact = ui.contacts.first(where: { $0.contactId == r.contactId }) else { continue }
            do { try await upload(client, l, r, contact) } catch { setProgress(r.messageId, nil); return true }
        }
        let downs = (try? await background { try client.attachmentsPendingDownload() }) ?? []
        for r in downs {
            do { try await download(client, l, r) } catch { setProgress(r.messageId, nil); return true }
        }
        return false
    }

    private func upload(_ client: QtClient, _ l: RelayLink, _ r: AttachmentRef, _ contact: Contact) async throws {
        let cid = r.contactId, mid = r.messageId
        guard let total = try await background({ try client.message(contactId: cid, messageId: mid)?.attachment?.transferSize }) else { return }
        var start: UInt64 = 0
        if try await background({ try client.attachmentBlobId(contactId: cid, messageId: mid) }) == nil {
            let req = try await background { try client.blobCreateRequest(contactId: cid, messageId: mid) }
            switch try await l.request(req) {
            case .blobCreated(let blobId, let token):
                try await background { try client.attachmentSetBlob(contactId: cid, messageId: mid, blobId: blobId, token: token) }
            case .error(let reason):
                _ = try? await background { try client.attachmentFailed(contactId: cid, messageId: mid, expired: false) }
                await refreshViews(client)
                showError(Self.relayRefusal(reason))
                return
            default:
                throw SessionError("resposta inesperada do relay")
            }
        } else {
            let req = try await background { try client.blobInfoRequest(contactId: cid, messageId: mid) }
            switch try await l.request(req) {
            case .blobStatus(_, let received, _):
                start = received - received % Self.chunk
            case .blobNotFound:
                // Passaram mais de 24 h sem conseguir acabar o envio.
                _ = try? await background { try client.attachmentFailed(contactId: cid, messageId: mid, expired: false) }
                await refreshViews(client)
                return
            default:
                throw SessionError("resposta inesperada do relay")
            }
        }
        var offset = start
        while offset < total {
            let batch = Array(stride(from: offset, to: total, by: Int(Self.chunk)).prefix(Self.window))
            let reqs = try await background { try batch.map { try client.blobPutRequest(contactId: cid, messageId: mid, offset: $0) } }
            let answers = try await l.requestMany(reqs)
            guard answers.allSatisfy({ if case .blobStatus = $0 { return true } else { return false } }) else {
                throw SessionError("bloco recusado")
            }
            offset = (batch.last ?? offset) + Self.chunk
            setProgress(mid, Double(min(offset, total)) / Double(total))
        }
        try await ensureSession(client, contact)
        _ = try await background { try client.attachmentUploaded(contactId: cid, messageId: mid) }   // mensagem com a chave na outbox
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
            let offsets: [UInt64]
            if let t = total {
                offsets = Array(stride(from: have, to: t, by: Int(Self.chunk)).prefix(Self.window))
            } else {
                offsets = [have]
            }
            let reqs = try await background { try offsets.map { try client.blobGetRequest(contactId: cid, messageId: mid, offset: $0) } }
            for resp in try await l.requestMany(reqs) {
                switch resp {
                case .blobData(let off, let size, let data):
                    total = size
                    have = try await background { try client.attachmentWriteChunk(contactId: cid, messageId: mid, offset: off, data: data) }
                case .blobNotFound:
                    _ = try? await background { try client.attachmentFailed(contactId: cid, messageId: mid, expired: true) }   // expirou (24 h)
                    setProgress(mid, nil)
                    await refreshViews(client)
                    return
                default:
                    throw SessionError("resposta inesperada do relay")
                }
            }
            if let t = total, t > 0 { setProgress(mid, Double(have) / Double(t)) }
        }
        let done = try await background { try client.attachmentDownloadFinished(contactId: cid, messageId: mid) }
        setProgress(mid, nil)
        await refreshViews(client)
        // Ja esta no iPhone (e verificado): o relay pode apagar a copia dele.
        if done.attachment?.state == .ready {
            if let req = try? await background({ try client.blobDeleteRequest(contactId: cid, messageId: mid) }) {
                _ = try? await l.request(req)
            }
        }
    }

    private static func relayRefusal(_ reason: String) -> String {
        if reason.contains("invalida") || reason.contains("desativados") {
            return "O servidor ainda não aceita imagens (tem de ser atualizado)."
        }
        if reason.contains("limite") { return "Atingiste o limite diário de imagens. Tenta amanhã." }
        if reason.contains("espaco") { return "O servidor está sem espaço para imagens. Tenta mais tarde." }
        return "Não foi possível enviar a imagem: \(reason)"
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

    /// Recebe mensagens enquanto a sessao estiver aberta.
    ///
    /// Com um relay que suporta entrega imediata, a app fica a espera numa ligacao propria
    /// (`waitLink`) e o relay responde no instante em que chega uma mensagem (ou ao fim de
    /// `waitSeconds`, e volta-se a esperar). Um relay antigo responde "mensagem invalida" ao pedido
    /// de espera: entao volta-se ao metodo antigo, perguntar de `pollSeconds` em `pollSeconds`.
    private func receiveLoop() async {
        var waitSupported = true
        var first: [RelayDelivered]?
        while !Task.isCancelled {
            await pollOnce(first: first)
            first = nil
            guard waitSupported, let client = qt, let w = waitLink else {
                try? await Task.sleep(nanoseconds: Self.pollSeconds * 1_000_000_000)
                continue
            }
            do {
                let secs = Self.waitSeconds   // copia: o bloco abaixo corre fora da thread principal
                let req = try await background { try client.relayRequestWait(timeoutSecs: secs) }
                switch try await w.request(req, timeout: Self.waitTimeoutSeconds) {
                case .messages(let msgs):
                    first = msgs.isEmpty ? nil : msgs
                case .error(let reason):
                    // Relay antigo nao conhece o pedido: volta ao metodo antigo.
                    // Outro erro (ex.: problema passageiro no relay): espera e tenta de novo.
                    if reason == Self.relayUnknownRequest {
                        waitSupported = false
                    } else {
                        try? await Task.sleep(nanoseconds: Self.pollSeconds * 1_000_000_000)
                    }
                default:
                    try? await Task.sleep(nanoseconds: Self.pollSeconds * 1_000_000_000)
                }
            } catch {
                // Sem rede ou relay em baixo: tenta de novo (e pergunta entretanto).
                try? await Task.sleep(nanoseconds: Self.pollSeconds * 1_000_000_000)
            }
        }
    }

    /// Busca, guarda e confirma as mensagens em fila. `first`: mensagens que ja chegaram pela
    /// ligacao de espera; sao tratadas logo (antes de enviar a outbox), para entregar depressa.
    private func pollOnce(first: [RelayDelivered]? = nil) async {
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
                    let r = try await l.request(try await background { try client.relayRequestFetch() })
                    guard case .messages(let m) = r else { break }
                    msgs = m
                }
                if msgs.isEmpty { break }
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
                if got {
                    received = true
                    // Mostra ja: as mensagens estao guardadas no telemovel. A confirmacao ao relay e o
                    // pedido da pagina seguinte (duas idas a rede) vem depois, sem atrasar o ecra.
                    await refreshViews(client)
                }
                if !ackIds.isEmpty {   // so DEPOIS de guardar
                    _ = try await l.request(try await background { try client.relayRequestAck(ids: ackIds) })
                }
            }
            if received {
                kickTransfers()   // imagens recebidas: descarregar ja
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
