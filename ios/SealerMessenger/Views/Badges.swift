import CoreImage
import SwiftUI
import UIKit

/// Badges (aro colorido a volta da foto). O certificado e verificado no nucleo Rust
/// (`ffi/src/badges.rs`); aqui so se desenha. Um so desenho (`BadgeRing`, o aro dourado em
/// tons de cinza) pintado com duas cores, sombra e brilho (`CIFalseColor`), igual ao Android.
/// A Legacy e diferente: um escudo prateado a volta da foto (`BadgeShieldLegacy`), sem aro nem
/// medalha no canto.
/// Numeros iguais aos do Rust: 0 nenhum, 1 Founder, 2 Co-founder, 3 Dev, 4 Tester, 5 Contributor,
/// 6 Bug Hunter, 7 Support, 8 Troll, 9 Legacy.
enum Badges {
    static let none = 0
    /// Parte do aro ocupada pela foto (o buraco do desenho tem 83% do diametro).
    static let inner: CGFloat = 0.84
    /// Medalha: 40% do avatar (45% na minha foto do cabecalho).
    static let coinScale: CGFloat = 0.40
    /// Legacy: escudo a volta da foto, em vez do aro e da medalha.
    static let legacy = 9
    /// Escudo: lado do desenho em relacao ao avatar (passa um pouco para fora, sobretudo em baixo,
    /// para a foto nao ficar muito mais pequena do que com o aro).
    static let shieldScale: CGFloat = 1.25
    /// Foto dentro do escudo, em relacao ao avatar (o buraco do desenho tem 59,2% do lado).
    static let shieldPhoto: CGFloat = shieldScale * 0.60
    /// O centro do buraco esta a 45,2% da altura do desenho: o escudo desce 4,8% do seu lado.
    static let shieldDown: CGFloat = shieldScale * 0.048

    /// Ha alguma coisa a desenhar (aro + medalha, ou o escudo).
    static func shows(_ badge: Int) -> Bool { colors(badge) != nil || badge == legacy }

    /// (sombra, brilho) em RGB 0...255; nil = sem aro.
    static func colors(_ badge: Int) -> (UInt32, UInt32)? {
        switch badge {
        case 1: return (0x8A5F12, 0xFFE48C)   // Founder: dourado
        case 2: return (0x9E2E66, 0xFFC2DD)   // Co-founder: rosa
        case 3: return (0x141414, 0x8A8A8A)   // Dev: preto
        case 4: return (0x0A6F9C, 0xB5F1FF)   // Tester: ciano
        case 5: return (0x0D5E33, 0x7FE3A8)   // Contributor: verde
        case 6: return (0x8E0E1A, 0xFFB0B0)   // Bug Hunter: vermelho
        case 7: return (0x9A4208, 0xFFD2A0)   // Support: laranja
        case 8: return (0x14287A, 0xA9C0FF)   // Troll: azul
        default: return nil
        }
    }

    static func name(_ badge: Int) -> String? {
        switch badge {
        case 1: return s("badge_founder")
        case 2: return s("badge_cofounder")
        case 3: return s("badge_dev")
        case 4: return s("badge_tester")
        case 5: return s("badge_contributor")
        case 6: return s("badge_bughunter")
        case 7: return s("badge_support")
        case 8: return s("badge_troll")
        case 9: return s("badge_legacy")
        default: return nil
        }
    }

    @MainActor private static var cache: [Int: UIImage] = [:]

    /// O aro ja pintado (feito uma vez por badge e guardado em memoria).
    @MainActor static func ring(_ badge: Int) -> UIImage? {
        if let img = cache[badge] { return img }
        guard let pair = colors(badge),
              let base = UIImage(named: "BadgeRing"), let cg = base.cgImage else { return nil }
        func ci(_ rgb: UInt32) -> CIColor {
            CIColor(red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255, blue: CGFloat(rgb & 0xFF) / 255)
        }
        let input = CIImage(cgImage: cg)
        guard let filter = CIFilter(name: "CIFalseColor") else { return nil }
        filter.setValue(input, forKey: kCIInputImageKey)
        filter.setValue(ci(pair.0), forKey: "inputColor0")
        filter.setValue(ci(pair.1), forKey: "inputColor1")
        // O CIFalseColor deixa o alfa como estava: o fundo transparente continua transparente.
        guard let out = filter.outputImage,
              let made = CIContext().createCGImage(out, from: input.extent) else { return nil }
        let img = UIImage(cgImage: made, scale: base.scale, orientation: .up)
        cache[badge] = img
        return img
    }
}

private struct BadgesKey: EnvironmentKey { static let defaultValue: [String: Int] = [:] }

extension EnvironmentValues {
    /// Badges dos contactos (contactId -> numero, so os que tem), para os Avatar.
    var badges: [String: Int] {
        get { self[BadgesKey.self] }
        set { self[BadgesKey.self] = newValue }
    }
}

/// O aro do badge, do tamanho `size`. Nada se o badge for 0 ou desconhecido.
struct BadgeRing: View {
    let badge: Int
    let size: CGFloat

    var body: some View {
        if let img = Badges.ring(badge) {
            Image(uiImage: img).resizable().interpolation(.high).frame(width: size, height: size)
                .accessibilityLabel(Badges.name(badge) ?? "")
        }
    }
}

/// O escudo da Legacy para um avatar de lado `size`. Usar em `.overlay` do avatar: fica maior do
/// que o avatar (passa para fora) sem mudar o tamanho dele.
struct BadgeShield: View {
    let size: CGFloat

    var body: some View {
        Image("BadgeShieldLegacy").resizable().interpolation(.high)
            .frame(width: size * Badges.shieldScale, height: size * Badges.shieldScale)
            .offset(y: size * Badges.shieldDown)
            .allowsHitTesting(false)
            .accessibilityLabel(Badges.name(Badges.legacy) ?? "")
    }
}

/// A medalha do badge (moeda com o simbolo), `BadgeCoin1`...`BadgeCoin8` no Assets.
struct BadgeCoin: View {
    let badge: Int
    let size: CGFloat

    var body: some View {
        if Badges.colors(badge) != nil, let img = UIImage(named: "BadgeCoin\(badge)") {
            Image(uiImage: img).resizable().interpolation(.high).frame(width: size, height: size)
                .shadow(color: .black.opacity(0.3), radius: 2, y: 1)
                .accessibilityHidden(true)
        }
    }
}
