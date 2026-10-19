import LocalAuthentication
import SwiftUI

// ======================================================================= bloqueio

/// Ecra de bloqueio: codigo de acesso ou Face ID / Touch ID / codigo do iPhone.
struct LockScreen: View {
    let ui: UiState
    let onCode: (String) -> Void
    let onBio: () -> Void
    let onWipe: () -> Void
    @State private var code = ""
    @State private var forgot = false
    @State private var wipe = false
    @FocusState private var focused: Bool
    @Environment(\.chat) private var c

    var body: some View {
        let minLen = AppModel.minCodeLen
        let canSubmit = !ui.unlocking && code.count >= minLen
        VStack(spacing: 0) {
            Spacer()
            BrandLogo(size: 80)
            Spacer().frame(height: 20)
            Text(s("t_esta_bloqueado", Config.appName)).font(Jakarta.font(22, .semibold)).multilineTextAlignment(.center)
            Spacer().frame(height: 24)
            CodeField(label: s("t_codigo_de_acesso"), code: $code)
                .focused($focused)
                .disabled(ui.unlocking)
                .onSubmit { if canSubmit { onCode(code) } }
            if let m = ui.lockMessage {
                Text(m).font(.sBodySmall).foregroundStyle(c.error).multilineTextAlignment(.center).padding(.top, 8)
            }
            Spacer().frame(height: 16)
            PrimaryButton(title: ui.unlocking ? s("t_a_verificar") : s("t_desbloquear_2")) { onCode(code) }.disabled(!canSubmit)
            if ui.bioEnabled {
                Spacer().frame(height: 8)
                OutlineButton(title: bioLabel(), systemImage: bioIcon(), height: 46, action: onBio).disabled(ui.unlocking)
            }
            Spacer().frame(height: 8)
            Button(s("t_esqueci_me_do_codigo")) { forgot = true }.font(.sLabelLarge)
            Spacer()
        }
        .padding(32)
        .frame(maxWidth: 520)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(c.background.ignoresSafeArea())
        .onAppear { if ui.bioEnabled { onBio() } else { focused = true } }
        .onChange(of: ui.lockMessage) { _, m in if m != nil { code = "" } }
        .alert(s("t_esqueceste_o_codigo"), isPresented: $forgot) {
            Button(s("t_apagar_tudo"), role: .destructive) { wipe = true }
            Button(s("t_voltar"), role: .cancel) {}
        } message: { Text(s("t_sem_o_codigo_nao_e_possivel")) }
        .sheet(isPresented: $wipe) { WipeAllSheet(serverToo: false, onConfirm: onWipe) }
    }
}

/// "Usar Face ID" / "Usar Touch ID" / "Usar o codigo do iPhone".
func bioLabel() -> String {
    switch BioUnlock.biometryKind() {
    case .faceID: return s("ios_use_face_id")
    case .touchID: return s("ios_use_touch_id")
    default: return s("ios_use_passcode")
    }
}

func bioIcon() -> String {
    switch BioUnlock.biometryKind() {
    case .faceID: return "faceid"
    case .touchID: return "touchid"
    default: return "lock.iphone"
    }
}

/// Tempos de bloqueio automatico oferecidos nas definicoes (rotulo, milissegundos).
var lockTimeouts: [(String, Int64)] {
    [(s("t_imediatamente"), 0), (s("t_apos_1_minuto"), 60_000), (s("t_apos_5_minutos"), 300_000), (s("t_apos_15_minutos"), 900_000)]
}

private enum LockDialog: Identifiable {
    case enable, disable, change, bioOn
    var id: Int { hashValue }
}

/// Definicoes do bloqueio: ativar, Face ID, tempo, mudar ou desativar o codigo.
struct LockSettingsScreen: View {
    let ui: UiState
    let onBack: () -> Void
    let onEnable: (String) -> Void
    let onDisable: (String) -> Void
    let onChange: (String, String) -> Void
    let onBioOn: (String) -> Void
    let onBioOff: () -> Void
    let onTimeout: (Int64) -> Void
    @State private var dialog: LockDialog?
    @Environment(\.chat) private var c

    var body: some View {
        VStack(spacing: 0) {
            SealerTopBar(onBack: onBack) { Text(s("t_bloqueio_da_app")) }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(s("t_com_o_bloqueio_ativo_as_conversas")).font(.sBodyMedium)
                    if !ui.lockEnabled {
                        PrimaryButton(title: s("t_ativar_bloqueio"), height: 46) { dialog = .enable }
                    } else {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(s("ios_quick_unlock")).font(.sBodyLarge)
                                Text(ui.bioAvailable || ui.bioEnabled ? s("ios_quick_unlock_hint") : s("ios_bio_unavailable"))
                                    .font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
                            }
                            Spacer()
                            Toggle("", isOn: Binding(get: { ui.bioEnabled }, set: { on in if on { dialog = .bioOn } else { onBioOff() } }))
                                .labelsHidden().tint(c.primary).disabled(!(ui.bioAvailable || ui.bioEnabled))
                        }
                        Divider()
                        Text(s("t_bloquear_automaticamente")).font(.sTitleSmall)
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(lockTimeouts.enumerated()), id: \.offset) { _, option in
                                let (label, ms) = option
                                Button { onTimeout(ms) } label: {
                                    HStack(spacing: 10) {
                                        Image(systemName: ui.lockTimeoutMs == ms ? "largecircle.fill.circle" : "circle")
                                            .font(.system(size: 20)).foregroundStyle(c.primary)
                                        Text(label)
                                        Spacer()
                                    }
                                    .padding(.vertical, 8)
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                            }
                        }
                        Divider()
                        OutlineButton(title: s("t_mudar_codigo"), height: 46) { dialog = .change }
                        Button(s("t_desativar_bloqueio")) { dialog = .disable }.foregroundStyle(c.error).font(.sLabelLarge)
                    }
                    Text(s("t_se_te_esqueceres_do_codigo_e")).font(.sBodySmall).foregroundStyle(c.error)
                }
                .padding(16)
            }
        }
        .background(c.background.ignoresSafeArea())
        .sheet(item: $dialog) { d in
            switch d {
            case .enable:
                CodeSheet(title: s("t_ativar_bloqueio"), text: s("t_escolhe_um_codigo_so_com_numeros"),
                          askCurrent: false, askNew: true, confirm: s("t_ativar")) { _, new in onEnable(new) }
            case .disable:
                CodeSheet(title: s("t_desativar_bloqueio"), text: nil, askCurrent: true, askNew: false,
                          confirm: s("t_desativar")) { cur, _ in onDisable(cur) }
            case .change:
                CodeSheet(title: s("t_mudar_codigo"), text: nil, askCurrent: true, askNew: true,
                          confirm: s("t_mudar")) { cur, new in onChange(cur, new) }
            case .bioOn:
                CodeSheet(title: s("t_desbloqueio_rapido"), text: s("ios_bio_confirm"),
                          askCurrent: true, askNew: false, confirm: s("t_continuar")) { cur, _ in onBioOn(cur) }
            }
        }
    }
}

/// Pede o codigo atual e/ou um codigo novo (duas vezes).
struct CodeSheet: View {
    let title: String
    let text: String?
    let askCurrent: Bool
    let askNew: Bool
    let confirm: String
    let onConfirm: (String, String) -> Void
    @State private var cur = ""
    @State private var new1 = ""
    @State private var new2 = ""
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        let minLen = AppModel.minCodeLen
        let okCur = !askCurrent || cur.count >= minLen
        let okNew = !askNew || (new1.count >= minLen && new1 == new2)
        VStack(alignment: .leading, spacing: 10) {
            HStack { Spacer(); Image(systemName: "lock.fill").font(.title2); Spacer() }
            Text(title).font(Jakarta.font(22, .bold))
            if let text { Text(text).font(.sBodyMedium) }
            if askCurrent { CodeField(label: s("t_codigo_atual"), code: $cur) }
            if askNew {
                CodeField(label: s("t_novo_codigo_minimo_digitos", minLen), code: $new1)
                CodeField(label: s("t_repetir_o_novo_codigo"), code: $new2)
                if !new2.isEmpty && new1 != new2 {
                    Text(s("t_os_codigos_nao_sao_iguais")).font(.sBodySmall).foregroundStyle(c.error)
                }
            }
            HStack {
                Spacer()
                Button(s("t_cancelar")) { dismiss() }
                Button(confirm) { dismiss(); onConfirm(cur, new1) }.disabled(!(okCur && okNew)).padding(.leading, 16)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.large])
        .presentationBackground(c.surface)
    }
}
