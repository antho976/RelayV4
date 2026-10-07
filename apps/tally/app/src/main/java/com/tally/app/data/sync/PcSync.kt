package com.tally.app.data.sync

import android.os.Build
import com.tally.app.data.prefs.SettingsRepository
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import javax.inject.Inject
import javax.inject.Singleton

/** How a pairing or a sync ended. */
sealed interface SyncOutcome {
    data object NotPaired : SyncOutcome
    data class Done(val sent: Int, val received: Int) : SyncOutcome
    data class Failed(val reason: String) : SyncOutcome
}

/**
 * Tally and the PC holding one ledger (docs/MONEY.md, "Sync"). Pairing goes through Relay's phone
 * door with the code the PC shows; each sync after that is one connection proving the phone's
 * key, one `money.sync` request carrying what changed here since the last sync, and the PC's
 * changes merged into the ledger. The first sync makes the PC's ledger this phone's.
 *
 * One sync at a time. Failure is quiet: it is recorded for the line in Settings, never thrown.
 */
@Singleton
class PcSync @Inject constructor(
    private val store: SyncStore,
    private val ledger: LedgerSync,
    private val settings: SettingsRepository,
    private val client: RelayClient,
) {
    private val mutex = Mutex()
    private val _busy = MutableStateFlow(false)

    /** A pairing or a sync is talking to the PC right now. */
    val busy: StateFlow<Boolean> = _busy.asStateFlow()

    /** When the last run ended, in epoch millis: the ledger writes it made are not local changes. */
    @Volatile var lastFinishedAt: Long = 0L
        private set

    /** The name the PC lists this phone under, and the device its syncs are recorded for. */
    private val deviceName: String
        get() = "Tally on " + (Build.MODEL?.trim()?.takeIf { it.isNotEmpty() } ?: "a phone")

    /** Pairs with the PC behind [link] and runs the first sync on the same connection. */
    suspend fun pair(link: PairLink): SyncOutcome = run {
        val session = try {
            client.connect(
                link.routes,
                hello = { RelayWire.pairHello(link.code, deviceName.take(64)) },
                welcomeTimeoutMs = RelayClient.PAIR_WELCOME_TIMEOUT_MS,
                oneShot = true,
            )
        } catch (e: RelayFailure) {
            return@run SyncOutcome.Failed(e.reason)
        }
        session.use {
            val token = it.welcome.token
            val device = it.welcome.device
            if (token.isNullOrBlank() || device.isNullOrBlank()) {
                return@run SyncOutcome.Failed("The PC let this phone in without a key. Run relay remote pair on the PC again.")
            }
            store.pair(
                PairedPc(
                    name = it.greeting.host.ifBlank { link.host },
                    hostId = it.greeting.hostId.ifBlank { link.hostId },
                    instance = it.greeting.instance.ifBlank { link.instance },
                    routes = link.routes,
                    deviceId = device,
                    token = token,
                    pairedAt = System.currentTimeMillis(),
                )
            )
            exchange(it, store.current())
        }
    }

    /** Syncs with the paired PC; [SyncOutcome.NotPaired] when there is none. */
    suspend fun syncNow(): SyncOutcome = run {
        val prefs = store.current()
        val pc = prefs.pc ?: return@run SyncOutcome.NotPaired
        val session = try {
            client.connect(pc.routes, hello = { g -> RelayWire.proofHello(pc.deviceId, g.challenge, pc.token) })
        } catch (e: RelayFailure) {
            store.recordFailure(System.currentTimeMillis(), e.reason)
            return@run SyncOutcome.Failed(e.reason)
        }
        session.use {
            if (it.greeting.host.isNotBlank() && it.greeting.host != pc.name) store.rename(it.greeting.host)
            exchange(it, prefs)
        }
    }

    /**
     * Forgets the PC and hands back what was kept, for Undo. Its copy of the ledger stays where
     * it is; this phone just stops syncing with it.
     */
    suspend fun forget(): SyncPrefs = mutex.withLock {
        val was = store.current()
        store.forget()
        was
    }

    suspend fun restore(prefs: SyncPrefs) = mutex.withLock { store.restore(prefs) }

    private suspend fun run(block: suspend () -> SyncOutcome): SyncOutcome = mutex.withLock {
        _busy.value = true
        try {
            block()
        } finally {
            lastFinishedAt = System.currentTimeMillis()
            _busy.value = false
        }
    }

    /**
     * One exchange on an admitted connection: send, receive, merge, remember where it stands.
     * The watermark is taken before anything is read, so an edit made while the sync runs is
     * sent next time rather than lost.
     */
    private suspend fun exchange(session: RelaySession, prefs: SyncPrefs): SyncOutcome {
        val started = System.currentTimeMillis()
        return try {
            val replace = !prefs.replaced
            val since = if (replace) null else prefs.watermark
            val local = settingsChanges(since) + ledger.collect(since)
            var cursor = if (replace) 0L else prefs.cursor
            val received = ArrayList<Change>()
            // Batched, so a ledger of years stays under the door's message size. Only the first
            // batch replaces; the rest add to what it put there. Tables stay in order across them.
            val batches = local.chunked(BATCH).ifEmpty { listOf(emptyList()) }
            batches.forEachIndexed { i, batch ->
                val payload = buildJsonObject {
                    put("device", deviceName.take(64))
                    put("replace", replace && i == 0)
                    put("since", cursor)
                    put("changes", JsonArray(batch.map { it.toJson() }))
                }
                val result = session.request("money.sync", payload) as? JsonObject
                    ?: throw RelayFailure("The PC's answer did not read")
                cursor = (result["cursor"] as? JsonPrimitive)?.longOrNull ?: throw RelayFailure("The PC's answer did not read")
                (result["changes"] as? JsonArray)?.forEach { c -> Change.fromJson(c)?.let { received += it } }
            }
            val (rows, settingRows) = received.partition { it.table != SyncTables.SETTINGS }
            val applied = ledger.apply(rows)
            var taken = 0
            settingRows.forEach { c ->
                val value = (c.row["value"] as? JsonPrimitive)?.content
                if (!c.deleted && value != null && settings.applySynced(c.uid, value, c.updatedAt)) taken++
            }
            // The PC has every tombstone from before this sync began.
            ledger.pruneTombstones(started)
            store.recordSuccess(System.currentTimeMillis(), cursor, started, local.size, applied.applied + taken)
            SyncOutcome.Done(sent = local.size, received = applied.applied + taken)
        } catch (e: CancellationException) {
            throw e
        } catch (e: RelayFailure) {
            store.recordFailure(System.currentTimeMillis(), e.reason)
            SyncOutcome.Failed(e.reason)
        } catch (e: Exception) {
            val reason = "The sync did not finish. Nothing here was changed."
            store.recordFailure(System.currentTimeMillis(), reason)
            SyncOutcome.Failed(reason)
        }
    }

    /** The synced settings changed since [since] (all of them on a first sync), as the PC keeps them. */
    private suspend fun settingsChanges(since: Long?): List<Change> =
        settings.synced().all
            .filter { since == null || it.updatedAt >= since }
            // A setting never changed here still has a value; the PC needs a time above 0 to take it.
            .map { s -> Change(SyncTables.SETTINGS, s.key, maxOf(s.updatedAt, 1L), row = buildJsonObject { put("value", s.value) }) }

    companion object {
        /** Changes per request: a few hundred kilobytes, far under the door's 8 MiB message. */
        const val BATCH = 1_500
    }
}
