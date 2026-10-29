package org.jetbrains.amper.ktor

import io.ktor.http.*
import io.ktor.server.application.*
import io.ktor.server.cio.*
import io.ktor.server.engine.*
import io.ktor.server.plugins.contentnegotiation.*
import io.ktor.server.plugins.cors.routing.*
import io.ktor.serialization.kotlinx.json.*
import kotlinx.serialization.json.Json

fun main(args: Array<String>) {
    val port = args.getPortOrDefault()
    val outputDir = args.getOutputDirOrDefault()
    val contextPath = args.getContextPathOrDefault()
    System.setProperty("app.outputDir", outputDir)
    System.setProperty("app.contextPath", contextPath)
    embeddedServer(CIO, port = port, module = Application::module)
        .start(wait = true)
}

private fun Array<String>.getPortOrDefault(): Int {
    val defaultPort = 8080
    for (i in indices) {
        when {
            this[i] == "--port" && i + 1 < size -> {
                return this[i + 1].toIntOrNull() ?: defaultPort
            }
            this[i].startsWith("--port=") -> {
                return this[i].substringAfter("=").toIntOrNull() ?: defaultPort
            }
            this[i] == "-p" && i + 1 < size -> {
                return this[i + 1].toIntOrNull() ?: defaultPort
            }
        }
    }
    return defaultPort
}

private fun Array<String>.getOutputDirOrDefault(): String {
    val defaultDir = "output"
    for (i in indices) {
        when {
            this[i] == "--output" && i + 1 < size -> {
                return this[i + 1]
            }
            this[i].startsWith("--output=") -> {
                return this[i].substringAfter("=")
            }
            this[i] == "-o" && i + 1 < size -> {
                return this[i + 1]
            }
        }
    }
    return defaultDir
}

private fun Array<String>.getContextPathOrDefault(): String {
    val defaultPath = ""
    for (i in indices) {
        when {
            this[i] == "--context-path" && i + 1 < size -> {
                return this[i + 1].let { p -> if (p.startsWith("/")) p else "/$p" }
            }
            this[i].startsWith("--context-path=") -> {
                return this[i].substringAfter("=").let { p -> if (p.startsWith("/")) p else "/$p" }
            }
        }
    }
    return defaultPath
}

fun Application.module() {
    install(ContentNegotiation) {
        json(
            Json {
                ignoreUnknownKeys = true
                prettyPrint = true
            }
        )
    }

    install(CORS) {
        anyHost()
        allowMethod(HttpMethod.Options)
        allowMethod(HttpMethod.Put)
        allowMethod(HttpMethod.Delete)
        allowMethod(HttpMethod.Patch)
        allowHeader(HttpHeaders.Authorization)
        allowNonSimpleContentTypes = true
    }

    configureRouting()
}
