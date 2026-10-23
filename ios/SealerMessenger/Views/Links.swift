import SwiftUI
import UIKit

/// Links nas mensagens (0.9.29), igual ao `Links.kt` do Android: ficam clicaveis e copiaveis.
/// Nada sai da app sem a pessoa confirmar o site, e nao ha pre-visualizacao (quem recebe nunca
/// contacta o site sozinho). So `http(s)://` e `www.`; outros esquemas ficam texto.
enum Links {
    private static let re = try! NSRegularExpression(pattern: #"(?i)\b(?:https?://|www\.)[^\s<>"]+"#)
    private static let trail = Set(".,;:!?)]}'\"…»")
    private static let maxLinks = 20

    /// Links do texto (sem a pontuacao do fim da frase), com a posicao e o endereco para abrir.
    static func find(_ text: String) -> [(range: Range<String.Index>, raw: String, url: URL)] {
        let ns = NSRange(text.startIndex..., in: text)
        var out: [(range: Range<String.Index>, raw: String, url: URL)] = []
        for m in re.matches(in: text, range: ns).prefix(maxLinks) {
            guard let r = Range(m.range, in: text) else { continue }
            var end = r.upperBound
            while end > r.lowerBound {
                let last = text[text.index(before: end)]
                guard trail.contains(last) else { break }
                // ")" fica se o link tiver "(" (ex.: wikipedia/Foo_(bar)).
                if last == ")" && text[r.lowerBound..<text.index(before: end)].contains("(") { break }
                end = text.index(before: end)
            }
            let range = r.lowerBound..<end
            let raw = String(text[range])
            if let url = open(raw) { out.append((range, raw, url)) }
        }
        return out
    }

    static func urls(_ text: String) -> [String] {
        var seen = Set<String>()
        return find(text).map(\.raw).filter { seen.insert($0).inserted }
    }

    /// Endereco para abrir (com `https://` se comecar por `www.`), ou `nil` se nao for web.
    static func open(_ raw: String) -> URL? {
        let u = raw.lowercased().hasPrefix("www.") ? "https://" + raw : raw
        guard let url = URL(string: u), let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https", let host = url.host, !host.isEmpty else { return nil }
        return url
    }

    /// O nome do site. O `URL` da Apple ja guarda os nomes com outros alfabetos em `xn--`
    /// (os links falsos usam letras parecidas): mostra-se esse nome, que e o verdadeiro.
    static func host(_ url: URL) -> String { (url.host ?? "").lowercased() }
}

/// Texto da mensagem com os links sublinhados. Tocar num link pede confirmacao antes de abrir.
struct LinkedText: View {
    let text: String
    let color: Color
    @State private var confirm: URL?

    var body: some View {
        Text(attributed)
            .environment(\.openURL, OpenURLAction { url in
                confirm = url
                return .handled
            })
            .alert(s("link_open_title"), isPresented: Binding(get: { confirm != nil }, set: { if !$0 { confirm = nil } })) {
                Button(s("t_abrir")) {
                    if let u = confirm { UIApplication.shared.open(u) }
                    confirm = nil
                }
                Button(s("t_cancelar"), role: .cancel) { confirm = nil }
            } message: {
                if let u = confirm {
                    Text("\(Links.host(u))\n\n\(u.absoluteString)\n\n\(s("link_open_warn"))")
                }
            }
    }

    private var attributed: AttributedString {
        var a = AttributedString(text)
        for l in Links.find(text) {
            guard let lo = AttributedString.Index(l.range.lowerBound, within: a),
                  let hi = AttributedString.Index(l.range.upperBound, within: a) else { continue }
            a[lo..<hi].link = l.url
            a[lo..<hi].underlineStyle = .single
            a[lo..<hi].foregroundColor = color
        }
        return a
    }
}
