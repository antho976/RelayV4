package com.quietsoftware.relay.core.term

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue

internal const val ESC = "\u001b"
internal const val CSI = "\u001b["

/** A terminal that throws instead of recovering, so a bug fails the test that hit it. */
internal fun term(cols: Int = 10, rows: Int = 5, scrollback: Int = 100): Terminal =
    Terminal(cols, rows, scrollback).also { it.strict = true }

/** A line's text: empty cells as spaces, wide tails skipped, extras kept, trailing spaces trimmed. */
internal fun Line.str(): String {
    val sb = StringBuilder()
    for (c in 0 until length) {
        val v = char(c)
        if (v == Line.WIDE_TAIL) continue
        if (v == 0) sb.append(' ') else sb.appendCodePoint(v)
        extra(c)?.let(sb::append)
    }
    return sb.toString().trimEnd()
}

internal fun Terminal.line(r: Int): String = row(r).str()

internal fun Terminal.screen(): List<String> = (0 until rows).map { line(it) }

internal fun Terminal.scrollback(): List<String> = (0 until scrollbackSize).map { scrollbackRow(it).str() }

internal fun Terminal.assertCursor(row: Int, col: Int) {
    assertEquals("cursor (row, col)", row to col, cursorRow to cursorCol)
}

/** What must hold after any input: the cursor on screen, rows full width, wide pairs whole. */
internal fun Terminal.assertInvariants() {
    assertTrue("cursorRow $cursorRow of $rows", cursorRow in 0 until rows)
    assertTrue("cursorCol $cursorCol of $cols", cursorCol in 0 until cols)
    assertTrue(scrollbackSize <= scrollbackLimit)
    for (r in 0 until rows) {
        val l = row(r)
        assertEquals("row $r width", cols, l.length)
        for (c in 0 until l.length) {
            val v = l.char(c)
            if (v == Line.WIDE_TAIL) {
                assertTrue("row $r col $c: tail without a head", c > 0 && l.char(c - 1) > 0)
                assertEquals("row $r col $c: tail after a narrow char", 2, CharWidth.of(l.char(c - 1)))
            } else if (v > 0 && cols >= 2 && CharWidth.of(v) == 2) {
                assertTrue("row $r col $c: head without a tail", c + 1 < l.length && l.char(c + 1) == Line.WIDE_TAIL)
            } else {
                assertTrue("row $r col $c: bad char $v", v >= 0)
            }
        }
    }
}
