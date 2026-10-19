package pt.qt.app

import android.Manifest
import android.annotation.SuppressLint
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

/**
 * Notificacoes com a app fechada (Firebase Cloud Messaging), "opcao 1":
 *
 *  - O relay so toca a campainha: pede a Google para acordar este telemovel, com uma
 *    mensagem VAZIA (sem remetente, sem conteudo). Ver server/src/push.rs. Desde a 0.9.6
 *    as chamadas vem marcadas ("call" / "call_end"), para tocarem mesmo com o bloqueio
 *    da app ativo: a Google fica a saber o momento de uma chamada, nada mais.
 *  - A mensagem continua a ser sempre a mesma notificacao generica, "Tens mensagens
 *    novas", sem nome nem texto. Desde a 0.9.6 (chamadas com a app fechada) a app abre a
 *    base de dados em segundo plano, vai buscar o que chegou e, se for uma chamada de um
 *    contacto, toca com o nome ([SealerCore]). Com o bloqueio da app ativo nada se
 *    decifra: so o aviso generico, ou "Chamada SEALER" sem nome.
 *  - O "endereco de campainha" (token FCM) e enviado ao relay sempre que a app se liga
 *    (`MainViewModel.registerPush`). Desligar o interruptor envia um token vazio.
 *
 * O Firebase e iniciado a mao (sem o plugin google-services nem o FirebaseInitProvider),
 * com os valores do `android/app/google-services.json` lidos na build (BuildConfig).
 * Sem esse ficheiro, a app compila e funciona, mas sem notificacoes.
 */
object Push {
    private const val TAG = "SealerPush"
    private const val CHANNEL = "mensagens"
    private const val NOTIFICATION_ID = 1

    /** A app esta no ecra (atualizado pela Activity): entao nao se mostra notificacao. */
    @Volatile var appVisible = false

    /** Esta build tem o Firebase configurado? */
    val available: Boolean get() = BuildConfig.FCM_ENABLED

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("qt_prefs", Context.MODE_PRIVATE)

    /** Interruptor "Notificações" (ligado por omissao). */
    fun isOn(ctx: Context): Boolean = prefs(ctx).getBoolean("push_on", true)

    fun setOn(ctx: Context, on: Boolean) = prefs(ctx).edit().putBoolean("push_on", on).apply()

    /** Inicia o Firebase (uma vez). Devolve false se esta build nao o tiver. */
    fun init(ctx: Context): Boolean {
        if (!available) return false
        return try {
            if (FirebaseApp.getApps(ctx).isEmpty()) {
                val opts = FirebaseOptions.Builder()
                    .setProjectId(BuildConfig.FCM_PROJECT_ID)
                    .setApplicationId(BuildConfig.FCM_APP_ID)
                    .setApiKey(BuildConfig.FCM_API_KEY)
                    .setGcmSenderId(BuildConfig.FCM_SENDER_ID)
                    .build()
                FirebaseApp.initializeApp(ctx.applicationContext, opts)
            }
            true
        } catch (e: Exception) {
            Log.w(TAG, "Firebase nao iniciou: ${e.javaClass.simpleName}")
            false
        }
    }

    /** O token FCM deste telemovel, ou null (sem Google Play Services, sem rede...). */
    suspend fun token(ctx: Context): String? {
        if (!init(ctx)) return null
        return suspendCancellableCoroutine { cont ->
            try {
                FirebaseMessaging.getInstance().token.addOnCompleteListener { t ->
                    cont.resume(if (t.isSuccessful) t.result else null)
                }
            } catch (e: Exception) {
                cont.resume(null)
            }
        }
    }

    /** Esquece o token deste telemovel na Google (ao desligar ou ao apagar o perfil). */
    fun deleteToken(ctx: Context) {
        if (!init(ctx)) return
        try {
            FirebaseMessaging.getInstance().deleteToken()
        } catch (e: Exception) {
            // Sem Google Play Services: nada a apagar.
        }
    }

    /** Android 13+: a permissao de notificacoes ja foi dada? */
    fun permissionGranted(ctx: Context): Boolean =
        Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(ctx, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun ensureChannel(ctx: Context) {
        if (Build.VERSION.SDK_INT < 26) return
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        if (nm.getNotificationChannel(CHANNEL) != null) return
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, s(R.string.t_mensagens), NotificationManager.IMPORTANCE_HIGH).apply {
                description = s(R.string.t_aviso_de_mensagens_novas_sem_o)
            },
        )
    }

    /** "Tens mensagens novas": sempre a mesma, sem nome nem texto. Substitui a anterior. */
    @SuppressLint("MissingPermission")   // verificada em permissionGranted
    fun showGeneric(ctx: Context) {
        if (appVisible || !isOn(ctx) || !permissionGranted(ctx)) return
        ensureChannel(ctx)
        val open = Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val pi = PendingIntent.getActivity(ctx, 0, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_sealer)
            .setContentTitle("SEALER")
            .setContentText(s(R.string.t_tens_mensagens_novas))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        try {
            NotificationManagerCompat.from(ctx).notify(NOTIFICATION_ID, n)
        } catch (e: SecurityException) {
            // Permissao retirada entretanto.
        }
    }

    /** A app abriu: a notificacao ja nao faz falta. */
    fun clear(ctx: Context) {
        NotificationManagerCompat.from(ctx).cancel(NOTIFICATION_ID)
    }
}

/** Recebe a campainha da Google e passa-a ao nucleo (mensagens e chamadas, ver [SealerCore]). */
class SealerPushService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) {
        SealerCore.onPush(applicationContext, message.data["qt"])
    }

    /** Token novo (raro): a app envia-o ao relay da proxima vez que se ligar. */
    override fun onNewToken(token: String) {
        // Nada a fazer aqui: sem a base de dados aberta nao ha ligacao autenticada ao relay.
    }
}
