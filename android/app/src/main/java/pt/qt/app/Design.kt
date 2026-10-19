package pt.qt.app

import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.AccountBox
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Build
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Done
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Face
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import com.google.zxing.client.android.Intents

// Design da 0.9.11: cabecalho roxo com o fundo curvo (canto inferior direito), ecra de
// Definicoes (em vez do menu de tres pontos) e ecra Novo contacto (em vez da janela).

/**
 * Forma dos cabecalhos: em baixo, a direita, uma curva larga que comeca a 36% da largura
 * e sobe `curve` ate ao lado direito.
 */
class CurvedBottomShape(private val curve: Dp, private val widthFraction: Float = 0.64f) : Shape {
    override fun createOutline(size: Size, layoutDirection: LayoutDirection, density: Density): Outline {
        val ry = with(density) { curve.toPx() }.coerceAtMost(size.height)
        val rx = size.width * widthFraction
        val path = Path().apply {
            moveTo(0f, 0f)
            lineTo(size.width, 0f)
            lineTo(size.width, size.height - ry)
            arcTo(Rect(size.width - 2 * rx, size.height - 2 * ry, size.width, size.height), 0f, 90f, false)
            lineTo(0f, size.height)
            close()
        }
        return Outline.Generic(path)
    }
}

/** Fundo "galaxia" dos cabecalhos: gradiente da marca, estrelas fixas, brilho na curva. */
@Composable
fun GalaxyHeader(curve: Dp, modifier: Modifier = Modifier, content: @Composable BoxScope.() -> Unit) {
    val c = LocalChatColors.current
    // Estrelas: posicoes fixas (sempre as mesmas), pequenas e meio transparentes.
    val stars = remember {
        val r = java.util.Random(7)
        List(28) { floatArrayOf(r.nextFloat(), r.nextFloat(), 0.6f + r.nextFloat() * 0.8f, 0.25f + r.nextFloat() * 0.45f) }
    }
    Box(
        modifier
            .fillMaxWidth()
            .clip(CurvedBottomShape(curve))
            .background(Brush.linearGradient(c.topBarGradient))
            .drawBehind {
                drawOval(
                    color = Color.White.copy(alpha = 0.09f),
                    topLeft = Offset(size.width - 250.dp.toPx(), size.height - 100.dp.toPx()),
                    size = Size(340.dp.toPx(), 250.dp.toPx()),
                )
                for (s in stars) {
                    drawCircle(
                        color = Color.White.copy(alpha = s[3]),
                        radius = s[2].dp.toPx() / 2f,
                        center = Offset(s[0] * size.width, s[1] * size.height),
                    )
                }
            },
        content = content,
    )
}

/** Logotipo parado (fora do ecra inicial o logo nao roda). */
@Composable
fun BrandLogo(sizeDp: Int) {
    Image(painterResource(R.drawable.sealer_logo), contentDescription = null, modifier = Modifier.size(sizeDp.dp))
}

/**
 * A minha foto (ou um boneco, sem foto) num circulo com aro, e um emblema redondo no canto
 * (engrenagem para as Definicoes, lapis para a foto). Tocar em qualquer dos dois faz o mesmo.
 */
@Composable
fun MyAvatar(ui: UiState, badge: ImageVector, label: String, onClick: () -> Unit, sizeDp: Int = 76) {
    val c = LocalChatColors.current
    Box(Modifier.size((sizeDp + 10).dp)) {
        Box(
            Modifier.size(sizeDp.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.22f)).padding(4.dp)
                .clip(CircleShape).background(c.headerAvatar).clickable(onClickLabel = label, onClick = onClick),
            contentAlignment = Alignment.Center,
        ) {
            val mine = ui.myPhoto
            if (mine != null) {
                Image(mine, contentDescription = s(R.string.t_a_tua_foto), modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
            } else {
                Icon(Icons.Filled.Person, contentDescription = s(R.string.t_a_tua_foto), tint = c.headerAvatarInk, modifier = Modifier.size((sizeDp * 0.55f).dp))
            }
        }
        IconButton(onClick = onClick, modifier = Modifier.align(Alignment.BottomEnd).offset(x = 6.dp, y = 6.dp).size(44.dp)) {
            Box(
                Modifier.size(32.dp).shadow(4.dp, CircleShape).clip(CircleShape).background(MaterialTheme.colorScheme.surface),
                contentAlignment = Alignment.Center,
            ) {
                Icon(badge, contentDescription = label, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(18.dp))
            }
        }
    }
}

/** O meu ID abreviado numa pastilha (tocar mostra o QR e o ID completo). */
@Composable
fun IdChip(publicId: String, onClick: () -> Unit) {
    val ink = LocalChatColors.current.onTopBar.copy(alpha = 0.92f)
    Row(
        Modifier.clip(RoundedCornerShape(50)).background(Color.White.copy(alpha = 0.14f))
            .clickable(onClick = onClick).heightIn(min = 34.dp).padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Icon(Icons.Filled.Lock, contentDescription = null, tint = ink, modifier = Modifier.size(14.dp))
        Text(shortId(publicId), color = ink, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
    }
}

/**
 * Cabecalho do ecra inicial: o logo a rodar como um piao (so aqui), "Sealer", a minha foto
 * com a engrenagem das Definicoes, o titulo e o meu ID. Sem estado da ligacao (decisao do Nuno).
 */
@Composable
fun HomeHeader(ui: UiState, onSettings: () -> Unit, onMyId: () -> Unit) {
    val c = LocalChatColors.current
    GalaxyHeader(curve = 76.dp) {
        Column(
            Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars)
                .padding(start = 20.dp, end = 20.dp, top = 12.dp, bottom = 34.dp),
        ) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
                Row(Modifier.weight(1f).padding(top = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                    BrandMark(46)
                    Spacer(Modifier.width(10.dp))
                    Text("Sealer", fontFamily = ManropeBold, fontWeight = FontWeight.Bold, fontSize = 27.sp, color = c.onTopBar)
                }
                MyAvatar(ui, Icons.Filled.Settings, s(R.string.t_definicoes), onSettings)
            }
            Spacer(Modifier.height(10.dp))
            Text(s(R.string.t_conversas), color = c.onTopBar, fontWeight = FontWeight.ExtraBold, fontSize = 29.sp)
            Spacer(Modifier.height(8.dp))
            IdChip(ui.publicId, onMyId)
        }
    }
}

// ====================================================================== definicoes

@Composable
private fun SectionLabel(text: String) {
    Text(
        text.uppercase(), Modifier.padding(start = 20.dp, end = 20.dp, top = 18.dp, bottom = 6.dp),
        style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.Bold, letterSpacing = 1.sp,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** Quadrado arredondado com o icone (ou duas letras, quando nao ha icone). */
@Composable
private fun IconTile(icon: ImageVector?, glyph: String?, danger: Boolean) {
    val c = LocalChatColors.current
    val ink = if (danger) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary
    Box(
        Modifier.size(44.dp).clip(RoundedCornerShape(14.dp)).background(if (danger) MaterialTheme.colorScheme.errorContainer else c.tile),
        contentAlignment = Alignment.Center,
    ) {
        if (icon != null) Icon(icon, contentDescription = null, tint = ink, modifier = Modifier.size(22.dp))
        else Text(glyph ?: "", color = ink, fontWeight = FontWeight.ExtraBold, fontSize = 15.sp)
    }
}

@Composable
private fun SettingsRow(
    icon: ImageVector?, title: String, onClick: (() -> Unit)?,
    glyph: String? = null, danger: Boolean = false, divider: Boolean = true,
    trailing: @Composable (() -> Unit)? = null,
) {
    val base = Modifier.fillMaxWidth().heightIn(min = 68.dp)
    Row(
        (if (onClick != null) base.clickable(onClick = onClick) else base).padding(start = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconTile(icon, glyph, danger)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Row(Modifier.heightIn(min = 67.dp).padding(end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    title, Modifier.weight(1f), fontSize = 16.sp, fontWeight = FontWeight.Bold,
                    color = if (danger) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                )
                when {
                    trailing != null -> trailing()
                    onClick != null -> Icon(
                        Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            if (divider) HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
        }
    }
}

@Composable
private fun SettingsSwitch(icon: ImageVector, title: String, checked: Boolean, onChange: (Boolean) -> Unit, divider: Boolean = true) {
    SettingsRow(icon, title, onClick = { onChange(!checked) }, divider = divider) {
        Switch(checked = checked, onCheckedChange = onChange)
    }
}

/**
 * Ecra Definicoes (0.9.11): tudo o que estava no menu de tres pontos do ecra inicial, mais a
 * versao instalada. A foto muda-se no lapis do cabecalho ou em "Foto de perfil".
 */
@Composable
fun ProfileSettingsScreen(
    ui: UiState,
    onBack: () -> Unit,
    onProfilePhoto: () -> Unit,
    onMyId: () -> Unit,
    onLockSettings: () -> Unit,
    onLockNow: () -> Unit,
    onUnblock: (String) -> Unit,
    onPresence: (Boolean) -> Unit,
    onReadReceipts: (Boolean) -> Unit,
    onPush: (Boolean) -> Unit,
    onLanguage: () -> Unit,
    onRelay: () -> Unit,
    onWipeAll: () -> Unit,
) {
    val c = LocalChatColors.current
    var showBlocked by remember { mutableStateOf(false) }
    var showWipe by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        GalaxyHeader(curve = 76.dp) {
            Column(
                Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars)
                    .padding(start = 4.dp, end = 20.dp, top = 8.dp, bottom = 34.dp),
            ) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
                    Row(Modifier.weight(1f).padding(top = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = s(R.string.t_voltar), tint = c.onTopBar)
                        }
                        Text(s(R.string.t_definicoes), color = c.onTopBar, fontWeight = FontWeight.ExtraBold, fontSize = 24.sp)
                    }
                    MyAvatar(ui, Icons.Filled.Edit, s(R.string.t_foto_de_perfil), onProfilePhoto)
                }
                Spacer(Modifier.height(12.dp))
                Box(Modifier.padding(start = 16.dp)) { IdChip(ui.publicId, onMyId) }
            }
        }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).navigationBarsPadding()) {
            SectionLabel(s(R.string.t_sec_conta))
            SettingsRow(Icons.Filled.Face, s(R.string.t_foto_de_perfil), onClick = onProfilePhoto)
            SettingsRow(Icons.Filled.AccountBox, s(R.string.t_o_meu_id), onClick = onMyId)
            SettingsRow(Icons.Filled.Lock, s(R.string.t_bloqueio_da_app), onClick = onLockSettings, divider = ui.lockEnabled || ui.blocked.isNotEmpty())
            if (ui.lockEnabled) {
                SettingsRow(Icons.Filled.Lock, s(R.string.t_bloquear_agora), onClick = onLockNow, divider = ui.blocked.isNotEmpty())
            }
            if (ui.blocked.isNotEmpty()) {
                SettingsRow(Icons.Filled.Close, s(R.string.t_contactos_bloqueados), onClick = { showBlocked = true }, divider = false)
            }

            SectionLabel(s(R.string.t_sec_privacidade))
            SettingsSwitch(Icons.Filled.Person, s(R.string.t_mostrar_quando_estou_online), ui.presenceOn, onPresence)
            SettingsSwitch(Icons.Filled.Done, s(R.string.t_confirmacoes_de_leitura), ui.readReceipts, onReadReceipts, divider = false)

            SectionLabel(s(R.string.t_sec_app))
            if (Push.available) SettingsSwitch(Icons.Filled.Notifications, s(R.string.t_notificacoes), ui.pushOn, onPush)
            SettingsRow(null, s(R.string.lang_menu), onClick = onLanguage, glyph = "Aa")
            if (BuildConfig.RELAY_EDITABLE) SettingsRow(Icons.Filled.Build, s(R.string.t_relay_teste), onClick = onRelay)
            // Versao instalada (build.gradle.kts, versionName), so informativa.
            SettingsRow(Icons.Filled.Info, s(R.string.t_versao, BuildConfig.VERSION_NAME), onClick = null, divider = false)

            Spacer(Modifier.height(10.dp))
            HorizontalDivider(Modifier.padding(horizontal = 20.dp), thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
            Spacer(Modifier.height(6.dp))
            SettingsRow(Icons.Filled.Delete, s(R.string.t_destruir_perfil_e_dados), onClick = { showWipe = true }, danger = true, divider = false)
            Spacer(Modifier.height(24.dp))
        }
    }
    if (showBlocked) {
        BlockedContactsDialog(
            ui.blocked.map { cid -> cid to (ui.contacts.firstOrNull { it.contactId == cid }?.nickname ?: s(R.string.t_desconhecido_2, cid.take(8))) },
            onUnblock = onUnblock,
            onDismiss = { showBlocked = false },
        )
    }
    if (showWipe) WipeAllDialog(onDismiss = { showWipe = false }, onConfirm = { showWipe = false; onWipeAll() })
}

// ================================================================== novo contacto

/** Icone "ler codigo": quatro cantos e uma linha ao meio. */
@Composable
private fun ScanGlyph(color: Color) {
    Canvas(Modifier.size(22.dp)) {
        val w = size.width
        val k = w * 0.32f
        val st = Stroke(width = 2.2.dp.toPx(), cap = StrokeCap.Round)
        val p = Path().apply {
            moveTo(0f, k); lineTo(0f, 0f); lineTo(k, 0f)
            moveTo(w - k, 0f); lineTo(w, 0f); lineTo(w, k)
            moveTo(w, w - k); lineTo(w, w); lineTo(w - k, w)
            moveTo(k, w); lineTo(0f, w); lineTo(0f, w - k)
            moveTo(w * 0.22f, w / 2); lineTo(w * 0.78f, w / 2)
        }
        drawPath(p, color, style = st)
    }
}

/**
 * Ecra Novo contacto (0.9.11, em vez da janela): o meu codigo QR para me adicionarem, ler o
 * QR da outra pessoa ou escrever o ID dela, e criar um grupo.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AddContactScreen(
    ui: UiState,
    onBack: () -> Unit,
    onAdd: (String, String) -> Unit,
    onCreateGroup: (String, List<Contact>) -> Unit,
) {
    val clipboard = LocalClipboardManager.current
    val context = LocalContext.current
    // rememberSaveable: o leitor de QR e outro ecra; se o telemovel rodar enquanto
    // esta aberto, a app e recriada e o que ja estava escrito nao se perde.
    var nick by rememberSaveable { mutableStateOf("") }
    var pid by rememberSaveable { mutableStateOf("") }
    var scanError by rememberSaveable { mutableStateOf<String?>(null) }
    var showNewGroup by remember { mutableStateOf(false) }
    // Leitor de QR (ZXing, no telemovel, sem servicos da Google nem rede).
    val scanner = rememberLauncherForActivityResult(ScanContract()) { result ->
        val id = idFromQr(result.contents)
        when {
            id != null -> { pid = id; scanError = null }
            result.contents != null -> scanError = s(R.string.t_este_codigo_qr_nao_e_um)
        }
    }
    Scaffold(topBar = { SealerTopBar(title = { Text(s(R.string.t_novo_contacto)) }, onBack = onBack) }) { pad ->
        Column(
            Modifier.padding(pad).consumeWindowInsets(pad).fillMaxSize().imePadding()
                .verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            // O meu codigo, para a outra pessoa me adicionar.
            Column(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(24.dp)).background(MaterialTheme.colorScheme.surfaceContainerLow).padding(18.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(
                    s(R.string.t_mostra_este_codigo), style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (ui.publicId.isNotBlank()) {
                    // Em maiusculas o QR usa o modo alfanumerico: menos quadrados, as camaras leem melhor.
                    val qr = remember(ui.publicId) { qrCode(ui.publicId.uppercase()) }
                    Image(
                        bitmap = qr,
                        contentDescription = s(R.string.t_codigo_qr_do_teu_id),
                        modifier = Modifier.size(200.dp).clip(RoundedCornerShape(18.dp)).background(Color.White).padding(8.dp),
                    )
                }
                Text(shortId(ui.publicId), fontFamily = FontFamily.Monospace, fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilledTonalButton(onClick = {
                        val send = Intent(Intent.ACTION_SEND).apply {
                            type = "text/plain"
                            putExtra(Intent.EXTRA_TEXT, ui.publicId)
                        }
                        context.startActivity(Intent.createChooser(send, s(R.string.t_partilhar_o_meu_id)))
                    }) {
                        Icon(Icons.Filled.Share, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(8.dp))
                        Text(s(R.string.t_partilhar))
                    }
                    FilledTonalButton(onClick = { clipboard.setText(AnnotatedString(ui.publicId)) }) { Text(s(R.string.t_copiar)) }
                }
            }

            Button(
                onClick = {
                    AutoLockPause.active = true
                    scanner.launch(
                        ScanOptions()
                            .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                            .setPrompt(s(R.string.t_aponta_para_o_codigo_qr_da))
                            .setBeepEnabled(false)
                            .setOrientationLocked(false)
                            .addExtra(
                                Intents.Scan.MISSING_CAMERA_PERMISSION_DIALOG_MESSAGE,
                                s(R.string.t_sem_acesso_a_camara_para_ler),
                            ),
                    )
                },
                modifier = Modifier.fillMaxWidth().height(56.dp),
                shape = RoundedCornerShape(18.dp),
            ) {
                ScanGlyph(MaterialTheme.colorScheme.onPrimary)
                Spacer(Modifier.width(10.dp))
                Text(s(R.string.t_ler_codigo_qr), fontSize = 16.sp, fontWeight = FontWeight.Bold)
            }
            scanError?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }

            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                HorizontalDivider(Modifier.weight(1f), thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
                Text(s(R.string.t_ou), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                HorizontalDivider(Modifier.weight(1f), thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
            }

            OutlinedTextField(pid, { pid = it }, label = { Text(s(R.string.t_id_qt1)) }, modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp))
            OutlinedTextField(nick, { nick = it }, label = { Text(s(R.string.t_nome)) }, singleLine = true, modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp))
            OutlinedButton(
                onClick = { onAdd(nick, pid) }, enabled = pid.isNotBlank(),
                modifier = Modifier.fillMaxWidth().height(52.dp), shape = RoundedCornerShape(16.dp),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.primary),
            ) { Text(s(R.string.t_adicionar), fontWeight = FontWeight.Bold) }
            Text(
                s(R.string.t_recebe_o_id_diretamente_da_pessoa),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
            OutlinedButton(onClick = { showNewGroup = true }, modifier = Modifier.fillMaxWidth().height(52.dp), shape = RoundedCornerShape(16.dp)) {
                Icon(Icons.Filled.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(s(R.string.t_novo_grupo), maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Spacer(Modifier.height(8.dp))
        }
    }
    if (showNewGroup) {
        NewGroupDialog(ui, onDismiss = { showNewGroup = false }, onCreate = { n, members -> showNewGroup = false; onCreateGroup(n, members) })
    }
}
