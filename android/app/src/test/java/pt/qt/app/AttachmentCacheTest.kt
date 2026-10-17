package pt.qt.app

import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Cache dos anexos decifrados: limite total, ordem de uso e limpeza a zeros. */
class AttachmentCacheTest {
    private val mb = 1024 * 1024

    @After
    fun limpa() = AttachmentCache.clear()

    @Test
    fun guardaEDevolve() {
        val b = ByteArray(10) { it.toByte() }
        AttachmentCache.put("c", "1", b)
        assertArrayEquals(b, AttachmentCache.get("c", "1"))
        assertNull(AttachmentCache.get("c", "2"))
        assertNull(AttachmentCache.get("outra", "1"))
    }

    @Test
    fun saemOsUsadosHaMaisTempo() {
        AttachmentCache.put("c", "a", ByteArray(15 * mb))
        AttachmentCache.put("c", "b", ByteArray(15 * mb))
        AttachmentCache.get("c", "a")   // "a" passa a ser o mais recente
        AttachmentCache.put("c", "d", ByteArray(15 * mb))   // 45 MB > 40 MB: sai "b"
        assertNotNull(AttachmentCache.get("c", "a"))
        assertNull(AttachmentCache.get("c", "b"))
        assertNotNull(AttachmentCache.get("c", "d"))
    }

    @Test
    fun maiorQueOLimiteNaoEntra() {
        AttachmentCache.put("c", "x", ByteArray((AttachmentCache.MAX_BYTES + 1).toInt()))
        assertNull(AttachmentCache.get("c", "x"))
    }

    @Test
    fun limparPoeAZeros() {
        val b = ByteArray(100) { 7 }
        AttachmentCache.put("c", "1", b)
        AttachmentCache.clear()
        assertNull(AttachmentCache.get("c", "1"))
        assertTrue(b.all { it == 0.toByte() })
    }
}
