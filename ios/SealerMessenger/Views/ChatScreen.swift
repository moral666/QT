import SwiftUI

// =========================================================================== chat

/// Um elemento da lista do chat (mensagem, separador de dia, aviso ou "a carregar").
private enum ChatRow: Identifiable {
    case message(ChatMessage)
    case day(seq: Int64, ms: Int64)
    case loadingMore
    case encryption

    var id: String {
        switch self {
        case .message(let m): return "m\(m.seq)"
        case .day(let seq, _): return "day-\(seq)"
        case .loadingMore: return "more"
        case .encryption: return "e2e"
        }
    }
}

struct ChatScreen: View {
    let ui: UiState
    let contactId: String
    let contact: Contact?
    let onSend: (Contact, String) -> Void
    let onBack: () -> Void
    let onAdd: (String, String) -> Void
    let onSafety: (Contact) -> Void
    let onLoadOlder: () -> Void
    let onDeleteConversation: (String) -> Void
    let onDeleteContact: (String) -> Void

    @State private var deleteTarget: DeleteTarget?
    @State private var text = ""
    @State private var showAdd = false
    @FocusState private var inputFocused: Bool
    @Environment(\.chat) private var c

    /// Mesma ordem que o Android: da MAIS RECENTE para a mais antiga, com separadores de dia
    /// e o aviso de cifragem (ou "a carregar") no fim. A lista e desenhada "de pernas para o ar"
    /// (como o reverseLayout do Compose): fica colada ao fundo e "subir" percorre o passado.
    private var rows: [ChatRow] {
        let msgs = ui.chatMessages
        var out: [ChatRow] = []
        out.reserveCapacity(msgs.count + 8)
        for (i, m) in msgs.enumerated() {
            out.append(.message(m))
            let older = i + 1 < msgs.count ? msgs[i + 1] : nil
            if older == nil || dayKey(older!.localAtMs) != dayKey(m.localAtMs) {
                if older != nil || !ui.chatHasMore { out.append(.day(seq: m.seq, ms: m.localAtMs)) }
            }
        }
        out.append(ui.chatHasMore ? .loadingMore : .encryption)
        return out
    }

    /// Ao aparecer uma das 10 mensagens mais antigas carregadas, carrega a pagina seguinte.
    private var loadTriggerSeq: Int64 {
        let msgs = ui.chatMessages
        guard !msgs.isEmpty else { return Int64.min }
        return msgs[max(0, msgs.count - 10)].seq
    }

    var body: some View {
        let name = contact?.nickname ?? "Desconhecido"
        VStack(spacing: 0) {
            SealerTopBar(onBack: onBack, title: {
                HStack(spacing: 10) {
                    Avatar(name: name, seed: contactId, size: 38)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(name).font(.headline).lineLimit(1)
                        Text("cifrada ponta a ponta").font(.caption2).opacity(0.8)
                    }
                }
            }, actions: {
                if let contact {
                    TopBarIcon(systemName: "lock.fill", label: "Verificar segurança") { onSafety(contact) }
                }
                Menu {
                    Button("Eliminar conversa", systemImage: "trash") {
                        deleteTarget = .conversation(id: contactId, name: name)
                    }
                    .disabled(ui.chatMessages.isEmpty)
                    Button("Apagar contacto", systemImage: "person.crop.circle.badge.minus", role: .destructive) {
                        deleteTarget = .contact(id: contactId, name: name)
                    }
                } label: {
                    Image(systemName: "ellipsis").rotationEffect(.degrees(90))
                        .font(.system(size: 19, weight: .medium)).frame(width: 44, height: 44)
                }
                .accessibilityLabel("Mais opções")
            })

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 3) {
                        ForEach(rows) { row in
                            rowView(row)
                                .scaleEffect(x: 1, y: -1)   // desfaz a inversao da lista
                                .id(row.id)
                        }
                    }
                    .padding(.horizontal, 10).padding(.vertical, 8)
                }
                .scaleEffect(x: 1, y: -1)                   // lista invertida = reverseLayout
                .scrollIndicators(.hidden)
                .scrollDismissesKeyboard(.interactively)
                .background(c.chatBackground)
                .onChange(of: ui.chatMessages.first?.seq) { _, newest in
                    guard let newest else { return }
                    withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo("m\(newest)", anchor: .top) }
                }
            }

            if let contact {
                inputBar(contact)
            } else {
                VStack(spacing: 8) {
                    Text("Esta pessoa não está nos teus contactos.").font(.subheadline)
                    PrimaryButton(title: "Adicionar para responder", height: 44) { showAdd = true }
                        .fixedSize(horizontal: true, vertical: false)
                }
                .padding(16)
                .frame(maxWidth: .infinity)
                .background(c.surface)
            }
        }
        .background(c.chatBackground.ignoresSafeArea(edges: .bottom))
        .gesture(   // deslizar a partir da margem esquerda = voltar (gesto normal no iPhone)
            DragGesture(minimumDistance: 20).onEnded { g in
                if g.startLocation.x < 30 && g.translation.width > 80 && abs(g.translation.height) < 60 { onBack() }
            }
        )
        .sheet(isPresented: $showAdd) { AddContactSheet(onAdd: onAdd) }
        .deleteConfirmation($deleteTarget, onDeleteConversation: onDeleteConversation, onDeleteContact: onDeleteContact)
    }

    @ViewBuilder
    private func rowView(_ row: ChatRow) -> some View {
        switch row {
        case .message(let m):
            MessageBubble(m: m)
                .onAppear {
                    // Perto da mensagem mais antiga carregada: pede mais uma pagina (como no Android).
                    if ui.chatHasMore, m.seq <= loadTriggerSeq { onLoadOlder() }
                }
        case .day(_, let ms):
            DaySeparator(ms: ms)
        case .loadingMore:
            Text("A carregar mensagens anteriores…")
                .font(.caption2).frame(maxWidth: .infinity).padding(8)
                .onAppear(perform: onLoadOlder)
        case .encryption:
            EncryptionNotice()
        }
    }

    private func inputBar(_ contact: Contact) -> some View {
        HStack(alignment: .bottom, spacing: 6) {
            TextField("Mensagem", text: $text, axis: .vertical)
                .lineLimit(1...5)
                .focused($inputFocused)
                .padding(.horizontal, 16).padding(.vertical, 14)
                .background(RoundedRectangle(cornerRadius: 24).fill(c.surface))
            Button {
                let t = text
                if !t.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    onSend(contact, t)
                    text = ""
                }
            } label: {
                Image(systemName: "paperplane.fill").font(.system(size: 20))
                    .foregroundStyle(c.onPrimary)
                    .frame(width: 52, height: 52)
                    .background(Circle().fill(c.primary))
            }
            .accessibilityLabel("Enviar")
        }
        .padding(.horizontal, 8).padding(.vertical, 6)
        .background(c.chatBackground)
    }
}

struct EncryptionNotice: View {
    @Environment(\.chat) private var c
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "lock.fill").font(.system(size: 11))
            Text("As mensagens são cifradas ponta a ponta. Nem o servidor as consegue ler.")
                .font(.caption2).multilineTextAlignment(.center)
        }
        .foregroundStyle(c.onSecondaryContainer)
        .padding(.horizontal, 12).padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: 8).fill(c.secondaryContainer.opacity(0.7)))
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
    }
}

struct DaySeparator: View {
    let ms: Int64
    @Environment(\.chat) private var c
    var body: some View {
        Text(dayLabel(ms))
            .font(.caption2).foregroundStyle(c.onSurfaceVariant)
            .padding(.horizontal, 10).padding(.vertical, 3)
            .background(RoundedRectangle(cornerRadius: 8).fill(c.surface))
            .frame(maxWidth: .infinity)
            .padding(.vertical, 6)
    }
}

struct MessageBubble: View {
    let m: ChatMessage
    @Environment(\.chat) private var c

    var body: some View {
        let shape = UnevenRoundedRectangle(
            topLeadingRadius: 16,
            bottomLeadingRadius: m.outgoing ? 16 : 4,
            bottomTrailingRadius: m.outgoing ? 4 : 16,
            topTrailingRadius: 16
        )
        HStack {
            if m.outgoing { Spacer(minLength: 48) }
            VStack(alignment: .trailing, spacing: 2) {
                Text(m.text).font(.body).foregroundStyle(c.bubbleText)
                    .multilineTextAlignment(.leading)
                    .textSelection(.enabled)
                HStack(spacing: 4) {
                    Text(timeOf(m.localAtMs)).font(.system(size: 11)).foregroundStyle(c.bubbleMeta)
                    if m.outgoing {
                        Text(statusGlyph(m.status)).font(.system(size: 12, weight: .bold))
                            .foregroundStyle(m.status == .read ? c.readTick : c.bubbleMeta)
                    }
                }
            }
            .padding(.leading, 10).padding(.trailing, 8).padding(.top, 6).padding(.bottom, 4)
            .background(shape.fill(m.outgoing ? c.outgoingBubble : c.incomingBubble))
            .frame(maxWidth: 300, alignment: m.outgoing ? .trailing : .leading)
            if !m.outgoing { Spacer(minLength: 48) }
        }
    }
}
