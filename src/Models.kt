package org.jetbrains.amper.ktor

import kotlinx.serialization.Serializable

// Request from the UI to generate a song
@Serializable
data class GenerateRequest(
    val song_name: String = "generated_song",
    val artist: String = "AI Generated",
    val lyrics: String = "",
    val prompt: String = "",
    val duration: Int = 180,
    val bpm: Int = 120
)

// Response back to the UI
@Serializable
data class GenerateResponse(
    val message: String,
    val filename: String,
    val size: Long,
    val download_url: String
)

@Serializable
data class ErrorResponse(
    val error: String
)

// --- ACEMusic API types ---

@Serializable
data class AceMusicRequest(
    val model: String = "acemusic/acestep-v15-turbo",
    val prompt: String,
    val lyrics: String,
    val messages: List<Message>,
    val stream: Boolean = true,
    val audio_config: AudioConfig
)

@Serializable
data class Message(
    val role: String = "user",
    val content: String
)

@Serializable
data class AudioConfig(
    val format: String = "mp3",
    val vocal_language: String = "en",
    val duration: Int,
    val bpm: Int
)

// Streaming response chunks
@Serializable
data class AceMusicChunk(
    val choices: List<AceChoice>? = null
)

@Serializable
data class AceChoice(
    val delta: AceDelta? = null,
    val message: AceMessage? = null
)

@Serializable
data class AceDelta(
    val audio: AceAudio? = null
)

@Serializable
data class AceMessage(
    val audio: AceAudio? = null
)

@Serializable
data class AceAudio(
    val data: String? = null
)
