import AVFoundation
import SwiftUI

/// Ecra da chamada de GRUPO (0.9.64), igual ao do Android: a minha camara no quadrado grande em
/// cima (desde a 0.9.66) e os outros numa grelha de 2, com o nome e o icone do microfone ou da
/// camara. Cores e logo da app (o esboco do Nuno so serviu de referencia de layout). Com video,
/// o topo e os botoes desaparecem ao fim de 3,5 s sem toques; um toque mostra-os outra vez.
struct GroupCallScreen: View {
    let g: GroupCallUi
    let myName: String
    let onJoin: () -> Void
    let onLeave: () -> Void
    let onMute: () -> Void
    let onSpeaker: () -> Void
    let onCamera: () -> Void
    let onSwitchCamera: () -> Void
    let selfSession: AVCaptureSession?
    let remoteLayer: (String) -> AVSampleBufferDisplayLayer?
    /// Premir o tempo da chamada abre os numeros (como nas chamadas a dois).
    var diagnostic: (() -> String?)? = nil

    @Environment(\.chat) private var c
    @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
    @State private var shown = true
    @State private var showDiag = false
    @State private var touches = 0

    private var anyVideo: Bool { g.joined && (g.camera || g.tiles.contains { $0.camera && $0.connected }) }
    private var autoHide: Bool { anyVideo && !voiceOver }

    var body: some View {
        ZStack {
            if anyVideo {
                Color.black.ignoresSafeArea()
            } else {
                LinearGradient(colors: c.topBarGradient, startPoint: .top, endPoint: .bottom).ignoresSafeArea()
            }
            if g.joined { inCall } else { ringing }
        }
        .contentShape(Rectangle())
        .onTapGesture { if autoHide { shown.toggle(); touches += 1 } }
        .task(id: "\(autoHide)-\(shown)-\(touches)") {
            guard autoHide else { shown = true; return }
            guard shown else { return }
            try? await Task.sleep(nanoseconds: 3_500_000_000)
            if !Task.isCancelled { withAnimation { shown = false } }
        }
        .sheet(isPresented: $showDiag) {
            if let diagnostic { CallDiagSheet(diagnostic: diagnostic) }
        }
        .onAppear { UIApplication.shared.isIdleTimerDisabled = true }
        .onDisappear { UIApplication.shared.isIdleTimerDisabled = false }
    }

    // A tocar: o grupo, quem chama, Recusar / Atender.
    private var ringing: some View {
        VStack(spacing: 0) {
            Spacer().frame(height: 112)
            Avatar(name: g.groupName, seed: g.groupId, size: 120)
            Text(g.groupName).font(Jakarta.font(28, .semibold)).lineLimit(1).foregroundStyle(c.onTopBar).padding(.top, 20)
            Text(s(g.video ? "gcall_video_title" : "gcall_title") + " · " + s("gcall_from", g.fromName))
                .font(.sTitleMedium).foregroundStyle(c.onTopBar.opacity(0.85)).multilineTextAlignment(.center).padding(.top, 8)
            Spacer()
            HStack(spacing: 64) {
                RoundAction(label: s("t_recusar"), color: Color(hex: 0xD64545), hangup: true, action: onLeave)
                RoundAction(label: s("t_atender"), color: Color(hex: 0x2E9E5B), hangup: false, action: onJoin)
            }
            .padding(.bottom, 40)
        }
        .padding(.horizontal, 24)
    }

    private var inCall: some View {
        // 0.9.66: o quadrado grande e SEMPRE o meu e os outros ficam na grelha, sempre pela mesma
        // ordem (antes ia para cima quem falava e cada troca dava um flash). Quem fala fica so
        // com a moldura na cor da app.
        let cells = g.tiles
        return ZStack {
            VStack(spacing: 12) {
                GeometryReader { geo in
                    let w = geo.size.width
                    VStack(spacing: 12) {
                        selfTile.frame(width: w * 0.72, height: w * 0.72)
                        if cells.isEmpty {
                            Text(s("gcall_waiting")).font(.sTitleMedium).foregroundStyle(.white.opacity(0.85))
                                .multilineTextAlignment(.center)
                        } else {
                            ForEach(Array(stride(from: 0, to: cells.count, by: 2)), id: \.self) { i in
                                HStack(spacing: 12) {
                                    peerTile(cells[i], speaking: cells[i].contactId == g.speaker)
                                        .frame(width: (w - 12) / 2, height: (w - 12) / 2 / 1.1)
                                    if i + 1 < cells.count {
                                        peerTile(cells[i + 1], speaking: cells[i + 1].contactId == g.speaker)
                                            .frame(width: (w - 12) / 2, height: (w - 12) / 2 / 1.1)
                                    } else {
                                        Color.clear.frame(width: (w - 12) / 2, height: 1)
                                    }
                                }
                            }
                        }
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 56)
            .padding(.bottom, 12)
            VStack {
                if shown || !autoHide {
                    HStack {
                        Text(g.groupName).font(.sTitleMedium).foregroundStyle(.white).lineLimit(1)
                        Spacer()
                        GroupCallClock(since: g.startedAt)
                            .onLongPressGesture { if diagnostic != nil { showDiag = true } }
                    }
                    .padding(.horizontal, 20).padding(.vertical, 12)
                    .transition(.opacity)
                }
                Spacer()
                if shown || !autoHide {
                    VStack(spacing: 16) {
                        HStack(spacing: 12) {
                            ToggleRound(icon: g.muted ? "mic.slash.fill" : "mic.fill", label: s("t_silenciar"), on: g.muted) { touches += 1; onMute() }
                            ToggleRound(icon: "speaker.wave.2.fill", label: s("t_altifalante"), on: g.speakerOn) { touches += 1; onSpeaker() }
                            ToggleRound(icon: g.camera ? "video.fill" : "video.slash.fill", label: s("call_camera"), on: g.camera) { touches += 1; onCamera() }
                            ToggleRound(icon: "arrow.triangle.2.circlepath.camera", label: s("video_switch"), on: false) { touches += 1; onSwitchCamera() }
                                .disabled(!g.camera).opacity(g.camera ? 1 : 0.45)
                        }
                        .padding(.horizontal, 12).padding(.vertical, 10)
                        .background(Capsule().fill(Color.black.opacity(0.65)))
                        HangupButton { touches += 1; onLeave() }
                    }
                    .padding(.bottom, 20)
                    .transition(.opacity)
                }
            }
        }
    }

    @ViewBuilder
    private func cell(_ t: GroupTile?) -> some View {
        if let t { peerTile(t, speaking: t.contactId == g.speaker) } else { selfTile }
    }

    private func tileBorder(_ speaking: Bool) -> some View {
        RoundedRectangle(cornerRadius: 18)
            .strokeBorder(
                speaking ? AnyShapeStyle(LinearGradient(colors: Array(c.topBarGradient.dropFirst()), startPoint: .topLeading, endPoint: .bottomTrailing))
                    : AnyShapeStyle(Color.white.opacity(0.18)),
                lineWidth: speaking ? 2 : 1
            )
    }

    private func peerTile(_ t: GroupTile, speaking: Bool) -> some View {
        ZStack(alignment: .bottom) {
            LinearGradient(colors: Array(c.topBarGradient.prefix(2)), startPoint: .top, endPoint: .bottom)
            if t.camera, t.connected, let layer = remoteLayer(t.contactId) {
                RemoteVideoView(layer: layer, rotation: t.rotation)
            } else {
                Avatar(name: t.name, seed: t.contactId, size: 64).opacity(t.connected ? 1 : 0.5)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            label(t.name, camera: t.camera, muted: false)
        }
        .clipShape(RoundedRectangle(cornerRadius: 18))
        .overlay(tileBorder(speaking))
    }

    private var selfTile: some View {
        ZStack(alignment: .bottom) {
            LinearGradient(colors: Array(c.topBarGradient.prefix(2)), startPoint: .top, endPoint: .bottom)
            if g.camera, let session = selfSession {
                SelfVideoView(session: session)
            } else {
                Avatar(name: myName, seed: "me", size: 64).frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            label(myName, camera: g.camera, muted: g.muted)
        }
        .clipShape(RoundedRectangle(cornerRadius: 18))
        .overlay(tileBorder(g.meSpeaking))
    }

    private func label(_ name: String, camera: Bool, muted: Bool) -> some View {
        HStack(spacing: 6) {
            Image(systemName: camera ? "video.fill" : (muted ? "mic.slash.fill" : "mic.fill")).font(.system(size: 13))
            Text(name.uppercased()).font(.sLabelLarge).lineLimit(1)
        }
        .foregroundStyle(.white)
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
        .background(LinearGradient(colors: [.clear, Color.black.opacity(0.6)], startPoint: .top, endPoint: .bottom))
    }
}

private struct GroupCallClock: View {
    let since: Double
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            let sec = max(0, Int(monotonicSeconds() - since))
            Text(String(format: "%d:%02d", sec / 60, sec % 60)).font(.sTitleMedium).foregroundStyle(.white)
        }
    }
}
