import Foundation

/// Idiomas da app: PT (base), EN, FR, ES, DE, IT, JA, ZH (chines simplificado, `zh-Hans`), HI e AR,
/// os mesmos textos da app Android. O arabe escreve-se da direita para a esquerda: o SwiftUI vira os
/// ecras sozinho desde que se use `leading`/`trailing` (nunca esquerda/direita fixas).
///
/// Os ficheiros `Resources/<lingua>.lproj/Localizable.strings` sao gerados a partir de
/// `android/app/src/main/res/values*/strings.xml` por `scripts/strings-ios.py` (as mesmas
/// chaves). Nenhum texto do ecra e escrito no Swift: usar `s("chave", args...)`.
///
/// Segue a lingua do iPhone (PT se nao for nenhuma destas). A escolha manual faz-se nas
/// Definicoes do iPhone (SEALER > Idioma), como a Apple recomenda: a app nao guarda nada.
enum Lang {
    static let supported = ["pt", "en", "fr", "es", "de", "it", "ja", "zh", "hi", "ar"]

    /// Locale para datas e numeros: o do iPhone, mas em arabe com algarismos 0-9 (decisao do Nuno).
    static var numbersLocale: Locale {
        let cur = Locale.current
        guard cur.language.languageCode?.identifier == "ar" || Bundle.main.preferredLocalizations.first == "ar" else { return cur }
        return Locale(identifier: cur.identifier + "@numbers=latn")
    }

    /// Codigo de acesso: so algarismos e sempre 0-9 (teclados em arabe ou hindi enviam ٠-٩ ou ०-९).
    static func asciiDigits(_ v: String, max: Int = 32) -> String {
        String(v.compactMap { $0.wholeNumberValue }.filter { (0...9).contains($0) }.map(String.init).joined().prefix(max))
    }

    /// Para onde vao os avisos de erros de traducao (Definicoes > Reportar erro de traducao).
    static let reportEmail = "privacidade@sealer.pt"

    /// Email com um aviso de erro de traducao: so a lingua e a versao da app (nada do utilizador).
    static func reportURL(version: String) -> URL? {
        var c = URLComponents()
        c.scheme = "mailto"
        c.path = reportEmail
        let lang = Bundle.main.preferredLocalizations.first ?? "pt"
        c.queryItems = [
            URLQueryItem(name: "subject", value: "SEALER: erro de tradução (\(lang), \(version))"),
            URLQueryItem(name: "body", value: s("lang_report_body")),
        ]
        return c.url
    }

    /// A palavra escrita para confirmar "Destruir perfil e dados" (APAGAR, 削除, हटाएं...) esta
    /// certa? Aceita tambem as grafias de `confirm_word_alt` (separadas por virgulas). Ignora
    /// maiusculas, espacos nas pontas e as duas formas do nasal em hindi (ँ e ं).
    static func confirmWordOk(_ typed: String) -> Bool {
        func norm(_ t: String) -> String {
            t.trimmingCharacters(in: .whitespaces).precomposedStringWithCanonicalMapping
                .replacingOccurrences(of: "\u{0901}", with: "\u{0902}")
                .replacingOccurrences(of: "[\u{0640}\u{064B}-\u{0652}\u{200E}\u{200F}]", with: "", options: .regularExpression)
        }
        let alts = s("confirm_word_alt").split(separator: ",").map(String.init).filter { !$0.isEmpty }
        return ([s("confirm_word")] + alts).contains { norm(typed).caseInsensitiveCompare(norm($0)) == .orderedSame }
    }

    /// A lingua em uso agora ("pt", "en"...).
    static var current: String {
        let code = Bundle.main.preferredLocalizations.first ?? "pt"
        return String(code.prefix(2))
    }
}

/// Texto traduzido. Os argumentos substituem `%1$@`, `%2$@`... (vem de `%1$s` no Android).
func s(_ key: String, _ args: Any...) -> String {
    let format = NSLocalizedString(key, comment: "")
    guard !args.isEmpty else { return format }
    let strings: [CVarArg] = args.map { String(describing: $0) as NSString }
    return String(format: format, arguments: strings)
}

/// Textos que vem do nucleo Rust (sempre em portugues) para a lingua da app.
func coreText(_ t: String) -> String {
    if t == "📷 Foto" { return "📷 " + s("t_foto") }
    if t == "🎤 Mensagem de voz" { return "🎤 " + s("core_voice_message") }
    if t.hasPrefix("⚠️ Esta mensagem usa uma funcionalidade") { return "⚠️ " + s("core_unsupported") }
    return t
}
