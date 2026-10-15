package pt.qt.app

import android.content.Context
import android.util.AtomicFile
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/** Contacto guardado na app. `publicId` = "qt1:" + hex(identidade 32B || assinatura 32B). */
data class Contact(val nickname: String, val publicId: String, val relayId: String) {
    /** Identificador usado pelo QtClient = hex da chave publica de identidade (1.a metade do Public ID). */
    val contactId: String get() = publicId.substring(4, 68)
}

/**
 * Lista de contactos cifrada em repouso pelo Keystore (sao metadados: quem fala com quem).
 *
 * Dentro da camada do Keystore ha uma segunda camada (AES-256-GCM) com a chave dos
 * contactos, derivada da chave do armazenamento ([contactsKeyFrom]). Com o bloqueio
 * da app ativo, a lista so abre depois do codigo ou da biometria. Ficheiros antigos
 * (so com a camada do Keystore) continuam a abrir e passam ao formato novo na
 * gravacao seguinte.
 *
 * Escrita ATOMICA (AtomicFile): se a app morrer a meio de gravar, fica a versao
 * anterior inteira, e nunca um ficheiro meio escrito (antes, isso fazia
 * desaparecer a lista de contactos toda).
 */
class ContactStore(ctx: Context, private val box: KeystoreBox) {
    private val file = AtomicFile(File(ctx.noBackupFilesDir, "qt_contacts.bin"))

    private companion object {
        val INNER_MAGIC = byteArrayOf('Q'.code.toByte(), 'C'.code.toByte(), 'K'.code.toByte(), '1'.code.toByte())
        val INNER_AAD = "qt/contacts/v1".toByteArray(Charsets.UTF_8)
        const val IV_LEN = 12
        const val TAG_BITS = 128
    }

    private fun startsWithMagic(b: ByteArray) =
        b.size > INNER_MAGIC.size && b.copyOfRange(0, INNER_MAGIC.size).contentEquals(INNER_MAGIC)

    /** @param innerKey chave dos contactos ([contactsKeyFrom]). */
    fun load(innerKey: ByteArray): List<Contact> {
        // Sem verificar raw.exists(): o AtomicFile recupera sozinho a copia de
        // seguranca (.bak) se a ultima gravacao foi interrompida.
        return runCatching {
            val outer = box.open(file.readFully())
            val plain = if (startsWithMagic(outer)) openInner(innerKey, outer) else outer
            val arr = JSONArray(String(plain, Charsets.UTF_8))
            (0 until arr.length()).map {
                val o = arr.getJSONObject(it)
                Contact(o.getString("n"), o.getString("p"), o.getString("r"))
            }
        }.getOrDefault(emptyList())
    }

    /** `true` se o ficheiro existe e ainda esta no formato antigo (sem a segunda camada). */
    fun isLegacy(): Boolean = runCatching { !startsWithMagic(box.open(file.readFully())) }.getOrDefault(false)

    fun save(contacts: List<Contact>, innerKey: ByteArray) {
        val arr = JSONArray()
        contacts.forEach { arr.put(JSONObject().put("n", it.nickname).put("p", it.publicId).put("r", it.relayId)) }
        val sealed = box.seal(sealInner(innerKey, arr.toString().toByteArray(Charsets.UTF_8)))
        val out = file.startWrite()
        try {
            out.write(sealed)
            file.finishWrite(out)
        } catch (e: Exception) {
            file.failWrite(out)
            throw e
        }
    }

    private fun sealInner(key: ByteArray, plain: ByteArray): ByteArray {
        val iv = ByteArray(IV_LEN).also { SecureRandom().nextBytes(it) }
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(TAG_BITS, iv))
        c.updateAAD(INNER_AAD)
        return INNER_MAGIC + iv + c.doFinal(plain)
    }

    private fun openInner(key: ByteArray, blob: ByteArray): ByteArray {
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(TAG_BITS, blob, INNER_MAGIC.size, IV_LEN))
        c.updateAAD(INNER_AAD)
        val start = INNER_MAGIC.size + IV_LEN
        return c.doFinal(blob, start, blob.size - start)
    }

    /** Apaga o ficheiro (ja cifrado) da lista de contactos. */
    fun wipe() {
        file.delete()
    }
}
