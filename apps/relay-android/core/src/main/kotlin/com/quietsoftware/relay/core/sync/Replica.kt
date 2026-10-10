package com.quietsoftware.relay.core.sync

import com.quietsoftware.relay.core.wire.l
import com.quietsoftware.relay.core.wire.s
import kotlinx.serialization.json.JsonObject

/**
 * The phone's copy of the PC's data (docs/ANDROID.md, "One set of data"). Every entity the PC
 * owns is kept here as the JSON the bus returned, keyed by kind and id, so a newer engine's
 * extra fields survive and the phone can show everything while the PC is away.
 *
 * The PC is the authority for this data: it is where the agents, repositories and worktrees are.
 * The phone's own edits go through the outbox ([Outbox]) and show here at once as overlays on the
 * PC's rows; when the PC accepts one, its answer replaces the overlay.
 */
enum class Kind(val tag: String) {
    Workspace("workspace"),
    Project("project"),
    Session("session"),

    /** A stopped agent the PC offers to resume (`session.restorable`), keyed by session name. */
    Restorable("restorable"),
    Task("task"),
    Module("module"),
    Note("note"),
    Hold("hold"),
    Notification("notification"),
    Thread("thread"),

    /** A thread's message; `parent` is the thread id. */
    Message("message"),

    /** Agent mail (`mailbox.list`). */
    Mail("mail"),

    /** A project's task label, keyed by name. */
    Label("label"),
    ;

    /** The row id of an entity of this kind, from the JSON the bus returned. */
    fun idOf(row: JsonObject): String? = when (this) {
        Session -> row.s("name")
        Restorable -> (row["session"] as? JsonObject)?.s("name") ?: row.s("name")
        // Label names repeat across projects; the row id keeps them apart.
        Label -> row.s("name")?.let { name -> "${row.l("project_id") ?: ""}:$name" }
        else -> row.l("id")?.toString() ?: row.s("id")
    }

    fun projectOf(row: JsonObject): Long? = when (this) {
        Restorable -> (row["session"] as? JsonObject)?.l("project_id") ?: row.l("project_id")
        else -> row.l("project_id")
    }

    fun parentOf(row: JsonObject): String? = when (this) {
        Message -> row.l("thread_id")?.toString() ?: row.s("thread_id")
        else -> null
    }

    companion object {
        fun of(tag: String): Kind? = entries.firstOrNull { it.tag == tag }
    }
}

/**
 * One stored entity. [json] is what the phone shows: the PC's row ([base]) with the outbox's
 * pending edits laid over it. [base] is null for a row the phone created that the PC has not
 * answered yet; [pending] says the two differ.
 */
data class Row(
    val kind: Kind,
    val id: String,
    val projectId: Long?,
    val parent: String?,
    val json: JsonObject,
    val base: JsonObject? = json,
    val pending: Boolean = false,
) {
    companion object {
        fun of(kind: Kind, json: JsonObject, projectId: Long? = null, parent: String? = null): Row? {
            val id = kind.idOf(json) ?: return null
            return Row(kind, id, kind.projectOf(json) ?: projectId, kind.parentOf(json) ?: parent, json)
        }
    }
}

/** Which rows a snapshot replaces: every row of a kind, one project's, or one parent's. */
data class Scope(val projectId: Long? = null, val parent: String? = null) {
    companion object {
        val All = Scope()
    }
}

/**
 * Storage for the replica. Room on the phone, a map in tests. Writes here are the PC's truth;
 * [Replica] lays the outbox's pending edits over them before they reach this interface.
 */
interface ReplicaStore {
    suspend fun upsert(rows: List<Row>)

    suspend fun remove(kind: Kind, ids: Collection<String>)

    /** Replace every row of [kind] in [scope] with [rows]: what a list op returned is the whole truth for that scope. */
    suspend fun replace(kind: Kind, scope: Scope, rows: List<Row>)

    suspend fun get(kind: Kind, id: String): Row?

    suspend fun all(kind: Kind, scope: Scope = Scope.All): List<Row>

    /** A cached query result: anything that is not an entity (git status, a file, usage). */
    suspend fun putQuery(key: String, json: String, at: Long)

    suspend fun query(key: String): Pair<String, Long>?

    /** Forget everything: a different PC was paired. */
    suspend fun clear()
}
