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
import androidx.compose.material3.AlertDialog
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
    var openChat by remember { mutableStateOf<String?>(null) }   // contactId
    var showSettings by remember { mutableStateOf(false) }

    when {
        ui.loading -> Column(Modifier.fillMaxSize().padding(24.dp)) { Text("A abrir o armazenamento seguro…") }
        !ui.hasIdentity -> SetupScreen(ui, onCreate = vm::createIdentity)
        openChat != null -> {
            BackHandler { openChat = null }
            val contact = ui.contacts.firstOrNull { it.contactId == openChat }
            ChatScreen(ui, openChat!!, contact, onSend = vm::send, onBack = { openChat = null }, onAdd = vm::addContact, onSafety = vm::showSafetyNumber)
        }
        showSettings && BuildConfig.RELAY_EDITABLE -> {
            BackHandler { showSettings = false }
            SettingsScreen(ui, onSave = { u, k -> vm.saveRelay(u, k); showSettings = false })
        }
        else -> HomeScreen(ui, vm, onOpen = { openChat = it }, onSettings = { showSettings = true })
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
        Button(onClick = { showAdd = true }) { Text("Adicionar contacto") }
        Text("Conversas", style = MaterialTheme.typography.titleMedium)
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(ui.contacts) { c ->
                Card(Modifier.fillMaxWidth()) {
                    TextButton(onClick = { onOpen(c.contactId) }, modifier = Modifier.fillMaxWidth()) { Text(c.nickname) }
                }
            }
            items(ui.unknownSenders.toList()) { id ->
                Card(Modifier.fillMaxWidth()) {
                    TextButton(onClick = { onOpen(id) }, modifier = Modifier.fillMaxWidth()) { Text("Desconhecido ${id.take(8)}… (nova mensagem)") }
                }
            }
        }
        Text("Nota: o histórico só existe em memória nesta versão.", style = MaterialTheme.typography.bodySmall)
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
fun ChatScreen(
    ui: UiState, contactId: String, contact: Contact?,
    onSend: (Contact, String) -> Unit, onBack: () -> Unit, onAdd: (String, String) -> Unit,
    onSafety: (Contact) -> Unit,
) {
    var text by remember { mutableStateOf("") }
    var showAdd by remember { mutableStateOf(false) }
    val msgs = ui.chats[contactId].orEmpty()
    Column(Modifier.fillMaxSize().padding(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("‹ Voltar") }
            Text(contact?.nickname ?: "Desconhecido ${contactId.take(8)}…", style = MaterialTheme.typography.titleMedium)
            if (contact != null) TextButton(onClick = { onSafety(contact) }) { Text("Safety number") }
        }
        LazyColumn(Modifier.weight(1f).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            items(msgs) { m ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = if (m.fromMe) Arrangement.End else Arrangement.Start) {
                    Card { Text((if (m.pending) "⏳ " else "") + m.text, Modifier.padding(8.dp)) }
                }
            }
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
