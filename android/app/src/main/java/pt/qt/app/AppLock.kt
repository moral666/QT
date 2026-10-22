package pt.qt.app

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import androidx.annotation.RequiresApi
import androidx.biometric.BiometricManager
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Tempos de bloqueio automatico oferecidos nas definicoes (rotulo, milissegundos). */
val LOCK_TIMEOUTS: List<Pair<String, Long>> get() = listOf(
    s(R.string.t_imediatamente) to 0L,
    s(R.string.t_apos_1_minuto) to 60_000L,
    s(R.string.t_apos_5_minutos) to 300_000L,
    s(R.string.t_apos_15_minutos) to 900_000L,
)

/** Para que serve um pedido de autenticacao do sistema (BiometricPrompt). */
enum class BioPurpose {
    UNLOCK, ENABLE,
    /** Confirmar que e o dono antes de mostrar ou gerar as 12 palavras (sem Cipher). */
    RECOVERY,
}

/** Pedido do ViewModel a Activity: mostrar o BiometricPrompt com esta Cipher (RECOVERY: sem). */
class BioRequest(val purpose: BioPurpose, val cipher: Cipher?)

/**
 * Chave da lista de contactos, derivada da chave do armazenamento. Com o bloqueio
 * ativo, a lista de contactos (quem fala com quem) tambem so abre com o codigo.
 */
fun contactsKeyFrom(storageKey: ByteArray): ByteArray =
    MessageDigest.getInstance("SHA-256").run {
        update("qt/contacts-key/v1".toByteArray(Charsets.UTF_8))
        update(storageKey)
        digest()
    }

/**
 * Desbloqueio com impressao digital, rosto ou PIN do telemovel (Android 11+).
 *
 * Guarda uma segunda copia da chave do armazenamento, cifrada por uma chave do
 * Keystore que so funciona depois de o utilizador se autenticar no BiometricPrompt
 * (cada utilizacao exige autenticacao). Se for registada uma impressao digital
 * nova ou se o bloqueio do ecra for desligado, o Android invalida essa chave: a
 * copia deixa de servir e passa a ser preciso o codigo da app.
 */
class BioUnlock(ctx: Context) {
    private val appCtx = ctx.applicationContext
    private val file = AtomicFile(File(ctx.noBackupFilesDir, "qt_storage_key_bio.bin"))

    companion object {
        private const val ALIAS = "qt_bio_key_v1"
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val IV_LEN = 12
        private const val TAG_BITS = 128
        private val MAGIC = byteArrayOf('Q'.code.toByte(), 'T'.code.toByte(), 'B'.code.toByte(), '1'.code.toByte())
        private val AAD = "qt/sqlcipher-key/bio/v1".toByteArray(Charsets.UTF_8)

        /** Impressao digital/rosto (classe forte) ou o PIN/padrao do telemovel. */
        const val AUTHENTICATORS =
            BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL

        /** So confirmar quem e (sem chave do Keystore): funciona em todas as versoes do Android. */
        const val DEVICE_AUTHENTICATORS =
            BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL
    }

    /** O telemovel tem bloqueio de ecra (PIN, padrao, impressao digital ou rosto). */
    fun deviceAuthAvailable(): Boolean =
        BiometricManager.from(appCtx).canAuthenticate(DEVICE_AUTHENTICATORS) == BiometricManager.BIOMETRIC_SUCCESS

    /** Android 11+ e o telemovel tem bloqueio de ecra (com ou sem biometria). */
    fun isAvailable(): Boolean = Build.VERSION.SDK_INT >= 30 &&
        BiometricManager.from(appCtx).canAuthenticate(AUTHENTICATORS) == BiometricManager.BIOMETRIC_SUCCESS

    fun isEnabled(): Boolean = file.baseFile.exists()

    /** Gera uma chave nova e devolve a Cipher que, depois do BiometricPrompt, guarda a copia. */
    @RequiresApi(30)
    fun cipherForEnable(): Cipher {
        deleteKey()
        val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
            .setInvalidatedByBiometricEnrollment(true)
            .build()
        val key = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
            .apply { init(spec) }
            .generateKey()
        return Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, key) }
    }

    /** Cipher para abrir a copia; `null` se a copia ja nao serve (e entao e apagada). */
    @RequiresApi(30)
    fun cipherForUnlock(): Cipher? {
        val blob = runCatching { file.readFully() }.getOrNull() ?: return null
        if (blob.size <= MAGIC.size + IV_LEN || !blob.copyOfRange(0, MAGIC.size).contentEquals(MAGIC)) {
            disable()
            return null
        }
        val key = runCatching { keyStore().getKey(ALIAS, null) as? SecretKey }.getOrNull()
        if (key == null) {
            disable()
            return null
        }
        return try {
            Cipher.getInstance(TRANSFORMATION).apply {
                init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, blob, MAGIC.size, IV_LEN))
            }
        } catch (e: KeyPermanentlyInvalidatedException) {
            disable()
            null
        }
    }

    /** Guarda a copia da chave do armazenamento (Cipher ja autorizada pelo BiometricPrompt). */
    fun save(cipher: Cipher, storageKey: ByteArray) {
        cipher.updateAAD(AAD)
        val ct = cipher.doFinal(storageKey)
        val iv = cipher.iv
        check(iv != null && iv.size == IV_LEN) { "IV inesperado do Keystore" }
        val out = file.startWrite()
        try {
            out.write(MAGIC)
            out.write(iv)
            out.write(ct)
            file.finishWrite(out)
        } catch (e: Exception) {
            file.failWrite(out)
            throw e
        }
    }

    /** Devolve a chave do armazenamento (Cipher ja autorizada). O chamador faz `fill(0)`. */
    fun open(cipher: Cipher): ByteArray {
        val blob = file.readFully()
        cipher.updateAAD(AAD)
        return cipher.doFinal(blob, MAGIC.size + IV_LEN, blob.size - MAGIC.size - IV_LEN)
    }

    /** Desliga a biometria: apaga a copia e a chave do Keystore. */
    fun disable() {
        runCatching { file.delete() }
        deleteKey()
    }

    private fun deleteKey() {
        runCatching { keyStore().deleteEntry(ALIAS) }
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
}
