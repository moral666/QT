import SwiftUI
import UIKit

@main
struct SealerMessengerApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @ObservedObject private var model = AppModel.shared
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            SealerTheme {
                ZStack {
                    AppRoot(model: model)
                    // Equivalente ao FLAG_SECURE do Android: esconde as conversas na lista de apps
                    // recentes (o iOS tira uma "fotografia" do ecra quando a app sai do primeiro plano).
                    if scenePhase != .active {
                        PrivacyCover()
                    }
                }
            }
        }
        .onChange(of: scenePhase) { _, phase in
            // A capa tambem tapa as janelas por cima (fotos, documentos, denuncia, numero de
            // seguranca): numa janela propria, acima de todas as da app (0.9.23).
            if phase == .active { PrivacyShield.shared.hide() } else { PrivacyShield.shared.show() }
            switch phase {
            case .active: model.onAppVisible()
            case .background: model.onAppHidden()
            default: break   // .inactive: Face ID, centro de controlo, pedidos de permissao
            }
        }
    }
}

/// Janela da capa de privacidade, acima das `.sheet` e `.fullScreenCover` (que ficam por cima
/// da janela principal e, por isso, da capa que esta dentro dela).
@MainActor
final class PrivacyShield {
    static let shared = PrivacyShield()
    private var window: UIWindow?

    func show() {
        guard window == nil else { return }
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        guard let scene = scenes.first(where: { $0.activationState != .unattached }) ?? scenes.first else { return }
        let w = UIWindow(windowScene: scene)
        w.windowLevel = .alert + 1
        let host = UIHostingController(rootView: SealerTheme { PrivacyCover() })
        host.view.backgroundColor = .clear
        w.rootViewController = host
        w.isHidden = false
        window = w
    }

    func hide() {
        window?.isHidden = true
        window = nil
    }
}

private struct PrivacyCover: View {
    @Environment(\.chat) private var c
    var body: some View {
        ZStack {
            c.background.ignoresSafeArea()
            VStack(spacing: 16) {
                BrandLogo(size: 72)
                Text(Config.appName).font(Jakarta.font(22, .bold))
            }
        }
    }
}
