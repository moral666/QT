import Foundation

/// Um relay da federacao (nome, endereco e chave publica Noise fixada).
struct RelayInfo: Equatable {
    let name: String
    let url: String
    let pin: String
}

/// Relays da federacao fechada (fase 3), igual ao `Relays.kt` do Android: o principal da build
/// (`QT_RELAY_URL`/`QT_RELAY_PIN`/`QT_RELAY_NAME`) e os outros (`QT_RELAYS_OTHERS`), por esta ordem.
/// A conta vive num deles; a app fala so com esse.
///
///  * Sem escolha guardada, a conta esta no principal (todas as contas antigas estao la).
///  * Conta nova num relay fechado a contas novas: o seguinte da lista.
///  * O relay diz "conta noutro relay: X" (ex.: recuperada noutro telemovel): muda para X.
///
/// Sempre os da build, tambem na Debug (o relay de teste editavel saiu na 0.9.53).
enum Relays {
    private static let prefHome = "relay_home"

    static let all: [RelayInfo] = {
        var list = [RelayInfo(name: Config.relayName, url: Config.relayURL, pin: Config.relayPin)]
        for e in Config.relaysOthers.split(separator: ";") {
            let p = e.split(separator: "|", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }
            guard p.count == 3, !p[0].isEmpty, !p[1].isEmpty, !list.contains(where: { $0.name == p[0] }) else { continue }
            list.append(RelayInfo(name: p[0], url: p[1], pin: p[2].lowercased()))
        }
        return list
    }()

    static func byName(_ name: String?) -> RelayInfo? {
        guard let name else { return nil }
        return all.first { $0.name == name }
    }

    /// O relay principal (o de cima da lista).
    static var main: RelayInfo { all[0] }

    /// O relay da conta deste telemovel.
    static func home(_ prefs: UserDefaults) -> RelayInfo {
        byName(prefs.string(forKey: prefHome)) ?? all[0]
    }

    /// Muda o relay da conta. Falso com um nome desconhecido.
    @discardableResult
    static func setHome(_ prefs: UserDefaults, _ name: String) -> Bool {
        guard byName(name) != nil else { return false }
        prefs.set(name, forKey: prefHome)
        return true
    }

    /// Conta nova: volta a comecar pelo principal.
    static func resetHome(_ prefs: UserDefaults) { prefs.removeObject(forKey: prefHome) }

    /// O seguinte da lista que ainda nao foi tentado.
    static func next(_ tried: Set<String>) -> RelayInfo? { all.first { !tried.contains($0.name) } }

    /// O nome que vai nas ofertas de chamada: so quando a conta nao esta no principal.
    static func callName(_ prefs: UserDefaults) -> String? {
        let h = home(prefs)
        return h.name == main.name ? nil : h.name
    }

    /// O relay das chamadas de uma chamada (o de quem liga; sem nome: o principal).
    static func forCall(_ prefs: UserDefaults, _ name: String?) -> RelayInfo { byName(name) ?? main }
}
