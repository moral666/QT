import SwiftUI

/// Mesmas cores da app Android (Theme.kt): roxo "galatico" #5A2FD8, claro e escuro.
extension Color {
    init(hex: UInt32) {
        self.init(.sRGB,
                  red: Double((hex >> 16) & 0xFF) / 255,
                  green: Double((hex >> 8) & 0xFF) / 255,
                  blue: Double(hex & 0xFF) / 255,
                  opacity: 1)
    }
}

struct ChatColors {
    let primary: Color
    let onPrimary: Color
    let topBar: Color
    /// Fundo da barra de topo: gradiente "galaxia" (do indigo noturno ao violeta), com estrelas.
    let topBarGradient: [Color]
    let onTopBar: Color
    let background: Color
    let surface: Color
    let surfaceVariant: Color
    let onSurface: Color
    let onSurfaceVariant: Color
    let secondaryContainer: Color
    let onSecondaryContainer: Color
    let outlineVariant: Color
    let error: Color
    let chatBackground: Color
    let outgoingBubble: Color
    let incomingBubble: Color
    let bubbleText: Color
    let bubbleMeta: Color
    let readTick: Color
    /// Fundo dos icones das Definicoes (quadrados arredondados).
    let tile: Color
    /// A minha foto no cabecalho, quando nao ha foto: fundo e boneco.
    let headerAvatar: Color
    let headerAvatarInk: Color
    let surfaceContainerLow: Color
    let errorContainer: Color

    static let avatarPalette: [Color] = [
        Color(hex: 0x7E57C2), Color(hex: 0x5C6BC0), Color(hex: 0x26A69A), Color(hex: 0xEF6C00),
        Color(hex: 0xD81B60), Color(hex: 0x00897B), Color(hex: 0x8D6E63), Color(hex: 0x3949AB),
    ]

    static let light = ChatColors(
        primary: Color(hex: 0x5A2FD8), onPrimary: .white,
        topBar: Color(hex: 0x4321B0),
        topBarGradient: [Color(hex: 0x1E1052), Color(hex: 0x4321B0), Color(hex: 0x7038F0)],
        onTopBar: .white,
        background: Color(hex: 0xFCFAFF), surface: Color(hex: 0xFCFAFF),
        surfaceVariant: Color(hex: 0xE7E0EC), onSurface: Color(hex: 0x1C1B1F), onSurfaceVariant: Color(hex: 0x49454F),
        secondaryContainer: Color(hex: 0xE8DEF8), onSecondaryContainer: Color(hex: 0x1D192B),
        outlineVariant: Color(hex: 0xCAC4D0), error: Color(hex: 0xB3261E),
        chatBackground: Color(hex: 0xEFEAFB), outgoingBubble: Color(hex: 0xDCCDFF), incomingBubble: .white,
        bubbleText: Color(hex: 0x1C1B1F), bubbleMeta: Color(hex: 0x6E6A75), readTick: Color(hex: 0x34B7F1),
        tile: Color(hex: 0xF1ECFF), headerAvatar: .white, headerAvatarInk: Color(hex: 0x4321B0),
        surfaceContainerLow: Color(hex: 0xF6F2FC), errorContainer: Color(hex: 0xF9DEDC)
    )

    static let dark = ChatColors(
        primary: Color(hex: 0xB79CFF), onPrimary: Color(hex: 0x2A0C7A),
        topBar: Color(hex: 0x22105E),
        topBarGradient: [Color(hex: 0x0E0828), Color(hex: 0x22105E), Color(hex: 0x3A1C96)],
        onTopBar: Color(hex: 0xF1EAFF),
        background: Color(hex: 0x120E1F), surface: Color(hex: 0x120E1F),
        surfaceVariant: Color(hex: 0x49454F), onSurface: Color(hex: 0xE6E1E5), onSurfaceVariant: Color(hex: 0xCAC4D0),
        secondaryContainer: Color(hex: 0x4A4458), onSecondaryContainer: Color(hex: 0xE8DEF8),
        outlineVariant: Color(hex: 0x49454F), error: Color(hex: 0xF2B8B5),
        chatBackground: Color(hex: 0x0F0B1C), outgoingBubble: Color(hex: 0x4A24B8), incomingBubble: Color(hex: 0x231C36),
        bubbleText: Color(hex: 0xEDE6F7), bubbleMeta: Color(hex: 0xB9B2C4), readTick: Color(hex: 0x53BDEB),
        tile: Color(hex: 0x2A2142), headerAvatar: Color(hex: 0x2A2142), headerAvatarInk: Color(hex: 0xD9CCFF),
        surfaceContainerLow: Color(hex: 0x1C172B), errorContainer: Color(hex: 0x8C1D18)
    )
}

private struct ChatColorsKey: EnvironmentKey {
    static let defaultValue = ChatColors.light
}

extension EnvironmentValues {
    var chat: ChatColors {
        get { self[ChatColorsKey.self] }
        set { self[ChatColorsKey.self] = newValue }
    }
}

/// Cor do ponto "online".
let onlineGreen = Color(hex: 0x2EB872)

/// Letra da app (desde a 0.9.11, igual ao Android): Plus Jakarta Sans (SIL Open Font License 1.1,
/// ver Resources/PlusJakartaSans-OFL.txt). Respeita o tamanho de letra escolhido no iPhone.
enum Jakarta {
    static func font(_ size: CGFloat, _ weight: Font.Weight = .regular, relativeTo style: Font.TextStyle = .body) -> Font {
        let name: String
        switch weight {
        case .medium: name = "PlusJakartaSans-Medium"
        case .semibold: name = "PlusJakartaSans-SemiBold"
        case .bold: name = "PlusJakartaSans-Bold"
        case .heavy, .black: name = "PlusJakartaSans-ExtraBold"
        default: name = "PlusJakartaSans-Regular"
        }
        return .custom(name, size: size, relativeTo: style)
    }
}

extension Font {
    /// Os estilos do Material 3 usados no Android, na letra da app.
    static let sTitleLarge = Jakarta.font(22, .regular, relativeTo: .title2)
    static let sTitleMedium = Jakarta.font(16, .medium, relativeTo: .headline)
    static let sTitleSmall = Jakarta.font(14, .medium, relativeTo: .subheadline)
    static let sBodyLarge = Jakarta.font(16, .regular, relativeTo: .body)
    static let sBodyMedium = Jakarta.font(14, .regular, relativeTo: .callout)
    static let sBodySmall = Jakarta.font(12, .regular, relativeTo: .footnote)
    static let sLabelLarge = Jakarta.font(14, .medium, relativeTo: .subheadline)
    static let sLabelMedium = Jakarta.font(12, .medium, relativeTo: .caption)
    static let sLabelSmall = Jakarta.font(11, .medium, relativeTo: .caption2)
    static let sHeadline = Jakarta.font(28, .regular, relativeTo: .title)
}

/// Equivalente ao `SealerTheme` do Android: escolhe as cores pelo modo claro/escuro do sistema.
struct SealerTheme<Content: View>: View {
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder let content: () -> Content

    var body: some View {
        let c = scheme == .dark ? ChatColors.dark : ChatColors.light
        content()
            .environment(\.chat, c)
            .tint(c.primary)
            .background(c.background.ignoresSafeArea())
            .foregroundStyle(c.onSurface)
            .font(.sBodyLarge)
    }
}
