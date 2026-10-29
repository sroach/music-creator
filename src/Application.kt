package org.jetbrains.amper.ktor

import io.ktor.server.application.*
import io.ktor.server.cio.*
import io.ktor.server.engine.*
import io.ktor.server.plugins.contentnegotiation.*
import io.ktor.serialization.kotlinx.json.*
import kotlinx.serialization.json.Json

fun main(args: Array<String>) {
    val port = args.getPortOrDefault()
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

fun Application.module() {
    install(ContentNegotiation) {
        json(
            Json {
                ignoreUnknownKeys = true
                prettyPrint = true
            }
        )
    }
    configureRouting()
}
