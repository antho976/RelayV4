package com.quietsoftware.relay.core.term

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** UTF-8 decoding, character widths, wide and combining characters, DEC special graphics. */
class UnicodeTest {
    @Test fun `a wide char takes two cells, the second a tail`() {
        val t = term()
        t.feed("a中b")
        val l = t.row(0)
        assertEquals('a'.code, l.char(0))
        assertEquals('中'.code, l.char(1))
        assertEquals(Line.WIDE_TAIL, l.char(2))
        assertEquals('b'.code, l.char(3))
        t.assertCursor(0, 4)
        assertEquals("中", l.text(1))
        assertEquals("", l.text(2))
    }

    @Test fun `a wide char at the last column wraps first`() {
        val t = term(cols = 5)
        t.feed("abcd中")
        assertEquals("abcd", t.line(0))
        assertEquals(0, t.row(0).char(4))
        assertTrue(t.row(0).wrapped)
        assertEquals('中'.code, t.row(1).char(0))
        assertEquals(Line.WIDE_TAIL, t.row(1).char(1))
        t.assertCursor(1, 2)
    }

    @Test fun `a wide char filling the last two columns leaves a pending wrap`() {
        val t = term(cols = 5)
        t.feed("abc中")
        t.assertCursor(0, 4)
        t.feed("d")
        assertEquals(listOf("abc中", "d"), t.screen().take(2))
        assertTrue(t.row(0).wrapped)
    }

    @Test fun `a wide char at the margin without autowrap is pulled left`() {
        val t = term(cols = 5)
        t.feed("$CSI?7labcd中")
        assertEquals("abc中", t.line(0))
        t.assertInvariants()
    }

    @Test fun `overwriting either half of a wide char blanks the other`() {
        val t = term()
        t.feed("中文${CSI}1;1Hx")
        assertEquals("x 文", t.line(0))
        t.feed("${CSI}1;4Hy")
        assertEquals("x  y", t.line(0))
        t.assertInvariants()
        t.feed("${CSI}1;1H中文${CSI}1;2H日")
        assertEquals(" 日", t.line(0))
        t.assertInvariants()
    }

    @Test fun `erasing, inserting and deleting never split a wide pair`() {
        val t = term(cols = 6)
        t.feed("中文日${CSI}1;2H${CSI}X")
        t.assertInvariants()
        assertEquals("  文日", t.line(0))
        t.feed("\r中文日${CSI}1;2H${CSI}P")
        t.assertInvariants()
        t.feed("\r中文日${CSI}1;2H${CSI}@")
        t.assertInvariants()
        t.feed("\r中文日${CSI}1;1H${CSI}@")
        t.assertInvariants()
        assertEquals(" 中文", t.line(0))
        t.feed("\r中文日${CSI}1;4H${CSI}1K")
        t.assertInvariants()
        assertEquals("    日", t.line(0))
    }

    @Test fun `combining marks join the previous cell without moving the cursor`() {
        val t = term()
        t.feed("éx")
        val l = t.row(0)
        assertEquals('e'.code, l.char(0))
        assertEquals("́", l.extra(0))
        assertEquals('x'.code, l.char(1))
        assertNull(l.extra(1))
        t.assertCursor(0, 2)
        t.feed("\r${CSI}1;1Ho")
        assertNull("overwriting a cell drops its marks", t.row(0).extra(0))
    }

    @Test fun `a combining mark after a pending wrap joins the last column`() {
        val t = term(cols = 3)
        t.feed("abc̈")
        assertEquals("̈", t.row(0).extra(2))
        t.assertCursor(0, 2)
    }

    @Test fun `a combining mark with nothing before it is dropped`() {
        val t = term()
        t.feed("́a")
        assertEquals("a", t.line(0))
    }

    @Test fun `ZWJ sequences and skin tones stay in one wide cell`() {
        val family = "👨‍👩‍👧"
        val t = term()
        t.feed("${family}x")
        val l = t.row(0)
        assertEquals(0x1F468, l.char(0))
        assertEquals("‍👩‍👧", l.extra(0))
        assertEquals(Line.WIDE_TAIL, l.char(1))
        assertEquals('x'.code, l.char(2))
        assertEquals(family, l.text(0))

        t.feed("\r\n👍🏽y")
        val thumbs = t.row(1)
        assertEquals(0x1F44D, thumbs.char(0))
        assertEquals("🏽", thumbs.extra(0))
        assertEquals('y'.code, thumbs.char(2))
        t.assertInvariants()
    }

    @Test fun `a variation selector attaches`() {
        val t = term()
        t.feed("❤️a")
        assertEquals(0x2764, t.row(0).char(0))
        assertEquals("️", t.row(0).extra(0))
        assertEquals('a'.code, t.row(0).char(1))
    }

    @Test fun `a lone skin tone is a wide char of its own`() {
        val t = term()
        t.feed("a🏽")
        assertEquals(0x1F3FD, t.row(0).char(1))
        assertEquals(Line.WIDE_TAIL, t.row(0).char(2))
    }

    @Test fun `UTF-8 split across feeds is carried over`() {
        val text = "é中😀"
        val bytes = text.toByteArray(Charsets.UTF_8)
        val t = term()
        for (b in bytes) t.feed(byteArrayOf(b))
        assertEquals(text, t.line(0))
        val u = term()
        u.feed(bytes, 0, 2)
        u.feed(bytes, 2, 4)
        u.feed(bytes, 6, bytes.size - 6)
        assertEquals(text, u.line(0))
    }

    @Test fun `malformed UTF-8 becomes U+FFFD`() {
        fun decoded(vararg b: Int): String = term(cols = 20).apply { feed(ByteArray(b.size) { b[it].toByte() }) }.line(0)
        assertEquals("a�b", decoded(0x61, 0x80, 0x62))
        assertEquals("truncated, then ASCII", "�b", decoded(0xE4, 0xB8, 0x62))
        assertEquals("overlong", "��", decoded(0xC0, 0x80))
        assertEquals("surrogate", "���", decoded(0xED, 0xA0, 0x80))
        assertEquals("beyond U+10FFFF", "����", decoded(0xF4, 0x90, 0x80, 0x80))
        assertEquals("��", decoded(0xFF, 0xFE))
    }

    @Test fun `an escape interrupting a UTF-8 sequence still works`() {
        val t = term()
        t.feed(byteArrayOf(0xE4.toByte()) + "${CSI}2;2Hx".toByteArray())
        assertEquals("�", t.line(0))
        assertEquals(" x", t.line(1))
    }

    @Test fun `character widths`() {
        assertEquals(1, CharWidth.of('a'.code))
        assertEquals(1, CharWidth.of('é'.code))
        assertEquals(2, CharWidth.of('中'.code))
        assertEquals(2, CharWidth.of('한'.code))
        assertEquals(2, CharWidth.of('Ａ'.code))
        assertEquals(2, CharWidth.of(0x3000))
        assertEquals(2, CharWidth.of(0x1F600))
        assertEquals(2, CharWidth.of(0x1F680))
        assertEquals(2, CharWidth.of(0x1FAE0))
        assertEquals(2, CharWidth.of(0x2705))
        assertEquals(2, CharWidth.of(0x26A1))
        assertEquals(2, CharWidth.of(0x20000))
        assertEquals(0, CharWidth.of(0x0301))
        assertEquals(0, CharWidth.of(0x200D))
        assertEquals(0, CharWidth.of(0xFE0F))
        assertEquals(0, CharWidth.of(0xFE0E))
        assertEquals(0, CharWidth.of(0xE0100))
        assertEquals(0, CharWidth.of(0x1160))
        // What Claude Code and Codex draw with: all narrow.
        for (c in intArrayOf(0x23FA, 0x23BF, 0x273B, 0x2713, 0x2717, 0x25CF, 0x2500, 0x256D, 0x2026, 0x2192, 0x26A0, 0x2764, 0x1F321)) {
            assertEquals("U+%04X".format(c), 1, CharWidth.of(c))
        }
    }

    @Test fun `the width tables are sorted and disjoint`() {
        for (name in listOf("ZERO", "WIDE")) {
            val f = CharWidth::class.java.getDeclaredField(name).apply { isAccessible = true }
            val r = f.get(CharWidth) as IntArray
            assertEquals(0, r.size % 2)
            for (i in r.indices step 2) {
                assertTrue("$name ${"%X".format(r[i])}", r[i] <= r[i + 1])
                if (i > 0) assertTrue("$name ${"%X".format(r[i])} after ${"%X".format(r[i - 1])}", r[i] > r[i - 1])
            }
        }
    }

    @Test fun `DEC special graphics draw boxes`() {
        val t = term()
        t.feed("$ESC(0lqk\r\nx x\r\nmqj$ESC(Bq")
        assertEquals(listOf("┌─┐", "│ │", "└─┘q"), t.screen().take(3))
        t.feed("\r\n$ESC)0a\u000Eatuvwn~\u000Fa")
        assertEquals("a▒├┤┴┬┼·a", t.line(3))
    }

    @Test fun `the UK set swaps the pound sign`() {
        val t = term()
        t.feed("$ESC(A#$ESC(B#")
        assertEquals("£#", t.line(0))
    }
}
