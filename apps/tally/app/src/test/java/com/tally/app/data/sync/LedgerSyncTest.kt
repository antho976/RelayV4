package com.tally.app.data.sync

import com.tally.app.data.FixedClock
import com.tally.app.data.RoomTestDb
import com.tally.app.data.addAccount
import com.tally.app.data.addCategory
import com.tally.app.data.addExpense
import com.tally.app.data.db.AccountEntity
import com.tally.app.data.db.BudgetEntity
import com.tally.app.data.db.TallyDatabase
import com.tally.app.data.ledgerRepository
import com.tally.app.data.planRepository
import com.tally.core.AccountType
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.time.LocalDate

/**
 * The ledger's half of sync: what the triggers keep, what goes to the PC, and how the PC's
 * changes are merged (newest wins, tombstones delete, uids become ids, budgets by category).
 */
@RunWith(RobolectricTestRunner::class)
class LedgerSyncTest {

    private val day = LocalDate.of(2026, 10, 5)
    private val clock = FixedClock(day)
    private lateinit var db: TallyDatabase
    private lateinit var sync: LedgerSync

    /** Far ahead of any stamp a test run makes: the PC's newer edit. */
    private val future = System.currentTimeMillis() + 3_600_000L

    @Before fun open() {
        db = RoomTestDb.create()
        sync = LedgerSync(db)
    }

    @After fun close() { db.close() }

    private fun accountRow(name: String) = buildJsonObject {
        put("name", name)
        put("type", "CHEQUING")
        put("openingBalance", 1_000_00)
        put("archived", false)
        put("sortOrder", 0)
    }

    private fun expenseRow(account: String, amount: Long, category: String? = null) = buildJsonObject {
        put("type", "EXPENSE")
        put("amount", amount)
        put("date", "2026-10-05")
        put("account", account)
        put("toAccount", JsonNull)
        put("category", category)
        put("note", "Metro")
        put("recurring", JsonNull)
        put("createdAt", 100)
    }

    // ── What the database keeps by itself ────────────────────────────────────

    @Test fun everyWriteIsStampedAndAnEditorCannotChangeAUid() = runTest {
        val id = db.accounts().insert(AccountEntity(name = "Chequing", type = AccountType.CHEQUING, updatedAt = 5L))
        val stored = db.accounts().get(id)!!
        assertTrue("An insert is stamped now, whatever the row said", stored.updatedAt > 5L)

        // An editor rebuilds the row from its fields: a fresh default uid, a stale change time.
        db.accounts().update(AccountEntity(id = id, name = "Everyday", type = AccountType.CHEQUING, updatedAt = 5L))
        val edited = db.accounts().get(id)!!
        assertEquals("Everyday", edited.name)
        assertEquals("The uid never changes", stored.uid, edited.uid)
        assertTrue("An edit is newer than what it replaced", edited.updatedAt > stored.updatedAt)
    }

    @Test fun deletesLeaveTombstonesCascadesIncludedAndUndoTakesThemBack() = runTest {
        val ledger = db.ledgerRepository(clock)
        val account = db.addAccount("Visa", AccountType.CREDIT)
        val food = db.addCategory("Food")
        val tx = db.addExpense(12_00, day, account, food)
        val row = db.transactions().get(tx)!!

        val deleted = ledger.delete(tx)!!
        assertEquals(row.uid, db.sync().tombstone("transactions", row.uid)?.uid)

        ledger.restore(deleted)
        assertNull("Undo brings the row back, so it is not deleted on the PC", db.sync().tombstone("transactions", row.uid))
        val back = db.transactions().get(tx)!!
        assertTrue("and it counts as changed again", back.updatedAt >= db.sync().tombstones().maxOfOrNull { it.deletedAt } ?: 0L)

        val accountUid = db.accounts().get(account)!!.uid
        ledger.deleteAccount(account)
        val gone = db.sync().tombstones().map { it.tableName to it.uid }.toSet()
        assertTrue(("accounts" to accountUid) in gone)
        assertTrue("The account's entry went with it, and says so", ("transactions" to row.uid) in gone)
    }

    @Test fun aBudgetsTombstoneNamesItsCategory() = runTest {
        val ledger = db.ledgerRepository(clock)
        val plan = db.planRepository(clock)
        val food = db.addCategory("Food")
        val foodUid = db.categories().get(food)!!.uid
        plan.setBudget(food, 400_00)
        plan.setBudget(BudgetEntity.OVERALL, 2_000_00)
        val overallUid = db.budgets().getFor(0)!!.uid

        ledger.deleteCategory(food, moveTo = null)
        plan.setBudget(BudgetEntity.OVERALL, 0)

        val budgets = sync.collect(since = 0L).filter { it.table == "budgets" }
        assertEquals(2, budgets.size)
        val byCategory = budgets.associate { it.row["category"] to it }
        assertTrue(byCategory.getValue(JsonPrimitive(foodUid)).deleted)
        assertEquals(overallUid, byCategory.getValue(JsonNull).uid)
    }

    // ── What goes to the PC ──────────────────────────────────────────────────

    @Test fun aFirstSyncSendsEverythingWithReferencesAsUids() = runTest {
        val account = db.addAccount("Chequing")
        val food = db.addCategory("Food")
        db.addExpense(42_50, day, account, food, note = "Metro")
        db.planRepository(clock).setBudget(BudgetEntity.OVERALL, 310_00)
        db.addExpense(1_00, day, account).also { db.ledgerRepository(clock).delete(it) }

        val changes = sync.collect(since = null)

        assertEquals(listOf("accounts", "categories", "transactions", "budgets"), changes.map { it.table })
        assertTrue("A first sync replaces the PC: no tombstones", changes.none { it.deleted })
        val tx = changes.single { it.table == "transactions" }
        assertEquals(JsonPrimitive(db.accounts().get(account)!!.uid), tx.row["account"])
        assertEquals(JsonPrimitive(db.categories().get(food)!!.uid), tx.row["category"])
        assertEquals(JsonPrimitive(42_50L), tx.row["amount"])
        assertEquals(JsonPrimitive("2026-10-05"), tx.row["date"])
        assertEquals(JsonNull, changes.single { it.table == "budgets" }.row["category"])
        assertEquals("updated_at travels snake case", true, tx.toJson().containsKey("updated_at"))
    }

    @Test fun laterSyncsSendOnlyWhatChangedSinceTheWatermark() = runTest {
        val account = db.addAccount("Chequing")
        db.addExpense(5_00, day, account)
        val watermark = System.currentTimeMillis() + 1
        Thread.sleep(5)
        val coffee = db.addExpense(4_00, day, account, note = "Coffee")
        db.ledgerRepository(clock).delete(db.addExpense(9_00, day, account))

        val changes = sync.collect(since = watermark)

        assertEquals(2, changes.size)
        assertEquals(db.transactions().get(coffee)!!.uid, changes.first { !it.deleted }.uid)
        assertTrue(changes.any { it.deleted && it.table == "transactions" })
    }

    // ── What comes back from the PC ──────────────────────────────────────────

    @Test fun thePcsRowsArriveWithTheirUidsMappedToLocalIds() = runTest {
        val applied = sync.apply(
            listOf(
                // Out of order on purpose: the entry needs its account first.
                Change("transactions", "t1", future, row = expenseRow("a1", 42_50)),
                Change("accounts", "a1", future, row = accountRow("Chequing")),
            )
        )

        assertEquals(Applied(2, 0), applied)
        val account = db.accounts().all().single()
        assertEquals("a1", account.uid)
        assertEquals("The PC's change time is kept", future, account.updatedAt)
        val tx = db.transactions().all().single()
        assertEquals(account.id, tx.accountId)
        assertEquals("t1", tx.uid)
        assertTrue("Applying is not a local change: nothing to tell the PC", db.sync().tombstones().isEmpty())
    }

    @Test fun theNewestEditWinsAndAnOlderOneIsIgnored() = runTest {
        sync.apply(listOf(Change("accounts", "a1", future, row = accountRow("Chequing"))))

        val older = sync.apply(listOf(Change("accounts", "a1", future - 10, row = accountRow("Older"))))
        val equal = sync.apply(listOf(Change("accounts", "a1", future, row = accountRow("Same time"))))
        assertEquals(Applied(0, 2), Applied(older.applied + equal.applied, older.skipped + equal.skipped))
        assertEquals("Chequing", db.accounts().all().single().name)

        sync.apply(listOf(Change("accounts", "a1", future + 10, row = accountRow("Everyday"))))
        assertEquals("Everyday", db.accounts().all().single().name)

        // An edit here is newer than anything the PC sent before it.
        val local = db.accounts().all().single()
        db.accounts().update(local.copy(name = "Mine"))
        sync.apply(listOf(Change("accounts", "a1", future + 5, row = accountRow("Stale"))))
        assertEquals("Mine", db.accounts().all().single().name)
    }

    @Test fun tombstonesDeleteAndALocalDeleteNewerThanTheEditStands() = runTest {
        sync.apply(
            listOf(
                Change("accounts", "a1", future, row = accountRow("Chequing")),
                Change("transactions", "t1", future, row = expenseRow("a1", 10_00)),
                Change("transactions", "t2", future, row = expenseRow("a1", 20_00)),
            )
        )

        sync.apply(listOf(Change("transactions", "t1", future + 1, deleted = true)))
        assertEquals(listOf("t2"), db.transactions().all().map { it.uid })
        assertNull("The PC sent that delete; it is not sent back", db.sync().tombstone("transactions", "t1"))

        // Deleted here after the PC's last edit of it.
        db.ledgerRepository(clock).delete(db.transactions().all().single().id)
        val deletedAt = db.sync().tombstone("transactions", "t2")!!.deletedAt
        val stale = sync.apply(listOf(Change("transactions", "t2", deletedAt - 1, row = expenseRow("a1", 25_00))))
        assertEquals(Applied(0, 1), stale)
        assertTrue(db.transactions().all().isEmpty())

        // Edited on the PC after the delete here: the edit is newer and brings it back.
        sync.apply(listOf(Change("transactions", "t2", deletedAt + 1, row = expenseRow("a1", 25_00))))
        assertEquals(25_00L, db.transactions().all().single().amount)
        assertNull(db.sync().tombstone("transactions", "t2"))
    }

    @Test fun aRowThatPointsAtSomethingMissingIsSkippedNotFatal() = runTest {
        val applied = sync.apply(
            listOf(
                Change("transactions", "t1", future, row = expenseRow("nowhere", 10_00)),
                Change("accounts", "a1", future, row = accountRow("Chequing")),
                Change("transactions", "t2", future, row = expenseRow("a1", 10_00, category = "unknown")),
            )
        )
        assertEquals(Applied(2, 1), applied)
        assertNull("A category this phone does not have reads as uncategorized", db.transactions().all().single().categoryId)
    }

    @Test fun budgetsMatchByCategoryWhateverTheirUid() = runTest {
        val plan = db.planRepository(clock)
        val food = db.addCategory("Food")
        val foodUid = db.categories().get(food)!!.uid
        plan.setBudget(food, 400_00)
        plan.setBudget(BudgetEntity.OVERALL, 2_000_00)

        sync.apply(
            listOf(
                Change("budgets", "pc-food", future, row = buildJsonObject { put("category", foodUid); put("amount", 450_00) }),
                Change("budgets", "pc-overall", future, row = buildJsonObject { put("category", JsonNull); put("amount", 2_500_00) }),
            )
        )

        val budgets = db.budgets().all().associateBy { it.categoryId }
        assertEquals(2, budgets.size)
        assertEquals(450_00L, budgets.getValue(food).amount)
        assertEquals(2_500_00L, budgets.getValue(0).amount)
        assertEquals("The PC's uid is taken, so both sides name it the same", "pc-food", budgets.getValue(food).uid)

        sync.apply(listOf(Change("budgets", "pc-overall", future + 1, deleted = true, row = buildJsonObject { put("category", JsonNull) })))
        assertNull(db.budgets().getFor(0))
        assertNotNull(db.budgets().getFor(food))
    }

    @Test fun aDeletedAccountOnThePcTakesItsEntriesHereToo() = runTest {
        sync.apply(
            listOf(
                Change("accounts", "a1", future, row = accountRow("Chequing")),
                Change("transactions", "t1", future, row = expenseRow("a1", 10_00)),
            )
        )
        sync.apply(listOf(Change("accounts", "a1", future + 1, deleted = true, row = JsonObject(emptyMap()))))
        assertTrue(db.accounts().all().isEmpty())
        assertTrue(db.transactions().all().isEmpty())
        assertFalse("The cascade's entry is told to the PC, in case it kept it", db.sync().tombstones().none { it.uid == "t1" })
    }

    @Test fun changesReadBackFromTheWire() {
        val c = Change("transactions", "t1", 1234, deleted = false, row = expenseRow("a1", 5))
        assertEquals(c, Change.fromJson(c.toJson()))
        assertNull(Change.fromJson(JsonPrimitive("nonsense")))
    }

    @Test fun aDeleteOutranksTheRowEvenWhenItsStampIsAhead() = runTest {
        sync.apply(listOf(Change("accounts", "a1", future, row = accountRow("Chequing"))))
        db.ledgerRepository(clock).deleteAccount(db.accounts().all().single().id)
        assertEquals("The tombstone is stamped past the row it removes", future + 1, db.sync().tombstone("accounts", "a1")!!.deletedAt)
    }

    @Test fun aRowThatComesBackIsStampedPastItsTombstone() = runTest {
        val ledger = db.ledgerRepository(clock)
        sync.apply(
            listOf(
                Change("accounts", "a1", future, row = accountRow("Chequing")),
                Change("transactions", "t1", future, row = expenseRow("a1", 10_00)),
            )
        )
        val deleted = ledger.delete(db.transactions().all().single().id)!!
        val buried = db.sync().tombstone("transactions", "t1")!!.deletedAt
        ledger.restore(deleted)
        assertTrue(db.transactions().all().single().updatedAt > buried)
        assertNull(db.sync().tombstone("transactions", "t1"))
    }

    @Test fun aBudgetTombstoneWithAnotherUidIsFoundByItsCategory() = runTest {
        val plan = db.planRepository(clock)
        val food = db.addCategory("Food")
        val foodUid = db.categories().get(food)!!.uid
        plan.setBudget(food, 400_00)
        plan.setBudget(BudgetEntity.OVERALL, 2_000_00)

        sync.apply(listOf(Change("budgets", "pc-food", future, deleted = true, row = buildJsonObject { put("category", foodUid) })))
        sync.apply(listOf(Change("budgets", "pc-overall", future, deleted = true, row = buildJsonObject { put("category", JsonNull) })))

        assertTrue(db.budgets().all().isEmpty())
    }

    @Test fun takingThePcsLedgerWholeLeavesNothingOfThisOneBehind() = runTest {
        val account = db.addAccount("Mine")
        db.addExpense(5_00, day, account)
        db.ledgerRepository(clock).delete(db.addExpense(6_00, day, account))

        val applied = sync.replaceWith(
            listOf(
                Change("accounts", "a1", 100, row = accountRow("Theirs")),
                Change("transactions", "t1", 100, row = expenseRow("a1", 7_00)),
                Change("transactions", "t-gone", 100, deleted = true),
            )
        )

        assertEquals(Applied(2, 0), applied)
        assertEquals(listOf("Theirs"), db.accounts().all().map { it.name })
        assertEquals(listOf(100L), db.transactions().all().map { it.updatedAt })
        assertTrue(db.sync().tombstones().isEmpty())
    }
}
