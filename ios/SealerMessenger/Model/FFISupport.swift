import Foundation
import os

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
extension AttachmentInfo: @unchecked Sendable {}
extension AttachmentRef: @unchecked Sendable {}
extension AttachmentState: @unchecked Sendable {}
extension CallAudio: @unchecked Sendable {}
extension CallAudioStats: @unchecked Sendable {}
extension CallEvent: @unchecked Sendable {}
extension CallEndReason: @unchecked Sendable {}
extension CallStart: @unchecked Sendable {}
extension CallWake: @unchecked Sendable {}
extension GroupInfo: @unchecked Sendable {}
extension GroupMember: @unchecked Sendable {}
extension GroupMemberInput: @unchecked Sendable {}
extension Reaction: @unchecked Sendable {}

extension GroupInfo: Identifiable {
    public var id: String { groupId }
}

extension ChatMessage: Identifiable {
    public var id: Int64 { seq }
}

/// Corre trabalho bloqueante (SQLCipher / criptografia no Rust) FORA da thread principal.
func background<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
    try await Task.detached(priority: .userInitiated) { try work() }.value
}

/// O mesmo, para trabalho que nao falha.
func background<T: Sendable>(_ work: @escaping @Sendable () -> T) async -> T {
    await Task.detached(priority: .userInitiated) { work() }.value
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
        case .blobCreated: return "BlobCreated"
        case .blobStatus: return "BlobStatus"
        case .blobData: return "BlobData"
        case .blobNotFound: return "BlobNotFound"
        case .backupPart: return "BackupPart"
        case .backupNotFound: return "BackupNotFound"
        case .premium: return "Premium"
        // Fichas do video (0006): o iPhone ainda nao tem video, nunca as pede.
        case .fichas: return "Fichas"
        }
    }
}

/// Registos (Consola do Mac, como o `adb logcat -s SealerRelay` do Android): nunca conteudo
/// nem chaves, so estados e motivos de erro.
func log(_ tag: String, _ msg: @autoclosure () -> String) {
    let text = msg()
    // A etiqueta tambem no texto: o filtro da consola do Xcode procura no texto da mensagem.
    Logger(subsystem: "sealer", category: tag).notice("[\(tag, privacy: .public)] \(text, privacy: .public)")
}

/// Mensagem curta e SEGURA para mostrar: nunca inclui conteudo nem chaves.
func safeMessage(_ error: Error) -> String {
    // Falhas de rede: "Sem ligacao ao servidor" na lingua da app (0.9.23; antes o texto tecnico).
    // O detalhe fica no registo do RelayLink.
    if error is RelayLinkError { return s("t_sem_ligacao_ao_servidor") }
    if error is QtError || error is FfiError {
        // So o nome do caso (ex.: "Storage", "Replay"), sem argumentos.
        let full = String(describing: error)
        return String(full.prefix { $0 != "(" })
    }
    return String(error.localizedDescription.prefix(120))
}
