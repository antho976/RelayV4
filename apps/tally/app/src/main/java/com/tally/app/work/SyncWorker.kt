package com.tally.app.work

import android.content.Context
import androidx.hilt.work.HiltWorker
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.tally.app.data.sync.PcSync
import dagger.assisted.Assisted
import dagger.assisted.AssistedInject
import java.util.concurrent.TimeUnit

/**
 * Syncs with the paired PC every half hour while there is a network, so the two ledgers meet
 * even on days the app is not opened. A failed run waits for the next one: the reason is the
 * line in Settings, never a retry storm or a notification.
 */
@HiltWorker
class SyncWorker @AssistedInject constructor(
    @Assisted context: Context,
    @Assisted params: WorkerParameters,
    private val sync: PcSync,
) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result {
        runCatching { sync.syncNow() }
        return Result.success()
    }

    companion object {
        private const val NAME = "pc-sync"

        /** Schedules the periodic run while a PC is paired, cancels it when not. Safe on every launch. */
        fun sync(context: Context, paired: Boolean) {
            val work = WorkManager.getInstance(context)
            if (paired) {
                val request = PeriodicWorkRequestBuilder<SyncWorker>(30, TimeUnit.MINUTES)
                    .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                    .build()
                work.enqueueUniquePeriodicWork(NAME, ExistingPeriodicWorkPolicy.KEEP, request)
            } else {
                work.cancelUniqueWork(NAME)
            }
        }
    }
}
