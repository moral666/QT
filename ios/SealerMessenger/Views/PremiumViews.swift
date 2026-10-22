import SwiftUI

/// "Sealer Premium" (0.9.20), igual ao PremiumScreen do Android: o que desbloqueia e o preco.
/// Quem tem premium e se o bloqueio esta ligado decide o operador no relay (sealer-control, 0.9.21).
/// Antes do Play Billing/App Store, o botao de subscrever esta desligado.
struct PremiumScreen: View {
    let ui: UiState
    let onBack: () -> Void
    @Environment(\.chat) private var c

    private static let foreverMs: Int64 = 253_402_300_799_000

    private func perk(_ icon: String, _ text: String) -> some View {
        HStack(spacing: 12) {
            Image(systemName: icon).foregroundStyle(c.primary).frame(width: 24)
            Text(text).font(.sBodyMedium)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(onBack: onBack) { Text(s("premium_title")) }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(s("premium_price")).font(Jakarta.font(28, .heavy))
                        Text(ui.premium ? s("premium_active") : s("premium_free_text")).font(.sBodyMedium)
                        if ui.premiumUntil > Int64(Date().timeIntervalSince1970 * 1000) {
                            Text(ui.premiumUntil >= Self.foreverMs ? s("premium_until_forever")
                                 : s("premium_until", Date(timeIntervalSince1970: Double(ui.premiumUntil) / 1000)
                                        .formatted(date: .abbreviated, time: .omitted)))
                                .font(.sLabelLarge)
                        }
                    }
                    .padding(20)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 20).fill(c.tile))
                    Text(s("premium_unlocks")).font(.sTitleMedium)
                    perk("phone.fill", s("premium_perk_calls"))
                    perk("person.3.fill", s("premium_perk_groups"))
                    perk("photo.fill", s("premium_perk_media"))
                    perk("checkmark", s("premium_perk_receive"))
                    PrimaryButton(title: ui.premium ? s("premium_active_short") : s("premium_soon")) {}
                        .disabled(true)
                    if !ui.premiumLock && !ui.premium {
                        Text(s("premium_all_open")).font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
                    }
                }
                .padding(20)
            }
        }
        .background(c.background.ignoresSafeArea())
    }
}
