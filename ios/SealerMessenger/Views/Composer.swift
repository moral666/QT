import AVFoundation
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// Caixa de escrita das conversas (a dois e de grupo): texto, fotos e GIFs (galeria ou
/// colados), ficheiros ate 25 MB e mensagens de voz ate 10 s (o botao grava quando a caixa
/// esta vazia). Igual ao ChatComposer do Android.
struct ChatComposer: View {
    let onSendText: (String) -> Void
    let onSendImage: (Data, Bool, Bool) -> Void
    let onSendFile: (URL) -> Void
    let onSendVoice: (URL, Int64) -> Void
    /// Texto que nao chegou a ser enviado (0.9.23): volta a caixa se ela estiver vazia.
    var restoreText: String? = nil
    var onRestored: () -> Void = {}

    @State private var text = ""
    @State private var showPhotos = false
    @State private var photoItem: PhotosPickerItem?
    @State private var picked: PickedImage?
    @State private var showFiles = false
    @State private var pickedFile: URL?
    @State private var noImageToPaste = false
    @State private var micDenied = false
    @StateObject private var recorder = VoiceRecorder()
    @Environment(\.chat) private var c
    @Environment(\.openURL) private var openURL
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            if recorder.recording {
                VoiceRecordBar(recorder: recorder, onCancel: { recorder.cancel() }, onSend: sendVoice)
            } else {
                inputRow
            }
        }
        .background(c.chatBackground)
        .onDisappear { recorder.cancel() }   // sair da conversa apaga a gravacao a meio
        // A app saiu do ecra a meio de uma gravacao: cancela (0.9.23). Com o audio em segundo
        // plano (por causa das chamadas) o iPhone continuava a gravar e, ao voltar, a barra
        // enviava a gravacao sozinha.
        .onChange(of: scenePhase) { _, phase in if phase == .background { recorder.cancel() } }
        .onChange(of: restoreText) { _, t in
            guard let t else { return }
            if text.isEmpty { text = t }
            onRestored()
        }
        .photosPicker(isPresented: $showPhotos, selection: $photoItem, matching: .images)
        .onChange(of: photoItem) { _, item in
            guard let item else { return }
            photoItem = nil
            Task { @MainActor in
                // Os bytes originais (um GIF continua animado; a foto e reduzida depois).
                if let data = try? await item.loadTransferable(type: Data.self) { picked = PickedImage(data: data, pasted: false) }
            }
        }
        .sheet(item: $picked) { p in SendImageSheet(picked: p) { original in onSendImage(p.data, original, p.pasted) } }
        .fileImporter(isPresented: $showFiles, allowedContentTypes: [.item]) { result in
            // So o ficheiro escolhido fica acessivel a app (e so enquanto e lido).
            if case .success(let url) = result { pickedFile = url }
        }
        .sheet(isPresented: Binding(get: { pickedFile != nil }, set: { if !$0 { discardPickedFile() } })) {
            if let u = pickedFile {
                SendFileSheet(url: u, onSend: { onSendFile(u); pickedFile = nil }, onCancel: discardPickedFile)
            }
        }
        .alert(s("ios_no_image_to_paste"), isPresented: $noImageToPaste) { Button("OK", role: .cancel) {} } message: {
            Text(s("ios_no_image_to_paste_hint"))
        }
        .alert(s("t_sem_acesso_ao_microfone_nao_e"), isPresented: $micDenied) {
            Button(s("ios_open_settings")) { if let u = URL(string: UIApplication.openSettingsURLString) { openURL(u) } }
            Button(s("t_cancelar"), role: .cancel) {}
        }
    }

    private var inputRow: some View {
        HStack(alignment: .bottom, spacing: 6) {
            Menu {
                Button(s("t_foto_ou_gif"), systemImage: "photo.on.rectangle") { showPhotos = true }
                Button(s("t_ficheiro_ate_25_mb"), systemImage: "doc") { showFiles = true }
                Button(s("ios_paste_image"), systemImage: "doc.on.clipboard") { pasteImage() }
            } label: {
                Image(systemName: "plus").font(.system(size: 22, weight: .medium))
                    .foregroundStyle(c.primary)
                    .frame(width: 44, height: 52)
            }
            .accessibilityLabel(s("t_enviar_foto_gif_ou_ficheiro"))
            TextField(s("t_mensagem"), text: $text, axis: .vertical)
                .font(.sBodyLarge)
                .lineLimit(1...5)
                .padding(.horizontal, 16).padding(.vertical, 14)
                .background(RoundedRectangle(cornerRadius: 24).fill(c.surface))
            let empty = text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            Button {
                if !empty {
                    onSendText(text)
                    text = ""
                } else {
                    startRecording()
                }
            } label: {
                Image(systemName: empty ? "mic.fill" : "paperplane.fill").font(.system(size: 20))
                    .flipsForRightToLeftLayoutDirection(!empty)   // em arabe o aviao aponta para a esquerda
                    .foregroundStyle(c.onPrimary)
                    .frame(width: 52, height: 52)
                    .background(Circle().fill(c.primary))
            }
            .accessibilityLabel(empty ? s("ios_record_voice") : s("t_enviar"))
        }
        .padding(.horizontal, 8).padding(.vertical, 6)
    }

    private func startRecording() {
        func go() {
            do { try recorder.start() } catch { micDenied = true }
        }
        switch AVAudioApplication.shared.recordPermission {
        case .granted: go()
        case .denied: micDenied = true
        default:
            AVAudioApplication.requestRecordPermission { ok in
                Task { @MainActor in if ok { go() } else { micDenied = true } }
            }
        }
    }

    private func sendVoice() {
        if let r = recorder.stop() { onSendVoice(r.0, r.1) }
    }

    private func discardPickedFile() {
        pickedFile = nil   // o original fica onde estava (nunca se apaga)
    }

    /// GIF/imagem copiada (os teclados de GIFs do iPhone copiam para a area de transferencia).
    private func pasteImage() {
        let pb = UIPasteboard.general
        for t in [UTType.gif, .png, .webP, .jpeg, .heic] {
            if let d = pb.data(forPasteboardType: t.identifier) {
                picked = PickedImage(data: d, pasted: true)
                return
            }
        }
        if let img = pb.image, let d = img.pngData() {
            picked = PickedImage(data: d, pasted: true)
            return
        }
        noImageToPaste = true
    }
}

/// Barra de gravacao: tempo a contar ate 10 s, Cancelar e Enviar. Para sozinha aos 10 s.
struct VoiceRecordBar: View {
    @ObservedObject var recorder: VoiceRecorder
    let onCancel: () -> Void
    let onSend: () -> Void
    @State private var elapsed: Int64 = 0
    @Environment(\.chat) private var c

    var body: some View {
        HStack(spacing: 10) {
            Circle().fill(Color(hex: 0xE53935)).frame(width: 12, height: 12)
            Text("\(formatVoice(elapsed)) / \(formatVoice(Voice.maxMs))").font(.sBodyLarge).monospacedDigit()
            ProgressView(value: min(1, Double(elapsed) / Double(Voice.maxMs))).tint(c.primary)
            Button(s("t_cancelar"), action: onCancel).font(Jakarta.font(14, .semibold))
            Button(s("t_enviar"), action: onSend).font(Jakarta.font(14, .semibold))
        }
        .padding(.horizontal, 12).padding(.vertical, 14)
        .task {
            while recorder.recording && !Task.isCancelled {
                elapsed = recorder.elapsedMs()
                if elapsed >= Voice.maxMs { onSend(); break }
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
        }
    }
}

/// Confirmacao antes de enviar um ficheiro escolhido.
struct SendFileSheet: View {
    let url: URL
    let onSend: () -> Void
    let onCancel: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        let (name, size) = FilePrep.describe(url)
        let tooBig = (size ?? 0) > FilePrep.maxBytes
        VStack(alignment: .leading, spacing: 10) {
            Text(s("t_enviar_ficheiro")).font(Jakarta.font(22, .bold))
            Text(name).font(.sBodyLarge).lineLimit(3)
            if let size { Text(formatSize(size)).font(.sBodyMedium) }
            Text(tooBig ? s("t_o_ficheiro_tem_mais_de_25") : s("t_vai_tal_como_esta_cifrado_no"))
                .font(.sBodySmall).foregroundStyle(tooBig ? c.error : c.onSurfaceVariant)
            HStack {
                Spacer()
                Button(s("t_cancelar"), action: onCancel)
                Button(s("t_enviar"), action: onSend).disabled(tooBig).padding(.leading, 16)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.medium])
        .presentationBackground(c.surface)
    }
}
