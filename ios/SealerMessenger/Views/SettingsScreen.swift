import SwiftUI
import UIKit

// ====================================================================== definicoes

/// Ecra Definicoes (0.9.11): tudo o que estava no menu de tres pontos, mais a versao instalada.
struct ProfileSettingsScreen: View {
    let ui: UiState
    let onBack: () -> Void
    let onProfilePhoto: () -> Void
    let onMyId: () -> Void
    let onLockSettings: () -> Void
    let onLockNow: () -> Void
    let onRecovery: () -> Void
    let onPremium: () -> Void
    let onUnblock: (String) -> Void
    let onPresence: (Bool) -> Void
    let onReadReceipts: (Bool) -> Void
    let onPush: (Bool) -> Void
    let onLanguage: () -> Void
    let onRelay: () -> Void
    let onWipeAll: () -> Void
    @State private var showBlocked = false
    @State private var showWipe = false
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            GalaxyHeader(curve: 76) {
                VStack(alignment: .leading, spacing: 0) {
                    HStack(alignment: .top) {
                        HStack(spacing: 2) {
                            Button(action: onBack) {
                                Image(systemName: "chevron.left").font(.system(size: 20, weight: .semibold)).frame(width: 44, height: 44)
                            }
                            .accessibilityLabel(s("t_voltar"))
                            Text(s("t_definicoes")).font(Jakarta.font(24, .heavy))
                        }
                        .padding(.top, 6)
                        Spacer()
                        MyAvatar(photo: ui.myPhoto, ring: ui.myBadge, badge: "pencil", label: s("t_foto_de_perfil"), action: onProfilePhoto)
                    }
                    Spacer().frame(height: 12)
                    IdChip(publicId: ui.publicId, action: onMyId).padding(.leading, 16)
                }
                .foregroundStyle(c.onTopBar)
                .tint(c.onTopBar)
                .padding(.leading, 4).padding(.trailing, 20).padding(.top, 8).padding(.bottom, 34)
            }
            ScrollView {
                VStack(spacing: 0) {
                    SectionLabel(text: s("t_sec_conta"))
                    SettingsRow(icon: "face.smiling", title: s("t_foto_de_perfil"), action: onProfilePhoto)
                    SettingsRow(icon: "qrcode", title: s("t_o_meu_id"), action: onMyId)
                    SettingsRow(icon: "key.fill", title: s("recovery_title"), action: onRecovery)
                    SettingsRow(icon: "star.fill", title: ui.premium ? s("premium_title_on") : s("premium_title"), action: onPremium)
                    SettingsRow(icon: "lock.fill", title: s("t_bloqueio_da_app"),
                                divider: ui.lockEnabled || !ui.blocked.isEmpty, action: onLockSettings)
                    if ui.lockEnabled {
                        SettingsRow(icon: "lock.rotation", title: s("t_bloquear_agora"), divider: !ui.blocked.isEmpty, action: onLockNow)
                    }
                    if !ui.blocked.isEmpty {
                        SettingsRow(icon: "hand.raised.fill", title: s("t_contactos_bloqueados"), divider: false, action: { showBlocked = true })
                    }

                    SectionLabel(text: s("t_sec_privacidade"))
                    SettingsSwitch(icon: "person.fill", title: s("t_mostrar_quando_estou_online"), isOn: ui.presenceOn, onChange: onPresence)
                    SettingsSwitch(icon: "checkmark", title: s("t_confirmacoes_de_leitura"), isOn: ui.readReceipts,
                                   divider: false, onChange: onReadReceipts)

                    SectionLabel(text: s("t_sec_app"))
                    SettingsSwitch(icon: "bell.fill", title: s("t_notificacoes"), isOn: ui.pushOn, onChange: onPush)
                    SettingsRow(icon: nil, glyph: "Aa", title: s("lang_menu"), action: onLanguage)
                    if Config.relayEditable { SettingsRow(icon: "wrench.fill", title: s("t_relay_teste"), action: onRelay) }
                    SettingsRow(icon: "info.circle.fill", title: s("t_versao", Config.version), divider: false, action: nil)

                    Rectangle().fill(c.outlineVariant).frame(height: 0.5).padding(.horizontal, 20).padding(.top, 10)
                    Spacer().frame(height: 6)
                    SettingsRow(icon: "trash.fill", title: s("t_destruir_perfil_e_dados"), danger: true, divider: false, action: { showWipe = true })
                    Spacer().frame(height: 24)
                }
            }
        }
        .background(c.background.ignoresSafeArea())
        .sheet(isPresented: $showBlocked) {
            BlockedContactsSheet(
                items: ui.blocked.sorted().map { cid in (cid, ui.contact(cid)?.nickname ?? s("t_desconhecido_2", String(cid.prefix(8)))) },
                onUnblock: onUnblock
            )
        }
        .sheet(isPresented: $showWipe) { WipeAllSheet(onConfirm: onWipeAll) }
    }
}

// ================================================================== novo contacto

/// Icone "ler codigo": quatro cantos e uma linha ao meio.
struct ScanGlyph: View {
    var body: some View {
        Canvas { ctx, size in
            let w = size.width, k = w * 0.32
            var p = Path()
            p.move(to: CGPoint(x: 0, y: k)); p.addLine(to: .zero); p.addLine(to: CGPoint(x: k, y: 0))
            p.move(to: CGPoint(x: w - k, y: 0)); p.addLine(to: CGPoint(x: w, y: 0)); p.addLine(to: CGPoint(x: w, y: k))
            p.move(to: CGPoint(x: w, y: w - k)); p.addLine(to: CGPoint(x: w, y: w)); p.addLine(to: CGPoint(x: w - k, y: w))
            p.move(to: CGPoint(x: k, y: w)); p.addLine(to: CGPoint(x: 0, y: w)); p.addLine(to: CGPoint(x: 0, y: w - k))
            p.move(to: CGPoint(x: w * 0.22, y: w / 2)); p.addLine(to: CGPoint(x: w * 0.78, y: w / 2))
            ctx.stroke(p, with: .foreground, style: StrokeStyle(lineWidth: 2.2, lineCap: .round))
        }
        .frame(width: 22, height: 22)
    }
}

/// Ecra Novo contacto (0.9.11): o meu QR para me adicionarem, ler o QR da outra pessoa ou
/// escrever o ID dela, e criar um grupo.
struct AddContactScreen: View {
    let ui: UiState
    let onBack: () -> Void
    let onAdd: (String, String) -> Void
    let onCreateGroup: (String, [Contact]) -> Void
    @State private var nick = ""
    @State private var pid = ""
    @State private var scanning = false
    @State private var showNewGroup = false
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(onBack: onBack) { Text(s("t_novo_contacto")) }
            ScrollView {
                VStack(spacing: 14) {
                    VStack(spacing: 10) {
                        Text(s("t_mostra_este_codigo")).font(.sBodyMedium).multilineTextAlignment(.center).foregroundStyle(c.onSurfaceVariant)
                        if !ui.publicId.isEmpty, let qr = QR.image(for: ui.publicId.uppercased()) {
                            Image(uiImage: qr).interpolation(.none).resizable().scaledToFit()
                                .frame(width: 184, height: 184).padding(8)
                                .background(RoundedRectangle(cornerRadius: 18).fill(Color.white))
                                .accessibilityLabel(s("t_codigo_qr_do_teu_id"))
                        }
                        Text(shortId(ui.publicId)).font(.system(size: 13, design: .monospaced)).foregroundStyle(c.onSurfaceVariant)
                        HStack(spacing: 8) {
                            ShareLink(item: ui.publicId) {
                                Label(s("t_partilhar"), systemImage: "square.and.arrow.up").font(Jakarta.font(14, .semibold))
                                    .padding(.horizontal, 16).padding(.vertical, 10)
                                    .background(Capsule().fill(c.secondaryContainer))
                            }
                            Button { UIPasteboard.general.string = ui.publicId } label: {
                                Text(s("t_copiar")).font(Jakarta.font(14, .semibold))
                                    .padding(.horizontal, 16).padding(.vertical, 10)
                                    .background(Capsule().fill(c.secondaryContainer))
                            }
                        }
                        .foregroundStyle(c.onSecondaryContainer)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(18)
                    .background(RoundedRectangle(cornerRadius: 24).fill(c.surfaceContainerLow))

                    Button { scanning = true } label: {
                        HStack(spacing: 10) {
                            ScanGlyph()
                            Text(s("t_ler_codigo_qr")).font(Jakarta.font(16, .bold))
                        }
                        .foregroundStyle(c.onPrimary)
                        .frame(maxWidth: .infinity).frame(height: 56)
                        .background(RoundedRectangle(cornerRadius: 18).fill(c.primary))
                    }
                    .buttonStyle(.plain)

                    HStack(spacing: 12) {
                        Rectangle().fill(c.outlineVariant).frame(height: 0.5)
                        Text(s("t_ou")).font(.sLabelMedium).foregroundStyle(c.onSurfaceVariant)
                        Rectangle().fill(c.outlineVariant).frame(height: 0.5)
                    }

                    OutlinedField(label: s("t_id_qt1"), text: $pid, monospaced: true)
                    OutlinedField(label: s("t_nome"), text: $nick, singleLine: true)
                    OutlineButton(title: s("t_adicionar")) { onAdd(nick, pid) }
                        .disabled(pid.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    Text(s("t_recebe_o_id_diretamente_da_pessoa")).font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
                        .frame(maxWidth: .infinity, alignment: .leading)

                    Rectangle().fill(c.outlineVariant).frame(height: 0.5)
                    OutlineButton(title: s("t_novo_grupo"), systemImage: "plus") { showNewGroup = true }
                    Spacer().frame(height: 8)
                }
                .padding(.horizontal, 20).padding(.vertical, 12)
            }
            .scrollDismissesKeyboard(.interactively)
        }
        .background(c.background.ignoresSafeArea())
        .fullScreenCover(isPresented: $scanning) { QRScannerSheet { pid = $0 } }
        .sheet(isPresented: $showNewGroup) {
            NewGroupSheet(ui: ui) { n, members in showNewGroup = false; onCreateGroup(n, members) }
        }
    }
}
