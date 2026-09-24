package com.quietsoftware.relay.device

import android.os.Build
import java.io.File

/**
 * CPU facts for choosing llama.cpp settings. /proc/cpuinfo is read once per process; the
 * answer cannot change while the app runs.
 */
internal object CpuInfo {
  private val cached: Map<String, Any?> by lazy { compute() }

  fun read(): Map<String, Any?> = cached

  private fun compute(): Map<String, Any?> {
    val features = featureFlags()
    fun has(vararg names: String) = names.any { it in features }
    return mapOf(
      "abi" to (Build.SUPPORTED_ABIS.firstOrNull() ?: ""),
      "availableThreads" to Runtime.getRuntime().availableProcessors(),
      "possibleCores" to possibleCores(),
      "features" to features.sorted(),
      // Linux names the arm64 flags after the kernel hwcaps: asimddp is the dot product
      // extension, fphp/asimdhp are half-precision float. "dotprod"/"fp16" cover other kernels.
      "dotprod" to has("asimddp", "dotprod"),
      "i8mm" to has("i8mm"),
      "fp16" to has("fphp", "asimdhp", "fp16"),
      "sve" to has("sve"),
      "sve2" to has("sve2"),
    )
  }

  /** The first `Features` line; every core lists the same flags on shipping SoCs. */
  private fun featureFlags(): Set<String> = runCatching {
    File("/proc/cpuinfo").useLines { lines ->
      lines.firstOrNull { it.startsWith("Features") }
        ?.substringAfter(':')
        ?.trim()
        ?.split(Regex("\\s+"))
        ?.filter { it.isNotEmpty() }
        ?.toSet()
    }
  }.getOrNull() ?: emptySet()

  /** Cores the kernel knows about, including ones currently offline ("0-7" -> 8). */
  private fun possibleCores(): Int = runCatching {
    File("/sys/devices/system/cpu/possible").readText().trim().split(',').sumOf { range ->
      val bounds = range.split('-')
      if (bounds.size == 2) bounds[1].toInt() - bounds[0].toInt() + 1 else 1
    }
  }.getOrDefault(Runtime.getRuntime().availableProcessors())
}
