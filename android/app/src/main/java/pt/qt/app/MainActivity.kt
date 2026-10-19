package pt.qt.app

import android.app.KeyguardManager
import android.content.Intent
import android.view.WindowManager
import android.graphics.Color as AndroidColor
import android.os.Build
import android.os.Bundle
import androidx.activity.SystemBarStyle
import androidx.activity.compose.BackHandler
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.content.MediaType
import androidx.compose.foundation.content.ReceiveContentListener
import androidx.compose.foundation.content.consume
import androidx.compose.foundation.content.contentReceiver
import androidx.compose.foundation.content.hasMediaType
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import java.io.File
import android.Manifest
import android.content.pm.PackageManager
import androidx.compose.runtime.DisposableEffect
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.ui.layout.ContentScale
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import com.google.zxing.client.android.Intents
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.sin
import uniffi.qt_ffi.ChatMessage
import uniffi.qt_ffi.ConversationSummary
import uniffi.qt_ffi.MessageStatus

// FragmentActivity (e nao ComponentActivity): o BiometricPrompt precisa dela.
class MainActivity : FragmentActivity() {
    private val vm: MainViewModel by viewModels()
    private lateinit var bioPrompt: BiometricPrompt
    /**
     * Aberta por uma chamada com o telemovel bloqueado (notificacao de ecra inteiro): mostra-se
     * por cima do bloqueio, mas so com o ecra da chamada (ou o codigo da app). Quando a
     * chamada acaba, fecha; se o telemovel for desbloqueado, volta a app normal.
     */
    private val overLock = mutableStateOf(false)

    private val bioCallback = object : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
            vm.authInProgress = false
            val purpose = vm.bioPurpose
            vm.bioPurpose = null
            val cipher = result.cryptoObject?.cipher ?: return
            when (purpose) {
                BioPurpose.UNLOCK -> vm.unlockWithBio(cipher)
                BioPurpose.ENABLE -> vm.finishBioEnable(cipher)
                null -> {}
            }
        }

        override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
            vm.authInProgress = false
            val purpose = vm.bioPurpose
            vm.bioPurpose = null
            if (purpose == BioPurpose.ENABLE) vm.cancelBioEnable()
            val cancelled = errorCode == BiometricPrompt.ERROR_USER_CANCELED ||
                errorCode == BiometricPrompt.ERROR_NEGATIVE_BUTTON ||
                errorCode == BiometricPrompt.ERROR_CANCELED
            if (!cancelled) vm.bioMessage(errString.toString())
        }
    }

    // Antes do Android 13, a lingua escolhida no menu aplica-se aqui (no 13+ e o sistema).
    override fun attachBaseContext(newBase: android.content.Context) {
        super.attachBaseContext(Lang.localized(newBase))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        // Desenho de ponta a ponta (obrigatorio com o SDK 35+): a app desenha por baixo
        // das barras do sistema e cada ecra reserva o espaco delas (Scaffold/insets).
        // Barra de estado com icones claros, porque a barra de topo da app e escura.
        enableEdgeToEdge(statusBarStyle = SystemBarStyle.dark(AndroidColor.TRANSPARENT))
        super.onCreate(savedInstanceState)
        // Capturas de ecra permitidas. A miniatura na lista de apps recentes continua
        // escondida (Android 13+), para as conversas nao ficarem visiveis a quem
        // percorre as apps abertas.
        if (Build.VERSION.SDK_INT >= 33) setRecentsScreenshotEnabled(false)
        bioPrompt = BiometricPrompt(this, ContextCompat.getMainExecutor(this), bioCallback)
        setContent {
            SealerTheme {
                Surface(Modifier.fillMaxSize()) {
                    AppRoot(
                        vm,
                        overLock = overLock.value,
                        keyguardLocked = ::keyguardLocked,
                        onLeaveLock = ::leaveLock,
                        onDone = ::finishOverLock,
                    )
                }
            }
        }
        if (savedInstanceState == null) {
            handleCallIntent(intent)
        } else if (savedInstanceState.getBoolean(STATE_OVER_LOCK) && keyguardLocked()) {
            showOverLock(true)
            overLock.value = true
        }
        // Pedidos de autenticacao do ViewModel (desbloquear / ativar a biometria).
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                vm.bioRequests.collect { showBioPrompt(it) }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleCallIntent(intent)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putBoolean(STATE_OVER_LOCK, overLock.value)
    }

    /** Notificacao da chamada (tocar no aviso ou em "Atender"). */
    private fun handleCallIntent(i: Intent?) {
        val action = i?.action
        if (action != CallNotify.ACTION_SHOW && action != CallNotify.ACTION_ANSWER) return
        // So as notificacoes desta app (nao outra app, nem reabrir pelos recentes).
        if (i.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
        if (i.getStringExtra(CallNotify.EXTRA_TOKEN) != SealerCore.intentToken) return
        if (keyguardLocked()) {
            showOverLock(true)
            overLock.value = true
        }
        if (action == CallNotify.ACTION_ANSWER) vm.answerFromNotification()
    }

    private fun keyguardLocked(): Boolean =
        (getSystemService(KEYGUARD_SERVICE) as? KeyguardManager)?.isKeyguardLocked == true

    private fun showOverLock(on: Boolean) {
        if (Build.VERSION.SDK_INT >= 27) {
            setShowWhenLocked(on)
            setTurnScreenOn(on)
        } else {
            @Suppress("DEPRECATION")
            val flags = WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON
            if (on) window.addFlags(flags) else window.clearFlags(flags)
        }
    }

    /** O telemovel foi desbloqueado: a app volta ao normal (e deixa de aparecer por cima do bloqueio). */
    private fun leaveLock() {
        overLock.value = false
        showOverLock(false)
    }

    /** A chamada acabou com o telemovel ainda bloqueado: fecha a app. */
    private fun finishOverLock() {
        leaveLock()
        finish()
    }

    override fun onStart() {
        super.onStart()
        vm.onAppVisible()
    }

    override fun onStop() {
        super.onStop()
        if (!isChangingConfigurations) vm.onAppHidden()
    }

    private companion object {
        const val STATE_OVER_LOCK = "por_cima_do_bloqueio"
    }

    private fun showBioPrompt(req: BioRequest) {
        if (vm.bioPurpose != null) {   // ja ha um aberto: ignora o pedido repetido
            if (req.purpose == BioPurpose.ENABLE) vm.cancelBioEnable()
            return
        }
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(if (req.purpose == BioPurpose.UNLOCK) s(R.string.t_desbloquear, APP_NAME) else s(R.string.t_ativar_desbloqueio_rapido))
            .setSubtitle(s(R.string.t_impressao_digital_rosto_ou_pin_do))
            // Com o PIN do telemovel permitido, o Android nao aceita um botao "Cancelar" proprio.
            .setAllowedAuthenticators(BioUnlock.AUTHENTICATORS)
            .build()
        vm.bioPurpose = req.purpose
        vm.authInProgress = true
        try {
            bioPrompt.authenticate(info, BiometricPrompt.CryptoObject(req.cipher))
        } catch (e: Exception) {
            vm.authInProgress = false
            vm.bioPurpose = null
            if (req.purpose == BioPurpose.ENABLE) vm.cancelBioEnable()
            vm.bioMessage(s(R.string.t_nao_foi_possivel_pedir_a_autenticacao))
        }
    }
}

/**
 * Mantem a conversa colada a mensagem mais recente (lista com reverseLayout: o item 0 e o
 * fundo). Antes so descia se o primeiro item visivel fosse o 0 ou o 1, mas quando chegam
 * varias mensagens de uma vez a lista mantem visivel a mesma bolha (pela key), o indice
 * salta para 2 ou mais e deixava de descer. Agora "seguir" so se desliga quando a pessoa
 * arrasta a lista para cima, e volta a ligar-se quando ela regressa ao fundo. As minhas
 * mensagens descem sempre. Com muitas de uma vez, salta sem animacao (mais rapido).
 */
@Composable
internal fun FollowNewest(listState: LazyListState, newestKey: Long?, newestIsMine: Boolean) {
    var follow by remember { mutableStateOf(true) }
    val atBottom by remember {
        derivedStateOf { listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset <= 8 }
    }
    val dragged by listState.interactionSource.collectIsDraggedAsState()
    LaunchedEffect(atBottom, dragged) {
        if (atBottom) follow = true else if (dragged) follow = false
    }
    LaunchedEffect(newestKey) {
        if (newestKey == null || !(follow || newestIsMine)) return@LaunchedEffect
        follow = true
        if (listState.firstVisibleItemIndex > 6) listState.scrollToItem(0) else listState.animateScrollToItem(0)
    }
}

// ============================================================================ raiz

@Composable
fun AppRoot(
    vm: MainViewModel,
    overLock: Boolean = false,
    keyguardLocked: () -> Boolean = { false },
    onLeaveLock: () -> Unit = {},
    onDone: () -> Unit = {},
) {
    val ui by vm.ui.collectAsState()
    var showSettings by remember { mutableStateOf(false) }
    /** Ecras da 0.9.11: Definicoes (antes o menu de tres pontos) e Novo contacto (antes uma janela). */
    var showProfileSettings by rememberSaveable { mutableStateOf(false) }
    var showAddContact by rememberSaveable { mutableStateOf(false) }
    var showMyId by remember { mutableStateOf(false) }
    var showLockSettings by remember { mutableStateOf(false) }
    var showGroupInfo by remember { mutableStateOf(false) }
    LaunchedEffect(ui.openChatId) { showGroupInfo = false }
    val openGroup = ui.openChatId?.let { ui.groupOf(it) }
    // Notificacoes (Android 13+): pede a permissao uma vez, quando a app ja esta pronta a usar.
    val ctx = LocalContext.current
    val askNotifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (!granted && ui.pushOn) {
            android.widget.Toast.makeText(ctx, s(R.string.t_sem_permissao_a_app_nao_consegue), android.widget.Toast.LENGTH_LONG).show()
        }
    }
    // Chamadas: o microfone e pedido so ao ligar ou atender (uma vez).
    var afterMic by remember { mutableStateOf<(() -> Unit)?>(null) }
    val askMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val then = afterMic
        afterMic = null
        if (granted) {
            then?.invoke()
        } else {
            android.widget.Toast.makeText(ctx, s(R.string.t_sem_acesso_ao_microfone_nao_e_2), android.widget.Toast.LENGTH_LONG).show()
        }
    }
    val withMic: (() -> Unit) -> Unit = { action ->
        if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            action()
        } else {
            afterMic = action
            AutoLockPause.active = true   // o pedido de permissao nao conta como sair da app
            askMic.launch(Manifest.permission.RECORD_AUDIO)
        }
    }
    LaunchedEffect(ui.hasIdentity, ui.locked, ui.termsAccepted, ui.loading) {
        val ready = !ui.loading && ui.hasIdentity && !ui.locked && ui.termsAccepted
        if (ready && Push.available && ui.pushOn && !ui.pushAsked && Build.VERSION.SDK_INT >= 33 && !Push.permissionGranted(ctx)) {
            vm.markPushAsked()
            AutoLockPause.active = true   // o pedido de permissao nao conta como sair da app
            askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    // "Atender" na notificacao: atende, pedindo o microfone se ainda nao houver permissao.
    LaunchedEffect(ui.answerNow, ui.call?.callId, ui.call?.phase) {
        if (ui.answerNow != null && ui.answerNow == ui.call?.callId && ui.call?.phase == CallPhase.INCOMING) {
            vm.answerConsumed()
            withMic { vm.acceptCall() }
        }
    }

    val call = ui.call
    var showPhoto by remember { mutableStateOf(false) }
    var showLanguage by remember { mutableStateOf(false) }
    CompositionLocalProvider(LocalPhotos provides ui.photos) { when {
        overLock -> CallOverLock(ui, vm, withMic, keyguardLocked, onLeaveLock, onDone)
        ui.loading -> LoadingScreen()
        call != null && !ui.locked -> {
            BackHandler { }   // a chamada nao se fecha com o "voltar": desliga-se no botao
            CallScreen(
                call,
                onAccept = { withMic { vm.acceptCall() } },
                onHangup = vm::hangupCall,
                onMute = vm::toggleMute,
                onSpeaker = vm::toggleSpeaker,
            )
        }
        ui.locked -> LockScreen(ui, onCode = vm::unlockWithCode, onBio = vm::requestBioUnlock, onWipe = vm::wipeLockedData)
        !ui.termsAccepted -> TermsScreen(onAccept = vm::acceptTerms)
        !ui.hasIdentity -> SetupScreen(ui, onCreate = vm::createIdentity)
        openGroup != null && showGroupInfo -> {
            BackHandler { showGroupInfo = false }
            GroupInfoScreen(
                ui, openGroup, onBack = { showGroupInfo = false },
                onUpdate = { name, members -> vm.updateGroup(openGroup.groupId, name, members) },
                onLeave = { vm.leaveGroup(openGroup.groupId) },
                onDelete = { showGroupInfo = false; vm.deleteGroup(openGroup.groupId) },
                onAddContact = vm::addContact,
            )
        }
        openGroup != null -> {
            BackHandler { vm.closeChat() }
            GroupChatScreen(
                ui, openGroup,
                onSend = { text -> vm.sendGroup(openGroup.groupId, text) },
                onReact = { mid, emoji -> vm.reactGroup(openGroup.groupId, mid, emoji) },
                onBack = vm::closeChat, onLoadOlder = vm::loadOlder,
                onInfo = { showGroupInfo = true }, loadAttachment = vm::attachmentBytes,
                onSendImage = { uri, original, copy -> vm.sendImageTo(openGroup.conversationId, uri, original, copy) },
                onSendFile = { uri -> vm.sendFileTo(openGroup.conversationId, uri) },
                onSendVoice = { file, ms -> vm.sendVoiceTo(openGroup.conversationId, file, ms) },
            )
        }
        ui.openChatId?.startsWith("g:") == true -> {
            // O grupo foi apagado (ou ainda nao carregou): volta a lista.
            LaunchedEffect(ui.openChatId) { vm.closeChat() }
            LoadingScreen()
        }
        ui.openChatId != null -> {
            val chatId = ui.openChatId!!
            BackHandler { vm.closeChat() }
            ChatScreen(
                ui, chatId, ui.contacts.firstOrNull { it.contactId == chatId },
                onSend = vm::send, onBack = vm::closeChat, onAdd = vm::addContact,
                onSafety = vm::showSafetyNumber, onLoadOlder = vm::loadOlder,
                onDeleteConversation = vm::deleteConversation, onDeleteContact = vm::deleteContact,
                onSendImage = vm::sendImage, loadAttachment = vm::attachmentBytes,
                blocked = chatId in ui.blocked, onBlock = vm::blockContact, onUnblock = vm::unblockContact,
                invite = chatId in ui.invites, onAcceptInvite = vm::acceptInvite, onDeclineInvite = vm::declineInvite,
                onReact = vm::react, onSendVoice = vm::sendVoice, onSendFile = vm::sendFile,
                onCall = { c -> withMic { vm.startCall(c) } },
            )
        }
        showLockSettings -> {
            BackHandler { showLockSettings = false }
            LockSettingsScreen(
                ui, onBack = { showLockSettings = false },
                onEnable = vm::enableLock, onDisable = vm::disableLock, onChange = vm::changeCode,
                onBioOn = vm::startBioEnable, onBioOff = vm::disableBio, onTimeout = vm::setLockTimeout,
            )
        }
        showSettings && BuildConfig.RELAY_EDITABLE -> {
            BackHandler { showSettings = false }
            RelaySettingsScreen(ui, onSave = { u, k -> vm.saveRelay(u, k); showSettings = false }, onBack = { showSettings = false })
        }
        showAddContact -> {
            BackHandler { showAddContact = false }
            AddContactScreen(
                ui, onBack = { showAddContact = false },
                onAdd = { n, p -> showAddContact = false; vm.addContact(n, p) },
                onCreateGroup = { n, members -> showAddContact = false; vm.createGroup(n, members) },
            )
        }
        showProfileSettings -> {
            BackHandler { showProfileSettings = false }
            ProfileSettingsScreen(
                ui, onBack = { showProfileSettings = false },
                onProfilePhoto = { showPhoto = true },
                onMyId = { showMyId = true },
                onLockSettings = { showLockSettings = true }, onLockNow = vm::lockNow,
                onUnblock = vm::unblockContact,
                onPresence = vm::setPresence,
                onReadReceipts = vm::setReadReceipts,
                onPush = { on ->
                    vm.setPush(on)
                    if (on && !Push.permissionGranted(ctx) && Build.VERSION.SDK_INT >= 33) {
                        AutoLockPause.active = true
                        askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
                    }
                },
                onLanguage = { showLanguage = true },
                onRelay = { showSettings = true },
                onWipeAll = { showProfileSettings = false; vm.wipeAllData() },
            )
        }
        else -> HomeScreen(
            ui, onOpen = vm::openChat,
            onMyId = { showMyId = true },
            onSettings = { showProfileSettings = true },
            onAdd = { showAddContact = true },
            onDeleteConversation = vm::deleteConversation, onDeleteContact = vm::deleteContact,
            onDeleteGroup = vm::deleteGroup,
        )
    } }
    // Por cima do bloqueio do telemovel: nenhum dialogo (so a chamada).
    if (overLock) return
    if (showMyId && !ui.locked) MyIdDialog(ui.publicId, onDismiss = { showMyId = false })
    if (showLanguage && !ui.locked) {
        LanguageDialog(
            onPick = { tag ->
                showLanguage = false
                if (tag != Lang.chosen()) {
                    Lang.choose(tag)
                    // No Android 13+ o sistema recria a Activity sozinho; antes, recria-se aqui.
                    if (Build.VERSION.SDK_INT < 33) (ctx as? android.app.Activity)?.recreate()
                }
            },
            onDismiss = { showLanguage = false },
        )
    }
    if (showPhoto && !ui.locked) {
        ProfilePhotoDialog(
            ui,
            onPick = { uri -> vm.previewProfilePhoto(uri) },
            onApply = vm::applyProfilePhoto,
            onCancelPreview = vm::cancelProfilePhoto,
            onRemove = { vm.removeProfilePhoto() },
            onDismiss = { vm.cancelProfilePhoto(); showPhoto = false },
        )
    }
    // Avisos curtos ("Foto atualizada").
    LaunchedEffect(ui.notice) {
        ui.notice?.let {
            android.widget.Toast.makeText(ctx, it, android.widget.Toast.LENGTH_SHORT).show()
            vm.dismissNotice()
        }
    }
    ui.wipeServerFailed?.let { reason ->
        WipeServerFailedDialog(
            reason,
            onRetry = { vm.wipeAllData() },
            onLocalOnly = { vm.wipeAllData(localOnly = true) },
            onDismiss = vm::dismissWipeServerFailed,
        )
    }
    ui.safetyNumber?.let { text ->
        AlertDialog(
            onDismissRequest = vm::dismissSafetyNumber,
            icon = { Icon(Icons.Filled.Lock, contentDescription = null) },
            confirmButton = { TextButton(onClick = vm::dismissSafetyNumber) { Text(s(R.string.t_fechar)) } },
            title = { Text(s(R.string.t_verificar_seguranca)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    SelectionContainer {
                        Text(text, style = MaterialTheme.typography.bodyLarge, fontFamily = FontFamily.Monospace)
                    }
                    Text(
                        s(R.string.t_compara_estes_numeros_com_a_outra),
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            },
        )
    }
    ui.error?.let { msg ->
        AlertDialog(
            onDismissRequest = vm::dismissError,
            confirmButton = { TextButton(onClick = vm::dismissError) { Text("OK") } },
            text = { Text(msg) },
        )
    }
}

/**
 * Por cima do bloqueio do telemovel (a app abriu por uma chamada): so a chamada, ou o codigo
 * da app se o bloqueio estiver ativo. Nada das conversas. Sem chamada (acabou ou nunca
 * chegou), fecha; com o telemovel desbloqueado, volta a app normal.
 */
@Composable
private fun CallOverLock(
    ui: UiState,
    vm: MainViewModel,
    withMic: (() -> Unit) -> Unit,
    keyguardLocked: () -> Boolean,
    onLeaveLock: () -> Unit,
    onDone: () -> Unit,
) {
    val call = ui.call
    var hadCall by remember { mutableStateOf(false) }
    LaunchedEffect(call != null) { if (call != null) hadCall = true }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1_000L)
            if (!keyguardLocked()) {
                onLeaveLock()
                break
            }
        }
    }
    LaunchedEffect(call == null, ui.locked, ui.loading, hadCall) {
        if (call == null && !ui.locked && !ui.loading) {
            // Acabou: fecha logo. Nunca chegou (ex.: codigo da app a meio): da tempo a oferta.
            delay(if (hadCall) 300L else 15_000L)
            onDone()
        }
    }
    BackHandler { }
    when {
        call != null && !ui.locked -> CallScreen(
            call,
            onAccept = { withMic { vm.acceptCall() } },
            onHangup = vm::hangupCall,
            onMute = vm::toggleMute,
            onSpeaker = vm::toggleSpeaker,
        )
        // Sem "apagar os dados" aqui: o telemovel ainda esta bloqueado.
        ui.locked -> LockScreen(ui, onCode = vm::unlockWithCode, onBio = vm::requestBioUnlock, onWipe = {})
        else -> LoadingScreen()
    }
}

@Composable
fun LoadingScreen() {
    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(16.dp)) {
            BrandLogo(72)
            Text(s(R.string.t_a_abrir_o_armazenamento_seguro), style = MaterialTheme.typography.bodyMedium)
        }
    }
}

/**
 * Logotipo SEALER (bolha de vidro com o S e os tres pontos), sempre a rodar
 * devagar como um piao, a volta do eixo vertical: uma volta completa a cada 6
 * segundos. Desde a 0.9.11 so roda no cabecalho do ecra inicial (HomeHeader); nos
 * outros ecras o logo fica parado (BrandLogo).
 *
 * Para nao parecer uma folha de papel quando fica de lado, o logo tem espessura:
 * e desenhado em varias camadas, umas atras das outras; as da frente e de tras
 * sao o logo e as do meio sao a silhueta na cor do rebordo de vidro, mais
 * escura ao centro para a lateral parecer arredondada. De frente
 * so se ve a camada da frente; de lado, as camadas formam a parte lateral.
 * A rotacao e aplicada so na camada de desenho (graphicsLayer): o ecra so e
 * recomposto quando a face visivel muda (duas vezes por volta).
 */
@Composable
fun BrandMark(sizeDp: Int) {
    val spin = rememberInfiniteTransition(label = "logo")
    val angle by spin.animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(tween(durationMillis = 6_000, easing = LinearEasing)),
        label = "rotacao do logo",
    )
    // Que face esta virada para o ecra: decide a ordem de desenho das camadas.
    val frontVisible by remember { derivedStateOf { cos(Math.toRadians(angle.toDouble())) >= 0.0 } }
    val painter = painterResource(R.drawable.sealer_logo)
    val thicknessPx = with(LocalDensity.current) { (sizeDp * 0.10f).dp.toPx() }
    val layers = thicknessPx.toInt().coerceIn(6, 40)   // ~1 camada por pixel de espessura
    // Lateral: lilas claro junto as faces, mais escuro ao centro.
    val rim = remember(layers) {
        List(layers) { i ->
            val middle = 1f - abs(2f * (i / (layers - 1f) - 0.5f))
            ColorFilter.tint(lerp(Color(0xFFD6CEF0), Color(0xFF9C8FC9), middle))
        }
    }
    Box(Modifier.size(sizeDp.dp)) {
        for (k in 0 until layers) {
            // Desenha de tras para a frente, em relacao a quem esta a ver.
            val i = if (frontVisible) k else layers - 1 - k
            val depth = i / (layers - 1f) - 0.5f
            val face = i == 0 || i == layers - 1
            Image(
                painter = painter,
                contentDescription = null,
                colorFilter = if (face) null else rim[i],
                modifier = Modifier.matchParentSize().graphicsLayer {
                    rotationY = angle
                    cameraDistance = 12f * density   // perspetiva suave, sem deformar o logo
                    translationX = depth * thicknessPx * sin(Math.toRadians(angle.toDouble())).toFloat()
                },
            )
        }
    }
}

/** Fonte da marca: Manrope Bold (SIL Open Font License 1.1, ver assets/licencas/manrope-OFL.txt). */
val ManropeBold = FontFamily(Font(R.font.manrope_bold, FontWeight.Bold))

// ========================================================================= entrada

@Composable
fun SetupScreen(ui: UiState, onCreate: (String, String) -> Unit) {
    var url by remember { mutableStateOf(ui.relayUrl) }
    var key by remember { mutableStateOf(ui.relayKeyHex) }
    var advanced by remember { mutableStateOf(false) }
    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        BrandLogo(96)
        Spacer(Modifier.height(24.dp))
        Text(APP_NAME, style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold)
        Spacer(Modifier.height(8.dp))
        Text(
            s(R.string.t_mensagens_cifradas_ponta_a_ponta_com),
            style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(32.dp))
        Button(onClick = { onCreate(url, key) }, modifier = Modifier.fillMaxWidth().height(52.dp), shape = RoundedCornerShape(26.dp)) {
            Text(s(R.string.t_comecar), fontSize = 16.sp)
        }
        Spacer(Modifier.height(16.dp))
        Text(
            s(R.string.t_versao_alpha_ainda_nao_auditada_nao),
            style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.error,
        )
        if (BuildConfig.RELAY_EDITABLE) {   // so na build debug
            TextButton(onClick = { advanced = !advanced }) { Text(if (advanced) s(R.string.t_esconder_opcoes_de_teste) else s(R.string.t_opcoes_de_teste)) }
            if (advanced) {
                OutlinedTextField(url, { url = it }, label = { Text(s(R.string.t_url_do_relay)) }, modifier = Modifier.fillMaxWidth(), singleLine = true)
                OutlinedTextField(key, { key = it }, label = { Text(s(R.string.t_chave_publica_do_relay_pinning)) }, modifier = Modifier.fillMaxWidth())
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RelaySettingsScreen(ui: UiState, onSave: (String, String) -> Unit, onBack: () -> Unit) {
    var url by remember { mutableStateOf(ui.relayUrl) }
    var key by remember { mutableStateOf(ui.relayKeyHex) }
    Scaffold(topBar = { SealerTopBar(title = { Text(s(R.string.t_relay_teste)) }, onBack = onBack) }) { pad ->
        Column(Modifier.padding(pad).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(url, { url = it }, label = { Text(s(R.string.t_url_do_relay)) }, modifier = Modifier.fillMaxWidth(), singleLine = true)
            OutlinedTextField(key, { key = it }, label = { Text(s(R.string.t_chave_publica_do_relay_pinning)) }, modifier = Modifier.fillMaxWidth())
            Button(onClick = { onSave(url, key) }) { Text(s(R.string.t_guardar_e_voltar_a_ligar)) }
        }
    }
}

// ===================================================================== barra topo

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SealerTopBar(
    title: @Composable () -> Unit,
    onBack: (() -> Unit)? = null,
    actions: @Composable () -> Unit = {},
) {
    val c = LocalChatColors.current
    // Fundo "galaxia" com a curva em baixo a direita (0.9.11); os 14 dp de baixo sao para a curva.
    GalaxyHeader(curve = 30.dp) {
        TopAppBar(
            title = title,
            modifier = Modifier.padding(bottom = 14.dp),
            navigationIcon = {
                if (onBack != null) {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = s(R.string.t_voltar)) }
                }
            },
            actions = { actions() },
            colors = TopAppBarDefaults.topAppBarColors(
                containerColor = Color.Transparent,
                scrolledContainerColor = Color.Transparent,
                titleContentColor = c.onTopBar,
                navigationIconContentColor = c.onTopBar,
                actionIconContentColor = c.onTopBar,
            ),
        )
    }
}

// ===================================================================== conversas

@Composable
fun HomeScreen(
    ui: UiState,
    onOpen: (String) -> Unit,
    onMyId: () -> Unit,
    onSettings: () -> Unit,
    onAdd: () -> Unit,
    onDeleteConversation: (String) -> Unit,
    onDeleteContact: (String) -> Unit,
    onDeleteGroup: (String) -> Unit = {},
) {
    /** Grupo escolhido com toque longo (para apagar). */
    var groupActions by remember { mutableStateOf<uniffi.qt_ffi.GroupInfo?>(null) }
    /** Conversa escolhida com toque longo: (contactId, nome). */
    var actionsFor by remember { mutableStateOf<Pair<String, String>?>(null) }
    // Desde a 0.9.11: cabecalho com a minha foto (engrenagem = Definicoes, em vez do menu de
    // tres pontos) e o "+" abre o ecra Novo contacto (contacto ou grupo).
    Scaffold(
        topBar = { HomeHeader(ui, onSettings = onSettings, onMyId = onMyId) },
        floatingActionButton = {
            FloatingActionButton(
                onClick = onAdd,
                shape = RoundedCornerShape(20.dp),
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ) { Icon(Icons.Filled.Add, contentDescription = s(R.string.t_novo_contacto)) }
        },
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            val withHistory = ui.conversations.map { it.contactId }.toSet()
            val idle = ui.contacts.filter { it.contactId !in withHistory }
            val idleGroups = ui.groups.filter { it.conversationId !in withHistory }
            if (ui.conversations.isEmpty() && idle.isEmpty() && idleGroups.isEmpty()) {
                EmptyConversations(onMyId)
            } else {
                LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(top = 4.dp, bottom = 88.dp)) {
                    val onlineNow = ui.contacts.filter { it.contactId in ui.online && it.contactId !in ui.blocked }
                    if (onlineNow.isNotEmpty()) {
                        item(key = "online-now") { OnlineNowStrip(onlineNow, onOpen) }
                    }
                    items(ui.conversations, key = { it.contactId }) { conv ->
                        val group = ui.groupOf(conv.contactId)
                        if (group != null) {
                            ConversationRow(
                                group.name, group.groupId, conv,
                                previewPrefix = conv.lastMessage.takeIf { !it.outgoing }?.let { ui.memberName(it.sender) },
                                onClick = { onOpen(conv.contactId) },
                                onLongClick = { groupActions = group },
                            )
                        } else {
                            val name = ui.contacts.firstOrNull { it.contactId == conv.contactId }?.nickname
                            ConversationRow(
                                name ?: s(R.string.t_desconhecido), conv.contactId, conv,
                                blocked = conv.contactId in ui.blocked,
                                invite = name == null && conv.contactId in ui.invites,
                                online = name != null && conv.contactId in ui.online,
                                onClick = { onOpen(conv.contactId) },
                                onLongClick = { actionsFor = conv.contactId to (name ?: s(R.string.t_desconhecido)) },
                            )
                        }
                    }
                    items(idleGroups, key = { "idle-" + it.conversationId }) { g ->
                        ConversationRow(
                            g.name, g.groupId, null,
                            onClick = { onOpen(g.conversationId) },
                            onLongClick = { groupActions = g },
                        )
                    }
                    items(idle, key = { "idle-" + it.contactId }) { c ->
                        ConversationRow(
                            c.nickname, c.contactId, null,
                            blocked = c.contactId in ui.blocked,
                            online = c.contactId in ui.online,
                            onClick = { onOpen(c.contactId) },
                            onLongClick = { actionsFor = c.contactId to c.nickname },
                        )
                    }
                }
            }
        }
    }
    groupActions?.let { g ->
        GroupDeleteDialog(g.name, onDismiss = { groupActions = null }, onConfirm = { groupActions = null; onDeleteGroup(g.groupId) })
    }
    actionsFor?.let { (cid, name) ->
        DeleteActions(
            name = name,
            hasHistory = ui.conversations.any { it.contactId == cid },
            onDismiss = { actionsFor = null },
            onDeleteConversation = { onDeleteConversation(cid); actionsFor = null },
            onDeleteContact = { onDeleteContact(cid); actionsFor = null },
        )
    }
}

@Composable
fun EmptyConversations(onMyId: () -> Unit) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        BrandLogo(64)
        Spacer(Modifier.height(16.dp))
        Text(s(R.string.t_ainda_nao_tens_conversas), style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(8.dp))
        Text(
            s(R.string.t_partilha_o_teu_id_com_quem),
            style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(16.dp))
        OutlinedButton(onClick = onMyId) { Text(s(R.string.t_partilhar_o_meu_id)) }
    }
}

/** "Online agora": os contactos com a app aberta, numa fila no topo da lista (tocar abre a conversa). */
@Composable
fun OnlineNowStrip(contacts: List<Contact>, onOpen: (String) -> Unit) {
    Column(Modifier.fillMaxWidth().padding(top = 10.dp)) {
        Text(
            s(R.string.t_online_agora), Modifier.padding(horizontal = 16.dp),
            style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        LazyRow(
            contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 12.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            items(contacts, key = { it.contactId }) { c ->
                Column(
                    Modifier.width(64.dp).clip(RoundedCornerShape(12.dp)).clickable { onOpen(c.contactId) }.padding(4.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Avatar(c.nickname, c.contactId, sizeDp = 48, online = true)
                    Spacer(Modifier.height(4.dp))
                    Text(c.nickname, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
    }
}

/** Avatar: a foto de perfil do contacto (se houver) ou as iniciais, numa cor estavel por contacto. */
@Composable
fun Avatar(name: String, seed: String, sizeDp: Int = 48, online: Boolean = false) {
    val palette = LocalChatColors.current.avatarPalette
    val color = palette[(seed.hashCode() and 0x7fffffff) % palette.size]
    val initials = name.trim().split(" ").filter { it.isNotBlank() }.take(2).joinToString("") { it.take(1).uppercase() }.ifBlank { "?" }
    val photo = LocalPhotos.current[seed]
    Box(Modifier.size(sizeDp.dp)) {
        if (photo != null) {
            Image(photo, contentDescription = name, modifier = Modifier.matchParentSize().clip(CircleShape), contentScale = ContentScale.Crop)
        } else {
            Box(Modifier.matchParentSize().clip(CircleShape).background(color), contentAlignment = Alignment.Center) {
                Text(initials, color = Color.White, fontWeight = FontWeight.SemiBold, fontSize = (sizeDp / 2.6).sp)
            }
        }
        // Ponto verde: o contacto tem a app aberta agora (presenca).
        if (online) {
            Box(
                Modifier.align(Alignment.BottomEnd).size((sizeDp / 4).coerceAtLeast(10).dp)
                    .clip(CircleShape).background(MaterialTheme.colorScheme.surface).padding(2.dp)
                    .clip(CircleShape).background(OnlineGreen)
                    .semantics { contentDescription = s(R.string.t_online) },
            )
        }
    }
}

/** Cor do ponto "online". */
val OnlineGreen = Color(0xFF2EB872)

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ConversationRow(
    name: String, contactId: String, conv: ConversationSummary?,
    blocked: Boolean = false,
    invite: Boolean = false,
    /** Nos grupos: quem escreveu a ultima mensagem recebida ("Ana: ..."). */
    previewPrefix: String? = null,
    online: Boolean = false,
    onClick: () -> Unit, onLongClick: () -> Unit = {},
) {
    val last = conv?.lastMessage
    val unread = conv?.unread ?: 0u
    // 0.9.11: avatar maior, hora em cima a direita e, por baixo, as nao lidas ou uma seta.
    Row(
        Modifier.fillMaxWidth().combinedClickable(onClick = onClick, onLongClick = onLongClick).padding(start = 18.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Avatar(name, contactId, sizeDp = 52, online = online)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Row(Modifier.padding(end = 16.dp, top = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(
                        name, fontSize = 17.sp, fontWeight = FontWeight.Bold,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.height(3.dp))
                    val preview = when {
                        blocked -> s(R.string.t_bloqueado)
                        invite -> s(R.string.t_quer_falar_contigo_toca_para_aceitar)
                        last == null -> s(R.string.t_toca_para_comecar_a_conversar)
                        last.outgoing -> statusGlyph(last.status) + "  " + coreText(last.text)
                        previewPrefix != null -> "$previewPrefix: ${coreText(last.text)}"
                        else -> coreText(last.text)
                    }
                    Text(
                        preview, style = MaterialTheme.typography.bodyMedium,
                        fontWeight = if (unread > 0u) FontWeight.SemiBold else FontWeight.Normal,
                        color = if (unread > 0u) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                }
                Spacer(Modifier.width(10.dp))
                Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (last != null) {
                        Text(
                            shortTime(last.localAtMs), style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.SemiBold,
                            color = if (unread > 0u) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    if (unread > 0u) {
                        Box(
                            Modifier.clip(CircleShape).background(MaterialTheme.colorScheme.primary).padding(horizontal = 7.dp, vertical = 2.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(unread.toString(), color = MaterialTheme.colorScheme.onPrimary, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                        }
                    } else {
                        Icon(
                            Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp),
                        )
                    }
                }
            }
            HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
        }
    }
}

@Composable
fun MyIdDialog(publicId: String, onDismiss: () -> Unit) {
    val clipboard = LocalClipboardManager.current
    val context = LocalContext.current
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.t_o_teu_id)) },
        text = {
            Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (publicId.isNotBlank()) {
                    // Em maiusculas o QR usa o modo alfanumerico: menos quadrados, as camaras leem melhor.
                    val qr = remember(publicId) { qrCode(publicId.uppercase()) }
                    Image(
                        bitmap = qr,
                        contentDescription = s(R.string.t_codigo_qr_do_teu_id),
                        modifier = Modifier.size(250.dp).clip(RoundedCornerShape(12.dp)).background(Color.White).padding(6.dp),
                    )
                }
                // So o inicio e o fim (para reconhecer); Copiar e Partilhar dao o ID completo.
                Text(
                    shortId(publicId), fontFamily = FontFamily.Monospace, fontSize = 13.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = {
                val send = Intent(Intent.ACTION_SEND).apply {
                    type = "text/plain"
                    putExtra(Intent.EXTRA_TEXT, publicId)
                }
                context.startActivity(Intent.createChooser(send, s(R.string.t_partilhar_o_meu_id)))
            }) { Text(s(R.string.t_partilhar)) }
        },
        dismissButton = {
            TextButton(onClick = { clipboard.setText(AnnotatedString(publicId)); onDismiss() }) { Text(s(R.string.t_copiar)) }
        },
    )
}

@Composable
fun AddContactDialog(onDismiss: () -> Unit, onAdd: (String, String) -> Unit) {
    // rememberSaveable: o leitor de QR e outro ecra; se o telemovel rodar enquanto
    // esta aberto, a app e recriada e o que ja estava escrito nao se perde.
    var nick by rememberSaveable { mutableStateOf("") }
    var pid by rememberSaveable { mutableStateOf("") }
    var scanError by rememberSaveable { mutableStateOf<String?>(null) }
    // Leitor de QR (ZXing, no telemovel, sem servicos da Google nem rede).
    val scanner = rememberLauncherForActivityResult(ScanContract()) { result ->
        val id = idFromQr(result.contents)
        when {
            id != null -> { pid = id; scanError = null }
            result.contents != null -> scanError = s(R.string.t_este_codigo_qr_nao_e_um)
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = { onAdd(nick, pid) }, enabled = pid.isNotBlank()) { Text(s(R.string.t_adicionar)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
        title = { Text(s(R.string.t_novo_contacto)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(nick, { nick = it }, label = { Text(s(R.string.t_nome)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(pid, { pid = it }, label = { Text(s(R.string.t_id_qt1)) }, modifier = Modifier.fillMaxWidth())
                OutlinedButton(
                    onClick = {
                        AutoLockPause.active = true
                        scanner.launch(
                            ScanOptions()
                                .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                                .setPrompt(s(R.string.t_aponta_para_o_codigo_qr_da))
                                .setBeepEnabled(false)
                                .setOrientationLocked(false)
                                .addExtra(
                                    Intents.Scan.MISSING_CAMERA_PERMISSION_DIALOG_MESSAGE,
                                    s(R.string.t_sem_acesso_a_camara_para_ler),
                                ),
                        )
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(s(R.string.t_ler_codigo_qr)) }
                scanError?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
                Text(
                    s(R.string.t_recebe_o_id_diretamente_da_pessoa),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
    )
}

// =========================================================================== chat

// consumeWindowInsets era experimental em versoes antigas do Compose; o OptIn
// garante que compila com qualquer versao do BOM.
@OptIn(ExperimentalLayoutApi::class, ExperimentalFoundationApi::class)
@Composable
fun ChatScreen(
    ui: UiState, contactId: String, contact: Contact?,
    onSend: (Contact, String) -> Unit, onBack: () -> Unit, onAdd: (String, String) -> Unit,
    onSafety: (Contact) -> Unit, onLoadOlder: () -> Unit,
    onDeleteConversation: (String) -> Unit, onDeleteContact: (String) -> Unit,
    onSendImage: (Contact, Uri, Boolean, File?) -> Unit,
    loadAttachment: suspend (String, String) -> ByteArray?,
    blocked: Boolean = false,
    onBlock: (String) -> Unit = {},
    onUnblock: (String) -> Unit = {},
    invite: Boolean = false,
    onAcceptInvite: (String, String) -> Unit = { _, _ -> },
    onDeclineInvite: (String) -> Unit = {},
    onReact: (String, String, String) -> Unit = { _, _, _ -> },
    onSendVoice: (Contact, File, Long) -> Unit = { _, _, _ -> },
    onSendFile: (Contact, Uri) -> Unit = { _, _ -> },
    onCall: (Contact) -> Unit = {},
) {
    val colors = LocalChatColors.current
    var viewing by remember { mutableStateOf<ChatMessage?>(null) }
    var showAdd by rememberSaveable { mutableStateOf(false) }
    var chatMenu by remember { mutableStateOf(false) }
    var showDelete by remember { mutableStateOf(false) }
    var confirmBlock by remember { mutableStateOf(false) }
    var reactingTo by remember { mutableStateOf<ChatMessage?>(null) }
    val name = contact?.nickname ?: s(R.string.t_desconhecido)
    // Mensagens da MAIS RECENTE para a mais antiga; com reverseLayout a lista
    // fica colada ao fundo e "subir" percorre o passado.
    val msgs = ui.chatMessages
    val listState = rememberLazyListState()
    val nearOldest by remember {
        derivedStateOf {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull()?.index ?: 0
            info.totalItemsCount > 0 && last >= info.totalItemsCount - 10
        }
    }
    LaunchedEffect(nearOldest, msgs.size, ui.chatHasMore) {
        if (nearOldest && ui.chatHasMore) onLoadOlder()
    }
    FollowNewest(listState, msgs.firstOrNull()?.seq, msgs.firstOrNull()?.outgoing == true)

    Scaffold(
        containerColor = colors.chatBackground,
        topBar = {
            SealerTopBar(
                onBack = onBack,
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        val isOnline = contact != null && contactId in ui.online
                        Avatar(name, contactId, sizeDp = 38, online = isOnline)
                        Spacer(Modifier.width(10.dp))
                        Column {
                            Text(name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(
                                if (isOnline) s(R.string.t_online) else s(R.string.t_cifrada_ponta_a_ponta_2),
                                style = MaterialTheme.typography.labelSmall, color = colors.onTopBar.copy(alpha = 0.8f),
                            )
                        }
                    }
                },
                actions = {
                    if (contact != null && !blocked && !invite) {
                        IconButton(onClick = { onCall(contact) }) { Icon(Icons.Filled.Call, contentDescription = s(R.string.t_ligar)) }
                    }
                    if (contact != null) {
                        IconButton(onClick = { onSafety(contact) }) { Icon(Icons.Filled.Lock, contentDescription = s(R.string.t_verificar_seguranca)) }
                    }
                    Box {
                        IconButton(onClick = { chatMenu = true }) { Icon(Icons.Filled.MoreVert, contentDescription = s(R.string.t_mais_opcoes)) }
                        DropdownMenu(expanded = chatMenu, onDismissRequest = { chatMenu = false }) {
                            if (blocked) {
                                DropdownMenuItem(
                                    text = { Text(s(R.string.t_desbloquear_2)) },
                                    leadingIcon = { Icon(Icons.Filled.Check, contentDescription = null) },
                                    onClick = { chatMenu = false; onUnblock(contactId) },
                                )
                            } else {
                                DropdownMenuItem(
                                    text = { Text(s(R.string.t_bloquear)) },
                                    leadingIcon = { Icon(Icons.Filled.Close, contentDescription = null) },
                                    onClick = { chatMenu = false; confirmBlock = true },
                                )
                            }
                            DropdownMenuItem(
                                text = { Text(s(R.string.t_eliminar_conversa_ou_contacto)) },
                                leadingIcon = { Icon(Icons.Filled.Delete, contentDescription = null) },
                                onClick = { chatMenu = false; showDelete = true },
                            )
                        }
                    }
                },
            )
        },
    ) { pad ->
        // consumeWindowInsets: o Scaffold ja reservou a barra de navegacao; sem isto, com o
        // teclado aberto, o espaco dela era contado duas vezes (buraco por cima do teclado).
        Column(Modifier.padding(pad).consumeWindowInsets(pad).fillMaxSize().background(colors.chatBackground).imePadding()) {
            LazyColumn(
                Modifier.weight(1f).fillMaxWidth(),
                state = listState,
                reverseLayout = true,
                verticalArrangement = Arrangement.spacedBy(3.dp),
                contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 10.dp, vertical = 8.dp),
            ) {
                // Separadores de dia: com reverseLayout, um item emitido DEPOIS de uma
                // mensagem aparece ACIMA dela.
                for (i in msgs.indices) {
                    val m = msgs[i]
                    item(key = m.seq) {
                        MessageBubble(
                            m, ui.transfers[m.messageId], loadAttachment, onOpen = { viewing = m },
                            onLongPress = { if (!blocked) reactingTo = m },
                        )
                    }
                    val older = msgs.getOrNull(i + 1)
                    if (older == null || dayKey(older.localAtMs) != dayKey(m.localAtMs)) {
                        if (older != null || !ui.chatHasMore) {
                            item(key = "day-" + m.seq) { DaySeparator(m.localAtMs) }
                        }
                    }
                }
                if (ui.chatHasMore) {
                    item(key = "more") {
                        Text(
                            s(R.string.t_a_carregar_mensagens_anteriores), Modifier.fillMaxWidth().padding(8.dp),
                            style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Center,
                        )
                    }
                } else {
                    item(key = "e2e") { EncryptionNotice() }
                }
            }
            if (blocked) {
                Column(
                    Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).padding(16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(s(R.string.t_bloqueaste_esta_pessoa_as_mensagens_dela), style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center)
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(onClick = { onUnblock(contactId) }) { Text(s(R.string.t_desbloquear_2)) }
                }
            } else if (contact == null && invite) {
                InviteCard(
                    onAccept = { nick -> onAcceptInvite(contactId, nick) },
                    onDecline = { onDeclineInvite(contactId) },
                    onBlock = { confirmBlock = true },
                )
            } else if (contact == null) {
                Column(
                    Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).padding(16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(s(R.string.t_esta_pessoa_nao_esta_nos_teus), style = MaterialTheme.typography.bodyMedium)
                    Spacer(Modifier.height(8.dp))
                    Button(onClick = { showAdd = true }) { Text(s(R.string.t_adicionar_para_responder)) }
                }
            } else {
                ChatComposer(
                    onSendText = { text -> onSend(contact, text) },
                    onSendImage = { uri, original, copy -> onSendImage(contact, uri, original, copy) },
                    onSendFile = { uri -> onSendFile(contact, uri) },
                    onSendVoice = { file, ms -> onSendVoice(contact, file, ms) },
                )
            }
        }
    }
    if (showAdd) AddContactDialog(onDismiss = { showAdd = false }, onAdd = { n, p -> onAdd(n, p); showAdd = false })
    viewing?.let { m -> AttachmentOverlay(m, loadAttachment, onDismiss = { viewing = null }) }
    if (showDelete) {
        DeleteActions(
            name = name,
            hasHistory = ui.chatMessages.isNotEmpty(),
            onDismiss = { showDelete = false },
            onDeleteConversation = { onDeleteConversation(contactId); showDelete = false },
            onDeleteContact = { onDeleteContact(contactId); showDelete = false },
        )
    }
    reactingTo?.let { m ->
        ReactionPicker(
            current = m.reactions.firstOrNull { it.mine }?.emoji,
            onPick = { e -> onReact(contactId, m.messageId, e); reactingTo = null },
            onDismiss = { reactingTo = null },
        )
    }
    if (confirmBlock) {
        AlertDialog(
            onDismissRequest = { confirmBlock = false },
            icon = { Icon(Icons.Filled.Close, contentDescription = null) },
            title = { Text(s(R.string.t_bloquear_2, name)) },
            text = { ObscuredTouchGuard()
                Text(
                    s(R.string.t_as_mensagens_desta_pessoa_deixam_de),
                )
            },
            confirmButton = {
                TextButton(onClick = { confirmBlock = false; onBlock(contactId) }) {
                    Text(s(R.string.t_bloquear), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { confirmBlock = false }) { Text(s(R.string.t_cancelar)) } },
        )
    }
}

// ===================================================================== eliminar

/**
 * Escolha + confirmacao. Dois passos de proposito: eliminar e permanente
 * (as paginas apagadas sao sobrescritas na base de dados cifrada).
 */
@Composable
fun DeleteActions(
    name: String,
    hasHistory: Boolean,
    onDismiss: () -> Unit,
    onDeleteConversation: () -> Unit,
    onDeleteContact: () -> Unit,
) {
    var confirm by remember { mutableStateOf<String?>(null) }   // "conversa" | "contacto"
    when (confirm) {
        null -> AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    TextButton(onClick = { confirm = "conversa" }, enabled = hasHistory, modifier = Modifier.fillMaxWidth()) {
                        Text(s(R.string.t_eliminar_conversa), modifier = Modifier.fillMaxWidth())
                    }
                    TextButton(onClick = { confirm = "contacto" }, modifier = Modifier.fillMaxWidth()) {
                        Text(s(R.string.t_apagar_contacto), color = MaterialTheme.colorScheme.error, modifier = Modifier.fillMaxWidth())
                    }
                }
            },
            confirmButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
        )
        "conversa" -> AlertDialog(
            onDismissRequest = onDismiss,
            icon = { Icon(Icons.Filled.Delete, contentDescription = null) },
            title = { Text(s(R.string.t_eliminar_conversa_2)) },
            text = { ObscuredTouchGuard()
                Text(
                    s(R.string.t_todas_as_mensagens_com_sao_apagadas, name),
                )
            },
            confirmButton = {
                TextButton(onClick = onDeleteConversation) { Text(s(R.string.t_eliminar), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
        )
        else -> AlertDialog(
            onDismissRequest = onDismiss,
            icon = { Icon(Icons.Filled.Delete, contentDescription = null) },
            title = { Text(s(R.string.t_apagar_contacto_2)) },
            text = { ObscuredTouchGuard()
                Text(
                    s(R.string.t_e_removido_da_tua_lista_e, name),
                )
            },
            confirmButton = {
                TextButton(onClick = onDeleteContact) { Text(s(R.string.t_apagar_2), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
        )
    }
}

/** Apagar tudo: so com a palavra de confirmacao escrita (APAGAR em PT) (evita toques acidentais). */
@Composable
fun WipeAllDialog(onDismiss: () -> Unit, onConfirm: () -> Unit, serverToo: Boolean = true) {
    var typed by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
        title = { Text(s(R.string.t_destruir_perfil_e_dados_2)) },
        text = { ObscuredTouchGuard()
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    if (serverToo) {
                        s(R.string.t_sao_apagados_para_sempre_do_servidor)
                    } else {
                        // Com a app bloqueada nao ha acesso a identidade, por isso nao se consegue falar com o servidor.
                        s(R.string.t_sao_apagados_para_sempre_deste_telemovel)
                    },
                )
                Text(s(R.string.t_para_confirmar_escreve_apagar, s(R.string.confirm_word)), style = MaterialTheme.typography.bodySmall)
                OutlinedTextField(typed, { typed = it }, singleLine = true, modifier = Modifier.fillMaxWidth())
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm, enabled = typed.trim().equals(s(R.string.confirm_word), ignoreCase = true)) {
                Text(s(R.string.t_apagar_tudo), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
    )
}

// ===================================================================== convite

/**
 * Alguem que nao esta nos contactos escreveu-me e mandou o seu ID (convite): aceitar
 * (sem escrever o ID), recusar (apaga a conversa) ou bloquear.
 */
@Composable
fun InviteCard(onAccept: (String) -> Unit, onDecline: () -> Unit, onBlock: () -> Unit) {
    var nick by rememberSaveable { mutableStateOf("") }
    Column(
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(s(R.string.t_esta_pessoa_quer_falar_contigo), style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
        Text(
            s(R.string.t_ainda_nao_esta_nos_teus_contactos),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        OutlinedTextField(
            nick, { nick = it.take(40) }, singleLine = true, modifier = Modifier.fillMaxWidth(),
            label = { Text(s(R.string.t_nome_opcional)) },
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Button(onClick = { onAccept(nick) }) { Text(s(R.string.t_aceitar)) }
            OutlinedButton(onClick = onDecline) { Text(s(R.string.t_recusar)) }
            Spacer(Modifier.weight(1f))
            TextButton(onClick = onBlock) { Text(s(R.string.t_bloquear), color = MaterialTheme.colorScheme.error) }
        }
    }
}

// ===================================================================== termos

/** Endereco dos termos de utilizacao e da politica de privacidade (site). */
private const val TERMS_URL = "https://sealer.pt/termos"
private const val PRIVACY_URL = "https://sealer.pt/privacidade"

/**
 * Antes de usar a app: aceitar os termos de utilizacao e a politica de privacidade
 * (a Google Play exige-o em apps com conversas entre utilizadores). Volta a aparecer
 * se TERMS_VERSION subir, ou depois de apagar todos os dados.
 */
@Composable
fun TermsScreen(onAccept: () -> Unit) {
    val uri = LocalUriHandler.current
    var agreed by rememberSaveable { mutableStateOf(false) }
    Scaffold { pad ->
        Column(
            Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Spacer(Modifier.height(24.dp))
            Text(s(R.string.t_antes_de_comecar), style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.SemiBold)
            Text(s(R.string.t_a_sealer_esta_em_testes_ao), style = MaterialTheme.typography.bodyLarge)
            for (line in listOf(
                s(R.string.t_nao_uses_a_sealer_para_nada),
                s(R.string.t_se_alguem_te_incomodar_podes_bloquea),
                s(R.string.t_as_tuas_mensagens_so_existem_nos),
                s(R.string.t_por_estar_em_testes_a_app),
            )) {
                Row {
                    Text("•  ", style = MaterialTheme.typography.bodyMedium)
                    Text(line, style = MaterialTheme.typography.bodyMedium)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = { runCatching { uri.openUri(TERMS_URL) } }) { Text(s(R.string.t_termos_de_utilizacao)) }
                TextButton(onClick = { runCatching { uri.openUri(PRIVACY_URL) } }) { Text(s(R.string.t_politica_de_privacidade)) }
            }
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.clickable { agreed = !agreed }) {
                Checkbox(checked = agreed, onCheckedChange = { agreed = it })
                Text(s(R.string.t_li_e_aceito_os_termos_de), style = MaterialTheme.typography.bodyMedium)
            }
            Button(onClick = onAccept, enabled = agreed, modifier = Modifier.fillMaxWidth()) { Text(s(R.string.t_continuar)) }
        }
    }
}

/** Contactos bloqueados, cada um com "Desbloquear" (tambem para quem ja nao tem conversa). */
@Composable
fun BlockedContactsDialog(items: List<Pair<String, String>>, onUnblock: (String) -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.t_contactos_bloqueados)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (items.isEmpty()) Text(s(R.string.t_nao_tens_ninguem_bloqueado))
                for ((cid, name) in items) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text(name, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        TextButton(onClick = { onUnblock(cid) }) { Text(s(R.string.t_desbloquear_2)) }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_fechar)) } },
    )
}

/** Nao foi possivel apagar a conta no servidor: nada foi apagado ainda. */
@Composable
fun WipeServerFailedDialog(reason: String, onRetry: () -> Unit, onLocalOnly: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
        title = { Text(s(R.string.t_nao_foi_possivel_apagar_no_servidor)) },
        text = { ObscuredTouchGuard()
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(s(R.string.t_motivo_ainda_nao_foi_apagado_nada, reason))
                Text(
                    s(R.string.t_podes_tentar_de_novo_quando_tiveres),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        },
        confirmButton = { TextButton(onClick = onRetry) { Text(s(R.string.t_tentar_de_novo)) } },
        dismissButton = {
            Row {
                TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) }
                TextButton(onClick = onLocalOnly) { Text(s(R.string.t_apagar_so_aqui), color = MaterialTheme.colorScheme.error) }
            }
        },
    )
}

@Composable
fun EncryptionNotice() {
    Box(Modifier.fillMaxWidth().padding(vertical = 8.dp), contentAlignment = Alignment.Center) {
        Row(
            Modifier.clip(RoundedCornerShape(8.dp)).background(MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.7f))
                .padding(horizontal = 12.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Filled.Lock, contentDescription = null, modifier = Modifier.size(14.dp), tint = MaterialTheme.colorScheme.onSecondaryContainer)
            Spacer(Modifier.width(6.dp))
            Text(
                s(R.string.t_as_mensagens_sao_cifradas_ponta_a),
                style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSecondaryContainer,
                textAlign = TextAlign.Center,
            )
        }
    }
}

@Composable
fun DaySeparator(ms: Long) {
    Box(Modifier.fillMaxWidth().padding(vertical = 6.dp), contentAlignment = Alignment.Center) {
        Text(
            dayLabel(ms),
            Modifier.clip(RoundedCornerShape(8.dp)).background(MaterialTheme.colorScheme.surface).padding(horizontal = 10.dp, vertical = 3.dp),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
@OptIn(ExperimentalFoundationApi::class)
fun MessageBubble(
    m: ChatMessage,
    progress: Float? = null,
    loadAttachment: suspend (String, String) -> ByteArray? = { _, _ -> null },
    onOpen: () -> Unit = {},
    onLongPress: () -> Unit = {},
    /** Nos grupos: quem escreveu (so nas recebidas). */
    senderName: String? = null,
) {
    val c = LocalChatColors.current
    val shape = if (m.outgoing) {
        RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp, bottomStart = 16.dp, bottomEnd = 4.dp)
    } else {
        RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp, bottomStart = 4.dp, bottomEnd = 16.dp)
    }
    Column(Modifier.fillMaxWidth(), horizontalAlignment = if (m.outgoing) Alignment.End else Alignment.Start) {
        val att = m.attachment
        Column(
            Modifier.widthIn(max = 300.dp).clip(shape)
                .combinedClickable(onClick = {}, onLongClick = onLongPress)
                .background(if (m.outgoing) c.outgoingBubble else c.incomingBubble)
                .padding(
                    start = if (att != null) 4.dp else 10.dp, end = if (att != null) 4.dp else 8.dp,
                    top = if (att != null) 4.dp else 6.dp, bottom = 4.dp,
                ),
        ) {
            if (senderName != null && !m.outgoing) {
                Text(
                    senderName, fontSize = 12.sp, fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.primary, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(start = if (att != null) 6.dp else 0.dp, bottom = 2.dp),
                )
            }
            if (att != null) {
                AttachmentContent(m, att, progress, loadAttachment, onOpen, onLongPress)
            } else {
                Text(coreText(m.text), style = MaterialTheme.typography.bodyLarge, color = c.bubbleText)
            }
            Row(Modifier.align(Alignment.End), verticalAlignment = Alignment.CenterVertically) {
                Text(timeOf(m.localAtMs), fontSize = 11.sp, color = c.bubbleMeta)
                if (m.outgoing) {
                    Spacer(Modifier.width(4.dp))
                    Text(
                        statusGlyph(m.status), fontSize = 12.sp, fontWeight = FontWeight.Bold,
                        color = if (m.status == MessageStatus.READ) c.readTick else c.bubbleMeta,
                    )
                }
            }
        }
        // Reacoes por baixo da bolha (tocar abre o seletor, como o toque longo).
        if (m.reactions.isNotEmpty()) {
            Row(
                Modifier.padding(horizontal = 6.dp).offset(y = (-4).dp).clip(RoundedCornerShape(12.dp))
                    .background(MaterialTheme.colorScheme.surface).clickable(onClick = onLongPress)
                    .padding(horizontal = 6.dp, vertical = 2.dp),
                horizontalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                for (r in m.reactions) Text(r.emoji, fontSize = 15.sp)
            }
        }
    }
}

/** Emojis do seletor de reacoes. */
private val QUICK_REACTIONS = listOf("👍", "❤️", "😂", "😮", "😢", "🙏")

/** Escolher a reacao a uma mensagem (ou retirar a minha). */
@Composable
fun ReactionPicker(current: String?, onPick: (String) -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.t_reagir)) },
        text = {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                for (e in QUICK_REACTIONS) {
                    Box(
                        Modifier.size(44.dp).clip(CircleShape)
                            .background(if (e == current) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
                            .clickable { onPick(e) },
                        contentAlignment = Alignment.Center,
                    ) { Text(e, fontSize = 24.sp) }
                }
            }
        },
        confirmButton = {
            if (current != null) TextButton(onClick = { onPick("") }) { Text(s(R.string.t_retirar_a_minha)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
    )
}

// ===================================================================== utilitarios

/** 🕓 por enviar · ✓ enviado (relay) · ✓✓ entregue · ✓✓ azul visto. */
fun statusGlyph(s: MessageStatus): String = when (s) {
    MessageStatus.PENDING -> "🕓"
    MessageStatus.SENT -> "✓"
    MessageStatus.DELIVERED, MessageStatus.READ -> "✓✓"
    MessageStatus.RECEIVED, MessageStatus.READ_BY_ME -> ""
}

private val timeFmt = java.text.SimpleDateFormat("HH:mm", java.util.Locale.getDefault())
private val dateFmt = java.text.SimpleDateFormat("dd/MM/yyyy", java.util.Locale.getDefault())
private val dayFmt = java.text.SimpleDateFormat("yyyyMMdd", java.util.Locale.ROOT)

fun timeOf(ms: Long): String = timeFmt.format(java.util.Date(ms))

fun dayKey(ms: Long): String = dayFmt.format(java.util.Date(ms))

/** "Hoje", "Ontem" ou a data. */
fun dayLabel(ms: Long): String {
    val today = dayKey(System.currentTimeMillis())
    val yesterday = dayKey(System.currentTimeMillis() - 24L * 3600 * 1000)
    return when (dayKey(ms)) {
        today -> s(R.string.t_hoje)
        yesterday -> s(R.string.t_ontem)
        else -> dateFmt.format(java.util.Date(ms))
    }
}

/** Hora (hoje), "Ontem" ou a data, para a lista de conversas. */
fun shortTime(ms: Long): String = when (dayLabel(ms)) {
    s(R.string.t_hoje) -> timeOf(ms)
    s(R.string.t_ontem) -> s(R.string.t_ontem)
    else -> java.text.SimpleDateFormat("dd/MM", java.util.Locale.getDefault()).format(java.util.Date(ms))
}

// ======================================================================= bloqueio

/** Ecra de bloqueio: codigo de acesso ou impressao digital/rosto/PIN do telemovel. */
@Composable
fun LockScreen(ui: UiState, onCode: (String) -> Unit, onBio: () -> Unit, onWipe: () -> Unit) {
    val minLen = remember { MainViewModel.minCodeLen() }
    var code by remember { mutableStateOf("") }
    var forgot by remember { mutableStateOf(false) }
    var wipe by remember { mutableStateOf(false) }
    // Ao aparecer, pede logo a impressao digital (se estiver ativa).
    LaunchedEffect(Unit) { if (ui.bioEnabled) onBio() }
    // Depois de um codigo errado, o campo fica vazio para escrever de novo.
    LaunchedEffect(ui.lockMessage) { if (ui.lockMessage != null) code = "" }
    val canSubmit = !ui.unlocking && code.length >= minLen

    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        BrandLogo(80)
        Spacer(Modifier.height(20.dp))
        Text(
            s(R.string.t_esta_bloqueado, APP_NAME), style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        OutlinedTextField(
            value = code,
            onValueChange = { v -> code = v.filter { it.isDigit() }.take(32) },
            label = { Text(s(R.string.t_codigo_de_acesso)) },
            singleLine = true,
            enabled = !ui.unlocking,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { if (canSubmit) onCode(code) }),
            modifier = Modifier.fillMaxWidth(),
        )
        ui.lockMessage?.let {
            Spacer(Modifier.height(8.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
        }
        Spacer(Modifier.height(16.dp))
        Button(
            onClick = { onCode(code) }, enabled = canSubmit,
            modifier = Modifier.fillMaxWidth().height(52.dp), shape = RoundedCornerShape(26.dp),
        ) { Text(if (ui.unlocking) s(R.string.t_a_verificar) else s(R.string.t_desbloquear_2), fontSize = 16.sp) }
        if (ui.bioEnabled) {
            Spacer(Modifier.height(8.dp))
            OutlinedButton(onClick = onBio, enabled = !ui.unlocking, modifier = Modifier.fillMaxWidth()) {
                Text(s(R.string.t_usar_impressao_digital_ou_pin_do))
            }
        }
        Spacer(Modifier.height(8.dp))
        TextButton(onClick = { forgot = true }) { Text(s(R.string.t_esqueci_me_do_codigo)) }
    }
    if (forgot) {
        AlertDialog(
            onDismissRequest = { forgot = false },
            title = { Text(s(R.string.t_esqueceste_o_codigo)) },
            text = { ObscuredTouchGuard()
                Text(
                    s(R.string.t_sem_o_codigo_nao_e_possivel),
                )
            },
            confirmButton = {
                TextButton(onClick = { forgot = false; wipe = true }) {
                    Text(s(R.string.t_apagar_tudo), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { forgot = false }) { Text(s(R.string.t_voltar)) } },
        )
    }
    if (wipe) WipeAllDialog(onDismiss = { wipe = false }, onConfirm = { wipe = false; onWipe() }, serverToo = false)
}

private enum class LockDialog { ENABLE, DISABLE, CHANGE, BIO_ON }

/** Definicoes do bloqueio: ativar, biometria, tempo, mudar ou desativar o codigo. */
@Composable
fun LockSettingsScreen(
    ui: UiState,
    onBack: () -> Unit,
    onEnable: (String) -> Unit,
    onDisable: (String) -> Unit,
    onChange: (String, String) -> Unit,
    onBioOn: (String) -> Unit,
    onBioOff: () -> Unit,
    onTimeout: (Long) -> Unit,
) {
    var dialog by remember { mutableStateOf<LockDialog?>(null) }
    Scaffold(topBar = { SealerTopBar(title = { Text(s(R.string.t_bloqueio_da_app)) }, onBack = onBack) }) { pad ->
        Column(
            Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                s(R.string.t_com_o_bloqueio_ativo_as_conversas),
                style = MaterialTheme.typography.bodyMedium,
            )
            if (!ui.lockEnabled) {
                Button(onClick = { dialog = LockDialog.ENABLE }) { Text(s(R.string.t_ativar_bloqueio)) }
            } else {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(s(R.string.t_impressao_digital_rosto_ou_pin_do), style = MaterialTheme.typography.bodyLarge)
                        Text(
                            if (ui.bioAvailable || ui.bioEnabled) s(R.string.t_para_desbloquear_sem_escrever_o_codigo)
                            else s(R.string.t_indisponivel_precisa_de_android_11_ou),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Switch(
                        checked = ui.bioEnabled,
                        enabled = ui.bioAvailable || ui.bioEnabled,
                        onCheckedChange = { on -> if (on) dialog = LockDialog.BIO_ON else onBioOff() },
                    )
                }
                HorizontalDivider()
                Text(s(R.string.t_bloquear_automaticamente), style = MaterialTheme.typography.titleSmall)
                Column {
                    LOCK_TIMEOUTS.forEach { (label, ms) ->
                        Row(
                            Modifier.fillMaxWidth().clickable { onTimeout(ms) },
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = ui.lockTimeoutMs == ms, onClick = { onTimeout(ms) })
                            Text(label)
                        }
                    }
                }
                HorizontalDivider()
                OutlinedButton(onClick = { dialog = LockDialog.CHANGE }) { Text(s(R.string.t_mudar_codigo)) }
                TextButton(onClick = { dialog = LockDialog.DISABLE }) {
                    Text(s(R.string.t_desativar_bloqueio), color = MaterialTheme.colorScheme.error)
                }
            }
            Text(
                s(R.string.t_se_te_esqueceres_do_codigo_e),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
    when (dialog) {
        LockDialog.ENABLE -> CodeDialog(
            title = s(R.string.t_ativar_bloqueio),
            text = s(R.string.t_escolhe_um_codigo_so_com_numeros),
            askCurrent = false, askNew = true, confirm = s(R.string.t_ativar),
            onDismiss = { dialog = null },
            onConfirm = { _, new -> dialog = null; onEnable(new) },
        )
        LockDialog.DISABLE -> CodeDialog(
            title = s(R.string.t_desativar_bloqueio), text = null,
            askCurrent = true, askNew = false, confirm = s(R.string.t_desativar),
            onDismiss = { dialog = null },
            onConfirm = { cur, _ -> dialog = null; onDisable(cur) },
        )
        LockDialog.CHANGE -> CodeDialog(
            title = s(R.string.t_mudar_codigo), text = null,
            askCurrent = true, askNew = true, confirm = s(R.string.t_mudar),
            onDismiss = { dialog = null },
            onConfirm = { cur, new -> dialog = null; onChange(cur, new) },
        )
        LockDialog.BIO_ON -> CodeDialog(
            title = s(R.string.t_desbloqueio_rapido),
            text = s(R.string.t_confirma_com_o_teu_codigo_a),
            askCurrent = true, askNew = false, confirm = s(R.string.t_continuar),
            onDismiss = { dialog = null },
            onConfirm = { cur, _ -> dialog = null; onBioOn(cur) },
        )
        null -> {}
    }
}

/** Pede o codigo atual e/ou um codigo novo (duas vezes). */
@Composable
fun CodeDialog(
    title: String,
    text: String?,
    askCurrent: Boolean,
    askNew: Boolean,
    confirm: String,
    onDismiss: () -> Unit,
    onConfirm: (current: String, new: String) -> Unit,
) {
    val minLen = remember { MainViewModel.minCodeLen() }
    var cur by remember { mutableStateOf("") }
    var new1 by remember { mutableStateOf("") }
    var new2 by remember { mutableStateOf("") }
    val okCur = !askCurrent || cur.length >= minLen
    val okNew = !askNew || (new1.length >= minLen && new1 == new2)
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Filled.Lock, contentDescription = null) },
        title = { Text(title) },
        text = { ObscuredTouchGuard()
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                text?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                if (askCurrent) CodeField(cur, { cur = it }, s(R.string.t_codigo_atual))
                if (askNew) {
                    CodeField(new1, { new1 = it }, s(R.string.t_novo_codigo_minimo_digitos, minLen))
                    CodeField(new2, { new2 = it }, s(R.string.t_repetir_o_novo_codigo))
                    if (new2.isNotEmpty() && new1 != new2) {
                        Text(s(R.string.t_os_codigos_nao_sao_iguais), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onConfirm(cur, new1) }, enabled = okCur && okNew) { Text(confirm) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
    )
}

@Composable
fun CodeField(value: String, onChange: (String) -> Unit, label: String) {
    OutlinedTextField(
        value = value,
        onValueChange = { v -> onChange(v.filter { it.isDigit() }.take(32)) },
        label = { Text(label) },
        singleLine = true,
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
        modifier = Modifier.fillMaxWidth(),
    )
}
