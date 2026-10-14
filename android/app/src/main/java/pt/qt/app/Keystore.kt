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
class KeystoreBox(private val alias: String = DEFAULT_ALIAS) {

    companion object {
        const val DEFAULT_ALIAS = "qt_wrap_key_v1"
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
                    setUnlockedDeviceRequired(true)
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

/**
 * Chave do SQLCipher: 32 bytes aleatorios (CSPRNG), gerados UMA vez, guardados CIFRADOS pela
 * [KeystoreBox] num ficheiro privado fora de backups. Nunca existem como string, nem em
 * logs, Intents, SharedPreferences ou no codigo-fonte. O ficheiro sozinho e inutil: so o
 * Keystore deste aparelho o abre.
 *
 * Formato v2: "QTK2" || IV(12) || ct || tag, com AAD = proposito + nome da base de dados.
 * Ficheiros v1 (sem cabecalho, sem AAD) sao migrados automaticamente.
 *
 * @param dbFile ficheiro SQLCipher protegido por esta chave. Se ele existir e a chave nao,
 *               falha em vez de criar uma chave nova (o que esconderia uma perda de dados).
 */
class StorageKeyProvider(
    ctx: Context,
    private val box: KeystoreBox,
    private val dbFile: File? = null,
) {
    private companion object {
        val MAGIC = byteArrayOf('Q'.code.toByte(), 'T'.code.toByte(), 'K'.code.toByte(), '2'.code.toByte())
        const val KEY_LEN = 32
        val lock = Any()
    }

    private val file = AtomicFile(File(ctx.noBackupFilesDir, "qt_storage_key.bin"))
    private val aad: ByteArray = ("qt/sqlcipher-key/v2/" + (dbFile?.name ?: "default")).toByteArray(Charsets.UTF_8)

    /**
     * Devolve os 32 bytes da chave. O chamador DEVE fazer `fill(0)` ao array logo depois de
     * abrir o QtClient (o Rust copia-a para memoria com zeroize).
     */
    fun storageKey(): ByteArray = synchronized(lock) {
        if (!file.baseFile.exists()) return createNew()
        val blob = try { file.readFully() } catch (e: Exception) {
            throw StorageKeyUnavailableException("nao foi possivel ler o ficheiro da chave", e)
        }
        try {
            if (blob.size > MAGIC.size && blob.copyOfRange(0, MAGIC.size).contentEquals(MAGIC)) {
                box.open(blob.copyOfRange(MAGIC.size, blob.size), aad).also { checkLen(it) }
            } else {
                // Formato v1 (KeystoreBox antigo, sem AAD): abrir e regravar em v2.
                box.open(blob).also { checkLen(it); runCatching { write(it) } }
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
        val back = box.open(file.readFully().let { it.copyOfRange(MAGIC.size, it.size) }, aad)
        val same = MessageDigest.isEqual(back, raw)
        back.fill(0)
        if (!same) {
            raw.fill(0)
            throw StorageKeyUnavailableException("verificacao da chave acabada de gravar falhou")
        }
        return raw
    }

    /** Escrita atomica (ficheiro temporario + fsync + rename): nunca fica meio-escrita. */
    private fun write(raw: ByteArray) {
        val out = file.startWrite()
        try {
            out.write(MAGIC)
            out.write(box.seal(raw, aad))
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
 * usada, logo a base de dados nao abre em background com o aparelho bloqueado (ex.: um
 * WorkManager a receber mensagens). E o compromisso mais seguro; se precisares de rececao
 * em background com ecra bloqueado, remove essa linha APENAS para uma chave dedicada a
 * isso e documenta-o no threat model.
 */
