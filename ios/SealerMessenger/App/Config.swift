import Foundation

/// Configuracao da build (equivalente ao BuildConfig do Android).
/// Os valores vem do Info.plist, preenchidos pelos ficheiros ios/Configs/*.xcconfig:
///  * Release: relay FIXO (QT_RELAY_URL / QT_RELAY_PIN em Relay.xcconfig), nao editavel, so wss://.
///  * Debug:   relay editavel na app ("Opcoes de teste"); ws:// permitido para testes locais.
enum Config {
    static let appName = "SEALER"

    private static func plist(_ key: String) -> String {
        (Bundle.main.object(forInfoDictionaryKey: key) as? String)?.trimmingCharacters(in: .whitespaces) ?? ""
    }

    static var relayURL: String { plist("QTRelayURL") }
    static var relayPin: String { plist("QTRelayPin").lowercased() }
    static var relayEditable: Bool { plist("QTRelayEditable") == "YES" }
    static var allowCleartext: Bool { plist("QTAllowCleartext") == "YES" }
}
