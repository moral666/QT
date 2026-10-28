import PhotosUI
import SwiftUI

/// Ecra de chamada (a chamar, a tocar, a chegar, atendida, terminada). Igual ao Android; o
/// ecra do sistema (CallKit) faz o mesmo com a app fechada ou o iPhone bloqueado.
struct CallScreen: View {
    let call: CallUi
    let onAccept: () -> Void
    let onHangup: () -> Void
    let onMute: () -> Void
    let onSpeaker: () -> Void
    /// So na build de testes: o registo da chamada (para descobrir porque nao ha som).
    var diag: [String] = []
    /// Minimizar (volta-se pela barra no topo da app); `nil` = sem botao (a chegar, terminada).
    var onMinimize: (() -> Void)? = nil
    @Environment(\.chat) private var c

    private static let green = Color(hex: 0x2E9E5B)
    private static let red = Color(hex: 0xD64545)

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            VStack(spacing: 0) {
                Spacer().frame(height: 48)
                Avatar(name: call.name, seed: call.contactId, size: 112)
                Spacer().frame(height: 16)
                Text(call.name).font(Jakarta.font(28, .semibold)).lineLimit(1).foregroundStyle(c.onTopBar)
                Spacer().frame(height: 8)
                Text(status).font(.sTitleMedium).foregroundStyle(c.onTopBar.opacity(0.85)).monospacedDigit()
                Spacer()
                switch call.phase {
                case .incoming:
                    HStack(spacing: 56) {
                        RoundAction(label: s("t_recusar"), color: Self.red, hangup: true, action: onHangup)
                        RoundAction(label: s("t_atender"), color: Self.green, hangup: false, action: onAccept)
                    }
                case .ended:
                    Spacer().frame(height: 1)
                default:
                    VStack(spacing: 28) {
                        HStack(spacing: 16) {
                            toggle(call.muted ? s("t_ligar_o_microfone") : s("t_silenciar"), call.muted ? "mic.slash.fill" : "mic.fill", onMute)
                            toggle(call.speaker ? s("t_auricular") : s("t_altifalante"), call.speaker ? "speaker.wave.3.fill" : "ear", onSpeaker)
                        }
                        RoundAction(label: s("t_desligar"), color: Self.red, hangup: true, action: onHangup)
                    }
                }
                Spacer().frame(height: 32)
                if !diag.isEmpty {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(diag.enumerated()), id: \.offset) { _, line in
                            Text(line).font(.system(size: 10, design: .monospaced)).foregroundStyle(.white.opacity(0.85))
                        }
                    }
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Color.black.opacity(0.35)))
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .overlay(alignment: .topLeading) {
            if let onMinimize {
                Button(action: onMinimize) {
                    Image(systemName: "chevron.down").font(.system(size: 22, weight: .semibold)).foregroundStyle(c.onTopBar)
                        .frame(width: 48, height: 48)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(s("call_minimize"))
                .padding(8)
            }
        }
        .background(LinearGradient(colors: c.topBarGradient, startPoint: .top, endPoint: .bottom).ignoresSafeArea())
    }

    private var status: String { callStatusText(call) }

    private func toggle(_ title: String, _ icon: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: icon).font(Jakarta.font(14, .semibold)).foregroundStyle(c.onTopBar)
                .padding(.horizontal, 16).padding(.vertical, 10)
                .overlay(Capsule().stroke(c.onTopBar.opacity(0.6)))
        }
        .buttonStyle(.plain)
    }
}

/// "A chamar", "A tocar", o tempo da chamada ou como acabou (igual no ecra e na barra).
func callStatusText(_ call: CallUi) -> String {
    switch call.phase {
    case .calling: return s("t_a_chamar")
    case .ringingOut: return s("t_a_tocar")
    case .incoming: return s("t_chamada_de_voz")
    case .active:
        let sec = max(0, Int(monotonicSeconds() - call.answeredAt))
        return "\(sec / 60):" + String(format: "%02d", sec % 60)
    case .ended: return call.endText
    }
}

/// Barra verde no topo da app com a chamada minimizada (0.9.29): nome, estado/tempo e
/// "toca para voltar". Igual ao Android (`CallBar`).
struct CallBar: View {
    let call: CallUi
    let onTap: () -> Void
    var body: some View {
        Button(action: onTap) {
            TimelineView(.periodic(from: .now, by: 1)) { _ in
                HStack(spacing: 10) {
                    Image(systemName: "phone.fill").font(.system(size: 15)).foregroundStyle(.white)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("\(call.name) · \(callStatusText(call))").font(.sLabelLarge).foregroundStyle(.white)
                            .lineLimit(1).monospacedDigit()
                        Text(s("call_banner_back")).font(.sLabelSmall).foregroundStyle(.white.opacity(0.85)).lineLimit(1)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 16).padding(.vertical, 8)
                .frame(maxWidth: .infinity)
            }
        }
        .buttonStyle(.plain)
        .background(Color(hex: 0x2E9E5B).ignoresSafeArea(edges: .top))
    }
}

private struct RoundAction: View {
    let label: String
    let color: Color
    let hangup: Bool
    let action: () -> Void
    var body: some View {
        VStack(spacing: 8) {
            Button(action: action) {
                Image(systemName: "phone.fill").font(.system(size: 30)).foregroundStyle(.white)
                    .rotationEffect(.degrees(hangup ? 135 : 0))
                    .frame(width: 72, height: 72)
                    .background(Circle().fill(color))
            }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
            Text(label).font(.sLabelLarge).foregroundStyle(.white)
        }
    }
}

// ================================================================== foto de perfil

/// "Foto de perfil": ver, escolher (de 72 em 72 h) ou tirar a foto. A foto escolhida aparece
/// primeiro ja cortada, com o aviso das 72 h, e so fica com "Aplicar".
struct ProfilePhotoSheet: View {
    @ObservedObject var model: AppModel
    @State private var item: PhotosPickerItem?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        let ui = model.ui
        VStack(spacing: 14) {
            Text(s("t_foto_de_perfil")).font(Jakarta.font(22, .bold))
            if let preview = ui.photoPreview {
                Image(uiImage: preview).resizable().scaledToFill().frame(width: 160, height: 160).clipShape(Circle())
                Text(s("photo_preview_note")).font(.sBodySmall).multilineTextAlignment(.center)
                HStack {
                    Button(s("t_cancelar"), action: model.cancelProfilePhoto)
                    Spacer()
                    Button(s("t_aplicar")) { model.applyProfilePhoto(); dismiss() }
                }
                .font(Jakarta.font(15, .semibold))
            } else {
                Group {
                    if let mine = ui.myPhoto {
                        Image(uiImage: mine).resizable().scaledToFill().frame(width: 140, height: 140).clipShape(Circle())
                    } else {
                        Avatar(name: "Eu", seed: "eu", size: 140)
                    }
                }
                .accessibilityLabel(s("t_a_tua_foto"))
                Text(s("t_so_os_teus_contactos_a_veem")).font(.sBodySmall).multilineTextAlignment(.center).foregroundStyle(c.onSurfaceVariant)
                let canChange = ui.photoNextChangeMs <= 0
                if !canChange {
                    Text(s("t_proxima_mudanca", waitText(ui.photoNextChangeMs))).font(.sBodySmall).multilineTextAlignment(.center)
                }
                HStack {
                    Button(s("t_fechar")) { dismiss() }
                    Spacer()
                    if ui.myPhoto != nil { Button(s("t_tirar"), action: model.removeProfilePhoto) }
                    PhotosPicker(selection: $item, matching: .images) { Text(s("t_escolher_foto")) }
                        .disabled(!canChange).padding(.leading, 12)
                }
                .font(Jakarta.font(15, .semibold))
            }
        }
        .padding(24)
        .presentationDetents([.medium, .large])
        .presentationBackground(c.surface)
        .onChange(of: item) { _, it in
            guard let it else { return }
            item = nil
            Task { @MainActor in
                if let data = try? await it.loadTransferable(type: Data.self) { model.previewProfilePhoto(data) }
            }
        }
    }
}
