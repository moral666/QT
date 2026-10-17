package pt.qt.app

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import uniffi.qt_ffi.ChatMessage
import uniffi.qt_ffi.GroupInfo
import uniffi.qt_ffi.GroupMember
import uniffi.qt_ffi.GroupMemberInput

/** Limites dos grupos (iguais aos do Rust, ffi/src/groups.rs). */
const val GROUP_MAX_MEMBERS = 10
const val GROUP_MAX_ADMINS = 3
const val GROUP_MAX_NAME = 100

/** Conversa de um grupo: so texto e reacoes nesta versao (imagens e voz ainda nao). */
@OptIn(ExperimentalLayoutApi::class, ExperimentalFoundationApi::class)
@Composable
fun GroupChatScreen(
    ui: UiState,
    group: GroupInfo,
    onSend: (String) -> Unit,
    onReact: (String, String) -> Unit,
    onBack: () -> Unit,
    onLoadOlder: () -> Unit,
    onInfo: () -> Unit,
    loadAttachment: suspend (String, String) -> ByteArray?,
) {
    val colors = LocalChatColors.current
    val input = rememberTextFieldState()
    var reactingTo by remember { mutableStateOf<ChatMessage?>(null) }
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
                    Row(
                        Modifier.clickable(onClick = onInfo),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Avatar(group.name, group.groupId, sizeDp = 38)
                        Spacer(Modifier.width(10.dp))
                        Column {
                            Text(group.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(
                                "${group.members.size} pessoas · cifrado ponta a ponta",
                                style = MaterialTheme.typography.labelSmall,
                                color = colors.onTopBar.copy(alpha = 0.8f),
                            )
                        }
                    }
                },
                actions = {
                    IconButton(onClick = onInfo) { Icon(Icons.Filled.Info, contentDescription = "Informação do grupo") }
                },
            )
        },
    ) { pad ->
        Column(Modifier.padding(pad).consumeWindowInsets(pad).fillMaxSize().background(colors.chatBackground).imePadding()) {
            LazyColumn(
                Modifier.weight(1f).fillMaxWidth(),
                state = listState,
                reverseLayout = true,
                verticalArrangement = Arrangement.spacedBy(3.dp),
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 8.dp),
            ) {
                for (i in msgs.indices) {
                    val m = msgs[i]
                    item(key = m.seq) {
                        MessageBubble(
                            m, ui.transfers[m.messageId], loadAttachment,
                            onLongPress = { if (group.active) reactingTo = m },
                            senderName = if (m.outgoing) null else ui.memberName(m.sender),
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
                            "A carregar mensagens anteriores…", Modifier.fillMaxWidth().padding(8.dp),
                            style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Center,
                        )
                    }
                } else {
                    item(key = "e2e") { EncryptionNotice() }
                }
            }
            if (!group.active) {
                Text(
                    "Já não fazes parte deste grupo.",
                    Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).padding(16.dp),
                    style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center,
                )
            } else {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.Bottom,
                ) {
                    BasicTextField(
                        state = input,
                        modifier = Modifier.weight(1f),
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
                            if (text.isNotBlank()) {
                                onSend(text)
                                input.clearText()
                            }
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
    reactingTo?.let { m ->
        ReactionPicker(
            current = m.reactions.firstOrNull { it.mine }?.emoji,
            onPick = { e -> onReact(m.messageId, e); reactingTo = null },
            onDismiss = { reactingTo = null },
        )
    }
}

/** NOVO GRUPO: nome + ate 9 contactos. */
@Composable
fun NewGroupDialog(ui: UiState, onDismiss: () -> Unit, onCreate: (String, List<Contact>) -> Unit) {
    var name by rememberSaveable { mutableStateOf("") }
    var chosen by remember { mutableStateOf(setOf<String>()) }
    val choices = ui.contacts.filter { it.contactId !in ui.blocked }
    val full = chosen.size >= GROUP_MAX_MEMBERS - 1
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Novo grupo") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    name, { if (it.toByteArray().size <= GROUP_MAX_NAME) name = it },
                    label = { Text("Nome do grupo") }, singleLine = true, modifier = Modifier.fillMaxWidth(),
                )
                Text(
                    "Escolhe até ${GROUP_MAX_MEMBERS - 1} contactos (${chosen.size} escolhidos).",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (choices.isEmpty()) {
                    Text("Ainda não tens contactos. Adiciona primeiro as pessoas do grupo.", style = MaterialTheme.typography.bodyMedium)
                }
                LazyColumn(Modifier.heightIn(max = 320.dp)) {
                    items(choices, key = { it.contactId }) { c ->
                        val on = c.contactId in chosen
                        Row(
                            Modifier.fillMaxWidth().clickable(enabled = on || !full) {
                                chosen = if (on) chosen - c.contactId else chosen + c.contactId
                            }.padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Checkbox(checked = on, onCheckedChange = null, enabled = on || !full)
                            Spacer(Modifier.width(8.dp))
                            Avatar(c.nickname, c.contactId, sizeDp = 32)
                            Spacer(Modifier.width(10.dp))
                            Text(c.nickname, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onCreate(name.trim(), choices.filter { it.contactId in chosen }) },
                enabled = name.isNotBlank() && chosen.isNotEmpty(),
            ) { Text("Criar") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
}

/**
 * Informacao do grupo: membros e administradores. Os administradores (ate 3) mudam o
 * nome, acrescentam e retiram pessoas e escolhem os outros administradores.
 */
@Composable
fun GroupInfoScreen(
    ui: UiState,
    group: GroupInfo,
    onBack: () -> Unit,
    onUpdate: (String, List<GroupMemberInput>) -> Unit,
    onLeave: () -> Unit,
    onDelete: () -> Unit,
    onAddContact: (String, String) -> Unit,
) {
    var rename by remember { mutableStateOf(false) }
    var adding by remember { mutableStateOf(false) }
    var memberMenu by remember { mutableStateOf<GroupMember?>(null) }
    var saveContact by remember { mutableStateOf<GroupMember?>(null) }
    var confirmLeave by remember { mutableStateOf(false) }
    var confirmDelete by remember { mutableStateOf(false) }
    val admins = group.members.count { it.admin }
    fun current() = group.members.map { GroupMemberInput(it.publicId, it.admin) }
    fun isContact(m: GroupMember) = m.me || ui.contacts.any { it.contactId == m.contactId }

    Scaffold(topBar = { SealerTopBar(title = { Text("Grupo") }, onBack = onBack) }) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
            item(key = "head") {
                Column(Modifier.fillMaxWidth().padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Avatar(group.name, group.groupId, sizeDp = 72)
                    Spacer(Modifier.height(12.dp))
                    Text(group.name, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
                    Text(
                        "${group.members.size} de $GROUP_MAX_MEMBERS pessoas",
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    if (group.iAmAdmin) {
                        TextButton(onClick = { rename = true }) { Text("Mudar o nome") }
                    }
                    if (!group.active) {
                        Text("Já não fazes parte deste grupo.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
                    }
                }
                HorizontalDivider()
            }
            if (group.iAmAdmin && group.members.size < GROUP_MAX_MEMBERS) {
                item(key = "add") {
                    TextButton(onClick = { adding = true }, modifier = Modifier.padding(horizontal = 8.dp)) { Text("+ Adicionar pessoas") }
                }
            }
            items(group.members, key = { it.contactId }) { m ->
                Row(
                    Modifier.fillMaxWidth()
                        .clickable(enabled = !m.me && (group.iAmAdmin || !isContact(m))) { memberMenu = m }
                        .padding(horizontal = 16.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val n = ui.memberName(m.contactId)
                    Avatar(n, m.contactId, sizeDp = 40, online = m.contactId in ui.online)
                    Spacer(Modifier.width(12.dp))
                    Text(n, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (m.admin) {
                        Text(
                            "Administrador",
                            Modifier.clip(RoundedCornerShape(8.dp)).background(MaterialTheme.colorScheme.secondaryContainer)
                                .padding(horizontal = 8.dp, vertical = 2.dp),
                            fontSize = 12.sp, color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                    }
                }
            }
            item(key = "actions") {
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
                Column(Modifier.padding(horizontal = 8.dp)) {
                    if (group.active) {
                        TextButton(onClick = { confirmLeave = true }) { Text("Sair do grupo", color = MaterialTheme.colorScheme.error) }
                    }
                    TextButton(onClick = { confirmDelete = true }) { Text("Apagar grupo deste telemóvel", color = MaterialTheme.colorScheme.error) }
                }
            }
        }
    }

    memberMenu?.let { m ->
        val n = ui.memberName(m.contactId)
        AlertDialog(
            onDismissRequest = { memberMenu = null },
            title = { Text(n, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            text = {
                Column {
                    if (!isContact(m)) {
                        TextButton(onClick = { memberMenu = null; saveContact = m }) { Text("Guardar nos contactos") }
                    }
                    if (group.iAmAdmin) {
                        if (m.admin) {
                            TextButton(onClick = {
                                memberMenu = null
                                onUpdate(group.name, current().map { if (it.publicId == m.publicId) GroupMemberInput(it.publicId, false) else it })
                            }) { Text("Deixar de ser administrador") }
                        } else {
                            TextButton(
                                enabled = admins < GROUP_MAX_ADMINS,
                                onClick = {
                                    memberMenu = null
                                    onUpdate(group.name, current().map { if (it.publicId == m.publicId) GroupMemberInput(it.publicId, true) else it })
                                },
                            ) { Text(if (admins < GROUP_MAX_ADMINS) "Tornar administrador" else "Já há $GROUP_MAX_ADMINS administradores") }
                        }
                        TextButton(onClick = {
                            memberMenu = null
                            onUpdate(group.name, current().filter { it.publicId != m.publicId })
                        }) { Text("Retirar do grupo", color = MaterialTheme.colorScheme.error) }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { memberMenu = null }) { Text("Fechar") } },
        )
    }
    saveContact?.let { m ->
        var nick by remember(m.contactId) { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { saveContact = null },
            title = { Text("Guardar nos contactos") },
            text = {
                OutlinedTextField(nick, { nick = it }, label = { Text("Nome") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            },
            confirmButton = {
                TextButton(enabled = nick.isNotBlank(), onClick = { onAddContact(nick.trim(), m.publicId); saveContact = null }) { Text("Guardar") }
            },
            dismissButton = { TextButton(onClick = { saveContact = null }) { Text("Cancelar") } },
        )
    }
    if (rename) {
        var name by remember { mutableStateOf(group.name) }
        AlertDialog(
            onDismissRequest = { rename = false },
            title = { Text("Nome do grupo") },
            text = {
                OutlinedTextField(
                    name, { if (it.toByteArray().size <= GROUP_MAX_NAME) name = it },
                    singleLine = true, modifier = Modifier.fillMaxWidth(),
                )
            },
            confirmButton = {
                TextButton(enabled = name.isNotBlank(), onClick = { rename = false; onUpdate(name.trim(), current()) }) { Text("Guardar") }
            },
            dismissButton = { TextButton(onClick = { rename = false }) { Text("Cancelar") } },
        )
    }
    if (adding) {
        val inGroup = group.members.map { it.contactId }.toSet()
        val choices = ui.contacts.filter { it.contactId !in inGroup && it.contactId !in ui.blocked }
        var chosen by remember { mutableStateOf(setOf<String>()) }
        val room = GROUP_MAX_MEMBERS - group.members.size
        AlertDialog(
            onDismissRequest = { adding = false },
            title = { Text("Adicionar pessoas") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Ainda cabem $room (${chosen.size} escolhidos).", style = MaterialTheme.typography.bodySmall)
                    if (choices.isEmpty()) Text("Não há contactos para adicionar.")
                    LazyColumn(Modifier.heightIn(max = 320.dp)) {
                        items(choices, key = { it.contactId }) { c ->
                            val on = c.contactId in chosen
                            val can = on || chosen.size < room
                            Row(
                                Modifier.fillMaxWidth().clickable(enabled = can) {
                                    chosen = if (on) chosen - c.contactId else chosen + c.contactId
                                }.padding(vertical = 4.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Checkbox(checked = on, onCheckedChange = null, enabled = can)
                                Spacer(Modifier.width(8.dp))
                                Text(c.nickname, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(enabled = chosen.isNotEmpty(), onClick = {
                    adding = false
                    val add = choices.filter { it.contactId in chosen }.map { GroupMemberInput(it.publicId, false) }
                    onUpdate(group.name, current() + add)
                }) { Text("Adicionar") }
            },
            dismissButton = { TextButton(onClick = { adding = false }) { Text("Cancelar") } },
        )
    }
    if (confirmLeave) {
        AlertDialog(
            onDismissRequest = { confirmLeave = false },
            title = { Text("Sair de ${group.name}?") },
            text = { Text("Deixas de receber as mensagens do grupo. As que já tens ficam neste telemóvel até apagares o grupo.") },
            confirmButton = {
                TextButton(onClick = { confirmLeave = false; onLeave() }) { Text("Sair", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmLeave = false }) { Text("Cancelar") } },
        )
    }
    if (confirmDelete) {
        GroupDeleteDialog(group.name, onDismiss = { confirmDelete = false }, onConfirm = { confirmDelete = false; onDelete() })
    }
}

/** Confirmacao para apagar um grupo (sai dele, se ainda estiver, e apaga as mensagens). */
@Composable
fun GroupDeleteDialog(name: String, onDismiss: () -> Unit, onConfirm: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Apagar $name?") },
        text = {
            Text(
                "Sais do grupo (se ainda estiveres) e todas as mensagens dele são apagadas para sempre deste telemóvel. " +
                    "As mensagens nos telemóveis dos outros membros não são apagadas.",
            )
        },
        confirmButton = { TextButton(onClick = onConfirm) { Text("Apagar", color = MaterialTheme.colorScheme.error) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancelar") } },
    )
}
