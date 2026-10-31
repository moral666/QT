import Foundation

/// Configuracao da build (equivalente ao BuildConfig do Android).
/// Os valores vem do Info.plist, preenchidos pelos ficheiros ios/Configs/*.xcconfig:
///  * Relay FIXO em todas as builds (QT_RELAY_URL / QT_RELAY_PIN em Relay.xcconfig), nao editavel.
///  * Release: so wss://. Debug: ws:// permitido.
enum Config {
    static let appName = "SEALER"

    private static func plist(_ key: String) -> String {
        (Bundle.main.object(forInfoDictionaryKey: key) as? String)?.trimmingCharacters(in: .whitespaces) ?? ""
    }

    static var relayURL: String { plist("QTRelayURL") }
    static var relayPin: String { plist("QTRelayPin").lowercased() }
    /// Federacao: nome do relay de cima e os outros ("nome|wss://...|pin;..."), ver `Relays`.
    static var relayName: String { let n = plist("QTRelayName"); return n.isEmpty ? "estocolmo" : n }
    static var relaysOthers: String { plist("QTRelaysOthers") }
    static var allowCleartext: Bool { plist("QTAllowCleartext") == "YES" }
    /// Versao instalada (MARKETING_VERSION), so informativa nas Definicoes.
    static var version: String { plist("CFBundleShortVersionString") }

    /// Notificacoes da Apple: "s" (sandbox, apps instaladas pelo Xcode) ou "p" (TestFlight e
    /// App Store). Vem do perfil de assinatura embebido na app; sem ele (App Store), "p".
    static let pushEnvironment: String = {
        guard let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
              let raw = try? Data(contentsOf: url),
              let text = String(data: raw, encoding: .isoLatin1),
              let r = text.range(of: "<key>aps-environment</key>") else { return "p" }
        let rest = text[r.upperBound...].prefix(80)
        return rest.contains("development") ? "s" : "p"
    }()
}
