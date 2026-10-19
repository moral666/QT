import SwiftUI

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
            switch phase {
            case .active: model.onAppVisible()
            case .background: model.onAppHidden()
            default: break   // .inactive: Face ID, centro de controlo, pedidos de permissao
            }
        }
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
