package pt.qt.app

import org.junit.Assert.assertEquals
import org.junit.Test

/** O intervalo entre mudancas do nome de um contacto (0.9.72): 10 minutos, a primeira e logo. */
class ContactRenameTest {
    private val pid = "qt1:" + "ab".repeat(64)

    @Test
    fun primeiraMudancaELogo() {
        assertEquals(0L, Contact("Ana", pid, "r").renameWaitMs(now = 1_000L))
    }

    @Test
    fun depoisDeMudarEsperaDezMinutos() {
        val c = Contact("Ana", pid, "r", renamedAt = 1_000_000L)
        assertEquals(CONTACT_RENAME_COOLDOWN_MS, c.renameWaitMs(now = 1_000_000L))
        assertEquals(60_000L, c.renameWaitMs(now = 1_000_000L + CONTACT_RENAME_COOLDOWN_MS - 60_000L))
        assertEquals(0L, c.renameWaitMs(now = 1_000_000L + CONTACT_RENAME_COOLDOWN_MS))
    }

    @Test
    fun relogioAtrasadoContaOIntervaloInteiro() {
        val c = Contact("Ana", pid, "r", renamedAt = 5_000_000L)
        assertEquals(CONTACT_RENAME_COOLDOWN_MS, c.renameWaitMs(now = 1_000L))
    }

    @Test
    fun minutosArredondadosParaCima() {
        assertEquals(10L, renameWaitMinutes(CONTACT_RENAME_COOLDOWN_MS))
        assertEquals(1L, renameWaitMinutes(1L))
        assertEquals(0L, renameWaitMinutes(0L))
    }
}
