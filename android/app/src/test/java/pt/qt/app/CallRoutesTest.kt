package pt.qt.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Saidas do som das chamadas no sistema de chamadas (0.9.52): o botao e a lista. */
class CallRoutesTest {
    private val phone = AudioRoute("1", RouteKind.PHONE, "Telemovel")
    private val speaker = AudioRoute("2", RouteKind.SPEAKER, "Altifalante")
    private val bt = AudioRoute("3", RouteKind.BLUETOOTH, "Buds")
    private val wired = AudioRoute("4", RouteKind.WIRED, "Auscultadores")

    @Test
    fun soTelemovelEAltifalanteNaoAbreALista() {
        assertFalse(listOf(phone, speaker).needsMenu())
        assertFalse(emptyList<AudioRoute>().needsMenu())
    }

    @Test
    fun bluetoothOuAuscultadoresAbremALista() {
        assertTrue(listOf(phone, speaker, bt).needsMenu())
        assertTrue(listOf(speaker, wired).needsMenu())
    }

    @Test
    fun oBotaoAlternaEntreTelemovelEAltifalante() {
        val routes = listOf(phone, speaker)
        assertEquals(speaker, routes.toggleTarget(phone))
        assertEquals(phone, routes.toggleTarget(speaker))
        // Ainda sem a saida de agora (o sistema ainda nao disse): liga o altifalante.
        assertEquals(speaker, routes.toggleTarget(null))
        // Sem altifalante: nada a fazer.
        assertNull(listOf(phone).toggleTarget(phone))
    }

    @Test
    fun aListaVemPorOrdem() {
        assertEquals(listOf(phone, speaker, bt), listOf(bt, speaker, phone).menuOrder())
        assertEquals(listOf(wired, speaker, bt), listOf(bt, wired, speaker).menuOrder())
    }
}
