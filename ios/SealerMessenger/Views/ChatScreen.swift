import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

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

/// Conversa a dois (`group == nil`) ou de grupo. Mesmo comportamento que o ChatScreen e o
/// GroupChatScreen do Android: bloquear, convite, reacoes, fotos, GIFs, ficheiros e voz.
struct ChatScreen: View {
    @ObservedObject var model: AppModel
    let ui: UiState
    let conversationId: String
    let group: GroupInfo?
    let onInfo: () -> Void
    let onCall: (Contact) -> Void
    /// Videochamada (so para contactos que disseram que tem video).
    var onVideoCall: (Contact) -> Void = { _ in }
    /// Chamada de grupo (0.9.64): `true` = videochamada. `nil`: sem os botoes.
    var onGroupCall: ((Bool) -> Void)? = nil
    /// Ha uma chamada a decorrer neste grupo em que nao estou (0.9.65): "Entrar na chamada".
    var onJoinOngoing: (() -> Void)? = nil

    @State private var deleteTarget: DeleteTarget?
    @State private var viewing: ChatMessage?
    @State private var reactingTo: ChatMessage?
    @State private var showAdd = false
    @State private var confirmBlock = false
    @State private var showReport = false
    @Environment(\.chat) private var c
    @Environment(\.layoutDirection) private var layoutDirection

    private var contact: Contact? { group == nil ? ui.contact(conversationId) : nil }
    private var blocked: Bool { group == nil && ui.blocked.contains(conversationId) }
    private var invite: Bool { group == nil && contact == nil && ui.invites.contains(conversationId) }

    /// Da MAIS RECENTE para a mais antiga, com separadores de dia e o aviso de cifragem (ou "a
    /// carregar") no fim. A lista e desenhada "de pernas para o ar" (como o reverseLayout do
    /// Compose): fica colada ao fundo e "subir" percorre o passado.
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

    private var loadTriggerSeq: Int64 {
        let msgs = ui.chatMessages
        guard !msgs.isEmpty else { return Int64.min }
        return msgs[max(0, msgs.count - 10)].seq
    }

    var body: some View {
        let name = group?.name ?? contact?.nickname ?? s("t_desconhecido")
        VStack(spacing: 0) {
            topBar(name)
            messages
            bottom(name)
        }
        .background(c.chatBackground.ignoresSafeArea(edges: .bottom))
        .gesture(   // deslizar a partir da margem esquerda = voltar (gesto normal no iPhone; em arabe, da direita)
            DragGesture(minimumDistance: 20).onEnded { g in
                let rtl = layoutDirection == .rightToLeft
                let w = UIScreen.main.bounds.width
                let fromEdge = rtl ? g.startLocation.x > w - 30 : g.startLocation.x < 30
                let dx = rtl ? -g.translation.width : g.translation.width
                if fromEdge && dx > 80 && abs(g.translation.height) < 60 { model.closeChat() }
            }
        )
        .fullScreenCover(item: $viewing) { m in
            if m.attachment?.fileName != nil {
                FileActionsView(m: m, load: model.attachmentBytes)
            } else {
                AttachmentViewer(m: m, load: model.attachmentBytes)
            }
        }
        .sheet(item: $reactingTo) { m in
            ReactionPicker(current: m.reactions.first(where: \.mine)?.emoji,
                           text: m.attachment == nil ? coreText(m.text) : nil,
                           canReact: group?.active ?? !blocked) { e in
                model.react(conversationId, m.messageId, e)
                reactingTo = nil
            }
        }
        .sheet(isPresented: $showAdd) {
            AddContactSheet { n, p in model.addContact(nickname: n, publicId: p) }
        }
        .alert(s("t_bloquear_2", name), isPresented: $confirmBlock) {
            Button(s("t_bloquear"), role: .destructive) { model.blockContact(conversationId) }
            Button(s("t_cancelar"), role: .cancel) {}
        } message: { Text(s("t_as_mensagens_desta_pessoa_deixam_de")) }
        .deleteConfirmation($deleteTarget, onDeleteConversation: model.deleteConversation, onDeleteContact: model.deleteContact)
        .sheet(isPresented: $showReport) {
            ReportSheet(name: name, msgs: ui.chatMessages) { ids, reason in
                showReport = false
                model.reportContact(conversationId, messageIds: ids, reason: reason)
            } onCancel: { showReport = false }
        }
    }

    @ViewBuilder
    private func topBar(_ name: String) -> some View {
        if let g = group {
            SealerTopBar(onBack: model.closeChat, title: {
                Button(action: onInfo) {
                    HStack(spacing: 10) {
                        Avatar(name: g.name, seed: g.groupId, size: 38)
                        VStack(alignment: .leading, spacing: 0) {
                            Text(g.name).font(.sTitleMedium).lineLimit(1)
                            Text(s("t_n_pessoas", g.members.count)).font(.sLabelSmall).opacity(0.8)
                        }
                    }
                }
                .buttonStyle(.plain)
            }, actions: {
                if let onJoinOngoing, g.active {
                    Button(action: onJoinOngoing) {
                        Label(s("gcall_join"), systemImage: "phone.fill").font(.sLabelLarge)
                            .foregroundStyle(Color(hex: 0x2E9E5B))
                    }
                    .buttonStyle(.plain)
                } else if let onGroupCall, g.active {
                    TopBarIcon(systemName: "video.fill", label: s("gcall_video_title")) { onGroupCall(true) }
                    TopBarIcon(systemName: "phone.fill", label: s("gcall_title")) { onGroupCall(false) }
                }
                TopBarIcon(systemName: "info.circle", label: s("t_informacao_do_grupo"), action: onInfo)
            })
        } else {
            let isOnline = contact != nil && ui.online.contains(conversationId)
            SealerTopBar(onBack: model.closeChat, title: {
                HStack(spacing: 10) {
                    Avatar(name: name, seed: conversationId, size: 38, online: isOnline)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(name).font(.sTitleMedium).lineLimit(1)
                        if isOnline {
                            Text(s("t_online")).font(.sLabelSmall).opacity(0.8)
                        }
                    }
                }
            }, actions: {
                if let contact, !blocked, !invite {
                    if ui.videoContacts.contains(conversationId) {
                        TopBarIcon(systemName: "video.fill", label: s("video_call_button")) { onVideoCall(contact) }
                    }
                    TopBarIcon(systemName: "phone.fill", label: s("t_ligar")) { onCall(contact) }
                }
                if let contact {
                    TopBarIcon(systemName: "lock.fill", label: s("t_verificar_seguranca")) { model.showSafetyNumber(contact) }
                }
                Menu {
                    if blocked {
                        Button(s("t_desbloquear_2"), systemImage: "checkmark") { model.unblockContact(conversationId) }
                    } else {
                        Button(s("t_bloquear"), systemImage: "hand.raised") { confirmBlock = true }
                    }
                    if reportsAvailable() {
                        Button(s("report"), systemImage: "exclamationmark.bubble") { showReport = true }
                    }
                    Button(s("t_eliminar_conversa_ou_contacto"), systemImage: "trash") {
                        deleteTarget = .choose(id: conversationId, name: name, hasHistory: !ui.chatMessages.isEmpty)
                    }
                } label: {
                    Image(systemName: "ellipsis").rotationEffect(.degrees(90))
                        .font(.system(size: 19, weight: .medium)).frame(width: 44, height: 44)
                }
                .accessibilityLabel(s("t_mais_opcoes"))
            })
        }
    }

    private var messages: some View {
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
    }

    @ViewBuilder
    private func rowView(_ row: ChatRow) -> some View {
        switch row {
        case .message(let m):
            let canReact = group?.active ?? !blocked
            MessageBubble(
                m: m, progress: ui.transfers[m.messageId], load: model.attachmentBytes,
                senderName: group != nil && !m.outgoing ? ui.memberName(m.sender) : nil,
                onOpen: { viewing = m },
                onLongPress: { if canReact || m.attachment == nil { reactingTo = m } }
            )
            .onAppear { if ui.chatHasMore, m.seq <= loadTriggerSeq { model.loadOlder() } }
        case .day(_, let ms):
            DaySeparator(ms: ms)
        case .loadingMore:
            Text(s("t_a_carregar_mensagens_anteriores"))
                .font(.sLabelSmall).frame(maxWidth: .infinity).padding(8)
                .onAppear(perform: model.loadOlder)
        case .encryption:
            EncryptionNotice()
        }
    }

    @ViewBuilder
    private func bottom(_ name: String) -> some View {
        if let g = group {
            if !g.active {
                Text(s("t_ja_nao_fazes_parte_deste_grupo")).font(.sBodyMedium).multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity).padding(16).background(c.surface)
            } else {
                composer
            }
        } else if blocked {
            VStack(spacing: 8) {
                Text(s("t_bloqueaste_esta_pessoa_as_mensagens_dela")).font(.sBodyMedium).multilineTextAlignment(.center)
                Button(s("t_desbloquear_2")) { model.unblockContact(conversationId) }.font(.sLabelLarge)
            }
            .frame(maxWidth: .infinity).padding(16).background(c.surface)
        } else if invite {
            InviteCard(onAccept: { nick in model.acceptInvite(conversationId, nickname: nick) },
                       onDecline: { model.declineInvite(conversationId) },
                       onBlock: { confirmBlock = true })
        } else if contact == nil {
            VStack(spacing: 8) {
                Text(s("t_esta_pessoa_nao_esta_nos_teus")).font(.sBodyMedium)
                PrimaryButton(title: s("t_adicionar_para_responder"), height: 44) { showAdd = true }
                    .fixedSize(horizontal: true, vertical: false)
            }
            .frame(maxWidth: .infinity).padding(16).background(c.surface)
        } else {
            composer
        }
    }

    private var composer: some View {
        ChatComposer(
            onSendText: { t in
                if let g = group { model.sendGroup(g.groupId, t) } else if let ct = contact { model.send(ct, t) }
            },
            onSendImage: { data, original, pasted in model.sendImage(to: conversationId, data: data, original: original, pasted: pasted) },
            onSendFile: { url in model.sendFile(to: conversationId, url: url) },
            onSendVoice: { url, ms in model.sendVoice(to: conversationId, file: url, durationMs: ms) },
            restoreText: model.ui.unsentText.flatMap { $0.conversationId == conversationId ? $0.text : nil },
            onRestored: model.draftRestored
        )
    }
}

/// Novo contacto (nome e ID, ou ler o QR), a partir de uma conversa com quem nao esta nos contactos.
struct AddContactSheet: View {
    let onAdd: (String, String) -> Void
    @State private var nick = ""
    @State private var pid = ""
    @State private var scanning = false
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(s("t_novo_contacto")).font(Jakarta.font(22, .bold))
            OutlinedField(label: s("t_nome"), text: $nick, singleLine: true)
            OutlinedField(label: s("t_id_qt1"), text: $pid, monospaced: true)
            OutlineButton(title: s("t_ler_codigo_qr"), systemImage: "qrcode.viewfinder", height: 46) { scanning = true }
            Text(s("t_recebe_o_id_diretamente_da_pessoa")).font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
            HStack {
                Spacer()
                Button(s("t_cancelar")) { dismiss() }
                Button(s("t_adicionar")) { onAdd(nick, pid); dismiss() }
                    .disabled(pid.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .padding(.leading, 16)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.medium, .large])
        .presentationBackground(c.surface)
        .fullScreenCover(isPresented: $scanning) { QRScannerSheet { pid = $0 } }
    }
}

/// Alguem que nao esta nos contactos escreveu-me e mandou o seu ID (convite): aceitar (sem
/// escrever o ID), recusar (apaga a conversa) ou bloquear.
struct InviteCard: View {
    let onAccept: (String) -> Void
    let onDecline: () -> Void
    let onBlock: () -> Void
    @State private var nick = ""
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(s("t_esta_pessoa_quer_falar_contigo")).font(Jakarta.font(14, .semibold))
            Text(s("t_ainda_nao_esta_nos_teus_contactos")).font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
            OutlinedField(label: s("t_nome_opcional"), text: Binding(get: { nick }, set: { nick = String($0.prefix(40)) }), singleLine: true)
            HStack(spacing: 8) {
                Button(s("t_aceitar")) { onAccept(nick) }
                    .font(Jakarta.font(14, .semibold)).foregroundStyle(c.onPrimary)
                    .padding(.horizontal, 18).padding(.vertical, 10).background(Capsule().fill(c.primary))
                Button(s("t_recusar"), action: onDecline)
                    .font(Jakarta.font(14, .semibold))
                    .padding(.horizontal, 18).padding(.vertical, 10).overlay(Capsule().stroke(c.outlineVariant))
                Spacer()
                Button(s("t_bloquear"), action: onBlock).font(Jakarta.font(14, .semibold)).foregroundStyle(c.error)
            }
            .buttonStyle(.plain)
        }
        .padding(16)
        .background(c.surface)
    }
}

struct EncryptionNotice: View {
    @Environment(\.chat) private var c
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "lock.fill").font(.system(size: 11))
            Text(s("t_as_mensagens_sao_cifradas_ponta_a")).font(.sLabelSmall).multilineTextAlignment(.center)
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
            .font(.sLabelSmall).foregroundStyle(c.onSurfaceVariant)
            .padding(.horizontal, 10).padding(.vertical, 3)
            .background(RoundedRectangle(cornerRadius: 8).fill(c.surface))
            .frame(maxWidth: .infinity)
            .padding(.vertical, 6)
    }
}

struct MessageBubble: View {
    let m: ChatMessage
    var progress: Double? = nil
    var load: (ChatMessage) async -> Data? = { _ in nil }
    /// Nos grupos: quem escreveu (so nas recebidas).
    var senderName: String? = nil
    var onOpen: () -> Void = {}
    var onLongPress: () -> Void = {}
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
            VStack(alignment: m.outgoing ? .trailing : .leading, spacing: 0) {
                VStack(alignment: .trailing, spacing: 2) {
                    if let senderName {
                        Text(senderName).font(Jakarta.font(12, .semibold)).foregroundStyle(c.primary).lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.leading, m.attachment != nil ? 6 : 0)
                    }
                    content
                    HStack(spacing: 4) {
                        Text(timeOf(m.localAtMs)).font(.system(size: 11)).foregroundStyle(c.bubbleMeta)
                        if m.outgoing {
                            Text(statusGlyph(m.status)).font(.system(size: 12, weight: .bold))
                                .foregroundStyle(m.status == .read ? c.readTick : c.bubbleMeta)
                        }
                    }
                }
                .padding(.leading, m.attachment == nil ? 10 : 4).padding(.trailing, m.attachment == nil ? 8 : 4)
                .padding(.top, m.attachment == nil ? 6 : 4).padding(.bottom, 4)
                .background(shape.fill(m.outgoing ? c.outgoingBubble : c.incomingBubble))
                .contentShape(shape)
                .onLongPressGesture(minimumDuration: 0.4, perform: onLongPress)
                // Reacoes por baixo da bolha (tocar abre o seletor, como o toque longo).
                if !m.reactions.isEmpty {
                    Button(action: onLongPress) {
                        HStack(spacing: 2) { ForEach(Array(m.reactions.enumerated()), id: \.offset) { _, r in Text(r.emoji).font(.system(size: 15)) } }
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(RoundedRectangle(cornerRadius: 12).fill(c.surface))
                    }
                    .buttonStyle(.plain)
                    .padding(.horizontal, 6)
                    .offset(y: -4)
                    .accessibilityLabel(s("t_reagir"))
                }
            }
            .frame(maxWidth: 300, alignment: m.outgoing ? .trailing : .leading)
            if !m.outgoing { Spacer(minLength: 48) }
        }
    }

    @ViewBuilder
    private var content: some View {
        if let att = m.attachment {
            if att.fileName != nil {
                FileContent(m: m, att: att, progress: progress, onOpen: onOpen)
            } else if att.mime == Voice.mime {
                VoiceContent(m: m, att: att, progress: progress, load: load)
            } else {
                AttachmentContent(m: m, att: att, progress: progress, load: load, onOpen: onOpen)
            }
        } else {
            LinkedText(text: coreText(m.text), color: c.bubbleText).font(.sBodyLarge).foregroundStyle(c.bubbleText)
                .multilineTextAlignment(.leading)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Emojis do seletor de reacoes (os mesmos do Android).
private let quickReactions = ["👍", "❤️", "😂", "😮", "😢", "🙏"]

/// Escolher a reacao a uma mensagem (ou retirar a minha).
/// Toque longo numa mensagem: reagir (ou retirar a minha) e, nas de texto, copiar a mensagem ou
/// um link dela (0.9.29, igual ao Android). `text` = nil nos anexos; `canReact` = false com o
/// contacto bloqueado ou fora do grupo.
struct ReactionPicker: View {
    let current: String?
    var text: String? = nil
    var canReact: Bool = true
    let onPick: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    private var links: [String] { text.map { Array(Links.urls($0).prefix(3)) } ?? [] }

    var body: some View {
        VStack(spacing: 16) {
            if canReact {
                Text(s("t_reagir")).font(Jakarta.font(20, .bold))
                HStack {
                    ForEach(quickReactions, id: \.self) { e in
                        Button { onPick(e) } label: {
                            Text(e).font(.system(size: 28)).frame(width: 48, height: 48)
                                .background(Circle().fill(e == current ? c.secondaryContainer : .clear))
                        }
                        .buttonStyle(.plain)
                        .frame(maxWidth: .infinity)
                    }
                }
            }
            if let text {
                VStack(alignment: .leading, spacing: 10) {
                    Button(s("msg_copy")) { copy(text) }
                    ForEach(links, id: \.self) { l in
                        Button("\(s("msg_copy_link")): \(Links.open(l).map(Links.host) ?? l)") { copy(l) }.lineLimit(1)
                    }
                }
                .font(Jakarta.font(15, .semibold))
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            HStack {
                if canReact && current != nil { Button(s("t_retirar_a_minha")) { onPick("") } }
                Spacer()
                Button(s("t_cancelar")) { dismiss() }
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.height(height)])
        .presentationBackground(c.surface)
    }
}

extension ReactionPicker {
    private var height: CGFloat {
        (canReact ? 160 : 70) + (text == nil ? 0 : 40 + CGFloat(links.count) * 34)
    }

    private func copy(_ t: String) {
        UIPasteboard.general.string = t
        dismiss()
    }
}

// =========================================================================== denunciar

/// Denunciar (0.9.18), igual ao ReportDialog do Android: motivo + mensagens a incluir (vem
/// marcadas as 10 mais recentes da outra pessoa; maximo 20). Seguem cifradas so para a
/// equipa do Sealer e o contacto fica bloqueado. Nada segue sem a pessoa confirmar.
struct ReportSheet: View {
    let name: String
    let shown: [ChatMessage]
    let onConfirm: ([String], String) -> Void
    let onCancel: () -> Void
    @State private var picked: Set<String>
    @State private var reason = "spam"

    private static let maxPicked = 20
    private let reasons: [(String, String)] = [
        ("spam", s("report_reason_spam")),
        ("assedio", s("report_reason_harassment")),
        ("ilegal", s("report_reason_illegal")),
        ("outro", s("report_reason_other")),
    ]

    init(name: String, msgs: [ChatMessage], onConfirm: @escaping ([String], String) -> Void, onCancel: @escaping () -> Void) {
        self.name = name
        let shown = Array(msgs.filter { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }.prefix(30))
        self.shown = shown
        self.onConfirm = onConfirm
        self.onCancel = onCancel
        _picked = State(initialValue: Set(shown.filter { !$0.outgoing }.prefix(10).map(\.messageId)))
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Text(s("report_explain", name)).font(.footnote)
                }
                Section(s("report_reason")) {
                    Picker(s("report_reason"), selection: $reason) {
                        ForEach(reasons, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()
                }
                Section(s("report_messages", picked.count, Self.maxPicked)) {
                    if shown.isEmpty { Text(s("report_no_messages")).font(.footnote) }
                    ForEach(shown, id: \.messageId) { m in
                        Button {
                            if picked.contains(m.messageId) { picked.remove(m.messageId) }
                            else if picked.count < Self.maxPicked { picked.insert(m.messageId) }
                        } label: {
                            HStack {
                                Image(systemName: picked.contains(m.messageId) ? "checkmark.square.fill" : "square")
                                VStack(alignment: .leading) {
                                    Text(m.outgoing ? s("report_from_me") : name).font(.caption).lineLimit(1)
                                    Text(m.text).font(.footnote).lineLimit(2)
                                }
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .navigationTitle(s("report_title", name))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(s("t_cancelar"), action: onCancel) }
                ToolbarItem(placement: .confirmationAction) {
                    Button(s("report_confirm"), role: .destructive) {
                        onConfirm(shown.map(\.messageId).filter { picked.contains($0) }, reason)
                    }
                }
            }
        }
    }
}
