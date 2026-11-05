package pt.qt.app

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** Na chamada: eu e mais 4 (como no nucleo, `MAX_GROUP_CALL`). */
const val CALL_MAX_PEOPLE = 5

/**
 * "ADICIONAR A LIGACAO" (0.9.71): escolher contactos para a chamada, com o mesmo desenho do
 * "Adicionar pessoas" dos grupos. `inCall`: quem ja esta (nao aparece na lista); `room`: quantos
 * ainda cabem.
 */
@Composable
fun AddToCallDialog(
    contacts: List<Contact>,
    blocked: Set<String>,
    inCall: Set<String>,
    room: Int,
    onAdd: (List<Contact>) -> Unit,
    onDismiss: () -> Unit,
) {
    val choices = contacts.filter { it.contactId !in inCall && it.contactId !in blocked }
    var chosen by remember { mutableStateOf(setOf<String>()) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.call_add)) },
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
                onAdd(choices.filter { it.contactId in chosen })
            }) { Text(s(R.string.t_adicionar)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
    )
}
