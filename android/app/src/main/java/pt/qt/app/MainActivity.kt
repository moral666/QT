package pt.qt.app

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
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
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import uniffi.qt_ffi.ChatMessage
import uniffi.qt_ffi.ConversationSummary
import uniffi.qt_ffi.MessageStatus

class MainActivity : ComponentActivity() {
    private val vm: MainViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Bloqueia capturas de ecra e miniaturas na lista de apps recentes.
        window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
        setContent { SealerTheme { Surface(Modifier.fillMaxSize()) { AppRoot(vm) } } }
    }
}

// ============================================================================ raiz

@Composable
fun AppRoot(vm: MainViewModel) {
    val ui by vm.ui.collectAsState()
    var showSettings by remember { mutableStateOf(false) }
    var showMyId by remember { mutableStateOf(false) }

    when {
        ui.loading -> LoadingScreen()
        !ui.hasIdentity -> SetupScreen(ui, onCreate = vm::createIdentity)
        ui.openChatId != null -> {
            val chatId = ui.openChatId!!
            BackHandler { vm.closeChat() }
            ChatScreen(
                ui, chatId, ui.contacts.firstOrNull { it.contactId == chatId },
                onSend = vm::send, onBack = vm::closeChat, onAdd = vm::addContact,
                onSafety = vm::showSafetyNumber, onLoadOlder = vm::loadOlder,
                onDeleteConversation = vm::deleteConversation, onDeleteContact = vm::deleteContact,
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
            onWipeAll = vm::wipeAllData,
        )
    }
    if (showMyId) MyIdDialog(ui.publicId, onDismiss = { showMyId = false })
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
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(16.dp)) {
            BrandMark(72)
            Text("A abrir o armazenamento seguro…", style = MaterialTheme.typography.bodyMedium)
        }
    }
}

/** Logotipo simples: cadeado branco num circulo roxo. */
@Composable
fun BrandMark(sizeDp: Int) {
    Box(
        Modifier.size(sizeDp.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary),
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Filled.Lock, contentDescription = null, tint = MaterialTheme.colorScheme.onPrimary, modifier = Modifier.size((sizeDp / 2).dp))
    }
}

// ========================================================================= entrada

@Composable
fun SetupScreen(ui: UiState, onCreate: (String, String) -> Unit) {
    var url by remember { mutableStateOf(ui.relayUrl) }
    var key by remember { mutableStateOf(ui.relayKeyHex) }
    var advanced by remember { mutableStateOf(false) }
    Column(
        Modifier.fillMaxSize().padding(32.dp),
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
    TopAppBar(
        title = title,
        navigationIcon = {
            if (onBack != null) {
                IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Voltar") }
            }
        },
        actions = { actions() },
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = c.topBar,
            titleContentColor = c.onTopBar,
            navigationIconContentColor = c.onTopBar,
            actionIconContentColor = c.onTopBar,
        ),
    )
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
) {
    var showAdd by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    /** Conversa escolhida com toque longo: (contactId, nome). */
    var actionsFor by remember { mutableStateOf<Pair<String, String>?>(null) }
    var showWipe by remember { mutableStateOf(false) }
    Scaffold(
        topBar = {
            SealerTopBar(
                title = { Text(APP_NAME, fontWeight = FontWeight.SemiBold) },
                actions = {
                    IconButton(onClick = onMyId) { Icon(Icons.Filled.Share, contentDescription = "O meu ID") }
                    Box {
                        IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, contentDescription = "Mais opções") }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(text = { Text("O meu ID") }, onClick = { menu = false; onMyId() })
                            DropdownMenuItem(
                                text = { Text("Recibos de leitura") },
                                onClick = { onReadReceipts(!ui.readReceipts) },
                                trailingIcon = { Switch(checked = ui.readReceipts, onCheckedChange = { onReadReceipts(it) }) },
                            )
                            if (BuildConfig.RELAY_EDITABLE) {
                                DropdownMenuItem(text = { Text("Relay (teste)") }, onClick = { menu = false; onSettings() })
                            }
                            HorizontalDivider()
                            DropdownMenuItem(
                                text = { Text("Apagar todos os dados", color = MaterialTheme.colorScheme.error) },
                                onClick = { menu = false; showWipe = true },
                            )
                        }
                    }
                },
            )
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = { showAdd = true },
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ) { Icon(Icons.Filled.Add, contentDescription = "Novo contacto") }
        },
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            if (ui.status.isNotBlank() && ui.status != "Ligado ao relay") ConnectionBanner(ui.status)
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
                            onClick = { onOpen(conv.contactId) },
                            onLongClick = { actionsFor = conv.contactId to (name ?: "Desconhecido") },
                        )
                    }
                    items(idle, key = { "idle-" + it.contactId }) { c ->
                        ConversationRow(
                            c.nickname, c.contactId, null,
                            onClick = { onOpen(c.contactId) },
                            onLongClick = { actionsFor = c.contactId to c.nickname },
                        )
                    }
                }
            }
        }
    }
    if (showAdd) AddContactDialog(onDismiss = { showAdd = false }, onAdd = { n, p -> onAddContact(n, p); showAdd = false })
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
                    "Quem tiver este ID pode escrever-te. Partilha-o com as pessoas com quem queres falar.",
                    style = MaterialTheme.typography.bodySmall,
                )
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
    var nick by remember { mutableStateOf("") }
    var pid by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = { onAdd(nick, pid) }, enabled = pid.isNotBlank()) { Text("Adicionar") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
        title = { Text("Novo contacto") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(nick, { nick = it }, label = { Text("Nome") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(pid, { pid = it }, label = { Text("ID (qt1:…)") }, modifier = Modifier.fillMaxWidth())
                Text(
                    "Recebe o ID diretamente da pessoa (pessoalmente ou por um canal em que confies).",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
    )
}

// =========================================================================== chat

@Composable
fun ChatScreen(
    ui: UiState, contactId: String, contact: Contact?,
    onSend: (Contact, String) -> Unit, onBack: () -> Unit, onAdd: (String, String) -> Unit,
    onSafety: (Contact) -> Unit, onLoadOlder: () -> Unit,
    onDeleteConversation: (String) -> Unit, onDeleteContact: (String) -> Unit,
) {
    val colors = LocalChatColors.current
    var text by remember { mutableStateOf("") }
    var showAdd by remember { mutableStateOf(false) }
    var chatMenu by remember { mutableStateOf(false) }
    var showDelete by remember { mutableStateOf(false) }
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
        Column(Modifier.padding(pad).fillMaxSize().background(colors.chatBackground).imePadding()) {
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
                    item(key = m.seq) { MessageBubble(m) }
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
            if (contact == null) {
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
                    TextField(
                        value = text,
                        onValueChange = { text = it },
                        modifier = Modifier.weight(1f),
                        placeholder = { Text("Mensagem") },
                        shape = RoundedCornerShape(24.dp),
                        maxLines = 5,
                        colors = TextFieldDefaults.colors(
                            focusedContainerColor = MaterialTheme.colorScheme.surface,
                            unfocusedContainerColor = MaterialTheme.colorScheme.surface,
                            focusedIndicatorColor = Color.Transparent,
                            unfocusedIndicatorColor = Color.Transparent,
                        ),
                    )
                    Spacer(Modifier.width(6.dp))
                    FloatingActionButton(
                        onClick = { if (text.isNotBlank()) { onSend(contact, text); text = "" } },
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
    if (showDelete) {
        DeleteActions(
            name = name,
            hasHistory = ui.chatMessages.isNotEmpty(),
            onDismiss = { showDelete = false },
            onDeleteConversation = { onDeleteConversation(contactId); showDelete = false },
            onDeleteContact = { onDeleteContact(contactId); showDelete = false },
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
fun WipeAllDialog(onDismiss: () -> Unit, onConfirm: () -> Unit) {
    var typed by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
        title = { Text("Apagar todos os dados?") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    "São apagados para sempre deste telemóvel: o teu ID e as tuas chaves, todos os contactos, " +
                        "todas as conversas e as definições. O teu ID atual deixa de funcionar e terás de criar um novo.",
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
fun MessageBubble(m: ChatMessage) {
    val c = LocalChatColors.current
    val shape = if (m.outgoing) {
        RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp, bottomStart = 16.dp, bottomEnd = 4.dp)
    } else {
        RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp, bottomStart = 4.dp, bottomEnd = 16.dp)
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = if (m.outgoing) Arrangement.End else Arrangement.Start) {
        Column(
            Modifier.widthIn(max = 300.dp).clip(shape)
                .background(if (m.outgoing) c.outgoingBubble else c.incomingBubble)
                .padding(start = 10.dp, end = 8.dp, top = 6.dp, bottom = 4.dp),
        ) {
            Text(m.text, style = MaterialTheme.typography.bodyLarge, color = c.bubbleText)
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
