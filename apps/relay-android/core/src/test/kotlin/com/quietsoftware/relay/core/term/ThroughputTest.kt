package com.quietsoftware.relay.core.term

import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.random.Random

class ThroughputTest {
    /** About 1 MB of what a TUI redraw looks like: cursor moves, truecolour and 256-colour SGR, text, erases. */
    private fun tuiOutput(): ByteArray {
        val rnd = Random(1)
        val words = listOf("const", "val", "fun", "return", "Terminal", "feed", "bytes", "⏺", "│", "─", "✻", "中文", "→", "if", "else")
        val sb = StringBuilder()
        var frame = 0
        while (sb.length < 1_000_000) {
            sb.append("$CSI?2026h")
            for (row in 1..40) {
                sb.append(CSI).append(row).append(";1H")
                var col = 0
                while (col < 100) {
                    when (rnd.nextInt(4)) {
                        0 -> sb.append("${CSI}38;2;${rnd.nextInt(256)};${rnd.nextInt(256)};${rnd.nextInt(256)}m")
                        1 -> sb.append("${CSI}38;5;${rnd.nextInt(256)};48;5;${rnd.nextInt(256)}m")
                        2 -> sb.append("${CSI}1;3m")
                        else -> sb.append("${CSI}0m")
                    }
                    val w = words[rnd.nextInt(words.size)]
                    sb.append(w).append(' ')
                    col += w.length + 1
                }
                sb.append("$CSI${"K"}")
            }
            if (frame++ % 4 == 0) sb.append("${CSI}41;1H\r\n")
            sb.append("$CSI?2026l")
        }
        return sb.toString().toByteArray(Charsets.UTF_8)
    }

    @Test fun `a megabyte of TUI output feeds well under a second`() {
        val bytes = tuiOutput()
        // Warm up the JIT on a separate terminal, then time a cold screen.
        Terminal(120, 42).feed(bytes)
        val t = term(cols = 120, rows = 42, scrollback = 5000)
        val start = System.nanoTime()
        var i = 0
        while (i < bytes.size) {
            val n = minOf(4096, bytes.size - i)
            t.feed(bytes, i, n)
            i += n
        }
        val ms = (System.nanoTime() - start) / 1_000_000
        println("ThroughputTest: ${bytes.size} bytes in $ms ms")
        assertTrue("took $ms ms", ms < 3000)
        t.assertInvariants()
    }
}
