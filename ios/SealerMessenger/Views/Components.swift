import SwiftUI

/// Logotipo SEALER (bolha de vidro com o S e os tres pontos), de Assets.xcassets/Logo,
/// sempre a rodar devagar como um piao, a volta do eixo vertical (uma volta a cada 6 s),
/// como na app Android. Tem espessura: varias camadas, umas atras das outras (as das
/// faces sao o logo, as do meio a silhueta lilas), para de lado nao parecer uma folha
/// de papel. Parado se o utilizador tiver "Reduzir movimento" ativo.
struct BrandMark: View {
    let size: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            if reduceMotion {
                SpinningLogo(size: size, angle: 0)
            } else {
                TimelineView(.animation) { context in
                    let seconds = context.date.timeIntervalSinceReferenceDate
                    SpinningLogo(size: size, angle: seconds.truncatingRemainder(dividingBy: 6) / 6 * 360)
                }
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

private struct SpinningLogo: View {
    let size: CGFloat
    let angle: Double

    var body: some View {
        let radians = angle * .pi / 180
        let thickness = Double(size) * 0.10
        let layers = 24
        let frontVisible = cos(radians) >= 0
        let shift = thickness * sin(radians)
        ZStack {
            ForEach(0..<layers, id: \.self) { k in
                let i = frontVisible ? k : layers - 1 - k
                let depth = Double(i) / Double(layers - 1) - 0.5
                layer(index: i, count: layers, depth: depth)
                    .rotation3DEffect(.degrees(angle), axis: (x: 0, y: 1, z: 0), perspective: 0.3)
                    .offset(x: CGFloat(depth * shift))
            }
        }
    }

    @ViewBuilder
    private func layer(index: Int, count: Int, depth: Double) -> some View {
        if index == 0 || index == count - 1 {
            Image("Logo").resizable().scaledToFit()
        } else {
            // Lilas claro junto as faces, mais escuro ao centro.
            let middle = 1 - abs(2 * depth)
            Image("Logo").renderingMode(.template).resizable().scaledToFit()
                .foregroundStyle(Color(red: 0.84 - 0.23 * middle, green: 0.81 - 0.25 * middle, blue: 0.94 - 0.15 * middle))
        }
    }
}

/// Marca na barra de topo: logo pequeno (parado) e "Sealer" em Manrope Bold
/// (SIL Open Font License 1.1, ver Resources/Manrope-OFL.txt), igual ao Android.
struct SealerWordmark: View {
    var body: some View {
        HStack(spacing: 10) {
            Image("Logo").resizable().scaledToFit().frame(width: 30, height: 30)
            Text("Sealer").font(.custom("Manrope-Bold", size: 22))
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Sealer")
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
        .background(GalaxyBackground(colors: c.topBarGradient).ignoresSafeArea(edges: .top))
    }
}

/// Fundo "galaxia" da barra de topo: gradiente diagonal com estrelas pequenas,
/// sempre nas mesmas posicoes (igual a app Android).
struct GalaxyBackground: View {
    let colors: [Color]
    private static let stars: [(x: Double, y: Double, size: Double, alpha: Double)] = {
        var seed: UInt64 = 7
        func next() -> Double {   // gerador simples e determinista
            seed = seed &* 6364136223846793005 &+ 1442695040888963407
            return Double(seed >> 11) / Double(1 << 53)
        }
        return (0..<28).map { _ in (next(), next(), 0.6 + next() * 0.8, 0.25 + next() * 0.45) }
    }()

    var body: some View {
        LinearGradient(colors: colors, startPoint: .topLeading, endPoint: .bottomTrailing)
            .overlay(
                Canvas { context, size in
                    for s in GalaxyBackground.stars {
                        let rect = CGRect(x: s.x * size.width, y: s.y * size.height, width: s.size, height: s.size)
                        context.fill(Path(ellipseIn: rect), with: .color(.white.opacity(s.alpha)))
                    }
                }
            )
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
