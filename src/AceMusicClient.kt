package org.jetbrains.amper.ktor

import io.ktor.client.*
import io.ktor.client.engine.java.*
import io.ktor.client.plugins.contentnegotiation.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import io.ktor.serialization.kotlinx.json.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

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
            header("Accept", "text/event-stream")
            header("Cache-Control", "no-cache")
            contentType(ContentType.Application.Json)
            setBody(payload)
        }

        val statusCode = response.status.value
        println("📥 API Response Status: $statusCode")

        // Process the response body - handle both SSE format and plain JSON
        val bodyText = response.bodyAsText()
        println("📥 Response body length: ${bodyText.length} chars")

        val trimmedBody = bodyText.trimStart()
        val isSseFormat = trimmedBody.startsWith("data: ")

        if (isSseFormat) {
            // SSE format: parse line by line
            var lineCount = 0
            for (line in bodyText.split("\n")) {
                if (line.isBlank()) continue
                if (line.startsWith(":")) continue

                if (line.startsWith("data: ")) {
                    val dataStr = line.substring(6)
                    if (dataStr.trim() == "[DONE]") break

                    lineCount++
                    try {
                        val element = json.parseToJsonElement(dataStr)
                        extractAudioData(element, audioChunks)
                    } catch (e: Exception) {
                        println("⚠️  Failed to parse line $lineCount: ${e.message}")
                        if (lineCount <= 3) {
                            println("   Raw: ${dataStr.take(200)}")
                        }
                    }
                }
            }
            println("📊 SSE mode: Parsed $lineCount data lines, found ${audioChunks.size} audio chunks")
        } else {
            // Plain JSON format: parse the entire response as a single JSON object
            try {
                val element = json.parseToJsonElement(bodyText)
                extractAudioData(element, audioChunks)
                println("📊 Plain JSON mode: Found ${audioChunks.size} audio chunks")
            } catch (e: Exception) {
                println("⚠️  Failed to parse plain JSON: ${e.message}")
            }
        }

        if (audioChunks.isEmpty()) {
            // Log the raw response for debugging
            println("❌ Raw response (first 2000 chars):")
            println(bodyText.take(2000))
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

    // Recursively extract audio data from JSON, matching Python's flexible parsing
    private fun extractAudioData(element: JsonElement, chunks: MutableList<String>) {
        if (element is JsonObject) {
            val choices = element["choices"]
            if (choices is kotlinx.serialization.json.JsonArray) {
                choices.forEach { choiceObj ->
                    if (choiceObj is JsonObject) {
                        // Check delta.audio
                        val delta = choiceObj["delta"]
                        if (delta is JsonObject) {
                            val audioData = delta["audio"]
                            extractAudioValue(audioData, chunks)
                        }
                        // Check message.audio
                        val message = choiceObj["message"]
                        if (message is JsonObject) {
                            val audioData = message["audio"]
                            extractAudioValue(audioData, chunks)
                        }
                    }
                }
            }
        }
    }

    // Handle audio value in all formats: object with "data", plain string, or list
    private fun extractAudioValue(audioElement: JsonElement?, chunks: MutableList<String>) {
        if (audioElement is JsonNull || audioElement == null) return

        when (audioElement) {
            is JsonObject -> {
                // audio is a dict with "data" field
                val dataField = audioElement["data"]
                if (dataField is JsonPrimitive && dataField.content.isNotBlank()) {
                    chunks.add(dataField.content)
                }
            }
            is JsonPrimitive -> {
                // audio is a plain string
                if (audioElement.content.isNotBlank()) {
                    chunks.add(audioElement.content)
                }
            }
            is kotlinx.serialization.json.JsonArray -> {
                // audio is a list
                for (item in audioElement) {
                    if (item is JsonObject) {
                        // Check audio_url.url
                        val audioUrl = item["audio_url"]
                        if (audioUrl is JsonObject) {
                            val url = audioUrl["url"]
                            if (url is JsonPrimitive && url.content.isNotBlank()) {
                                chunks.add(url.content)
                                continue
                            }
                        }
                        // Check data field
                        val dataField = item["data"]
                        if (dataField is JsonPrimitive && dataField.content.isNotBlank()) {
                            chunks.add(dataField.content)
                        }
                    } else if (item is JsonPrimitive && item.content.isNotBlank()) {
                        chunks.add(item.content)
                    }
                }
            }
        }
    }
}
