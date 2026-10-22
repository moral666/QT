package pt.qt.app

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import uniffi.qt_ffi.recoveryIsWord

/** Palavras confirmadas antes de dar a chave por escrita. */
private const val CHECKS = 3

/**
 * Enquanto as palavras estao no ecra: sem capturas de ecra nem miniatura nas apps recentes
 * (FLAG_SECURE so neste ecra).
 */
@Composable
private fun SecureWhileShown() {
    val activity = LocalContext.current as? MainActivity ?: return
    DisposableEffect(Unit) {
        activity.secureScreens++
        onDispose { activity.secureScreens-- }
    }
}

/** As 12 palavras numeradas, em duas colunas. */
@Composable
private fun WordGrid(words: List<String>) {
    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
        Row(Modifier.fillMaxWidth().padding(16.dp)) {
            for (col in 0..1) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (i in (col * 6) until (col * 6 + 6)) {
                        if (i < words.size) {
                            Text("${i + 1}. ${words[i]}", fontFamily = FontFamily.Monospace, fontWeight = FontWeight.SemiBold)
                        }
                    }
                }
            }
        }
    }
}

/**
 * Definicoes -> Chave de recuperacao. Sem chave: explica e cria. A criar: mostra as palavras e
 * pede 3 delas. Ativa: ver as palavras, gerar novas ou desligar (o relay apaga a copia).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RecoveryScreen(
    ui: UiState,
    onBack: () -> Unit,
    onCreate: () -> Unit,
    onShow: () -> Unit,
    onWritten: () -> Unit,
    onHide: () -> Unit,
    onDisable: () -> Unit,
    onCode: (String) -> Unit = {},
    onCancelCode: () -> Unit = {},
) {
    var checking by remember(ui.recoveryWords) { mutableStateOf(false) }
    var confirmNew by remember { mutableStateOf(false) }
    var confirmOff by remember { mutableStateOf(false) }
    // So pede as palavras a quem acabou de as criar (ver de novo nao pede).
    var justCreated by remember { mutableStateOf(false) }
    val words = ui.recoveryWords
    Scaffold(
        topBar = {
            SealerTopBar(title = { Text(s(R.string.recovery_title)) }, onBack = { if (words != null) onHide() else onBack() })
        },
    ) { pad ->
        Column(
            Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            when {
                ui.recoveryBusy -> {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.height(24.dp).width(24.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(12.dp))
                        Text(s(R.string.recovery_working))
                    }
                }
                words != null && checking -> CheckWords(words, onOk = { justCreated = false; onWritten() }, onBack = { checking = false })
                words != null -> {
                    SecureWhileShown()
                    Text(s(R.string.recovery_write_down), style = MaterialTheme.typography.bodyMedium)
                    WordGrid(words)
                    Text(s(R.string.recovery_warning), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                    Button(
                        onClick = { if (justCreated) checking = true else onHide() },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(if (justCreated) s(R.string.recovery_written) else s(R.string.recovery_done)) }
                }
                ui.recoveryOn -> {
                    Text(s(R.string.recovery_on), style = MaterialTheme.typography.bodyMedium)
                    Text(s(R.string.recovery_intro), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Button(onClick = onShow, modifier = Modifier.fillMaxWidth()) { Text(s(R.string.recovery_show)) }
                    OutlinedButton(onClick = { confirmNew = true }, modifier = Modifier.fillMaxWidth()) { Text(s(R.string.recovery_new)) }
                    TextButton(onClick = { confirmOff = true }) { Text(s(R.string.recovery_disable), color = MaterialTheme.colorScheme.error) }
                }
                else -> {
                    Text(s(R.string.recovery_intro), style = MaterialTheme.typography.bodyMedium)
                    Text(s(R.string.recovery_privacy), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Button(onClick = { justCreated = true; onCreate() }, modifier = Modifier.fillMaxWidth()) { Text(s(R.string.recovery_enable)) }
                }
            }
        }
    }
    if (ui.recoveryAskCode) {
        // Com o bloqueio da app: o codigo dela antes de ver ou gerar as palavras (0.9.23).
        CodeDialog(
            title = s(R.string.recovery_title), text = s(R.string.recovery_auth),
            askCurrent = true, askNew = false, confirm = s(R.string.t_continuar),
            onDismiss = onCancelCode,
            onConfirm = { cur, _ -> onCode(cur) },
        )
    }
    if (confirmNew) {
        AlertDialog(
            onDismissRequest = { confirmNew = false },
            title = { Text(s(R.string.recovery_new)) },
            text = { Text(s(R.string.recovery_new_confirm)) },
            confirmButton = { TextButton(onClick = { confirmNew = false; justCreated = true; onCreate() }) { Text(s(R.string.recovery_new)) } },
            dismissButton = { TextButton(onClick = { confirmNew = false }) { Text(s(R.string.t_cancelar)) } },
        )
    }
    if (confirmOff) {
        AlertDialog(
            onDismissRequest = { confirmOff = false },
            title = { Text(s(R.string.recovery_disable)) },
            text = { Text(s(R.string.recovery_disable_confirm)) },
            confirmButton = {
                TextButton(onClick = { confirmOff = false; onDisable() }) { Text(s(R.string.recovery_disable), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmOff = false }) { Text(s(R.string.t_cancelar)) } },
        )
    }
}

/** Pede 3 palavras ao acaso (pela posicao) para confirmar que foram escritas. */
@Composable
private fun CheckWords(words: List<String>, onOk: () -> Unit, onBack: () -> Unit) {
    val positions = remember(words) { words.indices.shuffled().take(CHECKS).sorted() }
    val typed = remember(words) { mutableStateOf(List(CHECKS) { "" }) }
    var wrong by remember { mutableStateOf(false) }
    Text(s(R.string.recovery_check_intro), style = MaterialTheme.typography.bodyMedium)
    positions.forEachIndexed { i, pos ->
        OutlinedTextField(
            value = typed.value[i],
            onValueChange = { v -> typed.value = typed.value.toMutableList().also { it[i] = v }; wrong = false },
            label = { Text(s(R.string.recovery_check, pos + 1)) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false,
                keyboardType = KeyboardType.Password, imeAction = ImeAction.Next,
            ),
        )
    }
    if (wrong) Text(s(R.string.recovery_check_wrong), color = MaterialTheme.colorScheme.error)
    Button(
        onClick = {
            val ok = positions.indices.all { typed.value[it].trim().lowercase() == words[positions[it]] }
            if (ok) onOk() else wrong = true
        },
        modifier = Modifier.fillMaxWidth(),
    ) { Text(s(R.string.recovery_done)) }
    TextButton(onClick = onBack) { Text(s(R.string.recovery_see_again)) }
}

/**
 * Primeiro ecra -> Recuperar conta: as 12 palavras (escritas ou coladas, separadas por
 * espacos). Avisa logo de palavras que nao existem na lista.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RestoreScreen(ui: UiState, onBack: () -> Unit, onRestore: (List<String>) -> Unit) {
    var text by remember { mutableStateOf("") }
    val words = text.trim().lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
    val unknown = words.filter { !recoveryIsWord(it) }
    SecureWhileShown()
    Scaffold(topBar = { SealerTopBar(title = { Text(s(R.string.recovery_restore)) }, onBack = onBack) }) { pad ->
        Column(
            Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text(s(R.string.recovery_restore_hint), style = MaterialTheme.typography.bodyMedium)
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                modifier = Modifier.fillMaxWidth().height(160.dp),
                textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Password,
                ),
                enabled = !ui.recoveryBusy,
            )
            Text(s(R.string.recovery_words_count, words.size), style = MaterialTheme.typography.labelMedium)
            if (unknown.isNotEmpty()) {
                Text(s(R.string.recovery_unknown_words, unknown.joinToString(", ")), color = MaterialTheme.colorScheme.error)
            }
            Text(s(R.string.recovery_restore_warning), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (ui.recoveryBusy) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.height(24.dp).width(24.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(12.dp))
                    Text(s(R.string.recovery_restoring))
                }
            } else {
                Button(
                    onClick = { onRestore(words) },
                    enabled = words.size == 12 && unknown.isEmpty(),
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(s(R.string.recovery_restore)) }
            }
        }
    }
}
