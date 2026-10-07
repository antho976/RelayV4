package com.tally.app.data.sync

import android.content.Context
import androidx.compose.runtime.Immutable
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import java.io.IOException
import javax.inject.Inject
import javax.inject.Singleton

/** The PC this phone is paired with, and the credential the PC gave it. */
@Immutable
data class PairedPc(
    val name: String,
    val hostId: String,
    val instance: String,
    /** Where to reach it, in the order tried: direct addresses, then its rendezvous. */
    val routes: List<String>,
    val deviceId: String,
    /** The phone's token. Never shown and never sent again: each connection sends a proof. */
    val token: String,
    val pairedAt: Long,
)

/** Where sync with the paired PC stands. Everything but [pc] is about the last run. */
@Immutable
data class SyncPrefs(
    val pc: PairedPc? = null,
    /** The PC's cursor from the last sync, sent back as `since`. */
    val cursor: Long = 0L,
    /** When the last successful sync started: rows stamped from then on are sent next time. */
    val watermark: Long = 0L,
    /** The first sync, which makes the PC's ledger this phone's, has been done. */
    val replaced: Boolean = false,
    val lastAttemptAt: Long = 0L,
    val lastSuccessAt: Long = 0L,
    /** Why the last run failed, in words; null when it worked. */
    val lastError: String? = null,
    val lastSent: Int = 0,
    val lastReceived: Int = 0,
) {
    val paired: Boolean get() = pc != null
}

/**
 * Kept in its own private DataStore file, apart from the settings, and left out of every backup
 * (res/xml): the token is this phone's key to the PC and stays on this phone.
 */
private val Context.syncStore: DataStore<Preferences> by preferencesDataStore(name = "relay_sync")

@Singleton
class SyncStore @Inject constructor(@ApplicationContext context: Context) {

    private val store = context.syncStore

    private object Keys {
        val name = stringPreferencesKey("pc_name")
        val hostId = stringPreferencesKey("pc_host_id")
        val instance = stringPreferencesKey("pc_instance")
        val routes = stringPreferencesKey("pc_routes")
        val deviceId = stringPreferencesKey("device_id")
        val token = stringPreferencesKey("device_token")
        val pairedAt = longPreferencesKey("paired_at")
        val cursor = longPreferencesKey("cursor")
        val watermark = longPreferencesKey("watermark")
        val replaced = booleanPreferencesKey("replaced")
        val lastAttemptAt = longPreferencesKey("last_attempt_at")
        val lastSuccessAt = longPreferencesKey("last_success_at")
        val lastError = stringPreferencesKey("last_error")
        val lastSent = intPreferencesKey("last_sent")
        val lastReceived = intPreferencesKey("last_received")
    }

    val prefs: Flow<SyncPrefs> = store.data
        .catch { if (it is IOException) emit(emptyPreferences()) else throw it }
        .map { p ->
            val token = p[Keys.token]
            val device = p[Keys.deviceId]
            SyncPrefs(
                pc = if (token != null && device != null) {
                    PairedPc(
                        name = p[Keys.name] ?: "Relay PC",
                        hostId = p[Keys.hostId].orEmpty(),
                        instance = p[Keys.instance].orEmpty(),
                        routes = p[Keys.routes].orEmpty().split('\n').filter { it.isNotBlank() },
                        deviceId = device,
                        token = token,
                        pairedAt = p[Keys.pairedAt] ?: 0L,
                    )
                } else {
                    null
                },
                cursor = p[Keys.cursor] ?: 0L,
                watermark = p[Keys.watermark] ?: 0L,
                replaced = p[Keys.replaced] ?: false,
                lastAttemptAt = p[Keys.lastAttemptAt] ?: 0L,
                lastSuccessAt = p[Keys.lastSuccessAt] ?: 0L,
                lastError = p[Keys.lastError],
                lastSent = p[Keys.lastSent] ?: 0,
                lastReceived = p[Keys.lastReceived] ?: 0,
            )
        }
        .distinctUntilChanged()

    suspend fun current(): SyncPrefs = prefs.first()

    /** A new pairing starts from nothing: cursor 0, and the first sync still to come. */
    suspend fun pair(pc: PairedPc) = store.edit {
        it.clear()
        it[Keys.name] = pc.name
        it[Keys.hostId] = pc.hostId
        it[Keys.instance] = pc.instance
        it[Keys.routes] = pc.routes.joinToString("\n")
        it[Keys.deviceId] = pc.deviceId
        it[Keys.token] = pc.token
        it[Keys.pairedAt] = pc.pairedAt
    }

    /** The PC's name as it greeted last; it can be renamed on the PC. */
    suspend fun rename(name: String) = store.edit { if (it[Keys.token] != null) it[Keys.name] = name }

    suspend fun recordSuccess(at: Long, cursor: Long, watermark: Long, sent: Int, received: Int) = store.edit {
        if (it[Keys.token] == null) return@edit
        it[Keys.cursor] = cursor
        it[Keys.watermark] = watermark
        it[Keys.replaced] = true
        it[Keys.lastAttemptAt] = at
        it[Keys.lastSuccessAt] = at
        it.remove(Keys.lastError)
        it[Keys.lastSent] = sent
        it[Keys.lastReceived] = received
    }

    suspend fun recordFailure(at: Long, reason: String) = store.edit {
        it[Keys.lastAttemptAt] = at
        it[Keys.lastError] = reason
    }

    /** Forget this PC: its name, its routes, the token and where sync stood with it. */
    suspend fun forget() = store.edit { it.clear() }

    /** Undo for [forget]: everything back as it was, the token and the cursor included. */
    suspend fun restore(prefs: SyncPrefs) {
        val pc = prefs.pc ?: return
        pair(pc)
        store.edit {
            it[Keys.cursor] = prefs.cursor
            it[Keys.watermark] = prefs.watermark
            it[Keys.replaced] = prefs.replaced
            it[Keys.lastAttemptAt] = prefs.lastAttemptAt
            it[Keys.lastSuccessAt] = prefs.lastSuccessAt
            prefs.lastError?.let { e -> it[Keys.lastError] = e }
            it[Keys.lastSent] = prefs.lastSent
            it[Keys.lastReceived] = prefs.lastReceived
        }
    }
}
