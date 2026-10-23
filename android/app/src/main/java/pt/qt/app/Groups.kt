package pt.qt.app

import android.net.Uri
import java.io.File

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

/** Limites dos grupos (iguais aos do Rust, ffi/src/groups.rs). O nome e livre: o Rust trata dele. */
const val GROUP_MAX_MEMBERS = 10
const val GROUP_MAX_ADMINS = 3

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
    onSendImage: (Uri, Boolean, File?) -> Unit = { _, _, _ -> },
    onSendFile: (Uri) -> Unit = {},
    onSendVoice: (File, Long) -> Unit = { _, _ -> },
    onDraftRestored: () -> Unit = {},
) {
    val colors = LocalChatColors.current
    var reactingTo by remember { mutableStateOf<ChatMessage?>(null) }
    var viewing by remember { mutableStateOf<ChatMessage?>(null) }
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
                    Row(
                        Modifier.clickable(onClick = onInfo),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Avatar(group.name, group.groupId, sizeDp = 38)
                        Spacer(Modifier.width(10.dp))
                        Column {
                            Text(group.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(
                                s(R.string.t_pessoas_cifrado_ponta_a_ponta, group.members.size),
                                style = MaterialTheme.typography.labelSmall,
                                color = colors.onTopBar.copy(alpha = 0.8f),
                            )
                        }
                    }
                },
                actions = {
                    IconButton(onClick = onInfo) { Icon(Icons.Filled.Info, contentDescription = s(R.string.t_informacao_do_grupo)) }
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
                            m, ui.transfers[m.messageId], loadAttachment, onOpen = { viewing = m },
                            onLongPress = { if (group.active || m.attachment == null) reactingTo = m },
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
                            s(R.string.t_a_carregar_mensagens_anteriores), Modifier.fillMaxWidth().padding(8.dp),
                            style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Center,
                        )
                    }
                } else {
                    item(key = "e2e") { EncryptionNotice() }
                }
            }
            if (!group.active) {
                Text(
                    s(R.string.t_ja_nao_fazes_parte_deste_grupo),
                    Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).padding(16.dp),
                    style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center,
                )
            } else {
                ChatComposer(
                    onSendText = onSend,
                    onSendImage = onSendImage,
                    onSendFile = onSendFile,
                    onSendVoice = onSendVoice,
                    restoreText = ui.unsentText?.takeIf { it.first == group.conversationId }?.second,
                    onRestored = onDraftRestored,
                )
            }
        }
    }
    viewing?.let { m -> AttachmentOverlay(m, loadAttachment, onDismiss = { viewing = null }) }
    reactingTo?.let { m ->
        ReactionPicker(
            current = m.reactions.firstOrNull { it.mine }?.emoji,
            onPick = { e -> onReact(m.messageId, e); reactingTo = null },
            onDismiss = { reactingTo = null },
            text = if (m.attachment == null) coreText(m.text) else null,
            canReact = group.active,
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
        title = { Text(s(R.string.t_novo_grupo)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    name, { name = it },
                    label = { Text(s(R.string.t_nome_do_grupo_opcional)) }, singleLine = true, modifier = Modifier.fillMaxWidth(),
                )
                Text(
                    s(R.string.t_pessoas_opcional_ate_escolhidas_podes_acrescen, GROUP_MAX_MEMBERS - 1, chosen.size),
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (choices.isEmpty()) {
                    Text(s(R.string.t_ainda_nao_tens_contactos_o_grupo), style = MaterialTheme.typography.bodyMedium)
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
            ) { Text(s(R.string.t_criar)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
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

    Scaffold(topBar = { SealerTopBar(title = { Text(s(R.string.t_grupo)) }, onBack = onBack) }) { pad ->
        LazyColumn(Modifier.padding(pad).fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
            item(key = "head") {
                Column(Modifier.fillMaxWidth().padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Avatar(group.name, group.groupId, sizeDp = 72)
                    Spacer(Modifier.height(12.dp))
                    Text(group.name, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
                    Text(
                        s(R.string.t_de_pessoas, group.members.size, GROUP_MAX_MEMBERS),
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    if (group.iAmAdmin) {
                        TextButton(onClick = { rename = true }) { Text(s(R.string.t_mudar_o_nome)) }
                    }
                    if (!group.active) {
                        Text(s(R.string.t_ja_nao_fazes_parte_deste_grupo), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
                    }
                }
                HorizontalDivider()
            }
            if (group.iAmAdmin && group.members.size < GROUP_MAX_MEMBERS) {
                item(key = "add") {
                    TextButton(onClick = { adding = true }, modifier = Modifier.padding(horizontal = 8.dp)) { Text(s(R.string.t_adicionar_pessoas)) }
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
                            s(R.string.t_administrador),
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
                        TextButton(onClick = { confirmLeave = true }) { Text(s(R.string.t_sair_do_grupo), color = MaterialTheme.colorScheme.error) }
                    }
                    TextButton(onClick = { confirmDelete = true }) { Text(s(R.string.t_apagar_grupo_deste_telemovel), color = MaterialTheme.colorScheme.error) }
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
                        TextButton(onClick = { memberMenu = null; saveContact = m }) { Text(s(R.string.t_guardar_nos_contactos)) }
                    }
                    if (group.iAmAdmin) {
                        if (m.admin) {
                            TextButton(onClick = {
                                memberMenu = null
                                onUpdate(group.name, current().map { if (it.publicId == m.publicId) GroupMemberInput(it.publicId, false) else it })
                            }) { Text(s(R.string.t_deixar_de_ser_administrador)) }
                        } else {
                            TextButton(
                                enabled = admins < GROUP_MAX_ADMINS,
                                onClick = {
                                    memberMenu = null
                                    onUpdate(group.name, current().map { if (it.publicId == m.publicId) GroupMemberInput(it.publicId, true) else it })
                                },
                            ) { Text(if (admins < GROUP_MAX_ADMINS) s(R.string.t_tornar_administrador) else s(R.string.t_ja_ha_administradores, GROUP_MAX_ADMINS)) }
                        }
                        TextButton(onClick = {
                            memberMenu = null
                            onUpdate(group.name, current().filter { it.publicId != m.publicId })
                        }) { Text(s(R.string.t_retirar_do_grupo), color = MaterialTheme.colorScheme.error) }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { memberMenu = null }) { Text(s(R.string.t_fechar)) } },
        )
    }
    saveContact?.let { m ->
        var nick by remember(m.contactId) { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { saveContact = null },
            title = { Text(s(R.string.t_guardar_nos_contactos)) },
            text = {
                OutlinedTextField(nick, { nick = it }, label = { Text(s(R.string.t_nome)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            },
            confirmButton = {
                TextButton(enabled = nick.isNotBlank(), onClick = { onAddContact(nick.trim(), m.publicId); saveContact = null }) { Text(s(R.string.t_guardar)) }
            },
            dismissButton = { TextButton(onClick = { saveContact = null }) { Text(s(R.string.t_cancelar)) } },
        )
    }
    if (rename) {
        var name by remember { mutableStateOf(group.name) }
        AlertDialog(
            onDismissRequest = { rename = false },
            title = { Text(s(R.string.t_nome_do_grupo)) },
            text = {
                OutlinedTextField(
                    name, { name = it },
                    singleLine = true, modifier = Modifier.fillMaxWidth(),
                )
            },
            confirmButton = {
                TextButton(onClick = { rename = false; onUpdate(name.trim(), current()) }) { Text(s(R.string.t_guardar)) }
            },
            dismissButton = { TextButton(onClick = { rename = false }) { Text(s(R.string.t_cancelar)) } },
        )
    }
    if (adding) {
        val inGroup = group.members.map { it.contactId }.toSet()
        val choices = ui.contacts.filter { it.contactId !in inGroup && it.contactId !in ui.blocked }
        var chosen by remember { mutableStateOf(setOf<String>()) }
        val room = GROUP_MAX_MEMBERS - group.members.size
        AlertDialog(
            onDismissRequest = { adding = false },
            title = { Text(s(R.string.t_adicionar_pessoas_2)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(s(R.string.t_ainda_cabem_escolhidos, room, chosen.size), style = MaterialTheme.typography.bodySmall)
                    if (choices.isEmpty()) Text(s(R.string.t_nao_ha_contactos_para_adicionar))
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
                }) { Text(s(R.string.t_adicionar)) }
            },
            dismissButton = { TextButton(onClick = { adding = false }) { Text(s(R.string.t_cancelar)) } },
        )
    }
    if (confirmLeave) {
        AlertDialog(
            onDismissRequest = { confirmLeave = false },
            title = { Text(s(R.string.t_sair_de, group.name)) },
            text = { ObscuredTouchGuard(); Text(s(R.string.t_deixas_de_receber_as_mensagens_do)) },
            confirmButton = {
                TextButton(onClick = { confirmLeave = false; onLeave() }) { Text(s(R.string.t_sair), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmLeave = false }) { Text(s(R.string.t_cancelar)) } },
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
        title = { Text(s(R.string.t_apagar, name)) },
        text = { ObscuredTouchGuard()
            Text(
                s(R.string.t_sais_do_grupo_se_ainda_estiveres),
            )
        },
        confirmButton = { TextButton(onClick = onConfirm) { Text(s(R.string.t_apagar_2), color = MaterialTheme.colorScheme.error) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
    )
}
