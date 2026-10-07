package com.tally.app.data.sync

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.tally.app.data.FixedClock
import com.tally.app.data.RoomTestDb
import com.tally.app.data.addAccount
import com.tally.app.data.addExpense
import com.tally.app.data.db.TallyDatabase
import com.tally.app.data.prefs.SettingsRepository
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.put
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.time.LocalDate
import java.util.concurrent.CopyOnWriteArrayList

/**
 * Tally against a fake Relay door: the greeting, the pairing code once, a proof every time after,
 * then one money.sync request per connection, end to end over a real WebSocket.
 */
@RunWith(RobolectricTestRunner::class)
class PcSyncTest {

    private val context: Context = ApplicationProvider.getApplicationContext()
    private val day = LocalDate.of(2026, 10, 5)
    private lateinit var db: TallyDatabase
    private lateinit var store: SyncStore
    private lateinit var settings: SettingsRepository
    private lateinit var sync: PcSync
    private val door = FakeDoor()

    @Before fun open() = runBlocking {
        db = RoomTestDb.create()
        store = SyncStore(context)
        store.forget()
        settings = SettingsRepository(context)
        sync = PcSync(store, LedgerSync(db), settings, RelayClient())
        door.server.start()
    }

    @After fun close() {
        db.close()
        door.server.shutdown()
    }

    @Test fun pairingRunsAFirstSyncThatReplacesThePcsLedger() = runBlocking {
        val account = db.addAccount("Chequing")
        db.addExpense(42_50, day, account, note = "Metro")
        door.connection()

        val outcome = sync.pair(PairLink.manual(door.address, "ABCD-EFGH")!!)

        assertTrue("$outcome", outcome is SyncOutcome.Done)
        val prefs = store.current()
        assertEquals("antho desktop", prefs.pc!!.name)
        assertEquals("dev-1", prefs.pc!!.deviceId)
        assertEquals("tok-1", prefs.pc!!.token)
        assertTrue(prefs.replaced)
        assertEquals(7L, prefs.cursor)
        assertNull(prefs.lastError)

        val sent = door.syncs.single()
        assertTrue("The first sync replaces the PC's ledger", sent["replace"]!!.jsonPrimitive.boolean)
        assertEquals(0L, sent["since"]!!.jsonPrimitive.long)
        val tables = sent["changes"]!!.jsonArray.map { it.jsonObject["table"]!!.jsonPrimitive.content }
        assertEquals("Settings first, then the ledger in the PC's order", listOf("settings", "settings", "settings", "accounts", "transactions"), tables)
    }

    @Test fun laterSyncsProveTheKeySendWhatChangedAndTakeThePcsChanges() = runBlocking {
        val account = db.addAccount("Chequing")
        door.connection()
        sync.pair(PairLink.manual(door.address, "ABCD-EFGH")!!)
        val accountUid = db.accounts().get(account)!!.uid
        Thread.sleep(5)
        db.addExpense(4_00, day, account, note = "Coffee")
        // The PC logged an entry of its own, and moved the month to start on the 15th.
        door.answer = { _ ->
            buildJsonObject {
                put("cursor", 12)
                put("changes", buildJsonArray {
                    add(Change("transactions", "pc-1", System.currentTimeMillis() + 60_000, row = buildJsonObject {
                        put("type", "EXPENSE"); put("amount", 9_00); put("date", "2026-10-06"); put("account", accountUid)
                        put("toAccount", JsonNull); put("category", JsonNull); put("note", "Lunch"); put("recurring", JsonNull); put("createdAt", 1)
                    }).toJson())
                    add(Change("settings", "month_start_day", System.currentTimeMillis() + 60_000, row = buildJsonObject { put("value", "15") }).toJson())
                })
                put("replaced", false)
            }
        }
        door.connection()

        val outcome = sync.syncNow()

        assertTrue("$outcome", outcome is SyncOutcome.Done)
        val sent = door.syncs.last()
        assertFalse(sent["replace"]!!.jsonPrimitive.boolean)
        assertEquals("The PC's cursor goes back as since", 7L, sent["since"]!!.jsonPrimitive.long)
        val notes = sent["changes"]!!.jsonArray.map { it.jsonObject["row"]!!.jsonObject["note"]?.jsonPrimitive?.content }
        assertEquals("Only the new entry goes", listOf("Coffee"), notes)
        assertTrue("The proof checked out", door.proven)

        assertEquals(12L, store.current().cursor)
        assertEquals(listOf("Coffee", "Lunch"), db.transactions().all().map { it.note }.sorted())
        assertEquals(15, settings.current().monthStartDay)
    }

    @Test fun aRevokedPhoneIsToldInWordsAndNothingIsThrown() = runBlocking {
        door.connection()
        sync.pair(PairLink.manual(door.address, "ABCD-EFGH")!!)
        door.token = "another"
        door.connection()

        val outcome = sync.syncNow()

        assertTrue(outcome is SyncOutcome.Failed)
        assertEquals("The PC no longer accepts this phone's key. Pair it again.", store.current().lastError)
    }

    @Test fun aWrongCodePairsNothing() = runBlocking {
        door.connection()
        val outcome = sync.pair(PairLink.manual(door.address, "WRONG-CODE")!!)
        assertTrue(outcome is SyncOutcome.Failed)
        assertFalse(store.current().paired)
    }

    @Test fun anUnreachablePcFailsQuietly() = runBlocking {
        door.connection()
        sync.pair(PairLink.manual(door.address, "ABCD-EFGH")!!)
        door.server.shutdown()
        val outcome = sync.syncNow()
        assertTrue(outcome is SyncOutcome.Failed)
        assertEquals("The PC could not be reached", store.current().lastError)
    }
}

/** Relay's phone door as bridge.rs speaks it, one scripted connection per [connection]. */
private class FakeDoor {
    val server = MockWebServer()
    @Volatile var token = "tok-1"
    @Volatile var proven = false
    val syncs = CopyOnWriteArrayList<JsonObject>()
    @Volatile var answer: (JsonObject) -> JsonObject = { payload ->
        buildJsonObject {
            put("cursor", 7)
            put("changes", JsonArray(emptyList()))
            put("replaced", payload["replace"]?.jsonPrimitive?.boolean ?: false)
            put("applied", payload["changes"]!!.jsonArray.size)
            put("skipped", 0)
        }
    }

    val address: String get() = "${server.hostName}:${server.port}"

    fun connection() {
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            val challenge = "challenge-${System.nanoTime()}"
            var admitted = false

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
            }

            override fun onOpen(webSocket: WebSocket, response: Response) {
                webSocket.send("""{"v":1,"relay":"remote","host":"antho desktop","host_id":"h1","instance":"dev","version":"0.4","challenge":"$challenge"}""")
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                val o = RelayWire.parse(text)!!
                if (!admitted) {
                    val pair = o["pair"]?.jsonPrimitive?.content
                    val proof = o["proof"]?.jsonPrimitive?.content
                    val provenNow = pair == null && proof == RelayWire.proof(challenge, token) && o["device"]?.jsonPrimitive?.content == "dev-1"
                    when {
                        pair == "ABCD-EFGH" -> webSocket.send("""{"v":1,"ok":true,"device":"dev-1","token":"$token"}""")
                        pair != null -> webSocket.send("""{"v":1,"ok":false,"error":"pair.invalid"}""")
                        provenNow -> {
                            proven = true
                            webSocket.send("""{"v":1,"ok":true,"device":"dev-1"}""")
                        }
                        else -> webSocket.send("""{"v":1,"ok":false,"error":"auth.bad_proof"}""")
                    }
                    admitted = pair == "ABCD-EFGH" || provenNow
                    return
                }
                val id = o["id"]!!.jsonPrimitive.content
                if (o["op"]!!.jsonPrimitive.content != "money.sync" || o["actor"]!!.jsonPrimitive.content != "user") {
                    webSocket.send("""{"v":1,"id":"$id","ok":false,"error":{"code":"remote.op","message":"refused"}}""")
                    return
                }
                val payload = o["payload"]!!.jsonObject
                syncs += payload
                // An event first: the client must wait for its own answer.
                webSocket.send("""{"v":1,"ev":"money.changed","ts":"2026-10-07T12:00:00Z","actor":"user","payload":{}}""")
                webSocket.send(buildJsonObject { put("v", 1); put("id", id); put("ok", true); put("result", answer(payload)) }.toString())
            }
        }))
    }
}
