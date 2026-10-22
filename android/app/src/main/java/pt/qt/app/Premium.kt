package pt.qt.app

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Star
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Premium "sem fim" dado pelo operador (31/12/9999). */
private const val FOREVER_MS = 253_402_300_799_000L

@Composable
private fun Perk(icon: ImageVector, text: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.width(12.dp))
        Text(text, style = MaterialTheme.typography.bodyLarge)
    }
}

/**
 * "Sealer Premium" (0.9.20): o que o premium desbloqueia e o preco. Abre a partir de um cadeado
 * (acao paga sem premium) ou das Definicoes. Quem tem premium e se o bloqueio esta ligado decide o
 * operador no relay (0.9.21). Antes do Play Billing, o botao de subscrever esta desligado.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PremiumScreen(ui: UiState, onBack: () -> Unit) {
    Scaffold(topBar = { SealerTopBar(title = { Text(s(R.string.premium_title)) }, onBack = onBack) }) { pad ->
        Column(
            Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.primaryContainer) {
                Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(s(R.string.premium_price), fontSize = 28.sp, fontWeight = FontWeight.ExtraBold)
                    Text(
                        if (ui.premium) s(R.string.premium_active) else s(R.string.premium_free_text),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    if (ui.premiumUntil > System.currentTimeMillis()) {
                        val forever = ui.premiumUntil >= FOREVER_MS
                        Text(
                            if (forever) s(R.string.premium_until_forever)
                            else s(R.string.premium_until, java.text.DateFormat.getDateInstance().format(java.util.Date(ui.premiumUntil))),
                            style = MaterialTheme.typography.labelLarge,
                        )
                    }
                }
            }
            Text(s(R.string.premium_unlocks), style = MaterialTheme.typography.titleMedium)
            Perk(Icons.Filled.Call, s(R.string.premium_perk_calls))
            Perk(Icons.Filled.Person, s(R.string.premium_perk_groups))
            Perk(Icons.Filled.Star, s(R.string.premium_perk_media))
            Perk(Icons.Filled.Check, s(R.string.premium_perk_receive))
            Button(onClick = {}, enabled = false, modifier = Modifier.fillMaxWidth()) {
                Text(if (ui.premium) s(R.string.premium_active_short) else s(R.string.premium_soon))
            }
            if (!ui.premiumLock && !ui.premium) {
                Text(s(R.string.premium_all_open), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
