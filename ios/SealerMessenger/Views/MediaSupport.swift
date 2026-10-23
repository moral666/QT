import AVFoundation
import ImageIO
import QuickLook
import SwiftUI
import UIKit
import UniformTypeIdentifiers

// ===================================================================== foto de perfil

/// Foto de perfil: a original pode ter ate 20 MB; vai para os contactos um quadrado pequeno
/// em JPEG que caiba no limite do Rust (9 973 bytes, `MAX_PHOTO_BYTES`). O iPhone nao grava
/// WebP; o Rust aceita JPEG e WebP, e o Android mostra os dois. Voltar a gravar apaga os
/// metadados (incluindo a localizacao). Tudo em memoria: nada em claro no disco.
enum ProfilePhoto {
    static let maxInputBytes = 20 * 1024 * 1024
    /// Com folga em relacao ao Rust (igual ao Android).
    private static let maxOutBytes = 9_900
    /// Fotos recebidas maiores do que isto nao se mostram (defesa contra imagens estranhas).
    private static let maxShownSide = 1024

    static func prepare(_ data: Data) throws -> Data {
        guard data.count <= maxInputBytes else { throw ImagePrepError.tooBig }
        guard let src = CGImageSourceCreateWithData(data as CFData, nil) else { throw ImagePrepError.unreadable }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,   // rotacao da camara
            kCGImageSourceThumbnailMaxPixelSize: 1024,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { throw ImagePrepError.unreadable }
        // Quadrado ao centro.
        let side = min(cg.width, cg.height)
        let crop = CGRect(x: (cg.width - side) / 2, y: (cg.height - side) / 2, width: side, height: side)
        guard let sq = cg.cropping(to: crop) else { throw ImagePrepError.unreadable }
        for px in [256, 224, 192, 160, 128] {
            let img = render(sq, px)
            for q in [0.8, 0.7, 0.6, 0.5, 0.4, 0.3] {
                if let jpeg = img.jpegData(compressionQuality: q), jpeg.count <= maxOutBytes { return jpeg }
            }
        }
        throw ImagePrepError.unreadable
    }

    private static func render(_ cg: CGImage, _ px: Int) -> UIImage {
        let fmt = UIGraphicsImageRendererFormat()
        fmt.scale = 1
        fmt.opaque = true
        let size = CGSize(width: px, height: px)
        return UIGraphicsImageRenderer(size: size, format: fmt).image { ctx in
            UIColor.white.setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            UIImage(cgImage: cg).draw(in: CGRect(origin: .zero, size: size))
        }
    }

    /// Foto recebida (bytes do Rust) para mostrar; nil se nao for uma imagem aceitavel.
    static func decode(_ bytes: Data) -> UIImage? {
        guard let src = CGImageSourceCreateWithData(bytes as CFData, nil),
              let p = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any],
              let w = p[kCGImagePropertyPixelWidth] as? Int, let h = p[kCGImagePropertyPixelHeight] as? Int,
              w > 0, h > 0, max(w, h) <= maxShownSide else { return nil }
        return ImageDecode.image(bytes, maxPixel: 256)
    }
}

/// "Daqui a 2 d e 5 h" / "daqui a 3 h" / "daqui a menos de 1 h".
func waitText(_ ms: Int64) -> String {
    let h = (ms + 3_599_999) / 3_600_000
    if h <= 1 { return s("t_daqui_a_menos_de_1_h") }
    if h < 24 { return s("t_daqui_a_h", h) }
    if h % 24 == 0 { return s("t_daqui_a_d", h / 24) }
    return s("t_daqui_a_d_e_h", h / 24, h % 24)
}

// ========================================================================= ficheiros

enum FilePrepError: Error { case tooBig, unreadable }

/// Ficheiro temporario pronto a cifrar pelo Rust (apagar depois de `attachmentCreateFile`).
struct PreparedFile: Sendable {
    let url: URL
    let name: String
    let mime: String
}

/// Ficheiros (PDF, documentos...): vao tal como estao, sem compressao, ate 25 MB. O nome e o
/// tipo sao limpos aqui e validados outra vez no Rust (igual ao Android).
enum FilePrep {
    static let maxBytes: Int64 = 25 * 1024 * 1024
    private static let maxNameBytes = 200

    /// Nome e tamanho de um ficheiro escolhido.
    static func describe(_ url: URL) -> (String, Int64?) {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let values = try? url.resourceValues(forKeys: [.nameKey, .fileSizeKey])
        return (cleanName(values?.name ?? url.lastPathComponent), values?.fileSize.map { Int64($0) })
    }

    static func prepare(_ url: URL) throws -> PreparedFile {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let (name, size) = describe(url)
        if let size, size > maxBytes { throw FilePrepError.tooBig }
        let out = FileManager.default.temporaryDirectory.appendingPathComponent("envio-\(UUID().uuidString)")
        guard let input = InputStream(url: url) else { throw FilePrepError.unreadable }
        input.open()
        defer { input.close() }
        FileManager.default.createFile(atPath: out.path, contents: nil, attributes: [.protectionKey: FileProtectionType.complete])
        guard let handle = try? FileHandle(forWritingTo: out) else { throw FilePrepError.unreadable }
        var total: Int64 = 0
        var buf = [UInt8](repeating: 0, count: 64 * 1024)
        do {
            defer { try? handle.close() }
            while true {
                let n = input.read(&buf, maxLength: buf.count)
                if n < 0 { throw FilePrepError.unreadable }
                if n == 0 { break }
                total += Int64(n)
                if total > maxBytes { throw FilePrepError.tooBig }
                try handle.write(contentsOf: buf[0..<n])
            }
            if total == 0 { throw FilePrepError.unreadable }
        } catch {
            try? FileManager.default.removeItem(at: out)
            throw error
        }
        let mime = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType
        return PreparedFile(url: out, name: name, mime: cleanMime(mime))
    }

    /// Sem caminhos nem caracteres de controlo, ate 200 bytes (mantendo a extensao).
    static func cleanName(_ raw: String?) -> String {
        let cleaned = String((raw ?? "").map { c -> Character in
            (c == "/" || c == "\\" || c.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) }) ? "_" : c
        }).trimmingCharacters(in: .whitespaces)
        if cleaned.isEmpty || cleaned == "." || cleaned == ".." { return "ficheiro" }
        if cleaned.utf8.count <= maxNameBytes { return cleaned }
        let ext: String = {
            guard let dot = cleaned.lastIndex(of: "."), dot != cleaned.startIndex else { return "" }
            let e = String(cleaned[dot...])
            return e.count <= 16 ? e : ""
        }()
        var base = String(cleaned.dropLast(ext.count))
        while base.utf8.count + ext.utf8.count > maxNameBytes { base.removeLast() }
        return base.trimmingCharacters(in: .whitespaces) + ext
    }

    /// "tipo/subtipo" em minusculas; senao `application/octet-stream`.
    static func cleanMime(_ raw: String?) -> String {
        let m = (raw ?? "").split(separator: ";").first.map { $0.trimmingCharacters(in: .whitespaces).lowercased() } ?? ""
        let ok = m.count <= 100 && m.range(of: #"^[a-z0-9!#$&^_.+-]+/[a-z0-9!#$&^_.+-]+$"#, options: .regularExpression) != nil
        return ok ? m : "application/octet-stream"
    }
}

/// "350 KB", "2,4 MB". Comeca com LRM: num texto em arabe fica "2,4 MB" e nao "MB 2,4".
func formatSize(_ bytes: Int64) -> String {
    if bytes < 1024 { return "\u{200E}\(bytes) B" }
    if bytes < 1024 * 1024 { return "\u{200E}\((bytes + 512) / 1024) KB" }
    let f = NumberFormatter()
    f.locale = Lang.numbersLocale
    f.minimumFractionDigits = 1
    f.maximumFractionDigits = 1
    return "\u{200E}" + (f.string(from: NSNumber(value: Double(bytes) / (1024 * 1024))) ?? "?") + " MB"
}

/// "PDF", "DOCX"... (para o icone); vazio se nao houver extensao curta.
func fileExtension(_ name: String) -> String {
    guard let dot = name.lastIndex(of: "."), dot != name.startIndex else { return "" }
    let ext = name[name.index(after: dot)...]
    return (!ext.isEmpty && ext.count <= 5) ? ext.uppercased() : ""
}

/// Ver um ficheiro decifrado DENTRO da app (Quick Look), sem o entregar a outra app.
///
/// O Quick Look precisa de um ficheiro: fica numa pasta temporaria com a protecao maxima
/// do iOS (`complete`, ilegivel com o iPhone bloqueado) e e apagado ao fechar, ao bloquear a
/// app e ao abrir a app. "Partilhar" no Quick Look e escolha do utilizador.
enum FilePreview {
    static func write(_ data: Data, name: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("ver-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true,
                                                attributes: [.protectionKey: FileProtectionType.complete])
        let url = dir.appendingPathComponent(FilePrep.cleanName(name))
        try data.write(to: url, options: [.atomic, .completeFileProtection])
        return url
    }

    static func remove(_ url: URL) {
        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
    }

    static func clear() { AppFiles.cleanTemporary() }
}

/// Quick Look dentro do SwiftUI.
struct QuickLookView: UIViewControllerRepresentable {
    let url: URL

    func makeCoordinator() -> Coordinator { Coordinator(url: url) }

    func makeUIViewController(context: Context) -> UINavigationController {
        let ql = QLPreviewController()
        ql.dataSource = context.coordinator
        return UINavigationController(rootViewController: ql)
    }

    func updateUIViewController(_ vc: UINavigationController, context: Context) {}

    final class Coordinator: NSObject, QLPreviewControllerDataSource {
        let url: URL
        init(url: URL) { self.url = url }
        func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }
        func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem { url as NSURL }
    }
}

/// "Guardar": o utilizador escolhe a pasta (Ficheiros, iCloud...). Fica la em claro, por escolha dele.
struct ExportPicker: UIViewControllerRepresentable {
    let url: URL
    let onDone: (Bool) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onDone: onDone) }

    func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
        let p = UIDocumentPickerViewController(forExporting: [url], asCopy: true)
        p.delegate = context.coordinator
        return p
    }

    func updateUIViewController(_ vc: UIDocumentPickerViewController, context: Context) {}

    final class Coordinator: NSObject, UIDocumentPickerDelegate {
        let onDone: (Bool) -> Void
        init(onDone: @escaping (Bool) -> Void) { self.onDone = onDone }
        func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) { onDone(true) }
        func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) { onDone(false) }
    }
}

// =========================================================================== voz

/// Mensagens de voz: AAC em MP4 (o Rust aceita `audio/mp4` ate 10 s), iguais as do Android.
enum Voice {
    static let mime = "audio/mp4"
    static let maxMs: Int64 = 10_000
    /// Gravacoes mais curtas do que isto sao descartadas (toque sem querer).
    static let minMs: Int64 = 600
}

/// "0:07".
func formatVoice(_ ms: Int64) -> String {
    let sec = max(0, ms / 1000)
    return "\(sec / 60):" + String(format: "%02d", sec % 60)
}

/// Grava do microfone para um ficheiro temporario (protecao maxima). O ficheiro so existe
/// ate ser cifrado pelo Rust e e apagado logo a seguir, ou ao cancelar.
@MainActor
final class VoiceRecorder: ObservableObject {
    private var recorder: AVAudioRecorder?
    private var file: URL?
    private var startedAt: Double = 0
    @Published private(set) var recording = false

    func start() throws {
        cancel()
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker, .allowBluetooth])
        try session.setActive(true)
        // Pasta com protecao maxima ANTES de gravar (0.9.23): o ficheiro nasce ja protegido
        // (antes so recebia a protecao depois de a gravacao comecar). "voz-" e limpa ao arrancar.
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("voz-gravacoes", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true,
                                                attributes: [.protectionKey: FileProtectionType.complete])
        try? FileManager.default.setAttributes([.protectionKey: FileProtectionType.complete], ofItemAtPath: dir.path)
        let f = dir.appendingPathComponent("voz-\(UUID().uuidString).m4a")
        FileManager.default.createFile(atPath: f.path, contents: nil, attributes: [.protectionKey: FileProtectionType.complete])
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 44_100,
            AVNumberOfChannelsKey: 1,
            AVEncoderBitRateKey: 48_000,
        ]
        let r = try AVAudioRecorder(url: f, settings: settings)
        // Rede de seguranca: quem para aos 10 s e o ecra.
        guard r.record(forDuration: Double(Voice.maxMs + 400) / 1000) else {
            try? FileManager.default.removeItem(at: f)
            throw SecureStoreError(message: "microfone indisponivel")
        }
        try? (f as NSURL).setResourceValue(URLFileProtection.complete, forKey: .fileProtectionKey)
        recorder = r
        file = f
        startedAt = monotonicSeconds()
        recording = true
    }

    func elapsedMs() -> Int64 { recorder == nil ? 0 : Int64((monotonicSeconds() - startedAt) * 1000) }

    /// Para e devolve o ficheiro e a duracao, ou nil se foi curta demais ou falhou.
    func stop() -> (URL, Int64)? {
        guard let r = recorder else { return nil }
        let ms = min(elapsedMs(), Voice.maxMs)
        r.stop()
        recorder = nil
        recording = false
        let f = file
        file = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        guard let f, ms >= Voice.minMs,
              let size = (try? f.resourceValues(forKeys: [.fileSizeKey]))?.fileSize, size > 0 else {
            if let f { try? FileManager.default.removeItem(at: f) }
            return nil
        }
        return (f, ms)
    }

    func cancel() {
        if let r = stop() { try? FileManager.default.removeItem(at: r.0) }
    }
}

/// Toca uma mensagem de voz diretamente da memoria (decifrada pelo Rust), sem a escrever em disco.
@MainActor
final class VoicePlayer: NSObject, ObservableObject, AVAudioPlayerDelegate {
    @Published private(set) var playing = false
    @Published private(set) var positionMs: Int64 = 0
    private var player: AVAudioPlayer?
    private var timer: Timer?

    func toggle(_ data: Data) {
        if playing {
            player?.pause()
            playing = false
            timer?.invalidate()
            return
        }
        if player == nil {
            try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
            try? AVAudioSession.sharedInstance().setActive(true)
            player = try? AVAudioPlayer(data: data, fileTypeHint: AVFileType.m4a.rawValue)
            player?.delegate = self
        }
        guard let p = player, p.play() else { return }
        playing = true
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, let p = self.player else { return }
                self.positionMs = Int64(p.currentTime * 1000)
            }
        }
    }

    func stop() {
        timer?.invalidate()
        let had = player != nil
        player?.stop()
        player = nil
        playing = false
        positionMs = 0
        // Liberta o audio (senao o CallKit pode nao conseguir ativa-lo numa chamada).
        if had { try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation) }
    }

    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        Task { @MainActor in
            self.timer?.invalidate()
            self.playing = false
            self.positionMs = 0
            self.player?.currentTime = 0
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
    }
}
