package com.tally.app.data.db

import androidx.room.RoomDatabase
import androidx.room.migration.Migration
import androidx.sqlite.db.SupportSQLiteDatabase

/**
 * What lets this ledger sync with the PC (docs/MONEY.md, "Sync"): a permanent uid and a change
 * time on every row, and a tombstone for every delete. The database keeps them itself, with
 * triggers, so no write path in the app can forget to: an insert or an update stamps the row with
 * the time, a delete (cascades included) leaves a tombstone, and a row's uid never changes.
 *
 * Every trigger but the delete one stands aside while a sync applies the PC's rows (the
 * "applying" row in sync_state), so those keep the PC's change time and uid.
 */
object SyncSchema {

    /** The synced Room tables, by their SQL name, in the order the PC applies them. */
    val TABLES = listOf("accounts", "categories", "recurring", "goals", "transactions", "budgets", "goal_contributions", "account_values")

    /*
     * Android builds SQLite with recursive triggers on, so a trigger's own write can fire it
     * again. Each WHEN below is false for the write its body makes, which ends the recursion.
     */

    /** Epoch millis inside SQLite. One value for a whole statement, its triggers included. */
    private const val NOW = "CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)"

    private const val LOCAL = "NOT EXISTS (SELECT 1 FROM sync_state WHERE key = 'applying')"

    /** A random UUID v4 in SQL, for rows that existed before uids did. */
    private const val UUID_SQL =
        "lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || substr(lower(hex(randomblob(2))), 2) || '-' || " +
            "substr('89ab', 1 + (abs(random()) % 4), 1) || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6)))"

    /** The triggers for [table]. Idempotent, so a fresh database and a migrated one get the same set. */
    private fun triggers(table: String): List<String> {
        // A budget is synced by its category, so its tombstone names it. One whose category is
        // already gone says nothing: the PC drops it with the category's own tombstone.
        val tombstone = if (table == "budgets") {
            """CREATE TRIGGER IF NOT EXISTS budgets_sync_delete AFTER DELETE ON budgets
               WHEN OLD.categoryId = 0 OR EXISTS (SELECT 1 FROM categories WHERE id = OLD.categoryId)
               BEGIN
                 INSERT OR REPLACE INTO sync_tombstones (tableName, uid, deletedAt, category)
                 VALUES ('budgets', OLD.uid, $NOW, (SELECT uid FROM categories WHERE id = OLD.categoryId));
               END"""
        } else {
            """CREATE TRIGGER IF NOT EXISTS ${table}_sync_delete AFTER DELETE ON $table
               BEGIN
                 INSERT OR REPLACE INTO sync_tombstones (tableName, uid, deletedAt, category) VALUES ('$table', OLD.uid, $NOW, NULL);
               END"""
        }
        return listOf(
            // A new row is new now, and a row that comes back (an Undo) is no longer deleted.
            """CREATE TRIGGER IF NOT EXISTS ${table}_sync_insert AFTER INSERT ON $table WHEN $LOCAL
               BEGIN
                 DELETE FROM sync_tombstones WHERE tableName = '$table' AND uid = NEW.uid;
                 UPDATE $table SET updatedAt = $NOW WHERE id = NEW.id AND updatedAt < $NOW;
               END""",
            // An edit is newer than both the clock and the version it replaced, even when the
            // writer handed back a stale copy of the row.
            """CREATE TRIGGER IF NOT EXISTS ${table}_sync_update AFTER UPDATE ON $table
               WHEN $LOCAL AND NEW.updatedAt < MAX($NOW, OLD.updatedAt + 1)
               BEGIN
                 UPDATE $table SET updatedAt = MAX($NOW, OLD.updatedAt + 1) WHERE id = NEW.id;
               END""",
            // An editor that rebuilds a row from its fields carries a fresh default uid; the
            // row keeps the one it has. Android's SQLite runs triggers recursively, so the
            // restoring write raises a flag this trigger waits out instead of undoing it again.
            """CREATE TRIGGER IF NOT EXISTS ${table}_sync_uid AFTER UPDATE OF uid ON $table
               WHEN NEW.uid <> OLD.uid AND NOT EXISTS (SELECT 1 FROM sync_state WHERE key IN ('applying', 'keeping_uid'))
               BEGIN
                 INSERT OR REPLACE INTO sync_state (key, value) VALUES ('keeping_uid', 1);
                 UPDATE $table SET uid = OLD.uid WHERE id = NEW.id;
                 DELETE FROM sync_state WHERE key = 'keeping_uid';
               END""",
            tombstone,
        )
    }

    fun createTriggers(db: SupportSQLiteDatabase) {
        TABLES.forEach { table -> triggers(table).forEach(db::execSQL) }
    }

    /** Fresh installs: Room creates the tables, this adds the triggers. */
    val callback = object : RoomDatabase.Callback() {
        override fun onCreate(db: SupportSQLiteDatabase) = createTriggers(db)
    }

    /**
     * 3: uids, change times and tombstones. Every existing row gets a random uid and "now" as its
     * change time, so the first sync carries all of it.
     */
    val MIGRATION_2_3 = object : Migration(2, 3) {
        override fun migrate(db: SupportSQLiteDatabase) {
            TABLES.forEach { table ->
                db.execSQL("ALTER TABLE `$table` ADD COLUMN `uid` TEXT NOT NULL DEFAULT ''")
                db.execSQL("ALTER TABLE `$table` ADD COLUMN `updatedAt` INTEGER NOT NULL DEFAULT 0")
                db.execSQL("UPDATE `$table` SET uid = $UUID_SQL, updatedAt = $NOW")
                db.execSQL("CREATE UNIQUE INDEX IF NOT EXISTS `index_${table}_uid` ON `$table` (`uid`)")
            }
            db.execSQL(
                "CREATE TABLE IF NOT EXISTS `sync_tombstones` (`tableName` TEXT NOT NULL, `uid` TEXT NOT NULL, " +
                    "`deletedAt` INTEGER NOT NULL, `category` TEXT, PRIMARY KEY(`tableName`, `uid`))"
            )
            db.execSQL("CREATE INDEX IF NOT EXISTS `index_sync_tombstones_deletedAt` ON `sync_tombstones` (`deletedAt`)")
            db.execSQL("CREATE TABLE IF NOT EXISTS `sync_state` (`key` TEXT NOT NULL, `value` INTEGER NOT NULL, PRIMARY KEY(`key`))")
            createTriggers(db)
        }
    }
}

/** The one place a [TallyDatabase] builder is finished, so the app and every test get the same database. */
fun RoomDatabase.Builder<TallyDatabase>.tally(): RoomDatabase.Builder<TallyDatabase> =
    addMigrations(SyncSchema.MIGRATION_2_3).addCallback(SyncSchema.callback)
