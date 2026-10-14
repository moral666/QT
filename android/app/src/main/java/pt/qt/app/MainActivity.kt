package pt.qt.app

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Switch
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import uniffi.qt_ffi.ChatMessage
import uniffi.qt_ffi.ConversationSummary
import uniffi.qt_ffi.MessageStatus
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp

class MainActivity : ComponentActivity() {
    private val vm: MainViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Bloqueia capturas de ecra e miniaturas na lista de apps recentes.
        window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
        setContent { MaterialTheme { Surface(Modifier.fillMaxSize()) { AppRoot(vm) } } }
    }
}

@Composable
fun AppRoot(vm: MainViewModel) {
    val ui by vm.ui.collectAsState()
    var showSettings by remember { mutableStateOf(false) }

    when {
        ui.loading -> Column(Modifier.fillMaxSize().padding(24.dp)) { Text("A abrir o armazenamento seguro…") }
        !ui.hasIdentity -> SetupScreen(ui, onCreate = vm::createIdentity)
        ui.openChatId != null -> {
            val chatId = ui.openChatId!!
            BackHandler { vm.closeChat() }
            val contact = ui.contacts.firstOrNull { it.contactId == chatId }
            ChatScreen(
                ui, chatId, contact,
                onSend = vm::send, onBack = vm::closeChat, onAdd = vm::addContact,
                onSafety = vm::showSafetyNumber, onLoadOlder = vm::loadOlder,
            )
        }
        showSettings && BuildConfig.RELAY_EDITABLE -> {
            BackHandler { showSettings = false }
            SettingsScreen(ui, onSave = { u, k -> vm.saveRelay(u, k); showSettings = false })
        }
        else -> HomeScreen(ui, vm, onOpen = vm::openChat, onSettings = { showSettings = true })
    }
    ui.safetyNumber?.let { text ->
        AlertDialog(
            onDismissRequest = vm::dismissSafetyNumber,
            confirmButton = { TextButton(onClick = vm::dismissSafetyNumber) { Text("Fechar") } },
            title = { Text("Safety number") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    SelectionContainer { Text(text, style = MaterialTheme.typography.bodyLarge) }
                    Text(
                        "Compara estes 60 digitos com a outra pessoa por outro canal (voz, pessoalmente). " +
                            "Se forem diferentes, alguem (ou o relay) esta no meio: nao confies nesta conversa.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            },
        )
    }
    ui.error?.let { msg ->
        AlertDialog(onDismissRequest = vm::dismissError, confirmButton = { TextButton(onClick = vm::dismissError) { Text("OK") } }, text = { Text(msg) })
    }
}

@Composable
fun SetupScreen(ui: UiState, onCreate: (String, String) -> Unit) {
    var url by remember { mutableStateOf(ui.relayUrl) }
    var key by remember { mutableStateOf(ui.relayKeyHex) }
    Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("QT — alpha", style = MaterialTheme.typography.headlineSmall)
        Text("Versão alpha, ainda não auditada. Não uses para informação sensível.", color = MaterialTheme.colorScheme.error)
        if (BuildConfig.RELAY_EDITABLE) {   // so na build debug
            OutlinedTextField(url, { url = it }, label = { Text("URL do relay (wss://; ws:// só em debug)") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
            OutlinedTextField(key, { key = it }, label = { Text("Chave pública do relay (hex, opcional — pinning)") }, modifier = Modifier.fillMaxWidth())
        }
        Button(onClick = { onCreate(url, key) }) { Text("Criar identidade") }
    }
}

@Composable
fun SettingsScreen(ui: UiState, onSave: (String, String) -> Unit) {
    var url by remember { mutableStateOf(ui.relayUrl) }
    var key by remember { mutableStateOf(ui.relayKeyHex) }
    Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("Relay", style = MaterialTheme.typography.headlineSmall)
        OutlinedTextField(url, { url = it }, label = { Text("URL do relay") }, modifier = Modifier.fillMaxWidth(), singleLine = true)
        OutlinedTextField(key, { key = it }, label = { Text("Chave pública do relay (hex, opcional)") }, modifier = Modifier.fillMaxWidth())
        Button(onClick = { onSave(url, key) }) { Text("Guardar e voltar a ligar") }
    }
}

@Composable
fun HomeScreen(ui: UiState, vm: MainViewModel, onOpen: (String) -> Unit, onSettings: () -> Unit) {
    val clipboard = LocalClipboardManager.current
    var showAdd by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Card(Modifier.fillMaxWidth()) {
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("O teu QT Public ID (partilha com quem quiseres falar)", style = MaterialTheme.typography.labelLarge)
                SelectionContainer { Text(ui.publicId, style = MaterialTheme.typography.bodySmall) }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = { clipboard.setText(AnnotatedString(ui.publicId)) }) { Text("Copiar") }
                    if (BuildConfig.RELAY_EDITABLE) OutlinedButton(onClick = onSettings) { Text("Relay") }
                }
                Text(ui.status, style = MaterialTheme.typography.bodySmall)
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Button(onClick = { showAdd = true }) { Text("Adicionar contacto") }
            Text("Recibos de leitura", style = MaterialTheme.typography.bodySmall)
            Switch(checked = ui.readReceipts, onCheckedChange = vm::setReadReceipts)
        }
        Text("Conversas", style = MaterialTheme.typography.titleMedium)
        // Conversas com mensagens (mais recente primeiro) + contactos ainda sem mensagens.
        val withHistory = ui.conversations.map { it.contactId }.toSet()
        val idle = ui.contacts.filter { it.contactId !in withHistory }
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(ui.conversations, key = { it.contactId }) { conv ->
                val name = ui.contacts.firstOrNull { it.contactId == conv.contactId }?.nickname
                    ?: "Desconhecido ${conv.contactId.take(8)}…"
                ConversationRow(name, conv, onClick = { onOpen(conv.contactId) })
            }
            items(idle, key = { "idle-" + it.contactId }) { c ->
                Card(Modifier.fillMaxWidth().clickable { onOpen(c.contactId) }) {
                    Text(c.nickname, Modifier.padding(12.dp), style = MaterialTheme.typography.titleSmall)
                }
            }
        }
    }
    if (showAdd) AddContactDialog(onDismiss = { showAdd = false }, onAdd = { n, p -> vm.addContact(n, p); showAdd = false })
}

@Composable
fun AddContactDialog(onDismiss: () -> Unit, onAdd: (String, String) -> Unit) {
    var nick by remember { mutableStateOf("") }
    var pid by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { TextButton(onClick = { onAdd(nick, pid) }) { Text("Adicionar") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
        title = { Text("Adicionar contacto") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(nick, { nick = it }, label = { Text("Nome") }, singleLine = true)
                OutlinedTextField(pid, { pid = it }, label = { Text("QT Public ID (qt1:…)") })
                Text("Confirma o ID por outro canal (voz, pessoalmente): é isso que impede que o relay se faça passar pelo teu contacto.", style = MaterialTheme.typography.bodySmall)
            }
        },
    )
}

@Composable
fun ConversationRow(name: String, conv: ConversationSummary, onClick: () -> Unit) {
    val last = conv.lastMessage
    Card(Modifier.fillMaxWidth().clickable(onClick = onClick)) {
        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(name, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(shortTime(last.localAtMs), style = MaterialTheme.typography.labelSmall)
                }
                Text(
                    (if (last.outgoing) statusGlyph(last.status) + " " else "") + last.text,
                    style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            if (conv.unread > 0u) {
                Spacer(Modifier.width(8.dp))
                Badge { Text(conv.unread.toString()) }
            }
        }
    }
}

@Composable
fun ChatScreen(
    ui: UiState, contactId: String, contact: Contact?,
    onSend: (Contact, String) -> Unit, onBack: () -> Unit, onAdd: (String, String) -> Unit,
    onSafety: (Contact) -> Unit, onLoadOlder: () -> Unit,
) {
    var text by remember { mutableStateOf("") }
    var showAdd by remember { mutableStateOf(false) }
    // As mensagens vem da MAIS RECENTE para a mais antiga; com reverseLayout a
    // lista comeca colada ao fundo (como em qualquer mensageiro) e "subir"
    // percorre o passado.
    val msgs = ui.chatMessages
    val listState = rememberLazyListState()
    // Ao chegar perto do topo (as mais antigas), pede mais uma pagina ao Rust.
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
    // Mensagem nova no fundo: manter a vista no fundo se o utilizador ja la estava.
    val newest = msgs.firstOrNull()?.seq
    LaunchedEffect(newest) {
        if (listState.firstVisibleItemIndex <= 1) listState.animateScrollToItem(0)
    }
    Column(Modifier.fillMaxSize().padding(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("‹ Voltar") }
            Text(
                contact?.nickname ?: "Desconhecido ${contactId.take(8)}…",
                style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f),
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            if (contact != null) TextButton(onClick = { onSafety(contact) }) { Text("Safety number") }
        }
        LazyColumn(
            Modifier.weight(1f).fillMaxWidth(),
            state = listState,
            reverseLayout = true,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            items(msgs, key = { it.seq }) { m -> MessageBubble(m) }
            if (ui.chatHasMore) item { Text("A carregar mensagens anteriores…", Modifier.padding(8.dp), style = MaterialTheme.typography.labelSmall) }
        }
        Spacer(Modifier.height(8.dp))
        if (contact == null) {
            Text("Para responderes, adiciona o QT Public ID desta pessoa.")
            Button(onClick = { showAdd = true }) { Text("Adicionar contacto") }
        } else {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(text, { text = it }, Modifier.weight(1f), placeholder = { Text("Mensagem") })
                Button(onClick = { if (text.isNotBlank()) { onSend(contact, text); text = "" } }) { Text("Enviar") }
            }
        }
    }
    if (showAdd) AddContactDialog(onDismiss = { showAdd = false }, onAdd = { n, p -> onAdd(n, p); showAdd = false })
}

@Composable
fun MessageBubble(m: ChatMessage) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = if (m.outgoing) Arrangement.End else Arrangement.Start) {
        Card(
            colors = CardDefaults.cardColors(
                containerColor = if (m.outgoing) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant,
            ),
            modifier = Modifier.widthIn(max = 300.dp),
        ) {
            Column(Modifier.padding(horizontal = 10.dp, vertical = 6.dp)) {
                Text(m.text, style = MaterialTheme.typography.bodyMedium)
                Row(Modifier.align(Alignment.End), verticalAlignment = Alignment.CenterVertically) {
                    Text(shortTime(m.localAtMs), style = MaterialTheme.typography.labelSmall)
                    if (m.outgoing) {
                        Spacer(Modifier.width(4.dp))
                        Text(
                            statusGlyph(m.status),
                            style = MaterialTheme.typography.labelSmall,
                            color = if (m.status == MessageStatus.READ) Color(0xFF2196F3) else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

/** 🕓 por enviar · ✓ enviado (relay) · ✓✓ entregue · ✓✓ azul visto. */
fun statusGlyph(s: MessageStatus): String = when (s) {
    MessageStatus.PENDING -> "🕓"
    MessageStatus.SENT -> "✓"
    MessageStatus.DELIVERED, MessageStatus.READ -> "✓✓"
    MessageStatus.RECEIVED, MessageStatus.READ_BY_ME -> ""
}

private val timeFmt = java.text.SimpleDateFormat("HH:mm", java.util.Locale.getDefault())
private val dateFmt = java.text.SimpleDateFormat("dd/MM", java.util.Locale.getDefault())

/** Hora (hoje) ou dia/mes (dias anteriores). */
fun shortTime(ms: Long): String {
    val d = java.util.Date(ms)
    val today = dateFmt.format(java.util.Date())
    return if (dateFmt.format(d) == today) timeFmt.format(d) else dateFmt.format(d)
}
