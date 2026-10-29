import AVFoundation
import PhotosUI
import SwiftUI

/// Ecra de chamada (a chamar, a tocar, a chegar, atendida, terminada). O mesmo desenho que o
/// Android (0.9.38 la, 0.9.50 aqui): logo e cores da app, "Sealer" em Manrope Bold, parado.
///
///  * Com video: o video do outro enche o ecra e a minha camara fica num canto, com cantos
///    arredondados. Enquanto o outro nao aceita, a minha camara enche o ecra com "A espera de
///    [Nome]..." (o unico texto de estado). Video do outro parado: so a foto ou a inicial dele.
///  * Topo: "Sealer" a esquerda, o tempo da chamada a direita (toque longo: diagnostico).
///  * Barra de controlos (microfone, altifalante, camara, trocar camara) e, por baixo, o
///    desligar vermelho.
///  * Com video, o topo e os controlos desaparecem ao fim de 3-4 s sem toques e voltam com um
///    toque no ecra. So de voz (ou com o VoiceOver): sempre visiveis.
///
/// Ecra: com video fica sempre aceso; so de voz apaga-se ao encostar ao ouvido (sensor de
/// proximidade). O ecra do sistema (CallKit) faz o mesmo com a app fechada ou o iPhone bloqueado.
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
    /// Video (`nil` = sem video neste ecra).
    var onCamera: (() -> Void)? = nil
    var onSwitchCamera: () -> Void = {}
    var onVideoRespond: ((VideoResponse) -> Void)? = nil
    var selfSession: AVCaptureSession? = nil
    var remoteLayer: AVSampleBufferDisplayLayer? = nil
    /// Toque longo no tempo da chamada: os numeros da chamada. `nil` = desligado.
    var diagnostic: (() -> String?)? = nil
    @Environment(\.chat) private var c
    @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
    @State private var controlsShown = true
    @State private var touches = 0
    @State private var showDiag = false

    private static let green = Color(hex: 0x2E9E5B)
    private static let red = Color(hex: 0xD64545)
    /// Com video, o topo e os controlos desaparecem ao fim disto sem toques.
    private static let hideAfter: UInt64 = 3_500_000_000

    var body: some View {
        let v = call.video
        let active = call.phase == .active
        let videoAllowed = onCamera != nil && active
        let showRemote = videoAllowed && v.showRemote && remoteLayer != nil
        // Videochamada minha a chamar ou a tocar: a minha camara ja esta no ecra (nada sai ainda).
        let calling = call.phase == .calling || call.phase == .ringingOut
        let showSelf = onCamera != nil && v.camera && (active || calling) && selfSession != nil
        let videoOn = showRemote || showSelf
        let request = videoAllowed && v.request && onVideoRespond != nil
        // Esconder o topo e os controlos: so com video e atendida, sem um pedido por responder,
        // e nunca com o VoiceOver (o leitor de ecra tem de encontrar sempre os botoes).
        let autoHide = videoOn && active && !request && !voiceOver
        let shown = controlsShown || !autoHide
        let ink: Color = videoOn ? .white : c.onTopBar
        let proximity = (active || calling) && !videoOn && !request && !call.speaker
        // Com video: "A espera de [Nome]..." enquanto o outro nao aceita; a chamar, o nome e o estado.
        let banner: String? = active ? (v.waiting ? s("video_waiting", call.name) : nil) : call.name + " · " + callStatusText(call)

        ZStack {
            if videoOn {
                Color.black.ignoresSafeArea()
            } else {
                LinearGradient(colors: c.topBarGradient, startPoint: .top, endPoint: .bottom).ignoresSafeArea()
            }
            videoLayers(v, showRemote: showRemote, showSelf: showSelf)
            // Sombras suaves por cima do video, para o topo e os controlos se lerem sempre.
            if videoOn && shown {
                VStack(spacing: 0) {
                    LinearGradient(colors: [Color.black.opacity(0.6), .clear], startPoint: .top, endPoint: .bottom).frame(height: 150)
                    Spacer()
                    LinearGradient(colors: [.clear, Color.black.opacity(0.6)], startPoint: .top, endPoint: .bottom).frame(height: 280)
                }
                .ignoresSafeArea()
                .allowsHitTesting(false)
                .transition(.opacity)
            }
            VStack(spacing: 0) {
                if shown {
                    topBar(active: active, ink: ink).transition(.opacity)
                } else {
                    Spacer().frame(height: 56)
                }
                if !videoOn {
                    // So de voz (ou a chegar): a foto, o nome e o estado, no meio.
                    Spacer().frame(height: 56)
                    Avatar(name: call.name, seed: call.contactId, size: 120)
                    Spacer().frame(height: 20)
                    Text(call.name).font(Jakarta.font(28, .semibold)).lineLimit(1).foregroundStyle(c.onTopBar)
                    if !active {
                        Spacer().frame(height: 8)
                        Text(callStatusText(call)).font(.sTitleMedium).foregroundStyle(c.onTopBar.opacity(0.85))
                            .multilineTextAlignment(.center)
                    }
                } else if let banner {
                    Text(banner).font(.sLabelLarge).foregroundStyle(.white).multilineTextAlignment(.center).lineLimit(2)
                        .padding(.horizontal, 14).padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: 16).fill(Color.black.opacity(0.6)))
                        .padding(.top, 8)
                }
                Spacer()
                if request, let respond = onVideoRespond {
                    VideoRequestCard(name: call.name, onRespond: respond).padding(.bottom, 16)
                }
                switch call.phase {
                case .incoming:
                    HStack(spacing: 64) {
                        RoundAction(label: s("t_recusar"), color: Self.red, hangup: true, action: onHangup)
                        RoundAction(label: s("t_atender"), color: Self.green, hangup: false, action: onAccept)
                    }
                case .ended:
                    Spacer().frame(height: 1)
                default:
                    if shown {
                        VStack(spacing: 20) {
                            controlBar(videoOn: videoOn, camera: videoAllowed && v.supported && onCamera != nil)
                            HangupButton(action: touched(onHangup))
                        }
                        .transition(.opacity)
                    }
                }
                if !diag.isEmpty && !videoOn {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(diag.enumerated()), id: \.offset) { _, line in
                            Text(line).font(.system(size: 10, design: .monospaced)).foregroundStyle(.white.opacity(0.85))
                        }
                    }
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Color.black.opacity(0.35)))
                    .padding(.top, 16)
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 28)
        }
        .contentShape(Rectangle())
        .onTapGesture {
            guard autoHide else { return }
            withAnimation(.easeOut(duration: 0.2)) { controlsShown.toggle() }
            touches += 1
        }
        .task(id: "\(autoHide)-\(controlsShown)-\(touches)") {
            if !autoHide {
                controlsShown = true
            } else if controlsShown {
                try? await Task.sleep(nanoseconds: Self.hideAfter)
                if !Task.isCancelled { withAnimation(.easeOut(duration: 0.3)) { controlsShown = false } }
            }
        }
        // Com video o ecra fica sempre aceso; so de voz, apaga-se ao encostar ao ouvido.
        .onChange(of: videoOn, initial: true) { _, on in UIApplication.shared.isIdleTimerDisabled = on }
        .onChange(of: proximity, initial: true) { _, on in UIDevice.current.isProximityMonitoringEnabled = on }
        .onDisappear {
            UIApplication.shared.isIdleTimerDisabled = false
            UIDevice.current.isProximityMonitoringEnabled = false
        }
        .sheet(isPresented: $showDiag) {
            if let diagnostic { CallDiagSheet(diagnostic: diagnostic) }
        }
    }

    /// Camadas do video (por baixo de tudo, ate as bordas do ecra).
    @ViewBuilder
    private func videoLayers(_ v: VideoUi, showRemote: Bool, showSelf: Bool) -> some View {
        if showRemote, let layer = remoteLayer {
            RemoteVideoView(layer: layer, rotation: v.remoteRotation).ignoresSafeArea()
            if v.remoteStalled {
                // O video dele parou (pausa, rede, sem fichas): so a foto ou a inicial, sem texto.
                ZStack {
                    LinearGradient(colors: c.topBarGradient, startPoint: .top, endPoint: .bottom)
                    Avatar(name: call.name, seed: call.contactId, size: 112)
                }
                .ignoresSafeArea()
            }
        }
        if showSelf, let session = selfSession {
            if showRemote {
                VStack {
                    HStack {
                        Spacer()
                        SelfVideoView(session: session)
                            .frame(width: 104, height: 156)
                            .clipShape(RoundedRectangle(cornerRadius: 16))
                            .overlay(RoundedRectangle(cornerRadius: 16).stroke(Color.white.opacity(0.35), lineWidth: 1))
                    }
                    Spacer()
                }
                .padding(.top, 68).padding(.trailing, 16)
            } else {
                SelfVideoView(session: session).ignoresSafeArea()
            }
        }
    }

    /// Tocar num botao tambem conta como toque (os 3-4 s recomecam).
    private func touched(_ f: @escaping () -> Void) -> () -> Void {
        { touches += 1; f() }
    }

    /// Topo da chamada: (minimizar) logo e "Sealer" a esquerda, o tempo a direita.
    private func topBar(active: Bool, ink: Color) -> some View {
        HStack(spacing: 8) {
            if let onMinimize {
                Button(action: touched(onMinimize)) {
                    Image(systemName: "chevron.down").font(.system(size: 22, weight: .semibold)).foregroundStyle(ink)
                        .frame(width: 44, height: 44)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(s("call_minimize"))
            } else {
                Spacer().frame(width: 8)
            }
            BrandLogo(size: 28)
            Text("Sealer").font(.custom("Manrope-Bold", size: 22)).foregroundStyle(ink).lineLimit(1)
            Spacer()
            if active {
                Text(callStatusText(call)).font(Jakarta.font(17, .semibold)).foregroundStyle(ink).monospacedDigit()
                    .onLongPressGesture { if diagnostic != nil { showDiag = true } }
            }
        }
        .frame(height: 56)
    }

    /// Barra com os botoes redondos: microfone, altifalante e, se os dois lados tiverem video,
    /// camara e trocar camara.
    private func controlBar(videoOn: Bool, camera: Bool) -> some View {
        let v = call.video
        return HStack(spacing: 12) {
            ToggleRound(icon: call.muted ? "mic.slash.fill" : "mic.fill", label: s("t_silenciar"), on: call.muted, action: touched(onMute))
            ToggleRound(icon: "speaker.wave.2.fill", label: s("t_altifalante"), on: call.speaker, action: touched(onSpeaker))
            if camera, let onCamera {
                ToggleRound(icon: v.camera ? "video.fill" : "video.slash.fill", label: s("call_camera"), on: v.camera, action: touched(onCamera))
                Button(action: touched(onSwitchCamera)) {
                    Image(systemName: "arrow.triangle.2.circlepath.camera.fill").font(.system(size: 22))
                        .foregroundStyle(.white.opacity(v.camera ? 1 : 0.45))
                        .frame(width: 56, height: 56)
                        .background(Circle().fill(Color.white.opacity(v.camera ? 0.16 : 0.08)))
                }
                .buttonStyle(.plain)
                .disabled(!v.camera)
                .accessibilityLabel(s("video_switch"))
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .background(Capsule().fill(videoOn ? Color.black.opacity(0.65) : Color.white.opacity(0.1)))
    }
}

/// Botao redondo de ligar/desligar: ligado = fundo branco e icone escuro.
private struct ToggleRound: View {
    let icon: String
    let label: String
    let on: Bool
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Image(systemName: icon).font(.system(size: 22))
                .foregroundStyle(on ? Color(hex: 0x1C1B1F) : .white)
                .frame(width: 56, height: 56)
                .background(Circle().fill(on ? Color.white : Color.white.opacity(0.16)))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// Desligar: botao redondo vermelho com o auscultador.
private struct HangupButton: View {
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Image(systemName: "phone.down.fill").font(.system(size: 30)).foregroundStyle(.white)
                .frame(width: 72, height: 72)
                .background(Circle().fill(Color(hex: 0xD64545)))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(s("t_desligar"))
    }
}

/// "[Nome] quer mostrar video": [Ver] [Ligar a minha camara] [Agora nao].
private struct VideoRequestCard: View {
    let name: String
    let onRespond: (VideoResponse) -> Void
    var body: some View {
        VStack(spacing: 8) {
            Text(s("video_request_title", name)).font(.sTitleMedium).foregroundStyle(.white).multilineTextAlignment(.center)
                .padding(.bottom, 4)
            choice(s("video_view"), filled: true) { onRespond(.view) }
            choice(s("video_view_and_camera"), filled: true) { onRespond(.viewAndCamera) }
            choice(s("video_not_now"), filled: false) { onRespond(.notNow) }
        }
        .padding(16)
        .frame(maxWidth: .infinity)
        .background(RoundedRectangle(cornerRadius: 16).fill(Color.black.opacity(0.8)))
    }

    private func choice(_ title: String, filled: Bool, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).font(Jakarta.font(15, .semibold)).frame(maxWidth: .infinity).padding(.vertical, 11)
                .foregroundStyle(filled ? Color(hex: 0x1C1B1F) : .white)
                .background(Capsule().fill(filled ? Color.white : .clear))
                .overlay(Capsule().stroke(Color.white.opacity(filled ? 0 : 0.7)))
        }
        .buttonStyle(.plain)
    }
}

/// Painel dos testers: os numeros da chamada (de segundo a segundo) e "Copiar diagnostico".
private struct CallDiagSheet: View {
    let diagnostic: () -> String?
    @State private var text = ""
    @State private var copied = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(s("call_diag_title")).font(Jakarta.font(20, .bold))
            ScrollView {
                Text(text).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if copied { Text(s("call_diag_copied")).font(.sBodySmall) }
            HStack {
                Button(s("t_fechar")) { dismiss() }
                Spacer()
                Button(s("call_diag_copy")) {
                    UIPasteboard.general.string = text
                    copied = true
                }
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(20)
        .presentationDetents([.medium, .large])
        .task {
            while !Task.isCancelled {
                text = diagnostic() ?? text
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
    }
}

/// "A chamar", "A tocar", o tempo da chamada ou como acabou (igual no ecra e na barra).
func callStatusText(_ call: CallUi) -> String {
    switch call.phase {
    case .calling: return s("t_a_chamar")
    case .ringingOut: return s("t_a_tocar")
    case .incoming: return call.videoCall ? s("video_call_from", call.name) : s("t_chamada_de_voz")
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
