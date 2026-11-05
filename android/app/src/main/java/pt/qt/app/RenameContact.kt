package pt.qt.app

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * "MUDAR O NOME" de um contacto (0.9.72): com o mesmo desenho do nome do grupo. Enquanto falta
 * tempo para a proxima mudanca (10 minutos), diz quanto falta e o "Guardar" fica desligado.
 */
@Composable
fun RenameContactDialog(contact: Contact, onSave: (String) -> Unit, onDismiss: () -> Unit) {
    var name by remember { mutableStateOf(contact.nickname) }
    val wait = remember { contact.renameWaitMs() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(s(R.string.t_mudar_o_nome)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    name, { name = it }, label = { Text(s(R.string.t_nome)) },
                    singleLine = true, enabled = wait == 0L, modifier = Modifier.fillMaxWidth(),
                )
                if (wait > 0L) {
                    Text(s(R.string.contact_rename_wait, renameWaitMinutes(wait)), style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = wait == 0L && name.isNotBlank() && name.trim() != contact.nickname,
                onClick = { onSave(name.trim()) },
            ) { Text(s(R.string.t_guardar)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(s(R.string.t_cancelar)) } },
    )
}
