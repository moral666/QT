import Foundation
import PushKit
import UIKit
import UserNotifications

/// Notificacoes com a app fechada, direto pela Apple (APNs), SEM Firebase nem nenhum SDK
/// de terceiros (igual a "opcao 1" do Android, ver server/src/apns.rs):
///
///  * Mensagens: o relay pede a Apple um alerta com o texto fixo da propria app (`push_new`,
///    "Tens mensagens novas"), sem remetente nem conteudo.
///  * Chamadas: um push VoIP (PushKit) vazio. O iOS obriga a mostrar logo a chamada no
///    CallKit; a app abre a base de dados, ve de quem e e mete o nome (sem o bloqueio da app).
///
/// O token enviado ao relay e `apns:<p|s>:<alertas>:<voip>`. Sem conta paga da Apple (sem a
/// capacidade "Push Notifications" na assinatura) nao ha tokens: a app funciona na mesma,
/// sem notificacoes.
final class PushTokens: NSObject, PKPushRegistryDelegate, UNUserNotificationCenterDelegate {
    static let shared = PushTokens()

    private var alert: String?
    private var voip: String?
    private var registry: PKPushRegistry?

    /// Chamar ao arrancar (AppDelegate): o PushKit tem de estar pronto antes de chegar um push.
    func start() {
        UNUserNotificationCenter.current().delegate = self
        let r = PKPushRegistry(queue: .main)
        r.delegate = self
        r.desiredPushTypes = [.voIP]
        registry = r
        UIApplication.shared.registerForRemoteNotifications()
    }

    /// Pede a permissao de mostrar notificacoes (uma vez; o iOS lembra-se da resposta).
    func askPermission() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    /// Token para o relay, ou nil se ainda nao ha nenhum.
    func relayToken() -> String? {
        guard alert != nil || voip != nil else { return nil }
        return "apns:\(Config.pushEnvironment):\(alert ?? ""):\(voip ?? "")"
    }

    func setAlertToken(_ token: Data) {
        let hex = token.hex
        guard hex != alert else { return }
        alert = hex
        MainActor.assumeIsolated { AppModel.shared.pushTokensChanged() }
    }

    // ---------------------------------------------------------------------------- PushKit

    func pushRegistry(_ registry: PKPushRegistry, didUpdate pushCredentials: PKPushCredentials, for type: PKPushType) {
        guard type == .voIP else { return }
        let hex = pushCredentials.token.hex
        guard hex != voip else { return }
        voip = hex
        MainActor.assumeIsolated { AppModel.shared.pushTokensChanged() }
    }

    func pushRegistry(_ registry: PKPushRegistry, didInvalidatePushTokenFor type: PKPushType) {
        guard type == .voIP else { return }
        voip = nil
        MainActor.assumeIsolated { AppModel.shared.pushTokensChanged() }
    }

    func pushRegistry(_ registry: PKPushRegistry, didReceiveIncomingPushWith payload: PKPushPayload,
                      for type: PKPushType, completion: @escaping () -> Void) {
        guard type == .voIP else { completion(); return }
        // O iOS obriga a mostrar ja uma chamada (senao deixa de entregar pushes VoIP a app).
        let uuid = UUID()
        CallKitBridge.shared.reportIncoming(uuid, name: s("t_chamada_sealer")) { ok in
            if ok { Task { @MainActor in AppModel.shared.onVoipPush(uuid) } }
            completion()
        }
    }

    // ------------------------------------------------------------- notificacoes no ecra

    /// Com a app aberta nao se mostra o aviso (as mensagens ja aparecem na app).
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        let active = UIApplication.shared.applicationState == .active
        completionHandler(active ? [] : [.banner, .sound])
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler completionHandler: @escaping () -> Void) {
        completionHandler()
    }
}

/// Recebe os tokens das notificacoes (o SwiftUI nao tem outra forma de os receber).
final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        // O nucleo abre ja (tambem quando o iOS acorda a app em segundo plano por uma chamada).
        _ = AppModel.shared
        _ = CallKitBridge.shared
        PushTokens.shared.start()
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        PushTokens.shared.setAlertToken(deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // Ex.: assinatura sem "Push Notifications" (conta gratuita da Apple). A app funciona sem elas.
        log("SealerPush", "sem notificacoes: \(error.localizedDescription)")
    }
}
