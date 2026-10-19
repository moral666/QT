import SwiftUI

/// Limites dos grupos (iguais aos do Rust, ffi/src/groups.rs).
let groupMaxMembers = 10
let groupMaxAdmins = 3

/// NOVO GRUPO: nome + ate 9 contactos (os dois opcionais).
struct NewGroupSheet: View {
    let ui: UiState
    let onCreate: (String, [Contact]) -> Void
    @State private var name = ""
    @State private var chosen = Set<String>()
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        let choices = ui.contacts.filter { !ui.blocked.contains($0.contactId) }
        let full = chosen.count >= groupMaxMembers - 1
        VStack(alignment: .leading, spacing: 10) {
            Text(s("t_novo_grupo")).font(Jakarta.font(22, .bold))
            OutlinedField(label: s("t_nome_do_grupo_opcional"), text: $name, singleLine: true)
            Text(s("t_pessoas_opcional_ate_escolhidas_podes_acrescen", groupMaxMembers - 1, chosen.count))
                .font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
            if choices.isEmpty { Text(s("t_ainda_nao_tens_contactos_o_grupo")).font(.sBodyMedium) }
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(choices) { ct in
                        let on = chosen.contains(ct.contactId)
                        Button {
                            if on { chosen.remove(ct.contactId) } else if !full { chosen.insert(ct.contactId) }
                        } label: {
                            HStack(spacing: 10) {
                                Image(systemName: on ? "checkmark.square.fill" : "square").font(.system(size: 20))
                                    .foregroundStyle(on || !full ? c.primary : c.outlineVariant)
                                Avatar(name: ct.nickname, seed: ct.contactId, size: 32)
                                Text(ct.nickname).lineLimit(1)
                                Spacer()
                            }
                            .padding(.vertical, 6)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            HStack {
                Spacer()
                Button(s("t_cancelar")) { dismiss() }
                Button(s("t_criar")) {
                    onCreate(name.trimmingCharacters(in: .whitespaces), choices.filter { chosen.contains($0.contactId) })
                }
                .padding(.leading, 16)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.large])
        .presentationBackground(c.surface)
    }
}

/// Informacao do grupo: membros e administradores. Os administradores (ate 3) mudam o nome,
/// acrescentam e retiram pessoas e escolhem os outros administradores.
struct GroupInfoScreen: View {
    let ui: UiState
    let group: GroupInfo
    let onBack: () -> Void
    let onUpdate: (String, [GroupMemberInput]) -> Void
    let onLeave: () -> Void
    let onDelete: () -> Void
    let onAddContact: (String, String) -> Void
    @State private var renaming = false
    @State private var newName = ""
    @State private var adding = false
    @State private var memberMenu: GroupMember?
    @State private var saveContact: GroupMember?
    @State private var saveNick = ""
    @State private var confirmLeave = false
    @State private var confirmDelete = false
    @Environment(\.chat) private var c

    private var admins: Int { group.members.filter(\.admin).count }
    private func current() -> [GroupMemberInput] { group.members.map { GroupMemberInput(publicId: $0.publicId, admin: $0.admin) } }
    private func isContact(_ m: GroupMember) -> Bool { m.me || ui.contact(m.contactId) != nil }

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(onBack: onBack) { Text(s("t_grupo")) }
            ScrollView {
                VStack(spacing: 0) {
                    VStack(spacing: 6) {
                        Avatar(name: group.name, seed: group.groupId, size: 72)
                        Text(group.name).font(Jakarta.font(22, .semibold)).multilineTextAlignment(.center).padding(.top, 6)
                        Text(s("t_de_pessoas", group.members.count, groupMaxMembers)).font(.sBodyMedium).foregroundStyle(c.onSurfaceVariant)
                        if group.iAmAdmin {
                            Button(s("t_mudar_o_nome")) { newName = group.name; renaming = true }.font(.sLabelLarge)
                        }
                        if !group.active {
                            Text(s("t_ja_nao_fazes_parte_deste_grupo")).font(.sBodyMedium).foregroundStyle(c.error)
                        }
                    }
                    .frame(maxWidth: .infinity).padding(24)
                    Rectangle().fill(c.outlineVariant).frame(height: 0.5)
                    if group.iAmAdmin && group.members.count < groupMaxMembers {
                        Button(s("t_adicionar_pessoas")) { adding = true }.font(.sLabelLarge)
                            .frame(maxWidth: .infinity, alignment: .leading).padding(16)
                    }
                    ForEach(group.members, id: \.contactId) { m in
                        let n = ui.memberName(m.contactId)
                        Button { memberMenu = m } label: {
                            HStack(spacing: 12) {
                                Avatar(name: n, seed: m.contactId, size: 40, online: ui.online.contains(m.contactId))
                                Text(n).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                                if m.admin {
                                    Text(s("t_administrador")).font(Jakarta.font(12, .medium)).foregroundStyle(c.onSecondaryContainer)
                                        .padding(.horizontal, 8).padding(.vertical, 2)
                                        .background(RoundedRectangle(cornerRadius: 8).fill(c.secondaryContainer))
                                }
                            }
                            .padding(.horizontal, 16).padding(.vertical, 10)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(m.me || !(group.iAmAdmin || !isContact(m)))
                    }
                    Rectangle().fill(c.outlineVariant).frame(height: 0.5).padding(.vertical, 8)
                    VStack(alignment: .leading, spacing: 4) {
                        if group.active {
                            Button(s("t_sair_do_grupo")) { confirmLeave = true }.foregroundStyle(c.error).padding(.vertical, 8)
                        }
                        Button(s("t_apagar_grupo_deste_telemovel")) { confirmDelete = true }.foregroundStyle(c.error).padding(.vertical, 8)
                    }
                    .font(.sLabelLarge)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 16).padding(.bottom, 24)
                }
            }
        }
        .background(c.background.ignoresSafeArea())
        .confirmationDialog(memberMenu.map { ui.memberName($0.contactId) } ?? "",
                            isPresented: Binding(get: { memberMenu != nil }, set: { if !$0 { memberMenu = nil } }),
                            titleVisibility: .visible, presenting: memberMenu) { m in
            if !isContact(m) {
                Button(s("t_guardar_nos_contactos")) { saveNick = ""; saveContact = m }
            }
            if group.iAmAdmin {
                if m.admin {
                    Button(s("t_deixar_de_ser_administrador")) {
                        onUpdate(group.name, current().map { $0.publicId == m.publicId ? GroupMemberInput(publicId: $0.publicId, admin: false) : $0 })
                    }
                } else if admins < groupMaxAdmins {
                    Button(s("t_tornar_administrador")) {
                        onUpdate(group.name, current().map { $0.publicId == m.publicId ? GroupMemberInput(publicId: $0.publicId, admin: true) : $0 })
                    }
                }
                Button(s("t_retirar_do_grupo"), role: .destructive) {
                    onUpdate(group.name, current().filter { $0.publicId != m.publicId })
                }
            }
            Button(s("t_fechar"), role: .cancel) {}
        }
        .alert(s("t_guardar_nos_contactos"), isPresented: Binding(get: { saveContact != nil }, set: { if !$0 { saveContact = nil } })) {
            TextField(s("t_nome"), text: $saveNick)
            Button(s("t_guardar")) {
                if let m = saveContact, !saveNick.trimmingCharacters(in: .whitespaces).isEmpty {
                    onAddContact(saveNick.trimmingCharacters(in: .whitespaces), m.publicId)
                }
                saveContact = nil
            }
            Button(s("t_cancelar"), role: .cancel) { saveContact = nil }
        }
        .alert(s("t_nome_do_grupo"), isPresented: $renaming) {
            TextField(s("t_nome_do_grupo"), text: $newName)
            Button(s("t_guardar")) { onUpdate(newName.trimmingCharacters(in: .whitespaces), current()) }
            Button(s("t_cancelar"), role: .cancel) {}
        }
        .alert(s("t_sair_de", group.name), isPresented: $confirmLeave) {
            Button(s("t_sair"), role: .destructive, action: onLeave)
            Button(s("t_cancelar"), role: .cancel) {}
        } message: { Text(s("t_deixas_de_receber_as_mensagens_do")) }
        .alert(s("t_apagar", group.name), isPresented: $confirmDelete) {
            Button(s("t_apagar_2"), role: .destructive, action: onDelete)
            Button(s("t_cancelar"), role: .cancel) {}
        } message: { Text(s("t_sais_do_grupo_se_ainda_estiveres")) }
        .sheet(isPresented: $adding) {
            AddMembersSheet(ui: ui, group: group) { add in
                adding = false
                onUpdate(group.name, current() + add.map { GroupMemberInput(publicId: $0.publicId, admin: false) })
            }
        }
    }
}

struct AddMembersSheet: View {
    let ui: UiState
    let group: GroupInfo
    let onAdd: ([Contact]) -> Void
    @State private var chosen = Set<String>()
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        let inGroup = Set(group.members.map(\.contactId))
        let choices = ui.contacts.filter { !inGroup.contains($0.contactId) && !ui.blocked.contains($0.contactId) }
        let room = groupMaxMembers - group.members.count
        VStack(alignment: .leading, spacing: 10) {
            Text(s("t_adicionar_pessoas_2")).font(Jakarta.font(22, .bold))
            Text(s("t_ainda_cabem_escolhidos", room, chosen.count)).font(.sBodySmall)
            if choices.isEmpty { Text(s("t_nao_ha_contactos_para_adicionar")).font(.sBodyMedium) }
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(choices) { ct in
                        let on = chosen.contains(ct.contactId)
                        let can = on || chosen.count < room
                        Button {
                            if on { chosen.remove(ct.contactId) } else if can { chosen.insert(ct.contactId) }
                        } label: {
                            HStack(spacing: 10) {
                                Image(systemName: on ? "checkmark.square.fill" : "square").font(.system(size: 20))
                                    .foregroundStyle(can ? c.primary : c.outlineVariant)
                                Text(ct.nickname).lineLimit(1)
                                Spacer()
                            }
                            .padding(.vertical, 6)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            HStack {
                Spacer()
                Button(s("t_cancelar")) { dismiss() }
                Button(s("t_adicionar")) { onAdd(choices.filter { chosen.contains($0.contactId) }) }
                    .disabled(chosen.isEmpty).padding(.leading, 16)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.medium, .large])
        .presentationBackground(c.surface)
    }
}
