package com.quietsoftware.relay.device

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import expo.modules.kotlin.exception.CodedException

/**
 * The "Ask in Relay" entry in the text-selection menu.
 *
 * The config plugin declares an activity-alias named [ALIAS] that points at the app's main
 * activity and accepts ACTION_PROCESS_TEXT. So the selected text arrives in the main activity's
 * own intent: as the launch intent on a cold start, through onNewIntent otherwise. There is no
 * trampoline activity and nothing in MainActivity to patch.
 *
 * The text is held here until JavaScript consumes it once, and removed from the intent, so a
 * resume, a JS reload or a relaunch from Recents does not deliver it again.
 */
internal class ProcessText(private val context: Context) {
  private val alias = ComponentName(context.packageName, ALIAS)
  private var pending: String? = null

  /** Takes the text out of [intent] if it carries one. Returns true when text was taken. */
  @Synchronized
  fun capture(intent: Intent): Boolean {
    if (intent.action != Intent.ACTION_PROCESS_TEXT) return false
    // Android replays the original launch intent when the task is reopened from Recents
    if ((intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY) != 0) return false
    val text = intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString() ?: return false
    intent.removeExtra(Intent.EXTRA_PROCESS_TEXT)
    intent.action = Intent.ACTION_MAIN
    if (text.isBlank()) return false
    pending = text
    return true
  }

  /** Returns the pending text once, then null until new text arrives. */
  @Synchronized
  fun consume(): String? = pending.also { pending = null }

  fun isEnabled(): Boolean {
    val pm = context.packageManager
    return when (pm.getComponentEnabledSetting(alias)) {
      PackageManager.COMPONENT_ENABLED_STATE_ENABLED -> true
      PackageManager.COMPONENT_ENABLED_STATE_DEFAULT -> manifestDefault() ?: false
      else -> false
    }
  }

  fun setEnabled(enabled: Boolean): Boolean {
    if (manifestDefault() == null) {
      throw CodedException(
        "ERR_NOT_CONFIGURED",
        "No $ALIAS in the manifest; add the relay-device config plugin and rebuild",
        null
      )
    }
    val state = if (enabled) {
      PackageManager.COMPONENT_ENABLED_STATE_ENABLED
    } else {
      PackageManager.COMPONENT_ENABLED_STATE_DISABLED
    }
    context.packageManager.setComponentEnabledSetting(alias, state, PackageManager.DONT_KILL_APP)
    return isEnabled()
  }

  /** The alias's `android:enabled` from the manifest, or null when the alias is missing. */
  @Suppress("DEPRECATION")
  private fun manifestDefault(): Boolean? = try {
    context.packageManager.getActivityInfo(alias, PackageManager.MATCH_DISABLED_COMPONENTS).enabled
  } catch (e: PackageManager.NameNotFoundException) {
    null
  }

  companion object {
    /** Must match ALIAS_NAME in the module's app.plugin.js. */
    const val ALIAS = "com.quietsoftware.relay.device.ProcessTextAlias"
  }
}
