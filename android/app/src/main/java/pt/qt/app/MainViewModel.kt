package pt.qt.app

import android.app.Application
import android.net.Uri
import android.os.Build
import android.os.SystemClock
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import java.io.IOException
import javax.crypto.Cipher
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.qt_ffi.AttachmentRef
import uniffi.qt_ffi.AttachmentState
import uniffi.qt_ffi.ChatMessage
import uniffi.qt_ffi.ConversationSummary
import uniffi.qt_ffi.QtClient
import uniffi.qt_ffi.QtException
import uniffi.qt_ffi.ReceiveResult
import uniffi.qt_ffi.RelayDelivered
import uniffi.qt_ffi.RelayResponse
import uniffi.qt_ffi.peerBundleFromRelayBytes
import uniffi.qt_ffi.relayIdOfPublicId
import uniffi.qt_ffi.relayRecipientDeletedReason

data class UiState(
    val loading: Boolean = true,
    val hasIdentity: Boolean = false,
    val publicId: String = "",
    val relayUrl: String = BuildConfig.RELAY_URL,
    val relayKeyHex: String = BuildConfig.RELAY_PIN,
    val status: String = "",
    /** Diagnostico da entrega imediata: vazio quando esta ativa; senao, o motivo (versao alpha). */
    val delivery: String = "",
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
    // ----- bloqueio da app -----
    /** A app esta bloqueada agora (base de dados fechada, nada sensivel em memoria). */
    val locked: Boolean = false,
    /** O bloqueio com codigo esta ativo nas definicoes. */
    val lockEnabled: Boolean = false,
    /** Desbloqueio com impressao digital/rosto/PIN do telemovel ativo. */
    val bioEnabled: Boolean = false,
    /** O telemovel permite esse desbloqueio (Android 11+ com bloqueio de ecra). */
    val bioAvailable: Boolean = false,
    /** Bloquear automaticamente depois de a app estar este tempo em segundo plano. */
    val lockTimeoutMs: Long = DEFAULT_LOCK_TIMEOUT_MS,
    /** A verificar o codigo (Argon2 demora ~1 s). */
    val unlocking: Boolean = false,
    /** Mensagem no ecra de bloqueio (codigo errado, esperar...). */
    val lockMessage: String? = null,
    /** Progresso (0 a 1) dos anexos a enviar/descarregar, por message_id. */
    val transfers: Map<String, Float> = emptyMap(),
    /**
     * Pediu-se para apagar a conta mas nao foi possivel apaga-la no servidor (motivo).
     * Nada foi apagado: a app pergunta se quer tentar de novo ou apagar so deste telemovel.
     */
    val wipeServerFailed: String? = null,
)

const val DEFAULT_LOCK_TIMEOUT_MS = 60_000L

class MainViewModel(app: Application) : AndroidViewModel(app) {
    private val _ui = MutableStateFlow(UiState())
    val ui: StateFlow<UiState> = _ui

    private val prefs = app.getSharedPreferences("qt_prefs", 0)   // so URL do relay e chave PUBLICA (nao secretos)
    private val box = KeystoreBox()
    private val contactStore = ContactStore(app, box)
    private val dbFile: File = app.noBackupFilesDir.resolve("qt.db")
    private val storageKeys = StorageKeyProvider(app, box, dbFile)
    private val bio = BioUnlock(app)
    @Volatile private var qt: QtClient? = null
    @Volatile private var link: RelayLink? = null
    @Volatile private var poller: Job? = null
    /** Segunda ligacao ao relay, so para anexos: um envio grande nao atrasa as mensagens. */
    @Volatile private var fileLink: RelayLink? = null
    /**
     * Terceira ligacao, so para ESPERAR por mensagens (relayRequestWait): o relay responde no
     * instante em que chega uma, em vez de a app perguntar de POLL_MS em POLL_MS. Fica ocupada
     * durante a espera, por isso nao partilha a ligacao dos envios.
     */
    @Volatile private var waitLink: RelayLink? = null
    @Volatile private var transferJob: Job? = null
    /** "Ha anexos para enviar/descarregar": acorda o ciclo de transferencias. */
    private val transferKick = Channel<Unit>(Channel.CONFLATED)
    /** Chave da lista de contactos; so existe em memoria com a app desbloqueada. */
    @Volatile private var contactsKey: ByteArray? = null
    /** Chave do armazenamento a espera do BiometricPrompt para ativar a biometria. */
    @Volatile private var pendingBioKey: ByteArray? = null
    /** Quando a app foi para segundo plano (relogio desde o arranque do telemovel). */
    @Volatile private var hiddenAt: Long? = null
    /** Fecha as ligacoes ao relay depois de BACKGROUND_GRACE_MS em segundo plano (ver onAppHidden). */
    @Volatile private var pauseJob: Job? = null
    /** As ligacoes foram fechadas por a app estar em segundo plano: religar ao voltar. */
    @Volatile private var linksPaused = false
    /** A app esta no ecra. Com [pauseLock]: fechar e religar nunca se cruzam. */
    @Volatile private var visible = true
    private val pauseLock = Any()
    /** Ha um BiometricPrompt aberto: sair/voltar a app nao conta para o bloqueio. */
    @Volatile var authInProgress = false
    /** Para que serve o BiometricPrompt aberto (no ViewModel para sobreviver a rodar o ecra). */
    @Volatile var bioPurpose: BioPurpose? = null
    /** Um so envio da outbox de cada vez (declarado ANTES do init, que ja usa a rede). */
    private val flushLock = Mutex()
    private val bioChannel = Channel<BioRequest>(Channel.BUFFERED)
    /** Pedidos de autenticacao do sistema; a Activity mostra o BiometricPrompt. */
    val bioRequests: Flow<BioRequest> = bioChannel.receiveAsFlow()


    init {
        ImagePrep.cleanCache(app)
        io { openStorage() }
    }

    /** Abre a base de dados, ou mostra o ecra de bloqueio se o codigo estiver ativo. */
    private suspend fun openStorage() {
        try {
            if (storageKeys.lockEnabled()) {
                showLocked()
                return
            }
            openClient(storageKeys.storageKey())
        } catch (e: AppLockedException) {
            showLocked()
        } catch (e: StorageKeyUnavailableException) {
            // Nunca se cria uma chave nova por cima: o utilizador decide se apaga os dados locais.
            _ui.update { it.copy(loading = false, error = "Chave do armazenamento seguro indisponivel: ${e.message}") }
        } catch (e: Exception) {
            _ui.update { it.copy(loading = false, error = "Nao foi possivel abrir o armazenamento seguro: ${e.safeMsg()}") }
        }
    }

    /** Abre o QtClient com a chave do armazenamento. Apaga `key` da memoria no fim. */
    private suspend fun openClient(key: ByteArray) {
        val client = try {
            contactsKey?.fill(0)
            contactsKey = contactsKeyFrom(key)
            QtClient.open(dbFile.path, key)
        } finally {
            key.fill(0)
        }
        qt = client
        val ck = contactsKey!!
        // Lista de contactos no formato antigo: passa ja a ter a segunda camada.
        if (contactStore.isLegacy()) {
            val old = contactStore.load(ck)
            if (old.isNotEmpty()) runCatching { contactStore.save(old, ck) }
        }
        // Release: relay fixo da build, ignora preferencias antigas. Debug: editavel.
        val url = if (BuildConfig.RELAY_EDITABLE) prefs.getString("relay_url", null) ?: BuildConfig.RELAY_URL else BuildConfig.RELAY_URL
        val pin = if (BuildConfig.RELAY_EDITABLE) prefs.getString("relay_key", null) ?: BuildConfig.RELAY_PIN else BuildConfig.RELAY_PIN
        val has = client.hasIdentity()
        _ui.update {
            it.copy(
                loading = false, hasIdentity = has, relayUrl = url, relayKeyHex = pin,
                publicId = if (has) client.exportPublicIdentity().publicId else "",
                contacts = contactStore.load(ck),
                conversations = client.conversations(),
                readReceipts = prefs.getBoolean("read_receipts", true),
                locked = false, unlocking = false, lockMessage = null,
                lockEnabled = storageKeys.lockEnabled(),
                bioEnabled = bio.isEnabled(), bioAvailable = bio.isAvailable(),
                lockTimeoutMs = prefs.getLong("lock_timeout_ms", DEFAULT_LOCK_TIMEOUT_MS),
            )
        }
        if (has) startLink()
    }

    /** Ecra de bloqueio, sem nenhum dado das conversas no estado. */
    private fun showLocked(message: String? = null) {
        _ui.update {
            UiState(
                loading = false, locked = true, lockEnabled = true,
                bioEnabled = bio.isEnabled(), bioAvailable = bio.isAvailable(),
                lockTimeoutMs = prefs.getLong("lock_timeout_ms", DEFAULT_LOCK_TIMEOUT_MS),
                lockMessage = message,
            )
        }
    }

    /** Fecha a base de dados e a ligacao ao relay; apaga as chaves da memoria. */
    private fun closeSession() {
        poller?.cancel()
        poller = null
        transferJob?.cancel()
        transferJob = null
        link?.shutdown()
        link = null
        fileLink?.shutdown()
        fileLink = null
        waitLink?.shutdown()
        waitLink = null
        // Imagens decifradas que estavam em memoria para o ecra.
        SealerImages.clearMemory()
        val c = qt
        qt = null
        c?.close()
        contactsKey?.fill(0)
        contactsKey = null
    }

    // -------------------------------------------------------------- bloqueio da app

    /** Bloqueia ja (so se o bloqueio estiver ativo). */
    fun lockNow() {
        if (!_ui.value.lockEnabled || _ui.value.locked) return
        closeSession()
        showLocked()
    }

    /**
     * A app saiu do ecra (Activity.onStop). Conta o tempo para o bloqueio automatico e, ao fim de
     * BACKGROUND_GRACE_MS, fecha as ligacoes ao relay: em segundo plano a app nao mostra nada
     * (ainda nao ha notificacoes), e cada ligacao aberta acordava o radio de 25 em 25 s (ping e
     * pedido de espera), gastando bateria sem proveito. As mensagens ficam no relay ate voltar.
     */
    fun onAppHidden() {
        if (authInProgress) return
        hiddenAt = SystemClock.elapsedRealtime()
        synchronized(pauseLock) { visible = false }
        pauseJob?.cancel()
        pauseJob = viewModelScope.launch(Dispatchers.IO) {
            delay(BACKGROUND_GRACE_MS)
            // Anexos a meio (enviar ou descarregar): deixa-os acabar, ate BACKGROUND_TRANSFER_MAX_MS.
            val until = SystemClock.elapsedRealtime() + BACKGROUND_TRANSFER_MAX_MS
            while (_ui.value.transfers.isNotEmpty() && SystemClock.elapsedRealtime() < until) delay(5_000L)
            pauseLinks()
        }
    }

    /**
     * A app voltou ao ecra (Activity.onStart): bloqueia se passou o tempo escolhido; senao,
     * religa ao relay se as ligacoes tinham sido fechadas em segundo plano.
     */
    fun onAppVisible() {
        synchronized(pauseLock) {
            visible = true
            pauseJob?.cancel()
            pauseJob = null
        }
        val t = hiddenAt
        hiddenAt = null
        if (t != null) {
            // Voltar do leitor de QR: mais AutoLockPause.GRACE_MS antes de bloquear.
            val grace = if (AutoLockPause.active) AutoLockPause.GRACE_MS else 0L
            AutoLockPause.active = false
            val st = _ui.value
            if (!authInProgress && st.lockEnabled && !st.locked && SystemClock.elapsedRealtime() - t - grace >= st.lockTimeoutMs) {
                lockNow()
            }
        }
        resumeLinks()
    }

    /** Fecha as ligacoes ao relay e para a rececao e as transferencias (app em segundo plano). */
    private fun pauseLinks() {
        synchronized(pauseLock) {
            // Voltou ao ecra entretanto; ou bloqueada, sem identidade ou ja fechadas.
            if (visible || qt == null || link == null) return
            poller?.cancel()
            poller = null
            transferJob?.cancel()
            transferJob = null
            link?.shutdown()
            link = null
            fileLink?.shutdown()
            fileLink = null
            waitLink?.shutdown()
            waitLink = null
            linksPaused = true
        }
        Log.d(TAG, "segundo plano: ligacoes ao relay fechadas")
    }

    /** Religa ao relay (e vai buscar o que chegou entretanto) se as ligacoes estavam fechadas. */
    private fun resumeLinks() {
        synchronized(pauseLock) {
            if (!linksPaused) return
            linksPaused = false
        }
        // Bloqueada: ao desbloquear, openClient religa.
        if (_ui.value.locked) return
        io { if (qt?.hasIdentity() == true && link == null) startLink() }
    }

    fun unlockWithCode(code: String) = io {
        val wait = lockWaitMs()
        if (wait > 0) {
            _ui.update { it.copy(lockMessage = waitMessage(wait)) }
            return@io
        }
        _ui.update { it.copy(unlocking = true, lockMessage = null) }
        try {
            val key = storageKeys.storageKeyWithCode(code)
            resetFails()
            openClient(key)
        } catch (e: QtException.Crypto) {
            val wait2 = registerFail()
            _ui.update { it.copy(unlocking = false, lockMessage = "Código errado." + (if (wait2 > 0) " " + waitMessage(wait2) else "")) }
        } catch (e: QtException.InvalidInput) {
            _ui.update { it.copy(unlocking = false, lockMessage = "O código tem pelo menos ${minCodeLen()} dígitos.") }
        } catch (e: Exception) {
            _ui.update { it.copy(unlocking = false, lockMessage = "Não foi possível desbloquear: ${e.safeMsg()}") }
        }
    }

    /** Pede o BiometricPrompt para desbloquear. */
    fun requestBioUnlock() = io {
        if (!bio.isEnabled()) return@io
        // Verificacao explicita da versao (o lint do Android exige-a junto da chamada).
        val cipher = if (Build.VERSION.SDK_INT >= 30) bio.cipherForUnlock() else null
        if (cipher == null) {
            _ui.update {
                it.copy(
                    bioEnabled = false,
                    lockMessage = "A impressão digital ou o bloqueio do ecrã mudaram. Usa o código e volta a ativar a biometria.",
                )
            }
            return@io
        }
        bioChannel.send(BioRequest(BioPurpose.UNLOCK, cipher))
    }

    /** O BiometricPrompt autorizou a Cipher de desbloqueio. */
    fun unlockWithBio(cipher: Cipher) = io {
        _ui.update { it.copy(unlocking = true, lockMessage = null) }
        try {
            val key = bio.open(cipher)
            resetFails()
            openClient(key)
        } catch (e: Exception) {
            _ui.update { it.copy(unlocking = false, lockMessage = "Não foi possível desbloquear assim. Usa o código.") }
        }
    }

    /** Erro no BiometricPrompt que o utilizador deve ver (cancelar nao conta). */
    fun bioMessage(msg: String) {
        if (_ui.value.locked) _ui.update { it.copy(lockMessage = msg) } else error(msg)
    }

    fun enableLock(code: String) = io {
        try {
            storageKeys.enableLock(code)
            resetFails()
            _ui.update { it.copy(lockEnabled = true, bioAvailable = bio.isAvailable()) }
        } catch (e: QtException.InvalidInput) {
            error("O código tem de ter pelo menos ${minCodeLen()} dígitos.")
        } catch (e: Exception) {
            error("Não foi possível ativar o bloqueio: ${e.safeMsg()}")
        }
    }

    fun disableLock(code: String) = io {
        if (lockWaitMs() > 0) { error(waitMessage(lockWaitMs())); return@io }
        try {
            storageKeys.disableLock(code)
            bio.disable()
            resetFails()
            _ui.update { it.copy(lockEnabled = false, bioEnabled = false) }
        } catch (e: QtException.Crypto) {
            registerFail()
            error("Código errado.")
        } catch (e: Exception) {
            error("Não foi possível desativar o bloqueio: ${e.safeMsg()}")
        }
    }

    fun changeCode(oldCode: String, newCode: String) = io {
        if (lockWaitMs() > 0) { error(waitMessage(lockWaitMs())); return@io }
        try {
            storageKeys.changeCode(oldCode, newCode)
            resetFails()
            error("Código alterado.")
        } catch (e: QtException.Crypto) {
            registerFail()
            error("O código atual está errado.")
        } catch (e: QtException.InvalidInput) {
            error("O código tem de ter pelo menos ${minCodeLen()} dígitos.")
        } catch (e: Exception) {
            error("Não foi possível mudar o código: ${e.safeMsg()}")
        }
    }

    /** Ativar a biometria: confirma o codigo e pede o BiometricPrompt. */
    fun startBioEnable(code: String) = io {
        if (Build.VERSION.SDK_INT < 30 || !bio.isAvailable()) {
            error("Este telemóvel não permite desbloquear assim (precisa de Android 11 ou mais recente e de bloqueio de ecrã).")
            return@io
        }
        if (lockWaitMs() > 0) { error(waitMessage(lockWaitMs())); return@io }
        try {
            val key = storageKeys.storageKeyWithCode(code)
            resetFails()
            pendingBioKey?.fill(0)
            pendingBioKey = key
            if (Build.VERSION.SDK_INT >= 30) bioChannel.send(BioRequest(BioPurpose.ENABLE, bio.cipherForEnable()))
        } catch (e: QtException.Crypto) {
            registerFail()
            error("Código errado.")
        } catch (e: Exception) {
            cancelBioEnable()
            error("Não foi possível ativar: ${e.safeMsg()}")
        }
    }

    /** O BiometricPrompt autorizou a Cipher para guardar a copia da chave. */
    fun finishBioEnable(cipher: Cipher) = io {
        val key = pendingBioKey ?: return@io
        try {
            bio.save(cipher, key)
            _ui.update { it.copy(bioEnabled = true) }
        } catch (e: Exception) {
            bio.disable()
            error("Não foi possível ativar: ${e.safeMsg()}")
        } finally {
            key.fill(0)
            pendingBioKey = null
        }
    }

    fun cancelBioEnable() {
        pendingBioKey?.fill(0)
        pendingBioKey = null
        if (!_ui.value.bioEnabled) bio.disable()
    }

    fun disableBio() {
        bio.disable()
        _ui.update { it.copy(bioEnabled = false) }
    }

    fun setLockTimeout(ms: Long) {
        prefs.edit().putLong("lock_timeout_ms", ms).apply()
        _ui.update { it.copy(lockTimeoutMs = ms) }
    }

    /**
     * Esqueci-me do codigo: sem ele a base de dados nao abre, por isso so resta
     * apagar tudo e comecar de novo (os ficheiros estao cifrados e sao apagados).
     */
    fun wipeLockedData() = io { resetStorage() }

    /** Apaga a base de dados, as chaves, os contactos e as definicoes; abre um armazenamento novo. */
    private suspend fun resetStorage() {
        closeSession()
        cancelBioEnable()
        bio.disable()
        storageKeys.deleteKeyFile()
        for (suffix in listOf("", "-wal", "-shm", "-journal")) File(dbFile.path + suffix).delete()
        // Imagens/GIFs (cifrados com chaves que estavam na base de dados apagada).
        File(dbFile.path + ".anexos").deleteRecursively()
        contactStore.wipe()
        prefs.edit().clear().apply()
        _ui.update { UiState(loading = true) }
        openStorage()
    }

    // Tentativas erradas: depois de 5, espera 30 s, 1 min, 2 min... ate 15 min.
    private fun lockWaitMs(): Long = (prefs.getLong("lock_until", 0L) - System.currentTimeMillis()).coerceAtLeast(0L)

    private fun registerFail(): Long {
        val n = prefs.getInt("lock_fails", 0) + 1
        val wait = if (n < 5) 0L else (30_000L shl (n - 5).coerceAtMost(5)).coerceAtMost(900_000L)
        prefs.edit().putInt("lock_fails", n).putLong("lock_until", System.currentTimeMillis() + wait).apply()
        return wait
    }

    private fun resetFails() {
        prefs.edit().remove("lock_fails").remove("lock_until").apply()
    }

    private fun waitMessage(ms: Long) = "Demasiadas tentativas. Tenta de novo daqui a ${(ms + 999) / 1000} s."

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
        link?.shutdown()
        link = RelayLink(client, _ui.value.relayUrl, _ui.value.relayKeyHex.ifBlank { null })
        transferJob?.cancel()
        fileLink?.shutdown()
        fileLink = RelayLink(client, _ui.value.relayUrl, _ui.value.relayKeyHex.ifBlank { null })
        waitLink?.shutdown()
        waitLink = RelayLink(client, _ui.value.relayUrl, _ui.value.relayKeyHex.ifBlank { null })
        status("A ligar ao relay…")
        try {
            // Publicar (ou re-publicar) o bundle publico para que outros nos possam contactar.
            val r = link!!.request(client.relayRequestRegisterBundle(client.publishableBundle()))
            if (r is RelayResponse.Ack) status("Ligado ao relay") else status("Relay recusou o bundle: ${r.describe()}")
        } catch (e: Exception) {
            status("Sem ligacao ao relay: ${e.safeMsg()}")
        }
        poller = viewModelScope.launch(Dispatchers.IO) { receiveLoop(client) }
        transferJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) {
                val retry = try {
                    runTransfers()
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    true
                }
                // Depois de uma falha de rede tenta de novo daqui a 15 s (ou antes, se houver novidades).
                if (retry) withTimeoutOrNull(RETRY_MS) { transferKick.receive() } else transferKick.receive()
            }
        }
        transferKick.trySend(Unit)
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
            contactStore.save(list, contactsKey ?: return@io)
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
            contactStore.save(list, contactsKey ?: return@io)
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
    /**
     * Apaga a conta: primeiro no relay (bundle de chaves publicas e mensagens em espera),
     * depois tudo deste telemovel. Se nao for possivel apagar no relay (sem rede, relay
     * antigo), nao apaga nada e pergunta ([UiState.wipeServerFailed]). Com [localOnly], o
     * utilizador aceitou apagar so deste telemovel: o que ficou no relay apaga-se sozinho
     * (o bundle ao fim de 90 dias, as mensagens ao fim de 30).
     */
    fun wipeAllData(localOnly: Boolean = false) = io {
        val client = qt ?: return@io
        _ui.update { it.copy(wipeServerFailed = null) }
        if (!localOnly) {
            val problem = deleteOnRelay(client)
            if (problem != null) {
                _ui.update { it.copy(wipeServerFailed = problem) }
                return@io
            }
        }
        try {
            poller?.cancel()
            link?.shutdown()
            link = null
            transferJob?.cancel()
            fileLink?.shutdown()
            fileLink = null
            waitLink?.shutdown()
            waitLink = null
            client.deleteIdentity()          // secure_delete + VACUUM no Rust (e os ficheiros dos anexos)
            // Depois apaga os ficheiros e as chaves (incluindo o bloqueio) e abre um armazenamento novo.
            resetStorage()
        } catch (e: Exception) {
            error("Nao foi possivel apagar os dados: ${e.safeMsg()}")
        }
    }

    fun dismissWipeServerFailed() = _ui.update { it.copy(wipeServerFailed = null) }

    /** Pede ao relay para apagar a conta. `null` se apagou; senao, o motivo (para mostrar). */
    private suspend fun deleteOnRelay(client: QtClient): String? {
        val l = link ?: return "sem ligação ao servidor"
        return try {
            when (val r = l.request(client.relayRequestDeleteAccount())) {
                is RelayResponse.Ack -> null
                is RelayResponse.Error ->
                    if (r.reason == RELAY_UNKNOWN_REQUEST) "o servidor ainda não tem esta função" else "o servidor respondeu: ${r.reason}"
                else -> "resposta inesperada do servidor"
            }
        } catch (e: Exception) {
            "sem ligação ao servidor"
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

    // ---------------------------------------------------------------- imagens/GIFs

    /**
     * Envia uma imagem/GIF. `uri` vem da galeria ou e uma copia do que o teclado
     * mandou (`keyboardCopy`, apagada no fim). O ficheiro e preparado no telemovel,
     * cifrado pelo Rust e enviado em segundo plano (ver `runTransfers`).
     */
    fun sendImage(contact: Contact, uri: Uri, original: Boolean, keyboardCopy: File? = null) = io {
        val client = qt ?: return@io
        val prepared = try {
            ImagePrep.prepare(getApplication<Application>(), uri, original, keepAsIs = keyboardCopy != null)
        } catch (e: ImageTooBigException) {
            error("A imagem tem mais de 25 MB.")
            return@io
        } catch (e: Exception) {
            error("Não foi possível ler esta imagem.")
            return@io
        } finally {
            keyboardCopy?.delete()
        }
        try {
            ensureSession(client, contact)
            client.attachmentCreate(contact.contactId, prepared.file.path, prepared.mime, prepared.width.toUInt(), prepared.height.toUInt())
            refreshViews(client)
            transferKick.trySend(Unit)
        } catch (e: Exception) {
            error("Não foi possível enviar a imagem: ${e.safeMsg()}")
        } finally {
            prepared.file.delete()
        }
    }

    /** A imagem decifrada, so em memoria, para mostrar no ecra. */
    suspend fun attachmentBytes(contactId: String, messageId: String): ByteArray? = withContext(Dispatchers.IO) {
        try {
            qt?.attachmentRead(contactId, messageId)
        } catch (e: Exception) {
            null
        }
    }

    private fun setProgress(messageId: String, p: Float?) = _ui.update {
        it.copy(transfers = if (p == null) it.transfers - messageId else it.transfers + (messageId to p))
    }

    /**
     * Envia e descarrega os anexos pendentes (tambem os que ficaram a meio antes de a
     * app fechar). Devolve true se algo falhou por rede e deve ser tentado de novo.
     */
    private suspend fun runTransfers(): Boolean {
        val client = qt ?: return false
        val l = fileLink ?: return false
        for (r in client.attachmentsPendingUpload()) {
            val contact = _ui.value.contacts.firstOrNull { it.contactId == r.contactId } ?: continue
            try {
                upload(client, l, r, contact)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                setProgress(r.messageId, null)
                return true
            }
        }
        for (r in client.attachmentsPendingDownload()) {
            try {
                download(client, l, r)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                setProgress(r.messageId, null)
                return true
            }
        }
        return false
    }

    private suspend fun upload(client: QtClient, l: RelayLink, r: AttachmentRef, contact: Contact) {
        val cid = r.contactId
        val mid = r.messageId
        val total = client.message(cid, mid)?.attachment?.transferSize ?: return
        var start = 0uL
        if (client.attachmentBlobId(cid, mid) == null) {
            when (val resp = l.request(client.blobCreateRequest(cid, mid))) {
                is RelayResponse.BlobCreated -> client.attachmentSetBlob(cid, mid, resp.blobId, resp.token)
                is RelayResponse.Error -> {
                    client.attachmentFailed(cid, mid, false)
                    refreshViews(client)
                    error(relayRefusal(resp.reason))
                    return
                }
                else -> throw IOException("resposta inesperada do relay")
            }
        } else {
            when (val resp = l.request(client.blobInfoRequest(cid, mid))) {
                is RelayResponse.BlobStatus -> start = resp.received - resp.received % CHUNK
                is RelayResponse.BlobNotFound -> {
                    // Passaram mais de 24 h sem conseguir acabar o envio.
                    client.attachmentFailed(cid, mid, false)
                    refreshViews(client)
                    return
                }
                else -> throw IOException("resposta inesperada do relay")
            }
        }
        var offset = start
        while (offset < total) {
            val batch = generateSequence(offset) { it + CHUNK }.takeWhile { it < total }.take(WINDOW).toList()
            val answers = l.requestMany(batch.map { client.blobPutRequest(cid, mid, it) })
            if (answers.any { it !is RelayResponse.BlobStatus }) throw IOException("bloco recusado")
            offset = batch.last() + CHUNK
            setProgress(mid, (minOf(offset, total).toDouble() / total.toDouble()).toFloat())
        }
        ensureSession(client, contact)
        client.attachmentUploaded(cid, mid)   // a mensagem com a chave entra na outbox
        setProgress(mid, null)
        refreshViews(client)
        flushOutbox()
    }

    private suspend fun download(client: QtClient, l: RelayLink, r: AttachmentRef) {
        val cid = r.contactId
        val mid = r.messageId
        var have = client.attachmentDownloadedBytes(cid, mid)
        var total: ULong? = null
        while (true) {
            val t = total
            if (t != null && have >= t) break
            val offsets = if (t == null) listOf(have) else generateSequence(have) { it + CHUNK }.takeWhile { it < t }.take(WINDOW).toList()
            for (resp in l.requestMany(offsets.map { client.blobGetRequest(cid, mid, it) })) {
                when (resp) {
                    is RelayResponse.BlobData -> {
                        total = resp.size
                        have = client.attachmentWriteChunk(cid, mid, resp.offset, resp.data)
                    }
                    is RelayResponse.BlobNotFound -> {
                        client.attachmentFailed(cid, mid, true)   // expirou (24 h)
                        setProgress(mid, null)
                        refreshViews(client)
                        return
                    }
                    else -> throw IOException("resposta inesperada do relay")
                }
            }
            val now = total
            if (now != null && now > 0uL) setProgress(mid, (have.toDouble() / now.toDouble()).toFloat())
        }
        val done = client.attachmentDownloadFinished(cid, mid)
        setProgress(mid, null)
        refreshViews(client)
        // Ja esta no telemovel (e verificado): o relay pode apagar a copia dele.
        if (done.attachment?.state == AttachmentState.READY) {
            runCatching { l.request(client.blobDeleteRequest(cid, mid)) }
        }
    }

    private fun relayRefusal(reason: String): String = when {
        reason.contains("invalida") || reason.contains("desativados") ->
            "O servidor ainda não aceita imagens (tem de ser atualizado)."
        reason.contains("limite") -> "Atingiste o limite diário de imagens. Tenta amanhã."
        reason.contains("espaco") -> "O servidor está sem espaço para imagens. Tenta mais tarde."
        else -> "Não foi possível enviar a imagem: $reason"
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
    /** Contactos que ja se avisou nesta sessao que apagaram a conta. */
    private val deletedNotified = mutableSetOf<String>()

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
            } else if (r is RelayResponse.Error && r.reason == relayRecipientDeletedReason()) {
                // O contacto apagou a conta: avisa uma vez por sessao (a mensagem fica por enviar).
                if (deletedNotified.add(item.contactId)) {
                    val name = _ui.value.contacts.firstOrNull { it.contactId == item.contactId }?.nickname ?: "Este contacto"
                    error("$name apagou a conta SEALER: as mensagens para este contacto já não são entregues.")
                }
            }
            // Outra resposta (ex.: fila do destinatario cheia): fica na outbox e segue para o proximo.
        }
        if (changed) refreshViews(client)
    }

    // --------------------------------------------------------------------- receber

    /**
     * Recebe mensagens enquanto a sessao estiver aberta.
     *
     * Com um relay que suporta entrega imediata, a app fica a espera numa ligacao propria
     * (waitLink) e o relay responde no instante em que chega uma mensagem (ou ao fim de
     * WAIT_SECONDS, e volta-se a esperar). Um relay antigo responde Error ao pedido de espera:
     * entao volta-se ao metodo antigo, perguntar de POLL_MS em POLL_MS. Sem rede, tambem.
     */
    private suspend fun receiveLoop(client: QtClient) {
        // Se o relay disser que nao conhece o pedido de espera (relay antigo), pergunta-se de
        // POLL_MS em POLL_MS, mas volta-se a tentar a espera de WAIT_RETRY_MS em WAIT_RETRY_MS:
        // o relay pode ter sido atualizado entretanto (nunca desistir para sempre).
        var waitBlockedUntil = 0L
        var first: List<RelayDelivered>? = null
        while (currentCoroutineContext().isActive) {
            pollOnce(first)
            first = null
            val w = waitLink
            if (w == null || SystemClock.elapsedRealtime() < waitBlockedUntil) { delay(POLL_MS); continue }
            try {
                val t0 = SystemClock.elapsedRealtime()
                when (val r = w.request(client.relayRequestWait(WAIT_SECONDS.toUInt()), WAIT_TIMEOUT_MS)) {
                    is RelayResponse.Messages -> {
                        first = r.messages.ifEmpty { null }
                        delivery("")
                        if (first != null) Log.d(TAG, "espera: ${r.messages.size} mensagem(ns) ao fim de ${SystemClock.elapsedRealtime() - t0} ms")
                    }
                    is RelayResponse.Error -> {
                        if (r.reason == RELAY_UNKNOWN_REQUEST) {
                            waitBlockedUntil = SystemClock.elapsedRealtime() + WAIT_RETRY_MS
                            delivery("Entrega imediata: o relay não a suporta; a verificar de 3 em 3 s")
                        } else {
                            delivery("Entrega imediata: erro do relay (${r.reason})")
                            delay(POLL_MS)
                        }
                        Log.w(TAG, "espera recusada pelo relay: ${r.reason}")
                    }
                    else -> {
                        delivery("Entrega imediata: resposta inesperada (${r.describe()})")
                        Log.w(TAG, "espera: resposta inesperada ${r.describe()}")
                        delay(POLL_MS)
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Sem rede ou relay em baixo: tenta de novo (e pergunta entretanto).
                delivery("Entrega imediata falhou: ${e.safeMsg()}")
                Log.w(TAG, "espera falhou: ${e.javaClass.simpleName}: ${e.safeMsg()}")
                delay(POLL_MS)
            }
        }
    }

    /**
     * Busca, guarda e confirma as mensagens em fila. `first`: mensagens que ja chegaram pela
     * ligacao de espera; sao tratadas logo (antes de enviar a outbox), para entregar depressa.
     */
    private suspend fun pollOnce(first: List<RelayDelivered>? = null) {
        val client = qt ?: return
        val l = link ?: return
        try {
            if (first == null) flushOutbox()
            var received = false
            var pending = first
            for (page in 0 until MAX_PAGES) {
                val msgs = pending ?: (l.request(client.relayRequestFetch()) as? RelayResponse.Messages)?.messages ?: break
                pending = null
                if (msgs.isEmpty()) break
                val ackIds = mutableListOf<ULong>()
                var gotPage = false
                for (m in msgs) {
                    try {
                        // Decifrar + guardar no historico + agendar o recibo, tudo numa transacao.
                        when (client.receiveSealed(m.ciphertext)) {
                            is ReceiveResult.NewMessage, is ReceiveResult.StatusChanged -> { received = true; gotPage = true }
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
                // Mostra ja: as mensagens estao guardadas no telemovel. A confirmacao ao relay e o
                // pedido da pagina seguinte (duas idas a rede) vem depois, sem atrasar o ecra.
                if (gotPage) refreshViews(client)
                if (ackIds.isNotEmpty()) l.request(client.relayRequestAck(ackIds))   // so DEPOIS de guardar
            }
            if (received) {
                transferKick.trySend(Unit)   // imagens recebidas: descarregar ja
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
    private fun delivery(s: String) { if (_ui.value.delivery != s) _ui.update { it.copy(delivery = s) } }
    private fun error(s: String) = _ui.update { it.copy(error = s) }
    /**
     * Trabalho em segundo plano. Qualquer erro e apanhado: ao bloquear, a base de
     * dados fecha e uma operacao que ja estava a meio pode falhar; isso nao pode
     * fechar a app.
     */
    private fun io(block: suspend () -> Unit) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                block()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (qt != null) error("Erro: ${e.safeMsg()}")
            }
        }
    }

    override fun onCleared() {
        closeSession()
        pendingBioKey?.fill(0)
    }

    companion object {
        /** Sem entrega imediata (relay antigo ou sem rede): perguntar de 3 em 3 s. */
        private const val POLL_MS = 3_000L
        /** Em segundo plano ha mais do que isto: fecha as ligacoes ao relay (poupa bateria). */
        private const val BACKGROUND_GRACE_MS = 30_000L
        /** Maximo que se espera por anexos a meio antes de fechar, em segundo plano. */
        private const val BACKGROUND_TRANSFER_MAX_MS = 5 * 60_000L
        /** Espera maxima de cada pedido de espera (o relay limita a 25 s). */
        private const val WAIT_SECONDS = 25
        /** Tempo pela resposta a um pedido de espera: a espera mais margem para a rede. */
        private const val WAIT_TIMEOUT_MS = 35_000L
        /** Com um relay sem entrega imediata, voltar a tentar a espera ao fim deste tempo. */
        private const val WAIT_RETRY_MS = 5 * 60_000L
        /** Etiqueta dos registos (adb logcat -s SealerRelay). Nunca regista conteudo nem chaves. */
        private const val TAG = "SealerRelay"
        /** Resposta de um relay a um pedido que nao conhece (relay sem entrega imediata). */
        private const val RELAY_UNKNOWN_REQUEST = "mensagem invalida"
        private const val MAX_PAGES = 20
        private const val MAX_TEXT_BYTES = 9_000
        private const val PAGE = 50
        /** Bytes cifrados por bloco de anexo (igual ao Rust e ao relay). */
        private const val CHUNK = 40_960uL
        /** Blocos em transito ao mesmo tempo (~320 KB). */
        private const val WINDOW = 8
        private const val RETRY_MS = 15_000L
        fun minCodeLen(): Int = uniffi.qt_ffi.appLockMinCodeLen().toInt()
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
