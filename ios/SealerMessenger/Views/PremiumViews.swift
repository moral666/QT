import SwiftUI

/// "Sealer Premium" (0.9.20), igual ao PremiumScreen do Android: o que desbloqueia e o preco.
/// Antes do Play Billing/App Store, o botao de subscrever esta desligado e ha interruptores de teste.
struct PremiumScreen: View {
    let ui: UiState
    let onBack: () -> Void
    let onTest: (Bool, Bool) -> Void
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
                    Rectangle().fill(c.outlineVariant).frame(height: 0.5)
                    Text(s("premium_test_title")).font(.sLabelLarge)
                    Text(s("premium_test_hint")).font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
                    Toggle(s("premium_test_lock"), isOn: Binding(get: { ui.premiumLock }, set: { onTest($0, ui.premium) }))
                    if Config.relayEditable {   // simular premium: so na build de debug
                        Toggle(s("premium_test_paid"), isOn: Binding(get: { ui.premium }, set: { onTest(ui.premiumLock, $0) }))
                    }
                }
                .padding(20)
            }
        }
        .background(c.background.ignoresSafeArea())
    }
}
