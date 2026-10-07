package com.tally.app.data.sync

import android.content.Context
import com.tally.app.data.db.SyncSchema
import com.tally.app.data.db.TallyDatabase
import com.tally.app.data.prefs.SettingsRepository
import com.tally.app.di.AppScope
import com.tally.app.work.SyncWorker
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.merge
import kotlinx.coroutines.launch
import javax.inject.Inject
import javax.inject.Singleton

/**
 * When the phone syncs with its PC: when the app comes to the front, a few seconds after a change
 * here (so a burst of edits is one sync), and every half hour in the background (SyncWorker).
 * Nothing happens until a PC is paired.
 */
@Singleton
class SyncScheduler @Inject constructor(
    @ApplicationContext private val context: Context,
    private val db: TallyDatabase,
    private val settings: SettingsRepository,
    private val store: SyncStore,
    private val sync: PcSync,
    @AppScope private val scope: CoroutineScope,
) {
    private var started = false
    private var pending: Job? = null
    @Volatile private var dirty = false

    /** Once, from the Application. */
    @OptIn(FlowPreview::class)
    fun start() {
        if (started) return
        started = true
        scope.launch {
            // The periodic run follows the pairing: on while paired, gone once forgotten.
            store.prefs.map { it.paired }.distinctUntilChanged().collect { SyncWorker.sync(context, it) }
        }
        scope.launch {
            val ledger = db.invalidationTracker.createFlow(*SyncSchema.TABLES.toTypedArray(), emitInitialState = false).map { }
            val format = settings.settings.map { Triple(it.currency, it.monthStartDay, it.weekStartsMonday) }.distinctUntilChanged().drop(1).map { }
            merge(ledger, format)
                // While a sync runs, and just after, a write may be the sync's own or an edit
                // made meanwhile; it cannot be told which, so it is noted and the sync runs once
                // more when it ends (below). An edit is never left waiting for the next trigger.
                .filter {
                    val quiet = !sync.busy.value && System.currentTimeMillis() - sync.lastFinishedAt > SETTLE_MS
                    if (!quiet) dirty = true
                    quiet
                }
                .debounce(DEBOUNCE_MS)
                .collect { syncIfPaired() }
        }
        scope.launch {
            sync.busy.collectLatest { busy ->
                if (busy) return@collectLatest
                delay(SETTLE_MS + 100)
                if (dirty && !sync.busy.value) {
                    dirty = false
                    later(DEBOUNCE_MS)
                }
            }
        }
    }

    /** The app came to the front. Skipped when a sync ran moments ago. */
    fun onForeground() {
        scope.launch {
            val prefs = store.current()
            if (!prefs.paired) return@launch
            if (System.currentTimeMillis() - prefs.lastAttemptAt < FOREGROUND_GAP_MS) return@launch
            later(0L)
        }
    }

    private suspend fun syncIfPaired() {
        if (store.current().paired) sync.syncNow()
    }

    private fun later(delayMs: Long) {
        pending?.cancel()
        pending = scope.launch(Dispatchers.IO) {
            delay(delayMs)
            syncIfPaired()
        }
    }

    companion object {
        /** A burst of edits settles into one sync. */
        const val DEBOUNCE_MS = 5_000L
        /** Invalidations this soon after a sync are its own writes arriving late. */
        const val SETTLE_MS = 1_500L
        /** Coming back to the app twice in a minute syncs once. */
        const val FOREGROUND_GAP_MS = 60_000L
    }
}
