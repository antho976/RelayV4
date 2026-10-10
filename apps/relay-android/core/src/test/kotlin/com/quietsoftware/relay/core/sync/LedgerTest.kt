package com.quietsoftware.relay.core.sync

import com.quietsoftware.relay.core.sync.Ledger.Companion.isHidden
import com.quietsoftware.relay.core.testing.MemoryOutbox
import com.quietsoftware.relay.core.testing.MemoryReplica
import com.quietsoftware.relay.core.testing.obj
import com.quietsoftware.relay.core.wire.BusError
import com.quietsoftware.relay.core.wire.s
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LedgerTest {
    private val replica = MemoryReplica()
    private val outbox = MemoryOutbox()
    private val ledger = Ledger(replica, outbox) { 1_760_000_000_000 }

    private fun task(id: Long, title: String, column: String = "backlog") =
        obj("id" to id, "project_id" to 1, "title" to title, "body" to "", "column" to column, "labels" to emptyList<String>(), "updated_at" to "2026-10-09T00:00:00Z")

    private suspend fun shown(id: String) = replica.get(Kind.Task, id)!!.json

    @Test
    fun `an edit shows at once and survives the PC rewriting the row`() = runBlocking {
        ledger.replace(Kind.Task, Scope(projectId = 1), listOf(Row.of(Kind.Task, task(7, "old"))!!))
        ledger.enqueue("task.update", obj("task_id" to 7, "title" to "new", "expected" to obj("title" to "old")), "Rename #7")
        assertEquals("new", shown("7").s("title"))
        assertTrue(replica.get(Kind.Task, "7")!!.pending)

        // The PC sends its (older) row again, with another field changed there: both show.
        ledger.replace(Kind.Task, Scope(projectId = 1), listOf(Row.of(Kind.Task, task(7, "old", column = "ready"))!!))
        assertEquals("new", shown("7").s("title"))
        assertEquals("ready", shown("7").s("column"))
        assertEquals("old", replica.get(Kind.Task, "7")!!.base!!.s("title"))
    }

    @Test
    fun `the PC's answer replaces the overlay`() = runBlocking {
        ledger.upsert(listOf(Row.of(Kind.Task, task(7, "old"))!!))
        val e = ledger.enqueue("task.move", obj("task_id" to 7, "column" to "done"), "Move #7")
        assertEquals("done", shown("7").s("column"))
        outbox.update(e.copy(state = OutboxEntry.State.Done))
        ledger.applied(e, task(7, "old", column = "done"))
        assertFalse(replica.get(Kind.Task, "7")!!.pending)
    }

    @Test
    fun `a task made on the phone is a temp row until the PC names it, and later edits follow it`() = runBlocking {
        val create = ledger.enqueue("task.create", obj("project_id" to 1, "title" to "From the bus stop"), "New task")
        val temp = Optimistic.tempId(create.id)
        assertEquals("From the bus stop", shown(temp).s("title"))
        val label = ledger.enqueue("task.label.add", obj("task_id" to Refs.ref(create.id), "label" to "phone"), "Label")
        assertTrue(shown(temp).toString().contains("phone"))

        // A snapshot from the PC does not know the temp row yet; it stays.
        ledger.replace(Kind.Task, Scope(projectId = 1), emptyList())
        assertEquals("From the bus stop", shown(temp).s("title"))

        outbox.update(create.copy(state = OutboxEntry.State.Done, result = task(42, "From the bus stop")))
        ledger.applied(outbox.get(create.id)!!, task(42, "From the bus stop"))
        assertNull(replica.get(Kind.Task, temp))
        // The label edit is still pending and now lies over the real row.
        assertTrue(shown("42").toString().contains("phone"))
        assertTrue(replica.get(Kind.Task, "42")!!.pending)
        assertEquals(OutboxEntry.State.Pending, outbox.get(label.id)!!.state)
    }

    @Test
    fun `dropping a refused edit brings back what the PC said`() = runBlocking {
        ledger.upsert(listOf(Row.of(Kind.Task, task(7, "old"))!!))
        val e = ledger.enqueue("task.update", obj("task_id" to 7, "title" to "mine"), "Rename")
        outbox.update(e.copy(state = OutboxEntry.State.Conflict, error = BusError("conflict", "task.edit_conflict", "changed")))
        ledger.discard(e.id)
        assertEquals("old", shown("7").s("title"))
        assertFalse(replica.get(Kind.Task, "7")!!.pending)
    }

    @Test
    fun `retrying a conflict sends the phone's version without its check, under a new id`() = runBlocking {
        ledger.upsert(listOf(Row.of(Kind.Task, task(7, "old"))!!))
        val e = ledger.enqueue("task.update", obj("task_id" to 7, "title" to "mine", "expected" to obj("title" to "old")), "Rename")
        outbox.update(e.copy(state = OutboxEntry.State.Conflict))
        val again = ledger.retry(e.id, overwrite = true)!!
        assertTrue(again.id != e.id)
        assertFalse(again.payload.containsKey("expected"))
        assertEquals(OutboxEntry.State.Pending, again.state)
    }

    @Test
    fun `a delete hides the row until the PC confirms it`() = runBlocking {
        ledger.upsert(listOf(Row.of(Kind.Task, task(7, "old"))!!))
        ledger.enqueue("task.delete", obj("task_id" to 7), "Delete #7")
        assertTrue(shown("7").isHidden())
        ledger.replace(Kind.Task, Scope(projectId = 1), listOf(Row.of(Kind.Task, task(7, "old"))!!))
        assertTrue(shown("7").isHidden())
    }

    @Test
    fun `a delete and a restore made offline leave the row showing`() = runBlocking {
        ledger.upsert(listOf(Row.of(Kind.Task, task(7, "old"))!!))
        ledger.enqueue("task.delete", obj("task_id" to 7), "Delete #7")
        assertTrue(shown("7").isHidden())
        ledger.enqueue("task.restore", obj("task_id" to 7), "Restore #7")
        assertFalse(shown("7").isHidden())
        // And the same after the PC rewrites the row underneath both.
        ledger.replace(Kind.Task, Scope(projectId = 1), listOf(Row.of(Kind.Task, task(7, "old"))!!))
        assertFalse(shown("7").isHidden())
    }

    @Test
    fun `relations made offline show at once`() = runBlocking {
        ledger.upsert(listOf(Row.of(Kind.Task, task(7, "old"))!!))
        ledger.enqueue("task.relate", obj("task_id" to 7, "relation" to "blocked_by", "other_id" to 3), "Blocked by #3")
        assertEquals("[3]", shown("7")["blocked_by"].toString())
        ledger.enqueue("task.unrelate", obj("task_id" to 7, "relation" to "blocked_by", "other_id" to 3), "Unblock")
        assertEquals("[]", shown("7")["blocked_by"].toString())
    }

    @Test
    fun `references resolve from earlier answers and wait while one is missing`() {
        val payload = obj("task_id" to Refs.ref("e1"), "nested" to obj("x" to Refs.ref("e2", "message.id")))
        assertEquals(setOf("e1", "e2"), Refs.of(payload))
        assertNull(Refs.resolve(payload) { if (it == "e1") obj("id" to 5) else null })
        val done = Refs.resolve(payload) { if (it == "e1") obj("id" to 5) else obj("message" to obj("id" to 9)) }!!
        assertEquals("5", done["task_id"].toString())
        assertEquals("9", (done["nested"] as JsonObject)["x"].toString())
    }
}
