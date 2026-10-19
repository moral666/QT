package pt.qt.app

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec
import uniffi.qt_ffi.appLockUnwrap
import uniffi.qt_ffi.appLockWrap

/** A chave do Keystore deixou de estar utilizavel (invalidada, apagada ou dados corrompidos). */
class KeystoreUnavailableException(message: String, cause: Throwable? = null) : Exception(message, cause)

/**
 * Chave AES-256-GCM gerada DENTRO do Android Keystore e nao exportavel: o material da chave
 * nunca entra no processo da app. Fica no TEE do SoC ou, se o aparelho tiver, no chip
 * StrongBox dedicado (ver [isHardwareBacked]). Serve APENAS para proteger dados da APP em
 * repouso (o segredo que abre o SQLCipher e a lista de contactos). NAO e criptografia do
 * protocolo - essa vive toda no Rust.
 *
 * Porque nao `androidx.security.crypto.MasterKey`: essa biblioteca (Jetpack Security Crypto)
 * foi descontinuada pela Google; por baixo fazia exatamente isto (AES-256-GCM no
 * AndroidKeyStore), por isso usamos a API da plataforma diretamente, sem dependencias.
 */
class KeystoreBox(
    private val alias: String = DEFAULT_ALIAS,
    /**
     * `true` (antiga): a chave so funciona com o ecra desbloqueado. `false`: funciona tambem
     * com o telemovel bloqueado (depois do primeiro desbloqueio desde que ligou), para a app
     * receber chamadas e mensagens em segundo plano (fase 4b das chamadas, ver nota no fim).
     */
    private val unlockedDeviceRequired: Boolean = true,
) {

    companion object {
        const val DEFAULT_ALIAS = "qt_wrap_key_v1"
        /** Chave para o segundo plano (sem `setUnlockedDeviceRequired`), desde a app 0.9.6. */
        const val BACKGROUND_ALIAS = "qt_wrap_key_bg_v1"
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val IV_LEN = 12
        private const val TAG_BITS = 128
        private val lock = Any()
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }

    /** Devolve a chave existente; so gera uma nova se `createIfMissing`. */
    private fun key(createIfMissing: Boolean): SecretKey = synchronized(lock) {
        (keyStore().getKey(alias, null) as? SecretKey)?.let { return it }
        if (!createIfMissing) {
            throw KeystoreUnavailableException("chave '$alias' inexistente no Android Keystore")
        }
        // 1.o tenta StrongBox (chip dedicado); sem StrongBox cai para o TEE.
        if (Build.VERSION.SDK_INT >= 28) {
            try {
                return generate(strongBox = true)
            } catch (_: Exception) {
                // StrongBoxUnavailableException ou ProviderException conforme o fabricante.
                runCatching { keyStore().deleteEntry(alias) }
            }
        }
        generate(strongBox = false)
    }

    private fun generate(strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setRandomizedEncryptionRequired(true) // IV escolhido pelo Keystore, nunca por nos
            .apply {
                if (Build.VERSION.SDK_INT >= 28) {
                    // So utilizavel com o ecra desbloqueado (ver nota no fim do ficheiro).
                    if (unlockedDeviceRequired) setUnlockedDeviceRequired(true)
                    if (strongBox) setIsStrongBoxBacked(true)
                }
            }
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
            .apply { init(spec) }
            .generateKey()
    }

    /**
     * Cifra `plain`. Formato: IV(12) || ciphertext || tag(16).
     * `aad` (opcional) liga o blob ao seu proposito: um blob cifrado para X nao abre como Y.
     */
    fun seal(plain: ByteArray, aad: ByteArray? = null): ByteArray {
        val c = Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, key(createIfMissing = true)) }
        aad?.let { c.updateAAD(it) }
        val iv = c.iv
        check(iv.size == IV_LEN) { "IV inesperado do Keystore" }
        return iv + c.doFinal(plain)
    }

    /** Decifra um blob de [seal]. Nunca cria uma chave nova: sem chave, falha. */
    fun open(blob: ByteArray, aad: ByteArray? = null): ByteArray {
        require(blob.size >= IV_LEN + TAG_BITS / 8) { "blob invalido" }
        try {
            val c = Cipher.getInstance(TRANSFORMATION).apply {
                init(Cipher.DECRYPT_MODE, key(createIfMissing = false), GCMParameterSpec(TAG_BITS, blob, 0, IV_LEN))
            }
            aad?.let { c.updateAAD(it) }
            return c.doFinal(blob, IV_LEN, blob.size - IV_LEN)
        } catch (e: KeyPermanentlyInvalidatedException) {
            throw KeystoreUnavailableException("chave do Keystore invalidada pelo sistema", e)
        }
    }

    /** `true` se a chave vive em hardware seguro (TEE ou StrongBox). Emuladores devolvem `false`. */
    fun isHardwareBacked(): Boolean = runCatching {
        val k = key(createIfMissing = false)
        val info = SecretKeyFactory.getInstance(k.algorithm, ANDROID_KEYSTORE).getKeySpec(k, KeyInfo::class.java) as KeyInfo
        if (Build.VERSION.SDK_INT >= 31) {
            info.securityLevel == KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT ||
                info.securityLevel == KeyProperties.SECURITY_LEVEL_STRONGBOX
        } else {
            @Suppress("DEPRECATION")
            info.isInsideSecureHardware
        }
    }.getOrDefault(false)
}

/** O segredo do SQLCipher nao pode ser obtido. NUNCA se gera um novo por cima de uma base existente. */
class StorageKeyUnavailableException(message: String, cause: Throwable? = null) : Exception(message, cause)

/** O bloqueio da app esta ativo: a chave so sai com o codigo (ou com a biometria, ver [BioUnlock]). */
class AppLockedException : Exception("a app esta bloqueada")

/**
 * Chave do SQLCipher: 32 bytes aleatorios (CSPRNG), gerados UMA vez, guardados CIFRADOS pela
 * [KeystoreBox] num ficheiro privado fora de backups. Nunca existem como string, nem em
 * logs, Intents, SharedPreferences ou no codigo-fonte. O ficheiro sozinho e inutil: so o
 * Keystore deste aparelho o abre.
 *
 * Formato v3 (desde a 0.9.6): "QTK3" || IV(12) || ct || tag, cifrado por `bgBox` (chave do
 * Keystore que funciona com o telemovel bloqueado, para tocar chamadas com a app fechada).
 * Com o bloqueio da app ativo ("QTL1"), a chave continua a precisar do codigo.
 * Formato v2: "QTK2", cifrado por `box` (so com o ecra desbloqueado): passa a v3 da primeira
 * vez que abre. Ficheiros v1 (sem cabecalho, sem AAD) tambem sao migrados.
 *
 * @param dbFile ficheiro SQLCipher protegido por esta chave. Se ele existir e a chave nao,
 *               falha em vez de criar uma chave nova (o que esconderia uma perda de dados).
 */
class StorageKeyProvider(
    ctx: Context,
    private val box: KeystoreBox,
    private val bgBox: KeystoreBox,
    private val dbFile: File? = null,
) {
    private companion object {
        val MAGIC = byteArrayOf('Q'.code.toByte(), 'T'.code.toByte(), 'K'.code.toByte(), '2'.code.toByte())
        val MAGIC3 = byteArrayOf('Q'.code.toByte(), 'T'.code.toByte(), 'K'.code.toByte(), '3'.code.toByte())
        /** Bloqueio ativo: "QTL1" || Keystore( Rust app_lock_wrap(codigo, chave) ). */
        val MAGIC_LOCKED = byteArrayOf('Q'.code.toByte(), 'T'.code.toByte(), 'L'.code.toByte(), '1'.code.toByte())
        const val KEY_LEN = 32
        val lock = Any()
    }

    private val file = AtomicFile(File(ctx.noBackupFilesDir, "qt_storage_key.bin"))
    private val aad: ByteArray = ("qt/sqlcipher-key/v2/" + (dbFile?.name ?: "default")).toByteArray(Charsets.UTF_8)
    private val lockedAad: ByteArray = ("qt/sqlcipher-key/locked/v1/" + (dbFile?.name ?: "default")).toByteArray(Charsets.UTF_8)

    private fun startsWith(blob: ByteArray, magic: ByteArray): Boolean =
        blob.size > magic.size && blob.copyOfRange(0, magic.size).contentEquals(magic)

    private fun readBlob(): ByteArray = try { file.readFully() } catch (e: Exception) {
        throw StorageKeyUnavailableException("nao foi possivel ler o ficheiro da chave", e)
    }

    /** `true` se o bloqueio da app (codigo) esta ativo. */
    fun lockEnabled(): Boolean = synchronized(lock) {
        file.baseFile.exists() && runCatching { startsWith(file.readFully(), MAGIC_LOCKED) }.getOrDefault(false)
    }

    /**
     * Chave com o bloqueio ativo. Codigo errado: `QtException.Crypto` (vem do Rust).
     * O chamador DEVE fazer `fill(0)` ao resultado.
     */
    fun storageKeyWithCode(code: String): ByteArray = synchronized(lock) {
        val blob = readBlob()
        if (!startsWith(blob, MAGIC_LOCKED)) throw StorageKeyUnavailableException("o bloqueio nao esta ativo")
        val inner = try {
            box.open(blob.copyOfRange(MAGIC_LOCKED.size, blob.size), lockedAad)
        } catch (e: AEADBadTagException) {
            throw StorageKeyUnavailableException("ficheiro da chave corrompido ou de outro aparelho", e)
        } catch (e: KeystoreUnavailableException) {
            throw StorageKeyUnavailableException("a chave do Keystore ja nao esta disponivel", e)
        }
        try {
            appLockUnwrap(code, inner).also { checkLen(it) }
        } finally {
            inner.fill(0)
        }
    }

    /** Ativa o bloqueio: a chave passa a precisar do codigo. Confirma antes de dar por feito. */
    fun enableLock(code: String) {
        synchronized(lock) {
            val key = storageKey()
            try {
                writeLocked(appLockWrap(code, key))
                verifyLocked(code, key)
            } finally {
                key.fill(0)
            }
        }
    }

    /** Desativa o bloqueio (exige o codigo atual). */
    fun disableLock(code: String) {
        synchronized(lock) {
            val key = storageKeyWithCode(code)
            try {
                write(key)
                verifyUnlocked(key)
            } finally {
                key.fill(0)
            }
        }
    }

    /** Muda o codigo (exige o atual). */
    fun changeCode(oldCode: String, newCode: String) {
        synchronized(lock) {
            val key = storageKeyWithCode(oldCode)
            try {
                writeLocked(appLockWrap(newCode, key))
                verifyLocked(newCode, key)
            } finally {
                key.fill(0)
            }
        }
    }

    /** Apaga o ficheiro da chave (so ao apagar todos os dados). */
    fun deleteKeyFile() {
        synchronized(lock) { file.delete() }
    }

    private fun verifyUnlocked(key: ByteArray) {
        val back = storageKey()
        val same = MessageDigest.isEqual(back, key)
        back.fill(0)
        if (!same) throw StorageKeyUnavailableException("verificacao da chave falhou")
    }

    private fun verifyLocked(code: String, key: ByteArray) {
        val back = storageKeyWithCode(code)
        val same = MessageDigest.isEqual(back, key)
        back.fill(0)
        if (!same) {
            write(key)   // volta ao formato sem bloqueio em vez de arriscar perder a chave
            throw StorageKeyUnavailableException("verificacao do bloqueio falhou")
        }
    }

    private fun writeLocked(lockBlob: ByteArray) {
        val out = file.startWrite()
        try {
            out.write(MAGIC_LOCKED)
            out.write(box.seal(lockBlob, lockedAad))
            file.finishWrite(out)
        } catch (e: Exception) {
            file.failWrite(out)
            throw e
        }
    }

    /**
     * Devolve os 32 bytes da chave. O chamador DEVE fazer `fill(0)` ao array logo depois de
     * abrir o QtClient (o Rust copia-a para memoria com zeroize).
     */
    fun storageKey(): ByteArray = synchronized(lock) {
        if (!file.baseFile.exists()) return createNew()
        val blob = readBlob()
        if (startsWith(blob, MAGIC_LOCKED)) throw AppLockedException()
        try {
            if (startsWith(blob, MAGIC3)) {
                bgBox.open(blob.copyOfRange(MAGIC3.size, blob.size), aad).also { checkLen(it) }
            } else if (startsWith(blob, MAGIC)) {
                // v2 (so com o ecra desbloqueado): passa a v3 e confirma antes de dar por feito.
                box.open(blob.copyOfRange(MAGIC.size, blob.size), aad).also { checkLen(it); migrateToV3(it) }
            } else {
                // Formato v1 (KeystoreBox antigo, sem AAD): abrir e regravar em v2.
                box.open(blob).also { checkLen(it); migrateToV3(it) }
            }
        } catch (e: StorageKeyUnavailableException) {
            throw e
        } catch (e: AEADBadTagException) {
            throw StorageKeyUnavailableException("ficheiro da chave corrompido ou de outro aparelho", e)
        } catch (e: KeystoreUnavailableException) {
            throw StorageKeyUnavailableException("a chave do Keystore ja nao esta disponivel", e)
        } catch (e: Exception) {
            throw StorageKeyUnavailableException("falha ao decifrar a chave do armazenamento", e)
        }
    }

    private fun createNew(): ByteArray {
        if (dbFile?.exists() == true) {
            throw StorageKeyUnavailableException(
                "a base de dados existe mas a chave desapareceu; os dados locais sao irrecuperaveis"
            )
        }
        val raw = ByteArray(KEY_LEN).also { SecureRandom().nextBytes(it) }
        write(raw)
        // Confirma que o que ficou em disco abre e da a mesma chave antes de a usar.
        val back = bgBox.open(file.readFully().let { it.copyOfRange(MAGIC3.size, it.size) }, aad)
        val same = MessageDigest.isEqual(back, raw)
        back.fill(0)
        if (!same) {
            raw.fill(0)
            throw StorageKeyUnavailableException("verificacao da chave acabada de gravar falhou")
        }
        return raw
    }

    /**
     * v2 -> v3. Se algo falhar, o ficheiro fica em v2 (que continua a abrir com o ecra
     * desbloqueado): nunca se arrisca perder a chave.
     */
    private fun migrateToV3(key: ByteArray) {
        val old = file.readFully()
        try {
            write(key)
            val back = bgBox.open(file.readFully().let { it.copyOfRange(MAGIC3.size, it.size) }, aad)
            val same = MessageDigest.isEqual(back, key)
            back.fill(0)
            if (!same) throw IllegalStateException("verificacao falhou")
        } catch (e: Exception) {
            runCatching {
                val out = file.startWrite()
                try { out.write(old); file.finishWrite(out) } catch (e2: Exception) { file.failWrite(out); throw e2 }
            }
        }
    }

    /** Escrita atomica (ficheiro temporario + fsync + rename): nunca fica meio-escrita. */
    private fun write(raw: ByteArray) {
        val out = file.startWrite()
        try {
            out.write(MAGIC3)
            out.write(bgBox.seal(raw, aad))
            file.finishWrite(out)
        } catch (e: Exception) {
            file.failWrite(out)
            throw e
        }
    }

    private fun checkLen(k: ByteArray) {
        if (k.size != KEY_LEN) {
            k.fill(0)
            throw StorageKeyUnavailableException("chave com tamanho invalido")
        }
    }
}

/*
 * NOTA sobre setUnlockedDeviceRequired(true): com o ecra bloqueado a chave nao pode ser
 * usada, logo a base de dados nao abre em segundo plano com o aparelho bloqueado.
 *
 * Desde a 0.9.6 (chamadas com a app fechada, aprovado pelo Nuno a 19/10/2026, "1B") a chave
 * do SQLCipher e a camada exterior da lista de contactos usam uma chave DEDICADA
 * (`BACKGROUND_ALIAS`) sem essa restricao: funciona com o telemovel bloqueado, mas so depois
 * do primeiro desbloqueio desde que ligou (antes disso os ficheiros da app nem estao
 * acessiveis). Continua presa ao hardware do telemovel e nunca sai dele. Quem quiser a
 * protecao antiga usa o bloqueio da app (codigo): com ele ativo nada abre sem o codigo.
 * O ficheiro do bloqueio ("QTL1") continua com a chave antiga.
 */
