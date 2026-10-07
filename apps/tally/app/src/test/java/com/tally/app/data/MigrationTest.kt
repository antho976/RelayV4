package com.tally.app.data

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import androidx.room.Room
import androidx.test.core.app.ApplicationProvider
import com.tally.app.data.db.AccountEntity
import com.tally.app.data.db.TallyDatabase
import com.tally.app.data.db.TransactionEntity
import com.tally.app.data.db.tally
import com.tally.core.AccountType
import com.tally.core.TxType
import com.tally.core.GoalKind
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.time.LocalDate

/**
 * A ledger written by the first release must open in this one with nothing lost. The version 1
 * database is built from its exported schema, exactly as that release created it, filled, and
 * then opened by Room at the current version: the auto-migration runs and Room checks the result
 * against the current schema, so a missing default or a wrong column fails here, not on a phone.
 */
@RunWith(RobolectricTestRunner::class)
class MigrationTest {

    private val context: Context = ApplicationProvider.getApplicationContext()
    private val name = "migration-test.db"
    private var opened: TallyDatabase? = null

    @After fun clean() {
        opened?.close()
        context.deleteDatabase(name)
    }

    /** A released version's tables, indices and Room's own bookkeeping, from its exported schema. */
    private fun schemaStatements(version: Int): List<String> {
        val text = checkNotNull(javaClass.classLoader?.getResourceAsStream("com.tally.app.data.db.TallyDatabase/$version.json")) {
            "The version $version schema is not on the test classpath"
        }.bufferedReader().use { it.readText() }
        val db = JSONObject(text).getJSONObject("database")
        val out = ArrayList<String>()
        val entities = db.getJSONArray("entities")
        for (i in 0 until entities.length()) {
            val e = entities.getJSONObject(i)
            val table = e.getString("tableName")
            out += e.getString("createSql").replace("\${TABLE_NAME}", table)
            val indices = e.optJSONArray("indices") ?: continue
            for (k in 0 until indices.length()) {
                val index = indices.getJSONObject(k)
                out += index.getString("createSql").replace("\${TABLE_NAME}", table).replace("\${INDEX_NAME}", index.getString("name"))
            }
        }
        val setup = db.getJSONArray("setupQueries")
        for (i in 0 until setup.length()) out += setup.getString(i)
        return out
    }

    @Test fun aVersion1LedgerOpensWithEverythingInIt() = runTest {
        val file = context.getDatabasePath(name)
        file.parentFile?.mkdirs()
        val day = LocalDate.of(2026, 9, 14).toEpochDay()
        SQLiteDatabase.openOrCreateDatabase(file, null).use { v1 ->
            schemaStatements(1).forEach { v1.execSQL(it) }
            v1.execSQL("INSERT INTO accounts (id, name, type, openingBalance, archived, sortOrder) VALUES (1, 'Chequing', 'CHEQUING', 100000, 0, 0)")
            v1.execSQL("INSERT INTO categories (id, name, kind, color, icon, archived, sortOrder) VALUES (1, 'Groceries', 'EXPENSE', 0, 'cart', 0, 0)")
            v1.execSQL(
                "INSERT INTO transactions (id, type, amount, date, accountId, toAccountId, categoryId, note, recurringId, createdAt) " +
                    "VALUES (1, 'EXPENSE', 6842, $day, 1, NULL, 1, 'Metro', NULL, 0)"
            )
            v1.execSQL("INSERT INTO goals (id, name, target, targetDate, color, archived) VALUES (1, 'Lisbon in May', 320000, NULL, 1, 0)")
            v1.execSQL("INSERT INTO goal_contributions (id, goalId, amount, date, note) VALUES (1, 1, 50000, $day, '')")
            v1.version = 1
        }

        val db = Room.databaseBuilder(context, TallyDatabase::class.java, name).allowMainThreadQueries().tally().build()
        opened = db

        val goal = db.goals().all().single()
        assertEquals("Lisbon in May", goal.name)
        assertEquals("A goal from before kinds is a savings pot", GoalKind.SAVINGS, goal.kind)
        assertNull(goal.accountId)
        assertEquals(0, goal.percent)
        assertNull(goal.startDate)
        assertEquals(0L, goal.startAmount)
        assertEquals(1, db.goals().allContributions().size)

        val balance = db.accounts().observeBalances().first().single()
        assertEquals(100_000L - 6_842, balance.balance)
        assertNull("No values were recorded before version 2", balance.valuedOn)
        assertTrue(db.values().all().isEmpty())
    }

    /**
     * Version 2 to 3 adds what sync needs. Every row a version 2 phone holds gets its own uid and a
     * change time, nothing else about it moves, and from then on the triggers keep both: a delete
     * leaves a tombstone, its cascade one for each row it takes.
     */
    @Test fun aVersion2LedgerOpensReadyToSync() = runTest {
        val file = context.getDatabasePath(name)
        file.parentFile?.mkdirs()
        val day = LocalDate.of(2026, 10, 1).toEpochDay()
        SQLiteDatabase.openOrCreateDatabase(file, null).use { v2 ->
            schemaStatements(2).forEach { v2.execSQL(it) }
            v2.execSQL("INSERT INTO accounts (id, name, type, openingBalance, archived, sortOrder) VALUES (1, 'Chequing', 'CHEQUING', 100000, 0, 0)")
            v2.execSQL("INSERT INTO accounts (id, name, type, openingBalance, archived, sortOrder) VALUES (2, 'Visa', 'CREDIT', 0, 0, 1)")
            v2.execSQL("INSERT INTO categories (id, name, kind, color, icon, archived, sortOrder) VALUES (1, 'Groceries', 'EXPENSE', 0, 'cart', 0, 0)")
            v2.execSQL(
                "INSERT INTO transactions (id, type, amount, date, accountId, toAccountId, categoryId, note, recurringId, createdAt) " +
                    "VALUES (1, 'EXPENSE', 6842, $day, 1, NULL, 1, 'Metro', NULL, 0), (2, 'EXPENSE', 1200, $day, 2, NULL, 1, 'Cafe', NULL, 0)"
            )
            v2.execSQL("INSERT INTO budgets (id, categoryId, amount) VALUES (1, 0, 310000), (2, 1, 60000)")
            v2.execSQL(
                "INSERT INTO recurring (id, name, type, amount, accountId, toAccountId, categoryId, frequency, interval, anchorDate, nextDate, endDate, autoPost, active) " +
                    "VALUES (1, 'Rent', 'EXPENSE', 150000, 1, NULL, NULL, 'MONTHLY', 1, $day, $day, NULL, 1, 1)"
            )
            v2.execSQL("INSERT INTO goals (id, name, target, targetDate, color, archived, kind, accountId, percent, startDate, startAmount) VALUES (1, 'Lisbon', 320000, NULL, 1, 0, 'SAVINGS', NULL, 0, NULL, 0)")
            v2.execSQL("INSERT INTO goal_contributions (id, goalId, amount, date, note) VALUES (1, 1, 50000, $day, '')")
            v2.execSQL("INSERT INTO account_values (id, accountId, date, value) VALUES (1, 2, $day, -5000)")
            v2.version = 2
        }
        val before = System.currentTimeMillis()

        val db = Room.databaseBuilder(context, TallyDatabase::class.java, name).allowMainThreadQueries().tally().build()
        opened = db

        val uids = db.accounts().all().map { it.uid } + db.categories().all().map { it.uid } + db.transactions().all().map { it.uid } +
            db.budgets().all().map { it.uid } + db.recurring().all().map { it.uid } + db.goals().all().map { it.uid } +
            db.goals().allContributions().map { it.uid } + db.values().all().map { it.uid }
        assertEquals("Every row was kept", 11, uids.size)
        assertTrue("Every row has a uuid", uids.all { Regex("[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}").matches(it) })
        assertEquals("No two rows share one", uids.size, uids.toSet().size)
        assertTrue("Every row counts as changed now", db.transactions().all().all { it.updatedAt >= before - 1_000 })

        val balances = db.accounts().observeBalances().first().associate { it.name to it.balance }
        assertEquals(100_000L - 6_842, balances["Chequing"])
        assertEquals(310_000L, db.budgets().getFor(0)!!.amount)
        assertTrue("Nothing was deleted by the migration", db.sync().tombstones().isEmpty())

        // The triggers came with it: deleting Visa takes its entry and its value, and says so.
        val visa = db.accounts().get(2)!!
        val cafe = db.transactions().get(2)!!
        db.accounts().delete(visa)
        val gone = db.sync().tombstones().associate { it.tableName to it.uid }
        assertEquals(visa.uid, gone["accounts"])
        assertEquals(cafe.uid, gone["transactions"])
        assertNotNull(gone["account_values"])

        // And a new row gets a uid of its own.
        val id = db.accounts().insert(AccountEntity(name = "Cash", type = AccountType.CASH))
        assertNotEquals(visa.uid, db.accounts().get(id)!!.uid)
        db.transactions().insert(TransactionEntity(type = TxType.EXPENSE, amount = 500, date = LocalDate.of(2026, 10, 2), accountId = id))
    }
}
