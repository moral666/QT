import SwiftUI

// ===================================================================== conversas

struct HomeScreen: View {
    let ui: UiState
    let onOpen: (String) -> Void
    let onAddContact: (String, String) -> Void
    let onMyId: () -> Void
    let onSettings: () -> Void
    let onReadReceipts: (Bool) -> Void
    let onDeleteConversation: (String) -> Void
    let onDeleteContact: (String) -> Void
    let onWipeAll: () -> Void
    @State private var showAdd = false
    @State private var deleteTarget: DeleteTarget?
    @State private var showWipe = false
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(title: { SealerWordmark() }) {
                TopBarIcon(systemName: "square.and.arrow.up", label: "O meu ID", action: onMyId)
                Menu {
                    Button("O meu ID", action: onMyId)
                    Toggle("Recibos de leitura", isOn: Binding(get: { ui.readReceipts }, set: onReadReceipts))
                    if Config.relayEditable {
                        Button("Relay (teste)", action: onSettings)
                    }
                    Divider()
                    Button("Apagar todos os dados", role: .destructive) { showWipe = true }
                } label: {
                    Image(systemName: "ellipsis").rotationEffect(.degrees(90))
                        .font(.system(size: 19, weight: .medium)).frame(width: 44, height: 44)
                }
                .accessibilityLabel("Mais opções")
            }
            if !ui.status.isEmpty && ui.status != "Ligado ao relay" {
                ConnectionBanner(status: ui.status)
            }
            let withHistory = Set(ui.conversations.map(\.contactId))
            let idle = ui.contacts.filter { !withHistory.contains($0.contactId) }
            if ui.conversations.isEmpty && idle.isEmpty {
                EmptyConversations(onMyId: onMyId)
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(ui.conversations, id: \.contactId) { conv in
                            let name = ui.contacts.first { $0.contactId == conv.contactId }?.nickname
                            ConversationRow(name: name ?? "Desconhecido", contactId: conv.contactId, conv: conv) {
                                onOpen(conv.contactId)
                            }
                            .contextMenu {   // toque longo, como no Android
                                Button("Eliminar conversa", systemImage: "trash") {
                                    deleteTarget = .conversation(id: conv.contactId, name: name ?? "Desconhecido")
                                }
                                Button("Apagar contacto", systemImage: "person.crop.circle.badge.minus", role: .destructive) {
                                    deleteTarget = .contact(id: conv.contactId, name: name ?? "Desconhecido")
                                }
                            }
                        }
                        ForEach(idle) { contact in
                            ConversationRow(name: contact.nickname, contactId: contact.contactId, conv: nil) {
                                onOpen(contact.contactId)
                            }
                            .contextMenu {
                                Button("Apagar contacto", systemImage: "person.crop.circle.badge.minus", role: .destructive) {
                                    deleteTarget = .contact(id: contact.contactId, name: contact.nickname)
                                }
                            }
                        }
                    }
                    .padding(.bottom, 88)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(c.background.ignoresSafeArea())
        .overlay(alignment: .bottomTrailing) {
            Button { showAdd = true } label: {
                Image(systemName: "plus").font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(c.onPrimary)
                    .frame(width: 56, height: 56)
                    .background(RoundedRectangle(cornerRadius: 16).fill(c.primary))
                    .shadow(color: .black.opacity(0.25), radius: 4, y: 2)
            }
            .accessibilityLabel("Novo contacto")
            .padding(16)
        }
        .sheet(isPresented: $showAdd) { AddContactSheet(onAdd: onAddContact) }
        .sheet(isPresented: $showWipe) { WipeAllSheet(onConfirm: onWipeAll) }
        .deleteConfirmation($deleteTarget, onDeleteConversation: onDeleteConversation, onDeleteContact: onDeleteContact)
    }
}

struct EmptyConversations: View {
    let onMyId: () -> Void
    @Environment(\.chat) private var c
    var body: some View {
        VStack(spacing: 0) {
            BrandMark(size: 64)
            Spacer().frame(height: 16)
            Text("Ainda não tens conversas").font(.headline)
            Spacer().frame(height: 8)
            Text("Partilha o teu ID com quem quiseres falar, ou toca em + para adicionar o ID de outra pessoa.")
                .font(.subheadline).multilineTextAlignment(.center).foregroundStyle(c.onSurfaceVariant)
            Spacer().frame(height: 16)
            Button(action: onMyId) {
                Text("Partilhar o meu ID").font(.subheadline.weight(.medium))
                    .padding(.horizontal, 24).padding(.vertical, 10)
                    .overlay(Capsule().stroke(c.onSurfaceVariant.opacity(0.6), lineWidth: 1))
            }
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct ConversationRow: View {
    let name: String
    let contactId: String
    let conv: ConversationSummary?
    let onClick: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        let last = conv?.lastMessage
        let unread = conv?.unread ?? 0
        Button(action: onClick) {
            VStack(spacing: 0) {
                HStack(spacing: 14) {
                    Avatar(name: name, seed: contactId)
                    VStack(alignment: .leading, spacing: 2) {
                        HStack {
                            Text(name).font(.headline).lineLimit(1).truncationMode(.tail)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if let last {
                                Text(shortTime(last.localAtMs)).font(.caption2)
                                    .foregroundStyle(unread > 0 ? c.primary : c.onSurfaceVariant)
                            }
                        }
                        HStack {
                            Text(preview(last)).font(.subheadline).foregroundStyle(c.onSurfaceVariant)
                                .lineLimit(1).truncationMode(.tail)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if unread > 0 {
                                Text(String(unread)).font(.system(size: 12, weight: .bold))
                                    .foregroundStyle(c.onPrimary)
                                    .padding(.horizontal, 7).padding(.vertical, 2)
                                    .background(Capsule().fill(c.primary))
                            }
                        }
                    }
                }
                .padding(.horizontal, 16).padding(.vertical, 10)
                Rectangle().fill(c.outlineVariant).frame(height: 0.5).padding(.leading, 78)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func preview(_ last: ChatMessage?) -> String {
        guard let last else { return "Toca para começar a conversar" }
        return last.outgoing ? statusGlyph(last.status) + "  " + last.text : last.text
    }
}
