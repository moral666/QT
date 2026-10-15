import Foundation

/// Contacto guardado na app. `publicId` = "qt1:" + hex(identidade 32B || assinatura 32B).
struct Contact: Equatable, Hashable, Identifiable {
    let nickname: String
    let publicId: String
    let relayId: String

    /// Identificador usado pelo QtClient = hex da chave publica de identidade (1.a metade do Public ID).
    var contactId: String {
        let chars = Array(publicId)
        guard chars.count >= 68 else { return publicId }
        return String(chars[4..<68])
    }

    var id: String { contactId }
}

/// Lista de contactos cifrada em repouso pelo Secure Enclave (sao metadados: quem fala com quem).
/// Mesmo formato JSON que o Android: [{"n": nome, "p": publicId, "r": relayId}].
enum ContactStore {
    private struct Row: Codable { let n: String; let p: String; let r: String }

    private static func file() throws -> URL { try AppFiles.url("qt_contacts.bin") }

    static func load() -> [Contact] {
        guard let url = try? file(), FileManager.default.fileExists(atPath: url.path),
              let blob = try? Data(contentsOf: url),
              let plain = try? SecureBox.open(blob),
              let rows = try? JSONDecoder().decode([Row].self, from: plain)
        else { return [] }
        return rows.map { Contact(nickname: $0.n, publicId: $0.p, relayId: $0.r) }
    }

    static func wipe() throws {
        let url = try file()
        if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
    }

    static func save(_ contacts: [Contact]) throws {
        let rows = contacts.map { Row(n: $0.nickname, p: $0.publicId, r: $0.relayId) }
        let plain = try JSONEncoder().encode(rows)
        try AppFiles.writeAtomically(SecureBox.seal(plain), to: file())
    }
}
