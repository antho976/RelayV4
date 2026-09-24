package com.quietsoftware.relay.device

import android.content.ContentValues
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import androidx.annotation.RequiresApi
import expo.modules.kotlin.exception.CodedException
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream

/** Writing into the shared Downloads folder, and re-encoding images as PNG. */
internal class Downloads(private val context: Context) {

  /**
   * Copies the file at [sourcePath] into Downloads as [fileName] (the source's own name when
   * blank). An existing file is never overwritten: the copy becomes `name (1).ext` and so on.
   * Returns the new item's URI and the name it was actually saved under.
   */
  fun save(sourcePath: String, fileName: String, mimeType: String): Map<String, Any?> {
    val source = File(sourcePath.removePrefix("file://"))
    if (!source.isFile) throw CodedException("ERR_NOT_FOUND", "No file at ${source.path}", null)
    val name = fileName.ifBlank { source.name }
    val mime = mimeType.ifBlank { guessMime(name) }

    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      saveWithMediaStore(source, name, mime)
    } else {
      saveToPublicDirectory(source, name, mime)
    }
  }

  @RequiresApi(Build.VERSION_CODES.Q)
  private fun saveWithMediaStore(source: File, name: String, mime: String): Map<String, Any?> {
    val resolver = context.contentResolver
    val collection = MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)

    // MediaProvider de-duplicates names on insert on most releases; on the ones that refuse a
    // clash instead, try the next candidate ourselves.
    var item: Uri? = null
    var lastError: Exception? = null
    for (candidate in candidateNames(name).take(MAX_ATTEMPTS)) {
      val values = ContentValues().apply {
        put(MediaStore.MediaColumns.DISPLAY_NAME, candidate)
        put(MediaStore.MediaColumns.MIME_TYPE, mime)
        put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
        put(MediaStore.MediaColumns.IS_PENDING, 1)
      }
      try {
        item = resolver.insert(collection, values)
        if (item != null) break
      } catch (e: IllegalStateException) {
        lastError = e
      }
    }
    val uri = item ?: throw CodedException("ERR_SAVE_FAILED", "MediaStore refused the new file", lastError)

    try {
      val out = resolver.openOutputStream(uri, "w")
        ?: throw CodedException("ERR_SAVE_FAILED", "Cannot write to $uri", null)
      out.use { o -> source.inputStream().use { it.copyTo(o, BUFFER) } }
      resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
    } catch (e: Exception) {
      runCatching { resolver.delete(uri, null, null) }
      if (e is CodedException) throw e
      throw CodedException("ERR_SAVE_FAILED", "Saving to Downloads failed: ${e.message}", e)
    }

    val savedName = runCatching {
      resolver.query(uri, arrayOf(MediaStore.MediaColumns.DISPLAY_NAME), null, null, null)
        ?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
    }.getOrNull() ?: name
    return mapOf("uri" to uri.toString(), "name" to savedName)
  }

  /** Android 9 and below: a plain file in the public folder, which needs WRITE_EXTERNAL_STORAGE. */
  @Suppress("DEPRECATION")
  private fun saveToPublicDirectory(source: File, name: String, mime: String): Map<String, Any?> {
    val dir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
    if (!dir.isDirectory && !dir.mkdirs()) {
      throw CodedException("ERR_SAVE_FAILED", "Cannot create ${dir.path}", null)
    }
    val target = candidateNames(name).map { File(dir, it) }.take(MAX_ATTEMPTS).firstOrNull { !it.exists() }
      ?: throw CodedException("ERR_SAVE_FAILED", "No free name for $name", null)
    try {
      source.inputStream().use { input -> FileOutputStream(target).use { input.copyTo(it, BUFFER) } }
    } catch (e: SecurityException) {
      target.delete()
      throw CodedException("ERR_PERMISSION", "Storage permission is required below Android 10", e)
    } catch (e: Exception) {
      target.delete()
      throw CodedException("ERR_SAVE_FAILED", "Saving to Downloads failed: ${e.message}", e)
    }
    MediaScannerConnection.scanFile(context, arrayOf(target.path), arrayOf(mime), null)
    return mapOf("uri" to Uri.fromFile(target).toString(), "name" to target.name)
  }

  /**
   * Decodes any image Android can read (JPEG, WebP, GIF, HEIF, PNG ...) and writes it to
   * [destPath] as PNG. Returns the width and height.
   */
  fun convertToPng(source: String, destPath: String): Map<String, Any?> {
    val bitmap = openImage(source).use { BitmapFactory.decodeStream(it) }
      ?: throw CodedException("ERR_DECODE_FAILED", "Cannot decode an image from $source", null)
    val dest = File(destPath.removePrefix("file://"))
    try {
      dest.parentFile?.mkdirs()
      val ok = FileOutputStream(dest).use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
      if (!ok) throw CodedException("ERR_ENCODE_FAILED", "PNG encoding failed", null)
      return mapOf("path" to dest.path, "width" to bitmap.width, "height" to bitmap.height)
    } catch (e: Exception) {
      dest.delete()
      if (e is CodedException) throw e
      throw CodedException("ERR_ENCODE_FAILED", "Writing ${dest.path} failed: ${e.message}", e)
    } finally {
      bitmap.recycle()
    }
  }

  private fun openImage(source: String): InputStream =
    if (source.startsWith("content://")) {
      context.contentResolver.openInputStream(Uri.parse(source))
        ?: throw CodedException("ERR_NOT_FOUND", "The provider returned nothing for $source", null)
    } else {
      File(if (source.startsWith("file://")) Uri.parse(source).path ?: "" else source).inputStream()
    }

  private fun guessMime(name: String): String =
    MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase())
      ?: "application/octet-stream"

  /** `a.png`, `a (1).png`, `a (2).png` ... */
  private fun candidateNames(name: String): Sequence<String> {
    val dot = name.lastIndexOf('.')
    val stem = if (dot > 0) name.substring(0, dot) else name
    val ext = if (dot > 0) name.substring(dot) else ""
    return sequenceOf(name) + generateSequence(1) { it + 1 }.map { "$stem ($it)$ext" }
  }

  companion object {
    private const val BUFFER = 1 shl 20
    private const val MAX_ATTEMPTS = 1000
  }
}
