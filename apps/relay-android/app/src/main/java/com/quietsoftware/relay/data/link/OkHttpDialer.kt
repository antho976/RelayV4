package com.quietsoftware.relay.data.link

import com.quietsoftware.relay.core.link.Dialer
import com.quietsoftware.relay.core.link.LineSocket
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.ReceiveChannel
import kotlinx.coroutines.withTimeout
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.io.IOException
import java.util.concurrent.TimeUnit

/**
 * The phone's WebSockets to the PC's door. The only network code in the app: it carries bus
 * lines to the person's own PC, directly or through their own rendezvous, and nothing else.
 */
class OkHttpDialer(
    private val client: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(8, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        // The door pings every 30 s and drops a phone silent for 90; this keeps NAT flows open too.
        .pingInterval(25, TimeUnit.SECONDS)
        .build(),
) : Dialer {
    override suspend fun dial(url: String, timeoutMs: Long): LineSocket = withTimeout(timeoutMs) {
        val lines = Channel<String>(LINE_BUFFER)
        val opened = CompletableDeferred<WebSocket>()
        val request = Request.Builder().url(url.replaceFirst("ws://", "http://").replaceFirst("wss://", "https://")).build()
        val socket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                opened.complete(webSocket)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                // A text message may carry several bus lines; each goes up on its own.
                for (line in text.split('\n')) if (line.isNotBlank()) {
                    if (lines.trySend(line).isFailure) {
                        // The reader stopped keeping up for 4096 lines: close rather than drop
                        // silently; the link redials and re-reads.
                        webSocket.close(1011, "phone fell behind")
                        lines.close()
                        return
                    }
                }
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
                lines.close()
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                lines.close()
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                opened.completeExceptionally(IOException(t.message ?: "connection failed", t))
                lines.close()
            }
        })
        try {
            opened.await()
        } catch (e: Throwable) {
            // A timeout or a refusal: drop the half-open socket with it.
            socket.cancel()
            throw e
        }
        OkLineSocket(socket, lines)
    }

    private class OkLineSocket(private val socket: WebSocket, private val lines: Channel<String>) : LineSocket {
        override val incoming: ReceiveChannel<String> = lines

        // OkHttp queues up to 16 MiB per socket before it refuses; a refused send is a dead link.
        override fun send(line: String): Boolean = socket.send(line)

        override fun close() {
            socket.close(1000, null)
            lines.close()
        }
    }

    companion object {
        const val LINE_BUFFER = 4096
    }
}
