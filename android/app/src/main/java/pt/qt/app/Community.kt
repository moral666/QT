package pt.qt.app

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

/**
 * Comunidade (0.9.52): as pessoas com medalha, so com a foto e a medalha (sem nome nem ID), por
 * medalha e, dentro de cada uma, por ordem de entrada. A lista e assinada e cifrada (ver
 * `qt_core::community`); so desce quando este ecra abre, e so o que mudou (MainViewModel).
 */
data class CommunityMemberUi(val badge: Int, val photo: ImageBitmap?)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CommunityScreen(ui: UiState, onBack: () -> Unit) {
    Scaffold(topBar = { SealerTopBar(title = { Text(s(R.string.community_title)) }, onBack = onBack) }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            Text(
                s(R.string.community_subtitle),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 20.dp, end = 20.dp, top = 12.dp, bottom = 4.dp),
            )
            when {
                ui.community.isEmpty() && ui.communityLoading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
                ui.community.isEmpty() -> Box(Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) {
                    Text(
                        s(R.string.community_empty), textAlign = TextAlign.Center,
                        style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                else -> LazyVerticalGrid(
                    columns = GridCells.Adaptive(84.dp),
                    contentPadding = PaddingValues(16.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalArrangement = Arrangement.spacedBy(18.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    itemsIndexed(ui.community, key = { i, _ -> i }) { _, m ->
                        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { CommunityAvatar(m, 72) }
                    }
                }
            }
        }
    }
}

/**
 * Como o Avatar dos contactos (aro e medalha no canto, ou o escudo da Legacy), sem nome. Sem
 * foto: o avatar por omissao, com a medalha ao centro. O leitor de ecra diz a medalha.
 */
@Composable
fun CommunityAvatar(m: CommunityMemberUi, sizeDp: Int) {
    val ringed = Badges.colors(m.badge) != null
    val shield = m.badge == Badges.LEGACY
    val inner = when {
        shield -> (sizeDp * Badges.SHIELD_PHOTO).dp
        ringed -> (sizeDp * Badges.INNER).dp
        else -> sizeDp.dp
    }
    val label = Badges.name(m.badge) ?: ""
    Box(Modifier.size(sizeDp.dp).semantics { contentDescription = label }) {
        if (m.photo != null) {
            Image(m.photo, contentDescription = null, modifier = Modifier.size(inner).align(Alignment.Center).clip(CircleShape), contentScale = ContentScale.Crop)
        } else {
            Box(
                Modifier.size(inner).align(Alignment.Center).clip(CircleShape).background(MaterialTheme.colorScheme.surfaceVariant),
                contentAlignment = Alignment.Center,
            ) {
                // A medalha ao centro (na Legacy, que nao tem moeda, fica so o escudo a volta).
                if (!shield) BadgeCoin(m.badge, Modifier.size((sizeDp * 0.42f).dp).alpha(0.9f))
            }
        }
        if (ringed) {
            BadgeRing(m.badge, Modifier.matchParentSize())
            if (m.photo != null) {
                BadgeCoin(m.badge, Modifier.align(Alignment.BottomEnd).offset(x = (sizeDp * 0.06f).dp, y = (sizeDp * 0.04f).dp).size((sizeDp * Badges.COIN).dp))
            }
        }
        if (shield) BadgeShield(sizeDp, Modifier.align(Alignment.Center))
    }
}
