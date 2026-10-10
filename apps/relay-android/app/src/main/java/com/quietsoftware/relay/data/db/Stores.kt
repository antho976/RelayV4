package com.quietsoftware.relay.data.db

import com.quietsoftware.relay.core.sync.Kind
import com.quietsoftware.relay.core.sync.OutboxEntry
import com.quietsoftware.relay.core.sync.OutboxStore
import com.quietsoftware.relay.core.sync.ReplicaStore
import com.quietsoftware.relay.core.sync.Row
import com.quietsoftware.relay.core.sync.Scope
import com.quietsoftware.relay.core.wire.BusError
import com.quietsoftware.relay.core.wire.Wire
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** The replica's [ReplicaStore] on Room. */
class RoomReplica(private val db: RelayDb) : ReplicaStore {
    private val entities = db.entities()
    private val queries = db.queries()

    override suspend fun upsert(rows: List<Row>) = entities.upsert(rows.map { it.toEntity() })

    override suspend fun remove(kind: Kind, ids: Collection<String>) {
        if (ids.isNotEmpty()) entities.remove(kind.tag, ids.toList())
    }

    override suspend fun replace(kind: Kind, scope: Scope, rows: List<Row>) =
        entities.replace(kind.tag, scope.projectId, scope.parent, rows.map { it.toEntity() })

    override suspend fun get(kind: Kind, id: String): Row? = entities.get(kind.tag, id)?.toRow()

    override suspend fun all(kind: Kind, scope: Scope): List<Row> = entities.all(kind.tag, scope.projectId, scope.parent).mapNotNull { it.toRow() }

    /**
     * A row Android's 2 MB cursor window cannot read back (a large diff or file) is not kept: it
     * would fail every later read of that query. The stale answer it replaces goes too.
     */
    override suspend fun putQuery(key: String, json: String, at: Long) =
        if (json.length > MAX_CACHED_CHARS) queries.delete(key) else queries.put(QueryRow(key, json, at))

    override suspend fun query(key: String): Pair<String, Long>? = try {
        queries.get(key)?.let { it.json to it.at }
    } catch (_: android.database.sqlite.SQLiteException) {
        // Written before the cap: unreadable, so forgotten.
        queries.delete(key)
        null
    }

    override suspend fun clear() {
        entities.clear()
        queries.clear()
    }
}

/** UTF-8 can take three bytes a character; this keeps a cached answer well inside 2 MB. */
private const val MAX_CACHED_CHARS = 600_000

fun Row.toEntity() = EntityRow(kind.tag, id, projectId, parent, json.toString(), base?.toString(), pending)

fun EntityRow.toRow(): Row? {
    val kind = Kind.of(kind) ?: return null
    val json = Wire.parse(json) ?: return null
    return Row(kind, id, projectId, parent, json, base?.let { Wire.parse(it) }, pending)
}

/** The outbox's [OutboxStore] on Room. */
class RoomOutbox(private val db: RelayDb) : OutboxStore {
    private val dao = db.outbox()

    override suspend fun add(entry: OutboxEntry) = dao.put(entry.toRow())
    override suspend fun open(): List<OutboxEntry> = dao.open().mapNotNull { it.toEntry() }
    override suspend fun get(id: String): OutboxEntry? = dao.get(id)?.toEntry()
    override suspend fun update(entry: OutboxEntry) = dao.put(entry.toRow())
    override suspend fun delete(id: String) = dao.delete(id)
    override suspend fun nextSeq(): Long = dao.nextSeq()
    override suspend fun prune(before: Long) = dao.prune(before)
}

fun OutboxEntry.toRow() = OutboxRow(
    id = id,
    seq = seq,
    op = op,
    payload = payload.toString(),
    label = label,
    createdAt = createdAt,
    state = state.name,
    error = error?.let { e ->
        buildJsonObject {
            put("kind", e.kind); put("code", e.code); put("message", e.message)
            e.hint?.let { put("hint", it) }
            e.details?.let { put("details", it) }
            e.confirm?.let { put("confirm", it) }
        }.toString()
    },
    result = result?.toString(),
    attempts = attempts,
)

fun OutboxRow.toEntry(): OutboxEntry? {
    val payload = Wire.parse(payload) ?: return null
    return OutboxEntry(
        id = id,
        seq = seq,
        op = op,
        payload = payload,
        label = label,
        createdAt = createdAt,
        state = runCatching { OutboxEntry.State.valueOf(state) }.getOrDefault(OutboxEntry.State.Pending),
        error = error?.let { Wire.parse(it) }?.let { BusError.from(it) },
        result = result?.let { runCatching { Wire.json.parseToJsonElement(it) }.getOrNull() },
        attempts = attempts,
    )
}
