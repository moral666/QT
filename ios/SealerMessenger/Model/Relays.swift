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
///  * Conta nova (0.9.69): no relay que responde mais depressa (`newAccountOrder`, medido pelo
///    nucleo); se estiver fechado a contas novas, o seguinte dessa ordem. Nunca depois.
///  * O relay diz "conta noutro relay: X" (ex.: recuperada noutro telemovel): muda para X.
///
/// Sempre os da build, tambem na Debug (o relay de teste editavel saiu na 0.9.53).
enum Relays {
    private static let prefHome = "relay_home"
    /// Tempo maximo da medida dos relays (conta nova), em ms.
    private static let probeMs: UInt32 = 2_000

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

    /// CONTA NOVA (0.9.69): os relays do mais rapido para o mais lento. O nucleo mede so a ligacao
    /// TCP a cada um (sem TLS nem pedidos, no maximo `probeMs`); os que nao respondem ficam no fim
    /// e, sem nenhuma resposta, fica a ordem da lista. Bloqueia: chamar com `background`.
    static func newAccountOrder() -> [String] {
        let order = relayOrderForNewAccount(relays: all.map { RelayCandidate(name: $0.name, url: $0.url) }, timeoutMs: probeMs)
        return order.isEmpty ? all.map(\.name) : order
    }

    /// CONTA NOVA: o primeiro de `order` ainda nao tentado (os tentados disseram "sem contas novas").
    static func nextForNewAccount(_ order: [String], _ tried: Set<String>) -> RelayInfo? {
        byName(relayNextForNewAccount(order: order, tried: Array(tried)))
    }

    /// O nome que vai nas ofertas de chamada: so quando a conta nao esta no principal.
    static func callName(_ prefs: UserDefaults) -> String? {
        let h = home(prefs)
        return h.name == main.name ? nil : h.name
    }

    /// O relay das chamadas de uma chamada (o de quem liga; sem nome: o principal).
    static func forCall(_ prefs: UserDefaults, _ name: String?) -> RelayInfo { byName(name) ?? main }
}
