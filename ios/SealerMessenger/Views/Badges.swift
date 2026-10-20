import CoreImage
import SwiftUI
import UIKit

/// Badges (aro colorido a volta da foto). O certificado e verificado no nucleo Rust
/// (`ffi/src/badges.rs`); aqui so se desenha. Um so desenho (`BadgeRing`, o aro dourado em
/// tons de cinza) pintado com duas cores, sombra e brilho (`CIFalseColor`), igual ao Android.
/// Numeros iguais aos do Rust: 0 nenhum, 1 Founder, 2 Co-founder, 3 Dev, 4 Tester, 5 Contributor.
enum Badges {
    static let none = 0
    /// Parte do aro ocupada pela foto (o buraco do desenho tem 83% do diametro).
    static let inner: CGFloat = 0.84

    /// (sombra, brilho) em RGB 0...255; nil = sem aro.
    static func colors(_ badge: Int) -> (UInt32, UInt32)? {
        switch badge {
        case 1: return (0x8A5F12, 0xFFE48C)   // Founder: dourado
        case 2: return (0x9E2E66, 0xFFC2DD)   // Co-founder: rosa
        case 3: return (0x141414, 0x8A8A8A)   // Dev: preto
        case 4: return (0x0A6F9C, 0xB5F1FF)   // Tester: ciano
        case 5: return (0x0D5E33, 0x7FE3A8)   // Contributor: verde
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
