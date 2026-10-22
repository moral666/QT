package pt.qt.app

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream

/** Metadados tirados dos PNG e WebP enviados tal como estao (0.9.23). */
class MetaStripTest {
    private fun be32(n: Int) = byteArrayOf((n ushr 24).toByte(), (n ushr 16).toByte(), (n ushr 8).toByte(), n.toByte())
    private fun le32(n: Int) = byteArrayOf(n.toByte(), (n ushr 8).toByte(), (n ushr 16).toByte(), (n ushr 24).toByte())

    private fun pngChunk(type: String, data: ByteArray): ByteArray =
        be32(data.size) + type.toByteArray(Charsets.ISO_8859_1) + data + byteArrayOf(1, 2, 3, 4)   // CRC nao e verificado

    private val pngSig = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte(), 13, 10, 26, 10)

    private fun webpChunk(type: String, data: ByteArray): ByteArray {
        val pad = if (data.size % 2 == 1) byteArrayOf(0) else byteArrayOf()
        return type.toByteArray(Charsets.ISO_8859_1) + le32(data.size) + data + pad
    }

    private fun riff(vararg chunks: ByteArray): ByteArray {
        val body = ByteArrayOutputStream().apply { chunks.forEach { write(it) } }.toByteArray()
        return "RIFF".toByteArray() + le32(body.size + 4) + "WEBP".toByteArray() + body
    }

    @Test
    fun pngPerdeOsTextosEaDataEFicaComAImagem() {
        val ihdr = pngChunk("IHDR", ByteArray(13) { 1 })
        val idat = pngChunk("IDAT", ByteArray(20) { 7 })
        val iend = pngChunk("IEND", ByteArray(0))
        val png = pngSig + ihdr + pngChunk("tEXt", "Author\u0000Nuno".toByteArray()) +
            pngChunk("eXIf", ByteArray(10)) + pngChunk("tIME", ByteArray(7)) + idat + iend
        val out = MetaStrip.strip(png, "image/png")!!
        assertArrayEquals(pngSig + ihdr + idat + iend, out)
    }

    @Test
    fun pngMalFormadoERecusado() {
        assertNull(MetaStrip.strip(pngSig + be32(1000) + "IDAT".toByteArray(), "image/png"))
        assertNull(MetaStrip.strip(pngSig + pngChunk("IHDR", ByteArray(13)), "image/png"))   // sem IEND
    }

    @Test
    fun webpPerdeExifEXmpETiraAsMarcas() {
        val vp8x = ByteArray(10).also { it[0] = (0x08 or 0x04 or 0x10).toByte() }   // EXIF + XMP + alfa
        val img = webpChunk("VP8L", ByteArray(9) { 3 })
        val webp = riff(webpChunk("VP8X", vp8x), img, webpChunk("EXIF", ByteArray(5)), webpChunk("XMP ", ByteArray(4)))
        val out = MetaStrip.strip(webp, "image/webp")!!
        val expectedX = vp8x.copyOf().also { it[0] = 0x10 }
        assertArrayEquals(riff(webpChunk("VP8X", expectedX), img), out)
        assertEquals(out.size - 8, (out[4].toInt() and 0xFF) or ((out[5].toInt() and 0xFF) shl 8))
        assertFalse(String(out, Charsets.ISO_8859_1).contains("EXIF"))
    }

    @Test
    fun webpSemMetadadosFicaIgual() {
        val webp = riff(webpChunk("VP8 ", ByteArray(12) { 5 }))
        assertArrayEquals(webp, MetaStrip.strip(webp, "image/webp"))
        assertTrue(MetaStrip.strip(webp, "image/gif").contentEquals(webp))
    }
}
