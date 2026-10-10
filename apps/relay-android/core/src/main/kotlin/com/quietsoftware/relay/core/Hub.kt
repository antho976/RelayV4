package com.quietsoftware.relay.core

import com.quietsoftware.relay.core.link.BusSession
import com.quietsoftware.relay.core.link.Dialer
import com.quietsoftware.relay.core.link.Link
import com.quietsoftware.relay.core.link.LinkState
import com.quietsoftware.relay.core.sync.Ledger
import com.quietsoftware.relay.core.sync.Optimistic
import com.quietsoftware.relay.core.sync.OutboxEntry
import com.quietsoftware.relay.core.sync.OutboxRunner
import com.quietsoftware.relay.core.sync.OutboxStore
import com.quietsoftware.relay.core.sync.QueryKey
import com.quietsoftware.relay.core.sync.ReplicaStore
import com.quietsoftware.relay.core.sync.Syncer
import com.quietsoftware.relay.core.wire.BusError
import com.quietsoftware.relay.core.wire.BusException
import com.quietsoftware.relay.core.wire.Wire
import com.quietsoftware.relay.core.wire.l
import com.quietsoftware.relay.core.wire.s
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

/**
 * Everything between the screens and the PC: the link, the replica, the outbox, and the rule
 * that decides which of them a request goes through.
 *
 * - **Reads** come from the replica, which the [Syncer] keeps in step while the PC is in reach.
 *   A query that is not an entity is cached by its op and payload, so a screen opened before
 *   shows its last answer with the PC away.
 * - **Edits to the PC's data** go through the outbox ([change]): shown at once, sent now if the
 *   PC is in reach, later if not, exactly once either way.
 * - **Acts** that only make sense with the PC there (typing into a terminal, allowing a held
 *   command, pushing) go straight to it ([call]) and fail with `link.down` when it is away.
 */
class Hub(
    private val scope: CoroutineScope,
    dialer: Dialer,
    profiles: Link.ProfileSource,
    val store: ReplicaStore,
    val outbox: OutboxStore,
    private val now: () -> Long = System::currentTimeMillis,
) {
    val ledger = Ledger(store, outbox, now)
    val link = Link(scope, dialer, profiles, now)
    val syncer = Syncer(scope, ledger, store, now)

    private val _parked = MutableSharedFlow<OutboxEntry>(extraBufferCapacity = 16)

    /** An outbox entry the PC refused or held: the screens say so, the person decides. */
    val parked: SharedFlow<OutboxEntry> = _parked

    private val _outboxChanged = MutableStateFlow(0L)

    /** Bumps whenever the outbox moves, for the status bar's count. */
    val outboxChanged: StateFlow<Long> = _outboxChanged

    private val runner = OutboxRunner(
        outbox,
        onApplied = { e, r ->
            ledger.applied(e, r)
            _outboxChanged.value++
        },
        onParked = {
            _parked.tryEmit(it)
            _outboxChanged.value++
        },
        now = now,
    )
    private val flushing = Mutex()

    val state: StateFlow<LinkState> get() = link.state
    val session: StateFlow<BusSession?> get() = link.session

    init {
        link.onSession = { s ->
            val holds = scope.launch { s.events.collect { if (it.ev == "guardrail.resolved") resolved(it.payload) } }
            scope.launch {
                s.ended.await()
                holds.cancel()
            }
            syncer.attach(s)
            flush()
        }
        scope.launch {
            ledger.queued.collect {
                _outboxChanged.value++
                flush()
            }
        }
    }

    fun start() = link.start()

    @Volatile private var again = false

    /** Send what the outbox holds, if the PC is in reach. Safe to call any time, from anywhere. */
    suspend fun flush() {
        again = true
        if (!flushing.tryLock()) return
        try {
            while (again) {
                again = false
                val s = link.session.value?.takeIf { it.isOpen } ?: break
                if (runner.flush(s) is OutboxRunner.Outcome.LinkLost) break
            }
        } finally {
            flushing.unlock()
            _outboxChanged.value++
        }
        // An entry queued between the last pass and the unlock is not left waiting.
        if (again && link.session.value?.isOpen == true) flush()
    }

    /**
     * A hold the PC resolved: an outbox entry it was holding is done when someone allowed it (the
     * confirm ran it, under its own id), and refused when someone said no. Resending would only
     * replay `held` (BUS.md §5.3).
     */
    private suspend fun resolved(payload: JsonObject) {
        val hold = payload.l("hold_id") ?: return
        val state = payload.s("state").orEmpty()
        for (e in outbox.open().filter { it.state == OutboxEntry.State.Held }) {
            val held = (e.error?.confirm?.get("payload") as? JsonObject)?.l("hold_id") ?: e.error?.details?.l("hold_id")
            if (held != hold) continue
            if (state == "rejected") {
                val why = payload.s("reason")?.let { ": $it" }.orEmpty()
                outbox.update(e.copy(state = OutboxEntry.State.Failed, error = BusError("refused", "guardrail.rejected", "Denied on the PC$why")))
            } else {
                val done = e.copy(state = OutboxEntry.State.Done, error = null)
                outbox.update(done)
                ledger.applied(done, kotlinx.serialization.json.JsonNull)
            }
            _outboxChanged.value++
        }
    }

    /** An edit to the PC's data, through the outbox. Ops that cannot wait are sent straight away instead. */
    suspend fun change(op: String, payload: JsonObject, label: String): Change {
        if (op !in Optimistic.QUEUEABLE) return Change.Now(call(op, payload))
        val entry = ledger.enqueue(op, payload, label)
        return Change.Queued(entry)
    }

    /** An act that needs the PC now. Throws [BusException] (`link.down` with the PC away). */
    suspend fun call(op: String, payload: JsonObject = JsonObject(emptyMap()), timeoutMs: Long = OutboxRunner.timeoutFor(op)): JsonElement =
        link.require().result(op, payload, timeoutMs = timeoutMs)

    /**
     * A read that is not an entity: the PC's answer when in reach (cached for later), the last
     * cached answer otherwise. [Answer.fresh] says which.
     */
    suspend fun query(op: String, payload: JsonObject = JsonObject(emptyMap())): Answer {
        val key = QueryKey.of(op, payload)
        val s = link.session.value?.takeIf { it.isOpen }
        if (s != null) {
            try {
                val result = s.result(op, payload, timeoutMs = OutboxRunner.timeoutFor(op))
                store.putQuery(key, result.toString(), now())
                return Answer(result, now(), fresh = true)
            } catch (e: BusException) {
                if (!e.error.transient) throw e
            }
        }
        val cached = store.query(key) ?: throw BusException(BusError.link("link.down", "Not connected to the PC, and this was never read before"))
        return Answer(Wire.json.parseToJsonElement(cached.first), cached.second, fresh = false)
    }

    /** The cached answer only, for a screen's first frame. */
    suspend fun cached(op: String, payload: JsonObject = JsonObject(emptyMap())): Answer? =
        store.query(QueryKey.of(op, payload))?.let { Answer(Wire.json.parseToJsonElement(it.first), it.second, fresh = false) }

    suspend fun discard(entryId: String) {
        ledger.discard(entryId)
        _outboxChanged.value++
    }

    suspend fun retry(entryId: String, overwrite: Boolean) {
        ledger.retry(entryId, overwrite)
        _outboxChanged.value++
        flush()
    }

    /** Forget the PC's data: another PC was paired, or this one forgotten. */
    suspend fun forgetData() = flushing.withLock {
        store.clear()
        outbox.open().forEach { outbox.delete(it.id) }
        _outboxChanged.value++
    }

    sealed interface Change {
        /** Sent straight away; the PC's answer. */
        data class Now(val result: JsonElement) : Change

        /** In the outbox; shown already, sent when the PC is in reach. */
        data class Queued(val entry: OutboxEntry) : Change
    }

    data class Answer(val result: JsonElement, val at: Long, val fresh: Boolean)
}
