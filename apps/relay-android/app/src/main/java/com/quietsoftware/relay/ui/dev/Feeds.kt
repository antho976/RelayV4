package com.quietsoftware.relay.ui.dev

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import com.quietsoftware.relay.data.Relay
import com.quietsoftware.relay.data.TerminalFeed
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * One [TerminalFeed] per agent for the whole app. The engine streams a session once per
 * connection: a second attach replaces the first, and a detach ends it for everyone on the link.
 * So the wall's miniature and the full terminal share one feed, or leaving the wall would detach
 * the terminal just opened. A feed is kept a moment after its last screen lets go, so moving
 * between the wall and a terminal keeps the stream rather than replaying it.
 *
 * Used from the main thread only (composition, effects and [scope]).
 */
internal object Feeds {
    /** Outlives screens: the feeds' streams, and acts whose answer is still worth a toast after the screen closed. */
    val scope: CoroutineScope = MainScope()

    private class Entry(val feed: TerminalFeed) {
        var users = 0
        var closing: Job? = null
        var closed = false

        /** Held at least once; one only just made is about to be, in this frame's effects. */
        var held = false
    }

    private val open = LinkedHashMap<String, Entry>()

    /** The feed for [name], made if there is none; nobody holds it until [hold]. */
    fun of(relay: Relay, name: String): TerminalFeed {
        open[name]?.let { return it.feed }
        val e = Entry(TerminalFeed(relay, name, scope))
        open[name] = e
        letGo(e)
        return e.feed
    }

    fun hold(feed: TerminalFeed) {
        val e = open[feed.name]?.takeIf { it.feed === feed } ?: Entry(feed).also { fresh ->
            open.put(feed.name, fresh)?.let(::close)
        }
        e.closing?.cancel()
        e.closing = null
        e.users++
        e.held = true
        feed.start()
    }

    fun release(feed: TerminalFeed) {
        val e = open[feed.name]?.takeIf { it.feed === feed } ?: return
        e.users = (e.users - 1).coerceAtLeast(0)
        if (e.users > 0) return
        // To the end of the order: the idle cap closes the longest unwatched first.
        open.remove(feed.name)
        open[feed.name] = e
        letGo(e)
    }

    private fun letGo(e: Entry) {
        e.closing?.cancel()
        e.closing = scope.launch {
            delay(GRACE_MS)
            close(e)
        }
        // Streams nobody watches cost the PC and the link: only the newest few wait out the grace.
        val idle = open.values.filter { it.users == 0 && it.held }
        if (idle.size > MAX_IDLE) idle.take(idle.size - MAX_IDLE).forEach(::close)
    }

    /** Once only: a second stop would detach whatever feed holds the session by then. */
    private fun close(e: Entry) {
        if (e.users > 0 || e.closed) return
        e.closed = true
        if (open[e.feed.name] === e) open.remove(e.feed.name)
        e.feed.save()
        e.feed.stop()
    }

    private const val GRACE_MS = 2_500L
    private const val MAX_IDLE = 3
}

/** The shared feed for [name], held while this composable is on screen. */
@Composable
internal fun rememberFeed(relay: Relay, name: String): TerminalFeed {
    val feed = remember(name) { Feeds.of(relay, name) }
    DisposableEffect(feed) {
        Feeds.hold(feed)
        onDispose { Feeds.release(feed) }
    }
    return feed
}
