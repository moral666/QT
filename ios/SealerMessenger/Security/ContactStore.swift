import CryptoKit
import Foundation

/// Contacto guardado na app. `publicId` = "qt1:" + hex(identidade 32B || assinatura 32B).
struct Contact: Equatable, Hashable, Identifiable, Sendable {
    let nickname: String
    let publicId: String
    let relayId: String

    /// Identificador usado pelo QtClient = hex da chave publica de identidade (1.a metade do Public ID).
    var contactId: String { contactIdOf(publicId) }

    var id: String { contactId }
}

/// contactId (64 hex) de um Public ID "qt1:...".
func contactIdOf(_ publicId: String) -> String {
    let chars = Array(publicId)
    guard chars.count >= 68 else { return publicId }
    return String(chars[4..<68])
}

/// Lista de contactos cifrada em repouso (sao metadados: quem fala com quem). Igual ao Android:
///
///  * camada exterior: "QCB1" || Secure Enclave (`SecureBox.background`, abre com o iPhone
///    bloqueado para se saber quem liga com a app fechada);
///  * camada interior: "QCK1" || nonce || AES-256-GCM com a chave dos contactos, derivada da
///    chave do armazenamento (`contactsKeyFrom`). Com o bloqueio da app ativo, a lista so abre
///    depois do codigo ou do Face ID.
///
/// O ficheiro antigo (ate a 0.9.3: so `SecureBox.foreground`, sem cabecalho) continua a abrir e
/// passa ao formato novo na gravacao seguinte. Mesmo JSON que o Android: [{"n","p","r"}].
enum ContactStore {
    private struct Row: Codable { let n: String; let p: String; let r: String }
    private static let outerMagic = Data("QCB1".utf8)
    private static let innerMagic = Data("QCK1".utf8)
    private static let innerAad = Data("qt/contacts/v1".utf8)
    private static let lock = NSLock()

    private static func file() throws -> URL { try AppFiles.url("qt_contacts.bin") }

    private static func openOuter(_ raw: Data) throws -> Data {
        if raw.count > outerMagic.count, raw.prefix(outerMagic.count) == outerMagic,
           let d = try? SecureBox.background.open(raw.dropFirst(outerMagic.count)) {
            return d
        }
        return try SecureBox.foreground.open(raw)
    }

    static func load(key: Data) -> [Contact] {
        lock.lock(); defer { lock.unlock() }
        guard let url = try? file(), FileManager.default.fileExists(atPath: url.path),
              let raw = try? Data(contentsOf: url),
              let outer = try? openOuter(raw) else { return [] }
        let plain: Data
        if outer.count > innerMagic.count, outer.prefix(innerMagic.count) == innerMagic {
            guard let box = try? AES.GCM.SealedBox(combined: outer.dropFirst(innerMagic.count)),
                  let p = try? AES.GCM.open(box, using: SymmetricKey(data: key), authenticating: innerAad) else { return [] }
            plain = p
        } else {
            plain = outer
        }
        guard let rows = try? JSONDecoder().decode([Row].self, from: plain) else { return [] }
        return rows.map { Contact(nickname: $0.n, publicId: $0.p, relayId: $0.r) }
    }

    /// O ficheiro existe e ainda esta num formato antigo.
    static func isLegacy() -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard let url = try? file(), let raw = try? Data(contentsOf: url) else { return false }
        guard raw.prefix(outerMagic.count) == outerMagic, let outer = try? openOuter(raw) else { return true }
        return outer.prefix(innerMagic.count) != innerMagic
    }

    static func save(_ contacts: [Contact], key: Data) throws {
        lock.lock(); defer { lock.unlock() }
        let rows = contacts.map { Row(n: $0.nickname, p: $0.publicId, r: $0.relayId) }
        let plain = try JSONEncoder().encode(rows)
        guard let inner = try AES.GCM.seal(plain, using: SymmetricKey(data: key), authenticating: innerAad).combined else {
            throw SecureStoreError(message: "falha ao cifrar os contactos")
        }
        let sealed = try SecureBox.background.seal(innerMagic + inner)
        try AppFiles.writeAtomically(outerMagic + sealed, to: file())
    }

    static func wipe() {
        lock.lock(); defer { lock.unlock() }
        if let url = try? file() { try? FileManager.default.removeItem(at: url) }
    }
}
