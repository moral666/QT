import SwiftUI

@main
struct SealerMessengerApp: App {
    @StateObject private var model = AppModel()
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
            if phase == .active { model.becameActive() }
        }
    }
}

private struct PrivacyCover: View {
    @Environment(\.chat) private var c
    var body: some View {
        ZStack {
            c.background.ignoresSafeArea()
            VStack(spacing: 16) {
                BrandMark(size: 72)
                Text(Config.appName).font(.title2.bold())
            }
        }
    }
}
