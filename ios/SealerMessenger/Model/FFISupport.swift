import Foundation

// Os objetos do UniFFI (QtClient, NoiseSession) sao Arc<...> com Mutex no Rust: podem ser
// usados a partir de qualquer thread. Os tipos de dados sao copias imutaveis.
extension QtClient: @unchecked Sendable {}
extension NoiseSession: @unchecked Sendable {}
extension ChatMessage: @unchecked Sendable {}
extension ConversationSummary: @unchecked Sendable {}
extension OutboxItem: @unchecked Sendable {}
extension RelayResponse: @unchecked Sendable {}
extension RelayDelivered: @unchecked Sendable {}
extension ReceiveResult: @unchecked Sendable {}
extension MessageStatus: @unchecked Sendable {}
extension PublicIdentity: @unchecked Sendable {}
extension PeerBundle: @unchecked Sendable {}

extension ChatMessage: Identifiable {
    public var id: Int64 { seq }
}

/// Corre trabalho bloqueante (SQLCipher / criptografia no Rust) FORA da thread principal.
func background<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
    try await Task.detached(priority: .userInitiated) { try work() }.value
}

/// Exclusao mutua para codigo async (equivalente ao Mutex das corrotinas Kotlin).
actor AsyncLock {
    private var locked = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func lock() async {
        if !locked { locked = true; return }
        await withCheckedContinuation { waiters.append($0) }
    }

    func unlock() {
        if waiters.isEmpty { locked = false } else { waiters.removeFirst().resume() }
    }
}

extension Data {
    var hex: String { map { String(format: "%02x", $0) }.joined() }
}

extension RelayResponse {
    var describe: String {
        switch self {
        case .ack: return "Ack"
        case .bundle: return "Bundle"
        case .bundleNotFound: return "BundleNotFound"
        case .messages: return "Messages"
        case .error(let reason): return reason
        }
    }
}

/// Mensagem curta e SEGURA para mostrar: nunca inclui conteudo nem chaves.
func safeMessage(_ error: Error) -> String {
    if error is QtError || error is FfiError {
        // So o nome do caso (ex.: "Storage", "Replay"), sem argumentos.
        let full = String(describing: error)
        return String(full.prefix { $0 != "(" })
    }
    return String(error.localizedDescription.prefix(120))
}
