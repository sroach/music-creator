package org.jetbrains.amper.ktor

import java.io.File
import java.nio.charset.StandardCharsets

/**
 * Minimal ID3v2.4 tag writer for MP3 files.
 * Embeds: Title (TIT2), Artist (TPE1), Comment (TCOM), Lyrics (USLT).
 */
object Id3TagWriter {

    fun embedTags(
        file: File,
        title: String,
        artist: String,
        comment: String,
        lyrics: String
    ) {
        val mp3Data = file.readBytes()

        // Build ID3v2.4 frame data
        val frames = mutableListOf<ByteArray>()

        // TIT2 - Title
        frames.add(buildTextFrame("TIT2", title))
        // TPE1 - Artist
        frames.add(buildTextFrame("TPE1", artist))
        // TCOM - Comment (truncated to 300 chars)
        frames.add(buildTextFrame("TCOM", comment.take(300)))
        // USLT - Unsynchronized lyrics
        frames.add(buildLyricsFrame(lyrics))

        val framesData = frames.fold(byteArrayOf()) { acc, frame -> acc + frame }
        val header = buildId3Header(framesData.size)

        // Write new file: ID3 header + frames + original MP3
        val outputFile = File(file.parentFile, file.name) // same path, overwrite
        outputFile.outputStream().use { out ->
            out.write(header)
            out.write(framesData)
            out.write(mp3Data)
        }

        println("🏷️  ID3 tags embedded in ${file.name}")
    }

    private fun buildTextFrame(id: String, text: String): ByteArray {
        val textBytes = text.toByteArray(StandardCharsets.UTF_8)
        // Frame: ID(4) + size(4) + flags(2) + encoding(1) + text
        val frameData = byteArrayOf(0.toByte()) + textBytes // encoding 0 = ISO-8859-1, but we use UTF-8 with encoding=1
        val frameDataUtf8 = byteArrayOf(1.toByte()) + textBytes // encoding 1 = UTF-8
        val size = frameDataUtf8.size
        return id.toByteArray(StandardCharsets.US_ASCII) +
                size.toSyncSafeByte(4) +
                byteArrayOf(0, 0) + // flags
                frameDataUtf8
    }

    private fun buildLyricsFrame(lyrics: String): ByteArray {
        val lyricsBytes = lyrics.toByteArray(StandardCharsets.UTF_8)
        val lang = "eng".toByteArray(StandardCharsets.US_ASCII)
        val desc = "Lyrics".toByteArray(StandardCharsets.UTF_8) + byteArrayOf(0)
        val frameData = byteArrayOf(1.toByte()) + lang + desc + byteArrayOf(0) + lyricsBytes // encoding 1 = UTF-8
        val size = frameData.size
        return "USLT".toByteArray(StandardCharsets.US_ASCII) +
                size.toSyncSafeByte(4) +
                byteArrayOf(0, 0) + // flags
                frameData
    }

    private fun buildId3Header(size: Int): ByteArray {
        val header = ByteArray(10)
        // "ID3"
        header[0] = 'I'.code.toByte()
        header[1] = 'D'.code.toByte()
        header[2] = '3'.code.toByte()
        // Version 2.4.0
        header[3] = 4
        header[4] = 0
        // Flags
        header[5] = 0
        // Size (sync safe)
        System.arraycopy(size.toSyncSafeByte(4), 0, header, 6, 4)
        return header
    }

    private fun Int.toSyncSafeByte(len: Int): ByteArray {
        val buf = ByteArray(len)
        var value = this
        for (i in len - 1 downTo 0) {
            buf[i] = (value and 0x7F).toByte()
            value = value ushr 7
        }
        return buf
    }
}
