import ImageIO
import SwiftUI
import UIKit
import UniformTypeIdentifiers

// ======================================================================== preparar

/// Tira os metadados de um PNG ou WebP sem voltar a gravar a imagem (fica igual, com a
/// transparencia e a animacao). PNG: sem tEXt, zTXt, iTXt, eXIf e tIME. WebP: sem EXIF e XMP.
/// `nil` se o ficheiro estiver mal formado. Igual ao `MetaStrip` do Android.
enum MetaStrip {
    private static let pngDrop: Set<String> = ["tEXt", "zTXt", "iTXt", "eXIf", "tIME"]
    private static let webpDrop: Set<String> = ["EXIF", "XMP "]

    static func strip(_ d: Data, mime: String) -> Data? {
        let b = [UInt8](d)
        switch mime {
        case "image/png": return png(b)
        case "image/webp": return webp(b)
        default: return d
        }
    }

    private static func tag(_ b: [UInt8], _ i: Int) -> String {
        String(decoding: b[i..<(i + 4)], as: UTF8.self)
    }

    private static func png(_ b: [UInt8]) -> Data? {
        guard b.count >= 8 else { return nil }
        var out = Data(b[0..<8])
        var i = 8
        while i + 12 <= b.count {
            let len = Int(b[i]) << 24 | Int(b[i + 1]) << 16 | Int(b[i + 2]) << 8 | Int(b[i + 3])
            let total = 12 + len
            guard len >= 0, i + total <= b.count else { return nil }
            let type = tag(b, i + 4)
            if !pngDrop.contains(type) { out.append(contentsOf: b[i..<(i + total)]) }
            i += total
            if type == "IEND" { return out }
        }
        return nil
    }

    private static func webp(_ b: [UInt8]) -> Data? {
        guard b.count >= 12 else { return nil }
        var body = Data()
        var i = 12
        while i + 8 <= b.count {
            let type = tag(b, i)
            let len = Int(b[i + 4]) | Int(b[i + 5]) << 8 | Int(b[i + 6]) << 16 | Int(b[i + 7]) << 24
            guard i + 8 + len <= b.count else { return nil }
            let end = min(b.count, i + 8 + len + (len & 1))
            if !webpDrop.contains(type) {
                var chunk = Array(b[i..<end])
                // VP8X: tira as marcas "tem EXIF" (0x08) e "tem XMP" (0x04).
                if type == "VP8X", chunk.count > 8 { chunk[8] &= ~UInt8(0x0C) }
                body.append(contentsOf: chunk)
            }
            i = end
        }
        let size = UInt32(body.count + 4)
        var out = Data("RIFF".utf8)
        withUnsafeBytes(of: size.littleEndian) { out.append(contentsOf: $0) }
        out.append(contentsOf: Array("WEBP".utf8))
        out.append(body)
        return out
    }
}

enum ImagePrepError: Error {
    case tooBig
    case unreadable
}

/// Ficheiro temporario pronto a cifrar pelo Rust (apagar depois de `attachmentCreate`).
struct PreparedImage: Sendable {
    let url: URL
    let mime: String
    let width: UInt32
    let height: UInt32
}

/// Prepara uma imagem para enviar, tudo no iPhone (mesmas regras que a app Android):
///  - GIF: vai como esta (para continuar animado);
///  - fotos: reduzidas para 1600 px no lado maior e JPEG 80 (~300 KB), ou, com
///    "qualidade original", ate 4096 px e JPEG 92;
///  - PNG/WebP colados ou em qualidade original: a mesma imagem, sem os metadados (`MetaStrip`,
///    desde a 0.9.23; antes iam tal como estavam).
/// Voltar a gravar a foto apaga os metadados (incluindo a localizacao GPS).
enum ImagePrep {
    static let maxBytes = 25 * 1024 * 1024

    /// O tipo verdadeiro, pelos primeiros bytes.
    static func sniffMime(_ d: Data) -> String? {
        let h = [UInt8](d.prefix(16))
        guard h.count >= 12 else { return nil }
        func at(_ i: Int, _ s: String) -> Bool { Array(s.utf8).enumerated().allSatisfy { h[i + $0.offset] == $0.element } }
        if at(0, "GIF87a") || at(0, "GIF89a") { return "image/gif" }
        if h[0] == 0x89 && at(1, "PNG") { return "image/png" }
        if h[0] == 0xFF && h[1] == 0xD8 && h[2] == 0xFF { return "image/jpeg" }
        if at(0, "RIFF") && at(8, "WEBP") { return "image/webp" }
        if at(4, "ftyp") { return "image/heic" }   // HEIC/HEIF: convertido para JPEG
        return nil
    }

    static func prepare(_ data: Data, original: Bool, keepAsIs: Bool) throws -> PreparedImage {
        guard let mime = sniffMime(data) else { throw ImagePrepError.unreadable }
        let asIs = mime == "image/gif" || ((original || keepAsIs) && (mime == "image/png" || mime == "image/webp"))
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("envio-\(UUID().uuidString)")
        if asIs {
            guard data.count <= maxBytes else { throw ImagePrepError.tooBig }
            // Data, aparelho e as vezes a localizacao: fora (a imagem fica igual, bit a bit).
            let clean: Data
            if mime == "image/gif" { clean = data } else {
                guard let d = MetaStrip.strip(data, mime: mime) else { throw ImagePrepError.unreadable }
                clean = d
            }
            try clean.write(to: url, options: [.atomic, .completeFileProtection])
            let (w, h) = pixelSize(data)
            return PreparedImage(url: url, mime: mime, width: w, height: h)
        }
        guard let src = CGImageSourceCreateWithData(data as CFData, nil) else { throw ImagePrepError.unreadable }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,   // aplica a rotacao da camara
            kCGImageSourceThumbnailMaxPixelSize: original ? 4096 : 1600,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { throw ImagePrepError.unreadable }
        let flat = flatten(cg)
        guard let jpeg = UIImage(cgImage: flat).jpegData(compressionQuality: original ? 0.92 : 0.8) else {
            throw ImagePrepError.unreadable
        }
        guard jpeg.count <= maxBytes else { throw ImagePrepError.tooBig }
        try jpeg.write(to: url, options: [.atomic, .completeFileProtection])
        return PreparedImage(url: url, mime: "image/jpeg", width: UInt32(flat.width), height: UInt32(flat.height))
    }

    private static func pixelSize(_ data: Data) -> (UInt32, UInt32) {
        guard let src = CGImageSourceCreateWithData(data as CFData, nil),
              let p = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any] else { return (1, 1) }
        let w = (p[kCGImagePropertyPixelWidth] as? Int) ?? 1
        let h = (p[kCGImagePropertyPixelHeight] as? Int) ?? 1
        return (UInt32(max(w, 1)), UInt32(max(h, 1)))
    }

    /// JPEG nao tem transparencia: fundo branco em vez de preto.
    private static func flatten(_ cg: CGImage) -> CGImage {
        switch cg.alphaInfo {
        case .none, .noneSkipFirst, .noneSkipLast: return cg
        default: break
        }
        let size = CGSize(width: cg.width, height: cg.height)
        let fmt = UIGraphicsImageRendererFormat()
        fmt.scale = 1
        fmt.opaque = true
        let img = UIGraphicsImageRenderer(size: size, format: fmt).image { ctx in
            UIColor.white.setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            UIImage(cgImage: cg).draw(in: CGRect(origin: .zero, size: size))
        }
        return img.cgImage ?? cg
    }
}

// ========================================================================== mostrar

/// Descodifica para o ecra, reduzida a `maxPixel` (e animada, se for um GIF).
/// As imagens em claro so existem em memoria.
enum ImageDecode {
    /// Memoria maxima para os fotogramas de um GIF (os restantes sao saltados).
    private static let maxAnimationBytes = 48 * 1024 * 1024

    static func image(_ data: Data, maxPixel: Int) -> UIImage? {
        guard let src = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let opts: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixel,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        let count = CGImageSourceGetCount(src)
        guard count > 1 else {
            guard let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { return nil }
            return UIImage(cgImage: cg)
        }
        guard let first = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { return nil }
        let frameBytes = max(first.width * first.height * 4, 1)
        let step = max(1, Int((Double(count * frameBytes) / Double(maxAnimationBytes)).rounded(.up)))
        var frames: [UIImage] = []
        var duration = 0.0
        var i = 0
        while i < count {
            if let cg = i == 0 ? first : CGImageSourceCreateThumbnailAtIndex(src, i, opts as CFDictionary) {
                frames.append(UIImage(cgImage: cg))
            }
            for j in i..<min(i + step, count) { duration += frameDelay(src, j) }
            i += step
        }
        guard !frames.isEmpty else { return nil }
        return UIImage.animatedImage(with: frames, duration: duration > 0 ? duration : Double(frames.count) * 0.1)
    }

    private static func frameDelay(_ src: CGImageSource, _ i: Int) -> Double {
        guard let p = CGImageSourceCopyPropertiesAtIndex(src, i, nil) as? [CFString: Any] else { return 0.1 }
        let gif = p[kCGImagePropertyGIFDictionary] as? [CFString: Any]
        let webp = p[kCGImagePropertyWebPDictionary] as? [CFString: Any]
        let d = (gif?[kCGImagePropertyGIFUnclampedDelayTime] as? Double)
            ?? (gif?[kCGImagePropertyGIFDelayTime] as? Double)
            ?? (webp?[kCGImagePropertyWebPUnclampedDelayTime] as? Double)
            ?? (webp?[kCGImagePropertyWebPDelayTime] as? Double)
            ?? 0.1
        return d < 0.02 ? 0.1 : d   // como os navegadores: atrasos quase nulos passam a 0,1 s
    }

    /// UIImage e imutavel; o involucro so serve para atravessar a tarefa em segundo plano.
    private struct Decoded: @unchecked Sendable { let image: UIImage? }

    static func decode(_ data: Data, maxPixel: Int) async -> UIImage? {
        await Task.detached(priority: .userInitiated) { Decoded(image: image(data, maxPixel: maxPixel)) }.value.image
    }
}

/// UIImageView dentro do SwiftUI: e o que faz os GIFs mexerem (o Image do SwiftUI so
/// mostra o primeiro fotograma).
struct AnimatedImageView: UIViewRepresentable {
    let image: UIImage
    var contentMode: UIView.ContentMode = .scaleAspectFill

    func makeUIView(context: Context) -> UIImageView {
        let v = UIImageView()
        v.clipsToBounds = true
        for axis in [NSLayoutConstraint.Axis.horizontal, .vertical] {
            v.setContentCompressionResistancePriority(.defaultLow, for: axis)
            v.setContentHuggingPriority(.defaultLow, for: axis)
        }
        return v
    }

    func updateUIView(_ v: UIImageView, context: Context) {
        v.contentMode = contentMode
        if v.image !== image { v.image = image }
    }
}

/// Conteudo de uma bolha com imagem/GIF: a imagem, ou o estado da descarga.
struct AttachmentContent: View {
    let m: ChatMessage
    let att: AttachmentInfo
    let progress: Double?
    let load: (ChatMessage) async -> Data?
    let onOpen: () -> Void
    @State private var image: UIImage?

    private var ready: Bool { m.outgoing || att.state == .ready }

    var body: some View {
        // Proporcao limitada: nem bolhas gigantes nem tiras finas.
        let ratio = att.width > 0 && att.height > 0 ? min(max(Double(att.width) / Double(att.height), 0.5), 2) : 1
        ZStack {
            Color.black.opacity(0.08)
            if let image {
                AnimatedImageView(image: image)
            }
            if att.state == .uploading || att.state == .downloading {
                ZStack {
                    Circle().fill(Color.black.opacity(0.45)).frame(width: 56, height: 56)
                    if let progress, progress > 0 {
                        Circle().stroke(Color.white.opacity(0.3), lineWidth: 4).frame(width: 40, height: 40)
                        Circle().trim(from: 0, to: progress)
                            .stroke(Color.white, style: StrokeStyle(lineWidth: 4, lineCap: .round))
                            .rotationEffect(.degrees(-90))
                            .frame(width: 40, height: 40)
                    } else {
                        ProgressView().tint(.white)
                    }
                }
            }
            if let problem {
                VStack {
                    if image != nil { Spacer() }
                    Text(problem)
                        .font(.system(size: 13)).foregroundStyle(.white).multilineTextAlignment(.center)
                        .padding(8).frame(maxWidth: .infinity)
                        .background(Color.black.opacity(0.55))
                }
            }
        }
        .frame(width: 240, height: 240 / ratio)
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .contentShape(Rectangle())
        .onTapGesture { if image != nil { onOpen() } }
        .accessibilityLabel(att.mime == "image/gif" ? "GIF" : s("t_imagem"))
        .task(id: "\(m.messageId)-\(ready)") {
            guard ready, image == nil, let d = await load(m) else { return }
            image = await ImageDecode.decode(d, maxPixel: 720)
        }
    }

    private var problem: String? {
        switch att.state {
        case .expired: return s("t_a_imagem_ja_nao_esta_disponivel")
        case .failed: return m.outgoing ? s("t_nao_foi_possivel_enviar") : s("t_nao_foi_possivel_receber_a_imagem")
        default: return nil
        }
    }
}

/// Imagem em ecra inteiro (toque para fechar).
struct AttachmentViewer: View {
    let m: ChatMessage
    let load: (ChatMessage) async -> Data?
    @Environment(\.dismiss) private var dismiss
    @State private var image: UIImage?

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            if let image {
                AnimatedImageView(image: image, contentMode: .scaleAspectFit).ignoresSafeArea()
            } else {
                ProgressView().tint(.white)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { dismiss() }
        .task {
            guard let d = await load(m) else { return }
            image = await ImageDecode.decode(d, maxPixel: 2048)
        }
    }
}

/// Imagem escolhida (galeria ou colada) a espera de confirmacao.
struct PickedImage: Identifiable {
    let id = UUID()
    let data: Data
    /// Colada (ex.: GIF/autocolante de um teclado): PNG/WebP vao como estao.
    let pasted: Bool
}

/// Pre-visualizacao antes de enviar, com a opcao de qualidade original.
struct SendImageSheet: View {
    let picked: PickedImage
    let onSend: (Bool) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c
    @State private var original = false
    @State private var preview: UIImage?

    private var isGif: Bool { ImagePrep.sniffMime(picked.data) == "image/gif" }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(isGif ? s("t_enviar_gif") : s("t_enviar_imagem")).font(Jakarta.font(22, .bold))
            Group {
                if let preview {
                    AnimatedImageView(image: preview, contentMode: .scaleAspectFit)
                } else {
                    ProgressView()
                }
            }
            .frame(maxWidth: .infinity).frame(height: 300)
            .clipShape(RoundedRectangle(cornerRadius: 12))
            if !isGif {
                Toggle(isOn: $original) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(s("t_qualidade_original"))
                        Text(original ? s("t_mais_nitida_mas_gasta_mais_dados") : s("t_comprimida_envia_mais_depressa_e_gasta"))
                            .font(.sBodySmall).foregroundStyle(c.onSurfaceVariant)
                    }
                }
                .tint(c.primary)
            }
            Text(s("t_cifrada_no_teu_telemovel_antes_de"))
                .font(.footnote).foregroundStyle(c.onSurfaceVariant)
            HStack {
                Spacer()
                Button(s("t_cancelar")) { dismiss() }
                Button(s("t_enviar")) {
                    onSend(original)
                    dismiss()
                }
                .padding(.leading, 16)
            }
            .font(Jakarta.font(15, .semibold))
        }
        .padding(24)
        .presentationDetents([.large])
        .presentationBackground(c.surface)
        .task { preview = await ImageDecode.decode(picked.data, maxPixel: 1200) }
    }
}


// ========================================================================= ficheiros

/// Bolha de um ficheiro: icone com a extensao, nome, tamanho e estado. Tocar abre as opcoes.
struct FileContent: View {
    let m: ChatMessage
    let att: AttachmentInfo
    let progress: Double?
    let onOpen: () -> Void
    @Environment(\.chat) private var c

    var body: some View {
        let name = att.fileName ?? "ficheiro"
        let ready = m.outgoing || att.state == .ready
        let busy = att.state == .uploading || att.state == .downloading
        let ext = fileExtension(name)
        let size = formatSize(Int64(att.size))
        let subtitle: String = {
            switch att.state {
            case .expired: return s("t_ja_nao_esta_disponivel_passaram_mais")
            case .failed: return m.outgoing ? s("t_nao_foi_possivel_enviar") : s("t_nao_foi_possivel_receber")
            case .uploading, .downloading: return "… " + size
            default: return ext.isEmpty ? size : "\(size) · \(ext)"
            }
        }()
        Button { if ready { onOpen() } } label: {
            HStack(spacing: 10) {
                ZStack {
                    RoundedRectangle(cornerRadius: 10).fill(c.primary.opacity(0.15))
                    if busy {
                        if let progress, progress > 0 {
                            ProgressView(value: progress).progressViewStyle(.circular).tint(c.primary)
                        } else {
                            ProgressView().tint(c.primary)
                        }
                    } else {
                        Text(ext.isEmpty ? "📄" : ext).font(Jakarta.font(ext.isEmpty ? 20 : 11, .bold)).foregroundStyle(c.primary).lineLimit(1)
                    }
                }
                .frame(width: 44, height: 44)
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).font(.sBodyMedium).foregroundStyle(c.bubbleText).lineLimit(2).multilineTextAlignment(.leading)
                    Text(subtitle).font(.system(size: 12)).foregroundStyle(c.bubbleMeta).lineLimit(2)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(10)
            .frame(width: 250)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color.black.opacity(0.06)))
        }
        .buttonStyle(.plain)
    }
}

/// Opcoes de um ficheiro: ver dentro da app (Quick Look) ou guardar onde o utilizador escolher.
struct FileActionsView: View {
    let m: ChatMessage
    let load: (ChatMessage) async -> Data?
    @State private var url: URL?
    @State private var failed = false
    @State private var exporting = false
    @State private var saved: Bool?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.chat) private var c

    var body: some View {
        let name = m.attachment?.fileName ?? "ficheiro"
        NavigationStack {
            Group {
                if let url {
                    QuickLookView(url: url).ignoresSafeArea(edges: .bottom)
                } else if failed {
                    Text(s("t_nao_foi_possivel_abrir_o_ficheiro")).font(.sBodyMedium).padding()
                } else {
                    ProgressView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .navigationTitle(name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(s("t_fechar")) { dismiss() } }
                ToolbarItem(placement: .primaryAction) { Button(s("t_guardar")) { exporting = true }.disabled(url == nil) }
            }
        }
        .sheet(isPresented: $exporting) {
            if let url { ExportPicker(url: url) { ok in exporting = false; saved = ok ? true : nil }.ignoresSafeArea() }
        }
        .alert(s("t_ficheiro_guardado"), isPresented: Binding(get: { saved == true }, set: { if !$0 { saved = nil } })) {
            Button("OK", role: .cancel) {}
        }
        .task {
            guard url == nil, let data = await load(m) else { failed = url == nil; return }
            do { url = try FilePreview.write(data, name: name) } catch { failed = true }
        }
        .onDisappear { if let u = url { FilePreview.remove(u) } }
    }
}

// ============================================================================== voz

/// Mensagem de voz na conversa: tocar / pausar, progresso e duracao.
struct VoiceContent: View {
    let m: ChatMessage
    let att: AttachmentInfo
    let progress: Double?
    let load: (ChatMessage) async -> Data?
    @StateObject private var player = VoicePlayer()
    @State private var data: Data?
    @Environment(\.chat) private var c

    var body: some View {
        let ready = m.outgoing || att.state == .ready
        let total = Int64(att.width)   // duracao em ms (ver o Rust)
        let busy = att.state == .uploading || att.state == .downloading
        let problem: String? = {
            switch att.state {
            case .expired: return s("t_ja_nao_esta_disponivel_passaram_mais")
            case .failed: return m.outgoing ? s("t_nao_foi_possivel_enviar") : s("t_nao_foi_possivel_receber")
            default: return nil
            }
        }()
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 10) {
                Button { if let data { player.toggle(data) } } label: {
                    ZStack {
                        Circle().fill(c.primary)
                        if busy && data == nil {
                            ProgressView().tint(.white)
                        } else {
                            Image(systemName: player.playing ? "pause.fill" : "play.fill").foregroundStyle(.white)
                        }
                    }
                    .frame(width: 40, height: 40)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(s("t_ouvir"))
                VStack(alignment: .leading, spacing: 4) {
                    ProgressView(value: total > 0 ? min(1, Double(player.positionMs) / Double(total)) : 0).tint(c.primary)
                    Text(player.playing || player.positionMs > 0 ? formatVoice(player.positionMs) : formatVoice(total))
                        .font(.sLabelSmall).foregroundStyle(c.bubbleMeta)
                }
            }
            if let problem { Text(problem).font(.sLabelSmall).foregroundStyle(c.error) }
        }
        .padding(6)
        .frame(width: 230)
        .task(id: "\(m.messageId)-\(ready)") {
            guard ready, data == nil else { return }
            data = await load(m)
        }
        .onDisappear { player.stop() }
    }
}
