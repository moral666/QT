import SwiftUI
import UIKit

/// Um membro da Comunidade pronto a mostrar: so a medalha e a foto (sem nome nem ID).
struct CommunityMemberUi: Identifiable {
    /// A posicao na lista (a ordem e a da lista: medalha e, dentro de cada uma, entrada).
    let id: Int
    let badge: Int
    let photo: UIImage?
}

/// Comunidade (0.9.52), igual ao CommunityScreen do Android: as pessoas com medalha, so com a foto
/// e a medalha, por medalha e, dentro de cada uma, por ordem de entrada. A lista e assinada e
/// cifrada (`qt_core::community`); so desce quando este ecra abre, e so o que mudou (AppModel).
struct CommunityScreen: View {
    let ui: UiState
    let onBack: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(onBack: onBack) { Text(s("community_title")) }
            Text(s("community_subtitle")).font(.sBodyMedium).foregroundStyle(c.onSurfaceVariant)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 20).padding(.top, 12).padding(.bottom, 4)
            if ui.community.isEmpty && ui.communityLoading {
                Spacer()
                ProgressView()
                Spacer()
            } else if ui.community.isEmpty {
                Spacer()
                Text(s("community_empty")).font(.sBodyLarge).foregroundStyle(c.onSurfaceVariant)
                    .multilineTextAlignment(.center).padding(32)
                Spacer()
            } else {
                ScrollView {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 84), spacing: 12)], spacing: 18) {
                        ForEach(ui.community) { m in
                            CommunityAvatar(member: m, size: 72).frame(maxWidth: .infinity)
                        }
                    }
                    .padding(16)
                }
            }
        }
    }
}

/// Como o Avatar dos contactos (aro e medalha no canto, ou o escudo da Legacy), sem nome. Sem
/// foto: o avatar por omissao, com a medalha ao centro. O VoiceOver diz a medalha.
struct CommunityAvatar: View {
    let member: CommunityMemberUi
    let size: CGFloat
    @Environment(\.chat) private var c
    @Environment(\.layoutDirection) private var direction

    var body: some View {
        let badge = member.badge
        let ringed = Badges.colors(badge) != nil
        let shield = badge == Badges.legacy
        let inner = ringed ? size * Badges.inner : shield ? size * Badges.shieldPhoto : size
        ZStack(alignment: .bottomTrailing) {
            ZStack {
                if let photo = member.photo {
                    Image(uiImage: photo).resizable().scaledToFill().frame(width: inner, height: inner).clipShape(Circle())
                } else {
                    ZStack {
                        Circle().fill(c.tile)
                        // A medalha ao centro (na Legacy, que nao tem moeda, fica so o escudo).
                        if !shield { BadgeCoin(badge: badge, size: size * 0.42).opacity(0.9) }
                    }
                    .frame(width: inner, height: inner)
                }
                if ringed { BadgeRing(badge: badge, size: size) }
            }
            .frame(width: size, height: size)
            .overlay { if shield { BadgeShield(size: size) } }
            if ringed && member.photo != nil {
                BadgeCoin(badge: badge, size: size * Badges.coinScale)
                    .offset(x: (direction == .rightToLeft ? -1 : 1) * size * 0.06, y: size * 0.04)
            }
        }
        .frame(width: size, height: size)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Badges.name(badge) ?? "")
    }
}
