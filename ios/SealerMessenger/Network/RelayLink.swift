import Foundation

enum RelayLinkError: LocalizedError {
    /// NAO deve ser repetido (possivel MITM / autenticacao recusada / URL proibido).
    case permanent(String)
    case timeout
    case closed
    case unavailable(String)

    var errorDescription: String? {
        switch self {
        case .permanent(let m): return m
        case .timeout: return "tempo esgotado"
        case .closed: return "ligacao fechada"
        case .unavailable(let m): return "relay indisponivel: \(m)"
        }
    }
}

/// WebSocket (URLSession) + Noise (NoiseSession, no Rust). O Swift so move bytes: a chave estatica
/// Noise, a assinatura de autenticacao e o codec das mensagens vivem no Rust.
/// Igual ao RelayLink.kt do Android:
///   ligar -> handshake Noise -> (pinning) -> autenticar -> pedidos sequenciais.
/// Qualquer erro: fecha, DESCARTA a NoiseSession e re-liga com backoff (nova sessao sempre).
actor RelayLink {
    private let qt: QtClient
    private let url: String
    private let pinnedRelayKeyHex: String
    private let session: URLSession
    private let lock = AsyncLock()
    private var conn: Conn?

    private struct Conn {
        let ws: URLSessionWebSocketTask
        let noise: NoiseSession
    }

    private static let maxRetries = 3
    private static let backoffMs: UInt64 = 300
    private static let handshakeSeconds: Double = 10
    static let requestSeconds: Double = 15

    init(qt: QtClient, url: String, pinnedRelayKeyHex: String?) {
        self.qt = qt
        self.url = url.trimmingCharacters(in: .whitespaces)
        self.pinnedRelayKeyHex = (pinnedRelayKeyHex ?? "").trimmingCharacters(in: .whitespaces).lowercased()
        let cfg = URLSessionConfiguration.ephemeral          // sem cache, sem cookies em disco
        cfg.timeoutIntervalForRequest = 10
        cfg.waitsForConnectivity = false
        cfg.requestCachePolicy = .reloadIgnoringLocalCacheData
        cfg.httpCookieStorage = nil
        cfg.urlCache = nil
        self.session = URLSession(configuration: cfg)
    }

    /// Envia um pedido (ja construido com qt.relayRequest*) e devolve a resposta, com reconnect.
    /// `timeout`: tempo maximo pela resposta (maior para `relayRequestWait`, que so responde
    /// quando chega uma mensagem).
    func request(_ bytes: Data, timeout: Double = RelayLink.requestSeconds) async throws -> RelayResponse {
        await lock.lock()
        do {
            let r = try await requestLocked(bytes, timeout: timeout)
            await lock.unlock()
            return r
        } catch {
            await lock.unlock()
            throw error
        }
    }

    /// Varios pedidos seguidos e so depois as respostas, pela mesma ordem: para os blocos
    /// dos anexos, sem esperar uma volta inteira a rede por cada bloco. Sem repeticao
    /// automatica: se falhar, quem chama retoma (o relay diz onde ficou).
    func requestMany(_ requests: [Data]) async throws -> [RelayResponse] {
        await lock.lock()
        do {
            let c: Conn
            if let existing = conn { c = existing } else { c = try await connect(); conn = c }
            for r in requests {
                try await c.ws.send(.data(try c.noise.encrypt(plaintext: r)))
            }
            var out: [RelayResponse] = []
            out.reserveCapacity(requests.count)
            for _ in requests {
                let frame = try await Self.withTimeout(Self.requestSeconds, ws: c.ws) { try await Self.receiveData(c.ws) }
                out.append(try relayParseResponse(bytes: try c.noise.decrypt(ciphertext: frame)))
            }
            await lock.unlock()
            return out
        } catch {
            close()
            await lock.unlock()
            throw error
        }
    }

    func close() {
        conn?.ws.cancel(with: .normalClosure, reason: nil)
        conn = nil
    }

    private func requestLocked(_ bytes: Data, timeout: Double) async throws -> RelayResponse {
        var last: Error?
        let repeatable = Self.repeatable(bytes)
        for attempt in 0...Self.maxRetries {
            if attempt > 0 {
                let ms = min(Self.backoffMs << UInt64(attempt - 1), 8_000)
                try await Task.sleep(nanoseconds: ms * 1_000_000)
            }
            var sent = false
            do {
                let c: Conn
                if let existing = conn { c = existing } else { c = try await connect(); conn = c }
                try await c.ws.send(.data(try c.noise.encrypt(plaintext: bytes)))
                sent = true
                let frame = try await Self.withTimeout(timeout, ws: c.ws) { try await Self.receiveData(c.ws) }
                return try relayParseResponse(bytes: try c.noise.decrypt(ciphertext: frame))
            } catch let e as RelayLinkError {
                close()
                log("SealerRelay", "pedido falhou (tentativa \(attempt + 1)): \(e.errorDescription ?? "?")")
                if case .permanent = e { throw e }
                last = e
            } catch {
                close()
                log("SealerRelay", "pedido falhou (tentativa \(attempt + 1)): \(String(error.localizedDescription.prefix(120)))")
                last = error                  // timeout, queda de rede, frame invalido...
            }
            // Ja tinha saido e a resposta perdeu-se: um envio ou um anexo novo nao se repete aqui
            // (podia chegar duas vezes). Quem chama tenta mais tarde (a outbox fica).
            if sent && !repeatable { throw RelayLinkError.unavailable(last.map { String($0.localizedDescription.prefix(120)) } ?? "sem resposta") }
        }
        throw RelayLinkError.unavailable(last.map { String($0.localizedDescription.prefix(120)) } ?? "?")
    }

    /// Pode repetir-se sem efeito duplicado (leituras, confirmacoes, registos)? Desde a 0.9.23
    /// nao se repetem automaticamente, depois de enviados: mensagens, anexos novos, denuncias e
    /// a espera por mensagens.
    private static let notRepeatable = ["{\"SendMessage\"", "{\"BlobCreate\"", "{\"SubmitReport\"", "{\"WaitMessages\""]

    private static func repeatable(_ bytes: Data) -> Bool {
        let head = String(decoding: bytes.prefix(24), as: UTF8.self)
        return !notRepeatable.contains { head.hasPrefix($0) }
    }

    private func connect() async throws -> Conn {
        // Defesa em profundidade alem do ATS: release so aceita wss://.
        let lower = url.lowercased()
        guard lower.hasPrefix("wss://") || (Config.allowCleartext && lower.hasPrefix("ws://")),
              let u = URL(string: url), u.user == nil, u.password == nil
        else {
            throw RelayLinkError.permanent("URL do relay recusado: usa wss:// (ws:// so na build debug)")
        }
        let noise = try qt.relayNoiseSession()
        let ws = session.webSocketTask(with: u)
        ws.maximumMessageSize = 4 * 1024 * 1024
        ws.resume()
        let c = Conn(ws: ws, noise: noise)
        do {
            // Handshake Noise XX: write, read, write
            try await Self.withTimeout(Self.handshakeSeconds, ws: ws) {
                while !noise.isFinished() {
                    try await ws.send(.data(try noise.writeStep()))
                    if !noise.isFinished() {
                        try noise.readStep(message: try await Self.receiveData(ws))
                    }
                }
            }
            // Fora da Debug o pin e obrigatorio: sem ele nao se liga (0.9.23, defesa extra; o
            // script da build Release ja obriga a ter o pin).
            if pinnedRelayKeyHex.isEmpty && !Config.relayEditable {
                throw RelayLinkError.permanent("falta a chave do relay (pin)")
            }
            // Pinning: se a chave do relay nao for a esperada, aborta SEM repetir (possivel MITM)
            if !pinnedRelayKeyHex.isEmpty, try noise.remoteStaticPublicKey().hex != pinnedRelayKeyHex {
                throw RelayLinkError.permanent("chave do relay diferente da esperada (possivel intermediario)")
            }
            // Autenticacao ligada a esta sessao Noise
            let r = try await roundtrip(c, try qt.relayAuthMessage(handshakeHash: try noise.handshakeHash()))
            guard case .ack = r else { throw RelayLinkError.permanent("autenticacao recusada pelo relay") }
            return c
        } catch {
            ws.cancel(with: .goingAway, reason: nil)
            throw error
        }
    }

    private func roundtrip(_ c: Conn, _ bytes: Data, timeout: Double = RelayLink.requestSeconds) async throws -> RelayResponse {
        try await c.ws.send(.data(try c.noise.encrypt(plaintext: bytes)))
        let frame = try await Self.withTimeout(timeout, ws: c.ws) { try await Self.receiveData(c.ws) }
        return try relayParseResponse(bytes: try c.noise.decrypt(ciphertext: frame))
    }

    private static func receiveData(_ ws: URLSessionWebSocketTask) async throws -> Data {
        switch try await ws.receive() {
        case .data(let d): return d
        case .string: throw RelayLinkError.permanent("frame de texto inesperado do relay")
        @unknown default: throw RelayLinkError.closed
        }
    }

    /// Timeout que tambem fecha o socket (o receive() do URLSession nao reage a cancelamento).
    private static func withTimeout<T: Sendable>(
        _ seconds: Double, ws: URLSessionWebSocketTask, _ op: @escaping @Sendable () async throws -> T
    ) async throws -> T {
        try await withThrowingTaskGroup(of: T.self) { group in
            group.addTask { try await op() }
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
                ws.cancel(with: .goingAway, reason: nil)
                throw RelayLinkError.timeout
            }
            defer { group.cancelAll() }
            guard let first = try await group.next() else { throw RelayLinkError.closed }
            return first
        }
    }
}
