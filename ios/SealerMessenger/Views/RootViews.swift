import AVFoundation
import SwiftUI
import UIKit

// ============================================================================ raiz

/// Mesma navegacao que o AppRoot do Android: um ecra de cada vez, decidido pelo estado.
struct AppRoot: View {
    @ObservedObject var model: AppModel
    @State private var showRelay = false
    @State private var showSettings = false
    @State private var showAddContact = false
    @State private var showMyId = false
    @State private var showLockSettings = false
    @State private var showRecovery = false
    @State private var showRestore = false
    @State private var showGroupInfo = false
    @State private var showPhoto = false
    @State private var micDenied = false
    /// Chamada minimizada (0.9.29): o resto da app fica usavel e a barra no topo volta a chamada.
    @State private var callMinimized = false
    @Environment(\.openURL) private var openURL

    var body: some View {
        let ui = model.ui
        ZStack(alignment: .bottom) {
            VStack(spacing: 0) {
                if let call = ui.call, showCallBar(ui) {
                    CallBar(call: call, onTap: { callMinimized = false })
                }
                screen(ui)
            }
            if let n = ui.notice {
                NoticeBanner(text: n)
                    .task {
                        try? await Task.sleep(nanoseconds: 2_000_000_000)
                        model.dismissNotice()
                    }
            }
        }
        .environment(\.photos, ui.photos)
        .environment(\.badges, ui.badges)
        .animation(.easeOut(duration: 0.2), value: ui.openChatId)
        .onChange(of: ui.openChatId) { _, _ in showGroupInfo = false }
        // Cada chamada nova comeca em ecra inteiro.
        .onChange(of: ui.call?.callId) { _, _ in callMinimized = false }
        .onChange(of: ready(ui)) { _, ok in
            // Notificacoes: pede a permissao uma vez, quando a app ja esta pronta a usar.
            if ok && ui.pushOn && !ui.pushAsked {
                model.markPushAsked()
                PushTokens.shared.askPermission()
            }
        }
        .sheet(isPresented: Binding(get: { showMyId && !ui.locked }, set: { showMyId = $0 })) { MyIdSheet(publicId: ui.publicId) }
        .sheet(isPresented: Binding(get: { model.ui.safetyNumber != nil }, set: { if !$0 { model.dismissSafetyNumber() } })) {
            SafetyNumberSheet(text: ui.safetyNumber ?? "", onClose: model.dismissSafetyNumber)
        }
        .sheet(isPresented: Binding(get: { showPhoto && !ui.locked }, set: { if !$0 { model.cancelProfilePhoto(); showPhoto = false } })) {
            ProfilePhotoSheet(model: model)
        }
        .sheet(isPresented: Binding(get: { model.ui.wipeServerFailed != nil }, set: { if !$0 { model.dismissWipeServerFailed() } })) {
            WipeServerFailedSheet(reason: ui.wipeServerFailed ?? "", onRetry: { model.wipeAllData() },
                                  onLocalOnly: { model.wipeAllData(localOnly: true) }, onDismiss: model.dismissWipeServerFailed)
        }
        .alert("", isPresented: Binding(get: { model.ui.error != nil }, set: { if !$0 { model.dismissError() } }),
               actions: { Button("OK", action: model.dismissError) }, message: { Text(ui.error ?? "") })
        // A lista de contactos guardada nao abriu (0.9.23): nada e gravado por cima.
        .alert(s("contacts_unreadable_title"),
               isPresented: Binding(get: { model.ui.contactsUnreadable && !model.ui.locked }, set: { if !$0 { model.dismissContactsUnreadable() } }),
               actions: {
                   Button(s("contacts_unreadable_new"), role: .destructive, action: model.startNewContactList)
                   Button(s("t_fechar"), role: .cancel, action: model.dismissContactsUnreadable)
               },
               message: { Text(s("contacts_unreadable_text")) })
        .alert(s("ios_mic_denied_call"), isPresented: $micDenied) {
            Button(s("ios_open_settings")) { if let u = URL(string: UIApplication.openSettingsURLString) { openURL(u) } }
            Button(s("t_cancelar"), role: .cancel) {}
        }
    }

    /// A chamada a chegar nunca fica escondida; minimizar so a chamar, a tocar ou atendida.
    private func canMinimize(_ call: CallUi) -> Bool { call.phase != .incoming && call.phase != .ended }
    private func showCallBar(_ ui: UiState) -> Bool {
        guard let call = ui.call else { return false }
        return callMinimized && call.phase != .incoming && !ui.locked && !ui.loading
    }

    private func ready(_ ui: UiState) -> Bool { !ui.loading && ui.hasIdentity && !ui.locked && ui.termsAccepted }

    /// Chamadas: o microfone e pedido so ao ligar (uma vez).
    private func withMic(_ action: @escaping () -> Void) {
        switch AVAudioApplication.shared.recordPermission {
        case .granted: action()
        case .denied: micDenied = true
        default:
            AVAudioApplication.requestRecordPermission { ok in
                Task { @MainActor in if ok { action() } else { micDenied = true } }
            }
        }
    }

    @ViewBuilder
    private func screen(_ ui: UiState) -> some View {
        let openGroup = ui.openChatId.flatMap { ui.groupOf($0) }
        if ui.loading {
            LoadingScreen()
        } else if let call = ui.call, !ui.locked, !showCallBar(ui) {
            CallScreen(call: call, onAccept: { withMic { model.acceptCall() } }, onHangup: model.hangupCall,
                       onMute: model.toggleMute, onSpeaker: model.toggleSpeaker, diag: model.callDiag,
                       onMinimize: canMinimize(call) ? { callMinimized = true } : nil)
        } else if ui.locked {
            LockScreen(ui: ui, onCode: model.unlockWithCode, onBio: model.requestBioUnlock, onWipe: model.wipeLockedData)
        } else if !ui.termsAccepted {
            TermsScreen(onAccept: model.acceptTerms)
        } else if !ui.hasIdentity && showRestore {
            RestoreScreen(ui: ui, onBack: { showRestore = false }, onRestore: model.restoreAccount)
        } else if !ui.hasIdentity {
            SetupScreen(ui: ui, onCreate: model.createIdentity, onRestore: { showRestore = true })
        } else if ui.showPremium {
            PremiumScreen(ui: ui, onBack: model.closePremium)
        } else if let g = openGroup, showGroupInfo {
            GroupInfoScreen(ui: ui, group: g, onBack: { showGroupInfo = false },
                            onUpdate: { name, members in model.updateGroup(g.groupId, name: name, members: members) },
                            onLeave: { model.leaveGroup(g.groupId) },
                            onDelete: { showGroupInfo = false; model.deleteGroup(g.groupId) },
                            onAddContact: { n, p in model.addContact(nickname: n, publicId: p) })
        } else if let g = openGroup {
            ChatScreen(model: model, ui: ui, conversationId: g.conversationId, group: g, onInfo: { showGroupInfo = true },
                       onCall: { _ in })
                .transition(.move(edge: .trailing))
        } else if let id = ui.openChatId, id.hasPrefix("g:") {
            // O grupo foi apagado (ou ainda nao carregou): volta a lista.
            LoadingScreen().task { model.closeChat() }
        } else if let id = ui.openChatId {
            ChatScreen(model: model, ui: ui, conversationId: id, group: nil, onInfo: {},
                       onCall: { c in withMic { model.startCall(c) } })
                .transition(.move(edge: .trailing))
        } else if showRecovery {
            RecoveryScreen(ui: ui, onBack: { showRecovery = false }, onCreate: model.recoveryCreate,
                           onShow: model.recoveryShow, onWritten: model.recoveryWritten,
                           onHide: model.recoveryHide, onDisable: model.recoveryDisable,
                           onCode: model.recoveryWithCode, onCancelCode: model.recoveryCancelAuth)
        } else if showLockSettings {
            LockSettingsScreen(ui: ui, onBack: { showLockSettings = false },
                               onEnable: model.enableLock, onDisable: model.disableLock, onChange: model.changeCode,
                               onBioOn: model.startBioEnable, onBioOff: model.disableBio, onTimeout: model.setLockTimeout)
        } else if showRelay && Config.relayEditable {
            RelaySettingsScreen(ui: ui, onSave: { u, k in model.saveRelay(relayUrl: u, relayKeyHex: k); showRelay = false },
                                onBack: { showRelay = false })
        } else if showAddContact {
            AddContactScreen(ui: ui, onBack: { showAddContact = false },
                             onAdd: { n, p in showAddContact = false; model.addContact(nickname: n, publicId: p) },
                             onCreateGroup: { n, members in showAddContact = false; model.createGroup(name: n, members: members) })
        } else if showSettings {
            ProfileSettingsScreen(
                ui: ui, onBack: { showSettings = false },
                onProfilePhoto: { showPhoto = true }, onMyId: { showMyId = true },
                onLockSettings: { showLockSettings = true }, onLockNow: model.lockNow,
                onRecovery: { showRecovery = true },
                onPremium: model.openPremium,
                onUnblock: model.unblockContact, onPresence: model.setPresence, onReadReceipts: model.setReadReceipts,
                onPush: model.setPush,
                onLanguage: { if let u = URL(string: UIApplication.openSettingsURLString) { openURL(u) } },
                onRelay: { showRelay = true },
                onWipeAll: { showSettings = false; model.wipeAllData() }
            )
        } else {
            HomeScreen(ui: ui, onOpen: model.openChat, onMyId: { showMyId = true }, onSettings: { showSettings = true },
                       onAdd: { showAddContact = true },
                       onDeleteConversation: model.deleteConversation, onDeleteContact: model.deleteContact,
                       onDeleteGroup: model.deleteGroup)
        }
    }
}

struct LoadingScreen: View {
    @Environment(\.chat) private var c
    var body: some View {
        VStack(spacing: 16) {
            BrandLogo(size: 72)
            Text(s("t_a_abrir_o_armazenamento_seguro")).font(.sBodyMedium)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(c.background.ignoresSafeArea())
    }
}

// ========================================================================= entrada

struct SetupScreen: View {
    let ui: UiState
    let onCreate: (String, String) -> Void
    var onRestore: () -> Void = {}
    @State private var url = ""
    @State private var key = ""
    @State private var advanced = false
    @Environment(\.chat) private var c

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                Spacer(minLength: 60)
                BrandLogo(size: 96)
                Spacer().frame(height: 24)
                Text(Config.appName).font(Jakarta.font(28, .bold))
                Spacer().frame(height: 8)
                Text(s("t_mensagens_cifradas_ponta_a_ponta_com"))
                    .font(.sBodyMedium).multilineTextAlignment(.center).foregroundStyle(c.onSurfaceVariant)
                Spacer().frame(height: 32)
                PrimaryButton(title: s("t_comecar")) { onCreate(url, key) }
                Button(s("recovery_have_words"), action: onRestore).font(.sLabelLarge).padding(.top, 12)
                Spacer().frame(height: 16)
                Text(s("t_versao_de_testes"))
                    .font(.sLabelSmall).multilineTextAlignment(.center).foregroundStyle(c.onSurfaceVariant)
                if Config.relayEditable {   // so na build debug
                    Button(advanced ? s("t_esconder_opcoes_de_teste") : s("t_opcoes_de_teste")) { advanced.toggle() }
                        .font(.sLabelLarge).padding(.top, 8)
                    if advanced {
                        VStack(spacing: 12) {
                            OutlinedField(label: s("t_url_do_relay"), text: $url, singleLine: true)
                            OutlinedField(label: s("t_chave_publica_do_relay_pinning"), text: $key, monospaced: true)
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

/// Termos de utilizacao e politica de privacidade (a App Store tambem o exige em apps com
/// conversas entre utilizadores: forma de bloquear e de denunciar abusos).
struct TermsScreen: View {
    let onAccept: () -> Void
    @State private var agreed = false
    @Environment(\.chat) private var c
    @Environment(\.openURL) private var openURL

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Spacer().frame(height: 24)
                Text(s("t_antes_de_comecar")).font(Jakarta.font(24, .semibold))
                Text(s("t_a_sealer_esta_em_testes_ao")).font(.sBodyLarge)
                ForEach([s("t_nao_uses_a_sealer_para_nada"), s("t_se_alguem_te_incomodar_podes_bloquea"),
                         s("t_as_tuas_mensagens_so_existem_nos"), s("t_por_estar_em_testes_a_app")], id: \.self) { line in
                    HStack(alignment: .top, spacing: 6) {
                        Text("•")
                        Text(line)
                    }
                    .font(.sBodyMedium)
                }
                HStack(spacing: 16) {
                    Button(s("t_termos_de_utilizacao")) { if let u = URL(string: "https://sealer.pt/termos") { openURL(u) } }
                    Button(s("t_politica_de_privacidade")) { if let u = URL(string: "https://sealer.pt/privacidade") { openURL(u) } }
                }
                .font(.sLabelLarge)
                Button { agreed.toggle() } label: {
                    HStack(spacing: 10) {
                        Image(systemName: agreed ? "checkmark.square.fill" : "square").font(.system(size: 22)).foregroundStyle(c.primary)
                        Text(s("t_li_e_aceito_os_termos_de")).font(.sBodyMedium).multilineTextAlignment(.leading)
                    }
                }
                .buttonStyle(.plain)
                PrimaryButton(title: s("t_continuar"), action: onAccept).disabled(!agreed)
            }
            .padding(24)
        }
        .background(c.background.ignoresSafeArea())
    }
}

struct RelaySettingsScreen: View {
    let ui: UiState
    let onSave: (String, String) -> Void
    let onBack: () -> Void
    @State private var url = ""
    @State private var key = ""
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(onBack: onBack) { Text(s("t_relay_teste")) }
            VStack(alignment: .leading, spacing: 12) {
                OutlinedField(label: s("t_url_do_relay"), text: $url, singleLine: true)
                OutlinedField(label: s("t_chave_publica_do_relay_pinning"), text: $key, monospaced: true)
                PrimaryButton(title: s("t_guardar_e_voltar_a_ligar"), height: 44) { onSave(url, key) }
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
        VStack(spacing: 16) {
            Text(s("t_o_teu_id")).font(Jakarta.font(22, .bold)).frame(maxWidth: .infinity, alignment: .leading)
            // Em maiusculas o QR usa o modo alfanumerico: menos quadrados, as camaras leem melhor.
            if !publicId.isEmpty, let qr = QR.image(for: publicId.uppercased()) {
                Image(uiImage: qr).interpolation(.none).resizable().scaledToFit()
                    .frame(width: 250, height: 250).padding(6)
                    .background(RoundedRectangle(cornerRadius: 12).fill(Color.white))
                    .accessibilityLabel(s("t_codigo_qr_do_teu_id"))
            }
            Text(shortId(publicId)).font(.system(size: 13, design: .monospaced)).foregroundStyle(c.onSurfaceVariant)
            HStack {
                Spacer()
                Button(s("t_copiar")) { UIPasteboard.general.string = publicId; dismiss() }
                ShareLink(s("t_partilhar"), item: publicId).padding(.leading, 16)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.large])
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
            Text(s("t_verificar_seguranca")).font(Jakarta.font(22, .bold)).frame(maxWidth: .infinity)
            Text(text).font(.system(.body, design: .monospaced)).textSelection(.enabled)
            Text(s("t_compara_estes_numeros_com_a_outra")).font(.sBodySmall)
            HStack { Spacer(); Button(s("t_fechar"), action: onClose).font(Jakarta.font(15, .semibold)) }
        }
        .padding(24)
        .presentationDetents([.medium, .large])
        .presentationBackground(c.surface)
    }
}

// ===================================================================== eliminar

/// O que esta a ser confirmado (dois passos de proposito: e permanente).
enum DeleteTarget: Identifiable {
    case choose(id: String, name: String, hasHistory: Bool)
    case conversation(id: String, name: String)
    case contact(id: String, name: String)
    case group(id: String, name: String)
    var id: String {
        switch self {
        case .choose(let id, _, _): return "choose-" + id
        case .conversation(let id, _): return "conv-" + id
        case .contact(let id, _): return "contact-" + id
        case .group(let id, _): return "group-" + id
        }
    }
}

extension View {
    /// Mesmos textos e os mesmos dois passos que na app Android.
    func deleteConfirmation(_ target: Binding<DeleteTarget?>,
                            onDeleteConversation: @escaping (String) -> Void,
                            onDeleteContact: @escaping (String) -> Void,
                            onDeleteGroup: @escaping (String) -> Void = { _ in }) -> some View {
        let isChoose: Bool = { if case .choose = target.wrappedValue { return true } else { return false } }()
        return self
            .confirmationDialog(
                { if case .choose(_, let name, _) = target.wrappedValue { return name } else { return "" } }(),
                isPresented: Binding(get: { isChoose }, set: { if !$0, isChoose { target.wrappedValue = nil } }),
                titleVisibility: .visible,
                presenting: target.wrappedValue
            ) { t in
                if case .choose(let id, let name, let hasHistory) = t {
                    if hasHistory {
                        Button(s("t_eliminar_conversa")) { DispatchQueue.main.async { target.wrappedValue = .conversation(id: id, name: name) } }
                    }
                    Button(s("t_apagar_contacto"), role: .destructive) {
                        DispatchQueue.main.async { target.wrappedValue = .contact(id: id, name: name) }
                    }
                    Button(s("t_cancelar"), role: .cancel) { target.wrappedValue = nil }
                }
            }
            .alert(
                { () -> String in
                    switch target.wrappedValue {
                    case .conversation: return s("t_eliminar_conversa_2")
                    case .contact: return s("t_apagar_contacto_2")
                    case .group(_, let name): return s("t_apagar", name)
                    default: return ""
                    }
                }(),
                isPresented: Binding(get: { target.wrappedValue != nil && !isChoose }, set: { if !$0, !isChoose { target.wrappedValue = nil } }),
                presenting: target.wrappedValue,
                actions: { t in
                    switch t {
                    case .conversation(let id, _): Button(s("t_eliminar"), role: .destructive) { onDeleteConversation(id) }
                    case .contact(let id, _): Button(s("t_apagar_2"), role: .destructive) { onDeleteContact(id) }
                    case .group(let id, _): Button(s("t_apagar_2"), role: .destructive) { onDeleteGroup(id) }
                    case .choose: EmptyView()
                    }
                    Button(s("t_cancelar"), role: .cancel) {}
                },
                message: { t in
                    switch t {
                    case .conversation(_, let name): Text(s("t_todas_as_mensagens_com_sao_apagadas", name))
                    case .contact(_, let name): Text(s("t_e_removido_da_tua_lista_e", name))
                    case .group: Text(s("t_sais_do_grupo_se_ainda_estiveres"))
                    case .choose: EmptyView()
                    }
                }
            )
    }
}

/// Destruir perfil e dados: so com a palavra de confirmacao escrita (evita toques acidentais).
struct WipeAllSheet: View {
    var serverToo = true
    let onConfirm: () -> Void
    @State private var typed = ""
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        let word = s("confirm_word")
        VStack(alignment: .leading, spacing: 12) {
            HStack { Spacer(); Image(systemName: "trash.fill").font(.title2).foregroundStyle(c.error); Spacer() }
            Text(s("t_destruir_perfil_e_dados_2")).font(Jakarta.font(22, .bold)).frame(maxWidth: .infinity)
            Text(serverToo ? s("t_sao_apagados_para_sempre_do_servidor") : s("t_sao_apagados_para_sempre_deste_telemovel"))
                .font(.sBodyMedium)
            OutlinedField(label: s("t_para_confirmar_escreve_apagar", word), text: $typed, singleLine: true)
            HStack {
                Spacer()
                Button(s("t_cancelar")) { dismiss() }
                Button(s("t_apagar_tudo"), role: .destructive) { dismiss(); onConfirm() }
                    .disabled(!Lang.confirmWordOk(typed))
                    .padding(.leading, 16)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.medium, .large])
        .presentationBackground(c.surface)
    }
}

/// Nao foi possivel apagar a conta no servidor: nada foi apagado ainda.
struct WipeServerFailedSheet: View {
    let reason: String
    let onRetry: () -> Void
    let onLocalOnly: () -> Void
    let onDismiss: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(s("t_nao_foi_possivel_apagar_no_servidor")).font(Jakarta.font(20, .bold))
            Text(s("t_motivo_ainda_nao_foi_apagado_nada", reason)).font(.sBodyMedium)
            Text(s("t_podes_tentar_de_novo_quando_tiveres")).font(.sBodySmall)
            HStack {
                Button(s("t_apagar_so_aqui"), role: .destructive, action: onLocalOnly).foregroundStyle(c.error)
                Spacer()
                Button(s("t_cancelar"), action: onDismiss)
                Button(s("t_tentar_de_novo"), action: onRetry).padding(.leading, 12)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.medium])
        .presentationBackground(c.surface)
    }
}

/// Contactos bloqueados, cada um com "Desbloquear".
struct BlockedContactsSheet: View {
    let items: [(String, String)]
    let onUnblock: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(s("t_contactos_bloqueados")).font(Jakarta.font(20, .bold))
            if items.isEmpty { Text(s("t_nao_tens_ninguem_bloqueado")).font(.sBodyMedium) }
            ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                let (cid, name) = item
                HStack {
                    Text(name).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                    Button(s("t_desbloquear_2")) { onUnblock(cid) }.font(Jakarta.font(14, .semibold))
                }
                .padding(.vertical, 6)
            }
            HStack { Spacer(); Button(s("t_fechar")) { dismiss() }.font(Jakarta.font(15, .semibold)) }
        }
        .padding(24)
        .presentationDetents([.medium, .large])
        .presentationBackground(c.surface)
    }
}
