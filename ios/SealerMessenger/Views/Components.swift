import SwiftUI

/// Logotipo simples: cadeado branco num circulo roxo.
struct BrandMark: View {
    let size: CGFloat
    @Environment(\.chat) private var c
    var body: some View {
        ZStack {
            Circle().fill(c.primary)
            Image(systemName: "lock.fill")
                .resizable().scaledToFit()
                .frame(width: size / 2.6, height: size / 2.6)
                .foregroundStyle(c.onPrimary)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// Barra de topo roxa (como a TopAppBar do Android), com voltar opcional e acoes.
struct SealerTopBar<Title: View, Actions: View>: View {
    var onBack: (() -> Void)?
    @ViewBuilder var title: () -> Title
    @ViewBuilder var actions: () -> Actions
    @Environment(\.chat) private var c

    init(onBack: (() -> Void)? = nil,
         @ViewBuilder title: @escaping () -> Title,
         @ViewBuilder actions: @escaping () -> Actions) {
        self.onBack = onBack
        self.title = title
        self.actions = actions
    }

    var body: some View {
        HStack(spacing: 4) {
            if let onBack {
                Button(action: onBack) {
                    Image(systemName: "arrow.left").font(.system(size: 20, weight: .medium))
                        .frame(width: 44, height: 44)
                }
                .accessibilityLabel("Voltar")
            } else {
                Spacer().frame(width: 12)
            }
            title().frame(maxWidth: .infinity, alignment: .leading)
            actions()
        }
        .padding(.horizontal, 4)
        .frame(height: 56)
        .foregroundStyle(c.onTopBar)
        .tint(c.onTopBar)
        .background(c.topBar.ignoresSafeArea(edges: .top))
    }
}

extension SealerTopBar where Actions == EmptyView {
    init(onBack: (() -> Void)? = nil, @ViewBuilder title: @escaping () -> Title) {
        self.init(onBack: onBack, title: title, actions: { EmptyView() })
    }
}

struct TopBarIcon: View {
    let systemName: String
    let label: String
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Image(systemName: systemName).font(.system(size: 19, weight: .medium)).frame(width: 44, height: 44)
        }
        .accessibilityLabel(label)
    }
}

/// Avatar com as iniciais, numa cor estavel por contacto (mesma cor que no Android).
struct Avatar: View {
    let name: String
    let seed: String
    var size: CGFloat = 48

    var body: some View {
        let palette = ChatColors.avatarPalette
        let color = palette[Int(javaHashCode(seed) & 0x7fffffff) % palette.count]
        ZStack {
            Circle().fill(color)
            Text(initials(name)).font(.system(size: size / 2.6, weight: .semibold)).foregroundStyle(.white)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// O mesmo `String.hashCode()` do Java/Kotlin, para a cor do avatar ser igual nas duas apps.
func javaHashCode(_ s: String) -> Int32 {
    var h: Int32 = 0
    for unit in s.utf16 { h = h &* 31 &+ Int32(unit) }
    return h
}

func initials(_ name: String) -> String {
    let parts = name.trimmingCharacters(in: .whitespaces).split(separator: " ").filter { !$0.isEmpty }.prefix(2)
    let s = parts.map { String($0.prefix(1)).uppercased() }.joined()
    return s.isEmpty ? "?" : s
}

struct ConnectionBanner: View {
    let status: String
    @Environment(\.chat) private var c
    var body: some View {
        Text(status)
            .font(.caption.weight(.medium))
            .lineLimit(1).truncationMode(.tail)
            .foregroundStyle(c.onSecondaryContainer)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16).padding(.vertical, 6)
            .background(c.secondaryContainer)
    }
}

/// Botao principal arredondado (como o Button do Material 3).
struct PrimaryButton: View {
    let title: String
    var height: CGFloat = 52
    let action: () -> Void
    @Environment(\.chat) private var c
    @Environment(\.isEnabled) private var enabled
    var body: some View {
        Button(action: action) {
            Text(title).font(.system(size: 16, weight: .medium))
                .frame(maxWidth: .infinity).frame(height: height)
                .foregroundStyle(c.onPrimary)
                .background(Capsule().fill(c.primary.opacity(enabled ? 1 : 0.4)))
        }
        .buttonStyle(.plain)
    }
}

struct OutlinedField: View {
    let label: String
    @Binding var text: String
    var monospaced = false
    @Environment(\.chat) private var c
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label).font(.caption).foregroundStyle(c.onSurfaceVariant)
            TextField("", text: $text, axis: .vertical)
                .lineLimit(1...4)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(monospaced ? .system(.footnote, design: .monospaced) : .body)
                .padding(12)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(c.onSurfaceVariant.opacity(0.6), lineWidth: 1))
        }
    }
}

// ===================================================================== utilitarios

/// 🕓 por enviar · ✓ enviado (relay) · ✓✓ entregue · ✓✓ azul visto.
func statusGlyph(_ s: MessageStatus) -> String {
    switch s {
    case .pending: return "🕓"
    case .sent: return "✓"
    case .delivered, .read: return "✓✓"
    case .received, .readByMe: return ""
    }
}

private func formatter(_ pattern: String, locale: Locale = .current) -> DateFormatter {
    let f = DateFormatter()
    f.locale = locale
    f.dateFormat = pattern
    return f
}

private let timeFmt = formatter("HH:mm")
private let dateFmt = formatter("dd/MM/yyyy")
private let shortDateFmt = formatter("dd/MM")
private let dayFmt = formatter("yyyyMMdd", locale: Locale(identifier: "en_US_POSIX"))

private func date(_ ms: Int64) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }

func timeOf(_ ms: Int64) -> String { timeFmt.string(from: date(ms)) }

func dayKey(_ ms: Int64) -> String { dayFmt.string(from: date(ms)) }

/// "Hoje", "Ontem" ou a data.
func dayLabel(_ ms: Int64) -> String {
    let now = Date()
    let today = dayFmt.string(from: now)
    let yesterday = dayFmt.string(from: now.addingTimeInterval(-24 * 3600))
    switch dayKey(ms) {
    case today: return "Hoje"
    case yesterday: return "Ontem"
    default: return dateFmt.string(from: date(ms))
    }
}

/// Hora (hoje), "Ontem" ou a data, para a lista de conversas.
func shortTime(_ ms: Int64) -> String {
    switch dayLabel(ms) {
    case "Hoje": return timeOf(ms)
    case "Ontem": return "Ontem"
    default: return shortDateFmt.string(from: date(ms))
    }
}
