import SwiftUI

// ===================================================================== conversas

/// Ecra inicial (design 0.9.11): cabecalho com o logo a rodar, a minha foto (engrenagem =
/// Definicoes), "Conversas" e o meu ID; "Online agora"; a lista; o "+" abre o Novo contacto.
struct HomeScreen: View {
    let ui: UiState
    let onOpen: (String) -> Void
    let onMyId: () -> Void
    let onSettings: () -> Void
    let onAdd: () -> Void
    let onDeleteConversation: (String) -> Void
    let onDeleteContact: (String) -> Void
    let onDeleteGroup: (String) -> Void
    @State private var deleteTarget: DeleteTarget?
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            HomeHeader(ui: ui, onSettings: onSettings, onMyId: onMyId)
            let withHistory = Set(ui.conversations.map(\.contactId))
            let idle = ui.contacts.filter { !withHistory.contains($0.contactId) }
            let idleGroups = ui.groups.filter { !withHistory.contains($0.conversationId) }
            if ui.conversations.isEmpty && idle.isEmpty && idleGroups.isEmpty {
                EmptyConversations(onMyId: onMyId)
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        let onlineNow = ui.contacts.filter { ui.online.contains($0.contactId) && !ui.blocked.contains($0.contactId) }
                        if !onlineNow.isEmpty { OnlineNowStrip(contacts: onlineNow, onOpen: onOpen) }
                        ForEach(ui.conversations, id: \.contactId) { conv in
                            row(conv)
                        }
                        ForEach(idleGroups) { g in
                            ConversationRow(name: g.name, seed: g.groupId, conv: nil) { onOpen(g.conversationId) }
                                .contextMenu { groupMenu(g) }
                        }
                        ForEach(idle) { contact in
                            ConversationRow(name: contact.nickname, seed: contact.contactId, conv: nil,
                                            blocked: ui.blocked.contains(contact.contactId),
                                            online: ui.online.contains(contact.contactId)) { onOpen(contact.contactId) }
                                .contextMenu { contactMenu(contact.contactId, contact.nickname, hasHistory: false) }
                        }
                    }
                    .padding(.top, 4)
                    .padding(.bottom, 96)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(c.background.ignoresSafeArea())
        .overlay(alignment: .bottomTrailing) {
            Button(action: onAdd) {
                Image(systemName: "plus").font(.system(size: 24, weight: .semibold)).foregroundStyle(c.onPrimary)
                    .frame(width: 58, height: 58)
                    .background(RoundedRectangle(cornerRadius: 20).fill(c.primary))
                    .shadow(color: .black.opacity(0.25), radius: 5, y: 2)
            }
            .accessibilityLabel(s("t_novo_contacto"))
            .padding(18)
        }
        .deleteConfirmation($deleteTarget, onDeleteConversation: onDeleteConversation,
                            onDeleteContact: onDeleteContact, onDeleteGroup: onDeleteGroup)
    }

    @ViewBuilder
    private func row(_ conv: ConversationSummary) -> some View {
        if let g = ui.groupOf(conv.contactId) {
            let last = conv.lastMessage
            ConversationRow(name: g.name, seed: g.groupId, conv: conv,
                            previewPrefix: last.outgoing ? nil : ui.memberName(last.sender)) { onOpen(conv.contactId) }
                .contextMenu { groupMenu(g) }
        } else {
            let name = ui.contact(conv.contactId)?.nickname
            ConversationRow(name: name ?? s("t_desconhecido"), seed: conv.contactId, conv: conv,
                            blocked: ui.blocked.contains(conv.contactId),
                            invite: name == nil && ui.invites.contains(conv.contactId),
                            online: name != nil && ui.online.contains(conv.contactId)) { onOpen(conv.contactId) }
                .contextMenu { contactMenu(conv.contactId, name ?? s("t_desconhecido"), hasHistory: true) }
        }
    }

    @ViewBuilder
    private func contactMenu(_ id: String, _ name: String, hasHistory: Bool) -> some View {
        if hasHistory {
            Button(s("t_eliminar_conversa"), systemImage: "trash") { deleteTarget = .conversation(id: id, name: name) }
        }
        Button(s("t_apagar_contacto"), systemImage: "person.crop.circle.badge.minus", role: .destructive) {
            deleteTarget = .contact(id: id, name: name)
        }
    }

    @ViewBuilder
    private func groupMenu(_ g: GroupInfo) -> some View {
        Button(s("t_apagar", g.name), systemImage: "trash", role: .destructive) { deleteTarget = .group(id: g.groupId, name: g.name) }
    }
}

/// Cabecalho do ecra inicial: o logo a rodar como um piao (so aqui), "Sealer", a minha foto com
/// a engrenagem das Definicoes, o titulo e o meu ID. Sem estado da ligacao (decisao do Nuno).
struct HomeHeader: View {
    let ui: UiState
    let onSettings: () -> Void
    let onMyId: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        GalaxyHeader(curve: 76) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .top) {
                    HStack(spacing: 10) {
                        BrandMark(size: 46)
                        Text("Sealer").font(.custom("Manrope-Bold", size: 27)).foregroundStyle(c.onTopBar)
                    }
                    .padding(.top, 10)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("Sealer")
                    Spacer()
                    MyAvatar(photo: ui.myPhoto, badge: "gearshape.fill", label: s("t_definicoes"), action: onSettings)
                }
                Spacer().frame(height: 10)
                Text(s("t_conversas")).font(Jakarta.font(29, .heavy, relativeTo: .largeTitle)).foregroundStyle(c.onTopBar)
                Spacer().frame(height: 8)
                IdChip(publicId: ui.publicId, action: onMyId)
            }
            .padding(.horizontal, 20).padding(.top, 12).padding(.bottom, 34)
        }
    }
}

struct EmptyConversations: View {
    let onMyId: () -> Void
    @Environment(\.chat) private var c
    var body: some View {
        VStack(spacing: 0) {
            BrandLogo(size: 64)
            Spacer().frame(height: 16)
            Text(s("t_ainda_nao_tens_conversas")).font(.sTitleMedium)
            Spacer().frame(height: 8)
            Text(s("t_partilha_o_teu_id_com_quem"))
                .font(.sBodyMedium).multilineTextAlignment(.center).foregroundStyle(c.onSurfaceVariant)
            Spacer().frame(height: 16)
            Button(action: onMyId) {
                Text(s("t_partilhar_o_meu_id")).font(.sLabelLarge)
                    .padding(.horizontal, 24).padding(.vertical, 10)
                    .overlay(Capsule().stroke(c.onSurfaceVariant.opacity(0.6), lineWidth: 1))
            }
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// "Online agora": os contactos com a app aberta, numa fila no topo da lista.
struct OnlineNowStrip: View {
    let contacts: [Contact]
    let onOpen: (String) -> Void
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(s("t_online_agora")).font(.sLabelMedium).foregroundStyle(c.onSurfaceVariant).padding(.horizontal, 16)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 12) {
                    ForEach(contacts) { ct in
                        Button { onOpen(ct.contactId) } label: {
                            VStack(spacing: 4) {
                                Avatar(name: ct.nickname, seed: ct.contactId, size: 48, online: true)
                                Text(ct.nickname).font(.sLabelSmall).lineLimit(1)
                            }
                            .frame(width: 64)
                            .padding(4)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 12).padding(.vertical, 8)
            }
            Rectangle().fill(c.outlineVariant).frame(height: 0.5)
        }
        .padding(.top, 10)
    }
}

struct ConversationRow: View {
    let name: String
    let seed: String
    let conv: ConversationSummary?
    var blocked = false
    var invite = false
    /// Nos grupos: quem escreveu a ultima mensagem recebida ("Ana: ...").
    var previewPrefix: String?
    var online = false
    let onClick: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        let last = conv?.lastMessage
        let unread = conv?.unread ?? 0
        Button(action: onClick) {
            HStack(spacing: 14) {
                Avatar(name: name, seed: seed, size: 52, online: online)
                VStack(spacing: 0) {
                    HStack(alignment: .center, spacing: 10) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(name).font(Jakarta.font(17, .bold)).lineLimit(1)
                            Text(preview(last))
                                .font(unread > 0 ? Jakarta.font(14, .semibold) : .sBodyMedium)
                                .foregroundStyle(unread > 0 ? c.onSurface : c.onSurfaceVariant)
                                .lineLimit(1)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        VStack(alignment: .trailing, spacing: 6) {
                            if let last {
                                Text(shortTime(last.localAtMs)).font(Jakarta.font(11, .semibold))
                                    .foregroundStyle(unread > 0 ? c.primary : c.onSurfaceVariant)
                            }
                            if unread > 0 {
                                Text(String(unread)).font(Jakarta.font(12, .bold)).foregroundStyle(c.onPrimary)
                                    .padding(.horizontal, 7).padding(.vertical, 2)
                                    .background(Capsule().fill(c.primary))
                            } else {
                                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(c.onSurfaceVariant)
                            }
                        }
                    }
                    .padding(.vertical, 12).padding(.trailing, 16)
                    Rectangle().fill(c.outlineVariant).frame(height: 0.5)
                }
            }
            .padding(.leading, 18)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func preview(_ last: ChatMessage?) -> String {
        if blocked { return s("t_bloqueado") }
        if invite { return s("t_quer_falar_contigo_toca_para_aceitar") }
        guard let last else { return s("t_toca_para_comecar_a_conversar") }
        if last.outgoing { return statusGlyph(last.status) + "  " + coreText(last.text) }
        if let p = previewPrefix { return "\(p): \(coreText(last.text))" }
        return coreText(last.text)
    }
}
