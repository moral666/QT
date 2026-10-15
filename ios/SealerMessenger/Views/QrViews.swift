import AVFoundation
import CoreImage.CIFilterBuiltins
import SwiftUI
import UIKit

/// Codigo QR com o ID publico (o mesmo texto "qt1:..." que se partilha ou copia).
/// Gerado e lido no telemovel, sem rede. Mesmas regras que a app Android (Qr.kt).
enum QR {
    static func image(for text: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let small = filter.outputImage else { return nil }
        let big = small.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        guard let cg = CIContext().createCGImage(big, from: big.extent) else { return nil }
        return UIImage(cgImage: cg)
    }

    /// O texto lido e um ID SEALER? Devolve-o limpo, ou nil. O Rust valida-o a seguir.
    static func id(from text: String?) -> String? {
        guard let t = text?.trimmingCharacters(in: .whitespacesAndNewlines),
              t.hasPrefix("qt1:"), (68...400).contains(t.count) else { return nil }
        return t
    }
}

/// Camara a ler codigos QR. `onCode` devolve true quando o codigo serve (a leitura para).
final class QRScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onCode: ((String) -> Bool)?
    var onUnavailable: (() -> Void)?
    private let session = AVCaptureSession()
    private let sessionQueue = DispatchQueue(label: "sealer.qr.session")
    private var preview: AVCaptureVideoPreviewLayer?
    private var finished = false
    private var lastRejected: String?

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            configure()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { granted in
                DispatchQueue.main.async { [weak self] in
                    if granted { self?.configure() } else { self?.onUnavailable?() }
                }
            }
        default:
            DispatchQueue.main.async { [weak self] in self?.onUnavailable?() }
        }
    }

    private func configure() {
        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input) else {
            onUnavailable?()
            return
        }
        session.addInput(input)
        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else {
            onUnavailable?()
            return
        }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]
        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        layer.frame = view.bounds
        view.layer.addSublayer(layer)
        preview = layer
        let s = session
        sessionQueue.async { s.startRunning() }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        stop()
    }

    private func stop() {
        let s = session
        sessionQueue.async { if s.isRunning { s.stopRunning() } }
    }

    // O delegate corre na fila principal (ver setMetadataObjectsDelegate acima).
    nonisolated func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        let text = metadataObjects.lazy
            .compactMap { ($0 as? AVMetadataMachineReadableCodeObject)?.stringValue }
            .first
        MainActor.assumeIsolated {
            guard let text, !self.finished, text != self.lastRejected else { return }
            if self.onCode?(text) == true {
                self.finished = true
                self.stop()
                UINotificationFeedbackGenerator().notificationOccurred(.success)
            } else {
                self.lastRejected = text
            }
        }
    }
}

struct QRScannerView: UIViewControllerRepresentable {
    let onCode: (String) -> Bool
    let onUnavailable: () -> Void

    func makeUIViewController(context: Context) -> QRScannerController {
        let vc = QRScannerController()
        vc.onCode = onCode
        vc.onUnavailable = onUnavailable
        return vc
    }

    func updateUIViewController(_ vc: QRScannerController, context: Context) {
        vc.onCode = onCode
        vc.onUnavailable = onUnavailable
    }
}

/// Ecra inteiro com a camara, para ler o codigo QR de outra pessoa.
struct QRScannerSheet: View {
    let onFound: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var message = "Aponta para o código QR da outra pessoa"
    @State private var problem = false

    var body: some View {
        ZStack(alignment: .bottom) {
            QRScannerView(
                onCode: { text in
                    if let id = QR.id(from: text) {
                        onFound(id)
                        dismiss()
                        return true
                    }
                    message = "Este código QR não é um ID SEALER."
                    problem = true
                    return false
                },
                onUnavailable: {
                    message = "Sem acesso à câmara. Ativa-o em Definições > SEALER > Câmara, ou escreve o ID à mão."
                    problem = true
                }
            )
            .ignoresSafeArea()

            RoundedRectangle(cornerRadius: 20)
                .stroke(Color.white.opacity(0.9), lineWidth: 3)
                .frame(width: 240, height: 240)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .allowsHitTesting(false)

            VStack(spacing: 14) {
                Text(message)
                    .font(.callout.weight(.medium))
                    .multilineTextAlignment(.center)
                    .foregroundStyle(problem ? Color.yellow : Color.white)
                Button("Cancelar") { dismiss() }
                    .font(.body.weight(.semibold))
                    .foregroundStyle(.white)
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 18)
            .frame(maxWidth: .infinity)
            .background(Color.black.opacity(0.55))
        }
        .background(Color.black)
    }
}
