import Foundation
import Security

/// Erro de armazenamento seguro (nunca contem material de chave).
struct SecureStoreError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// Equivalente iOS do `KeystoreBox` do Android: uma chave P-256 gerada DENTRO do
/// Secure Enclave (chip dedicado; nao exportavel) cifra/decifra pequenos segredos da APP
/// (a chave do SQLCipher e a lista de contactos) com ECIES (ECDH + AES-GCM).
///
/// * So utilizavel com o iPhone desbloqueado (`WhenUnlockedThisDeviceOnly`), igual ao
///   `setUnlockedDeviceRequired(true)` do Android, e nunca sai deste aparelho (sem backup).
/// * No simulador (sem Secure Enclave) usa uma chave normal do Keychain, para testes.
///
/// NAO e criptografia do protocolo: essa vive toda no Rust.
enum SecureBox {
    private static let tag = Data("pt.qt.sealermessenger.wrapkey.v1".utf8)
    private static let algorithm: SecKeyAlgorithm = .eciesEncryptionCofactorVariableIVX963SHA256AESGCM

    static var isHardwareBacked: Bool {
        #if targetEnvironment(simulator)
        return false
        #else
        return true
        #endif
    }

    private static func existingKey() -> SecKey? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: tag,
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecReturnRef as String: true,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess, let found = item else { return nil }
        return (found as! SecKey)
    }

    private static func createKey() throws -> SecKey {
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, isHardwareBacked ? .privateKeyUsage : [], &error
        ) else {
            throw SecureStoreError(message: "controlo de acesso do Keychain indisponivel")
        }
        var attrs: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeySizeInBits as String: 256,
            kSecPrivateKeyAttrs as String: [
                kSecAttrIsPermanent as String: true,
                kSecAttrApplicationTag as String: tag,
                kSecAttrAccessControl as String: access,
            ] as [String: Any],
        ]
        if isHardwareBacked {
            attrs[kSecAttrTokenID as String] = kSecAttrTokenIDSecureEnclave
        }
        guard let key = SecKeyCreateRandomKey(attrs as CFDictionary, &error) else {
            throw SecureStoreError(message: "nao foi possivel criar a chave no Secure Enclave")
        }
        return key
    }

    private static func key(createIfMissing: Bool) throws -> SecKey {
        if let k = existingKey() { return k }
        guard createIfMissing else { throw SecureStoreError(message: "chave do Secure Enclave inexistente") }
        return try createKey()
    }

    /// Cifra `plain` (so o Secure Enclave deste aparelho o consegue decifrar).
    static func seal(_ plain: Data) throws -> Data {
        let priv = try key(createIfMissing: true)
        guard let pub = SecKeyCopyPublicKey(priv) else { throw SecureStoreError(message: "chave publica indisponivel") }
        var error: Unmanaged<CFError>?
        guard let out = SecKeyCreateEncryptedData(pub, algorithm, plain as CFData, &error) as Data? else {
            throw SecureStoreError(message: "falha ao cifrar com o Secure Enclave")
        }
        return out
    }

    /// Decifra um blob de `seal`. Nunca cria uma chave nova: sem chave, falha.
    static func open(_ blob: Data) throws -> Data {
        let priv = try key(createIfMissing: false)
        var error: Unmanaged<CFError>?
        guard let out = SecKeyCreateDecryptedData(priv, algorithm, blob as CFData, &error) as Data? else {
            throw SecureStoreError(message: "nao foi possivel decifrar (chave invalidada, dados de outro aparelho ou corrompidos)")
        }
        return out
    }
}

/// Pasta privada da app, fora de backups do iCloud/iTunes e com protecao de ficheiros completa.
enum AppFiles {
    static func directory() throws -> URL {
        var dir = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("QT", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true,
                                                attributes: [.protectionKey: FileProtectionType.complete])
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try dir.setResourceValues(values)
        return dir
    }

    static func url(_ name: String) throws -> URL { try directory().appendingPathComponent(name) }

    /// Escrita atomica (ficheiro temporario + rename), protegida enquanto o iPhone esta bloqueado.
    static func writeAtomically(_ data: Data, to url: URL) throws {
        try data.write(to: url, options: [.atomic, .completeFileProtection])
    }
}

/// Chave do SQLCipher: 32 bytes aleatorios, gerados UMA vez, guardados CIFRADOS pelo Secure Enclave
/// num ficheiro privado (fora de backups). Igual ao `StorageKeyProvider` do Android.
enum StorageKeyProvider {
    private static let magic = Data("QTK2".utf8)

    /// O chamador deve apagar o `Data` devolvido assim que abrir o QtClient.
    static func storageKey(dbFile: URL) throws -> Data {
        let file = try AppFiles.url("qt_storage_key.bin")
        if !FileManager.default.fileExists(atPath: file.path) {
            if FileManager.default.fileExists(atPath: dbFile.path) {
                // Nunca criar uma chave nova por cima: esconderia uma perda de dados.
                throw SecureStoreError(message: "a base de dados existe mas a chave desapareceu; os dados locais sao irrecuperaveis")
            }
            var raw = Data(count: 32)
            let ok = raw.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }
            guard ok == errSecSuccess else { throw SecureStoreError(message: "gerador aleatorio indisponivel") }
            try AppFiles.writeAtomically(magic + SecureBox.seal(raw), to: file)
            // Confirma que o que ficou em disco abre e da a mesma chave antes de a usar.
            guard try load(file) == raw else { throw SecureStoreError(message: "verificacao da chave acabada de gravar falhou") }
            return raw
        }
        return try load(file)
    }

    private static func load(_ file: URL) throws -> Data {
        let blob = try Data(contentsOf: file)
        guard blob.count > magic.count, blob.prefix(magic.count) == magic else {
            throw SecureStoreError(message: "ficheiro da chave com formato desconhecido")
        }
        let key = try SecureBox.open(blob.dropFirst(magic.count))
        guard key.count == 32 else { throw SecureStoreError(message: "chave com tamanho invalido") }
        return key
    }
}
