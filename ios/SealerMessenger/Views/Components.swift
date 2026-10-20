import SwiftUI
import UIKit

// ======================================================================== logotipo

/// Logotipo SEALER (bolha de vidro com o S e os tres pontos), de Assets.xcassets/Logo,
/// a rodar devagar como um piao (uma volta a cada 6 s), com espessura. Desde a 0.9.11 so
/// roda no cabecalho do ecra inicial; nos outros ecras fica parado (`BrandLogo`).
/// Parado se o utilizador tiver "Reduzir movimento" ativo.
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
        let layers = max(6, min(24, Int(thickness)))
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
            let middle = 1 - abs(2 * depth)
            Image("Logo").renderingMode(.template).resizable().scaledToFit()
                .foregroundStyle(Color(red: 0.84 - 0.23 * middle, green: 0.81 - 0.25 * middle, blue: 0.94 - 0.15 * middle))
        }
    }
}

/// Logotipo parado (fora do ecra inicial o logo nao roda).
struct BrandLogo: View {
    let size: CGFloat
    var body: some View {
        Image("Logo").resizable().scaledToFit().frame(width: size, height: size).accessibilityHidden(true)
    }
}

// ===================================================================== cabecalhos

/// Forma dos cabecalhos (0.9.11): em baixo, a direita, uma curva larga que comeca a 36% da
/// largura e sobe `curve` ate ao lado direito.
struct CurvedBottomShape: Shape {
    let curve: CGFloat
    var widthFraction: CGFloat = 0.64

    func path(in rect: CGRect) -> Path {
        let ry = min(curve, rect.height)
        let rx = rect.width * widthFraction
        var p = Path()
        p.move(to: CGPoint(x: rect.minX, y: rect.minY))
        p.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        p.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY - ry))
        // Quarto de elipse do lado direito ate ao fundo.
        p.addQuadCurve(to: CGPoint(x: rect.maxX - rx, y: rect.maxY), control: CGPoint(x: rect.maxX, y: rect.maxY))
        p.addLine(to: CGPoint(x: rect.minX, y: rect.maxY))
        p.closeSubpath()
        return p
    }
}

/// Estrelas do fundo dos cabecalhos: sempre nas mesmas posicoes (gerador simples e determinista).
private let galaxyStars: [(x: Double, y: Double, size: Double, alpha: Double)] = {
    var seed: UInt64 = 7
    func next() -> Double {
        seed = seed &* 6364136223846793005 &+ 1442695040888963407
        return Double(seed >> 11) / Double(1 << 53)
    }
    return (0..<28).map { _ in (next(), next(), 0.6 + next() * 0.8, 0.25 + next() * 0.45) }
}()

/// Fundo "galaxia" dos cabecalhos: gradiente da marca, estrelas fixas, brilho na curva.
struct GalaxyHeader<Content: View>: View {
    let curve: CGFloat
    @ViewBuilder let content: () -> Content
    @Environment(\.chat) private var c

    var body: some View {
        content()
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                ZStack {
                    LinearGradient(colors: c.topBarGradient, startPoint: .topLeading, endPoint: .bottomTrailing)
                    Canvas { ctx, size in
                        ctx.fill(Path(ellipseIn: CGRect(x: size.width - 250, y: size.height - 100, width: 340, height: 250)),
                                 with: .color(.white.opacity(0.09)))
                        for st in galaxyStars {
                            let r = CGRect(x: st.x * size.width, y: st.y * size.height, width: st.size, height: st.size)
                            ctx.fill(Path(ellipseIn: r), with: .color(.white.opacity(st.alpha)))
                        }
                    }
                }
                .clipShape(CurvedBottomShape(curve: curve))
                .ignoresSafeArea(edges: .top)
            )
    }
}

/// Barra de topo dos outros ecras (fundo curvo pequeno), com voltar opcional e acoes.
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
        GalaxyHeader(curve: 30) {
            HStack(spacing: 4) {
                if let onBack {
                    Button(action: onBack) {
                        Image(systemName: "chevron.left").font(.system(size: 20, weight: .semibold))
                            .frame(width: 44, height: 44)
                    }
                    .accessibilityLabel(s("t_voltar"))
                } else {
                    Spacer().frame(width: 12)
                }
                title().font(.sTitleMedium).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                actions()
            }
            .padding(.horizontal, 4)
            .frame(height: 56)
            .padding(.bottom, 14)
            .foregroundStyle(c.onTopBar)
            .tint(c.onTopBar)
        }
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
            Image(systemName: systemName).font(.system(size: 18, weight: .medium)).frame(width: 44, height: 44)
        }
        .accessibilityLabel(label)
    }
}

/// A minha foto (ou um boneco, sem foto) num circulo com aro, e um emblema redondo no canto
/// (engrenagem para as Definicoes, lapis para a foto). Tocar em qualquer dos dois faz o mesmo.
struct MyAvatar: View {
    let photo: UIImage?
    /// O meu badge (aro colorido); 0 = o aro claro de sempre.
    var ring: Int = Badges.none
    let badge: String
    let label: String
    let action: () -> Void
    var size: CGFloat = 76
    @Environment(\.chat) private var c

    var body: some View {
        Button(action: action) {
            ZStack(alignment: .bottomTrailing) {
                // Com badge, o aro do badge substitui o aro claro.
                let ringed = Badges.colors(ring) != nil
                let pad = ringed ? size * (1 - Badges.inner) / 2 : 4
                ZStack {
                    if !ringed { Circle().fill(Color.white.opacity(0.22)) }
                    Circle().fill(c.headerAvatar).padding(pad)
                    if let photo {
                        Image(uiImage: photo).resizable().scaledToFill().clipShape(Circle()).padding(pad)
                    } else {
                        Image(systemName: "person.fill").font(.system(size: size * 0.45)).foregroundStyle(c.headerAvatarInk)
                    }
                    if ringed { BadgeRing(badge: ring, size: size) }
                }
                .frame(width: size, height: size)
                // A minha medalha: canto de baixo, a esquerda (a engrenagem fica a direita).
                if ringed {
                    BadgeCoin(badge: ring, size: size * 0.45)
                        .frame(width: size, height: size, alignment: .bottomLeading)
                        .offset(x: -6, y: 4)
                }
                Circle().fill(c.surface).frame(width: 32, height: 32).shadow(color: .black.opacity(0.2), radius: 3, y: 1)
                    .overlay(Image(systemName: badge).font(.system(size: 15, weight: .semibold)).foregroundStyle(c.primary))
                    .offset(x: 6, y: 6)
            }
            .frame(width: size + 10, height: size + 10, alignment: .topLeading)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// O meu ID abreviado numa pastilha (tocar mostra o QR e o ID completo).
struct IdChip: View {
    let publicId: String
    let action: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: "lock.fill").font(.system(size: 12))
                Text(shortId(publicId)).font(Jakarta.font(13, .semibold)).lineLimit(1)
            }
            .foregroundStyle(c.onTopBar.opacity(0.92))
            .padding(.horizontal, 12)
            .frame(minHeight: 34)
            .background(Capsule().fill(Color.white.opacity(0.14)))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(s("t_o_meu_id"))
    }
}

/// So o inicio e o fim do ID (para reconhecer); Copiar e Partilhar dao o ID completo.
func shortId(_ id: String) -> String { id.count > 20 ? String(id.prefix(10)) + "…" + String(id.suffix(6)) : id }

// ========================================================================= avatares

private struct PhotosKey: EnvironmentKey { static let defaultValue: [String: UIImage] = [:] }

extension EnvironmentValues {
    /// Fotos de perfil ja decifradas, por contactId (os Avatar leem daqui).
    var photos: [String: UIImage] {
        get { self[PhotosKey.self] }
        set { self[PhotosKey.self] = newValue }
    }
}

/// Avatar: a foto de perfil do contacto (se houver) ou as iniciais, numa cor estavel por
/// contacto (a mesma do Android), com o ponto verde quando esta online.
struct Avatar: View {
    let name: String
    let seed: String
    var size: CGFloat = 48
    var online = false
    @Environment(\.photos) private var photos
    @Environment(\.badges) private var badges
    @Environment(\.chat) private var c

    var body: some View {
        let palette = ChatColors.avatarPalette
        let color = palette[Int(javaHashCode(seed) & 0x7fffffff) % palette.count]
        // Badge: a foto encolhe para caber dentro do aro (o tamanho total fica igual).
        let badge = badges[seed] ?? Badges.none
        let ringed = Badges.colors(badge) != nil
        let inner = ringed ? size * Badges.inner : size
        ZStack(alignment: .bottomTrailing) {
            ZStack {
                if let photo = photos[seed] {
                    Image(uiImage: photo).resizable().scaledToFill().frame(width: inner, height: inner).clipShape(Circle())
                } else {
                    ZStack {
                        Circle().fill(color)
                        Text(initials(name)).font(Jakarta.font(size / 2.6, .semibold)).foregroundStyle(.white)
                    }
                    .frame(width: inner, height: inner)
                }
                if ringed { BadgeRing(badge: badge, size: size) }
            }
            .frame(width: size, height: size)
            if ringed {
                // Medalha no canto de baixo, a direita, um pouco por fora do aro.
                BadgeCoin(badge: badge, size: size * Badges.coinScale).offset(x: size * 0.06, y: size * 0.04)
            }
            if online {
                let d = max(10, size / 4)
                Circle().fill(onlineGreen).frame(width: d - 4, height: d - 4)
                    .padding(2).background(Circle().fill(c.surface))
                    .accessibilityLabel(s("t_online"))
                    // Com medalha, o ponto verde vai para cima.
                    .frame(width: size, height: size, alignment: ringed ? .topTrailing : .bottomTrailing)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(!online)
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

// ========================================================================== botoes

/// Botao principal arredondado (como o Button do Material 3).
struct PrimaryButton: View {
    let title: String
    var height: CGFloat = 52
    var radius: CGFloat = 26
    let action: () -> Void
    @Environment(\.chat) private var c
    @Environment(\.isEnabled) private var enabled
    var body: some View {
        Button(action: action) {
            Text(title).font(Jakarta.font(16, .semibold))
                .frame(maxWidth: .infinity).frame(height: height)
                .foregroundStyle(c.onPrimary)
                .background(RoundedRectangle(cornerRadius: radius).fill(c.primary.opacity(enabled ? 1 : 0.4)))
        }
        .buttonStyle(.plain)
    }
}

/// Botao com contorno (OutlinedButton do Material 3).
struct OutlineButton: View {
    let title: String
    var systemImage: String?
    var height: CGFloat = 52
    var color: Color?
    let action: () -> Void
    @Environment(\.chat) private var c
    @Environment(\.isEnabled) private var enabled
    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                if let systemImage { Image(systemName: systemImage) }
                Text(title).font(Jakarta.font(15, .bold)).lineLimit(1)
            }
            .frame(maxWidth: .infinity).frame(height: height)
            .foregroundStyle((color ?? c.primary).opacity(enabled ? 1 : 0.4))
            .overlay(RoundedRectangle(cornerRadius: 16).stroke(c.outlineVariant, lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct OutlinedField: View {
    let label: String
    @Binding var text: String
    var monospaced = false
    var secure = false
    var numeric = false
    var singleLine = false
    @Environment(\.chat) private var c
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label).font(.sLabelMedium).foregroundStyle(c.onSurfaceVariant)
            Group {
                if secure {
                    SecureField("", text: $text)
                } else if singleLine {
                    TextField("", text: $text)
                } else {
                    TextField("", text: $text, axis: .vertical).lineLimit(1...4)
                }
            }
            .keyboardType(numeric ? .numberPad : .default)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .font(monospaced ? .system(.footnote, design: .monospaced) : .sBodyLarge)
            .padding(12)
            .overlay(RoundedRectangle(cornerRadius: 14).stroke(c.onSurfaceVariant.opacity(0.5), lineWidth: 1))
        }
    }
}

/// Campo do codigo de acesso: so algarismos, escondido.
struct CodeField: View {
    let label: String
    @Binding var code: String
    var body: some View {
        OutlinedField(label: label, text: Binding(get: { code }, set: { code = String($0.filter(\.isNumber).prefix(32)) }),
                      secure: true, numeric: true, singleLine: true)
    }
}

/// Texto de seccao das Definicoes ("CONTA", "PRIVACIDADE"...).
struct SectionLabel: View {
    let text: String
    @Environment(\.chat) private var c
    var body: some View {
        Text(text.uppercased())
            .font(Jakarta.font(12, .bold)).kerning(1)
            .foregroundStyle(c.onSurfaceVariant)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20).padding(.top, 18).padding(.bottom, 6)
    }
}

/// Linha das Definicoes: quadrado com o icone, titulo e seta (ou um interruptor).
struct SettingsRow<Trailing: View>: View {
    let icon: String?
    var glyph: String?
    let title: String
    var danger = false
    var divider = true
    let action: (() -> Void)?
    @ViewBuilder var trailing: () -> Trailing
    @Environment(\.chat) private var c

    var body: some View {
        let ink = danger ? c.error : c.primary
        let row = HStack(spacing: 14) {
            ZStack {
                RoundedRectangle(cornerRadius: 14).fill(danger ? c.errorContainer : c.tile)
                if let icon {
                    Image(systemName: icon).font(.system(size: 19, weight: .semibold)).foregroundStyle(ink)
                } else {
                    Text(glyph ?? "").font(Jakarta.font(15, .heavy)).foregroundStyle(ink)
                }
            }
            .frame(width: 44, height: 44)
            VStack(spacing: 0) {
                HStack {
                    Text(title).font(Jakarta.font(16, .bold)).foregroundStyle(danger ? c.error : c.onSurface)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    trailing()
                }
                .frame(minHeight: 67)
                .padding(.trailing, 16)
                if divider { Rectangle().fill(c.outlineVariant).frame(height: 0.5) }
            }
        }
        .padding(.leading, 20)
        .frame(minHeight: 68)
        .contentShape(Rectangle())
        if let action {
            Button(action: action) { row }.buttonStyle(.plain)
        } else {
            row
        }
    }
}

extension SettingsRow where Trailing == AnyView {
    /// Linha com a seta (se tiver acao).
    init(icon: String?, glyph: String? = nil, title: String, danger: Bool = false, divider: Bool = true, action: (() -> Void)?) {
        self.icon = icon
        self.glyph = glyph
        self.title = title
        self.danger = danger
        self.divider = divider
        self.action = action
        let hasAction = action != nil
        self.trailing = {
            AnyView(Group {
                if hasAction { Image(systemName: "chevron.right").font(.system(size: 14, weight: .semibold)).foregroundStyle(.secondary) }
            })
        }
    }
}

/// Linha com interruptor.
struct SettingsSwitch: View {
    let icon: String
    let title: String
    let isOn: Bool
    var divider = true
    let onChange: (Bool) -> Void
    @Environment(\.chat) private var c
    var body: some View {
        SettingsRow(icon: icon, title: title, divider: divider, action: { onChange(!isOn) }) {
            Toggle("", isOn: Binding(get: { isOn }, set: onChange)).labelsHidden().tint(c.primary)
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
    switch dayKey(ms) {
    case dayFmt.string(from: now): return s("t_hoje")
    case dayFmt.string(from: now.addingTimeInterval(-24 * 3600)): return s("t_ontem")
    default: return dateFmt.string(from: date(ms))
    }
}

/// Hora (hoje), "Ontem" ou a data, para a lista de conversas.
func shortTime(_ ms: Int64) -> String {
    let now = Date()
    switch dayKey(ms) {
    case dayFmt.string(from: now): return timeOf(ms)
    case dayFmt.string(from: now.addingTimeInterval(-24 * 3600)): return s("t_ontem")
    default: return shortDateFmt.string(from: date(ms))
    }
}

/// Toque com feedback (os avisos curtos do Android, "Toast").
struct NoticeBanner: View {
    let text: String
    @Environment(\.chat) private var c
    var body: some View {
        Text(text).font(.sBodyMedium).foregroundStyle(c.onPrimary)
            .padding(.horizontal, 18).padding(.vertical, 12)
            .background(Capsule().fill(c.onSurface.opacity(0.88)))
            .padding(.bottom, 40)
            .transition(.move(edge: .bottom).combined(with: .opacity))
    }
}
