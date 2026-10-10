package com.quietsoftware.relay.ui.code

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/*
 * The PC's answers the code screens read (bus.v1.json). Decoded leniently: an engine's extra
 * fields are ignored, and every field a screen can do without has a default.
 */

@Serializable
data class Worktree(
    val path: String = "",
    val branch: String = "",
    val head: String = "",
    val session: String? = null,
    val dirty: Boolean = false,
)

@Serializable
data class WorktreeList(val worktrees: List<Worktree> = emptyList())

/** A `file.tree` entry, or the result of `file.create`, `file.rename` and `file.restore`. */
@Serializable
data class TreeEntry(
    val path: String = "",
    val name: String = "",
    val kind: String = "file",
    val size: Long? = null,
    val badge: String? = null,
)

@Serializable
data class TreeOut(val entries: List<TreeEntry> = emptyList())

@Serializable
data class FileRead(
    val text: String? = null,
    @SerialName("bytes_b64") val bytesB64: String? = null,
    val mime: String = "",
    val size: Long = 0,
    val truncated: Boolean = false,
)

@Serializable
data class WriteOut(
    @SerialName("added_lines") val added: Int = 0,
    @SerialName("removed_lines") val removed: Int = 0,
)

@Serializable
data class TrashOut(@SerialName("trash_id") val trashId: Long = 0)

@Serializable
data class Hit(val path: String = "", val line: Int = 0, val text: String = "")

@Serializable
data class SearchOut(val hits: List<Hit> = emptyList())

@Serializable
data class FileStatus(
    val path: String = "",
    val index: String = " ",
    val worktree: String = " ",
    @SerialName("renamed_from") val renamedFrom: String? = null,
)

@Serializable
data class GitStatus(
    val branch: String = "",
    val upstream: String? = null,
    val ahead: Int? = null,
    val behind: Int? = null,
    val files: List<FileStatus> = emptyList(),
)

@Serializable
data class DiffFile(
    val path: String = "",
    val status: String = "",
    val added: Int = 0,
    val removed: Int = 0,
    val binary: Boolean = false,
)

@Serializable
data class DiffFiles(val files: List<DiffFile> = emptyList())

@Serializable
data class Hunk(
    @SerialName("old_start") val oldStart: Int = 0,
    @SerialName("new_start") val newStart: Int = 0,
    val text: String = "",
)

@Serializable
data class FileDiff(val hunks: List<Hunk> = emptyList())

@Serializable
data class Suggestion(val message: String = "")

@Serializable
data class CommitOut(val sha: String = "")

@Serializable
data class PrOpened(val url: String = "")

@Serializable
data class PullRequest(
    val number: Int = 0,
    val branch: String = "",
    val draft: Boolean = false,
    val url: String = "",
    val title: String = "",
    val state: String = "",
    @SerialName("same_repository") val sameRepository: Boolean = false,
)

@Serializable
data class PrList(@SerialName("pull_requests") val pullRequests: List<PullRequest> = emptyList())

@Serializable
data class Commit(
    val sha: String = "",
    val author: String = "",
    val at: String = "",
    val subject: String = "",
    val body: String = "",
    val refs: List<String> = emptyList(),
)

@Serializable
data class CommitLog(val commits: List<Commit> = emptyList())

@Serializable
data class CommitShow(val commit: Commit? = null, val files: List<DiffFile> = emptyList())

@Serializable
data class Branch(
    val name: String = "",
    val upstream: String? = null,
    val ahead: Int? = null,
    val behind: Int? = null,
    val merged: Boolean = false,
    val session: String? = null,
    val current: Boolean = false,
)

@Serializable
data class Branches(val current: String = "", val branches: List<Branch> = emptyList())

@Serializable
data class FetchOut(val ahead: Int? = null, val behind: Int? = null)
