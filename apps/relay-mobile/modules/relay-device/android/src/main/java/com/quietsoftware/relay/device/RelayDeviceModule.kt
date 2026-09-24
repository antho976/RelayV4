package com.quietsoftware.relay.device

import android.content.Context
import expo.modules.kotlin.exception.Exceptions
import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Device services for Relay: content-URI files, the public Downloads folder, CPU facts and the
 * "process text" selection-menu entry. Each concern lives in its own class; this file only maps
 * them onto the JavaScript API.
 */
class RelayDeviceModule : Module() {
  private val context: Context
    get() = appContext.reactContext?.applicationContext ?: throw Exceptions.ReactContextLost()

  private val contentFiles by lazy { ContentFiles(context) }
  private val downloads by lazy { Downloads(context) }
  private val processText by lazy { ProcessText(context) }

  override fun definition() = ModuleDefinition {
    Name("RelayDevice")

    Events(COPY_PROGRESS, PROCESS_TEXT)

    OnDestroy {
      // a reload or teardown must not strand descriptors or leave copies running
      contentFiles.closeAll()
      contentFiles.cancelAllCopies()
    }

    // region content files

    AsyncFunction("openContentFd") { uri: String -> contentFiles.open(uri) }

    Function("closeContentFd") { fd: Int -> contentFiles.close(fd) }

    Function("closeAllContentFds") { contentFiles.closeAll() }

    Function("openContentFds") { contentFiles.openFds() }

    AsyncFunction("persistContentPermission") { uri: String -> contentFiles.persistPermission(uri) }

    AsyncFunction("releaseContentPermission") { uri: String -> contentFiles.releasePermission(uri) }

    AsyncFunction("copyContentToFile") Coroutine { source: String, destPath: String, taskId: String ->
      withContext(Dispatchers.IO) {
        contentFiles.copyToFile(source, destPath, taskId) { progress ->
          sendEvent(COPY_PROGRESS, progress)
        }
      }
    }

    Function("cancelCopy") { taskId: String -> contentFiles.cancelCopy(taskId) }

    // endregion

    // region downloads and images

    AsyncFunction("saveToDownloads") Coroutine { sourcePath: String, fileName: String, mimeType: String ->
      withContext(Dispatchers.IO) { downloads.save(sourcePath, fileName, mimeType) }
    }

    AsyncFunction("convertImageToPng") Coroutine { source: String, destPath: String ->
      withContext(Dispatchers.IO) { downloads.convertToPng(source, destPath) }
    }

    // endregion

    // region cpu

    Function("availableThreads") { Runtime.getRuntime().availableProcessors() }

    Function("cpuInfo") { CpuInfo.read() }

    // endregion

    // region process text

    AsyncFunction("isProcessTextEnabled") { processText.isEnabled() }

    AsyncFunction("setProcessTextEnabled") { enabled: Boolean -> processText.setEnabled(enabled) }

    Function("consumeProcessText") {
      appContext.currentActivity?.intent?.let { processText.capture(it) }
      processText.consume()
    }

    OnNewIntent { intent ->
      if (processText.capture(intent)) sendEvent(PROCESS_TEXT, mapOf<String, Any?>())
    }

    OnActivityEntersForeground {
      // covers a cold start, where the text arrives in the launch intent rather than onNewIntent
      val intent = appContext.currentActivity?.intent
      if (intent != null && processText.capture(intent)) sendEvent(PROCESS_TEXT, mapOf<String, Any?>())
    }

    // endregion
  }

  companion object {
    const val COPY_PROGRESS = "onCopyProgress"
    const val PROCESS_TEXT = "onProcessText"
  }
}
