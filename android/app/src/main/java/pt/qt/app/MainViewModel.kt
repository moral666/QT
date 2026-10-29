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
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.qt_ffi.AttachmentRef
import uniffi.qt_ffi.CallAudio
import uniffi.qt_ffi.CallEndReason
import uniffi.qt_ffi.CallEvent
import uniffi.qt_ffi.VideoResponse
import uniffi.qt_ffi.AttachmentState
import uniffi.qt_ffi.ChatMessage
import uniffi.qt_ffi.ConversationSummary
import uniffi.qt_ffi.GroupInfo
import uniffi.qt_ffi.GroupMemberInput
import uniffi.qt_ffi.QtClient
import uniffi.qt_ffi.QtException
import uniffi.qt_ffi.ReceiveResult
import uniffi.qt_ffi.RelayDelivered
import uniffi.qt_ffi.RelayResponse
import uniffi.qt_ffi.groupIdOfConversation
import uniffi.qt_ffi.peerBundleFromRelayBytes
import uniffi.qt_ffi.presenceHeartbeatMs
import uniffi.qt_ffi.relayIdOfPublicId
import uniffi.qt_ffi.relayPremiumRequiredReason
import uniffi.qt_ffi.relayElsewhereName
import uniffi.qt_ffi.relayNoNewAccountsReason
import uniffi.qt_ffi.relayRecipientDeletedReason
import uniffi.qt_ffi.BackupContact

data class UiState(
    val loading: Boolean = true,
    /** Diagnostico da ultima chamada (so com `CallDiag.enabled`), para "Copiar diagnostico". */
    val lastCallDiag: String? = null,
    val hasIdentity: Boolean = false,
    val publicId: String = "",
    val relayUrl: String = BuildConfig.RELAY_URL,
    val relayKeyHex: String = BuildConfig.RELAY_PIN,
    val status: String = "",
    /**
     * Diagnostico da entrega imediata: vazio quando esta ativa; senao, o motivo. Ja nao e
     * mostrado na app (como `status`): fica para depuracao.
     */
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
    /** Notificacoes com a app fechada (interruptor "Notificações"). */
    val pushOn: Boolean = true,
    /** Ja se pediu a permissao de notificacoes (Android 13+) uma vez. */
    val pushAsked: Boolean = false,
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
    /** Contactos bloqueados (contactId): as mensagens deles sao descartadas ao chegar. */
    val blocked: Set<String> = emptySet(),
    /** Convites: quem me escreveu sem estar nos contactos e mandou o seu ID (contactId). */
    val invites: Set<String> = emptySet(),
    /** Aceitou os termos de utilizacao em vigor (TERMS_VERSION). */
    val termsAccepted: Boolean = true,
    /** Grupos (ate 10 pessoas). A conversa de cada um tem o id `conversationId` ("g:..."). */
    val groups: List<GroupInfo> = emptyList(),
    /** Mostrar aos contactos quando estou online (e ver o deles). */
    val presenceOn: Boolean = true,
    /** Contactos com a app aberta agora (contactId). So em memoria. */
    val online: Set<String> = emptySet(),
    /** Chamada de voz a decorrer (a chamar, a tocar, atendida ou acabada de terminar). */
    val call: CallUi? = null,
    /** Contactos com video (o botao da videochamada so aparece para eles). Vem do Rust. */
    val videoContacts: Set<String> = emptySet(),
    /** "Atender" carregado na notificacao: o ecra atende (pedindo o microfone, se faltar). */
    val answerNow: String? = null,
    /** Fotos de perfil dos contactos (contactId), ja prontas a mostrar. So em memoria. */
    val photos: Map<String, androidx.compose.ui.graphics.ImageBitmap> = emptyMap(),
    /** A minha foto de perfil (null = sem foto). */
    val myPhoto: androidx.compose.ui.graphics.ImageBitmap? = null,
    /** Badges dos contactos (contactId -> numero, so os que tem). Verificados no Rust. */
    val badges: Map<String, Int> = emptyMap(),
    /** O meu badge (0 = nenhum). */
    val myBadge: Int = 0,
    /** Daqui a quantos ms posso trocar de foto (0 = ja). */
    val photoNextChangeMs: Long = 0L,
    /** Foto escolhida e ja reduzida, a espera de "Aplicar" (0.9.11). So em memoria. */
    val photoPreview: androidx.compose.ui.graphics.ImageBitmap? = null,
    /** Aviso curto a mostrar uma vez (Toast), por exemplo "Foto atualizada". */
    val notice: String? = null,
    /** Chave de recuperacao (12 palavras) ativa neste telemovel. */
    val recoveryOn: Boolean = false,
    /** Palavras a mostrar agora (ao criar ou ao pedir para as ver); `null` fora disso. */
    val recoveryWords: List<String>? = null,
    /** A gerar palavras, a recuperar a conta ou a apagar a copia (mostra progresso). */
    val recoveryBusy: Boolean = false,
    /**
     * Premium (0.9.20): com o bloqueio ligado, quem nao tem premium so envia texto (recebe e
     * atende tudo). Antes do Play Billing, os dois vem dos interruptores de teste do ecra Premium.
     */
    val premiumLock: Boolean = false,
    val premium: Boolean = false,
    /** Premium dado pelo relay (operador ou, mais tarde, Play Billing): ate quando, em ms; 0 = nao. */
    val premiumUntil: Long = 0,
    /** Ecra "Sealer Premium" aberto (por um cadeado ou pelas Definicoes). */
    val showPremium: Boolean = false,
    /**
     * A lista de contactos guardada nao abriu (0.9.23). Nada e gravado por cima ate o
     * utilizador escolher: reabrir a app (o mais comum) ou comecar uma lista nova.
     */
    val contactsUnreadable: Boolean = false,
    /** Pedir o codigo da app antes de ver ou gerar as palavras de recuperacao (0.9.23). */
    val recoveryAskCode: Boolean = false,
    /** Ja se explicou uma vez a permissao de ecra inteiro das chamadas (Android 14+). */
    val fullScreenAsked: Boolean = true,
    /** Texto que nao chegou a ser enviado (conversa, texto): volta a caixa de escrita (0.9.23). */
    val unsentText: Pair<String, String>? = null,
)

enum class CallPhase { CALLING, RINGING_OUT, INCOMING, ACTIVE, ENDED }

data class CallUi(
    val callId: String,
    val contactId: String,
    val name: String,
    val phase: CallPhase,
    /** Quando foi atendida (SystemClock.elapsedRealtime), para o contador. */
    val answeredAt: Long = 0L,
    val muted: Boolean = false,
    val speaker: Boolean = false,
    /** Porque acabou (no ecra durante uns segundos). */
    val endText: String = "",
    val video: VideoUi = VideoUi(),
    /** Fui eu que recebi a chamada (para a janela de aviso depois de desligar). */
    val incoming: Boolean = false,
    /** Videochamada (quem liga comecou com a camara ligada): "Videochamada de [Nome]". */
    val videoCall: Boolean = false,
)

/** Video da chamada (fase 5b). */
data class VideoUi(
    /** Os dois lados tem video: o botao da camara aparece. */
    val supported: Boolean = false,
    /** A minha camara ligada (o outro so a ve depois de aceitar). */
    val camera: Boolean = false,
    val front: Boolean = true,
    /** Aceitei o video do outro. */
    val accepted: Boolean = false,
    /** Recusei o pedido atual do outro ("Agora nao"). */
    val declined: Boolean = false,
    val peerCamera: Boolean = false,
    val peerAccepts: Boolean = false,
    val peerDeclined: Boolean = false,
    /**
     * O video do outro parou (pausa por rede, sem fichas ou outra razao): o ecra mostra so a
     * foto ou a inicial dele, sem texto. A app nunca diz porque.
     */
    val remoteStalled: Boolean = false,
    /** Imagem do outro: tamanho e rotacao (para a por direita no ecra). */
    val remoteW: Int = 0,
    val remoteH: Int = 0,
    val remoteRotation: Int = 0,
    /** Pre-visualizacao da minha camara: tamanho (muda com a camara). */
    val previewW: Int = 0,
    val previewH: Int = 0,
) {
    /** "[Nome] quer mostrar video": a camara dele ligada e eu ainda nao respondi. */
    val request: Boolean get() = peerCamera && !accepted && !declined
    /** A minha camara ligada e o outro ainda nao respondeu ("A espera de [Nome]..."). */
    val waiting: Boolean get() = camera && !peerAccepts && !peerDeclined
    /** O video do outro esta no ecra. */
    val showRemote: Boolean get() = accepted && peerCamera
}

/** O grupo de uma conversa ("g:..."), se for de um grupo. */
fun UiState.groupOf(conversationId: String): GroupInfo? =
    if (conversationId.startsWith("g:")) groups.firstOrNull { it.conversationId == conversationId } else null

/** Nome a mostrar para um membro de um grupo: "Tu", o nome nos contactos, ou um nome curto. */
fun UiState.memberName(contactId: String): String = when {
    publicId.length >= 68 && publicId.substring(4, 68) == contactId -> s(R.string.t_tu)
    else -> contacts.firstOrNull { it.contactId == contactId }?.nickname ?: s(R.string.t_membro, contactId.take(6))
}

/** Versao dos termos de utilizacao: subir quando mudarem, para voltar a pedir a aceitacao. */
const val TERMS_VERSION = 1

const val DEFAULT_LOCK_TIMEOUT_MS = 60_000L

class MainViewModel(app: Application) : AndroidViewModel(app) {
    private val _ui = MutableStateFlow(UiState())
    val ui: StateFlow<UiState> = _ui

    private val prefs = app.getSharedPreferences("qt_prefs", 0)   // so URL do relay e chave PUBLICA (nao secretos)
    // A base de dados e as chaves sao do processo (SealerCore): o segundo plano tambem as usa.
    private val contactStore = SealerCore.contactStore(app)
    private val dbFile: File = SealerCore.dbFile(app)
    private val storageKeys = SealerCore.storageKeys(app)
    private val bio = BioUnlock(app)
    @Volatile private var qt: QtClient? = null
    /**
     * A ligacao ao relay: uma so para tudo desde a 0.9.45 (pedidos, espera por mensagens e
     * anexos; antes eram tres). Os pedidos vao em fila e um envio acaba a espera (ver RelayLink).
     */
    @Volatile private var link: RelayLink? = null
    @Volatile private var poller: Job? = null
    @Volatile private var transferJob: Job? = null
    /** Pedido das fichas do video (0006), numa hora aleatoria: ver `scheduleFichas`. */
    @Volatile private var fichasJob: Job? = null
    /** "Ha anexos para enviar/descarregar": acorda o ciclo de transferencias. */
    private val transferKick = Channel<Unit>(Channel.CONFLATED)
    /** Chave da lista de contactos; so existe em memoria com a base de dados aberta. */
    private val contactsKey: ByteArray? get() = SealerCore.contactsKey()
    /** Chave do armazenamento a espera do BiometricPrompt para ativar a biometria. */
    @Volatile private var pendingBioKey: ByteArray? = null
    /** Bloquear quando a chamada acabar (o tempo do bloqueio passou durante ela). */
    @Volatile private var lockAfterCall = false
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
    /** Um so envio da outbox de cada vez, partilhado com o segundo plano. */
    private val flushLock get() = SealerCore.flushLock
    /** Contactos que ja se avisou nesta sessao que apagaram a conta (tambem ANTES do init). */
    private val deletedNotified = java.util.Collections.synchronizedSet(mutableSetOf<String>())
    /** Batimento da presenca e atualizacao de quem esta online (com a app aberta). */
    @Volatile private var presenceJob: Job? = null
    // ------------------------------------------------------------- chamadas de voz
    /** Motor de audio da chamada (Rust) e microfone/altifalante (Android). */
    @Volatile private var callAudio: CallAudio? = null
    @Volatile private var callIO: CallAudioIO? = null
    /** De segundo a segundo durante uma chamada: eventos, expiracao e a linha tecnica. */
    @Volatile private var callTicker: Job? = null
    /** Video: camara + codificador, descodificador e o aviso de mudanca de rede. */
    @Volatile private var videoSender: VideoSender? = null
    @Volatile private var videoReceiver: VideoReceiver? = null
    @Volatile private var stopNetWatch: (() -> Unit)? = null
    @Volatile private var selfPreview: android.graphics.SurfaceTexture? = null
    @Volatile private var remoteSurface: android.view.Surface? = null
    /**
     * A minha camara esta em pausa porque a app saiu do ecra (segundo plano ou ecra bloqueado,
     * 0010). O botao continua "ligado": volta sozinha quando a app voltar ao ecra.
     */
    @Volatile private var cameraHeld = false
    /** Abrir e fechar a camara nunca se cruzam (ecra, ciclo da chamada e segundo plano). */
    private val senderLock = Any()
    private val callSounds = CallSounds(app)
    /**
     * O ficheiro dos contactos existe mas nao abriu nesta sessao: nao se grava por cima nem se
     * envia a copia de recuperacao (senao a lista verdadeira perdia-se). Ver [saveContacts].
     */
    @Volatile private var contactsBroken = false
    /** Bytes da ultima foto decifrada de cada contacto (para nao voltar a descodificar). */
    private val photoCache = HashMap<String, Pair<Int, androidx.compose.ui.graphics.ImageBitmap>>()
    private val bioChannel = Channel<BioRequest>(Channel.BUFFERED)
    /** Pedidos de autenticacao do sistema; a Activity mostra o BiometricPrompt. */
    val bioRequests: Flow<BioRequest> = bioChannel.receiveAsFlow()


    init {
        // Este ecra passa a ser o dono do nucleo (um ecra antigo que acabe ja nao o fecha).
        SealerCore.uiOwner = this
        ImagePrep.cleanCache(app)
        io { openStorage() }
        presenceJob = viewModelScope.launch(Dispatchers.IO) { presenceLoop() }
    }

    /** Abre a base de dados, ou mostra o ecra de bloqueio se o codigo estiver ativo. */
    private suspend fun openStorage() {
        try {
            if (storageKeys.lockEnabled()) {
                showLocked()
                return
            }
            // Ja aberta pelo segundo plano (uma chamada acordou a app): usa a mesma.
            SealerCore.client?.let { setupClient(it); return }
            openClient(storageKeys.storageKey())
        } catch (e: AppLockedException) {
            showLocked()
        } catch (e: StorageKeyUnavailableException) {
            // Nunca se cria uma chave nova por cima: o utilizador decide se apaga os dados locais.
            _ui.update { it.copy(loading = false, error = s(R.string.t_chave_do_armazenamento_seguro_indisponivel, e.message)) }
        } catch (e: Exception) {
            _ui.update { it.copy(loading = false, error = s(R.string.t_nao_foi_possivel_abrir_o_armazenamento, e.safeMsg())) }
        }
    }

    /** Abre o QtClient com a chave do armazenamento. Apaga `key` da memoria no fim. */
    private suspend fun openClient(key: ByteArray) {
        setupClient(SealerCore.open(getApplication(), key))
    }

    /** O ecra passa a usar a base de dados aberta (por ele ou pelo segundo plano). */
    private suspend fun setupClient(client: QtClient) {
        qt = client
        val ck = contactsKey ?: throw IllegalStateException("sem a chave dos contactos")
        // Lista de contactos no formato antigo: passa ja a ter a segunda camada.
        val contacts = try {
            val list = contactStore.load(ck)
            if (list.isNotEmpty() && contactStore.isLegacy()) runCatching { contactStore.save(list, ck) }
            contactsBroken = false
            list
        } catch (e: ContactsUnreadableException) {
            Log.w(TAG, "contactos: o ficheiro nao abriu (${e.cause?.javaClass?.simpleName})")
            contactsBroken = true
            emptyList()
        }
        // Release: o relay da conta, da lista fixa da build (federacao, `Relays`). Debug: editavel.
        val home = Relays.home(prefs)
        val url = home.url
        val pin = home.pin
        // As ofertas de chamada levam o relay da conta quando nao e o principal (fase 4).
        runCatching { client.callSetRelay(Relays.callName(prefs)) }
        val has = client.hasIdentity()
        // Tudo lido antes do update: a funcao do update pode correr mais de uma vez (concorrencia).
        val publicId = if (has) client.exportPublicIdentity().publicId else ""
        val conversations = client.conversations()
        val blocked = client.blockedContacts().toSet()
        val invites = client.pendingIntros().toSet()
        val recoveryOn = has && client.recoveryEnabled()
        _ui.update {
            it.copy(
                loading = false, hasIdentity = has, relayUrl = url, relayKeyHex = pin,
                publicId = publicId,
                contacts = contacts,
                conversations = conversations,
                readReceipts = prefs.getBoolean("read_receipts", true),
                pushOn = Push.isOn(getApplication()),
                pushAsked = prefs.getBoolean("push_asked", false),
                fullScreenAsked = prefs.getBoolean("full_screen_asked", false),
                locked = false, unlocking = false, lockMessage = null,
                lockEnabled = storageKeys.lockEnabled(),
                bioEnabled = bio.isEnabled(), bioAvailable = bio.isAvailable(),
                lockTimeoutMs = prefs.getLong("lock_timeout_ms", DEFAULT_LOCK_TIMEOUT_MS),
                blocked = blocked,
                invites = invites,
                termsAccepted = prefs.getInt("terms_version", 0) >= TERMS_VERSION,
                presenceOn = prefs.getBoolean("presence", true),
                online = emptySet(),
                recoveryOn = recoveryOn,
                premiumLock = prefs.getBoolean(PREF_PREMIUM_ENFORCED, false),
                premiumUntil = prefs.getLong(PREF_PREMIUM_UNTIL, 0),
                premium = premiumCached(),
                contactsUnreadable = contactsBroken,
            )
        }
        syncPresence(client)
        updateReceiving()
        adoptRinging(client)
        if (has) startLink()
    }

    /** Ecra de bloqueio, sem nenhum dado das conversas no estado. */
    private fun showLocked(message: String? = null) {
        synchronized(photoCache) { photoCache.clear() }
        _ui.update {
            UiState(
                loading = false, locked = true, lockEnabled = true,
                bioEnabled = bio.isEnabled(), bioAvailable = bio.isAvailable(),
                lockTimeoutMs = prefs.getLong("lock_timeout_ms", DEFAULT_LOCK_TIMEOUT_MS),
                lockMessage = message,
            )
        }
    }

    /**
     * Fecha a ligacao ao relay e, com [closeCore], a base de dados (apaga as chaves da memoria).
     * Sem [closeCore]: o ecra acabou mas o nucleo continua (outro ecra usa-o, ou ainda falta
     * avisar o outro lado de uma chamada).
     */
    private fun closeSession(closeCore: Boolean = true) {
        stopCallMedia()
        _ui.update { it.copy(call = null) }
        poller?.cancel()
        poller = null
        transferJob?.cancel()
        transferJob = null
        fichasJob?.cancel()
        fichasJob = null
        link?.shutdown()
        link = null
        // Imagens decifradas que estavam em memoria para o ecra, e ficheiros abertos noutra app.
        SealerImages.clearMemory()
        AttachmentCache.clear()
        SealerFileProvider.clear()
        pendingPhoto = null
        qt = null
        updateReceiving()
        if (closeCore) SealerCore.close()
    }

    /** O ecra esta visivel e com a base de dados aberta: ele recebe tudo (o segundo plano nao mexe). */
    private fun updateReceiving() {
        // Um ecra antigo (ja substituido) nao mexe no estado do ecra novo.
        val owner = SealerCore.uiOwner
        if (owner != null && owner !== this) return
        SealerCore.uiReceiving = visible && qt != null
    }

    // -------------------------------------------------------------- bloqueio da app

    /** Bloqueia ja (so se o bloqueio estiver ativo). */
    fun lockNow() {
        if (!_ui.value.lockEnabled || _ui.value.locked) return
        // Durante uma chamada nao se fecha a base de dados (a chamada precisa dela).
        if (_ui.value.call != null) return
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
        Push.appVisible = false
        // Videochamada: a minha camara pausa em silencio (privacidade e regras do Android); a voz continua.
        holdCameraForBackground()
        if (authInProgress) return
        hiddenAt = SystemClock.elapsedRealtime()
        synchronized(pauseLock) { visible = false }
        updateReceiving()
        // Presenca: "offline" a quem me via online (o seletor de fotos nao conta como sair).
        if (!AutoLockPause.active) io { presenceAnnounce(false) }
        scheduleAutoLock()
        pauseJob?.cancel()
        // Chamada a decorrer: as ligacoes ficam abertas (o servico da chamada mantem a app viva).
        if (_ui.value.call != null) return
        pauseJob = viewModelScope.launch(Dispatchers.IO) {
            delay(BACKGROUND_GRACE_MS)
            // Anexos a meio (enviar ou descarregar): deixa-os acabar, ate BACKGROUND_TRANSFER_MAX_MS.
            val until = SystemClock.elapsedRealtime() + BACKGROUND_TRANSFER_MAX_MS
            while (_ui.value.transfers.isNotEmpty() && SystemClock.elapsedRealtime() < until) delay(5_000L)
            pauseLinks()
        }
    }

    @Volatile private var autoLockJob: Job? = null

    /**
     * Bloqueia em segundo plano quando passa o tempo escolhido (0.9.23). Antes so se via ao
     * voltar a app, e a base de dados e as chaves ficavam em memoria entretanto. Com uma
     * chamada (no ecra ou a tocar) bloqueia quando ela acabar. O Android pode atrasar este
     * relogio com o telemovel a dormir: [onAppVisible] volta a ver ao regressar.
     */
    private fun scheduleAutoLock() {
        autoLockJob?.cancel()
        val st = _ui.value
        if (!st.lockEnabled || st.locked) return
        // Leitor de QR, seletor de fotos ou pedido de permissao: mais tempo (como ao voltar).
        val grace = if (AutoLockPause.active) AutoLockPause.GRACE_MS else 0L
        autoLockJob = viewModelScope.launch(Dispatchers.IO) {
            delay(st.lockTimeoutMs + grace)
            if (visible || authInProgress) return@launch
            val now = _ui.value
            if (!now.lockEnabled || now.locked) return@launch
            if (now.call != null || SealerCore.ringing != null) { lockAfterCall = true; return@launch }
            lockNow()
        }
    }

    /**
     * A app voltou ao ecra (Activity.onStart): bloqueia se passou o tempo escolhido; senao,
     * religa ao relay se as ligacoes tinham sido fechadas em segundo plano.
     */
    fun onAppVisible() {
        Push.appVisible = true
        Push.clear(getApplication())
        autoLockJob?.cancel()
        autoLockJob = null
        synchronized(pauseLock) {
            visible = true
            pauseJob?.cancel()
            pauseJob = null
        }
        updateReceiving()
        CallNotify.clearMissed(getApplication())
        val t = hiddenAt
        hiddenAt = null
        if (t != null) {
            // Voltar do leitor de QR: mais AutoLockPause.GRACE_MS antes de bloquear.
            val grace = if (AutoLockPause.active) AutoLockPause.GRACE_MS else 0L
            AutoLockPause.active = false
            val st = _ui.value
            if (!authInProgress && st.lockEnabled && !st.locked && SystemClock.elapsedRealtime() - t - grace >= st.lockTimeoutMs) {
                // Com uma chamada (no ecra ou a tocar no segundo plano), fechar a base perdia-a:
                // bloqueia quando acabar.
                if (st.call == null && SealerCore.ringing == null) lockNow() else lockAfterCall = true
            }
        }
        resumeLinks()
        // Ligacoes ainda abertas (voltou antes de 30 s): "online" ja. Senao, startLink trata.
        io { if (link != null && !_ui.value.locked) presenceAnnounce(true) }
        // O segundo plano pode ter recebido mensagens ou estar a tocar uma chamada.
        io {
            val c = qt ?: return@io
            // Sem identidade (ecra inicial ou a recuperar a conta) nao ha nada para refrescar: os
            // grupos pedem a identidade e o erro aparecia ao voltar a app (0.9.31; ex.: sair para
            // copiar as 12 palavras de um gestor de palavras-passe).
            if (_ui.value.locked || !_ui.value.hasIdentity) return@io
            refreshViews(c)
            adoptRinging(c)
            // O toque do segundo plano ja tinha acabado: bloqueia como devia.
            if (lockAfterCall && _ui.value.call == null) {
                lockAfterCall = false
                lockNow()
            }
        }
    }

    /** Fecha as ligacoes ao relay e para a rececao e as transferencias (app em segundo plano). */
    private fun pauseLinks() {
        synchronized(pauseLock) {
            // Voltou ao ecra entretanto; ou bloqueada, sem identidade ou ja fechadas; ou em chamada.
            if (visible || qt == null || link == null || _ui.value.call != null) return
            poller?.cancel()
            poller = null
            transferJob?.cancel()
            transferJob = null
            link?.shutdown()
            link = null
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
        io { if (qt?.hasIdentity() == true && link == null) startLink(publishBundle = false) }
    }

    fun unlockWithCode(code: String) = io {
        val wait = lockWaitMs()
        if (wait > 0) {
            _ui.update { it.copy(lockMessage = waitMessage(wait)) }
            return@io
        }
        _ui.update { it.copy(unlocking = true, lockMessage = null) }
        try {
            val key = codeAttempt { storageKeys.storageKeyWithCode(code) }
            resetFails()
            openClient(key)
        } catch (e: QtException.Crypto) {
            val wait2 = registerFail()
            _ui.update { it.copy(unlocking = false, lockMessage = s(R.string.t_codigo_errado) + (if (wait2 > 0) " " + waitMessage(wait2) else "")) }
        } catch (e: QtException.InvalidInput) {
            _ui.update { it.copy(unlocking = false, lockMessage = s(R.string.t_o_codigo_tem_pelo_menos_digitos, minCodeLen())) }
        } catch (e: Exception) {
            _ui.update { it.copy(unlocking = false, lockMessage = s(R.string.t_nao_foi_possivel_desbloquear, e.safeMsg())) }
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
                    lockMessage = s(R.string.t_a_impressao_digital_ou_o_bloqueio),
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
            _ui.update { it.copy(unlocking = false, lockMessage = s(R.string.t_nao_foi_possivel_desbloquear_assim_usa)) }
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
            error(s(R.string.t_o_codigo_tem_de_ter_pelo, minCodeLen()))
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_ativar_o_bloqueio, e.safeMsg()))
        }
    }

    fun disableLock(code: String) = io {
        if (lockWaitMs() > 0) { error(waitMessage(lockWaitMs())); return@io }
        try {
            codeAttempt { storageKeys.disableLock(code) }
            bio.disable()
            resetFails()
            _ui.update { it.copy(lockEnabled = false, bioEnabled = false) }
        } catch (e: QtException.Crypto) {
            registerFail()
            error(s(R.string.t_codigo_errado))
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_desativar_o_bloqueio, e.safeMsg()))
        }
    }

    fun changeCode(oldCode: String, newCode: String) = io {
        if (lockWaitMs() > 0) { error(waitMessage(lockWaitMs())); return@io }
        try {
            codeAttempt { storageKeys.changeCode(oldCode, newCode) }
            resetFails()
            _ui.update { it.copy(notice = s(R.string.t_codigo_alterado)) }   // aviso curto, nao um erro
        } catch (e: QtException.Crypto) {
            registerFail()
            error(s(R.string.t_o_codigo_atual_esta_errado))
        } catch (e: QtException.InvalidInput) {
            error(s(R.string.t_o_codigo_tem_de_ter_pelo, minCodeLen()))
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_mudar_o_codigo, e.safeMsg()))
        }
    }

    /** Ativar a biometria: confirma o codigo e pede o BiometricPrompt. */
    fun startBioEnable(code: String) = io {
        if (Build.VERSION.SDK_INT < 30 || !bio.isAvailable()) {
            error(s(R.string.t_este_telemovel_nao_permite_desbloquear_assim))
            return@io
        }
        if (lockWaitMs() > 0) { error(waitMessage(lockWaitMs())); return@io }
        try {
            val key = codeAttempt { storageKeys.storageKeyWithCode(code) }
            resetFails()
            pendingBioKey?.fill(0)
            pendingBioKey = key
            if (Build.VERSION.SDK_INT >= 30) bioChannel.send(BioRequest(BioPurpose.ENABLE, bio.cipherForEnable()))
        } catch (e: QtException.Crypto) {
            registerFail()
            error(s(R.string.t_codigo_errado))
        } catch (e: Exception) {
            cancelBioEnable()
            error(s(R.string.t_nao_foi_possivel_ativar, e.safeMsg()))
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
            error(s(R.string.t_nao_foi_possivel_ativar, e.safeMsg()))
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
        contactsBroken = false
        prefs.edit().clear().apply()
        _ui.update { UiState(loading = true) }
        openStorage()
    }

    // Tentativas erradas: depois de 5, espera 30 s, 1 min, 2 min... ate 15 min.
    // Desde a 0.9.22 a espera conta pelo relogio desde que o telemovel ligou
    // (`elapsedRealtime`, conta tambem com o ecra desligado), que nao se muda nas definicoes:
    // mudar a hora do telemovel ja nao salta a espera. Depois de reiniciar o telemovel esse
    // relogio volta a zero e a espera recomeca inteira (nunca fica mais curta).
    private fun lockWaitMs(): Long {
        // Uma tentativa ficou a meio (a app foi fechada durante o Argon2): conta como errada.
        if (!attemptRunning && prefs.getBoolean(PREF_LOCK_PENDING, false)) {
            prefs.edit().remove(PREF_LOCK_PENDING).commit()
            registerFail()
        }
        val wait = prefs.getLong("lock_wait", 0L)
        if (wait <= 0L) return 0L
        val now = SystemClock.elapsedRealtime()
        var at = prefs.getLong("lock_wait_at", now)
        if (at > now) {   // o telemovel reiniciou: a espera recomeca
            at = now
            prefs.edit().putLong("lock_wait_at", at).apply()
        }
        return (wait - (now - at)).coerceAtLeast(0L)
    }

    private fun registerFail(): Long {
        val n = prefs.getInt("lock_fails", 0) + 1
        val wait = if (n < 5) 0L else (30_000L shl (n - 5).coerceAtMost(5)).coerceAtMost(900_000L)
        prefs.edit()
            .putInt("lock_fails", n)
            .putLong("lock_wait", wait)
            .putLong("lock_wait_at", SystemClock.elapsedRealtime())
            .remove("lock_until")   // formato antigo (hora do telemovel), ate a 0.9.21
            .commit()   // escrito no disco antes de mostrar "codigo errado" (fechar a app nao apaga a tentativa)
        return wait
    }

    /** Ha uma tentativa com o codigo a decorrer (Argon2, ~1 s). */
    @Volatile private var attemptRunning = false

    /**
     * Argon2 com o codigo: a tentativa fica marcada no disco ANTES de calcular (0.9.23). Se a
     * app for fechada a meio, conta como errada na vez seguinte (antes nao contava, e fechar a
     * app durante o calculo permitia tentar sem limite). Certa ou errada, a marca sai no fim.
     */
    private fun <T> codeAttempt(block: () -> T): T {
        attemptRunning = true
        prefs.edit().putBoolean(PREF_LOCK_PENDING, true).commit()
        try {
            return block()
        } finally {
            prefs.edit().remove(PREF_LOCK_PENDING).commit()
            attemptRunning = false
        }
    }

    private fun resetFails() {
        prefs.edit().remove("lock_fails").remove("lock_wait").remove("lock_wait_at").remove("lock_until").apply()
    }

    private fun waitMessage(ms: Long) = s(R.string.t_demasiadas_tentativas_tenta_de_novo_daqui, (ms + 999) / 1000)

    // ---------------------------------------------------------------- configuracao

    fun createIdentity(relayUrl: String, relayKeyHex: String) = io {
        val client = qt ?: return@io
        // Conta nova: comeca no relay principal (se estiver fechado a contas novas, o startLink
        // passa ao seguinte da lista).
        Relays.resetHome(prefs)
        Relays.home(prefs).let { h -> _ui.update { it.copy(relayUrl = h.url, relayKeyHex = h.pin) } }
        runCatching { client.callSetRelay(null) }
        saveRelay(relayUrl, relayKeyHex)
        val me = client.createIdentity()
        // Conta nova: "Mostrar quando estou online" comeca desligado (0.9.23). Liga-se no menu.
        // Quem ja tinha conta fica como estava.
        newAccountPresenceOff()
        _ui.update { it.copy(hasIdentity = true, publicId = me.publicId) }
        syncPresence(client)
        startLink()
    }

    private fun newAccountPresenceOff() {
        prefs.edit().putBoolean("presence", false).apply()
        _ui.update { it.copy(presenceOn = false, online = emptySet()) }
    }

    fun saveRelay(relayUrl: String, relayKeyHex: String) {
        if (!BuildConfig.RELAY_EDITABLE) return   // release: relay fixo, nunca alteravel
        val u = relayUrl.trim().lowercase()
        if (!u.startsWith("wss://") && !(BuildConfig.ALLOW_CLEARTEXT && u.startsWith("ws://"))) {
            error(s(R.string.t_o_relay_tem_de_usar_wss))
            return
        }
        prefs.edit().putString("relay_url", relayUrl.trim()).putString("relay_key", relayKeyHex.trim()).apply()
        _ui.update { it.copy(relayUrl = relayUrl.trim(), relayKeyHex = relayKeyHex.trim()) }
        // hasIdentity() toca no SQLCipher: nunca na thread principal.
        io { if (qt?.hasIdentity() == true) startLink() }
    }

    /**
     * Abre as ligacoes ao relay e comeca a receber. Com [publishBundle], republica tambem as
     * chaves publicas (renova os 90 dias no relay): ao abrir a app, desbloquear ou criar o ID.
     * Ao voltar de segundo plano nao e preciso (as chaves ja la estao e nao mudaram).
     */
    private suspend fun startLink(publishBundle: Boolean = true) {
        val client = qt ?: return
        poller?.cancel()
        link?.shutdown()
        val l = RelayLink(client, _ui.value.relayUrl, _ui.value.relayKeyHex.ifBlank { null })
        link = l
        transferJob?.cancel()
        status("A ligar ao relay…")
        // Desde a 0.9.45 o bundle vai pela mesma ligacao, a seguir ao pedido das mensagens
        // (o receiveLoop arranca primeiro): normalmente as mensagens continuam a aparecer primeiro.
        val bundleLink = l
        poller = viewModelScope.launch(Dispatchers.IO) { receiveLoop(client) }
        transferJob = viewModelScope.launch(Dispatchers.IO) {
            var rounds = 0
            while (isActive) {
                val retry = try {
                    runTransfers()
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    true
                }
                // Depois de uma falha tenta de novo daqui a 15 s, 30 s, 1 min... ate 5 min (ou
                // antes, se houver novidades). Antes era sempre de 15 em 15 s.
                if (retry) {
                    val wait = (RETRY_MS shl rounds.coerceAtMost(4)).coerceAtMost(RETRY_MAX_MS)
                    rounds++
                    withTimeoutOrNull(wait) { transferKick.receive() }
                } else {
                    rounds = 0
                    transferKick.receive()
                }
            }
        }
        transferKick.trySend(Unit)
        if (publishBundle) {
            try {
                // Publicar (ou re-publicar) o bundle publico para que outros nos possam contactar.
                val r = bundleLink.request(client.relayRequestRegisterBundle(client.publishableBundle()))
                // Sem avisos de ligacao na app (por agora): so no registo (adb logcat -s SealerRelay).
                if (r !is RelayResponse.Ack) Log.w(TAG, "relay recusou o bundle: ${r.describe()}")
                // Federacao: a conta vive noutro relay, ou este ja nao aceita contas novas.
                if (r is RelayResponse.Error && moveRelay(r.reason)) return
            } catch (e: Exception) {
                // Sem rede: o receiveLoop mostra o estado da ligacao; volta a publicar no proximo arranque.
            }
        }
        // Notificacoes: o "endereco de campainha" vai sempre que a app se liga (renova o prazo no relay).
        registerPush(client, bundleLink)
        // Copia de recuperacao: so se mudou alguma coisa desde a ultima (o relay renova o prazo
        // da que la esta quando recebe o bundle).
        if (publishBundle) scheduleBackup()
        refreshPremium(client, bundleLink)
        scheduleFichas()
        // Presenca: "online" aos contactos (se a app estiver no ecra).
        if (visible) presenceAnnounce(true)
    }

    /** Relays ja tentados nesta sessao por estarem fechados a contas novas. */
    private val relaysTried = mutableSetOf<String>()
    /** Mudancas de relay nesta sessao (nunca anda aos saltos entre relays sem fim). */
    private var relayMoves = 0

    /**
     * Federacao (fase 3): o relay diz que a conta vive noutro ("conta noutro relay: X", ex.: conta
     * recuperada noutro telemovel), ou que ja nao aceita contas novas (tenta o seguinte da lista).
     * Guarda o relay da conta e volta a ligar-se a ele. Verdadeiro se mudou.
     */
    private fun moveRelay(reason: String): Boolean {
        if (relayMoves >= 4) return false
        val current = Relays.home(prefs)
        val elsewhere = relayElsewhereName(reason)
        val target = when {
            elsewhere != null -> Relays.byName(elsewhere)
            reason == relayNoNewAccountsReason() -> {
                relaysTried.add(current.name)
                Relays.next(relaysTried)
            }
            else -> null
        } ?: return false
        if (target.name == current.name || !Relays.setHome(prefs, target.name)) return false
        relayMoves++
        Log.i(TAG, "relay da conta: ${target.name}")
        runCatching { qt?.callSetRelay(Relays.callName(prefs)) }
        _ui.update { it.copy(relayUrl = target.url, relayKeyHex = target.pin) }
        io { startLink() }
        return true
    }

    /**
     * Envia ao relay o token FCM deste telemovel (ou vazio, com o interruptor desligado).
     * Sem Firebase nesta build, sem Google Play Services ou sem rede: nada (tenta no proximo arranque).
     */
    private suspend fun registerPush(client: QtClient, l: RelayLink) {
        if (!Push.available) return
        val app = getApplication<Application>()
        val token = if (Push.isOn(app)) Push.token(app) ?: return else ""
        try {
            val r = l.request(client.relayRequestPushToken(token))
            if (r !is RelayResponse.Ack) Log.w(TAG, "relay recusou o token de notificacoes: ${r.describe()}")
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Log.w(TAG, "token de notificacoes: ${e.safeMsg()}")
        }
    }

    /** Interruptor "Notificações". Desligar avisa o relay (token vazio) e esquece o token na Google. */
    fun setPush(on: Boolean) {
        val app = getApplication<Application>()
        Push.setOn(app, on)
        _ui.update { it.copy(pushOn = on) }
        io {
            val client = qt
            val l = link
            if (client != null && l != null) registerPush(client, l)
            if (!on) Push.deleteToken(app)
        }
    }

    /** O aviso da permissao de ecra inteiro (Android 14+) ja foi mostrado. */
    fun markFullScreenAsked() {
        prefs.edit().putBoolean("full_screen_asked", true).apply()
        _ui.update { it.copy(fullScreenAsked = true) }
    }

    /** O pedido de permissao de notificacoes (Android 13+) ja foi mostrado. */
    fun markPushAsked() {
        prefs.edit().putBoolean("push_asked", true).apply()
        _ui.update { it.copy(pushAsked = true) }
    }

    /** "online" (app no ecra) ou "offline" aos contactos. Sem avisos na app se falhar. */
    private suspend fun presenceAnnounce(onScreen: Boolean) {
        val client = qt ?: return
        try {
            if (client.presenceAnnounce(onScreen) > 0u) flushOutbox()
        } catch (e: Exception) {
            Log.w(TAG, "presenca: ${e.safeMsg()}")
        }
    }

    // ------------------------------------------------------------------- presenca

    /** Diz ao Rust quem sao os contactos (so eles veem o meu estado) e se a presenca esta ligada. */
    private suspend fun syncPresence(client: QtClient) {
        try {
            val n = client.presenceConfigure(_ui.value.presenceOn, _ui.value.contacts.map { it.contactId })
            if (n > 0u) flushOutbox()
        } catch (e: Exception) {
            Log.w(TAG, "presenca: ${e.safeMsg()}")
        }
        syncCallContacts(client)
        syncPhotos(client)
    }

    // ------------------------------------------------------------------- chamadas: contactos

    /**
     * Diz ao Rust quem sao os contactos: com eles (se me tiverem tambem) a chamada liga direto,
     * sem perguntar nada, e so eles recebem o aviso "tenho video" (uma vez).
     */
    private suspend fun syncCallContacts(client: QtClient) {
        try {
            if (client.callContactsSync(_ui.value.contacts.map { it.contactId }) > 0u) flushOutbox()
        } catch (e: Exception) {
            Log.w(TAG, "chamadas: ${e.safeMsg()}")
        }
        val video = runCatching { client.callVideoContacts().toSet() }.getOrDefault(emptySet())
        _ui.update { it.copy(videoContacts = video) }
    }

    // ------------------------------------------------------------------- foto de perfil


    /** Envia a minha foto a quem ainda nao a tem (contactos novos) e recarrega as fotos. */
    private suspend fun syncPhotos(client: QtClient) {
        try {
            if (client.profilePhotoSync(_ui.value.contacts.map { it.contactId }) > 0u) flushOutbox()
        } catch (e: Exception) {
            Log.w(TAG, "foto: ${e.safeMsg()}")
        }
        try {
            if (client.badgeSync(_ui.value.contacts.map { it.contactId }) > 0u) flushOutbox()
        } catch (e: Exception) {
            Log.w(TAG, "badge: ${e.safeMsg()}")
        }
        refreshPhotos(client)
        refreshBadges(client)
    }

    /** Badges (o meu e os dos contactos), ja verificados pelo nucleo. */
    private fun refreshBadges(client: QtClient) {
        try {
            val map = HashMap<String, Int>()
            for (c in _ui.value.contacts) {
                val b = client.badgeOf(c.contactId).toInt()
                if (b != Badges.NONE) map[c.contactId] = b
            }
            val mine = client.badgeMine().toInt()
            val mineChanged = mine != _ui.value.myBadge
            if (map != _ui.value.badges || mineChanged) _ui.update { it.copy(badges = map, myBadge = mine) }
            // O meu badge mudou (ou chegou): a copia de recuperacao passa a leva-lo (0.9.29; antes
            // so se refazia quando mudavam os contactos e o badge podia ficar de fora). O Rust so
            // envia se a copia mudou.
            if (mineChanged) scheduleBackup()
        } catch (e: Exception) {
            Log.w(TAG, "badge: ${e.safeMsg()}")
        }
    }

    private fun refreshPhotos(client: QtClient) = synchronized(photoCache) {
        try {
            fun show(key: String, bytes: ByteArray?): androidx.compose.ui.graphics.ImageBitmap? {
                if (bytes == null) return null
                val h = bytes.contentHashCode()
                photoCache[key]?.let { if (it.first == h) return it.second }
                return ProfilePhoto.decode(bytes)?.also { photoCache[key] = h to it }
            }
            val map = HashMap<String, androidx.compose.ui.graphics.ImageBitmap>()
            for (c in _ui.value.contacts) show(c.contactId, client.profilePhotoOf(c.contactId))?.let { map[c.contactId] = it }
            val mine = show(MY_PHOTO, client.profilePhotoMine())
            photoCache.keys.retainAll(map.keys + (if (mine != null) setOf(MY_PHOTO) else emptySet()))
            val wait = client.profilePhotoNextChangeMs()
            _ui.update { it.copy(photos = map, myPhoto = mine, photoNextChangeMs = wait) }
        } catch (e: Exception) {
            Log.w(TAG, "foto: ${e.safeMsg()}")
        }
    }

    /** Foto ja reduzida, a espera de "Aplicar". Nada vai para a base de dados antes disso. */
    @Volatile private var pendingPhoto: ByteArray? = null

    /**
     * Foto escolhida na galeria (ate 20 MB): reduzida aqui e mostrada para confirmar. So
     * "Aplicar" a guarda e envia (e conta para as 72 h). Ate a 0.9.10 era aplicada logo ao
     * escolher, sem se ver nada, e quem tentava outra ficava preso nas 72 h.
     */
    fun previewProfilePhoto(uri: android.net.Uri) = io {
        if (qt == null) return@io
        try {
            val bytes = ProfilePhoto.prepare(getApplication(), uri)
            val img = ProfilePhoto.decode(bytes) ?: throw java.io.IOException("foto ilegivel")
            pendingPhoto = bytes
            _ui.update { it.copy(photoPreview = img) }
        } catch (e: PhotoTooBigException) {
            error(s(R.string.t_a_foto_tem_mais_de_20))
        } catch (e: java.io.IOException) {
            error(s(R.string.t_nao_foi_possivel_ler_essa_imagem))
        }
    }

    /** "Aplicar": guarda a foto, envia-a aos contactos e comeca a contar as 72 h. */
    fun applyProfilePhoto() = io {
        val client = qt ?: return@io
        val bytes = pendingPhoto ?: return@io
        try {
            if (client.profilePhotoSet(bytes, _ui.value.contacts.map { it.contactId }) > 0u) flushOutbox()
            pendingPhoto = null
            _ui.update { it.copy(photoPreview = null, notice = s(R.string.photo_updated)) }
        } catch (e: QtException.InvalidInput) {
            pendingPhoto = null
            _ui.update { it.copy(photoPreview = null) }
            error(if (e.field == "photo_wait") s(R.string.photo_wait) else s(R.string.photo_invalid))
        }
        refreshPhotos(client)
    }

    /** "Cancelar" (ou fechar a janela): esquece a foto escolhida. */
    fun cancelProfilePhoto() {
        pendingPhoto = null
        if (_ui.value.photoPreview != null) _ui.update { it.copy(photoPreview = null) }
    }

    fun dismissNotice() = _ui.update { it.copy(notice = null) }

    /** Tira a minha foto (os contactos deixam de a ver). */
    fun removeProfilePhoto() = io {
        val client = qt ?: return@io
        if (client.profilePhotoRemove(_ui.value.contacts.map { it.contactId }) > 0u) flushOutbox()
        refreshPhotos(client)
    }

    /** Interruptor "Mostrar quando estou online". Desligado: nao mostro o meu nem vejo o dos outros. */
    fun setPresence(on: Boolean) {
        prefs.edit().putBoolean("presence", on).apply()
        _ui.update { it.copy(presenceOn = on, online = if (on) it.online else emptySet()) }
        io {
            val client = qt ?: return@io
            syncPresence(client)
            if (on && visible) presenceAnnounce(true)
        }
    }

    private fun refreshOnline(client: QtClient) {
        val on = client.presenceOnline().toSet()
        if (on != _ui.value.online) _ui.update { it.copy(online = on) }
    }

    /**
     * Com a app aberta: de minuto a minuto atualiza quem esta online (um "online" expira
     * sozinho ao fim de 5 min sem batimento) e, de 4 em 4 min, manda o batimento a quem
     * esta online.
     */
    private suspend fun presenceLoop() {
        var lastBeat = SystemClock.elapsedRealtime()
        while (currentCoroutineContext().isActive) {
            delay(PRESENCE_TICK_MS)
            val client = qt ?: continue
            if (!visible || link == null || _ui.value.locked) continue
            try {
                if (SystemClock.elapsedRealtime() - lastBeat >= presenceHeartbeatMs()) {
                    lastBeat = SystemClock.elapsedRealtime()
                    if (client.presenceHeartbeat() > 0u) flushOutbox()
                }
                refreshOnline(client)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "presenca: ${e.safeMsg()}")
            }
        }
    }

    // ------------------------------------------------------------------- contactos

    /**
     * Grava a lista de contactos. `false` se nao gravou: a lista guardada nao abriu nesta
     * sessao (mostra o aviso; gravar por cima apagava-a) ou a base de dados ja fechou.
     */
    private fun saveContacts(list: List<Contact>): Boolean {
        if (contactsBroken) {
            _ui.update { it.copy(contactsUnreadable = true) }
            return false
        }
        val ck = contactsKey ?: return false
        contactStore.save(list, ck)
        return true
    }

    /** "Fechar" no aviso dos contactos: nada muda (continua sem gravar ate reabrir a app). */
    fun dismissContactsUnreadable() = _ui.update { it.copy(contactsUnreadable = false) }

    /**
     * "Comecar uma lista nova": o ficheiro que nao abre fica guardado a parte (cifrado) e a
     * lista recomeca vazia. A copia de recuperacao passa a ter so os contactos novos.
     */
    fun startNewContactList() = io {
        contactStore.quarantine()
        contactsBroken = false
        _ui.update { it.copy(contactsUnreadable = false, contacts = emptyList()) }
    }

    /** Guarda o contacto. NAO inicia sessao (evita sessoes cruzadas): so ao enviar a 1.a mensagem. */
    fun addContact(nickname: String, publicId: String) = io {
        try {
            // Em minusculas: "QT1:ABC..." (como no QR) e o mesmo ID, e o contactId tem de ser igual
            // ao do nucleo (0.9.23).
            val id = publicId.trim().lowercase()
            val relayId = relayIdOfPublicId(id)               // valida o formato no Rust
            val c = Contact(nickname.trim().ifBlank { s(R.string.t_contacto) }, id, relayId)
            if (c.publicId == _ui.value.publicId) { error(s(R.string.t_esse_e_o_teu_proprio_id)); return@io }
            val list = (_ui.value.contacts.filter { it.contactId != c.contactId }) + c
            if (!saveContacts(list)) return@io
            _ui.update { it.copy(contacts = list, error = null) }
            scheduleBackup()
            qt?.let { client ->
                // Escreveu-me antes (convite) e eu adicionei-o a mao: o convite fica resolvido.
                // Antes ficava pendente e a conversa escondia o botao de ligar.
                runCatching { client.dismissIntro(c.contactId) }
                syncPresence(client)
                refreshViews(client)
            }
            flushOutbox()   // recibos que estavam a espera de saber o endereco deste contacto
        } catch (e: QtException) {
            error(s(R.string.t_public_id_invalido))
        }
    }

    // ---------------------------------------------------------- safety numbers

    /** Mostra o safety number com este contacto (calculado no Rust a partir dos dois Public IDs). */
    fun showSafetyNumber(contact: Contact) = io {
        val client = qt ?: return@io
        try {
            val n = client.safetyNumber(contact.publicId)
            _ui.update { it.copy(safetyNumber = "${contact.nickname}\n\n\u200E$n") }  // LRM: em arabe os grupos nao trocam de ordem
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_calcular_o_safety, e.safeMsg()))
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
        AttachmentCache.clear()   // nada do que vai ser apagado fica decifrado em memoria
        try {
            client.deleteConversation(contactId)
            _ui.update {
                if (it.openChatId == contactId) it.copy(chatMessages = emptyList(), chatHasMore = false) else it
            }
            refreshViews(client)
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_eliminar_a_conversa, e.safeMsg()))
        }
    }

    /**
     * BLOQUEAR: as mensagens deste contacto passam a ser descartadas ao chegar (no Rust),
     * sem recibos, e deixa de ser possivel enviar-lhe. Ele nao e avisado. A conversa fica.
     */
    fun blockContact(contactId: String) = io {
        val client = qt ?: return@io
        try {
            client.blockContact(contactId)
            _ui.update { it.copy(blocked = it.blocked + contactId) }
            refreshViews(client)
            scheduleBackup()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_bloquear, e.safeMsg()))
        }
    }

    /**
     * Denunciar (0.9.18): as mensagens escolhidas seguem cifradas para o operador (o Rust
     * cifra; o relay nao as le e nao guarda quem denunciou). Se o relay aceitar, bloqueia o
     * contacto. `reason`: "spam", "assedio", "ilegal" ou "outro".
     */
    fun reportContact(contactId: String, messageIds: List<String>, reason: String) = io {
        val client = qt ?: return@io
        val publicId = _ui.value.contacts.firstOrNull { it.contactId == contactId }?.publicId
            ?: runCatching { client.pendingIntro(contactId) }.getOrNull()
        if (publicId == null) {
            error(s(R.string.report_failed))
            return@io
        }
        val l = link ?: run { error(s(R.string.t_sem_ligacao_ao_servidor)); return@io }
        val outcome = try {
            when (val r = l.request(client.relayRequestReport(publicId, messageIds, reason))) {
                is RelayResponse.Ack -> null
                is RelayResponse.Error ->
                    if (r.reason == RELAY_UNKNOWN_REQUEST) s(R.string.t_o_servidor_ainda_nao_tem_esta) else s(R.string.t_o_servidor_respondeu, r.reason)
                else -> s(R.string.t_resposta_inesperada_do_servidor)
            }
        } catch (e: QtException) {
            Log.w("SealerRelay", "denuncia: ${e.safeMsg()}")
            s(R.string.report_failed)
        } catch (e: Exception) {
            s(R.string.t_sem_ligacao_ao_servidor)
        }
        if (outcome != null) {
            error(outcome)
            return@io
        }
        try {
            client.blockContact(contactId)
            _ui.update { it.copy(blocked = it.blocked + contactId, notice = s(R.string.report_sent)) }
            refreshViews(client)
        } catch (e: Exception) {
            _ui.update { it.copy(notice = s(R.string.report_sent)) }
        }
    }

    // ------------------------------------------------------------------- premium

    /**
     * Acao paga (fazer chamadas, criar grupos, enviar fotos, GIFs, voz e ficheiros): sem
     * premium e com o bloqueio ligado, abre o ecra Premium e devolve `true` (a acao nao segue).
     * Quem decide e o operador: pergunta-se ao relay antes de cada acao paga,
     * para o premium dado ou tirado contar logo. Sem rede, fica o ultimo estado conhecido.
     */
    private suspend fun premiumBlocked(): Boolean {
        val c = qt
        val l = link
        if (c != null && l != null) refreshPremium(c, l)
        val u = _ui.value
        if (!u.premiumLock || u.premium) return false
        _ui.update { it.copy(showPremium = true) }
        return true
    }

    fun openPremium() {
        _ui.update { it.copy(showPremium = true) }
        io { val c = qt ?: return@io; val l = link ?: return@io; refreshPremium(c, l) }
    }
    fun closePremium() = _ui.update { it.copy(showPremium = false) }

    /**
     * Premium agora: o que o relay diz, pelo relogio dele (0.9.23). Antes contava o relogio do
     * telemovel: adiantar ou atrasar a hora mudava o que a app mostrava. So um relay antigo
     * (sem `active`) usa o relogio do telemovel.
     */
    private fun premiumNow(until: Long, active: Boolean?): Boolean = active ?: (until > System.currentTimeMillis())

    /** Ultimo estado que o relay deu (antes de ele responder outra vez). */
    private fun premiumCached(): Boolean {
        val until = prefs.getLong(PREF_PREMIUM_UNTIL, 0)
        val active = if (prefs.contains(PREF_PREMIUM_ACTIVE)) prefs.getBoolean(PREF_PREMIUM_ACTIVE, false) else null
        return premiumNow(until, active)
    }

    /**
     * Pergunta ao relay ate quando esta conta tem premium e se o operador ligou o bloqueio (ao
     * ligar, ao abrir o ecra Premium e antes de cada acao paga).
     */
    private suspend fun refreshPremium(client: QtClient, l: RelayLink) {
        try {
            val r = l.request(client.relayRequestPremiumStatus(), PREMIUM_TIMEOUT_MS)
            if (r is RelayResponse.Premium) {
                val until = r.untilMs.toLong()
                val active = r.active
                val ed = prefs.edit().putLong(PREF_PREMIUM_UNTIL, until).putBoolean(PREF_PREMIUM_ENFORCED, r.enforced)
                if (active != null) ed.putBoolean(PREF_PREMIUM_ACTIVE, active) else ed.remove(PREF_PREMIUM_ACTIVE)
                ed.apply()
                _ui.update { it.copy(premiumUntil = until, premium = premiumNow(until, active), premiumLock = r.enforced) }
            }
        } catch (e: Exception) {
            // Sem rede: fica o ultimo estado conhecido.
        }
    }

    /**
     * Fichas do video pelo relay (0006): o relay das chamadas so passa o video de quem as tem
     * (quota configuravel no servidor; pelo caminho direto nao se gastam). Pedem-se numa hora ALEATORIA depois
     * de a app abrir (e depois mais ou menos de hora a hora, enquanto faltarem), nunca com uma
     * chamada a decorrer nem ao comecar uma: assim o relay nao junta o pedido a uma chamada.
     * Nada disto aparece no ecra.
     */
    private fun scheduleFichas() {
        fichasJob?.cancel()
        fichasJob = viewModelScope.launch(Dispatchers.IO) {
            delay(kotlin.random.Random.nextLong(FICHAS_FIRST_MIN_MS, FICHAS_FIRST_MAX_MS))
            while (isActive) {
                val client = qt ?: break
                val l = link ?: break
                requestFichas(client, l)
                delay(FICHAS_AGAIN_MS + kotlin.random.Random.nextLong(0L, FICHAS_AGAIN_MS))
            }
        }
    }

    private suspend fun requestFichas(client: QtClient, l: RelayLink) {
        refreshFichasKeys(client)
        // Varios lotes seguidos so com uma quota maior; para no primeiro sem nada.
        repeat(FICHAS_MAX_BATCHES) {
            if (_ui.value.call != null) return
            val req = runCatching { client.relayRequestFichas() }.getOrNull() ?: return
            try {
                when (val r = l.request(req, FICHAS_TIMEOUT_MS)) {
                    is RelayResponse.Fichas -> if (client.fichasAccept(r.issued) == 0u) return
                    else -> {
                        // Relay antigo, sem fichas ou conta sem registo: tenta daqui a umas horas.
                        Log.w(TAG, "fichas: ${r.describe()}")
                        client.fichasRefused()
                        return
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Sem rede ou resposta invalida: tenta outra vez mais tarde.
                Log.w(TAG, "fichas: ${e.safeMsg()}")
                return
            }
        }
    }

    /**
     * Chaves publicas das fichas de cada mes, publicadas FORA do relay (no site, `estado.json`,
     * servido pela Cloudflare). O nucleo so aceita fichas do emissor e so as gasta numa sala se a
     * chave for a do site: um relay comprometido nao pode dar uma chave diferente a cada conta
     * (particao de chaves). Sem rede fica a ultima lista boa; sem lista nenhuma o video pelo relay
     * fica em pausa (falha fechada) e a voz e o caminho direto continuam.
     */
    private fun refreshFichasKeys(client: QtClient) {
        val body = RelayLink.fetchSmall(BuildConfig.FICHAS_KEYS_URL, FICHAS_KEYS_MAX_BYTES) ?: return
        try {
            client.fichasSetPublishedKeys(body)
        } catch (e: Exception) {
            Log.w(TAG, "fichas: estado.json invalido (${e.safeMsg()})")
        }
    }

    /** O relay recusou um anexo ou uma chamada por falta de premium: mostra o ecra Premium. */
    private fun premiumRefused() {
        _ui.update { it.copy(showPremium = true) }
        io { val c = qt ?: return@io; val l = link ?: return@io; refreshPremium(c, l) }
    }

    // ------------------------------------------------------------ chave de recuperacao

    /** O que fazer depois de confirmar que e o dono (codigo da app ou bloqueio do telemovel). */
    private enum class RecoveryAction { SHOW, CREATE }
    @Volatile private var pendingRecovery: RecoveryAction? = null

    /**
     * Ver ou gerar as 12 palavras so depois de confirmar que e o dono (0.9.23): com o bloqueio
     * da app, o codigo dela; sem ele, o bloqueio do telemovel. Quem pegasse no telemovel
     * desbloqueado podia ver as palavras e levar a conta. Telemovel sem bloqueio nenhum: segue.
     */
    fun recoveryCreate() = recoveryAuth(RecoveryAction.CREATE)

    /** Mostra de novo as palavras atuais (depois de confirmar). */
    fun recoveryShow() = recoveryAuth(RecoveryAction.SHOW)

    private fun recoveryAuth(action: RecoveryAction) = io {
        pendingRecovery = action
        when {
            _ui.value.lockEnabled -> _ui.update { it.copy(recoveryAskCode = true) }
            bio.deviceAuthAvailable() -> bioChannel.send(BioRequest(BioPurpose.RECOVERY, null))
            else -> recoveryAuthorized()
        }
    }

    /** O codigo da app escrito na janela da chave de recuperacao. */
    fun recoveryWithCode(code: String) = io {
        val action = pendingRecovery
        _ui.update { it.copy(recoveryAskCode = false) }
        val wait = lockWaitMs()
        if (wait > 0) { pendingRecovery = null; error(waitMessage(wait)); return@io }
        try {
            codeAttempt { storageKeys.storageKeyWithCode(code) }.fill(0)
            resetFails()
            pendingRecovery = action
            recoveryAuthorized()
        } catch (e: QtException.Crypto) {
            pendingRecovery = null
            val w = registerFail()
            error(s(R.string.t_codigo_errado) + (if (w > 0) " " + waitMessage(w) else ""))
        } catch (e: QtException.InvalidInput) {
            pendingRecovery = null
            error(s(R.string.t_o_codigo_tem_pelo_menos_digitos, minCodeLen()))
        } catch (e: Exception) {
            pendingRecovery = null
            error(s(R.string.t_nao_foi_possivel_desbloquear, e.safeMsg()))
        }
    }

    /** Cancelou a confirmacao (janela do codigo ou do telemovel). */
    fun recoveryCancelAuth() {
        pendingRecovery = null
        _ui.update { it.copy(recoveryAskCode = false) }
    }

    /** Confirmado: faz o que estava pedido. */
    fun recoveryAuthorized() = io {
        when (pendingRecovery.also { pendingRecovery = null }) {
            RecoveryAction.SHOW -> recoveryShowNow()
            RecoveryAction.CREATE -> recoveryCreateNow()
            null -> {}
        }
    }

    /** Gera 12 palavras novas (as antigas deixam de servir) e mostra-as. Argon2: fora da UI. */
    private fun recoveryCreateNow() {
        val client = qt ?: return
        _ui.update { it.copy(recoveryBusy = true) }
        try {
            val words = client.recoveryEnable()
            _ui.update { it.copy(recoveryOn = true, recoveryWords = words, recoveryBusy = false) }
        } catch (e: Exception) {
            _ui.update { it.copy(recoveryBusy = false) }
            error(s(R.string.recovery_failed))
        }
    }

    private fun recoveryShowNow() {
        val words = qt?.recoveryWords() ?: return
        _ui.update { it.copy(recoveryWords = words) }
    }

    /** As palavras foram escritas (e confirmadas): esconde-as e envia a copia ja. */
    fun recoveryWritten() {
        _ui.update { it.copy(recoveryWords = null) }
        scheduleBackup(force = true, delayMs = 0)
    }

    fun recoveryHide() = _ui.update { it.copy(recoveryWords = null) }

    /** Desliga: o relay apaga a copia e as palavras deixam de servir. */
    fun recoveryDisable() = io {
        val client = qt ?: return@io
        val l = link ?: run { error(s(R.string.t_sem_ligacao_ao_servidor)); return@io }
        _ui.update { it.copy(recoveryBusy = true) }
        try {
            val r = l.request(client.relayRequestBackupDelete())
            if (r !is RelayResponse.Ack) throw IllegalStateException(r.describe())
            client.recoveryDisable()
            _ui.update { it.copy(recoveryOn = false, recoveryWords = null, recoveryBusy = false) }
        } catch (e: Exception) {
            _ui.update { it.copy(recoveryBusy = false) }
            error(s(R.string.t_sem_ligacao_ao_servidor))
        }
    }

    private var backupJob: Job? = null

    /**
     * Copia de recuperacao: refeita pouco depois de os contactos ou os bloqueados mudarem (e
     * ao abrir a app). O Rust so devolve pedidos se o conteudo mudou (ou com [force]).
     */
    private fun scheduleBackup(force: Boolean = false, delayMs: Long = BACKUP_DELAY_MS) {
        if (!_ui.value.recoveryOn) return
        // Lista de contactos por ler: a copia teria uma lista vazia (e apagava a boa no relay).
        if (contactsBroken) return
        backupJob?.cancel()
        backupJob = viewModelScope.launch(Dispatchers.IO) {
            delay(delayMs)
            val client = qt ?: return@launch
            val l = link ?: return@launch
            try {
                val contacts = _ui.value.contacts.map { BackupContact(it.nickname, it.publicId) }
                val up = client.recoveryBackup(contacts, force)
                if (up.requests.isEmpty()) return@launch
                for (req in up.requests) {
                    val r = l.request(req)
                    if (r !is RelayResponse.Ack) {
                        Log.w(TAG, "copia de recuperacao recusada: ${r.describe()}")
                        return@launch
                    }
                }
                client.recoveryBackupSent(up.digest)
                Log.d(TAG, "copia de recuperacao enviada (${up.requests.size} parte(s))")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "copia de recuperacao: ${e.safeMsg()}")
            }
        }
    }

    /**
     * RECUPERAR (telemovel sem conta): confere as palavras, vai buscar a copia ao relay (com
     * uma chave temporaria), instala a mesma identidade e os contactos e liga-se. As sessoes
     * com os contactos recomecam sozinhas (`sessionsToRestart`).
     */
    fun restoreAccount(words: List<String>) = io {
        val client = qt ?: return@io
        _ui.update { it.copy(recoveryBusy = true, error = null) }
        try {
            try {
                client.recoveryPrepare(words)
            } catch (e: QtException) {
                error(s(R.string.recovery_bad_words)); return@io
            }
            val l = RelayLink(client, _ui.value.relayUrl, _ui.value.relayKeyHex.ifBlank { null })
            val parts = mutableListOf<ByteArray>()
            try {
                var total = 1
                while (parts.size < total) {
                    when (val r = l.request(client.relayRequestBackupGet(parts.size.toUInt()))) {
                        is RelayResponse.BackupPart -> { total = r.parts.toInt(); parts += r.data }
                        is RelayResponse.BackupNotFound -> { error(s(R.string.recovery_not_found)); return@io }
                        else -> { error(s(R.string.t_resposta_inesperada_do_servidor)); return@io }
                    }
                }
            } catch (e: Exception) {
                error(s(R.string.t_sem_ligacao_ao_servidor)); return@io
            } finally {
                l.shutdown()
            }
            val acc = try {
                client.recoveryRestore(parts)
            } catch (e: QtException) {
                error(s(R.string.recovery_bad_backup)); return@io
            }
            val list = acc.contacts.mapNotNull { c ->
                runCatching { Contact(c.nickname.ifBlank { s(R.string.t_contacto) }, c.publicId, relayIdOfPublicId(c.publicId)) }.getOrNull()
            }
            // A copia substitui a lista; se a lista guardada nao abria, fica guardada a parte.
            if (contactsBroken) { contactStore.quarantine(); contactsBroken = false }
            newAccountPresenceOff()   // como numa conta nova, neste telemovel
            if (!saveContacts(list)) return@io
            val blocked = client.blockedContacts().toSet()
            _ui.update {
                it.copy(
                    hasIdentity = true, publicId = acc.publicId, contacts = list,
                    blocked = blocked, recoveryOn = true, contactsUnreadable = false,
                    notice = s(R.string.recovery_restored),
                )
            }
            refreshViews(client)
            startLink()
            // Presenca, fotos e o meu badge (vem na copia): sem isto o badge so aparecia depois
            // de reiniciar a app (0.9.22), porque so o arranque da app fazia esta sincronizacao.
            syncPresence(client)
        } finally {
            _ui.update { it.copy(recoveryBusy = false) }
        }
    }

    fun unblockContact(contactId: String) = io {
        val client = qt ?: return@io
        try {
            client.unblockContact(contactId)
            _ui.update { it.copy(blocked = it.blocked - contactId) }
            scheduleBackup()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_desbloquear_2, e.safeMsg()))
        }
    }

    /**
     * ACEITAR CONVITE: quem me escreveu mandou o seu Public ID (cifrado, verificado no Rust
     * contra a identidade da sessao), por isso nao e preciso escreve-lo. Fica nos contactos.
     */
    fun acceptInvite(contactId: String, nickname: String) = io {
        val client = qt ?: return@io
        try {
            val publicId = client.pendingIntro(contactId) ?: run { error(s(R.string.t_este_convite_ja_nao_esta_disponivel)); return@io }
            val c = Contact(nickname.trim().ifBlank { s(R.string.t_contacto_2, contactId.take(4)) }, publicId, relayIdOfPublicId(publicId))
            val list = _ui.value.contacts.filter { it.contactId != c.contactId } + c
            if (!saveContacts(list)) return@io
            client.dismissIntro(contactId)
            _ui.update { it.copy(contacts = list, invites = it.invites - contactId) }
            scheduleBackup()
            syncPresence(client)
            refreshViews(client)
            flushOutbox()   // recibos que estavam a espera de saber o endereco deste contacto
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_aceitar_o_convite, e.safeMsg()))
        }
    }

    /** RECUSAR CONVITE: apaga a conversa e a sessao (se voltar a escrever, aparece outro convite). */
    fun declineInvite(contactId: String) = deleteContact(contactId)

    /** Aceitou os termos de utilizacao e a politica de privacidade (ecra inicial). */
    fun acceptTerms() {
        prefs.edit().putInt("terms_version", TERMS_VERSION).apply()
        _ui.update { it.copy(termsAccepted = true) }
    }

    /**
     * APAGAR CONTACTO: apaga para sempre as mensagens, o que estava por enviar,
     * as chaves da sessao cifrada e o proprio contacto. Antes de apagar as chaves, avisa o
     * outro lado ("sessao terminada", sem nada no ecra dele): a app dele apaga a sessao e a
     * proxima mensagem dele abre uma nova (aqui aparece como convite). Sem o aviso, ele
     * continuava a cifrar com a sessao antiga e as mensagens perdiam-se. Bloqueado: nao avisa.
     */
    fun deleteContact(contactId: String) = io {
        val client = qt ?: return@io
        AttachmentCache.clear()   // nada do que vai ser apagado fica decifrado em memoria
        try {
            // Endereco no relay: do contacto ou, num convite recusado, do Public ID que ele mandou.
            val relayId = _ui.value.contacts.firstOrNull { it.contactId == contactId }?.relayId
                ?: client.pendingIntro(contactId)?.let { runCatching { relayIdOfPublicId(it) }.getOrNull() }
            // Primeiro o Rust (mensagens + chaves + aviso na outbox), na mesma transacao; depois a lista.
            if (relayId != null) client.deleteContactNotify(contactId, relayId) else client.deleteContact(contactId)
            val list = _ui.value.contacts.filter { it.contactId != contactId }
            saveContacts(list)   // com a lista guardada por ler, nao grava (e mostra o aviso)
            _ui.update {
                it.copy(
                    contacts = list,
                    openChatId = if (it.openChatId == contactId) null else it.openChatId,
                    chatMessages = if (it.openChatId == contactId) emptyList() else it.chatMessages,
                    chatHasMore = if (it.openChatId == contactId) false else it.chatHasMore,
                )
            }
            syncPresence(client)
            refreshViews(client)
            flushOutbox()   // o aviso "sessao terminada" (fica na outbox se nao houver rede)
            scheduleBackup()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_apagar_o_contacto, e.safeMsg()))
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
            client.deleteIdentity()          // secure_delete + VACUUM no Rust (e os ficheiros dos anexos)
            Push.deleteToken(getApplication())   // a Google esquece este telemovel (o relay ja apagou o token)
            // Depois apaga os ficheiros e as chaves (incluindo o bloqueio) e abre um armazenamento novo.
            resetStorage()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_apagar_os_dados, e.safeMsg()))
        }
    }

    fun dismissWipeServerFailed() = _ui.update { it.copy(wipeServerFailed = null) }

    /** Pede ao relay para apagar a conta. `null` se apagou; senao, o motivo (para mostrar). */
    private suspend fun deleteOnRelay(client: QtClient): String? {
        val l = link ?: return s(R.string.t_sem_ligacao_ao_servidor)
        return try {
            when (val r = l.request(client.relayRequestDeleteAccount())) {
                is RelayResponse.Ack -> null
                is RelayResponse.Error ->
                    if (r.reason == RELAY_UNKNOWN_REQUEST) s(R.string.t_o_servidor_ainda_nao_tem_esta) else s(R.string.t_o_servidor_respondeu, r.reason)
                else -> s(R.string.t_resposta_inesperada_do_servidor)
            }
        } catch (e: Exception) {
            s(R.string.t_sem_ligacao_ao_servidor)
        }
    }

    fun setReadReceipts(on: Boolean) {
        prefs.edit().putBoolean("read_receipts", on).apply()
        _ui.update { it.copy(readReceipts = on) }
    }

    private suspend fun markRead(client: QtClient, contactId: String) {
        // "Visto" so para contactos: a quem ainda nao aceitei (convite) nao se diz que li.
        val receipts = _ui.value.readReceipts && _ui.value.contacts.any { it.contactId == contactId }
        if (client.markConversationRead(contactId, receipts) > 0u) {
            flushOutbox()
        }
        refreshViews(client)
    }

    /**
     * Recarrega a lista de conversas e as mensagens mais recentes da conversa aberta (estados
     * ✓/✓✓, reacoes e mensagens novas). So relê ate REFRESH_MAX mensagens, mesmo que o
     * utilizador tenha subido muito na conversa: as mais antigas ficam como estavam (ver
     * `mergeWindow`; as que mudam la em cima sao relidas uma a uma, `refreshMessages`).
     */
    private fun refreshViews(client: QtClient) {
        val st = _ui.value
        val cid = st.openChatId
        val n = st.chatMessages.size.coerceIn(PAGE, REFRESH_MAX)
        val window = if (cid != null) client.messagesPage(cid, null, n.toUInt()) else null
        val invites = client.pendingIntros().toSet()
        val groups = client.groups()
        val online = client.presenceOnline().toSet()
        // Quem anunciou que tem video chega como mensagem: o botao da videochamada aparece.
        val video = runCatching { client.callVideoContacts().toSet() }.getOrDefault(emptySet())
        // Lida antes do update: a funcao do update pode correr mais de uma vez (concorrencia).
        val conversations = client.conversations()
        _ui.update {
            it.copy(
                invites = invites,
                groups = groups,
                online = online,
                videoContacts = video,
                conversations = conversations,
                chatMessages = if (window != null && it.openChatId == cid) mergeWindow(window, it.chatMessages, n) else it.chatMessages,
            )
        }
    }

    /** Janela nova (as `n` mais recentes) + as mais antigas que ja estavam carregadas. */
    private fun mergeWindow(fresh: List<ChatMessage>, current: List<ChatMessage>, n: Int): List<ChatMessage> {
        if (fresh.size < n) return fresh   // a conversa inteira cabe na janela
        val oldest = fresh.last().seq
        return fresh + current.filter { it.seq < oldest }
    }

    /** Rele mensagens concretas da conversa aberta (ex.: reacao ou recibo numa mensagem antiga). */
    private fun refreshMessages(client: QtClient, conversationId: String, ids: Collection<String>) {
        if (ids.isEmpty() || _ui.value.openChatId != conversationId) return
        val loaded = _ui.value.chatMessages.map { it.messageId }.toSet()
        val updated = ids.filter { it in loaded }.mapNotNull { id -> runCatching { client.message(conversationId, id) }.getOrNull() }
            .associateBy { it.messageId }
        if (updated.isEmpty()) return
        _ui.update {
            if (it.openChatId != conversationId) it
            else it.copy(chatMessages = it.chatMessages.map { m -> updated[m.messageId] ?: m })
        }
    }

    // ---------------------------------------------------------------------- enviar

    fun send(contact: Contact, text: String) = io {
        val client = qt ?: return@io
        val bytes = text.trim().toByteArray(Charsets.UTF_8)
        if (bytes.isEmpty() || bytes.size > MAX_TEXT_BYTES) {
            unsent(contact.contactId, text)
            error(s(R.string.t_mensagem_vazia_ou_maior_que_bytes, MAX_TEXT_BYTES)); return@io
        }
        try {
            ensureSession(client, contact)
            // Numa so transacao no Rust: ratchet avanca + mensagem guardada + envelope na outbox.
            client.sendText(contact.contactId, text.trim())
        } catch (e: Exception) {
            unsent(contact.contactId, text)
            error(s(R.string.t_nao_foi_possivel_enviar_2, e.safeMsg()))
            return@io
        }
        refreshViews(client)
        flushOutbox()
    }

    /** A mensagem nao ficou guardada: o texto volta a caixa de escrita (antes perdia-se). */
    private fun unsent(conversationId: String, text: String) = _ui.update { it.copy(unsentText = conversationId to text) }

    fun draftRestored() { if (_ui.value.unsentText != null) _ui.update { it.copy(unsentText = null) } }

    /** Envia uma mensagem de voz gravada (`file` e apagado no fim, cifrado ou nao). */
    fun sendVoice(contact: Contact, file: File, durationMs: Long) = sendVoiceTo(contact.contactId, file, durationMs)

    /** Voz para uma conversa a dois ou de grupo ("g:..."). */
    fun sendVoiceTo(conversationId: String, file: File, durationMs: Long) = io {
        if (premiumBlocked()) { file.delete(); return@io }
        val client = qt ?: run { file.delete(); return@io }
        try {
            prepareTarget(client, conversationId)
            client.attachmentCreate(conversationId, file.path, VOICE_MIME, durationMs.coerceIn(1L, VOICE_MAX_MS).toUInt(), 0u)
            refreshViews(client)
            transferKick.trySend(Unit)
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_enviar_a_mensagem, e.safeMsg()))
        } finally {
            file.delete()
        }
    }

    /** Reagir com um emoji a uma mensagem (vazio = retirar a minha reacao). */
    fun react(contactId: String, messageId: String, emoji: String) = io {
        val client = qt ?: return@io
        try {
            client.react(contactId, messageId, emoji)
            refreshViews(client)
            refreshMessages(client, contactId, listOf(messageId))
            flushOutbox()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_reagir, e.safeMsg()))
        }
    }

    // ---------------------------------------------------------------------- grupos
    //
    // Envio a cada membro (ver ffi/src/groups.rs): o relay so ve envelopes 1 a 1. Para
    // escrever no grupo e preciso ter sessao com cada membro; quem ainda nao a tem fica
    // de fora ate a sessao abrir (normalmente no envio seguinte).

    /** Abre sessao com quem ainda nao a tem (bundle no relay). Quem falhar fica para depois. */
    private suspend fun ensureSessions(client: QtClient, publicIds: List<String>) {
        val l = link ?: return
        for (pid in publicIds) {
            val cid = pid.substring(4, 68)
            if (cid in _ui.value.blocked || client.hasSession(cid)) continue
            try {
                val r = l.request(client.relayRequestFetchBundle(relayIdOfPublicId(pid)))
                if (r is RelayResponse.Bundle) client.startSession(pid, peerBundleFromRelayBytes(r.bytes))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "sessao com membro do grupo: ${e.safeMsg()}")
            }
        }
    }

    /** NOVO GRUPO: eu (administrador) + ate 9 contactos. Abre a conversa do grupo. */
    fun createGroup(name: String, members: List<Contact>) = io {
        if (premiumBlocked()) return@io
        val client = qt ?: return@io
        try {
            ensureSessions(client, members.map { it.publicId })
            val g = client.groupCreate(name.trim(), members.map { it.publicId })
            refreshViews(client)
            flushOutbox()
            openChat(g.conversationId)
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_criar_o_grupo, e.safeMsg()))
        }
    }

    fun sendGroup(groupId: String, text: String) = io {
        val client = qt ?: return@io
        val conv = _ui.value.groups.firstOrNull { it.groupId == groupId }?.conversationId
        val bytes = text.trim().toByteArray(Charsets.UTF_8)
        if (bytes.isEmpty() || bytes.size > MAX_TEXT_BYTES) {
            conv?.let { unsent(it, text) }
            error(s(R.string.t_mensagem_vazia_ou_maior_que_bytes, MAX_TEXT_BYTES)); return@io
        }
        try {
            ensureSessions(client, client.groupMembersNeedingSession(groupId))
            client.groupSendText(groupId, text.trim())
        } catch (e: Exception) {
            conv?.let { unsent(it, text) }
            error(s(R.string.t_nao_foi_possivel_enviar_2, e.safeMsg()))
            return@io
        }
        refreshViews(client)
        flushOutbox()
    }

    fun reactGroup(groupId: String, messageId: String, emoji: String) = io {
        val client = qt ?: return@io
        try {
            ensureSessions(client, client.groupMembersNeedingSession(groupId))
            client.groupReact(groupId, messageId, emoji)
            refreshViews(client)
            client.group(groupId)?.let { refreshMessages(client, it.conversationId, listOf(messageId)) }
            flushOutbox()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_reagir, e.safeMsg()))
        }
    }

    /** MUDAR O GRUPO (so administradores): nome, membros e administradores (ate 3). */
    fun updateGroup(groupId: String, name: String, members: List<GroupMemberInput>) = io {
        val client = qt ?: return@io
        try {
            ensureSessions(client, members.map { it.publicId })
            client.groupUpdate(groupId, name.trim(), members)
            refreshViews(client)
            flushOutbox()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_mudar_o_grupo, e.safeMsg()))
        }
    }

    /** SAIR DO GRUPO: os outros sao avisados; a conversa fica ate ser apagada. */
    fun leaveGroup(groupId: String) = io {
        val client = qt ?: return@io
        try {
            client.groupLeave(groupId)
            refreshViews(client)
            flushOutbox()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_sair_do_grupo, e.safeMsg()))
        }
    }

    /** APAGAR GRUPO deste telemovel (sai, se ainda estiver, e apaga as mensagens). */
    fun deleteGroup(groupId: String) = io {
        val client = qt ?: return@io
        AttachmentCache.clear()   // nada do que vai ser apagado fica decifrado em memoria
        try {
            val conv = client.group(groupId)?.conversationId
            client.groupDelete(groupId)
            if (conv != null && _ui.value.openChatId == conv) closeChat()
            refreshViews(client)
            flushOutbox()
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_apagar_o_grupo, e.safeMsg()))
        }
    }

    // ---------------------------------------------------------------- imagens/GIFs

    /**
     * Envia uma imagem/GIF. `uri` vem da galeria ou e uma copia do que o teclado
     * mandou (`keyboardCopy`, apagada no fim). O ficheiro e preparado no telemovel,
     * cifrado pelo Rust e enviado em segundo plano (ver `runTransfers`).
     */
    fun sendImage(contact: Contact, uri: Uri, original: Boolean, keyboardCopy: File? = null) =
        sendImageTo(contact.contactId, uri, original, keyboardCopy)

    /** Imagem/GIF para uma conversa a dois ou de grupo ("g:..."). */
    fun sendImageTo(conversationId: String, uri: Uri, original: Boolean, keyboardCopy: File? = null) = io {
        if (premiumBlocked()) { keyboardCopy?.delete(); return@io }
        val client = qt ?: run { keyboardCopy?.delete(); return@io }
        val prepared = try {
            ImagePrep.prepare(getApplication<Application>(), uri, original, keepAsIs = keyboardCopy != null)
        } catch (e: ImageTooBigException) {
            error(s(R.string.t_a_imagem_tem_mais_de_25))
            return@io
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_ler_esta_imagem))
            return@io
        } finally {
            keyboardCopy?.delete()
        }
        try {
            prepareTarget(client, conversationId)
            client.attachmentCreate(conversationId, prepared.file.path, prepared.mime, prepared.width.toUInt(), prepared.height.toUInt())
            refreshViews(client)
            transferKick.trySend(Unit)
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_enviar_a_imagem, e.safeMsg()))
        } finally {
            prepared.file.delete()
        }
    }

    /** Ficheiro (PDF, documento...) ate 25 MB, tal como esta. */
    fun sendFile(contact: Contact, uri: Uri) = sendFileTo(contact.contactId, uri)

    /** Ficheiro para uma conversa a dois ou de grupo ("g:..."). O original nao e alterado. */
    fun sendFileTo(conversationId: String, uri: Uri) = io {
        if (premiumBlocked()) return@io
        val client = qt ?: return@io
        val prepared = try {
            FilePrep.prepare(getApplication<Application>(), uri)
        } catch (e: FileTooBigException) {
            error(s(R.string.t_o_ficheiro_tem_mais_de_25_2))
            return@io
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_ler_este_ficheiro))
            return@io
        }
        try {
            prepareTarget(client, conversationId)
            client.attachmentCreateFile(conversationId, prepared.file.path, prepared.mime, prepared.name)
            refreshViews(client)
            transferKick.trySend(Unit)
        } catch (e: Exception) {
            error(s(R.string.t_nao_foi_possivel_enviar_o_ficheiro, e.safeMsg()))
        } finally {
            prepared.file.delete()
        }
    }

    /**
     * Antes de criar um anexo: a dois, abre a sessao com o contacto; num grupo, abre as
     * sessoes que faltam com os membros (quem falhar fica de fora, como no texto).
     */
    private suspend fun prepareTarget(client: QtClient, conversationId: String) {
        val gid = groupIdOfConversation(conversationId)
        if (gid != null) {
            ensureSessions(client, client.groupMembersNeedingSession(gid))
        } else {
            val contact = _ui.value.contacts.firstOrNull { it.contactId == conversationId }
                ?: throw IllegalStateException(s(R.string.err_unknown_contact))
            ensureSession(client, contact)
        }
    }

    /** O anexo decifrado, so em memoria (imagem para o ecra, voz, ou ficheiro para abrir/guardar). */
    suspend fun attachmentBytes(contactId: String, messageId: String): ByteArray? = withContext(Dispatchers.IO) {
        AttachmentCache.get(contactId, messageId)?.let { return@withContext it }
        try {
            val client = qt ?: return@withContext null
            val bytes = client.attachmentRead(contactId, messageId)
            // Ficheiros (PDF...) nao ficam em cache: sao entregues a outra app ou guardados e depois apagados da memoria.
            if (client.message(contactId, messageId)?.attachment?.fileName == null) {
                AttachmentCache.put(contactId, messageId, bytes)
            }
            bytes
        } catch (e: Exception) {
            null
        }
    }

    private fun setProgress(messageId: String, p: Float?) = _ui.update {
        it.copy(transfers = if (p == null) it.transfers - messageId else it.transfers + (messageId to p))
    }

    /** Recusas do relay (ou erros do nucleo) seguidas, por anexo. So em memoria. */
    private val transferFails = java.util.concurrent.ConcurrentHashMap<String, Int>()

    /**
     * Um anexo que o relay recusa (ou que o nucleo nao consegue tratar) salta-se e os outros
     * seguem (0.9.23: antes o primeiro que falhava parava a fila toda). Ao fim de
     * [TRANSFER_MAX_TRIES] fica como falhado.
     */
    private fun transferRefused(client: QtClient, r: AttachmentRef, e: Exception) {
        setProgress(r.messageId, null)
        val n = (transferFails[r.messageId] ?: 0) + 1
        Log.w(TAG, "anexo recusado ($n/$TRANSFER_MAX_TRIES): ${e.safeMsg()}")
        if (n < TRANSFER_MAX_TRIES) { transferFails[r.messageId] = n; return }
        transferFails.remove(r.messageId)
        runCatching { client.attachmentFailed(r.contactId, r.messageId, false) }
        refreshViews(client)
    }

    /**
     * Envia e descarrega os anexos pendentes (tambem os que ficaram a meio antes de a
     * app fechar). Devolve true se algo falhou e deve ser tentado de novo. Sem rede, para logo
     * (os outros tambem falhariam); uma recusa so salta esse anexo.
     */
    private suspend fun runTransfers(): Boolean {
        var retry = false
        val client = qt ?: return false
        val l = link ?: return false
        for (r in client.attachmentsPendingUpload()) {
            val gid = groupIdOfConversation(r.contactId)
            val beforeSend: suspend () -> Unit
            if (gid != null) {
                // Grupo de que ja sai (ou apagado): o anexo nao chega a sair.
                if (client.group(gid)?.active != true) {
                    client.attachmentFailed(r.contactId, r.messageId, false)
                    refreshViews(client)
                    continue
                }
                beforeSend = { ensureSessions(client, client.groupMembersNeedingSession(gid)) }
            } else {
                if (r.contactId in _ui.value.blocked) continue   // bloqueado: nao se envia (o Rust recusaria)
                val contact = _ui.value.contacts.firstOrNull { it.contactId == r.contactId } ?: continue
                beforeSend = { ensureSession(client, contact) }
            }
            try {
                upload(client, l, r, gid, beforeSend)
                transferFails.remove(r.messageId)
            } catch (e: CancellationException) {
                throw e
            } catch (e: RelayRefusedException) {
                transferRefused(client, r, e); retry = true
            } catch (e: QtException) {
                transferRefused(client, r, e); retry = true
            } catch (e: Exception) {
                setProgress(r.messageId, null)
                return true
            }
        }
        for (r in client.attachmentsPendingDownload()) {
            try {
                download(client, l, r)
                transferFails.remove(r.messageId)
            } catch (e: CancellationException) {
                throw e
            } catch (e: RelayRefusedException) {
                transferRefused(client, r, e); retry = true
            } catch (e: QtException) {
                transferRefused(client, r, e); retry = true
            } catch (e: Exception) {
                setProgress(r.messageId, null)
                return true
            }
        }
        return retry
    }

    private suspend fun upload(client: QtClient, l: RelayLink, r: AttachmentRef, groupId: String?, beforeSend: suspend () -> Unit) {
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
                    if (resp.reason == relayPremiumRequiredReason()) premiumRefused() else error(relayRefusal(resp.reason))
                    return
                }
                else -> throw RelayRefusedException("resposta inesperada do relay")
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
                else -> throw RelayRefusedException("resposta inesperada do relay")
            }
        }
        var offset = start
        while (offset < total) {
            val batch = generateSequence(offset) { it + CHUNK }.takeWhile { it < total }.take(WINDOW).toList()
            val answers = l.requestMany(batch.map { client.blobPutRequest(cid, mid, it) })
            if (answers.any { it !is RelayResponse.BlobStatus }) throw RelayRefusedException("bloco recusado")
            offset = batch.last() + CHUNK
            setProgress(mid, (minOf(offset, total).toDouble() / total.toDouble()).toFloat())
        }
        beforeSend()
        // A mensagem com a chave entra na outbox (num grupo, uma por membro).
        if (groupId != null) client.groupAttachmentUploaded(groupId, mid) else client.attachmentUploaded(cid, mid)
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
                    else -> throw RelayRefusedException("resposta inesperada do relay")
                }
            }
            val now = total
            if (now != null && now > 0uL) setProgress(mid, (have.toDouble() / now.toDouble()).toFloat())
        }
        val done = client.attachmentDownloadFinished(cid, mid)
        setProgress(mid, null)
        refreshViews(client)
        // Ja esta no telemovel (e verificado): o relay pode apagar a copia dele. Nos grupos
        // nao: os outros membros ainda a podem estar a descarregar (o relay apaga ao fim de 24 h).
        if (done.attachment?.state == AttachmentState.READY && groupIdOfConversation(cid) == null) {
            runCatching { l.request(client.blobDeleteRequest(cid, mid)) }
        }
    }

    private fun relayRefusal(reason: String): String = when {
        reason.contains("invalida") || reason.contains("desativados") ->
            s(R.string.t_o_servidor_ainda_nao_aceita_anexos)
        reason.contains("limite") -> s(R.string.att_daily_limit)
        reason.contains("espaco") -> s(R.string.att_no_space)
        else -> s(R.string.t_nao_foi_possivel_enviar_o_anexo, reason)
    }

    /**
     * Um contacto apagou-me e as chaves dele ("sessao terminada"): abre ja uma sessao nova com
     * ele, para o que estava por entregar seguir (o Rust volta a cifra-lo, com o convite).
     * Membros de grupos que nao estao nos contactos: tratados pelo ciclo das sessoes dos grupos.
     */
    private suspend fun restartSessions(client: QtClient) {
        val ids = client.sessionsToRestart()
        if (ids.isEmpty()) return
        var started = false
        for (cid in ids) {
            val c = _ui.value.contacts.firstOrNull { it.contactId == cid } ?: continue
            if (cid in _ui.value.blocked) continue
            try {
                ensureSession(client, c)
                started = true
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "sessao nova depois de terminada: ${e.safeMsg()}")
            }
        }
        if (started) {
            refreshViews(client)
            flushOutbox()
        }
    }

    private suspend fun ensureSession(client: QtClient, c: Contact) {
        if (client.hasSession(c.contactId)) return
        val l = link ?: throw IllegalStateException(s(R.string.t_sem_ligacao_ao_servidor))
        when (val r = l.request(client.relayRequestFetchBundle(c.relayId))) {
            is RelayResponse.Bundle -> client.startSession(c.publicId, peerBundleFromRelayBytes(r.bytes))
            is RelayResponse.BundleNotFound -> throw IllegalStateException(s(R.string.err_contact_no_keys))
            else -> throw IllegalStateException(s(R.string.t_resposta_inesperada_do_servidor))
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
        // Membros dos grupos que nao estao nos contactos: o endereco vem do Public ID na info do grupo.
        val members by lazy {
            client.groups().flatMap { it.members }.associate { it.contactId to it.publicId }
        }
        for (item in client.outbox()) {
            // O relay identifica caixas pela chave de assinatura (do Public ID do contacto).
            val relayId = item.relayId   // aviso "sessao terminada" a quem ja nao esta nos contactos
                ?: _ui.value.contacts.firstOrNull { it.contactId == item.contactId }?.relayId
                ?: members[item.contactId]?.let { runCatching { relayIdOfPublicId(it) }.getOrNull() }
                ?: continue
            val r = try {
                l.request(client.relayRequestSendItem(relayId, item))   // recibos/presenca: sem campainha
            } catch (e: Exception) {
                break   // sem rede: tenta no proximo ciclo, mantendo a ordem
            }
            if (r is RelayResponse.Ack) {
                client.outboxSent(item.outboxId)
                changed = true
            } else if (r is RelayResponse.Error && r.reason == relayPremiumRequiredReason()) {
                // Chamada sem premium (o operador tirou-o entretanto): a oferta nao segue.
                client.outboxSent(item.outboxId)
                changed = true
                if (_ui.value.call?.phase == CallPhase.CALLING) endCallLocal(s(R.string.t_chamada_terminada))
                premiumRefused()
            } else if (r is RelayResponse.Error && r.reason == relayRecipientDeletedReason()) {
                // O contacto apagou a conta: sai da outbox (nunca seria aceite; antes ficava la
                // para sempre) e a mensagem fica por enviar. Avisa uma vez por sessao.
                runCatching { client.outboxDrop(item.outboxId) }
                changed = true
                if (deletedNotified.add(item.contactId)) {
                    val name = _ui.value.contacts.firstOrNull { it.contactId == item.contactId }?.nickname ?: s(R.string.t_este_contacto)
                    error(s(R.string.t_apagou_a_conta_sealer_as_mensagens, name))
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
     * Com um relay que suporta entrega imediata, a app fica a espera (na ligacao de sempre:
     * qualquer outro pedido acaba a espera, ver RelayLink) e o relay responde no instante em que chega uma mensagem (ou ao fim de
     * WAIT_SECONDS, e volta-se a esperar). Um relay antigo responde Error ao pedido de espera:
     * entao volta-se ao metodo antigo, perguntar de POLL_MS em POLL_MS. Sem rede, tambem.
     */
    private suspend fun receiveLoop(client: QtClient) {
        // Se o relay disser que nao conhece o pedido de espera (relay antigo), pergunta-se de
        // POLL_MS em POLL_MS, mas volta-se a tentar a espera de WAIT_RETRY_MS em WAIT_RETRY_MS:
        // o relay pode ter sido atualizado entretanto (nunca desistir para sempre).
        var waitBlockedUntil = 0L
        var first: List<RelayDelivered>? = null
        // A ultima espera acabou cedo e vazia porque outro pedido a interrompeu (um envio, um
        // bloco de anexo...): volta-se logo a esperar, sem perguntar outra vez (a fila estava vazia).
        var interrupted = false
        while (currentCoroutineContext().isActive) {
            if (!interrupted) pollOnce(first)
            first = null
            interrupted = false
            val w = link
            if (w == null || SystemClock.elapsedRealtime() < waitBlockedUntil) { delay(POLL_MS); continue }
            try {
                val t0 = SystemClock.elapsedRealtime()
                when (val r = w.request(client.relayRequestWait(WAIT_SECONDS.toUInt()), WAIT_TIMEOUT_MS)) {
                    is RelayResponse.Messages -> {
                        first = r.messages.ifEmpty { null }
                        // Menos de 1 s e vazia: foi interrompida. Uma pausa curta evita um ciclo
                        // apertado se algo correr mal (com anexos ha uma interrupcao por lote).
                        if (first == null && SystemClock.elapsedRealtime() - t0 < INTERRUPTED_MS) {
                            interrupted = true
                            delay(INTERRUPT_PAUSE_MS)
                        }
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
    private suspend fun pollOnce(first: List<RelayDelivered>? = null) = SealerCore.receiveLock.withLock {
        pollOnceLocked(first)
    }

    private suspend fun pollOnceLocked(first: List<RelayDelivered>?) {
        val client = qt ?: return
        val l = link ?: return
        try {
            if (first == null) flushOutbox()
            var received = false
            var updated = false   // foto, badge, presenca, reacao ou grupo: algo alem de mensagens
            var pending = first
            for (page in 0 until MAX_PAGES) {
                val msgs = pending ?: (l.request(client.relayRequestFetch()) as? RelayResponse.Messages)?.messages ?: break
                pending = null
                if (msgs.isEmpty()) break
                val ackIds = mutableListOf<ULong>()
                var gotPage = false
                val statusIds = mutableMapOf<String, MutableSet<String>>()   // recibos: conversa -> mensagens
                var newMessage = false
                for (m in msgs) {
                    try {
                        // Decifrar + guardar no historico + agendar o recibo, tudo numa transacao.
                        when (val r = client.receiveSealed(m.ciphertext)) {
                            is ReceiveResult.StatusChanged -> {
                                received = true; gotPage = true
                                statusIds.getOrPut(r.contactId) { mutableSetOf() }.addAll(r.messageIds)
                            }
                            is ReceiveResult.NewMessage -> { received = true; gotPage = true; newMessage = true }
                            is ReceiveResult.Updated -> { received = true; gotPage = true; updated = true }
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
                // App em segundo plano (ainda ligada, nos primeiros 30 s): o relay nao toca a
                // campainha porque a ligacao de espera estava aberta, por isso o aviso e daqui.
                if (newMessage && !Push.appVisible) Push.showGeneric(getApplication())
                if (gotPage) {
                    refreshViews(client)
                    for ((conv, ids) in statusIds) refreshMessages(client, conv, ids)
                }
                if (ackIds.isNotEmpty()) l.request(client.relayRequestAck(ackIds))   // so DEPOIS de guardar
            }
            restartSessions(client)
            handleCallEvents(client)
            if (received) {
                // Fotos e badges recebidos (Updated) ja; por mensagens novas (contactos com sessao
                // nova), no maximo de minuto a minuto. Antes relia as fotos e os badges de todos os
                // contactos a cada pagina recebida (0.9.23).
                val now = SystemClock.elapsedRealtime()
                if (updated || now - lastPhotoSync >= PHOTO_SYNC_MS) {
                    lastPhotoSync = now
                    syncPhotos(client)
                }
                transferKick.trySend(Unit)   // imagens recebidas: descarregar ja
                client.prepareDeliveryReceipts()
                // A lista ja foi refrescada a cada pagina; so a conversa aberta precisa do "Visto".
                _ui.value.openChatId?.let { markRead(client, it) }
                flushOutbox()
            }
            status("Ligado ao relay")
        } catch (e: Exception) {
            status("Sem ligacao ao relay: ${e.safeMsg()}")
        }
    }

    /** Ultima vez que [syncPhotos] correu por causa de mensagens recebidas. */
    @Volatile private var lastPhotoSync = 0L

    // ---------------------------------------------------------------- chamadas de voz

    private fun pinBytes(pinHex: String): ByteArray? {
        val hex = pinHex.trim()
        if (hex.length != 64) return null
        return ByteArray(32) { i -> hex.substring(i * 2, i * 2 + 2).toInt(16).toByte() }
    }

    /**
     * Arranca o motor de audio e entra ja na sala do relay das chamadas (enquanto toca).
     * Federacao (fase 4): os dois telemoveis usam o relay das chamadas de quem liga (vem na
     * oferta; sem nome, o principal), mesmo que as contas estejam em relays diferentes.
     */
    private fun startCallAudio(client: QtClient, callId: String) {
        if (callAudio != null) return
        val r = Relays.forCall(prefs, runCatching { client.callRelay(callId) }.getOrNull())
        val pin = pinBytes(r.pin) ?: run { Log.w(TAG, "chamada: sem a chave do relay"); return }
        callAudio = client.callAudioStart(callId, r.url, pin)
    }

    /** Liga o microfone e o altifalante (a chamada foi atendida). */
    private fun startCallIO() {
        if (callIO != null) return
        val a = callAudio ?: return
        val app = getApplication<Application>()
        try {
            callIO = CallAudioIO(app, a).also {
                it.onMicLost = { _ui.update { u -> u.copy(notice = s(R.string.call_mic_lost)) } }
                it.start()
                it.muted = _ui.value.call?.muted == true
                // O altifalante escolhido antes de atender continua (0.9.30; antes perdia-se).
                if (_ui.value.call?.speaker == true) it.setSpeaker(true)
            }
        } catch (e: Exception) {
            callIO = null
            callSounds.leaveRoute()
            error(s(R.string.t_nao_foi_possivel_usar_o_microfone_2, e.safeMsg()))
        }
    }

    private fun stopCallMedia() {
        cameraHeld = false
        stopVideo()
        callSounds.stop()
        // Sem CallAudioIO (ninguem atendeu ou o microfone falhou) nao ha quem devolva o som ao normal.
        callIO?.stop() ?: callSounds.leaveRoute()
        callIO = null
        callAudio?.stop()
        // Depois do `stop` o Rust guarda o resumo final da chamada (so com o diagnostico ligado).
        if (CallDiag.enabled) {
            runCatching { callAudio?.diagnostic() }.getOrNull()?.takeIf { it.isNotBlank() }?.let { d ->
                val text = CallDiag.withHeader(d)
                CallDiag.log(TAG, "chamada terminada: " + d.trim().replace("\n", " | "))
                _ui.update { it.copy(lastCallDiag = text) }
            }
        }
        callAudio?.destroy()
        callAudio = null
        callTicker?.cancel()
        callTicker = null
        CallControl.hangup = null
        val app = getApplication<Application>()
        CallService.stop(app)
        CallNotify.clear(app)
    }

    private var callStatsTick = 0

    private fun ensureCallTicker() {
        if (callTicker?.isActive == true) return
        callTicker = viewModelScope.launch(Dispatchers.IO) {
            while (isActive && _ui.value.call != null) {
                delay(1_000L)
                val client = qt ?: break
                try {
                    handleCallEvents(client)
                    refreshVideoParams()
                    callAudio?.stats()?.let { st ->
                        val via = if (st.transport == "udp") "UDP" else if (st.transport == "websocket") "WebSocket" else st.transport
                        val net = if (st.direct) "direto" + (if (st.bothPaths) " + servidor" else "") else via
                        val rtt = st.relayRttMs?.let { " · ${it} ms ao servidor" } ?: ""
                        val lost = if (st.lost > 0uL) " · ${st.lost} perdidos na rede (${st.dredFrames} recuperados)" else ""
                        val late = if (st.late > 0uL) " · ${st.late} atrasados" else ""
                        val loss = "$lost$late"
                        // Os numeros da rede so vao para o registo (adb logcat), nunca para o ecra.
                        if (++callStatsTick % 5 == 0) CallDiag.log(TAG, "chamada: $net$rtt$loss")
                        if (CallDiag.enabled && callStatsTick % 10 == 0) {
                            runCatching { callAudio?.diagnostic() }.getOrNull()?.let { CallDiag.log(TAG, it.trim().replace("\n", " | ")) }
                        }
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    Log.w(TAG, "chamada: ${e.safeMsg()}")
                }
            }
        }
    }

    /**
     * LIGAR a um contacto. As permissoes (microfone; camara na videochamada) ja foram dadas
     * (MainActivity). `video`: videochamada, com a minha camara ligada desde o inicio (o outro
     * so a ve depois de atender e aceitar).
     */
    fun startCall(contact: Contact, video: Boolean = false) = io {
        if (premiumBlocked()) return@io
        val client = qt ?: return@io
        if (_ui.value.call != null) return@io
        if (contact.contactId in _ui.value.blocked) return@io
        try {
            ensureSession(client, contact)
            val start = if (video) client.callStartVideo(contact.contactId) else client.callStart(contact.contactId)
            val v = if (video) VideoUi(camera = true, accepted = true) else VideoUi()
            _ui.update { it.copy(call = CallUi(start.callId, contact.contactId, contact.nickname, CallPhase.CALLING, video = v, videoCall = video)) }
            CallControl.hangup = { hangupCall() }
            // "tuu... tuu" logo ao ligar (como no iPhone). Antes so comecava com o aviso "a tocar"
            // do outro telemovel, que nao chega com o bloqueio da app ligado nem sem rede (0.9.29).
            callSounds.ringback()
            flushOutbox()
            startCallAudio(client, start.callId)
            if (video) startSender()
            CallService.start(getApplication(), camera = video && !cameraHeld, video = video)
            ensureCallTicker()
        } catch (e: Exception) {
            stopCallMedia()
            _ui.update { it.copy(call = null) }
            error(s(R.string.t_nao_foi_possivel_ligar, e.safeMsg()))
        }
    }

    /** ATENDER a chamada que esta a tocar. */
    fun acceptCall() = io {
        val client = qt ?: return@io
        val c = _ui.value.call ?: return@io
        if (c.phase != CallPhase.INCOMING) return@io
        callSounds.stop()
        CallNotify.clear(getApplication())
        try {
            client.callAccept(c.callId)
            flushOutbox()
            startCallAudio(client, c.callId)
            CallService.start(getApplication(), camera = false, video = c.videoCall)
            _ui.update { it.copy(call = c.copy(phase = CallPhase.ACTIVE, answeredAt = SystemClock.elapsedRealtime())) }
            startCallIO()
            videoOnAnswered(client, c.callId)
            handleCallEvents(client)
        } catch (e: Exception) {
            endCallLocal(s(R.string.t_a_chamada_ja_nao_esta_disponivel))
        }
    }

    /** RECUSAR (a tocar), CANCELAR (a chamar) ou DESLIGAR (atendida): o motivo sai do estado. */
    fun hangupCall() = io {
        val c = _ui.value.call ?: return@io
        if (c.phase == CallPhase.ENDED) return@io
        try {
            qt?.callHangup(c.callId)
            flushOutbox()
        } catch (e: Exception) {
            Log.w(TAG, "desligar: ${e.safeMsg()}")
        }
        endCallLocal(if (c.phase == CallPhase.INCOMING) s(R.string.t_chamada_recusada) else s(R.string.t_chamada_terminada))
    }

    /** Diagnostico da chamada em curso (painel dos testers); `null` sem chamada ou desligado. */
    fun callDiagnostic(): String? =
        if (!CallDiag.enabled) null else runCatching { callAudio?.diagnostic() }.getOrNull()?.let { CallDiag.withHeader(it) }

    fun toggleMute() {
        val c = _ui.value.call ?: return
        val m = !c.muted
        callIO?.muted = m
        _ui.update { it.copy(call = c.copy(muted = m)) }
    }

    fun toggleSpeaker() {
        val c = _ui.value.call ?: return
        val on = !c.speaker
        // Antes de atender ainda nao ha microfone/altifalante da chamada: muda a saida do toque
        // (0.9.30; antes o botao so mudava o texto).
        val io = callIO
        if (io != null) runCatching { io.setSpeaker(on) } else callSounds.setSpeaker(on)
        _ui.update { it.copy(call = c.copy(speaker = on)) }
    }

    /** A chamada acabou: para o audio e mostra o motivo durante uns segundos. */
    private fun endCallLocal(text: String) {
        val c = _ui.value.call ?: return
        stopCallMedia()
        _ui.update { it.copy(call = c.copy(phase = CallPhase.ENDED, endText = text), answerNow = null) }
        // O bloqueio automatico que ficou adiado por causa da chamada.
        if (lockAfterCall) {
            lockAfterCall = false
            viewModelScope.launch {
                delay(ENDED_SHOW_MS + 300L)   // depois de o ecra "Chamada terminada" sair
                if (_ui.value.call == null) lockNow()
            }
        }
        viewModelScope.launch {
            delay(ENDED_SHOW_MS)
            _ui.update { u ->
                val cur = u.call
                if (cur != null && cur.callId == c.callId && cur.phase == CallPhase.ENDED) u.copy(call = null) else u
            }
        }
    }

    private fun endText(reason: CallEndReason, incoming: Boolean): String = when (reason) {
        CallEndReason.HANGUP -> s(R.string.t_chamada_terminada)
        CallEndReason.REJECTED -> s(R.string.t_chamada_recusada)
        CallEndReason.BUSY -> s(R.string.t_ocupado_noutra_chamada)
        CallEndReason.NO_ANSWER -> if (incoming) s(R.string.t_chamada_perdida) else s(R.string.t_sem_resposta)
        CallEndReason.CANCELLED -> if (incoming) s(R.string.t_chamada_perdida) else s(R.string.t_chamada_cancelada)
        CallEndReason.EXPIRED -> s(R.string.t_chamada_perdida)
    }

    /**
     * Uma chamada a tocar no segundo plano (app fechada, ver SealerCore) passa para o ecra.
     * Com o bloqueio da app nao se sabia qual era: aparece quando chegar a oferta (pollOnce).
     */
    private fun adoptRinging(client: QtClient) {
        // Ainda a mostrar outra chamada (que nao acabou): o toque fica no segundo plano.
        val shown = _ui.value.call
        if (shown != null && shown.phase != CallPhase.ENDED) return
        val r = SealerCore.takeRinging(getApplication()) ?: return
        val id = r.callId ?: return
        // Entretanto acabou (desligou, ou passou o tempo): fica so como perdida.
        if (runCatching { client.callCurrent() }.getOrNull() != id) {
            SealerCore.clearAnswer()
            return
        }
        // Nome: o do segundo plano, ou o dos contactos (com o bloqueio da app ele nao o sabia).
        val name = r.name ?: _ui.value.contacts.firstOrNull { it.contactId == r.contactId }?.nickname.orEmpty()
        val videoCall = r.video || runCatching { client.callIsVideo(id) }.getOrDefault(false)
        _ui.update { it.copy(call = CallUi(id, r.contactId.orEmpty(), name, CallPhase.INCOMING, incoming = true, videoCall = videoCall)) }
        CallControl.hangup = { hangupCall() }
        if (!SealerCore.answerRequested(id)) callSounds.ringIncoming()
        runCatching { startCallAudio(client, id) }
        ensureCallTicker()
        checkAutoAnswer()
    }

    /** "Atender" na notificacao (a Activity abriu com essa acao). */
    fun answerFromNotification() {
        // A chamada a que se carregou (com o bloqueio da app ainda nao se sabe: null).
        SealerCore.answerCallId = SealerCore.ringing?.callId ?: _ui.value.call?.callId
        SealerCore.answerRequestedAt = SystemClock.elapsedRealtime()
        checkAutoAnswer()
    }

    /** Ha um "Atender" da notificacao por usar e a chamada esta a tocar: o ecra atende (com a permissao do microfone). */
    private fun checkAutoAnswer() {
        val c = _ui.value.call ?: return
        if (c.phase != CallPhase.INCOMING || !SealerCore.answerRequested(c.callId)) return
        SealerCore.clearAnswer()
        callSounds.stop()
        _ui.update { it.copy(answerNow = c.callId) }
    }

    fun answerConsumed() = _ui.update { it.copy(answerNow = null) }

    /** Eventos das chamadas (vem do Rust, depois de receber mensagens e de segundo a segundo). */
    private suspend fun handleCallEvents(client: QtClient) {
        val events = SealerCore.callEvents(client)
        if (events.isEmpty()) return
        val app = getApplication<Application>()
        for (ev in events) {
            when (ev) {
                is CallEvent.Incoming -> {
                    val contact = _ui.value.contacts.firstOrNull { it.contactId == ev.contactId }
                    // So tocam os contactos guardados (e nao bloqueados). Os outros: recusa em
                    // silencio, sem lhes enviar nada (o aviso fica fora da outbox).
                    if (contact == null || ev.contactId in _ui.value.blocked) {
                        runCatching { client.callHangup(ev.callId)?.let { client.outboxSent(it.outboxId) } }
                        continue
                    }
                    // Ja ha outra chamada no ecra (o Rust so deixa uma de cada vez): recusa.
                    val shown = _ui.value.call
                    if (shown != null && shown.phase != CallPhase.ENDED) {
                        runCatching { client.callHangup(ev.callId) }
                        continue
                    }
                    // A op "videochamada" chega logo a seguir a oferta (normalmente no mesmo lote).
                    val videoCall = runCatching { client.callIsVideo(ev.callId) }.getOrDefault(false)
                    _ui.update { it.copy(call = CallUi(ev.callId, ev.contactId, contact.nickname, CallPhase.INCOMING, incoming = true, videoCall = videoCall)) }
                    CallControl.hangup = { hangupCall() }
                    callSounds.ringIncoming()
                    // Fora do ecra: notificacao de ecra inteiro (sem som: o toque e o de cima).
                    // Com o bloqueio da app, sem nome (a mesma regra que com a app fechada).
                    if (!visible) CallNotify.ringing(app, if (_ui.value.lockEnabled) null else contact.nickname, sound = false, video = videoCall)
                    runCatching { startCallAudio(client, ev.callId) }
                    ensureCallTicker()
                    checkAutoAnswer()
                }
                is CallEvent.Ringing -> _ui.update { u ->
                    val c = u.call
                    if (c != null && c.callId == ev.callId && c.phase == CallPhase.CALLING) {
                        // O toque ja esta a tocar desde o inicio: so muda o texto para "A tocar".
                        u.copy(call = c.copy(phase = CallPhase.RINGING_OUT))
                    } else u
                }
                is CallEvent.Answered -> {
                    val c = _ui.value.call
                    if (c != null && c.callId == ev.callId && c.phase != CallPhase.ACTIVE && c.phase != CallPhase.INCOMING) {
                        // O modo de chamada e a saida escolhida passam para o CallAudioIO.
                        callSounds.stop(keepRoute = true)
                        _ui.update { it.copy(call = c.copy(phase = CallPhase.ACTIVE, answeredAt = SystemClock.elapsedRealtime())) }
                        startCallIO()
                        videoOnAnswered(client, c.callId)
                    }
                }
                is CallEvent.Ended -> {
                    val c = _ui.value.call
                    if (c != null && c.callId == ev.callId && c.phase != CallPhase.ENDED) {
                        endCallLocal(endText(ev.reason, c.phase == CallPhase.INCOMING))
                    }
                }
                is CallEvent.Video -> onVideoEvent(ev)
            }
        }
        flushOutbox()
    }

    // ------------------------------------------------------------------ video (fase 5b)

    /** O outro lado: tem video, ligou/desligou a camara, aceitou ou recusou o meu. */
    private fun onVideoEvent(ev: CallEvent.Video) {
        val c = _ui.value.call ?: return
        if (c.callId != ev.callId) return
        val v = c.video
        // A camara dele voltou a ligar: e um pedido novo (a minha recusa antiga ja nao vale).
        val declined = if (ev.peerCamera && !v.peerCamera) false else v.declined
        val nv = v.copy(
            supported = ev.supported,
            peerCamera = ev.peerCamera,
            peerAccepts = ev.peerAccepts,
            peerDeclined = ev.peerDeclined,
            declined = declined,
        )
        // A tocar: a op "videochamada" pode chegar depois da oferta ("Videochamada de [Nome]").
        val videoCall = c.videoCall || (qt?.let { runCatching { it.callIsVideo(ev.callId) }.getOrNull() } ?: false)
        _ui.update { u -> u.call?.let { cur -> u.copy(call = cur.copy(video = nv, videoCall = videoCall)) } ?: u }
        // Fora do ecra, a tocar: o aviso de ecra inteiro passa a "Videochamada de [Nome]" (sem voltar a tocar).
        val stillRinging = _ui.value.call?.let { it.callId == c.callId && it.phase == CallPhase.INCOMING } == true
        if (videoCall && !c.videoCall && stillRinging && !visible) {
            CallNotify.ringing(
                getApplication(), if (_ui.value.lockEnabled) null else c.name,
                sound = false, video = true, update = true,
            )
        }
        // Recusou ver o meu video: desliga a minha camara (ninguem a ia ver), em silencio.
        if (ev.peerDeclined && v.camera && !v.peerDeclined) setCamera(false)
        syncReceiver()
    }

    /** O descodificador so corre com o video do outro aceite e no ecra. */
    private fun syncReceiver() {
        val v = _ui.value.call?.video ?: return
        val a = callAudio ?: return
        if (v.showRemote) {
            val r = videoReceiver ?: VideoReceiver(a) { w, h, rot ->
                _ui.update { u -> u.call?.let { cur -> u.copy(call = cur.copy(video = cur.video.copy(remoteW = w, remoteH = h, remoteRotation = rot))) } ?: u }
            }.also {
                videoReceiver = it
                it.setSurface(remoteSurface)
                it.start()
            }
            r.setSurface(remoteSurface)
        } else {
            videoReceiver?.stop()
            videoReceiver = null
        }
    }

    /** Chamada atendida: o outro lado tem video? E a rede (Wi-Fi ou dados) para o teto. */
    private fun videoOnAnswered(client: QtClient, callId: String) {
        val app = getApplication<Application>()
        val supported = runCatching { client.callVideoSupported(callId) }.getOrDefault(false)
        _ui.update { u ->
            u.call?.let { cur -> if (cur.callId == callId) u.copy(call = cur.copy(video = cur.video.copy(supported = cur.video.supported || supported))) else u } ?: u
        }
        callAudio?.setNetwork(VideoNet.current(app).also { CallDiag.log(TAG, "rede: $it") })
        if (stopNetWatch == null) stopNetWatch = VideoNet.watch(app) { kind ->
            CallDiag.log(TAG, "rede mudou: $kind")
            callAudio?.setNetwork(kind)
        }
    }

    /**
     * A cada segundo: videochamada para quem nao tem video, e se o video do outro parou (o ecra
     * mostra entao so a foto ou a inicial dele). Nada no ecra diz porque parou (rede, fichas).
     */
    private fun refreshVideoParams() {
        val c = _ui.value.call ?: return
        // Videochamada para quem afinal nao tem video (app antiga): a minha camara desliga-se.
        if (c.videoCall && c.phase == CallPhase.ACTIVE && c.video.camera && !c.video.supported &&
            SystemClock.elapsedRealtime() - c.answeredAt > NO_VIDEO_AFTER_MS
        ) {
            setCamera(false)
            return
        }
        val last = videoReceiver?.lastFrameAt ?: 0L
        val stalled = c.video.showRemote && (last == 0L || SystemClock.elapsedRealtime() - last > REMOTE_STALL_MS)
        if (stalled != c.video.remoteStalled) {
            _ui.update { u -> u.call?.let { cur -> u.copy(call = cur.copy(video = cur.video.copy(remoteStalled = stalled))) } ?: u }
        }
    }

    /** Botao da camara (a permissao ja foi pedida pela Activity). Ligar e um pedido ao outro. */
    fun toggleCamera() = io {
        val c = _ui.value.call ?: return@io
        if (c.phase != CallPhase.ACTIVE || !c.video.supported) return@io
        if (!c.video.camera && premiumBlocked()) return@io
        setCamera(!c.video.camera)
    }

    private fun setCamera(on: Boolean) {
        val client = qt ?: return
        val c = _ui.value.call ?: return
        try {
            client.callVideoSetCamera(c.callId, on)
        } catch (e: Exception) {
            Log.w(TAG, "camara: ${e.safeMsg()}")
            return
        }
        _ui.update { u ->
            u.call?.let { cur ->
                val v = cur.video
                // Ligar a minha camara tambem aceita o video dele.
                u.copy(call = cur.copy(video = if (on) v.copy(camera = true, accepted = true, declined = false, peerDeclined = false) else v.copy(camera = false)))
            } ?: u
        }
        viewModelScope.launch(Dispatchers.IO) { runCatching { flushOutbox() } }
        if (on) startSender() else stopSender()
        syncReceiver()
        CallService.update(getApplication(), camera = on && !cameraHeld)
    }

    /** Resposta ao pedido do outro: ver, ver e ligar a minha camara, ou agora nao. */
    fun respondVideo(response: VideoResponse) = io {
        val client = qt ?: return@io
        val c = _ui.value.call ?: return@io
        if (response == VideoResponse.VIEW_AND_CAMERA && premiumBlocked()) return@io
        try {
            client.callVideoRespond(c.callId, response)
        } catch (e: Exception) {
            Log.w(TAG, "video: ${e.safeMsg()}")
            return@io
        }
        _ui.update { u ->
            u.call?.let { cur ->
                val v = cur.video
                val nv = when (response) {
                    VideoResponse.VIEW -> v.copy(accepted = true, declined = false)
                    VideoResponse.VIEW_AND_CAMERA -> v.copy(accepted = true, declined = false, camera = true)
                    VideoResponse.NOT_NOW -> v.copy(accepted = false, declined = true)
                }
                u.copy(call = cur.copy(video = nv))
            } ?: u
        }
        flushOutbox()
        if (response == VideoResponse.VIEW_AND_CAMERA) {
            startSender()
            CallService.update(getApplication(), camera = !cameraHeld)
        }
        syncReceiver()
    }

    /** Frontal <-> traseira. */
    fun switchCamera() {
        val vs = videoSender ?: return
        vs.switchCamera()
        _ui.update { u -> u.call?.let { cur -> u.copy(call = cur.copy(video = cur.video.copy(front = !cur.video.front))) } ?: u }
    }

    private fun startSender() {
        synchronized(senderLock) {
            // A app fora do ecra: a camara so abre quando voltar (onAppResumed).
            if (videoSender != null || cameraHeld) return
            val a = callAudio ?: return
            val front = _ui.value.call?.video?.front ?: true
            videoSender = VideoSender(getApplication(), a).also { vs ->
                vs.onError = { _ui.update { it.copy(notice = s(R.string.video_camera_failed)) } }
                vs.onPreview = {
                    val size = vs.previewSize
                    if (size != null) {
                        _ui.update { u -> u.call?.let { cur -> u.copy(call = cur.copy(video = cur.video.copy(previewW = size.width, previewH = size.height))) } ?: u }
                    }
                }
                vs.setPreview(selfPreview)
                vs.start(front)
            }
        }
    }

    private fun stopSender() {
        synchronized(senderLock) {
            videoSender?.let { vs ->
                // Fechada por nos (ex.: segundo plano): uma queda da camara a meio nao e um erro a mostrar.
                vs.onError = null
                vs.stop()
            }
            videoSender = null
        }
    }

    /**
     * A app saiu do ecra (segundo plano ou ecra bloqueado) com a minha camara ligada: a camara
     * fecha e nada sai, sem avisar o outro lado (para ele o video para e ve so a minha foto ou
     * a inicial, sem texto). A voz continua. O botao fica "ligado".
     */
    private fun holdCameraForBackground() {
        val c = _ui.value.call ?: return
        if (c.phase == CallPhase.ENDED || !c.video.camera) return
        synchronized(senderLock) {
            if (cameraHeld) return
            cameraHeld = true
        }
        callAudio?.videoHold(true)
        stopSender()
        CallService.update(getApplication(), camera = false)
    }

    /**
     * A app voltou ao ecra (Activity.onResume): a camara volta sozinha se continuava ligada. O
     * tipo camara do servico primeiro (o Android so o da com a app no ecra), depois a camara.
     */
    fun onAppResumed() {
        val cur = _ui.value.call
        if (cur != null && cur.phase != CallPhase.INCOMING && cur.phase != CallPhase.ENDED) CallService.ensure(getApplication())
        synchronized(senderLock) {
            if (!cameraHeld) return
            cameraHeld = false
        }
        val c = _ui.value.call
        val on = c != null && c.phase != CallPhase.ENDED && c.video.camera
        if (on) CallService.update(getApplication(), camera = true)
        callAudio?.videoHold(false)
        if (on) startSender()
    }

    private fun stopVideo() {
        stopSender()
        videoReceiver?.stop()
        videoReceiver = null
        stopNetWatch?.invoke()
        stopNetWatch = null
    }

    /** A vista da minha camara (aparece ou desaparece com o ecra). */
    fun setSelfPreview(st: android.graphics.SurfaceTexture?) {
        selfPreview = st
        videoSender?.setPreview(st)
    }

    /** A vista do video do outro. */
    fun setRemoteSurface(surface: android.view.Surface?) {
        remoteSurface = surface
        videoReceiver?.setSurface(surface)
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
                if (qt != null) error(s(R.string.t_erro, e.safeMsg()))
            }
        }
    }

    override fun onCleared() {
        val owner = SealerCore.uiOwner === this
        if (owner) SealerCore.uiOwner = null
        // Chamada a meio: o outro lado e avisado ("desligou") antes de fechar o nucleo.
        val c = _ui.value.call
        val client = qt
        val endCall = client != null && c != null && c.phase != CallPhase.ENDED
        closeSession(closeCore = owner && !endCall)
        if (endCall) SealerCore.hangupInBackground(getApplication(), client!!, c!!.callId, closeAfter = owner)
        pendingBioKey?.fill(0)
    }

    companion object {
        private const val PREF_PREMIUM_ENFORCED = "premium_enforced"
        private const val PREF_LOCK_PENDING = "lock_pending"
        private const val PREF_PREMIUM_ACTIVE = "premium_active"
        private const val PREMIUM_TIMEOUT_MS = 4_000L
        /** Fichas do video: primeiro pedido entre 30 s e 10 min depois de abrir; depois 1-2 h. */
        private const val FICHAS_FIRST_MIN_MS = 30_000L
        private const val FICHAS_FIRST_MAX_MS = 600_000L
        private const val FICHAS_AGAIN_MS = 3_600_000L
        private const val FICHAS_TIMEOUT_MS = 20_000L
        private const val FICHAS_MAX_BATCHES = 3
        private const val FICHAS_KEYS_MAX_BYTES = 64 * 1024
        private const val PREF_PREMIUM_UNTIL = "premium_until"
        /** Copia de recuperacao: espera um pouco depois de uma mudanca (junta varias seguidas). */
        private const val BACKUP_DELAY_MS = 3_000L
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
        /** Uma espera vazia que acaba antes disto foi interrompida por outro pedido (0.9.45). */
        private const val INTERRUPTED_MS = 1_000L
        /** Pausa antes de voltar a esperar depois de uma interrupcao. */
        private const val INTERRUPT_PAUSE_MS = 20L
        /** Etiqueta dos registos (adb logcat -s SealerRelay). Nunca regista conteudo nem chaves. */
        private const val TAG = "SealerRelay"
        private const val MY_PHOTO = "\u0000eu"
        /** Resposta de um relay a um pedido que nao conhece (relay sem entrega imediata). */
        private const val RELAY_UNKNOWN_REQUEST = "mensagem invalida"
        private const val MAX_PAGES = 20
        private const val MAX_TEXT_BYTES = 9_000
        private const val PAGE = 50
        /** Maximo de mensagens relidas de cada vez que algo muda na conversa aberta. */
        private const val REFRESH_MAX = 200
        /** Bytes cifrados por bloco de anexo (igual ao Rust e ao relay). */
        private const val CHUNK = 40_960uL
        /**
         * Blocos em transito ao mesmo tempo (~160 KB; eram 8 ate a 0.9.44). Desde a 0.9.45 os
         * anexos partilham a ligacao com as mensagens: uma mensagem espera no maximo um lote.
         */
        private const val WINDOW = 4
        private const val RETRY_MS = 15_000L
        private const val RETRY_MAX_MS = 5 * 60_000L
        /** Recusas seguidas de um anexo antes de ficar como falhado. */
        private const val TRANSFER_MAX_TRIES = 5
        /** De quanto em quanto tempo se atualiza quem esta online (com a app aberta). */
        private const val PRESENCE_TICK_MS = 60_000L
        /** Fotos e badges dos contactos: relidos no maximo de minuto a minuto por mensagens novas. */
        private const val PHOTO_SYNC_MS = 60_000L
        /** Quanto tempo fica no ecra "Chamada terminada". */
        private const val ENDED_SHOW_MS = 2_000L
        /** Videochamada atendida sem "tenho video" do outro ao fim disto: a camara desliga-se. */
        private const val NO_VIDEO_AFTER_MS = 5_000L
        /** Sem imagens do outro ha mais disto: mostra a foto ou a inicial dele no lugar do video. */
        private const val REMOTE_STALL_MS = 2_000L
        fun minCodeLen(): Int = uniffi.qt_ffi.appLockMinCodeLen().toInt()
    }
}

/** O relay respondeu, mas recusou (ou respondeu outra coisa): nao e falta de rede. */
private class RelayRefusedException(msg: String) : IOException(msg)

private fun RelayResponse.describe(): String = when (this) {
    is RelayResponse.Error -> reason
    else -> this::class.simpleName ?: "?"
}

/**
 * Mensagem curta e SEGURA para mostrar/registar: nunca inclui conteudo nem chaves. Falhas de
 * rede aparecem como "Sem ligacao ao servidor" na lingua da app (0.9.23; antes apareciam os
 * textos tecnicos, em portugues sem acentos). O detalhe fica no registo do RelayLink.
 */
private fun Exception.safeMsg(): String = when (this) {
    is QtException -> this::class.simpleName ?: "erro"
    is RelayRefusedException -> message ?: "recusado"
    is IOException -> s(R.string.t_sem_ligacao_ao_servidor)
    else -> (message ?: this::class.simpleName ?: "erro").take(120)
}
