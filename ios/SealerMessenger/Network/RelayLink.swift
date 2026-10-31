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

/// Resposta por chegar a um pedido ja enviado. Resolve-se uma so vez: com a resposta do relay,
/// com um erro (ligacao fechada) ou com o cancelamento de quem pediu (a resposta, se chegar
/// depois, e ignorada; a ordem da fila mantem-se).
final class RelayAnswer: @unchecked Sendable {
    private let lock = NSLock()
    private var result: Result<RelayResponse, Error>?
    private var waiter: CheckedContinuation<RelayResponse, Error>?

    func resolve(_ r: Result<RelayResponse, Error>) {
        lock.lock()
        if result != nil { lock.unlock(); return }
        result = r
        let w = waiter
        waiter = nil
        lock.unlock()
        w?.resume(with: r)
    }

    func value() async throws -> RelayResponse {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (c: CheckedContinuation<RelayResponse, Error>) in
                lock.lock()
                if let r = result {
                    lock.unlock()
                    c.resume(with: r)
                } else {
                    waiter = c
                    lock.unlock()
                }
            }
        } onCancel: {
            resolve(.failure(CancellationError()))
        }
    }
}

/// WebSocket (URLSession) + Noise (NoiseSession, no Rust). O Swift so move bytes: a chave estatica
/// Noise, a assinatura de autenticacao e o codec das mensagens vivem no Rust.
/// Igual ao RelayLink.kt do Android:
///   ligar -> handshake Noise -> (pinning) -> autenticar -> pedidos.
/// Qualquer erro: fecha, DESCARTA a NoiseSession e re-liga com backoff (nova sessao sempre).
///
/// Desde a 0.9.32 a app usa UMA so ligacao para tudo (antes eram tres: pedidos, espera e anexos;
/// cada ligacao custa ~40 KB no servidor). Os pedidos vao em fila pela mesma ligacao, sem esperar
/// pela resposta do anterior, e o relay responde sempre pela mesma ordem: cada resposta vai para o
/// pedido mais antigo por responder (`Conn.pending`). A espera por mensagens (`WaitMessages`) e um
/// pedido como os outros: quando chega outro pedido a meio, o relay acaba a espera (responde com o
/// que houver) e trata o pedido a seguir. Assim um envio nunca fica parado atras da espera.
actor RelayLink {
    private let qt: QtClient
    private let url: String
    private let pinnedRelayKeyHex: String
    private let session: URLSession
    /// Para ligar e para escrever na ligacao: cifrar e enviar tem de ser pela mesma ordem.
    private let lock = AsyncLock()
    private var conn: Conn?

    private final class Conn: @unchecked Sendable {
        let ws: URLSessionWebSocketTask
        let noise: NoiseSession
        /// Respostas por chegar, pela ordem dos pedidos (so mexido dentro do actor).
        var pending: [RelayAnswer] = []
        var dead = false
        var reader: Task<Void, Never>?
        init(ws: URLSessionWebSocketTask, noise: NoiseSession) { self.ws = ws; self.noise = noise }
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
    /// quando chega uma mensagem ou outro pedido).
    func request(_ bytes: Data, timeout: Double = RelayLink.requestSeconds) async throws -> RelayResponse {
        var last: Error?
        let repeatable = Self.repeatable(bytes)
        for attempt in 0...Self.maxRetries {
            if attempt > 0 {
                let ms = min(Self.backoffMs << UInt64(attempt - 1), 8_000)
                try await Task.sleep(nanoseconds: ms * 1_000_000)
            }
            var sentOn: Conn?
            do {
                let (c, answers) = try await enqueue([bytes])
                sentOn = c
                return try await Self.withTimeout(timeout) { try await answers[0].value() }
            } catch is CancellationError {
                // Quem pediu desistiu: a resposta, quando chegar, fica sem dono e a ligacao
                // continua boa para os outros pedidos.
                throw CancellationError()
            } catch let e as RelayLinkError {
                close(sentOn)
                log("SealerRelay", "pedido falhou (tentativa \(attempt + 1)): \(e.errorDescription ?? "?")")
                if case .permanent = e { throw e }
                last = e
            } catch {
                close(sentOn)
                log("SealerRelay", "pedido falhou (tentativa \(attempt + 1)): \(String(error.localizedDescription.prefix(120)))")
                last = error                  // timeout, queda de rede, frame invalido...
            }
            // Ja tinha saido e a resposta perdeu-se: um envio ou um anexo novo nao se repete aqui
            // (podia chegar duas vezes). Quem chama tenta mais tarde (a outbox fica).
            if sentOn != nil && !repeatable { throw RelayLinkError.unavailable(last.map { String($0.localizedDescription.prefix(120)) } ?? "sem resposta") }
        }
        throw RelayLinkError.unavailable(last.map { String($0.localizedDescription.prefix(120)) } ?? "?")
    }

    /// Varios pedidos seguidos e so depois as respostas, pela mesma ordem: para os blocos
    /// dos anexos, sem esperar uma volta inteira a rede por cada bloco. Sem repeticao
    /// automatica: se falhar, quem chama retoma (o relay diz onde ficou). Um pedido de outra
    /// parte da app que chegue entretanto vai logo a seguir a estes blocos (por isso os lotes
    /// sao pequenos: 4 blocos de 40 KB).
    func requestMany(_ requests: [Data]) async throws -> [RelayResponse] {
        if requests.isEmpty { return [] }
        var sentOn: Conn?
        do {
            let (c, answers) = try await enqueue(requests)
            sentOn = c
            var out: [RelayResponse] = []
            out.reserveCapacity(answers.count)
            for a in answers {
                out.append(try await Self.withTimeout(Self.requestSeconds) { try await a.value() })
            }
            return out
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            close(sentOn)
            throw error
        }
    }

    /// Cifra e envia os pedidos (ligando primeiro, se preciso) e devolve onde ficam as respostas.
    /// Ligar e escrever acontecem com `lock`: os pedidos saem inteiros e pela ordem da fila.
    private func enqueue(_ requests: [Data]) async throws -> (Conn, [RelayAnswer]) {
        await lock.lock()
        do {
            let c: Conn
            if let existing = conn, !existing.dead {
                c = existing
            } else {
                close(conn)
                c = try await connect()
                conn = c
            }
            let answers = requests.map { _ in RelayAnswer() }
            c.pending.append(contentsOf: answers)
            do {
                for r in requests {
                    try await c.ws.send(.data(try c.noise.encrypt(plaintext: r)))
                }
            } catch {
                close(c)
                throw error
            }
            await lock.unlock()
            return (c, answers)
        } catch {
            await lock.unlock()
            throw error
        }
    }

    func close() { close(conn) }

    /// Fecha esta ligacao (se ainda for a atual, deixa de o ser). Os pedidos por responder falham.
    private func close(_ c: Conn?) {
        guard let c else { return }
        fail(c, RelayLinkError.closed)
        c.ws.cancel(with: .normalClosure, reason: nil)
        if conn === c { conn = nil }
    }

    /// A ligacao acabou: todos os pedidos por responder falham (quem pode repetir, repete).
    private func fail(_ c: Conn, _ e: Error) {
        c.dead = true
        let all = c.pending
        c.pending.removeAll()
        for a in all { a.resolve(.failure(e)) }
        c.reader?.cancel()
    }

    /// Uma frame do relay (pela ordem de chegada): decifra aqui, pela ordem (os contadores do
    /// Noise exigem-no), e entrega ao pedido mais antigo por responder.
    private func onFrame(_ c: Conn, _ frame: Data) {
        guard !c.dead else { return }
        guard !c.pending.isEmpty else {
            close(c)
            return
        }
        let a = c.pending.removeFirst()
        do {
            a.resolve(.success(try relayParseResponse(bytes: try c.noise.decrypt(ciphertext: frame))))
        } catch {
            a.resolve(.failure(error))
            close(c)
        }
    }

    /// Le as frames da ligacao, uma de cada vez, ate ela acabar.
    private func startReader(_ c: Conn) {
        let ws = c.ws
        c.reader = Task { [weak self] in
            while !Task.isCancelled {
                do {
                    let frame = try await Self.receiveData(ws)
                    guard let self else { return }
                    await self.onFrame(c, frame)
                } catch {
                    guard let self else { return }
                    await self.readerEnded(c, error)
                    return
                }
            }
        }
    }

    private func readerEnded(_ c: Conn, _ e: Error) {
        if !c.dead { log("SealerRelay", "ligacao caiu: \(String(e.localizedDescription.prefix(120)))") }
        close(c)
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
            // O pin e obrigatorio em todas as builds: sem ele nao se liga (0.9.23; tambem na Debug
            // desde a 0.9.53, sem relay de teste editavel).
            if pinnedRelayKeyHex.isEmpty {
                throw RelayLinkError.permanent("falta a chave do relay (pin)")
            }
            // Pinning: se a chave do relay nao for a esperada, aborta SEM repetir (possivel MITM)
            if !pinnedRelayKeyHex.isEmpty, try noise.remoteStaticPublicKey().hex != pinnedRelayKeyHex {
                throw RelayLinkError.permanent("chave do relay diferente da esperada (possivel intermediario)")
            }
            // Autenticacao ligada a esta sessao Noise
            let r = try await roundtrip(c, try qt.relayAuthMessage(handshakeHash: try noise.handshakeHash()))
            guard case .ack = r else { throw RelayLinkError.permanent("autenticacao recusada pelo relay") }
            startReader(c)   // daqui em diante as respostas vao para a fila dos pedidos
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

    /// Tempo maximo por uma resposta da fila (quem chama fecha a ligacao se acabar o tempo).
    private static func withTimeout<T: Sendable>(
        _ seconds: Double, _ op: @escaping @Sendable () async throws -> T
    ) async throws -> T {
        try await withThrowingTaskGroup(of: T.self) { group in
            group.addTask { try await op() }
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
                throw RelayLinkError.timeout
            }
            defer { group.cancelAll() }
            guard let first = try await group.next() else { throw RelayLinkError.closed }
            return first
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
