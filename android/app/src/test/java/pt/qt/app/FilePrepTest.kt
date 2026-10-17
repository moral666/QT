package pt.qt.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Nomes e tipos dos ficheiros a enviar: o que o Rust aceita (ver ffi/src/attachments.rs). */
class FilePrepTest {
    private fun bytes(s: String) = s.toByteArray(Charsets.UTF_8).size

    @Test
    fun nomeNormalFicaIgual() {
        assertEquals("Relatório final.pdf", FilePrep.cleanName("Relatório final.pdf"))
    }

    @Test
    fun semCaminhosNemControlo() {
        assertEquals("pasta_segredo.txt", FilePrep.cleanName("pasta/segredo.txt"))
        assertEquals("a_b.txt", FilePrep.cleanName("a\\b.txt"))
        assertEquals("linha_nova.txt", FilePrep.cleanName("linha\nnova.txt"))
        assertFalse(FilePrep.cleanName("../../etc/passwd").contains('/'))
    }

    @Test
    fun vazioOuPontosPassaAFicheiro() {
        for (n in listOf(null, "", "   ", ".", "..")) assertEquals("ficheiro", FilePrep.cleanName(n))
    }

    @Test
    fun nomeComprido() {
        val n = FilePrep.cleanName("a".repeat(300) + ".pdf")
        assertTrue(bytes(n) <= 200)
        assertTrue(n.endsWith(".pdf"))
        // Carateres de varios bytes nunca sao partidos ao meio.
        val e = FilePrep.cleanName("ã".repeat(150) + ".docx")
        assertTrue(bytes(e) <= 200)
        assertTrue(e.endsWith(".docx"))
        assertFalse(e.contains('�'))
        val emoji = FilePrep.cleanName("😀".repeat(80))
        assertTrue(bytes(emoji) <= 200)
        assertEquals(0, emoji.length % 2)
    }

    @Test
    fun tipos() {
        assertEquals("application/pdf", FilePrep.cleanMime("application/pdf"))
        assertEquals("text/plain", FilePrep.cleanMime("Text/Plain; charset=UTF-8"))
        assertEquals(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            FilePrep.cleanMime("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        )
        for (bad in listOf(null, "", "pdf", "/pdf", "a/", "a b/c", "a/b/c", "x".repeat(101) + "/y")) {
            assertEquals("application/octet-stream", FilePrep.cleanMime(bad))
        }
    }

    @Test
    fun tamanhoEExtensao() {
        assertEquals("900 B", formatSize(900))
        assertEquals("350 KB", formatSize(350L * 1024))
        assertEquals("2,4 MB", formatSize((2.4 * 1024 * 1024).toLong()))
        assertEquals("PDF", fileExtension("ata.pdf"))
        assertEquals("", fileExtension("sem_extensao"))
        assertEquals("", fileExtension(".oculto"))
        assertEquals("", fileExtension("x.extensaolonga"))
    }
}
