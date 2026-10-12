package pt.qt.app

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Contacto guardado na app. `publicId` = "qt1:" + hex(identidade 32B || assinatura 32B). */
data class Contact(val nickname: String, val publicId: String, val relayId: String) {
    /** Identificador usado pelo QtClient = hex da chave publica de identidade (1.a metade do Public ID). */
    val contactId: String get() = publicId.substring(4, 68)
}

/** Lista de contactos cifrada em repouso pelo Keystore (sao metadados: quem fala com quem). */
class ContactStore(ctx: Context, private val box: KeystoreBox) {
    private val file = File(ctx.noBackupFilesDir, "qt_contacts.bin")

    fun load(): List<Contact> {
        if (!file.exists()) return emptyList()
        return runCatching {
            val arr = JSONArray(String(box.open(file.readBytes()), Charsets.UTF_8))
            (0 until arr.length()).map {
                val o = arr.getJSONObject(it)
                Contact(o.getString("n"), o.getString("p"), o.getString("r"))
            }
        }.getOrDefault(emptyList())
    }

    fun save(contacts: List<Contact>) {
        val arr = JSONArray()
        contacts.forEach { arr.put(JSONObject().put("n", it.nickname).put("p", it.publicId).put("r", it.relayId)) }
        file.writeBytes(box.seal(arr.toString().toByteArray(Charsets.UTF_8)))
    }
}
