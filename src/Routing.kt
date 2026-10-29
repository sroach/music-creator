package org.jetbrains.amper.ktor

import io.ktor.http.*
import io.ktor.server.application.*
import io.ktor.server.request.*
import io.ktor.server.response.*
import io.ktor.server.routing.*
import java.io.File
import java.time.LocalDateTime
import java.time.format.DateTimeFormatter
import kotlinx.serialization.json.Json

private val apiKey = System.getenv("ACESTEP_API_KEY")

private val metadataJson = Json {
    prettyPrint = true
    ignoreUnknownKeys = true
}

private val Application.outputDir: File
    get() {
        val dir = System.getProperty("app.outputDir", "output")
        return File(dir).apply { mkdirs() }
    }

private val Application.contextPath: String
    get() = System.getProperty("app.contextPath", "")

fun Application.configureRouting() {
    val ctx = contextPath
    routing {
        route(ctx) {
            // Main page - serve HTML from resources
            get("/") {
                val html = javaClass.classLoader.getResource("static/index.html")
                    ?.readText()
                    ?: "<html><body><h1>Music Creator</h1><p>index.html not found</p></body></html>"
                call.respondText(html, ContentType.Text.Html)
            }

            // Favicon
            get("/favicon.svg") {
                val svg = javaClass.classLoader.getResource("static/favicon.svg")
                    ?.readText()
                    ?: ""
                if (svg.isNotBlank()) {
                    call.respondText(svg, ContentType.parse("image/svg+xml"))
                } else {
                    call.respond(HttpStatusCode.NotFound)
                }
            }

            // Generate song endpoint
            post("/generate") {
            if (apiKey.isNullOrBlank()) {
                call.respond(
                    HttpStatusCode.InternalServerError,
                    ErrorResponse("ACESTEP_API_KEY environment variable is not set")
                )
                return@post
            }

            val request = try {
                call.receive<GenerateRequest>()
            } catch (e: Exception) {
                call.respond(HttpStatusCode.BadRequest, ErrorResponse("Invalid request body"))
                return@post
            }

            // Validate
            if (request.lyrics.isBlank()) {
                call.respond(HttpStatusCode.BadRequest, ErrorResponse("Lyrics are required"))
                return@post
            }
            if (request.prompt.isBlank()) {
                call.respond(HttpStatusCode.BadRequest, ErrorResponse("Style prompt is required"))
                return@post
            }

            try {
                // Generate song via ACEMusic API
                val audioBytes = AceMusicClient.generateSong(apiKey, request)

                // Sanitize song name for filename
                val safeName = request.song_name
                    .filter { it.isLetterOrDigit() || it in setOf(' ', '_', '-') }
                    .trim()
                    .replace(' ', '_')
                    .takeIf { it.isNotBlank() } ?: "generated_song"

                val filename = "$safeName.mp3"
                val filepath = File(this@configureRouting.outputDir, filename)

                // Save MP3 file
                filepath.writeBytes(audioBytes)

                // Embed ID3 tags
                try {
                    Id3TagWriter.embedTags(
                        file = filepath,
                        title = request.song_name,
                        artist = request.artist,
                        comment = request.prompt,
                        lyrics = request.lyrics
                    )
                } catch (e: Exception) {
                    println("⚠️  Failed to embed ID3 tags: ${e.message}")
                }

                // Save metadata JSON using proper serialization
                val metadata = SongMetadata(
                    song_name = request.song_name,
                    artist = request.artist,
                    prompt = request.prompt,
                    lyrics = request.lyrics,
                    duration = request.duration,
                    bpm = request.bpm,
                    filename = filename,
                    created_at = LocalDateTime.now().format(DateTimeFormatter.ISO_LOCAL_DATE_TIME),
                    file_size_bytes = audioBytes.size
                )
                val metadataJson = metadataJson.encodeToString(metadata)
                File(this@configureRouting.outputDir, "$safeName.json").writeText(metadataJson)

                call.respond(GenerateResponse(
                    message = "Song generated successfully",
                    filename = filename,
                    size = audioBytes.size.toLong(),
                    download_url = "/download/$filename"
                ))
            } catch (e: Exception) {
                println("❌ Generation error: ${e.message}")
                e.printStackTrace()
                call.respond(
                    HttpStatusCode.InternalServerError,
                    ErrorResponse("Generation failed: ${e.message}")
                )
            }
        }

        // Download generated file
        get("/download/{filename}") {
            val filename = call.parameters["filename"] ?: run {
                call.respond(HttpStatusCode.NotFound, ErrorResponse("File not found"))
                return@get
            }
            val filepath = File(call.application.outputDir, filename)
            if (filepath.exists()) {
                call.response.headers.append("Content-Disposition", "attachment; filename=\"$filename\"")
                call.respondFile(filepath)
            } else {
                call.respond(HttpStatusCode.NotFound, ErrorResponse("File not found"))
            }
        }
    }
}
