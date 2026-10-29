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
 * Build debug com o relay de teste nas definicoes: so esse (nome "teste", sem federacao na app).
 */
object Relays {
    private const val PREF_HOME = "relay_home"
    const val TEST_NAME = "teste"

    val all: List<RelayInfo> by lazy {
        val main = RelayInfo(BuildConfig.RELAY_NAME, BuildConfig.RELAY_URL, BuildConfig.RELAY_PIN)
        val others = BuildConfig.RELAYS_OTHERS.split(';').mapNotNull { e ->
            val p = e.trim().split('|').map { it.trim() }
            if (p.size == 3 && p[0].isNotEmpty() && p[1].isNotEmpty()) RelayInfo(p[0], p[1], p[2].lowercase()) else null
        }
        (listOf(main) + others).distinctBy { it.name }
    }

    fun byName(name: String?): RelayInfo? = all.firstOrNull { it.name == name }

    private fun testRelay(prefs: SharedPreferences): RelayInfo? {
        if (!BuildConfig.RELAY_EDITABLE) return null
        val url = prefs.getString("relay_url", null) ?: return null
        return RelayInfo(TEST_NAME, url, prefs.getString("relay_key", null) ?: "")
    }

    /** O relay principal (o de cima da lista, ou o de teste numa build debug). */
    fun main(prefs: SharedPreferences): RelayInfo = testRelay(prefs) ?: all.first()

    /** O relay da conta deste telemovel. */
    fun home(prefs: SharedPreferences): RelayInfo =
        testRelay(prefs) ?: byName(prefs.getString(PREF_HOME, null)) ?: all.first()

    /** Muda o relay da conta. Falso se nao ha federacao na app (relay de teste) ou nao conhece o nome. */
    fun setHome(prefs: SharedPreferences, name: String): Boolean {
        if (testRelay(prefs) != null || byName(name) == null) return false
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
        return if (h.name == main(prefs).name) null else h.name
    }

    /** O relay das chamadas de uma chamada (o de quem liga; sem nome: o principal). */
    fun forCall(prefs: SharedPreferences, name: String?): RelayInfo = byName(name) ?: main(prefs)
}
