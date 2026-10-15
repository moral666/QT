import SwiftUI

/// Mesmas cores da app Android (Theme.kt): roxo #6750A4, claro e escuro.
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

    static let avatarPalette: [Color] = [
        Color(hex: 0x7E57C2), Color(hex: 0x5C6BC0), Color(hex: 0x26A69A), Color(hex: 0xEF6C00),
        Color(hex: 0xD81B60), Color(hex: 0x00897B), Color(hex: 0x8D6E63), Color(hex: 0x3949AB),
    ]

    static let light = ChatColors(
        primary: Color(hex: 0x6750A4), onPrimary: .white,
        topBar: Color(hex: 0x6750A4), onTopBar: .white,
        background: Color(hex: 0xFFFBFE), surface: Color(hex: 0xFFFBFE),
        surfaceVariant: Color(hex: 0xE7E0EC), onSurface: Color(hex: 0x1C1B1F), onSurfaceVariant: Color(hex: 0x49454F),
        secondaryContainer: Color(hex: 0xE8DEF8), onSecondaryContainer: Color(hex: 0x1D192B),
        outlineVariant: Color(hex: 0xCAC4D0), error: Color(hex: 0xB3261E),
        chatBackground: Color(hex: 0xF1ECF8), outgoingBubble: Color(hex: 0xE3D7FF), incomingBubble: .white,
        bubbleText: Color(hex: 0x1C1B1F), bubbleMeta: Color(hex: 0x6E6A75), readTick: Color(hex: 0x34B7F1)
    )

    static let dark = ChatColors(
        primary: Color(hex: 0xD0BCFF), onPrimary: Color(hex: 0x381E72),
        topBar: Color(hex: 0x2B2534), onTopBar: Color(hex: 0xEDE6F7),
        background: Color(hex: 0x141218), surface: Color(hex: 0x141218),
        surfaceVariant: Color(hex: 0x49454F), onSurface: Color(hex: 0xE6E1E5), onSurfaceVariant: Color(hex: 0xCAC4D0),
        secondaryContainer: Color(hex: 0x4A4458), onSecondaryContainer: Color(hex: 0xE8DEF8),
        outlineVariant: Color(hex: 0x49454F), error: Color(hex: 0xF2B8B5),
        chatBackground: Color(hex: 0x17141C), outgoingBubble: Color(hex: 0x4F378B), incomingBubble: Color(hex: 0x2B2930),
        bubbleText: Color(hex: 0xEDE6F7), bubbleMeta: Color(hex: 0xB9B2C4), readTick: Color(hex: 0x53BDEB)
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
    }
}
