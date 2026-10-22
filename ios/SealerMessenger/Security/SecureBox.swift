import CryptoKit
import Foundation
import LocalAuthentication
import Security

/// Erro de armazenamento seguro (nunca contem material de chave).
struct SecureStoreError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// O bloqueio da app esta ativo: a chave so sai com o codigo (ou com o Face ID, ver `BioUnlock`).
struct AppLockedError: Error {}

/// Equivalente iOS do `KeystoreBox` do Android: uma chave P-256 gerada DENTRO do
/// Secure Enclave (chip dedicado; nao exportavel) cifra/decifra pequenos segredos da APP
/// (a chave do SQLCipher e a lista de contactos) com ECIES (ECDH + AES-GCM).
///
/// Duas chaves, como no Android desde a 0.9.6:
///  * `foreground`: so com o iPhone desbloqueado (`WhenUnlockedThisDeviceOnly`). Usada pelo
///    ficheiro do bloqueio da app ("QTL1") e pelos ficheiros antigos.
///  * `background`: tambem com o iPhone bloqueado, depois do primeiro desbloqueio desde que
///    ligou (`AfterFirstUnlockThisDeviceOnly`), para tocar uma chamada com a app fechada
///    (decisao "1B" do Nuno). Quem quiser a protecao antiga usa o bloqueio da app.
/// Nenhuma sai deste aparelho (sem backup, sem iCloud). No simulador (sem Secure Enclave)
/// usa uma chave normal do Keychain, para testes.
///
/// NAO e criptografia do protocolo: essa vive toda no Rust.
struct SecureBox {
    let tag: Data
    let accessibility: CFString

    static let foreground = SecureBox(tag: Data("pt.qt.sealermessenger.wrapkey.v1".utf8),
                                      accessibility: kSecAttrAccessibleWhenUnlockedThisDeviceOnly)
    static let background = SecureBox(tag: Data("sealer.wrapkey.bg.v1".utf8),
                                      accessibility: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly)

    private static let algorithm: SecKeyAlgorithm = .eciesEncryptionCofactorVariableIVX963SHA256AESGCM
    private static let lock = NSLock()

    static var isHardwareBacked: Bool {
        #if targetEnvironment(simulator)
        return false
        #else
        return true
        #endif
    }

    private func existingKey() -> SecKey? {
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

    private func createKey() throws -> SecKey {
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, accessibility, Self.isHardwareBacked ? .privateKeyUsage : [], &error
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
        if Self.isHardwareBacked {
            attrs[kSecAttrTokenID as String] = kSecAttrTokenIDSecureEnclave
        }
        guard let key = SecKeyCreateRandomKey(attrs as CFDictionary, &error) else {
            throw SecureStoreError(message: "nao foi possivel criar a chave no Secure Enclave")
        }
        return key
    }

    private func key(createIfMissing: Bool) throws -> SecKey {
        Self.lock.lock()
        defer { Self.lock.unlock() }
        if let k = existingKey() { return k }
        guard createIfMissing else { throw SecureStoreError(message: "chave do Secure Enclave inexistente") }
        return try createKey()
    }

    /// Cifra `plain` (so o Secure Enclave deste aparelho o consegue decifrar).
    func seal(_ plain: Data) throws -> Data {
        let priv = try key(createIfMissing: true)
        guard let pub = SecKeyCopyPublicKey(priv) else { throw SecureStoreError(message: "chave publica indisponivel") }
        var error: Unmanaged<CFError>?
        guard let out = SecKeyCreateEncryptedData(pub, Self.algorithm, plain as CFData, &error) as Data? else {
            throw SecureStoreError(message: "falha ao cifrar com o Secure Enclave")
        }
        return out
    }

    /// Decifra um blob de `seal`. Nunca cria uma chave nova: sem chave, falha.
    func open(_ blob: Data) throws -> Data {
        let priv = try key(createIfMissing: false)
        var error: Unmanaged<CFError>?
        guard let out = SecKeyCreateDecryptedData(priv, Self.algorithm, blob as CFData, &error) as Data? else {
            throw SecureStoreError(message: "nao foi possivel decifrar (chave invalidada, dados de outro aparelho ou corrompidos)")
        }
        return out
    }

    /// Apaga a chave (so ao apagar todos os dados).
    func deleteKey() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: tag,
        ]
        SecItemDelete(query as CFDictionary)
    }
}

/// Pasta privada da app, fora de backups do iCloud/iTunes.
///
/// Protecao dos ficheiros: `completeUntilFirstUserAuthentication` (como o Android desde a
/// 0.9.6): a base de dados abre com o iPhone bloqueado, depois do primeiro desbloqueio,
/// para uma chamada tocar com a app fechada. Os ficheiros continuam cifrados pelo SQLCipher.
enum AppFiles {
    static let protection = FileProtectionType.completeUntilFirstUserAuthentication

    static func directory() throws -> URL {
        var dir = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("QT", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true,
                                                attributes: [.protectionKey: protection])
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try dir.setResourceValues(values)
        return dir
    }

    static func url(_ name: String) throws -> URL { try directory().appendingPathComponent(name) }

    /// Escrita atomica (ficheiro temporario + rename).
    static func writeAtomically(_ data: Data, to url: URL) throws {
        try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    /// A pasta e tudo o que esta dentro passam a `protection` (as versoes ate a 0.9.3 usavam
    /// `complete`, que nao abre com o iPhone bloqueado).
    static func applyProtection() {
        guard let dir = try? directory() else { return }
        let fm = FileManager.default
        try? fm.setAttributes([.protectionKey: protection], ofItemAtPath: dir.path)
        guard let walker = fm.enumerator(at: dir, includingPropertiesForKeys: nil) else { return }
        for case let u as URL in walker {
            try? fm.setAttributes([.protectionKey: protection], ofItemAtPath: u.path)
        }
    }

    /// Ficheiros temporarios da app (envios a meio, ficheiros abertos para ver): apagados ao abrir e ao bloquear.
    static func cleanTemporary() {
        let fm = FileManager.default
        let tmp = fm.temporaryDirectory
        guard let items = try? fm.contentsOfDirectory(at: tmp, includingPropertiesForKeys: nil) else { return }
        for u in items where u.lastPathComponent.hasPrefix("envio-") || u.lastPathComponent.hasPrefix("ver-") || u.lastPathComponent.hasPrefix("voz-") {
            try? fm.removeItem(at: u)
        }
    }
}

/// Chave da lista de contactos, derivada da chave do armazenamento (igual ao Android).
/// Com o bloqueio ativo, a lista de contactos tambem so abre com o codigo.
func contactsKeyFrom(_ storageKey: Data) -> Data {
    var h = SHA256()
    h.update(data: Data("qt/contacts-key/v1".utf8))
    h.update(data: storageKey)
    return Data(h.finalize())
}

/// Chave do SQLCipher: 32 bytes aleatorios, gerados UMA vez, guardados CIFRADOS pelo Secure
/// Enclave num ficheiro privado (fora de backups). Igual ao `StorageKeyProvider` do Android:
///
///  * "QTK3" || background.seal(chave): normal (abre com o iPhone bloqueado, apos o 1.o desbloqueio).
///  * "QTK2" || foreground.seal(chave): versoes antigas; passa a QTK3 da primeira vez que abre.
///  * "QTL1" || foreground.seal(app_lock_wrap(codigo, chave)): bloqueio da app ativo.
///
/// Se a base de dados existir e a chave nao, falha em vez de criar uma chave nova (o que
/// esconderia uma perda de dados).
enum StorageKeyProvider {
    private static let magic2 = Data("QTK2".utf8)
    private static let magic3 = Data("QTK3".utf8)
    private static let magicLocked = Data("QTL1".utf8)
    private static let lock = NSRecursiveLock()

    private static func file() throws -> URL { try AppFiles.url("qt_storage_key.bin") }

    private static func readBlob() throws -> Data {
        do { return try Data(contentsOf: try file()) } catch {
            throw SecureStoreError(message: "nao foi possivel ler o ficheiro da chave")
        }
    }

    private static func exists() -> Bool {
        guard let f = try? file() else { return false }
        return FileManager.default.fileExists(atPath: f.path)
    }

    /// `true` se o bloqueio da app (codigo) esta ativo.
    static func lockEnabled() -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard exists(), let b = try? readBlob() else { return false }
        return b.count > magicLocked.count && b.prefix(magicLocked.count) == magicLocked
    }

    /// O chamador deve apagar o `Data` devolvido assim que abrir o QtClient.
    static func storageKey(dbFile: URL) throws -> Data {
        lock.lock(); defer { lock.unlock() }
        if !exists() { return try createNew(dbFile: dbFile) }
        let blob = try readBlob()
        if blob.prefix(magicLocked.count) == magicLocked { throw AppLockedError() }
        if blob.count > magic3.count, blob.prefix(magic3.count) == magic3 {
            return try checked(SecureBox.background.open(blob.dropFirst(magic3.count)))
        }
        if blob.count > magic2.count, blob.prefix(magic2.count) == magic2 {
            let key = try checked(SecureBox.foreground.open(blob.dropFirst(magic2.count)))
            migrateToV3(key, old: blob)
            return key
        }
        throw SecureStoreError(message: "ficheiro da chave com formato desconhecido")
    }

    /// Chave com o bloqueio ativo. Codigo errado: `QtError.Crypto` (vem do Rust).
    static func storageKeyWithCode(_ code: String) throws -> Data {
        lock.lock(); defer { lock.unlock() }
        let blob = try readBlob()
        guard blob.count > magicLocked.count, blob.prefix(magicLocked.count) == magicLocked else {
            throw SecureStoreError(message: "o bloqueio nao esta ativo")
        }
        var inner = try SecureBox.foreground.open(blob.dropFirst(magicLocked.count))
        defer { inner.resetBytes(in: 0..<inner.count) }
        return try checked(appLockUnwrap(code: code, blob: inner))
    }

    /// Ativa o bloqueio: a chave passa a precisar do codigo. Confirma antes de dar por feito.
    static func enableLock(code: String, dbFile: URL) throws {
        lock.lock(); defer { lock.unlock() }
        var key = try storageKey(dbFile: dbFile)
        defer { key.resetBytes(in: 0..<key.count) }
        try writeLocked(appLockWrap(code: code, storageKey: key))
        try verifyLocked(code, key)
    }

    /// Desativa o bloqueio (exige o codigo atual).
    static func disableLock(code: String) throws {
        lock.lock(); defer { lock.unlock() }
        var key = try storageKeyWithCode(code)
        defer { key.resetBytes(in: 0..<key.count) }
        try write(key)
        var back = try checked(SecureBox.background.open(try readBlob().dropFirst(magic3.count)))
        defer { back.resetBytes(in: 0..<back.count) }
        guard back == key else { throw SecureStoreError(message: "verificacao da chave falhou") }
    }

    /// Muda o codigo (exige o atual).
    static func changeCode(old: String, new: String) throws {
        lock.lock(); defer { lock.unlock() }
        var key = try storageKeyWithCode(old)
        defer { key.resetBytes(in: 0..<key.count) }
        try writeLocked(appLockWrap(code: new, storageKey: key))
        try verifyLocked(new, key)
    }

    /// Apaga o ficheiro da chave (so ao apagar todos os dados).
    static func deleteKeyFile() {
        lock.lock(); defer { lock.unlock() }
        if let f = try? file() { try? FileManager.default.removeItem(at: f) }
    }

    private static func verifyLocked(_ code: String, _ key: Data) throws {
        var back = try storageKeyWithCode(code)
        defer { back.resetBytes(in: 0..<back.count) }
        if back != key {
            try? write(key)   // volta ao formato sem bloqueio em vez de arriscar perder a chave
            throw SecureStoreError(message: "verificacao do bloqueio falhou")
        }
    }

    private static func writeLocked(_ lockBlob: Data) throws {
        try AppFiles.writeAtomically(magicLocked + SecureBox.foreground.seal(lockBlob), to: try file())
    }

    private static func write(_ raw: Data) throws {
        try AppFiles.writeAtomically(magic3 + SecureBox.background.seal(raw), to: try file())
    }

    private static func createNew(dbFile: URL) throws -> Data {
        if FileManager.default.fileExists(atPath: dbFile.path) {
            // Nunca criar uma chave nova por cima: esconderia uma perda de dados.
            throw SecureStoreError(message: "a base de dados existe mas a chave desapareceu; os dados locais sao irrecuperaveis")
        }
        var raw = Data(count: 32)
        let ok = raw.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }
        guard ok == errSecSuccess else { throw SecureStoreError(message: "gerador aleatorio indisponivel") }
        // O Keychain sobrevive a apagar a app: um Face ID de uma instalacao antiga (com outra
        // chave) faria aparecer o botao, que depois falhava.
        BioUnlock.disable()
        try write(raw)
        // Confirma que o que ficou em disco abre e da a mesma chave antes de a usar.
        guard try SecureBox.background.open(try readBlob().dropFirst(magic3.count)) == raw else {
            throw SecureStoreError(message: "verificacao da chave acabada de gravar falhou")
        }
        return raw
    }

    /// QTK2 -> QTK3. Se algo falhar, fica em QTK2 (continua a abrir com o iPhone desbloqueado).
    private static func migrateToV3(_ key: Data, old: Data) {
        do {
            try write(key)
            guard try SecureBox.background.open(try readBlob().dropFirst(magic3.count)) == key else {
                throw SecureStoreError(message: "verificacao falhou")
            }
        } catch {
            if let f = try? file() { try? AppFiles.writeAtomically(old, to: f) }
        }
    }

    private static func checked(_ k: Data) throws -> Data {
        guard k.count == 32 else { throw SecureStoreError(message: "chave com tamanho invalido") }
        return k
    }
}

/// Desbloqueio com Face ID, Touch ID ou o codigo do iPhone (igual a impressao digital no Android).
///
/// Guarda uma segunda copia da chave do armazenamento no Keychain, num item que so se le
/// depois de o utilizador se autenticar (`biometryCurrentSet` ou o codigo do iPhone, como no
/// Android: decisao de desenho). Com `.or`, um rosto/dedo novo deixa de servir para este item,
/// mas o codigo do iPhone continua a abri-lo (o item nao e apagado).
/// So existe neste iPhone (`WhenPasscodeSetThisDeviceOnly`: sem codigo no iPhone, nao existe).
/// O Keychain fica depois de apagar a app: uma chave do armazenamento nova apaga este item
/// (`StorageKeyProvider.createNew`, 0.9.23).
enum BioUnlock {
    private static let service = "sealer.bio.v1"
    private static let account = "storage-key"

    private static func baseQuery() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    /// O iPhone tem codigo (com ou sem Face ID/Touch ID).
    static func isAvailable() -> Bool {
        var err: NSError?
        return LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: &err)
    }

    /// Nome do metodo para os textos ("Face ID", "Touch ID" ou o codigo).
    static func biometryKind() -> LABiometryType {
        let c = LAContext()
        var err: NSError?
        _ = c.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &err)
        return c.biometryType
    }

    static func isEnabled() -> Bool {
        var q = baseQuery()
        let ctx = LAContext()
        ctx.interactionNotAllowed = true   // so ver se existe, sem pedir o Face ID
        q[kSecUseAuthenticationContext as String] = ctx
        q[kSecReturnAttributes as String] = true
        let status = SecItemCopyMatching(q as CFDictionary, nil)
        return status == errSecSuccess || status == errSecInteractionNotAllowed
    }

    /// Pede o Face ID / codigo do iPhone so para confirmar que e o dono (sem chave). `true` se o
    /// iPhone nao tem codigo (nao ha como confirmar). Bloqueante: fora da thread principal.
    static func confirmOwner(reason: String) -> Bool {
        let ctx = LAContext()
        var err: NSError?
        guard ctx.canEvaluatePolicy(.deviceOwnerAuthentication, error: &err) else { return true }
        var ok = false
        let done = DispatchSemaphore(value: 0)
        ctx.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason) { success, _ in
            ok = success
            done.signal()
        }
        done.wait()
        return ok
    }

    /// Pede o Face ID / codigo do iPhone e guarda a copia da chave. Bloqueante: fora da thread principal.
    static func save(_ storageKey: Data, reason: String) throws {
        let ctx = LAContext()
        var ok = false
        var authError: Error?
        let done = DispatchSemaphore(value: 0)
        ctx.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason) { success, e in
            ok = success
            authError = e
            done.signal()
        }
        done.wait()
        guard ok else { throw authError ?? SecureStoreError(message: "autenticacao cancelada") }
        disable()
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, [.biometryCurrentSet, .or, .devicePasscode], &error
        ) else { throw SecureStoreError(message: "controlo de acesso indisponivel") }
        var q = baseQuery()
        q[kSecValueData as String] = storageKey
        q[kSecAttrAccessControl as String] = access
        q[kSecUseAuthenticationContext as String] = ctx
        let status = SecItemAdd(q as CFDictionary, nil)
        guard status == errSecSuccess else { throw SecureStoreError(message: "nao foi possivel guardar (\(status))") }
    }

    /// Pede o Face ID / codigo do iPhone e devolve a chave. `nil`: cancelado ou ja nao serve.
    /// Bloqueante: chamar fora da thread principal. O chamador apaga o resultado.
    static func open(reason: String) -> Data? {
        let ctx = LAContext()
        ctx.localizedReason = reason
        var q = baseQuery()
        q[kSecUseAuthenticationContext as String] = ctx
        q[kSecReturnData as String] = true
        var item: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &item)
        if status == errSecItemNotFound { disable() }   // invalidado (rosto/dedo novo) ou apagado
        guard status == errSecSuccess, let d = item as? Data, d.count == 32 else { return nil }
        return d
    }

    static func disable() {
        SecItemDelete(baseQuery() as CFDictionary)
    }
}
