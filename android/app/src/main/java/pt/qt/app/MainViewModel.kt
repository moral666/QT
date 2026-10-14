package pt.qt.app

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import uniffi.qt_ffi.QtClient
import uniffi.qt_ffi.QtException
import uniffi.qt_ffi.RelayResponse
import uniffi.qt_ffi.peerBundleFromRelayBytes
import uniffi.qt_ffi.relayIdOfPublicId

data class ChatMessage(val fromMe: Boolean, val text: String, val pending: Boolean = false)

data class UiState(
    val loading: Boolean = true,
    val hasIdentity: Boolean = false,
    val publicId: String = "",
    val relayUrl: String = if (BuildConfig.ALLOW_CLEARTEXT) "ws://192.168.1.10:9443" else "wss://",
    val relayKeyHex: String = "",
    val status: String = "",
    val error: String? = null,
    val contacts: List<Contact> = emptyList(),
    /** Conversas por contactId. SO EM MEMORIA nesta versao (ver docs). */
    val chats: Map<String, List<ChatMessage>> = emptyMap(),
    /** Remetentes que nos escreveram mas ainda nao adicionamos (contactId). */
    val unknownSenders: Set<String> = emptySet(),
    /** Safety number a mostrar (nome + 60 digitos), ou null. */
    val safetyNumber: String? = null,
)

class MainViewModel(app: Application) : AndroidViewModel(app) {
    private val _ui = MutableStateFlow(UiState())
    val ui: StateFlow<UiState> = _ui

    private val prefs = app.getSharedPreferences("qt_prefs", 0)   // so URL do relay e chave PUBLICA (nao secretos)
    private val box = KeystoreBox()
    private val contactStore = ContactStore(app, box)
    private var qt: QtClient? = null
    private var link: RelayLink? = null
    private var poller: Job? = null

    /** Envelopes JA CIFRADOS (ratchet ja avancou) a espera de rede. Sao ciphertext, nao plaintext. */
    private val outbox = mutableMapOf<String, MutableList<Pair<Contact, ByteArray>>>()

    init {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val dbFile = getApplication<Application>().noBackupFilesDir.resolve("qt.db")
                val provider = StorageKeyProvider(getApplication(), box, dbFile)
                val key = provider.storageKey()
                val client = try { QtClient.open(dbFile.path, key) } finally { key.fill(0) }
                qt = client
                val url = prefs.getString("relay_url", null) ?: _ui.value.relayUrl
                val pin = prefs.getString("relay_key", "") ?: ""
                val has = client.hasIdentity()
                _ui.update {
                    it.copy(
                        loading = false, hasIdentity = has, relayUrl = url, relayKeyHex = pin,
                        publicId = if (has) client.exportPublicIdentity().publicId else "",
                        contacts = contactStore.load(),
                    )
                }
                if (has) startLink()
            } catch (e: StorageKeyUnavailableException) {
                // Nunca se cria uma chave nova por cima: o utilizador decide se apaga os dados locais.
                _ui.update { it.copy(loading = false, error = "Chave do armazenamento seguro indisponivel: ${e.message}") }
            } catch (e: Exception) {
                _ui.update { it.copy(loading = false, error = "Nao foi possivel abrir o armazenamento seguro: ${e.safeMsg()}") }
            }
        }
    }

    // ---------------------------------------------------------------- configuracao

    fun createIdentity(relayUrl: String, relayKeyHex: String) = io {
        val client = qt ?: return@io
        saveRelay(relayUrl, relayKeyHex)
        val me = client.createIdentity()
        _ui.update { it.copy(hasIdentity = true, publicId = me.publicId) }
        startLink()
    }

    fun saveRelay(relayUrl: String, relayKeyHex: String) {
        val u = relayUrl.trim().lowercase()
        if (!u.startsWith("wss://") && !(BuildConfig.ALLOW_CLEARTEXT && u.startsWith("ws://"))) {
            error("O relay tem de usar wss:// (ws:// so na build debug)")
            return
        }
        prefs.edit().putString("relay_url", relayUrl.trim()).putString("relay_key", relayKeyHex.trim()).apply()
        _ui.update { it.copy(relayUrl = relayUrl.trim(), relayKeyHex = relayKeyHex.trim()) }
        // hasIdentity() toca no SQLCipher: nunca na thread principal.
        io { if (qt?.hasIdentity() == true) startLink() }
    }

    private suspend fun startLink() {
        val client = qt ?: return
        poller?.cancel()
        link?.close()
        link = RelayLink(client, _ui.value.relayUrl, _ui.value.relayKeyHex.ifBlank { null })
        status("A ligar ao relay…")
        try {
            // Publicar (ou re-publicar) o bundle publico para que outros nos possam contactar.
            val r = link!!.request(client.relayRequestRegisterBundle(client.publishableBundle()))
            if (r is RelayResponse.Ack) status("Ligado ao relay") else status("Relay recusou o bundle: ${r.describe()}")
        } catch (e: Exception) {
            status("Sem ligacao ao relay: ${e.safeMsg()}")
        }
        poller = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) { pollOnce(); delay(POLL_MS) }
        }
    }

    // ------------------------------------------------------------------- contactos

    /** Guarda o contacto. NAO inicia sessao (evita sessoes cruzadas): so ao enviar a 1.a mensagem. */
    fun addContact(nickname: String, publicId: String) = io {
        try {
            val id = publicId.trim()
            val relayId = relayIdOfPublicId(id)               // valida o formato no Rust
            val c = Contact(nickname.trim().ifBlank { "Contacto" }, id, relayId)
            if (c.publicId == _ui.value.publicId) { error("Esse e o teu proprio ID"); return@io }
            val list = (_ui.value.contacts.filter { it.contactId != c.contactId }) + c
            contactStore.save(list)
            _ui.update { it.copy(contacts = list, unknownSenders = it.unknownSenders - c.contactId, error = null) }
        } catch (e: QtException) {
            error("Public ID invalido")
        }
    }

    // ---------------------------------------------------------- safety numbers

    /** Mostra o safety number com este contacto (calculado no Rust a partir dos dois Public IDs). */
    fun showSafetyNumber(contact: Contact) = io {
        val client = qt ?: return@io
        try {
            val n = client.safetyNumber(contact.publicId)
            _ui.update { it.copy(safetyNumber = "${contact.nickname}\n\n$n") }
        } catch (e: Exception) {
            error("Nao foi possivel calcular o safety number: ${e.safeMsg()}")
        }
    }

    fun dismissSafetyNumber() = _ui.update { it.copy(safetyNumber = null) }

    // ---------------------------------------------------------------------- enviar

    fun send(contact: Contact, text: String) = io {
        val client = qt ?: return@io
        val bytes = text.toByteArray(Charsets.UTF_8)
        if (bytes.isEmpty() || bytes.size > MAX_TEXT_BYTES) { error("Mensagem vazia ou maior que $MAX_TEXT_BYTES bytes"); return@io }
        try {
            ensureSession(client, contact)
            val blob = client.encryptSealed(contact.contactId, bytes)      // o ratchet avanca e persiste aqui
            appendChat(contact.contactId, ChatMessage(true, text, pending = true))
            val ok = trySend(contact, blob)
            if (!ok) outbox.getOrPut(contact.contactId) { mutableListOf() }.add(contact to blob)
            else markSent(contact.contactId)
        } catch (e: Exception) {
            error("Nao foi possivel enviar: ${e.safeMsg()}")
        }
    }

    private suspend fun ensureSession(client: QtClient, c: Contact) {
        if (client.hasSession(c.contactId)) return
        val l = link ?: throw IllegalStateException("sem ligacao ao relay")
        when (val r = l.request(client.relayRequestFetchBundle(c.relayId))) {
            is RelayResponse.Bundle -> client.startSession(c.publicId, peerBundleFromRelayBytes(r.bytes))
            is RelayResponse.BundleNotFound -> throw IllegalStateException("o contacto ainda nao publicou as chaves (tem de abrir a app uma vez)")
            else -> throw IllegalStateException("resposta inesperada do relay: ${r.describe()}")
        }
    }

    private suspend fun trySend(c: Contact, blob: ByteArray): Boolean = try {
        val r = link!!.request(qt!!.relayRequestSend(c.relayId, blob))
        r is RelayResponse.Ack
    } catch (e: Exception) { false }

    // --------------------------------------------------------------------- receber

    private suspend fun pollOnce() {
        val client = qt ?: return
        val l = link ?: return
        try {
            flushOutbox()
            repeat(MAX_PAGES) {
                val r = l.request(client.relayRequestFetch())
                val msgs = (r as? RelayResponse.Messages)?.messages ?: return
                if (msgs.isEmpty()) return
                val ackIds = mutableListOf<ULong>()
                for (m in msgs) {
                    try {
                        val d = client.decryptSealed(m.ciphertext)
                        appendChat(d.contactId, ChatMessage(false, String(d.plaintext, Charsets.UTF_8)))
                        if (_ui.value.contacts.none { it.contactId == d.contactId })
                            _ui.update { it.copy(unknownSenders = it.unknownSenders + d.contactId) }
                        ackIds += m.id
                    } catch (e: QtException.Storage) {
                        // Problema local: NAO confirmar, o relay volta a entregar mais tarde.
                    } catch (e: QtException) {
                        // Replay (ja processada) ou envelope invalido: nao ha nada a recuperar -> confirmar.
                        ackIds += m.id
                    }
                }
                if (ackIds.isNotEmpty()) l.request(client.relayRequestAck(ackIds))   // so DEPOIS de processar
            }
            status("Ligado ao relay")
        } catch (e: Exception) {
            status("Sem ligacao ao relay: ${e.safeMsg()}")
        }
    }

    private suspend fun flushOutbox() {
        for ((cid, list) in outbox.toMap()) {
            val remaining = mutableListOf<Pair<Contact, ByteArray>>()
            for (p in list) if (!trySend(p.first, p.second)) remaining += p else markSent(cid)
            if (remaining.isEmpty()) outbox.remove(cid) else outbox[cid] = remaining
        }
    }

    // ------------------------------------------------------------------ utilitarios

    private fun appendChat(contactId: String, m: ChatMessage) = _ui.update {
        it.copy(chats = it.chats + (contactId to ((it.chats[contactId] ?: emptyList()) + m)))
    }

    private fun markSent(contactId: String) = _ui.update {
        val msgs = it.chats[contactId] ?: return@update it
        val i = msgs.indexOfFirst { m -> m.fromMe && m.pending }
        if (i < 0) it else it.copy(chats = it.chats + (contactId to msgs.toMutableList().also { l -> l[i] = l[i].copy(pending = false) }))
    }

    fun dismissError() = _ui.update { it.copy(error = null) }
    private fun status(s: String) = _ui.update { it.copy(status = s) }
    private fun error(s: String) = _ui.update { it.copy(error = s) }
    private fun io(block: suspend () -> Unit) { viewModelScope.launch(Dispatchers.IO) { block() } }

    override fun onCleared() { poller?.cancel(); link?.close(); qt?.close() }

    companion object {
        private const val POLL_MS = 3_000L
        private const val MAX_PAGES = 20
        private const val MAX_TEXT_BYTES = 10_000
    }
}

private fun RelayResponse.describe(): String = when (this) {
    is RelayResponse.Error -> reason
    else -> this::class.simpleName ?: "?"
}

/** Mensagem curta e SEGURA para mostrar/registar: nunca inclui conteudo nem chaves. */
private fun Exception.safeMsg(): String = when (this) {
    is QtException -> this::class.simpleName ?: "erro"
    else -> (message ?: this::class.simpleName ?: "erro").take(120)
}
