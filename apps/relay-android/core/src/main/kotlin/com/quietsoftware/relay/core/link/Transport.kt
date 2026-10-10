package com.quietsoftware.relay.core.link

import kotlinx.coroutines.channels.ReceiveChannel

/**
 * One WebSocket to the PC's door, seen as lines. A text message may hold several lines; the
 * transport splits them, so everything above it reads one JSON object at a time.
 */
interface LineSocket {
    /** Lines from the PC. Closes when the socket does. */
    val incoming: ReceiveChannel<String>

    /** Queue a line. False once the socket is closed or its send buffer is full. */
    fun send(line: String): Boolean

    fun close()
}

/** Opens sockets. OkHttp on the phone (`:app`), a scripted door in tests. */
fun interface Dialer {
    /** Throws when the address cannot be reached within [timeoutMs]. */
    suspend fun dial(url: String, timeoutMs: Long): LineSocket
}
