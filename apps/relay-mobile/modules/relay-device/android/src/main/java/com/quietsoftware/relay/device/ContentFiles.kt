package com.quietsoftware.relay.device

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.provider.OpenableColumns
import expo.modules.kotlin.exception.CodedException
import java.io.File
import java.io.FileInputStream
import java.io.FileNotFoundException
import java.io.FileOutputStream
import java.io.InputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

/**
 * File descriptors and copies for `content://` URIs (Storage Access Framework picks).
 *
 * Every descriptor handed to JavaScript stays owned by this class until it is closed, so a
 * forgotten one can still be found (`openFds`) and closed (`closeAll`, also run on teardown).
 */
internal class ContentFiles(private val context: Context) {
  private val descriptors = ConcurrentHashMap<Int, ParcelFileDescriptor>()
  private val copies = ConcurrentHashMap<String, AtomicBoolean>()

  // region descriptors

  fun open(uri: String): Map<String, Any?> {
    val parsed = parseContentUri(uri)
    val pfd = try {
      context.contentResolver.openFileDescriptor(parsed, "r")
    } catch (e: SecurityException) {
      throw CodedException("ERR_PERMISSION", "No read permission for $uri", e)
    } catch (e: FileNotFoundException) {
      throw CodedException("ERR_NOT_FOUND", "Cannot open $uri: ${e.message}", e)
    } ?: throw CodedException("ERR_NOT_FOUND", "The provider returned nothing for $uri", null)

    val fd = pfd.fd
    descriptors[fd] = pfd
    return mapOf(
      "fd" to fd,
      "path" to "/proc/self/fd/$fd",
      "size" to pfd.statSize.toDouble(),
    )
  }

  /** Returns false when the descriptor was not opened here or is already closed. */
  fun close(fd: Int): Boolean {
    val pfd = descriptors.remove(fd) ?: return false
    runCatching { pfd.close() }
    return true
  }

  fun closeAll(): Int {
    var closed = 0
    for (fd in descriptors.keys.toList()) if (close(fd)) closed++
    return closed
  }

  fun openFds(): List<Int> = descriptors.keys.sorted()

  // endregion

  // region permissions

  /**
   * Keeps access to a picked document across restarts. Asks for read and write, and settles
   * for read when the grant has no write. Returns whether write was kept.
   */
  fun persistPermission(uri: String): Boolean {
    val parsed = parseContentUri(uri)
    val resolver = context.contentResolver
    return try {
      resolver.takePersistableUriPermission(
        parsed,
        Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
      )
      true
    } catch (readWriteRefused: SecurityException) {
      try {
        resolver.takePersistableUriPermission(parsed, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        false
      } catch (e: SecurityException) {
        throw CodedException("ERR_PERMISSION", "No persistable grant for $uri", e)
      }
    }
  }

  /** Gives a persisted grant back; the system caps how many an app may hold. */
  fun releasePermission(uri: String): Boolean {
    val parsed = parseContentUri(uri)
    val held = context.contentResolver.persistedUriPermissions.firstOrNull { it.uri == parsed }
      ?: return false
    var flags = 0
    if (held.isReadPermission()) flags = flags or Intent.FLAG_GRANT_READ_URI_PERMISSION
    if (held.isWritePermission()) flags = flags or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
    context.contentResolver.releasePersistableUriPermission(parsed, flags)
    return true
  }

  // endregion

  // region copy

  /**
   * Copies [source] (content URI, file URI or path) to [destPath] in 1 MiB blocks.
   *
   * The data goes to a `.partial` file that is renamed into place only when complete, so a
   * failed or cancelled copy never leaves something that looks like a finished file.
   */
  fun copyToFile(
    source: String,
    destPath: String,
    taskId: String,
    onProgress: (Map<String, Any?>) -> Unit
  ): Map<String, Any?> {
    val cancelled = AtomicBoolean(false)
    val existing = copies.putIfAbsent(taskId, cancelled)
    if (existing != null) {
      // cancelCopy can arrive before this task starts; it leaves a raised flag behind
      if (existing.get()) {
        copies.remove(taskId)
        throw CodedException("ERR_COPY_CANCELLED", "Copy $taskId was cancelled", null)
      }
      throw CodedException("ERR_COPY_BUSY", "A copy with id $taskId is already running", null)
    }

    val dest = File(destPath.removePrefix("file://"))
    val partial = File(dest.path + ".partial")
    try {
      dest.parentFile?.mkdirs()
      val (input, knownSize) = openSource(source)
      val total = if (knownSize >= 0) knownSize else querySize(source)

      val free = dest.parentFile?.usableSpace ?: Long.MAX_VALUE
      if (total > 0 && total > free) {
        input.close()
        throw CodedException("ERR_NO_SPACE", "Needs $total bytes, $free free", null)
      }

      val started = SystemClock.elapsedRealtime()
      var lastReport = 0L
      var copied = 0L
      fun report() {
        val seconds = (SystemClock.elapsedRealtime() - started) / 1000.0
        onProgress(
          mapOf(
            "taskId" to taskId,
            "bytesCopied" to copied.toDouble(),
            "totalBytes" to total.toDouble(),
            "bytesPerSecond" to (if (seconds > 0) copied / seconds else 0.0),
          )
        )
      }

      input.use { inp ->
        FileOutputStream(partial).use { out ->
          val buffer = ByteArray(BLOCK)
          while (true) {
            if (cancelled.get()) throw CodedException("ERR_COPY_CANCELLED", "Copy $taskId was cancelled", null)
            val read = inp.read(buffer)
            if (read < 0) break
            out.write(buffer, 0, read)
            copied += read
            val now = SystemClock.elapsedRealtime()
            if (now - lastReport >= REPORT_INTERVAL_MS) {
              lastReport = now
              report()
            }
          }
          out.fd.sync()
        }
      }

      if (dest.exists() && !dest.delete()) {
        throw CodedException("ERR_COPY_FAILED", "Cannot replace existing ${dest.path}", null)
      }
      if (!partial.renameTo(dest)) {
        throw CodedException("ERR_COPY_FAILED", "Cannot move the copy into ${dest.path}", null)
      }
      report()
      return mapOf("path" to dest.path, "bytesCopied" to copied.toDouble())
    } catch (e: CodedException) {
      partial.delete()
      throw e
    } catch (e: SecurityException) {
      partial.delete()
      throw CodedException("ERR_PERMISSION", "No read permission for $source", e)
    } catch (e: Exception) {
      partial.delete()
      throw CodedException("ERR_COPY_FAILED", "Copy failed: ${e.message}", e)
    } finally {
      copies.remove(taskId, cancelled)
    }
  }

  /** Cancels a running copy, or one whose start is still queued. */
  fun cancelCopy(taskId: String) {
    copies.putIfAbsent(taskId, AtomicBoolean(true))?.set(true)
  }

  fun cancelAllCopies() {
    copies.values.forEach { it.set(true) }
  }

  private fun openSource(source: String): Pair<InputStream, Long> {
    if (source.startsWith("content://")) {
      val pfd = context.contentResolver.openFileDescriptor(Uri.parse(source), "r")
        ?: throw FileNotFoundException("The provider returned nothing for $source")
      return ParcelFileDescriptor.AutoCloseInputStream(pfd) to pfd.statSize
    }
    val file = File(if (source.startsWith("file://")) Uri.parse(source).path ?: "" else source)
    return FileInputStream(file) to file.length()
  }

  private fun querySize(source: String): Long {
    if (!source.startsWith("content://")) return -1
    return runCatching {
      context.contentResolver
        .query(Uri.parse(source), arrayOf(OpenableColumns.SIZE), null, null, null)
        ?.use { c -> if (c.moveToFirst() && !c.isNull(0)) c.getLong(0) else -1L }
        ?: -1L
    }.getOrDefault(-1L)
  }

  // endregion

  private fun parseContentUri(uri: String): Uri {
    if (!uri.startsWith("content://")) {
      throw CodedException("ERR_NOT_CONTENT_URI", "Expected a content:// URI, got $uri", null)
    }
    return Uri.parse(uri)
  }

  companion object {
    private const val BLOCK = 1 shl 20
    private const val REPORT_INTERVAL_MS = 200L
  }
}
