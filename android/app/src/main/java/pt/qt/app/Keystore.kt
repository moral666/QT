package pt.qt.app

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Chave AES-256-GCM guardada no Android Keystore (nao exportavel). Serve APENAS para
 * proteger dados da APP em repouso: (a) o segredo de 32 bytes que abre o SQLCipher e
 * (b) a lista de contactos. NAO e criptografia do protocolo - essa vive toda no Rust.
 */
class KeystoreBox {
    private val alias = "qt_wrap_key_v1"

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(alias, null) as? SecretKey)?.let { return it }
        // 1.o tenta StrongBox (chip dedicado); se o aparelho nao tiver, cai para o Keystore normal.
        if (Build.VERSION.SDK_INT >= 28) {
            try { return generate(strongBox = true) } catch (_: Exception) { /* sem StrongBox */ }
        }
        return generate(strongBox = false)
    }

    private fun generate(strongBox: Boolean): SecretKey {
        val b = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
        if (Build.VERSION.SDK_INT >= 28) {
            b.setUnlockedDeviceRequired(true)
            if (strongBox) b.setIsStrongBoxBacked(true)
        }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
            .apply { init(b.build()) }.generateKey()
    }

    /** IV(12) || ciphertext||tag */
    fun seal(plain: ByteArray): ByteArray {
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        return c.iv + c.doFinal(plain)
    }

    fun open(blob: ByteArray): ByteArray {
        require(blob.size > 12 + 16) { "blob invalido" }
        val c = Cipher.getInstance("AES/GCM/NoPadding")
            .apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, blob.copyOfRange(0, 12))) }
        return c.doFinal(blob, 12, blob.size - 12)
    }
}

/**
 * Chave do SQLCipher: 32 bytes aleatorios, gerados uma vez, guardados CIFRADOS pelo Keystore
 * em ficheiro privado (fora de backups). Nunca vao para logs, Intents nem SharedPreferences.
 */
class StorageKeyProvider(ctx: Context, private val box: KeystoreBox) {
    private val file = File(ctx.noBackupFilesDir, "qt_storage_key.bin")

    /** O chamador DEVE fazer `fill(0)` ao array depois de abrir o QtClient. */
    fun storageKey(): ByteArray {
        if (!file.exists()) {
            val raw = ByteArray(32).also { SecureRandom().nextBytes(it) }
            file.writeBytes(box.seal(raw))
            return raw
        }
        return box.open(file.readBytes())
    }
}
