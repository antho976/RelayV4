package com.tally.app.data.sync

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response as HttpResponse
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.UUID
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

/** Why talking to the PC failed, in words the Settings line can show as they are. */
class RelayFailure(
    val reason: String,
    /** The PC said this phone is no longer paired: retrying will not help, pairing again will. */
    val unpaired: Boolean = false,
) : Exception(reason)

/** One admitted connection to the PC: the greeting it opened with, and the bus behind it. */
class RelaySession internal constructor(
    private val socket: RelaySocket,
    val greeting: Greeting,
    val welcome: Welcome,
) : AutoCloseable {

    /** Sends one bus request and waits for its response; lines meant for anything else are skipped. */
    suspend fun request(op: String, payload: JsonObject, timeoutMs: Long = REQUEST_TIMEOUT_MS): JsonElement? {
        val id = UUID.randomUUID().toString()
        socket.send(RelayWire.request(op, payload, id))
        val response = try {
            withTimeout(timeoutMs) {
                var found: Response? = null
                while (found == null) found = RelayWire.response(socket.next(), id)
                found
            }
        } catch (e: TimeoutCancellationException) {
            throw RelayFailure("The PC did not answer in time")
        }
        if (!response.ok) {
            val reason = when (response.errorCode) {
                "remote.op" -> "This PC's Relay does not take Tally's sync yet. Update Relay on the PC."
                "bus.unknown_op" -> "This PC's Relay does not know Tally's sync yet. Update Relay on the PC."
                else -> response.errorMessage ?: response.errorCode ?: "The PC refused the sync"
            }
            throw RelayFailure(reason)
        }
        return response.result
    }

    override fun close() = socket.close()

    companion object {
        /** A large first sync is one request; the PC answers it in one transaction. */
        const val REQUEST_TIMEOUT_MS = 60_000L
    }
}

/**
 * The door's client: try each route in order until a PC greets, answer with [hello], and hand
 * back the admitted connection. A refusal after the hello is final (a spent code, a revoked
 * phone); a route that cannot be reached moves on to the next.
 */
@Singleton
class RelayClient @Inject constructor() {

    private val http: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(CONNECT_TIMEOUT_S, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.SECONDS)
            // The door pings every 30 s and closes a phone silent for 90; answering is OkHttp's.
            .pingInterval(25, TimeUnit.SECONDS)
            .build()
    }

    suspend fun connect(
        routes: List<String>,
        hello: (Greeting) -> String,
        welcomeTimeoutMs: Long = WELCOME_TIMEOUT_MS,
        /** Pairing: once the code went out it may be spent, so no other route is tried. */
        oneShot: Boolean = false,
    ): RelaySession {
        if (routes.isEmpty()) throw RelayFailure("This PC has no address to reach it at. Pair it again.")
        var last = "The PC could not be reached"
        for (route in routes) {
            val socket = try {
                RelaySocket.open(http, route)
            } catch (e: IllegalArgumentException) {
                last = "That address does not read as a PC's"
                continue
            }
            val greeting = try {
                withTimeout(GREETING_TIMEOUT_MS) { RelayWire.greeting(socket.next()) }
            } catch (e: Exception) {
                socket.close()
                last = unreachable(e)
                continue
            }
            if (greeting == null || greeting.refusal != null) {
                socket.close()
                last = when {
                    greeting == null -> "Something answered at that address, but not Relay"
                    greeting.refusal == "host.offline" -> "The PC is not connected to its server right now"
                    else -> "The PC refused the connection (${greeting.refusal})"
                }
                continue
            }
            socket.send(hello(greeting))
            val welcome = try {
                withTimeout(welcomeTimeoutMs) { RelayWire.welcome(socket.next()) }
            } catch (e: Exception) {
                socket.close()
                last = if (e is TimeoutCancellationException) "The PC did not answer in time" else unreachable(e)
                if (oneShot) break else continue
            }
            if (welcome == null || !welcome.ok) {
                socket.close()
                throw denial(welcome?.error)
            }
            return RelaySession(socket, greeting, welcome)
        }
        throw RelayFailure(last)
    }

    private fun unreachable(e: Exception): String = when (e) {
        is TimeoutCancellationException -> "The PC did not answer in time"
        else -> "The PC could not be reached"
    }

    companion object {
        const val CONNECT_TIMEOUT_S = 8L
        const val GREETING_TIMEOUT_MS = 10_000L
        const val WELCOME_TIMEOUT_MS = 20_000L
        /** A pairing may wait on a person at the PC: the door gives them 75 s. */
        const val PAIR_WELCOME_TIMEOUT_MS = 90_000L

        /** The door's refusal codes (crates/relay-remote/src/bridge.rs), in plain words. */
        fun denial(code: String?): RelayFailure = when (code) {
            "pair.invalid" -> RelayFailure("The pairing code was wrong or has expired. Run relay remote pair on the PC again.")
            "pair.declined" -> RelayFailure("The PC declined this pairing.")
            "pair.unconfirmed" -> RelayFailure("Nobody approved the pairing on the PC in time. Run relay remote pair again and answer its prompt.")
            "auth.unknown_device" -> RelayFailure("The PC no longer knows this phone. Pair it again.", unpaired = true)
            "auth.bad_proof" -> RelayFailure("The PC no longer accepts this phone's key. Pair it again.", unpaired = true)
            null -> RelayFailure("Something answered at that address, but not Relay")
            else -> RelayFailure("The PC refused the connection ($code)")
        }
    }
}

/** A WebSocket as a queue of lines: what the door sends arrives as [next], in order. */
internal class RelaySocket private constructor() : WebSocketListener() {

    private val lines = Channel<String>(Channel.UNLIMITED)
    private lateinit var ws: WebSocket

    suspend fun next(): String = try {
        lines.receive()
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        // Closed by the PC, or the link failed under it: the same thing to the person waiting.
        throw RelayFailure("The PC closed the connection")
    }

    fun send(line: String) {
        if (!ws.send(line)) throw RelayFailure("The PC closed the connection")
    }

    fun close() {
        ws.close(1000, null)
        lines.close()
    }

    override fun onMessage(webSocket: WebSocket, text: String) {
        // One line per message, but a frame holding several is several lines.
        text.split('\n').filter { it.isNotBlank() }.forEach { lines.trySend(it) }
    }

    override fun onFailure(webSocket: WebSocket, t: Throwable, response: HttpResponse?) {
        lines.close(t)
    }

    override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
        webSocket.close(1000, null)
        lines.close()
    }

    override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
        lines.close()
    }

    companion object {
        /** Throws [IllegalArgumentException] for a URL OkHttp cannot read. */
        fun open(http: OkHttpClient, url: String): RelaySocket {
            val socket = RelaySocket()
            socket.ws = http.newWebSocket(Request.Builder().url(url).build(), socket)
            return socket
        }
    }
}
