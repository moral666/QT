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
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.qt_ffi.ChatMessage
import uniffi.qt_ffi.ConversationSummary
import uniffi.qt_ffi.QtClient
import uniffi.qt_ffi.QtException
import uniffi.qt_ffi.ReceiveResult
import uniffi.qt_ffi.RelayResponse
import uniffi.qt_ffi.peerBundleFromRelayBytes
import uniffi.qt_ffi.relayIdOfPublicId

data class UiState(
    val loading: Boolean = true,
    val hasIdentity: Boolean = false,
    val publicId: String = "",
    val relayUrl: String = BuildConfig.RELAY_URL,
    val relayKeyHex: String = BuildConfig.RELAY_PIN,
    val status: String = "",
    val error: String? = null,
    val contacts: List<Contact> = emptyList(),
    /** Conversas com historico (vem do SQLCipher, no Rust), a mais recente primeiro. */
    val conversations: List<ConversationSummary> = emptyList(),
    /** Conversa aberta (contactId) e a janela de mensagens carregada, da MAIS RECENTE para a mais antiga. */
    val openChatId: String? = null,
    val chatMessages: List<ChatMessage> = emptyList(),
    /** Ainda ha mensagens mais antigas por carregar na conversa aberta. */
    val chatHasMore: Boolean = false,
    /** Enviar recibos "Visto" (o utilizador pode desligar). */
    val readReceipts: Boolean = true,
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
    /** Um so envio da outbox de cada vez (declarado ANTES do init, que ja usa a rede). */
    private val flushLock = Mutex()


    init {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val dbFile = getApplication<Application>().noBackupFilesDir.resolve("qt.db")
                val provider = StorageKeyProvider(getApplication(), box, dbFile)
                val key = provider.storageKey()
                val client = try { QtClient.open(dbFile.path, key) } finally { key.fill(0) }
                qt = client
                // Release: relay fixo da build, ignora preferencias antigas. Debug: editavel.
                val url = if (BuildConfig.RELAY_EDITABLE) prefs.getString("relay_url", null) ?: BuildConfig.RELAY_URL else BuildConfig.RELAY_URL
                val pin = if (BuildConfig.RELAY_EDITABLE) prefs.getString("relay_key", null) ?: BuildConfig.RELAY_PIN else BuildConfig.RELAY_PIN
                val has = client.hasIdentity()
                _ui.update {
                    it.copy(
                        loading = false, hasIdentity = has, relayUrl = url, relayKeyHex = pin,
                        publicId = if (has) client.exportPublicIdentity().publicId else "",
                        contacts = contactStore.load(),
                        conversations = client.conversations(),
                        readReceipts = prefs.getBoolean("read_receipts", true),
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
        if (!BuildConfig.RELAY_EDITABLE) return   // release: relay fixo, nunca alteravel
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
            _ui.update { it.copy(contacts = list, error = null) }
            flushOutbox()   // recibos que estavam a espera de saber o endereco deste contacto
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

    // ------------------------------------------------------------------ conversas

    /** Abre uma conversa: carrega as mensagens mais recentes e marca-as como lidas. */
    fun openChat(contactId: String) = io {
        val client = qt ?: return@io
        val page = client.messagesPage(contactId, null, PAGE.toUInt())
        _ui.update { it.copy(openChatId = contactId, chatMessages = page, chatHasMore = page.size == PAGE) }
        markRead(client, contactId)
    }

    fun closeChat() = _ui.update { it.copy(openChatId = null, chatMessages = emptyList(), chatHasMore = false) }

    /** Scroll ate ao topo: carrega mais uma pagina de mensagens antigas (do disco, cifrado). */
    fun loadOlder() = io {
        val client = qt ?: return@io
        val st = _ui.value
        val cid = st.openChatId ?: return@io
        if (!st.chatHasMore) return@io
        val oldest = st.chatMessages.lastOrNull()?.seq ?: return@io
        val older = client.messagesPage(cid, oldest, PAGE.toUInt())
        _ui.update {
            if (it.openChatId != cid) it
            else it.copy(chatMessages = it.chatMessages + older, chatHasMore = older.size == PAGE)
        }
    }

    // ------------------------------------------------- eliminacao (permanente)

    /**
     * ELIMINAR CONVERSA: apaga para sempre as mensagens desta conversa neste
     * aparelho (incluindo as que ainda estavam por enviar). O contacto e a
     * sessao cifrada mantem-se, por isso podem continuar a falar.
     */
    fun deleteConversation(contactId: String) = io {
        val client = qt ?: return@io
        try {
            client.deleteConversation(contactId)
            _ui.update {
                if (it.openChatId == contactId) it.copy(chatMessages = emptyList(), chatHasMore = false) else it
            }
            refreshViews(client)
        } catch (e: Exception) {
            error("Nao foi possivel eliminar a conversa: ${e.safeMsg()}")
        }
    }

    /**
     * APAGAR CONTACTO: apaga para sempre as mensagens, o que estava por enviar,
     * as chaves da sessao cifrada e o proprio contacto. Se voltarem a falar,
     * comeca uma sessao nova (o outro lado adota-a automaticamente).
     */
    fun deleteContact(contactId: String) = io {
        val client = qt ?: return@io
        try {
            // Primeiro o Rust (mensagens + chaves), na mesma transacao; depois a lista.
            client.deleteContact(contactId)
            val list = _ui.value.contacts.filter { it.contactId != contactId }
            contactStore.save(list)
            _ui.update {
                it.copy(
                    contacts = list,
                    openChatId = if (it.openChatId == contactId) null else it.openChatId,
                    chatMessages = if (it.openChatId == contactId) emptyList() else it.chatMessages,
                    chatHasMore = if (it.openChatId == contactId) false else it.chatHasMore,
                )
            }
            refreshViews(client)
        } catch (e: Exception) {
            error("Nao foi possivel apagar o contacto: ${e.safeMsg()}")
        }
    }

    /**
     * APAGAR TODOS OS DADOS: identidade, chaves, contactos, conversas e
     * definicoes deste aparelho. Irreversivel: o teu ID deixa de existir.
     */
    fun wipeAllData() = io {
        val client = qt ?: return@io
        try {
            poller?.cancel()
            link?.close()
            link = null
            client.deleteIdentity()          // secure_delete + VACUUM no Rust
            contactStore.wipe()
            prefs.edit().clear().apply()
            _ui.update {
                UiState(
                    loading = false,
                    hasIdentity = false,
                    relayUrl = BuildConfig.RELAY_URL,
                    relayKeyHex = BuildConfig.RELAY_PIN,
                )
            }
        } catch (e: Exception) {
            error("Nao foi possivel apagar os dados: ${e.safeMsg()}")
        }
    }

    fun setReadReceipts(on: Boolean) {
        prefs.edit().putBoolean("read_receipts", on).apply()
        _ui.update { it.copy(readReceipts = on) }
    }

    private suspend fun markRead(client: QtClient, contactId: String) {
        if (client.markConversationRead(contactId, _ui.value.readReceipts) > 0u) {
            flushOutbox()
        }
        refreshViews(client)
    }

    /** Recarrega a lista de conversas e a janela da conversa aberta (estados ✓/✓✓ e mensagens novas). */
    private fun refreshViews(client: QtClient) {
        val st = _ui.value
        val cid = st.openChatId
        val window = if (cid != null) client.messagesPage(cid, null, maxOf(PAGE, st.chatMessages.size).toUInt()) else null
        _ui.update {
            it.copy(
                conversations = client.conversations(),
                chatMessages = if (window != null && it.openChatId == cid) window else it.chatMessages,
            )
        }
    }

    // ---------------------------------------------------------------------- enviar

    fun send(contact: Contact, text: String) = io {
        val client = qt ?: return@io
        val bytes = text.trim().toByteArray(Charsets.UTF_8)
        if (bytes.isEmpty() || bytes.size > MAX_TEXT_BYTES) { error("Mensagem vazia ou maior que $MAX_TEXT_BYTES bytes"); return@io }
        try {
            ensureSession(client, contact)
            // Numa so transacao no Rust: ratchet avanca + mensagem guardada + envelope na outbox.
            client.sendText(contact.contactId, text.trim())
            refreshViews(client)
            flushOutbox()
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

    /**
     * Envia ao relay tudo o que esta na outbox (mensagens e recibos), pela ordem
     * de criacao. So sai da outbox o que o relay confirmar; o resto fica guardado
     * (cifrado) e volta a ser tentado no proximo ciclo - mesmo depois de reiniciar a app.
     */
    private suspend fun flushOutbox() = flushLock.withLock {
        val client = qt ?: return@withLock
        val l = link ?: return@withLock
        var changed = false
        for (item in client.outbox()) {
            // O relay identifica caixas pela chave de assinatura (do Public ID do contacto).
            val relayId = _ui.value.contacts.firstOrNull { it.contactId == item.contactId }?.relayId ?: continue
            val r = try {
                l.request(client.relayRequestSend(relayId, item.blob))
            } catch (e: Exception) {
                break   // sem rede: tenta no proximo ciclo, mantendo a ordem
            }
            if (r is RelayResponse.Ack) {
                client.outboxSent(item.outboxId)
                changed = true
            }
            // Outra resposta (ex.: fila do destinatario cheia): fica na outbox e segue para o proximo.
        }
        if (changed) refreshViews(client)
    }

    // --------------------------------------------------------------------- receber

    private suspend fun pollOnce() {
        val client = qt ?: return
        val l = link ?: return
        try {
            flushOutbox()
            var received = false
            for (page in 0 until MAX_PAGES) {
                val r = l.request(client.relayRequestFetch())
                val msgs = (r as? RelayResponse.Messages)?.messages ?: break
                if (msgs.isEmpty()) break
                val ackIds = mutableListOf<ULong>()
                for (m in msgs) {
                    try {
                        // Decifrar + guardar no historico + agendar o recibo, tudo numa transacao.
                        when (client.receiveSealed(m.ciphertext)) {
                            is ReceiveResult.NewMessage, is ReceiveResult.StatusChanged -> received = true
                            is ReceiveResult.NoChange -> {}
                        }
                        ackIds += m.id
                    } catch (e: QtException.Storage) {
                        // Problema local: NAO confirmar, o relay volta a entregar mais tarde.
                    } catch (e: QtException) {
                        // Replay (ja processada) ou envelope invalido: nada a recuperar -> confirmar.
                        ackIds += m.id
                    }
                }
                if (ackIds.isNotEmpty()) l.request(client.relayRequestAck(ackIds))   // so DEPOIS de guardar
            }
            if (received) {
                client.prepareDeliveryReceipts()
                val open = _ui.value.openChatId
                if (open != null) markRead(client, open) else refreshViews(client)
                flushOutbox()
            }
            status("Ligado ao relay")
        } catch (e: Exception) {
            status("Sem ligacao ao relay: ${e.safeMsg()}")
        }
    }

    // ------------------------------------------------------------------ utilitarios

    fun dismissError() = _ui.update { it.copy(error = null) }
    private fun status(s: String) = _ui.update { it.copy(status = s) }
    private fun error(s: String) = _ui.update { it.copy(error = s) }
    private fun io(block: suspend () -> Unit) { viewModelScope.launch(Dispatchers.IO) { block() } }

    override fun onCleared() { poller?.cancel(); link?.close(); qt?.close() }

    companion object {
        private const val POLL_MS = 3_000L
        private const val MAX_PAGES = 20
        private const val MAX_TEXT_BYTES = 9_000
        private const val PAGE = 50
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
