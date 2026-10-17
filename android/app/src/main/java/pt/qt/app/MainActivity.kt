package pt.qt.app

import android.content.Intent
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
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
        setContent { SealerTheme { Surface(Modifier.fillMaxSize()) { AppRoot(vm) } } }
        // Pedidos de autenticacao do ViewModel (desbloquear / ativar a biometria).
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                vm.bioRequests.collect { showBioPrompt(it) }
            }
        }
    }

    override fun onStart() {
        super.onStart()
        vm.onAppVisible()
    }

    override fun onStop() {
        super.onStop()
        if (!isChangingConfigurations) vm.onAppHidden()
    }

    private fun showBioPrompt(req: BioRequest) {
        if (vm.bioPurpose != null) {   // ja ha um aberto: ignora o pedido repetido
            if (req.purpose == BioPurpose.ENABLE) vm.cancelBioEnable()
            return
        }
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(if (req.purpose == BioPurpose.UNLOCK) "Desbloquear $APP_NAME" else "Ativar desbloqueio rápido")
            .setSubtitle("Impressão digital, rosto ou PIN do telemóvel")
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
            vm.bioMessage("Não foi possível pedir a autenticação do telemóvel.")
        }
    }
}

// ============================================================================ raiz

@Composable
fun AppRoot(vm: MainViewModel) {
    val ui by vm.ui.collectAsState()
    var showSettings by remember { mutableStateOf(false) }
    var showMyId by remember { mutableStateOf(false) }
    var showLockSettings by remember { mutableStateOf(false) }

    when {
        ui.loading -> LoadingScreen()
        ui.locked -> LockScreen(ui, onCode = vm::unlockWithCode, onBio = vm::requestBioUnlock, onWipe = vm::wipeLockedData)
        !ui.termsAccepted -> TermsScreen(onAccept = vm::acceptTerms)
        !ui.hasIdentity -> SetupScreen(ui, onCreate = vm::createIdentity)
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
            SettingsScreen(ui, onSave = { u, k -> vm.saveRelay(u, k); showSettings = false }, onBack = { showSettings = false })
        }
        else -> HomeScreen(
            ui, onOpen = vm::openChat, onAddContact = vm::addContact,
            onMyId = { showMyId = true }, onSettings = { showSettings = true },
            onReadReceipts = vm::setReadReceipts,
            onDeleteConversation = vm::deleteConversation, onDeleteContact = vm::deleteContact,
            onWipeAll = { vm.wipeAllData() },
            onUnblock = vm::unblockContact,
            onLockSettings = { showLockSettings = true }, onLockNow = vm::lockNow,
        )
    }
    if (showMyId && !ui.locked) MyIdDialog(ui.publicId, onDismiss = { showMyId = false })
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
            confirmButton = { TextButton(onClick = vm::dismissSafetyNumber) { Text("Fechar") } },
            title = { Text("Verificar segurança") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    SelectionContainer {
                        Text(text, style = MaterialTheme.typography.bodyLarge, fontFamily = FontFamily.Monospace)
                    }
                    Text(
                        "Compara estes números com a outra pessoa, pessoalmente ou numa chamada. " +
                            "Se forem iguais, ninguém (nem o servidor) está a intercetar a conversa.",
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

@Composable
fun LoadingScreen() {
    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(16.dp)) {
            BrandMark(72)
            Text("A abrir o armazenamento seguro…", style = MaterialTheme.typography.bodyMedium)
        }
    }
}

/**
 * Logotipo SEALER (bolha de vidro com o S e os tres pontos), sempre a rodar
 * devagar como um piao, a volta do eixo vertical: uma volta completa a cada 6
 * segundos.
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

/** Marca na barra de topo: logo pequeno (parado) e "Sealer" em Manrope Bold. */
@Composable
fun SealerWordmark() {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Image(
            painter = painterResource(R.drawable.sealer_logo),
            contentDescription = null,
            modifier = Modifier.size(30.dp),
        )
        Spacer(Modifier.width(10.dp))
        Text("Sealer", fontFamily = ManropeBold, fontWeight = FontWeight.Bold, fontSize = 22.sp)
    }
}

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
        BrandMark(96)
        Spacer(Modifier.height(24.dp))
        Text(APP_NAME, style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold)
        Spacer(Modifier.height(8.dp))
        Text(
            "Mensagens cifradas ponta a ponta, com criptografia pós-quântica. Sem número de telefone, sem email.",
            style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(32.dp))
        Button(onClick = { onCreate(url, key) }, modifier = Modifier.fillMaxWidth().height(52.dp), shape = RoundedCornerShape(26.dp)) {
            Text("Começar", fontSize = 16.sp)
        }
        Spacer(Modifier.height(16.dp))
        Text(
            "Versão alpha, ainda não auditada. Não uses para informação sensível.",
            style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.error,
        )
        if (BuildConfig.RELAY_EDITABLE) {   // so na build debug
            TextButton(onClick = { advanced = !advanced }) { Text(if (advanced) "Esconder opções de teste" else "Opções de teste") }
            if (advanced) {
                OutlinedTextField(url, { url = it }, label = { Text("URL do relay") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
                OutlinedTextField(key, { key = it }, label = { Text("Chave pública do relay (pinning)") }, modifier = Modifier.fillMaxWidth())
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(ui: UiState, onSave: (String, String) -> Unit, onBack: () -> Unit) {
    var url by remember { mutableStateOf(ui.relayUrl) }
    var key by remember { mutableStateOf(ui.relayKeyHex) }
    Scaffold(topBar = { SealerTopBar(title = { Text("Relay (teste)") }, onBack = onBack) }) { pad ->
        Column(Modifier.padding(pad).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(url, { url = it }, label = { Text("URL do relay") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
            OutlinedTextField(key, { key = it }, label = { Text("Chave pública do relay (pinning)") }, modifier = Modifier.fillMaxWidth())
            Button(onClick = { onSave(url, key) }) { Text("Guardar e voltar a ligar") }
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
    // Estrelas: posicoes fixas (sempre as mesmas), pequenas e meio transparentes.
    val stars = remember {
        val r = java.util.Random(7)
        List(28) { floatArrayOf(r.nextFloat(), r.nextFloat(), 0.6f + r.nextFloat() * 0.8f, 0.25f + r.nextFloat() * 0.45f) }
    }
    Box(
        Modifier
            .background(Brush.linearGradient(c.topBarGradient))
            .drawBehind {
                for (s in stars) {
                    drawCircle(
                        color = Color.White.copy(alpha = s[3]),
                        radius = s[2].dp.toPx() / 2f,
                        center = Offset(s[0] * size.width, s[1] * size.height),
                    )
                }
            },
    ) {
        TopAppBar(
            title = title,
            navigationIcon = {
                if (onBack != null) {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Voltar") }
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
    onAddContact: (String, String) -> Unit,
    onMyId: () -> Unit,
    onSettings: () -> Unit,
    onReadReceipts: (Boolean) -> Unit,
    onDeleteConversation: (String) -> Unit,
    onDeleteContact: (String) -> Unit,
    onWipeAll: () -> Unit,
    onLockSettings: () -> Unit,
    onLockNow: () -> Unit,
    onUnblock: (String) -> Unit,
) {
    var showAdd by rememberSaveable { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var showBlocked by remember { mutableStateOf(false) }
    /** Conversa escolhida com toque longo: (contactId, nome). */
    var actionsFor by remember { mutableStateOf<Pair<String, String>?>(null) }
    var showWipe by remember { mutableStateOf(false) }
    Scaffold(
        topBar = {
            SealerTopBar(
                title = { SealerWordmark() },
                actions = {
                    Box {
                        IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, contentDescription = "Mais opções") }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(text = { Text("O meu ID") }, onClick = { menu = false; onMyId() })
                            DropdownMenuItem(
                                text = { Text("Notificações") },
                                onClick = { onReadReceipts(!ui.readReceipts) },
                                trailingIcon = { Switch(checked = ui.readReceipts, onCheckedChange = { onReadReceipts(it) }) },
                            )
                            DropdownMenuItem(text = { Text("Bloqueio da app") }, onClick = { menu = false; onLockSettings() })
                            if (ui.lockEnabled) {
                                DropdownMenuItem(text = { Text("Bloquear agora") }, onClick = { menu = false; onLockNow() })
                            }
                            if (ui.blocked.isNotEmpty()) {
                                DropdownMenuItem(text = { Text("Contactos bloqueados") }, onClick = { menu = false; showBlocked = true })
                            }
                            if (BuildConfig.RELAY_EDITABLE) {
                                DropdownMenuItem(text = { Text("Relay (teste)") }, onClick = { menu = false; onSettings() })
                            }
                            HorizontalDivider()
                            DropdownMenuItem(
                                text = { Text("Destruir perfil e dados", color = MaterialTheme.colorScheme.error) },
                                onClick = { menu = false; showWipe = true },
                            )
                            HorizontalDivider()
                            // Versao instalada (build.gradle.kts, versionName), so informativa.
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        "Versão ${BuildConfig.VERSION_NAME}",
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                },
                                onClick = {},
                                enabled = false,
                            )
                        }
                    }
                },
            )
        },
        floatingActionButton = {
            // O logotipo a rodar como um piao, em vez do "+". O leitor de ecra continua
            // a anunciar "Novo contacto".
            FloatingActionButton(
                onClick = { showAdd = true },
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
                modifier = Modifier.semantics { contentDescription = "Novo contacto" },
            ) { BrandMark(34) }
        },
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            if (ui.status.isNotBlank() && ui.status != "Ligado ao relay") ConnectionBanner(ui.status)
            // Diagnostico da versao alpha: so aparece quando a entrega imediata nao esta ativa.
            if (ui.delivery.isNotBlank()) ConnectionBanner(ui.delivery)
            val withHistory = ui.conversations.map { it.contactId }.toSet()
            val idle = ui.contacts.filter { it.contactId !in withHistory }
            if (ui.conversations.isEmpty() && idle.isEmpty()) {
                EmptyConversations(onMyId)
            } else {
                LazyColumn(Modifier.fillMaxSize()) {
                    items(ui.conversations, key = { it.contactId }) { conv ->
                        val name = ui.contacts.firstOrNull { it.contactId == conv.contactId }?.nickname
                        ConversationRow(
                            name ?: "Desconhecido", conv.contactId, conv,
                            blocked = conv.contactId in ui.blocked,
                            onClick = { onOpen(conv.contactId) },
                            onLongClick = { actionsFor = conv.contactId to (name ?: "Desconhecido") },
                        )
                    }
                    items(idle, key = { "idle-" + it.contactId }) { c ->
                        ConversationRow(
                            c.nickname, c.contactId, null,
                            blocked = c.contactId in ui.blocked,
                            onClick = { onOpen(c.contactId) },
                            onLongClick = { actionsFor = c.contactId to c.nickname },
                        )
                    }
                }
            }
        }
    }
    if (showAdd) AddContactDialog(onDismiss = { showAdd = false }, onAdd = { n, p -> onAddContact(n, p); showAdd = false })
    if (showBlocked) {
        BlockedContactsDialog(
            ui.blocked.map { cid -> cid to (ui.contacts.firstOrNull { it.contactId == cid }?.nickname ?: "Desconhecido (${cid.take(8)}…)") },
            onUnblock = onUnblock,
            onDismiss = { showBlocked = false },
        )
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
    if (showWipe) WipeAllDialog(onDismiss = { showWipe = false }, onConfirm = { showWipe = false; onWipeAll() })
}

@Composable
fun ConnectionBanner(status: String) {
    Text(
        status,
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.secondaryContainer).padding(horizontal = 16.dp, vertical = 6.dp),
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSecondaryContainer,
        maxLines = 1, overflow = TextOverflow.Ellipsis,
    )
}

@Composable
fun EmptyConversations(onMyId: () -> Unit) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        BrandMark(64)
        Spacer(Modifier.height(16.dp))
        Text("Ainda não tens conversas", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(8.dp))
        Text(
            "Partilha o teu ID com quem quiseres falar, ou toca em + para adicionar o ID de outra pessoa.",
            style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(16.dp))
        OutlinedButton(onClick = onMyId) { Text("Partilhar o meu ID") }
    }
}

/** Avatar com as iniciais, numa cor estavel por contacto. */
@Composable
fun Avatar(name: String, seed: String, sizeDp: Int = 48) {
    val palette = LocalChatColors.current.avatarPalette
    val color = palette[(seed.hashCode() and 0x7fffffff) % palette.size]
    val initials = name.trim().split(" ").filter { it.isNotBlank() }.take(2).joinToString("") { it.take(1).uppercase() }.ifBlank { "?" }
    Box(Modifier.size(sizeDp.dp).clip(CircleShape).background(color), contentAlignment = Alignment.Center) {
        Text(initials, color = Color.White, fontWeight = FontWeight.SemiBold, fontSize = (sizeDp / 2.6).sp)
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ConversationRow(
    name: String, contactId: String, conv: ConversationSummary?,
    blocked: Boolean = false,
    onClick: () -> Unit, onLongClick: () -> Unit = {},
) {
    val last = conv?.lastMessage
    val unread = conv?.unread ?: 0u
    Column(Modifier.fillMaxWidth().combinedClickable(onClick = onClick, onLongClick = onLongClick)) {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(name, contactId)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        name, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                    if (last != null) {
                        Text(
                            shortTime(last.localAtMs), style = MaterialTheme.typography.labelSmall,
                            color = if (unread > 0u) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Spacer(Modifier.height(2.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    val preview = when {
                        blocked -> "Bloqueado"
                        last == null -> "Toca para começar a conversar"
                        last.outgoing -> statusGlyph(last.status) + "  " + last.text
                        else -> last.text
                    }
                    Text(
                        preview, style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                    if (unread > 0u) {
                        Spacer(Modifier.width(8.dp))
                        Box(
                            Modifier.clip(CircleShape).background(MaterialTheme.colorScheme.primary).padding(horizontal = 7.dp, vertical = 2.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(unread.toString(), color = MaterialTheme.colorScheme.onPrimary, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                        }
                    }
                }
            }
        }
        HorizontalDivider(Modifier.padding(start = 78.dp), thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
    }
}

@Composable
fun MyIdDialog(publicId: String, onDismiss: () -> Unit) {
    val clipboard = LocalClipboardManager.current
    val context = LocalContext.current
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("O teu ID") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    "Quem tiver este ID pode escrever-te. Se estiverem juntos, a outra pessoa lê este código " +
                        "com a SEALER (Novo contacto, Ler código QR). À distância, partilha o texto.",
                    style = MaterialTheme.typography.bodySmall,
                )
                if (publicId.isNotBlank()) {
                    val qr = remember(publicId) { qrCode(publicId) }
                    Image(
                        bitmap = qr,
                        contentDescription = "Código QR do teu ID",
                        modifier = Modifier.align(Alignment.CenterHorizontally).size(220.dp)
                            .clip(RoundedCornerShape(12.dp)).background(Color.White).padding(6.dp),
                    )
                }
                SelectionContainer {
                    Text(
                        publicId, fontFamily = FontFamily.Monospace, fontSize = 12.sp,
                        modifier = Modifier.clip(RoundedCornerShape(8.dp)).background(MaterialTheme.colorScheme.surfaceVariant).padding(10.dp),
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                val send = Intent(Intent.ACTION_SEND).apply {
                    type = "text/plain"
                    putExtra(Intent.EXTRA_TEXT, publicId)
                }
                context.startActivity(Intent.createChooser(send, "Partilhar o meu ID"))
            }) { Text("Partilhar") }
        },
        dismissButton = {
            TextButton(onClick = { clipboard.setText(AnnotatedString(publicId)); onDismiss() }) { Text("Copiar") }
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
            result.contents != null -> scanError = "Este código QR não é um ID SEALER."
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = { onAdd(nick, pid) }, enabled = pid.isNotBlank()) { Text("Adicionar") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
        title = { Text("Novo contacto") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(nick, { nick = it }, label = { Text("Nome") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(pid, { pid = it }, label = { Text("ID (qt1:…)") }, modifier = Modifier.fillMaxWidth())
                OutlinedButton(
                    onClick = {
                        AutoLockPause.active = true
                        scanner.launch(
                            ScanOptions()
                                .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                                .setPrompt("Aponta para o código QR da outra pessoa")
                                .setBeepEnabled(false)
                                .setOrientationLocked(false)
                                .addExtra(
                                    Intents.Scan.MISSING_CAMERA_PERMISSION_DIALOG_MESSAGE,
                                    "Sem acesso à câmara. Para ler o código QR, permite-o nas definições da app, " +
                                        "ou escreve o ID à mão.",
                                ),
                        )
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Ler código QR") }
                scanError?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
                Text(
                    "Recebe o ID diretamente da pessoa: lê o código QR dela quando estiverem juntos, " +
                        "ou usa um canal em que confies.",
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
) {
    val colors = LocalChatColors.current
    val ctx = LocalContext.current
    // TextFieldState (e nao String): so este campo recebe GIFs e autocolantes do
    // teclado (Gboard, SwiftKey...). Tambem sobrevive a rodar o ecra.
    val input = rememberTextFieldState()
    // Imagem escolhida na galeria, a espera de confirmacao (pre-visualizacao).
    var picked by rememberSaveable { mutableStateOf<Uri?>(null) }
    var viewing by remember { mutableStateOf<ChatMessage?>(null) }
    val pickImage = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri != null) picked = uri
    }
    val keyboardImages = remember(contact) {
        ReceiveContentListener { content ->
            if (contact == null || !content.hasMediaType(MediaType.Image)) return@ReceiveContentListener content
            var taken = false
            content.consume { item ->
                val uri = item.uri ?: return@consume false
                if (!taken) {
                    taken = true
                    // Copia ja, enquanto o teclado da permissao de leitura.
                    ImagePrep.copyToCache(ctx, uri)?.let { f -> onSendImage(contact, Uri.fromFile(f), false, f) }
                }
                true
            }
        }
    }
    var showAdd by rememberSaveable { mutableStateOf(false) }
    var chatMenu by remember { mutableStateOf(false) }
    var showDelete by remember { mutableStateOf(false) }
    var confirmBlock by remember { mutableStateOf(false) }
    val name = contact?.nickname ?: "Desconhecido"
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
    val newest = msgs.firstOrNull()?.seq
    LaunchedEffect(newest) {
        if (listState.firstVisibleItemIndex <= 1) listState.animateScrollToItem(0)
    }

    Scaffold(
        topBar = {
            SealerTopBar(
                onBack = onBack,
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Avatar(name, contactId, sizeDp = 38)
                        Spacer(Modifier.width(10.dp))
                        Column {
                            Text(name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text("cifrada ponta a ponta", style = MaterialTheme.typography.labelSmall, color = colors.onTopBar.copy(alpha = 0.8f))
                        }
                    }
                },
                actions = {
                    if (contact != null) {
                        IconButton(onClick = { onSafety(contact) }) { Icon(Icons.Filled.Lock, contentDescription = "Verificar segurança") }
                    }
                    Box {
                        IconButton(onClick = { chatMenu = true }) { Icon(Icons.Filled.MoreVert, contentDescription = "Mais opções") }
                        DropdownMenu(expanded = chatMenu, onDismissRequest = { chatMenu = false }) {
                            if (blocked) {
                                DropdownMenuItem(
                                    text = { Text("Desbloquear") },
                                    leadingIcon = { Icon(Icons.Filled.Check, contentDescription = null) },
                                    onClick = { chatMenu = false; onUnblock(contactId) },
                                )
                            } else {
                                DropdownMenuItem(
                                    text = { Text("Bloquear") },
                                    leadingIcon = { Icon(Icons.Filled.Close, contentDescription = null) },
                                    onClick = { chatMenu = false; confirmBlock = true },
                                )
                            }
                            DropdownMenuItem(
                                text = { Text("Eliminar conversa ou contacto") },
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
                        MessageBubble(m, ui.transfers[m.messageId], loadAttachment, onOpen = { viewing = m })
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
                            "A carregar mensagens anteriores…", Modifier.fillMaxWidth().padding(8.dp),
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
                    Text("Bloqueaste esta pessoa. As mensagens dela não te chegam.", style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center)
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(onClick = { onUnblock(contactId) }) { Text("Desbloquear") }
                }
            } else if (contact == null) {
                Column(
                    Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).padding(16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text("Esta pessoa não está nos teus contactos.", style = MaterialTheme.typography.bodyMedium)
                    Spacer(Modifier.height(8.dp))
                    Button(onClick = { showAdd = true }) { Text("Adicionar para responder") }
                }
            } else {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.Bottom,
                ) {
                    IconButton(
                        onClick = {
                            // O seletor de fotos e outro ecra: nao conta como sair da app (bloqueio).
                            AutoLockPause.active = true
                            pickImage.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                        },
                        modifier = Modifier.padding(bottom = 4.dp),
                    ) { Icon(Icons.Filled.Add, contentDescription = "Enviar imagem ou GIF") }
                    BasicTextField(
                        state = input,
                        modifier = Modifier.weight(1f).contentReceiver(keyboardImages),
                        textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                        lineLimits = TextFieldLineLimits.MultiLine(minHeightInLines = 1, maxHeightInLines = 5),
                        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                        decorator = { inner ->
                            Box(
                                Modifier.clip(RoundedCornerShape(24.dp)).background(MaterialTheme.colorScheme.surface)
                                    .padding(horizontal = 16.dp, vertical = 14.dp),
                            ) {
                                if (input.text.isEmpty()) {
                                    Text("Mensagem", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                                inner()
                            }
                        },
                    )
                    Spacer(Modifier.width(6.dp))
                    FloatingActionButton(
                        onClick = {
                            val text = input.text.toString()
                            if (text.isNotBlank()) { onSend(contact, text); input.clearText() }
                        },
                        shape = CircleShape,
                        containerColor = MaterialTheme.colorScheme.primary,
                        contentColor = MaterialTheme.colorScheme.onPrimary,
                        modifier = Modifier.size(52.dp),
                    ) { Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Enviar") }
                }
            }
        }
    }
    if (showAdd) AddContactDialog(onDismiss = { showAdd = false }, onAdd = { n, p -> onAdd(n, p); showAdd = false })
    picked?.let { uri ->
        if (contact != null) {
            SendImageDialog(
                uri,
                onSend = { original -> onSendImage(contact, uri, original, null); picked = null },
                onDismiss = { picked = null },
            )
        }
    }
    viewing?.let { m -> AttachmentViewer(m, loadAttachment, onDismiss = { viewing = null }) }
    if (showDelete) {
        DeleteActions(
            name = name,
            hasHistory = ui.chatMessages.isNotEmpty(),
            onDismiss = { showDelete = false },
            onDeleteConversation = { onDeleteConversation(contactId); showDelete = false },
            onDeleteContact = { onDeleteContact(contactId); showDelete = false },
        )
    }
    if (confirmBlock) {
        AlertDialog(
            onDismissRequest = { confirmBlock = false },
            icon = { Icon(Icons.Filled.Close, contentDescription = null) },
            title = { Text("Bloquear $name?") },
            text = {
                Text(
                    "As mensagens desta pessoa deixam de te chegar e não lhe consegues escrever. " +
                        "Ela não é avisada. A conversa fica, e podes desbloquear quando quiseres.",
                )
            },
            confirmButton = {
                TextButton(onClick = { confirmBlock = false; onBlock(contactId) }) {
                    Text("Bloquear", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { confirmBlock = false }) { Text("Cancelar") } },
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
                        Text("Eliminar conversa", modifier = Modifier.fillMaxWidth())
                    }
                    TextButton(onClick = { confirm = "contacto" }, modifier = Modifier.fillMaxWidth()) {
                        Text("Apagar contacto", color = MaterialTheme.colorScheme.error, modifier = Modifier.fillMaxWidth())
                    }
                }
            },
            confirmButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
        )
        "conversa" -> AlertDialog(
            onDismissRequest = onDismiss,
            icon = { Icon(Icons.Filled.Delete, contentDescription = null) },
            title = { Text("Eliminar conversa?") },
            text = {
                Text(
                    "Todas as mensagens com $name são apagadas para sempre deste telemóvel, incluindo as que ainda " +
                        "não foram enviadas. Não é possível recuperá-las.\n\nO contacto mantém-se e podem continuar a falar. " +
                        "As mensagens no telemóvel da outra pessoa não são apagadas.",
                )
            },
            confirmButton = {
                TextButton(onClick = onDeleteConversation) { Text("Eliminar", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
        )
        else -> AlertDialog(
            onDismissRequest = onDismiss,
            icon = { Icon(Icons.Filled.Delete, contentDescription = null) },
            title = { Text("Apagar contacto?") },
            text = {
                Text(
                    "$name é removido da tua lista e são apagadas para sempre deste telemóvel: as mensagens, " +
                        "as que estavam por enviar e as chaves da conversa cifrada. Não é possível recuperar.\n\n" +
                        "Se voltarem a falar, começa uma conversa cifrada nova. As mensagens no telemóvel da outra " +
                        "pessoa não são apagadas.",
                )
            },
            confirmButton = {
                TextButton(onClick = onDeleteContact) { Text("Apagar", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
        )
    }
}

/** Apagar tudo: so com a palavra APAGAR escrita (evita toques acidentais). */
@Composable
fun WipeAllDialog(onDismiss: () -> Unit, onConfirm: () -> Unit, serverToo: Boolean = true) {
    var typed by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
        title = { Text("Destruir perfil e dados?") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    if (serverToo) {
                        "São apagados para sempre: do servidor, as tuas chaves públicas e as mensagens à tua espera; " +
                            "deste telemóvel, o teu ID e as tuas chaves, todos os contactos, todas as conversas e as definições. " +
                            "O teu ID atual deixa de funcionar e terás de criar um novo."
                    } else {
                        // Com a app bloqueada nao ha acesso a identidade, por isso nao se consegue falar com o servidor.
                        "São apagados para sempre deste telemóvel: o teu ID e as tuas chaves, todos os contactos, " +
                            "todas as conversas e as definições. O teu ID atual deixa de funcionar e terás de criar um novo. " +
                            "O que está no servidor apaga-se sozinho: as mensagens em espera ao fim de 30 dias e as tuas " +
                            "chaves públicas ao fim de 90 dias."
                    },
                )
                Text("Para confirmar, escreve APAGAR:", style = MaterialTheme.typography.bodySmall)
                OutlinedTextField(typed, { typed = it }, singleLine = true, modifier = Modifier.fillMaxWidth())
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm, enabled = typed.trim() == "APAGAR") {
                Text("Apagar tudo", color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
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
            Text("Antes de começar", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.SemiBold)
            Text("A SEALER está em testes. Ao usá-la, aceitas o seguinte:", style = MaterialTheme.typography.bodyLarge)
            for (line in listOf(
                "Não uses a SEALER para nada ilegal, para ameaçar, assediar ou enganar outras pessoas, nem para enviar spam.",
                "Se alguém te incomodar, podes bloqueá-lo na conversa (menu ⋮ → Bloquear).",
                "As tuas mensagens só existem nos telemóveis. Se perderes o telemóvel ou apagares a app, as conversas perdem-se.",
                "Por estar em testes, a app pode ter falhas. Não a uses para informação crítica.",
            )) {
                Row {
                    Text("•  ", style = MaterialTheme.typography.bodyMedium)
                    Text(line, style = MaterialTheme.typography.bodyMedium)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = { runCatching { uri.openUri(TERMS_URL) } }) { Text("Termos de utilização") }
                TextButton(onClick = { runCatching { uri.openUri(PRIVACY_URL) } }) { Text("Política de privacidade") }
            }
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.clickable { agreed = !agreed }) {
                Checkbox(checked = agreed, onCheckedChange = { agreed = it })
                Text("Li e aceito os Termos de utilização e a Política de privacidade.", style = MaterialTheme.typography.bodyMedium)
            }
            Button(onClick = onAccept, enabled = agreed, modifier = Modifier.fillMaxWidth()) { Text("Continuar") }
        }
    }
}

/** Contactos bloqueados, cada um com "Desbloquear" (tambem para quem ja nao tem conversa). */
@Composable
fun BlockedContactsDialog(items: List<Pair<String, String>>, onUnblock: (String) -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Contactos bloqueados") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (items.isEmpty()) Text("Não tens ninguém bloqueado.")
                for ((cid, name) in items) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text(name, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        TextButton(onClick = { onUnblock(cid) }) { Text("Desbloquear") }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Fechar") } },
    )
}

/** Nao foi possivel apagar a conta no servidor: nada foi apagado ainda. */
@Composable
fun WipeServerFailedDialog(reason: String, onRetry: () -> Unit, onLocalOnly: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
        title = { Text("Não foi possível apagar no servidor") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("Motivo: $reason. Ainda não foi apagado nada.")
                Text(
                    "Podes tentar de novo quando tiveres rede, ou apagar já só deste telemóvel. Nesse caso, o que está " +
                        "no servidor apaga-se sozinho: as mensagens em espera ao fim de 30 dias e as tuas chaves públicas " +
                        "ao fim de 90 dias.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        },
        confirmButton = { TextButton(onClick = onRetry) { Text("Tentar de novo") } },
        dismissButton = {
            Row {
                TextButton(onClick = onDismiss) { Text("Cancelar") }
                TextButton(onClick = onLocalOnly) { Text("Apagar só aqui", color = MaterialTheme.colorScheme.error) }
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
                "As mensagens são cifradas ponta a ponta. Nem o servidor as consegue ler.",
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
fun MessageBubble(
    m: ChatMessage,
    progress: Float? = null,
    loadAttachment: suspend (String, String) -> ByteArray? = { _, _ -> null },
    onOpen: () -> Unit = {},
) {
    val c = LocalChatColors.current
    val shape = if (m.outgoing) {
        RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp, bottomStart = 16.dp, bottomEnd = 4.dp)
    } else {
        RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp, bottomStart = 4.dp, bottomEnd = 16.dp)
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = if (m.outgoing) Arrangement.End else Arrangement.Start) {
        val att = m.attachment
        Column(
            Modifier.widthIn(max = 300.dp).clip(shape)
                .background(if (m.outgoing) c.outgoingBubble else c.incomingBubble)
                .padding(
                    start = if (att != null) 4.dp else 10.dp, end = if (att != null) 4.dp else 8.dp,
                    top = if (att != null) 4.dp else 6.dp, bottom = 4.dp,
                ),
        ) {
            if (att != null) {
                AttachmentContent(m, att, progress, loadAttachment, onOpen)
            } else {
                Text(m.text, style = MaterialTheme.typography.bodyLarge, color = c.bubbleText)
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
    }
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
        today -> "Hoje"
        yesterday -> "Ontem"
        else -> dateFmt.format(java.util.Date(ms))
    }
}

/** Hora (hoje), "Ontem" ou a data, para a lista de conversas. */
fun shortTime(ms: Long): String = when (dayLabel(ms)) {
    "Hoje" -> timeOf(ms)
    "Ontem" -> "Ontem"
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
        BrandMark(80)
        Spacer(Modifier.height(20.dp))
        Text(
            "$APP_NAME está bloqueado", style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        OutlinedTextField(
            value = code,
            onValueChange = { v -> code = v.filter { it.isDigit() }.take(32) },
            label = { Text("Código de acesso") },
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
        ) { Text(if (ui.unlocking) "A verificar…" else "Desbloquear", fontSize = 16.sp) }
        if (ui.bioEnabled) {
            Spacer(Modifier.height(8.dp))
            OutlinedButton(onClick = onBio, enabled = !ui.unlocking, modifier = Modifier.fillMaxWidth()) {
                Text("Usar impressão digital ou PIN do telemóvel")
            }
        }
        Spacer(Modifier.height(8.dp))
        TextButton(onClick = { forgot = true }) { Text("Esqueci-me do código") }
    }
    if (forgot) {
        AlertDialog(
            onDismissRequest = { forgot = false },
            title = { Text("Esqueceste o código?") },
            text = {
                Text(
                    "Sem o código não é possível abrir as conversas: estão cifradas com ele e ninguém o pode " +
                        "recuperar. A única saída é apagar todos os dados deste telemóvel e começar de novo, com um ID novo.",
                )
            },
            confirmButton = {
                TextButton(onClick = { forgot = false; wipe = true }) {
                    Text("Apagar tudo", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { forgot = false }) { Text("Voltar") } },
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
    Scaffold(topBar = { SealerTopBar(title = { Text("Bloqueio da app") }, onBack = onBack) }) { pad ->
        Column(
            Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                "Com o bloqueio ativo, as conversas e os contactos só abrem com o teu código de acesso. " +
                    "O código protege a chave que cifra os dados neste telemóvel: não é só um ecrã por cima.",
                style = MaterialTheme.typography.bodyMedium,
            )
            if (!ui.lockEnabled) {
                Button(onClick = { dialog = LockDialog.ENABLE }) { Text("Ativar bloqueio") }
            } else {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text("Impressão digital, rosto ou PIN do telemóvel", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            if (ui.bioAvailable || ui.bioEnabled) "Para desbloquear sem escrever o código."
                            else "Indisponível: precisa de Android 11 ou mais recente e de bloqueio de ecrã.",
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
                Text("Bloquear automaticamente", style = MaterialTheme.typography.titleSmall)
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
                OutlinedButton(onClick = { dialog = LockDialog.CHANGE }) { Text("Mudar código") }
                TextButton(onClick = { dialog = LockDialog.DISABLE }) {
                    Text("Desativar bloqueio", color = MaterialTheme.colorScheme.error)
                }
            }
            Text(
                "Se te esqueceres do código e não puderes usar a impressão digital, não há forma de recuperar " +
                    "as conversas: a única saída é apagar todos os dados.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
    when (dialog) {
        LockDialog.ENABLE -> CodeDialog(
            title = "Ativar bloqueio",
            text = "Escolhe um código só com números e guarda-o bem: sem ele não há forma de abrir as conversas.",
            askCurrent = false, askNew = true, confirm = "Ativar",
            onDismiss = { dialog = null },
            onConfirm = { _, new -> dialog = null; onEnable(new) },
        )
        LockDialog.DISABLE -> CodeDialog(
            title = "Desativar bloqueio", text = null,
            askCurrent = true, askNew = false, confirm = "Desativar",
            onDismiss = { dialog = null },
            onConfirm = { cur, _ -> dialog = null; onDisable(cur) },
        )
        LockDialog.CHANGE -> CodeDialog(
            title = "Mudar código", text = null,
            askCurrent = true, askNew = true, confirm = "Mudar",
            onDismiss = { dialog = null },
            onConfirm = { cur, new -> dialog = null; onChange(cur, new) },
        )
        LockDialog.BIO_ON -> CodeDialog(
            title = "Desbloqueio rápido",
            text = "Confirma com o teu código. A seguir, o telemóvel pede a impressão digital, o rosto ou o PIN.",
            askCurrent = true, askNew = false, confirm = "Continuar",
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
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                text?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                if (askCurrent) CodeField(cur, { cur = it }, "Código atual")
                if (askNew) {
                    CodeField(new1, { new1 = it }, "Novo código (mínimo $minLen dígitos)")
                    CodeField(new2, { new2 = it }, "Repetir o novo código")
                    if (new2.isNotEmpty() && new1 != new2) {
                        Text("Os códigos não são iguais.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onConfirm(cur, new1) }, enabled = okCur && okNew) { Text(confirm) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
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
