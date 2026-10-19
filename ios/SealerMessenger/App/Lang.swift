import Foundation

/// Idiomas da app: PT (base), EN, FR, ES, DE e IT, os mesmos textos da app Android.
///
/// Os ficheiros `Resources/<lingua>.lproj/Localizable.strings` sao gerados a partir de
/// `android/app/src/main/res/values*/strings.xml` por `scripts/strings-ios.py` (as mesmas
/// chaves). Nenhum texto do ecra e escrito no Swift: usar `s("chave", args...)`.
///
/// Segue a lingua do iPhone (PT se nao for nenhuma destas). A escolha manual faz-se nas
/// Definicoes do iPhone (SEALER > Idioma), como a Apple recomenda: a app nao guarda nada.
enum Lang {
    static let supported = ["pt", "en", "fr", "es", "de", "it"]

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
