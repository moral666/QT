import SwiftUI
import UIKit
import UniformTypeIdentifiers

// ============================================================ chave de recuperacao

/// As 12 palavras numeradas, em duas colunas (igual ao WordGrid do Android).
private struct WordGrid: View {
    let words: [String]
    @Environment(\.chat) private var c
    var body: some View {
        HStack(alignment: .top) {
            ForEach(0..<2, id: \.self) { col in
                VStack(alignment: .leading, spacing: 8) {
                    ForEach((col * 6)..<min(col * 6 + 6, words.count), id: \.self) { i in
                        Text(verbatim: "\(i + 1). \(words[i])").font(.system(.body, design: .monospaced).weight(.semibold))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(c.tile))
        .privacySensitive()
    }
}

/// Definicoes -> Chave de recuperacao (igual ao RecoveryScreen do Android). Sem chave: explica
/// e cria. A criar: mostra as palavras e pede 3 delas. Ativa: ver, gerar novas ou desligar.
struct RecoveryScreen: View {
    let ui: UiState
    let onBack: () -> Void
    let onCreate: () -> Void
    let onShow: () -> Void
    let onWritten: () -> Void
    let onHide: () -> Void
    let onDisable: () -> Void
    var onCode: (String) -> Void = { _ in }
    var onCancelCode: () -> Void = {}
    @State private var checking = false
    @State private var justCreated = false
    @State private var confirmNew = false
    @State private var confirmOff = false
    @State private var copied = false
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(onBack: { if ui.recoveryWords != nil { onHide() } else { onBack() } }) { Text(s("recovery_title")) }
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if ui.recoveryBusy {
                        HStack(spacing: 12) { ProgressView(); Text(s("recovery_working")) }
                    } else if let words = ui.recoveryWords, checking {
                        CheckWords(words: words, onOk: { justCreated = false; checking = false; onWritten() },
                                   onBack: { checking = false })
                    } else if let words = ui.recoveryWords {
                        Text(s("recovery_write_down")).font(.sBodyMedium)
                        WordGrid(words: words)
                        // Copiar as 12 palavras com um toque (0.9.29): so neste iPhone (sem a
                        // area de transferencia partilhada com o Mac) e apagadas ao fim de 1 minuto.
                        OutlineButton(title: s("recovery_copy")) {
                            UIPasteboard.general.setItems(
                                [[UTType.plainText.identifier: words.joined(separator: " ")]],
                                options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(60)]
                            )
                            copied = true
                        }
                        if copied { Text(s("recovery_copied")).font(.sBodySmall) }
                        Text(s("recovery_warning")).font(.sBodySmall).foregroundStyle(c.error)
                        PrimaryButton(title: justCreated ? s("recovery_written") : s("recovery_done")) {
                            if justCreated { checking = true } else { onHide() }
                        }
                    } else if ui.recoveryOn {
                        Text(s("recovery_on")).font(.sBodyMedium)
                        Text(s("recovery_intro")).font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
                        PrimaryButton(title: s("recovery_show"), action: onShow)
                        Button(s("recovery_new")) { confirmNew = true }.font(.sLabelLarge).padding(.top, 4)
                        Button(s("recovery_disable"), role: .destructive) { confirmOff = true }.font(.sLabelLarge).padding(.top, 4)
                    } else {
                        Text(s("recovery_intro")).font(.sBodyMedium)
                        Text(s("recovery_privacy")).font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
                        PrimaryButton(title: s("recovery_enable")) { justCreated = true; onCreate() }
                    }
                }
                .padding(20)
            }
        }
        .background(c.background.ignoresSafeArea())
        // Com o bloqueio da app: o codigo dela antes de ver ou gerar as palavras (0.9.23).
        .sheet(isPresented: Binding(get: { ui.recoveryAskCode }, set: { if !$0 { onCancelCode() } })) {
            CodeSheet(title: s("recovery_title"), text: s("recovery_auth"), askCurrent: true, askNew: false,
                      confirm: s("t_continuar")) { cur, _ in onCode(cur) }
        }
        .alert(s("recovery_new"), isPresented: $confirmNew) {
            Button(s("recovery_new")) { justCreated = true; onCreate() }
            Button(s("t_cancelar"), role: .cancel) {}
        } message: { Text(s("recovery_new_confirm")) }
        .alert(s("recovery_disable"), isPresented: $confirmOff) {
            Button(s("recovery_disable"), role: .destructive) { onDisable() }
            Button(s("t_cancelar"), role: .cancel) {}
        } message: { Text(s("recovery_disable_confirm")) }
    }
}

/// Pede 3 palavras ao acaso (pela posicao) para confirmar que foram escritas.
private struct CheckWords: View {
    let words: [String]
    let onOk: () -> Void
    let onBack: () -> Void
    @State private var positions: [Int] = []
    @State private var typed = ["", "", ""]
    @State private var wrong = false
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(s("recovery_check_intro")).font(.sBodyMedium)
            ForEach(Array(positions.enumerated()), id: \.offset) { i, pos in
                OutlinedField(label: s("recovery_check", pos + 1), text: $typed[i], monospaced: true, singleLine: true)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            }
            if wrong { Text(s("recovery_check_wrong")).foregroundStyle(c.error) }
            PrimaryButton(title: s("recovery_done")) {
                let ok = positions.indices.allSatisfy {
                    typed[$0].trimmingCharacters(in: .whitespaces).lowercased() == words[positions[$0]]
                }
                if ok { onOk() } else { wrong = true }
            }
            Button(s("recovery_see_again"), action: onBack).font(.sLabelLarge)
        }
        .onAppear { if positions.isEmpty { positions = Array(words.indices.shuffled().prefix(3)).sorted() } }
    }
}

/// Primeiro ecra -> Recuperar conta: as 12 palavras (escritas ou coladas, separadas por
/// espacos). Avisa logo de palavras que nao existem na lista.
struct RestoreScreen: View {
    let ui: UiState
    let onBack: () -> Void
    let onRestore: ([String]) -> Void
    @State private var text = ""
    @Environment(\.chat) private var c

    private var words: [String] {
        text.lowercased().split(whereSeparator: { $0.isWhitespace }).map(String.init)
    }

    var body: some View {
        let unknown = words.filter { !recoveryIsWord(word: $0) }
        VStack(spacing: 0) {
            SealerTopBar(onBack: { if !ui.recoveryBusy { onBack() } }) { Text(s("recovery_restore")) }
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Text(s("recovery_restore_hint")).font(.sBodyMedium)
                    TextEditor(text: $text)
                        .font(.system(.body, design: .monospaced))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .frame(height: 160)
                        .padding(8)
                        .background(RoundedRectangle(cornerRadius: 12).stroke(c.outlineVariant))
                        .disabled(ui.recoveryBusy)
                        .privacySensitive()
                    Text(s("recovery_words_count", words.count)).font(.sLabelMedium)
                    if !unknown.isEmpty {
                        Text(s("recovery_unknown_words", unknown.joined(separator: ", "))).foregroundStyle(c.error)
                    }
                    // O que volta e o que nao volta, antes de recuperar (igual ao Android).
                    Text(s("recovery_restore_what")).font(.sBodyMedium)
                    Text(s("recovery_restore_warning")).font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
                    if ui.recoveryBusy {
                        HStack(spacing: 12) { ProgressView(); Text(s("recovery_restoring")) }
                    } else {
                        PrimaryButton(title: s("recovery_restore")) { onRestore(words) }
                            .disabled(words.count != 12 || !unknown.isEmpty)
                    }
                }
                .padding(20)
            }
        }
        .background(c.background.ignoresSafeArea())
    }
}
