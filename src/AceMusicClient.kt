package org.jetbrains.amper.ktor

import io.ktor.client.*
import io.ktor.client.engine.java.*
import io.ktor.client.plugins.contentnegotiation.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import io.ktor.serialization.kotlinx.json.*
import kotlinx.serialization.json.Json

object AceMusicClient {
    private const val BASE_URL = "https://api.acemusic.ai"

    private val json = Json {
        ignoreUnknownKeys = true
        isLenient = true
    }

    private val httpClient = HttpClient(Java) {
        install(ContentNegotiation) {
            json(json)
        }
    }

    suspend fun generateSong(
        apiKey: String,
        request: GenerateRequest
    ): ByteArray {
        val payload = AceMusicRequest(
            prompt = request.prompt,
            lyrics = request.lyrics,
            messages = listOf(Message(content = request.lyrics)),
            audio_config = AudioConfig(
                duration = request.duration,
                bpm = request.bpm
            )
        )

        println("=".repeat(60))
        println("📤 SENDING PAYLOAD TO ACEMusic API")
        println("=".repeat(60))
        println("Model: ${payload.model}")
        println("Prompt (style tags): ${request.prompt}")
        println("Lyrics (first 100 chars): ${request.lyrics.take(100)}...")
        println("Audio Config: format=mp3, vocal_language=en, duration=${request.duration}, bpm=${request.bpm}")
        println("URL: $BASE_URL/v1/chat/completions")
        println("=".repeat(60))

        val audioChunks = mutableListOf<String>()

        val response = httpClient.post("$BASE_URL/v1/chat/completions") {
            header("Authorization", "Bearer $apiKey")
            contentType(ContentType.Application.Json)
            setBody(payload)
        }

        // Process the response body line by line for SSE
        val bodyText = response.bodyAsText()
        for (line in bodyText.split("\n")) {
            if (line.isBlank()) continue
            if (line.startsWith(":")) continue

            if (line.startsWith("data: ")) {
                val dataStr = line.substring(6)
                if (dataStr.trim() == "[DONE]") break

                try {
                    val chunk = json.decodeFromString<AceMusicChunk>(dataStr)
                    chunk.choices?.forEach { choice ->
                        val audio = choice.delta?.audio ?: choice.message?.audio
                        audio?.data?.let { audioChunks.add(it) }
                    }
                } catch (e: Exception) {
                    // Skip malformed lines
                }
            }
        }

        if (audioChunks.isEmpty()) {
            throw IllegalStateException("No audio data found in API response")
        }

        // Combine and decode base64 audio
        val combinedAudio = audioChunks.joinToString("")
        val cleanAudio = if ("," in combinedAudio) {
            combinedAudio.split(",", limit = 2)[1]
        } else {
            combinedAudio
        }

        return java.util.Base64.getDecoder().decode(cleanAudio)
    }
}
