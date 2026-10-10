package com.quietsoftware.relay.core.term

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TerminalTest {
    @Test fun `printing advances the cursor`() {
        val t = term()
        t.feed("hello")
        assertEquals("hello", t.line(0))
        t.assertCursor(0, 5)
    }

    @Test fun `the last column holds the cursor until the next char wraps it`() {
        val t = term(cols = 5)
        t.feed("abcde")
        t.assertCursor(0, 4)
        assertFalse(t.row(0).wrapped)
        t.feed("f")
        assertEquals(listOf("abcde", "f"), t.screen().take(2))
        assertTrue(t.row(0).wrapped)
        t.assertCursor(1, 1)
    }

    @Test fun `CR LF after a full line does not leave an empty line`() {
        val t = term(cols = 5)
        t.feed("abcde\r\nfg")
        assertEquals(listOf("abcde", "fg", ""), t.screen().take(3))
        assertFalse(t.row(0).wrapped)
    }

    @Test fun `a cursor move cancels the pending wrap`() {
        val t = term(cols = 5)
        t.feed("abcde${CSI}2DX")
        assertEquals("abXde", t.line(0))
        t.assertCursor(0, 3)
    }

    @Test fun `backspace at the pending wrap steps back from the last column`() {
        val t = term(cols = 5)
        t.feed("abcde\bX")
        assertEquals("abcXe", t.line(0))
    }

    @Test fun `EL at the pending wrap spares the last column and keeps the wrap`() {
        // grep and GCC end coloured text with ESC[K; on a full line it must not eat the last char.
        val t = term(cols = 5)
        t.feed("abcde${CSI}K")
        assertEquals("abcde", t.line(0))
        t.feed("f")
        assertEquals(listOf("abcde", "f"), t.screen().take(2))
        t.feed("${CSI}1;1Hvwxyz${CSI}J")
        assertEquals(listOf("vwxyz", ""), t.screen().take(2))
        t.assertCursor(0, 4)
    }

    @Test fun `REP may run past a screenful`() {
        val t = term(cols = 2, rows = 1)
        t.feed("Z${CSI}4b")
        assertEquals(listOf("ZZ", "ZZ"), t.scrollback())
        assertEquals("Z", t.line(0))
    }

    @Test fun `with autowrap off the last column is overwritten`() {
        val t = term(cols = 5)
        t.feed("$CSI?7labcdefg")
        assertEquals(listOf("abcdg", ""), t.screen().take(2))
        t.assertCursor(0, 4)
    }

    @Test fun `CR returns and LF indexes, keeping the column`() {
        val t = term()
        t.feed("ab\ncd\rX")
        assertEquals(listOf("ab", "X cd"), t.screen().take(2))
    }

    @Test fun `LNM makes LF a newline`() {
        val t = term()
        t.feed("${CSI}20hab\ncd")
        assertEquals(listOf("ab", "cd"), t.screen().take(2))
        t.feed("${CSI}20l\nx")
        assertEquals("  x", t.line(2))
    }

    @Test fun `cursor movement clamps to the screen`() {
        val t = term(cols = 10, rows = 5)
        t.feed("${CSI}99A"); t.assertCursor(0, 0)
        t.feed("${CSI}99B"); t.assertCursor(4, 0)
        t.feed("${CSI}99C"); t.assertCursor(4, 9)
        t.feed("${CSI}3D"); t.assertCursor(4, 6)
        t.feed("${CSI}99D"); t.assertCursor(4, 0)
        t.feed("${CSI}99;99H"); t.assertCursor(4, 9)
        t.feed("${CSI}0;0H"); t.assertCursor(0, 0)
        t.feed("${CSI}3;4f"); t.assertCursor(2, 3)
        t.feed("${CSI}H"); t.assertCursor(0, 0)
        t.feed("${CSI};5H"); t.assertCursor(0, 4)
        t.feed("${CSI}7G"); t.assertCursor(0, 6)
        t.feed("${CSI}3d"); t.assertCursor(2, 6)
        t.feed("${CSI}2E"); t.assertCursor(4, 0)
        t.feed("${CSI}5`${CSI}F"); t.assertCursor(3, 0)
        t.feed("${CSI}2a"); t.assertCursor(3, 2)
        t.feed("${CSI}e"); t.assertCursor(4, 2)
        t.feed("${CSI}A"); t.assertCursor(3, 2)
    }

    @Test fun `CUU and CUD stop at the margins from inside the region`() {
        val t = term(rows = 10)
        t.feed("${CSI}3;6r")
        t.feed("${CSI}5;1H${CSI}99A"); t.assertCursor(2, 0)
        t.feed("${CSI}99B"); t.assertCursor(5, 0)
        t.feed("${CSI}9;1H${CSI}99B"); t.assertCursor(9, 0)
        t.feed("${CSI}1;1H${CSI}99B"); t.assertCursor(5, 0)
    }

    @Test fun `origin mode addresses the scroll region`() {
        val t = term(rows = 10)
        t.feed("${CSI}3;6r${CSI}?6h")
        t.assertCursor(2, 0)
        t.feed("${CSI}2;3H"); t.assertCursor(3, 2)
        t.feed("${CSI}99;1H"); t.assertCursor(5, 0)
        t.feed("${CSI}?6l"); t.assertCursor(0, 0)
    }

    @Test fun `DECSTBM homes the cursor and ignores an empty region`() {
        val t = term(rows = 10)
        t.feed("${CSI}5;5H${CSI}2;4r")
        t.assertCursor(0, 0)
        t.feed("${CSI}5;5H${CSI}4;4r")
        t.assertCursor(4, 4)
    }

    @Test fun `erase in line takes the pen's background`() {
        val t = term()
        t.feed("abcdefghij$CSI${"1;44;4m"}${CSI}1;4H${CSI}K")
        assertEquals("abc", t.line(0))
        val l = t.row(0)
        assertEquals(TermColor.DEFAULT_BG, Style.bg(l.style(2)))
        for (c in 3 until 10) {
            assertEquals(0, l.char(c))
            assertEquals(4, Style.bg(l.style(c)))
            assertFalse("erase keeps only the background", Style.bold(l.style(c)) || Style.underline(l.style(c)))
        }
        t.feed("${CSI}1K")
        assertEquals("", t.line(0))
        assertEquals(4, Style.bg(t.row(0).style(0)))
        t.feed("${CSI}0mxyz${CSI}2K")
        assertEquals("", t.line(0))
        assertEquals(TermColor.DEFAULT_BG, Style.bg(t.row(0).style(5)))
    }

    @Test fun `erase in display below, above and all`() {
        val t = term(cols = 4, rows = 3)
        t.feed("aaaa\r\nbbbb\r\ncccc")
        t.feed("${CSI}2;2H${CSI}J")
        assertEquals(listOf("aaaa", "b", ""), t.screen())
        t.feed("${CSI}1;1Haaaa\r\nbbbb\r\ncccc${CSI}2;2H${CSI}1J")
        assertEquals(listOf("", "  bb", "cccc"), t.screen())
        t.feed("${CSI}42m${CSI}2J")
        assertEquals(listOf("", "", ""), t.screen())
        assertEquals(2, Style.bg(t.row(0).style(0)))
        t.assertCursor(1, 1)
    }

    @Test fun `ICH, DCH and ECH edit within the line`() {
        val t = term()
        t.feed("abcdef${CSI}1;3H${CSI}2@")
        assertEquals("ab  cdef", t.line(0))
        t.feed("${CSI}3P")
        assertEquals("abdef", t.line(0))
        t.feed("${CSI}2X")
        assertEquals("ab  f", t.line(0))
        t.assertCursor(0, 2)
        t.feed("${CSI}1;1H${CSI}99@")
        assertEquals("", t.line(0))
    }

    @Test fun `ICH pushes cells off the right edge`() {
        val t = term(cols = 5)
        t.feed("abcde${CSI}1;1H${CSI}@")
        assertEquals(" abcd", t.line(0))
    }

    @Test fun `insert mode shifts the rest of the line right`() {
        val t = term()
        t.feed("abcd${CSI}1;2H${CSI}4hXY${CSI}4lZ")
        assertEquals("aXYZcd", t.line(0))
    }

    @Test fun `IL and DL work inside the scroll region only`() {
        val t = term(cols = 3, rows = 6)
        t.feed("1\r\n2\r\n3\r\n4\r\n5\r\n6")
        t.feed("${CSI}2;5r${CSI}3;2H${CSI}L")
        assertEquals(listOf("1", "2", "", "3", "4", "6"), t.screen())
        t.assertCursor(2, 0)
        t.feed("${CSI}2M")
        assertEquals(listOf("1", "2", "4", "", "", "6"), t.screen())
        t.feed("${CSI}6;1H${CSI}L${CSI}1;1H${CSI}M")
        assertEquals("outside the region IL/DL do nothing", listOf("1", "2", "4", "", "", "6"), t.screen())
        t.feed("${CSI}2;1H${CSI}9L")
        assertEquals(listOf("1", "", "", "", "", "6"), t.screen())
    }

    @Test fun `tabs stop every 8 columns, settable and clearable`() {
        val t = term(cols = 30)
        t.feed("\tx")
        t.assertCursor(0, 9)
        t.feed("\t\t\t\t")
        t.assertCursor(0, 29)
        t.feed("\r${CSI}3G${ESC}H\r\ty")
        t.assertCursor(0, 3)
        t.feed("${CSI}3G${CSI}g\r\t")
        t.assertCursor(0, 8)
        t.feed("${CSI}3g\r\t")
        t.assertCursor(0, 29)
        t.feed("${CSI}Z")
        t.assertCursor(0, 0)
    }

    @Test fun `back tab and forward tab take counts`() {
        val t = term(cols = 40)
        t.feed("${CSI}3I"); t.assertCursor(0, 24)
        t.feed("${CSI}2Z"); t.assertCursor(0, 8)
    }

    @Test fun `DECSC and DECRC restore position, pen and charset`() {
        val t = term()
        t.feed("${CSI}2;3H${CSI}1;31m$ESC(0${ESC}7")
        t.feed("${CSI}0m$ESC(B${CSI}5;5H")
        t.feed("${ESC}8q")
        val l = t.row(1)
        assertEquals(0x2500, l.char(2))
        assertTrue(Style.bold(l.style(2)))
        assertEquals(1, Style.fg(l.style(2)))
    }

    @Test fun `CSI s and u save and restore the cursor`() {
        val t = term()
        t.feed("${CSI}2;3H${CSI}s${CSI}5;5H${CSI}u")
        t.assertCursor(1, 2)
    }

    @Test fun `DECRC without a save homes the cursor`() {
        val t = term()
        t.feed("${CSI}3;3H${ESC}8")
        t.assertCursor(0, 0)
    }

    @Test fun `REP repeats the last printed char`() {
        val t = term(cols = 6)
        t.feed("-${CSI}4b")
        assertEquals("-----", t.line(0))
        t.feed("${CSI}5b")
        assertEquals(listOf("------", "----"), t.screen().take(2))
    }

    @Test fun `OSC 0 and 2 set the title, ended by BEL or ST`() {
        val t = term()
        t.feed("$ESC]0;first\u0007")
        assertEquals("first", t.title)
        t.feed("$ESC]2;second é$ESC\\after")
        assertEquals("second é", t.title)
        assertEquals("after", t.line(0))
        t.feed("$ESC]1;icon\u0007$ESC]8;;http://x\u0007")
        assertEquals("second é", t.title)
    }

    @Test fun `unknown and query sequences change nothing`() {
        val t = term()
        t.feed("${CSI}>4;1m${CSI}>1u${CSI}?u${CSI}=1;1u${CSI}>c${CSI}5 q${CSI}22;0t${CSI}?2026\$p${CSI}6n${CSI}c")
        t.feed("${ESC}P1\$r0m${ESC}\\${ESC}_Gf=100;AAAA${ESC}\\${ESC}^pm${ESC}\\${ESC}Xsos${ESC}\\")
        t.feed("${CSI}?1;2;3\$y${CSI}1;2;3;4;5;6;7;8;9;10;11;12;13;14;15;16;17;18;19;20;21;22;23;24;25;26;27;28;29;30;31;32;33;34;35x")
        t.feed("ok")
        assertEquals("ok", t.line(0))
        assertEquals(Style.DEFAULT, t.row(0).style(0))
        t.assertCursor(0, 2)
    }

    @Test fun `queries are answered only through onReply`() {
        val t = term()
        val replies = mutableListOf<String>()
        t.feed("${CSI}6n${CSI}c")
        t.onReply = { replies += it }
        t.feed("${CSI}3;5H${CSI}6n${CSI}c${CSI}0c${CSI}>c${CSI}5n")
        assertEquals(listOf("$CSI${"3;5R"}", "$CSI?62;22c", "$CSI?62;22c", "${CSI}0n"), replies)
    }

    @Test fun `C1 controls in UTF-8 work like their ESC forms`() {
        val t = term()
        t.feed("\u009b2;3HX\u009d2;c1 title\u009c")
        assertEquals("  X", t.line(1))
        assertEquals("c1 title", t.title)
    }

    @Test fun `CAN aborts a sequence`() {
        val t = term()
        t.feed("${CSI}31\u0018mX")
        assertEquals("mX", t.line(0))
        assertEquals(TermColor.DEFAULT_FG, Style.fg(t.row(0).style(0)))
    }

    @Test fun `C0 controls execute inside a CSI sequence`() {
        val t = term()
        t.feed("abc${CSI}1\r;2HX")
        // CR ran mid-sequence, then CUP 1;2 completed.
        assertEquals("aXc", t.line(0))
    }

    @Test fun `DECALN fills the screen with E`() {
        val t = term(cols = 3, rows = 2)
        t.feed("${CSI}2;2H${ESC}#8")
        assertEquals(listOf("EEE", "EEE"), t.screen())
        t.assertCursor(0, 0)
    }

    @Test fun `RIS resets modes, pen and screen`() {
        val t = term()
        t.feed("${CSI}?1h${CSI}?2004h${CSI}?25l${CSI}1;31mred${CSI}?1049h${ESC}=")
        t.feed("${ESC}c")
        assertFalse(t.applicationCursorKeys)
        assertFalse(t.bracketedPaste)
        assertTrue(t.cursorVisible)
        assertFalse(t.altScreen)
        assertFalse(t.applicationKeypad)
        assertEquals(listOf("", "", "", "", ""), t.screen())
        t.assertCursor(0, 0)
        t.feed("x")
        assertEquals(Style.DEFAULT, t.row(0).style(0))
    }

    @Test fun `DECSTR resets modes but keeps the screen`() {
        val t = term()
        t.feed("${CSI}1mab${CSI}?25l${CSI}4h${CSI}!p")
        assertTrue(t.cursorVisible)
        t.feed("c")
        assertEquals("abc", t.line(0))
        assertEquals(Style.DEFAULT, t.row(0).style(2))
    }

    @Test fun `reset forgets everything`() {
        val t = term(rows = 2)
        t.feed("$ESC]2;t\u0007a\r\nb\r\nc\r\n")
        assertTrue(t.scrollbackSize > 0)
        val before = t.version
        t.reset()
        assertEquals(0, t.scrollbackSize)
        assertEquals("", t.title)
        assertEquals(listOf("", ""), t.screen())
        assertNotEquals(before, t.version)
    }

    @Test fun `reset drops a half-received UTF-8 sequence`() {
        val t = term()
        t.feed(byteArrayOf(0xE4.toByte(), 0xB8.toByte()))
        t.reset()
        t.feed("a")
        assertEquals("a", t.line(0))
    }

    @Test fun `version moves on every feed`() {
        val t = term()
        val v0 = t.version
        t.feed("x")
        val v1 = t.version
        assertTrue(v1 > v0)
        t.feed(ByteArray(0))
        assertEquals(v1, t.version)
        t.resize(20, 5)
        assertTrue(t.version > v1)
    }

    @Test fun `feed honours offset and length`() {
        val t = term()
        val bytes = "xxabcxx".toByteArray()
        t.feed(bytes, 2, 3)
        assertEquals("abc", t.line(0))
        t.feed(bytes, 5)
        assertEquals("abcxx", t.line(0))
    }

    @Test fun `cursor visibility and focus and mouse flags follow DECSET`() {
        val t = term()
        t.feed("${CSI}?25l")
        assertFalse(t.cursorVisible)
        t.feed("${CSI}?1000;1006;1004h")
        assertEquals(1000, t.mouseTracking)
        assertEquals(1006, t.mouseEncoding)
        assertTrue(t.focusReporting)
        t.feed("${CSI}?1002h${CSI}?1000l")
        assertEquals("resetting a mode not in force keeps the current one", 1002, t.mouseTracking)
        t.feed("${CSI}?1002;1006;1004l${CSI}?25h")
        assertEquals(0, t.mouseTracking)
        assertEquals(0, t.mouseEncoding)
        assertFalse(t.focusReporting)
        assertTrue(t.cursorVisible)
    }

    @Test fun `keypad mode follows DECKPAM and DECKPNM`() {
        val t = term()
        t.feed("$ESC=")
        assertTrue(t.applicationKeypad)
        t.feed("$ESC>")
        assertFalse(t.applicationKeypad)
    }

    @Test fun `reverse index at the top margin scrolls the region down`() {
        val t = term(cols = 3, rows = 4)
        t.feed("1\r\n2\r\n3\r\n4")
        t.feed("${CSI}2;3r${CSI}2;1H${ESC}M")
        assertEquals(listOf("1", "", "2", "4"), t.screen())
        t.feed("${CSI}r${CSI}1;1H${ESC}M")
        assertEquals(listOf("", "1", "", "2"), t.screen())
    }

    @Test fun `NEL and IND`() {
        val t = term()
        t.feed("ab${ESC}Ec${ESC}Dd")
        assertEquals(listOf("ab", "c", " d"), t.screen().take(3))
    }
}
