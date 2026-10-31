package pt.qt.app

import android.content.SharedPreferences

/** Um relay da federacao (nome, endereco e chave publica Noise fixada). */
data class RelayInfo(val name: String, val url: String, val pin: String)

/**
 * Relays da federacao fechada (fase 3): o principal da build (`qtRelayUrl`/`qtRelayPin`/`qtRelayName`)
 * e os outros (`qtRelaysOutros`), por esta ordem. A conta vive num deles; a app fala so com esse.
 *
 *  * Sem escolha guardada, a conta esta no principal (todas as contas antigas estao la).
 *  * Conta nova num relay fechado a contas novas: o seguinte da lista.
 *  * O relay diz "conta noutro relay: X" (ex.: recuperada noutro telemovel): muda para X.
 *
 * Sempre os da build, tambem na debug (o relay de teste editavel saiu na 0.9.53).
 */
object Relays {
    private const val PREF_HOME = "relay_home"

    val all: List<RelayInfo> by lazy {
        val first = RelayInfo(BuildConfig.RELAY_NAME, BuildConfig.RELAY_URL, BuildConfig.RELAY_PIN)
        val others = BuildConfig.RELAYS_OTHERS.split(';').mapNotNull { e ->
            val p = e.trim().split('|').map { it.trim() }
            if (p.size == 3 && p[0].isNotEmpty() && p[1].isNotEmpty()) RelayInfo(p[0], p[1], p[2].lowercase()) else null
        }
        (listOf(first) + others).distinctBy { it.name }
    }

    fun byName(name: String?): RelayInfo? = all.firstOrNull { it.name == name }

    /** O relay principal (o de cima da lista). */
    val main: RelayInfo get() = all.first()

    /** O relay da conta deste telemovel. */
    fun home(prefs: SharedPreferences): RelayInfo = byName(prefs.getString(PREF_HOME, null)) ?: all.first()

    /** Muda o relay da conta. Falso se nao conhece o nome. */
    fun setHome(prefs: SharedPreferences, name: String): Boolean {
        if (byName(name) == null) return false
        prefs.edit().putString(PREF_HOME, name).apply()
        return true
    }

    /** Conta nova: volta a comecar pelo principal. */
    fun resetHome(prefs: SharedPreferences) = prefs.edit().remove(PREF_HOME).apply()

    /** O seguinte da lista que ainda nao foi tentado. */
    fun next(tried: Set<String>): RelayInfo? = all.firstOrNull { it.name !in tried }

    /** O nome que vai nas ofertas de chamada: so quando a conta nao esta no principal. */
    fun callName(prefs: SharedPreferences): String? {
        val h = home(prefs)
        return if (h.name == main.name) null else h.name
    }

    /** O relay das chamadas de uma chamada (o de quem liga; sem nome: o principal). */
    fun forCall(prefs: SharedPreferences, name: String?): RelayInfo = byName(name) ?: main
}
