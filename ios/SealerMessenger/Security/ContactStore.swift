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

    /// A lista guardada, ou vazia se ainda nao ha ficheiro (normal numa conta nova). Se o
    /// ficheiro existe mas nao abre, lanca `ContactsUnreadableError` (0.9.23): antes devolvia
    /// uma lista vazia e a gravacao seguinte apagava a lista verdadeira.
    static func load(key: Data) throws -> [Contact] {
        lock.lock(); defer { lock.unlock() }
        let url = try file()
        guard FileManager.default.fileExists(atPath: url.path) else { return [] }
        do {
            let raw = try Data(contentsOf: url)
            let outer = try openOuter(raw)
            let plain: Data
            if outer.count > innerMagic.count, outer.prefix(innerMagic.count) == innerMagic {
                let box = try AES.GCM.SealedBox(combined: outer.dropFirst(innerMagic.count))
                plain = try AES.GCM.open(box, using: SymmetricKey(data: key), authenticating: innerAad)
            } else {
                plain = outer
            }
            let rows = try JSONDecoder().decode([Row].self, from: plain)
            return rows.map { Contact(nickname: $0.n, publicId: $0.p, relayId: $0.r) }
        } catch {
            throw ContactsUnreadableError()
        }
    }

    /// Guarda uma copia do ficheiro que nao abriu ("qt_contacts.bad-<hora>.bin", cifrada como
    /// estava) e tira-o do caminho: a lista seguinte comeca vazia. So quando o utilizador escolhe.
    static func quarantine() {
        lock.lock(); defer { lock.unlock() }
        guard let url = try? file(), FileManager.default.fileExists(atPath: url.path) else { return }
        let dest = url.deletingLastPathComponent()
            .appendingPathComponent("qt_contacts.bad-\(Int64(Date().timeIntervalSince1970 * 1000)).bin")
        if (try? FileManager.default.moveItem(at: url, to: dest)) == nil {
            try? FileManager.default.removeItem(at: url)
        }
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

    /// Apaga a lista e as copias que nao abriam.
    static func wipe() {
        lock.lock(); defer { lock.unlock() }
        guard let url = try? file() else { return }
        try? FileManager.default.removeItem(at: url)
        let dir = url.deletingLastPathComponent()
        for u in (try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)) ?? []
        where u.lastPathComponent.hasPrefix("qt_contacts.bad-") {
            try? FileManager.default.removeItem(at: u)
        }
    }
}

/// O ficheiro dos contactos existe mas nao abre (chave, Secure Enclave ou ficheiro estragado).
struct ContactsUnreadableError: Error {}
