package pt.qt.app

import android.Manifest
import android.content.pm.PackageManager
import android.net.Uri
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.content.MediaType
import androidx.compose.foundation.content.ReceiveContentListener
import androidx.compose.foundation.content.consume
import androidx.compose.foundation.content.contentReceiver
import androidx.compose.foundation.content.hasMediaType
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import java.io.File

/**
 * Caixa de escrita das conversas (a dois e de grupo): texto, imagens e GIFs (galeria e
 * teclado), ficheiros ate 25 MB e mensagens de voz ate 10 s (o botao grava quando a
 * caixa esta vazia).
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ChatComposer(
    onSendText: (String) -> Unit,
    onSendImage: (Uri, Boolean, File?) -> Unit,
    onSendFile: (Uri) -> Unit,
    onSendVoice: (File, Long) -> Unit,
) {
    val ctx = LocalContext.current
    // TextFieldState (e nao String): so este campo recebe GIFs e autocolantes do
    // teclado (Gboard, SwiftKey...). Tambem sobrevive a rodar o ecra.
    val input = rememberTextFieldState()
    // Imagem ou ficheiro escolhido, a espera de confirmacao.
    var picked by rememberSaveable { mutableStateOf<Uri?>(null) }
    var pickedFile by rememberSaveable { mutableStateOf<Uri?>(null) }
    var attachMenu by remember { mutableStateOf(false) }
    val pickImage = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri != null) picked = uri
    }
    val pickFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) pickedFile = uri
    }
    // Mensagens de voz: gravador (para e apaga a gravacao se o ecra sair) e permissao do microfone.
    val recorder = remember { VoiceRecorder(ctx) }
    var recording by remember { mutableStateOf(false) }
    DisposableEffect(Unit) { onDispose { recorder.cancel() } }
    fun startRecording() {
        recording = try {
            recorder.start(); true
        } catch (e: Exception) {
            Toast.makeText(ctx, s(R.string.t_nao_foi_possivel_usar_o_microfone), Toast.LENGTH_SHORT).show()
            false
        }
    }
    val askMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) startRecording()
        else Toast.makeText(ctx, s(R.string.t_sem_acesso_ao_microfone_nao_e), Toast.LENGTH_SHORT).show()
    }
    val sendImage by rememberUpdatedState(onSendImage)
    val keyboardImages = remember {
        ReceiveContentListener { content ->
            if (!content.hasMediaType(MediaType.Image)) return@ReceiveContentListener content
            var taken = false
            content.consume { item ->
                val uri = item.uri ?: return@consume false
                if (!taken) {
                    taken = true
                    // Copia ja, enquanto o teclado da permissao de leitura.
                    ImagePrep.copyToCache(ctx, uri)?.let { f -> sendImage(Uri.fromFile(f), false, f) }
                }
                true
            }
        }
    }

    if (recording) {
        VoiceRecordBar(
            recorder,
            onCancel = { recorder.cancel(); recording = false },
            onSend = {
                val rec = recorder.stop()
                recording = false
                if (rec != null) onSendVoice(rec.first, rec.second)
            },
        )
    } else {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            Box(Modifier.padding(bottom = 4.dp)) {
                IconButton(onClick = { attachMenu = true }) {
                    Icon(Icons.Filled.Add, contentDescription = s(R.string.t_enviar_foto_gif_ou_ficheiro))
                }
                DropdownMenu(expanded = attachMenu, onDismissRequest = { attachMenu = false }) {
                    DropdownMenuItem(
                        text = { Text(s(R.string.t_foto_ou_gif)) },
                        leadingIcon = { Text("📷", fontSize = 18.sp) },
                        onClick = {
                            attachMenu = false
                            // O seletor de fotos e outro ecra: nao conta como sair da app (bloqueio).
                            AutoLockPause.active = true
                            pickImage.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                        },
                    )
                    DropdownMenuItem(
                        text = { Text(s(R.string.t_ficheiro_ate_25_mb)) },
                        leadingIcon = { Text("📄", fontSize = 18.sp) },
                        onClick = {
                            attachMenu = false
                            AutoLockPause.active = true
                            pickFile.launch(arrayOf("*/*"))
                        },
                    )
                }
            }
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
                            Text(s(R.string.t_mensagem), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        inner()
                    }
                },
            )
            Spacer(Modifier.width(6.dp))
            // Sem texto escrito, o botao grava uma mensagem de voz (ate 10 s).
            val empty = input.text.isBlank()
            FloatingActionButton(
                onClick = {
                    val text = input.text.toString()
                    if (text.isNotBlank()) {
                        onSendText(text); input.clearText()
                    } else if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                        startRecording()
                    } else {
                        AutoLockPause.active = true   // o pedido de permissao nao conta como sair da app
                        askMic.launch(Manifest.permission.RECORD_AUDIO)
                    }
                },
                shape = CircleShape,
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
                modifier = Modifier.size(52.dp),
            ) {
                if (empty) Text("🎤", fontSize = 22.sp)
                else Icon(Icons.AutoMirrored.Filled.Send, contentDescription = s(R.string.t_enviar))
            }
        }
    }

    picked?.let { uri ->
        SendImageDialog(
            uri,
            onSend = { original -> onSendImage(uri, original, null); picked = null },
            onDismiss = { picked = null },
        )
    }
    pickedFile?.let { uri ->
        SendFileDialog(
            uri,
            onSend = { onSendFile(uri); pickedFile = null },
            onDismiss = { pickedFile = null },
        )
    }
}

/**
 * Contra "tapjacking": enquanto este dialogo estiver aberto, os toques que lhe chegam
 * atraves de outra janela por cima (uma app maliciosa com uma sobreposicao transparente)
 * sao ignorados. Usado nos dialogos que apagam, bloqueiam ou mudam o codigo. So afeta a
 * janela do dialogo, nao o resto da app (filtros de luz azul continuam a funcionar la).
 */
@Composable
fun ObscuredTouchGuard() {
    val view = LocalView.current
    DisposableEffect(view) {
        val root = view.rootView
        val before = root.filterTouchesWhenObscured
        root.filterTouchesWhenObscured = true
        onDispose { root.filterTouchesWhenObscured = before }
    }
}
