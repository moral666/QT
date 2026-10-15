import SwiftUI
import UIKit

// ============================================================================ raiz

struct AppRoot: View {
    @ObservedObject var model: AppModel
    @State private var showSettings = false
    @State private var showMyId = false

    var body: some View {
        let ui = model.ui
        Group {
            if ui.loading {
                LoadingScreen()
            } else if !ui.hasIdentity {
                SetupScreen(ui: ui, onCreate: model.createIdentity)
            } else if let chatId = ui.openChatId {
                ChatScreen(
                    ui: ui, contactId: chatId,
                    contact: ui.contacts.first { $0.contactId == chatId },
                    onSend: model.send, onBack: model.closeChat, onAdd: model.addContact,
                    onSafety: model.showSafetyNumber, onLoadOlder: model.loadOlder
                )
                .transition(.move(edge: .trailing))
            } else if showSettings && Config.relayEditable {
                SettingsScreen(ui: ui, onSave: { u, k in model.saveRelay(relayUrl: u, relayKeyHex: k); showSettings = false },
                               onBack: { showSettings = false })
            } else {
                HomeScreen(
                    ui: ui, onOpen: model.openChat, onAddContact: model.addContact,
                    onMyId: { showMyId = true }, onSettings: { showSettings = true },
                    onReadReceipts: model.setReadReceipts
                )
            }
        }
        .animation(.easeOut(duration: 0.2), value: ui.openChatId)
        .sheet(isPresented: $showMyId) { MyIdSheet(publicId: ui.publicId) }
        .sheet(isPresented: Binding(get: { model.ui.safetyNumber != nil }, set: { if !$0 { model.dismissSafetyNumber() } })) {
            SafetyNumberSheet(text: ui.safetyNumber ?? "", onClose: model.dismissSafetyNumber)
        }
        .alert(
            "",
            isPresented: Binding(get: { model.ui.error != nil }, set: { if !$0 { model.dismissError() } }),
            actions: { Button("OK", action: model.dismissError) },
            message: { Text(ui.error ?? "") }
        )
    }
}

struct LoadingScreen: View {
    @Environment(\.chat) private var c
    var body: some View {
        VStack(spacing: 16) {
            BrandMark(size: 72)
            Text("A abrir o armazenamento seguro…").font(.subheadline)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(c.background.ignoresSafeArea())
    }
}

// ========================================================================= entrada

struct SetupScreen: View {
    let ui: UiState
    let onCreate: (String, String) -> Void
    @State private var url = ""
    @State private var key = ""
    @State private var advanced = false
    @Environment(\.chat) private var c

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                Spacer(minLength: 60)
                BrandMark(size: 96)
                Spacer().frame(height: 24)
                Text(Config.appName).font(.title.bold())
                Spacer().frame(height: 8)
                Text("Mensagens cifradas ponta a ponta, com criptografia pós-quântica. Sem número de telefone, sem email.")
                    .font(.subheadline).multilineTextAlignment(.center).foregroundStyle(c.onSurfaceVariant)
                Spacer().frame(height: 32)
                PrimaryButton(title: "Começar") { onCreate(url, key) }
                Spacer().frame(height: 16)
                Text("Versão alpha, ainda não auditada. Não uses para informação sensível.")
                    .font(.caption2).multilineTextAlignment(.center).foregroundStyle(c.error)
                if Config.relayEditable {   // so na build debug
                    Button(advanced ? "Esconder opções de teste" : "Opções de teste") { advanced.toggle() }
                        .padding(.top, 8)
                    if advanced {
                        VStack(spacing: 12) {
                            OutlinedField(label: "URL do relay", text: $url)
                            OutlinedField(label: "Chave pública do relay (pinning)", text: $key, monospaced: true)
                        }
                    }
                }
            }
            .padding(32)
            .frame(maxWidth: 520)
            .frame(maxWidth: .infinity)
        }
        .background(c.background.ignoresSafeArea())
        .onAppear { url = ui.relayUrl; key = ui.relayKeyHex }
    }
}

struct SettingsScreen: View {
    let ui: UiState
    let onSave: (String, String) -> Void
    let onBack: () -> Void
    @State private var url = ""
    @State private var key = ""
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(onBack: onBack) { Text("Relay (teste)").font(.title3) }
            VStack(alignment: .leading, spacing: 12) {
                OutlinedField(label: "URL do relay", text: $url)
                OutlinedField(label: "Chave pública do relay (pinning)", text: $key, monospaced: true)
                PrimaryButton(title: "Guardar e voltar a ligar", height: 44) { onSave(url, key) }
                    .fixedSize(horizontal: true, vertical: false)
            }
            .padding(16)
            Spacer()
        }
        .background(c.background.ignoresSafeArea())
        .onAppear { url = ui.relayUrl; key = ui.relayKeyHex }
    }
}

// ===================================================================== folhas

struct MyIdSheet: View {
    let publicId: String
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("O teu ID").font(.title2.bold())
            Text("Quem tiver este ID pode escrever-te. Partilha-o com as pessoas com quem queres falar.")
                .font(.footnote)
            Text(publicId)
                .font(.system(size: 12, design: .monospaced))
                .textSelection(.enabled)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 8).fill(c.surfaceVariant))
            HStack {
                Spacer()
                Button("Copiar") {
                    UIPasteboard.general.string = publicId
                    dismiss()
                }
                ShareLink("Partilhar", item: publicId)
                    .padding(.leading, 16)
            }
            .font(.body.weight(.medium))
        }
        .padding(24)
        .presentationDetents([.medium])
        .presentationBackground(c.surface)
    }
}

struct SafetyNumberSheet: View {
    let text: String
    let onClose: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Spacer(); Image(systemName: "lock.fill").font(.title2); Spacer() }
            Text("Verificar segurança").font(.title2.bold()).frame(maxWidth: .infinity)
            Text(text).font(.system(.body, design: .monospaced)).textSelection(.enabled)
            Text("Compara estes números com a outra pessoa, pessoalmente ou numa chamada. Se forem iguais, ninguém (nem o servidor) está a intercetar a conversa.")
                .font(.footnote)
            HStack { Spacer(); Button("Fechar", action: onClose).font(.body.weight(.medium)) }
        }
        .padding(24)
        .presentationDetents([.medium, .large])
        .presentationBackground(c.surface)
    }
}

struct AddContactSheet: View {
    let onAdd: (String, String) -> Void
    @State private var nick = ""
    @State private var pid = ""
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Novo contacto").font(.title2.bold())
            OutlinedField(label: "Nome", text: $nick)
            OutlinedField(label: "ID (qt1:…)", text: $pid, monospaced: true)
            Text("Recebe o ID diretamente da pessoa (pessoalmente ou por um canal em que confies).")
                .font(.footnote).foregroundStyle(c.onSurfaceVariant)
            HStack {
                Spacer()
                Button("Cancelar") { dismiss() }
                Button("Adicionar") { onAdd(nick, pid); dismiss() }
                    .disabled(pid.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .padding(.leading, 16)
            }
            .font(.body.weight(.medium))
        }
        .padding(24)
        .presentationDetents([.medium, .large])
        .presentationBackground(c.surface)
    }
}
