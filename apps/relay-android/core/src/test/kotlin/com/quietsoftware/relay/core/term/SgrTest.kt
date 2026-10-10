package com.quietsoftware.relay.core.term

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SgrTest {
    /** The style of the cell an `x` printed after [sgr] lands in. */
    private fun styleAfter(sgr: String): Long {
        val t = term()
        t.feed("$sgr" + "x")
        return t.row(0).style(0)
    }

    @After fun palette() = TermColor.resetAnsiPalette()

    @Test fun `the default style is zero and decodes to default colours`() {
        assertEquals(0L, Style.DEFAULT)
        assertEquals(TermColor.DEFAULT_FG, Style.fg(Style.DEFAULT))
        assertEquals(TermColor.DEFAULT_BG, Style.bg(Style.DEFAULT))
        assertEquals(Style.DEFAULT, styleAfter(""))
    }

    @Test fun `attributes switch on and off`() {
        val on = styleAfter("${CSI}1;2;3;4;5;7;8;9m")
        assertTrue(Style.bold(on)); assertTrue(Style.dim(on)); assertTrue(Style.italic(on))
        assertTrue(Style.underline(on)); assertTrue(Style.blink(on)); assertTrue(Style.inverse(on))
        assertTrue(Style.invisible(on)); assertTrue(Style.strikethrough(on))
        val off = styleAfter("${CSI}1;2;3;4;5;7;8;9m${CSI}22;23;24;25;27;28;29m")
        assertEquals(Style.DEFAULT, off)
        assertEquals(Style.DEFAULT, styleAfter("${CSI}1;31;44m${CSI}m"))
        assertEquals(Style.DEFAULT, styleAfter("${CSI}1;31;44m${CSI}0m"))
    }

    @Test fun `22 clears bold and dim together, 21 underlines`() {
        val s = styleAfter("${CSI}1;2m${CSI}22;21m")
        assertFalse(Style.bold(s)); assertFalse(Style.dim(s)); assertTrue(Style.underline(s))
    }

    @Test fun `underline styles with a colon all underline, 4 colon 0 removes it`() {
        assertTrue(Style.underline(styleAfter("${CSI}4:3m")))
        assertTrue(Style.underline(styleAfter("${CSI}4:1m")))
        assertFalse(Style.underline(styleAfter("${CSI}4m${CSI}4:0m")))
        val s = styleAfter("${CSI}4:3;31m")
        assertEquals("the parameter after a sub-parameter group still applies", 1, Style.fg(s))
    }

    @Test fun `16 colours, bright ones and defaults`() {
        val s = styleAfter("${CSI}31;42m")
        assertEquals(1, Style.fg(s)); assertEquals(2, Style.bg(s))
        val b = styleAfter("${CSI}97;100m")
        assertEquals(15, Style.fg(b)); assertEquals(8, Style.bg(b))
        val d = styleAfter("${CSI}31;42m${CSI}39;49m")
        assertEquals(TermColor.DEFAULT_FG, Style.fg(d)); assertEquals(TermColor.DEFAULT_BG, Style.bg(d))
    }

    @Test fun `256 colours with semicolons and colons`() {
        val s = styleAfter("${CSI}38;5;196;48;5;21m")
        assertEquals(196, Style.fg(s)); assertEquals(21, Style.bg(s))
        assertFalse(TermColor.isRgb(Style.fg(s)))
        assertEquals(196, TermColor.indexed(Style.fg(s)))
        val c = styleAfter("${CSI}38:5:208;48:5:0m")
        assertEquals(208, Style.fg(c)); assertEquals(0, Style.bg(c))
        assertEquals("index 0 is not the default", 0, Style.bg(c))
    }

    @Test fun `truecolour in every form`() {
        fun fgRgb(sgr: String): Int = Style.fg(styleAfter(sgr)).also { assertTrue(TermColor.isRgb(it)) }.let(TermColor::rgb)
        assertEquals(0x0A141E, fgRgb("${CSI}38;2;10;20;30m"))
        assertEquals(0x0A141E, fgRgb("${CSI}38:2::10:20:30m"))
        assertEquals(0x0A141E, fgRgb("${CSI}38:2:10:20:30m"))
        assertEquals(0x0A141E, fgRgb("${CSI}38:2:0:10:20:30m"))
        val bg = styleAfter("${CSI}48;2;255;128;0;1m")
        assertEquals(0xFF8000, TermColor.rgb(Style.bg(bg)))
        assertTrue("the parameter after the colour applies", Style.bold(bg))
        val black = styleAfter("${CSI}38;2;0;0;0m")
        assertTrue("RGB black is not the default colour", TermColor.isRgb(Style.fg(black)))
        assertEquals(0, TermColor.rgb(Style.fg(black)))
    }

    @Test fun `underline colour is consumed and ignored`() {
        val s = styleAfter("${CSI}58;2;1;2;3;31m")
        assertEquals(1, Style.fg(s))
        assertEquals(TermColor.DEFAULT_BG, Style.bg(s))
        val c = styleAfter("${CSI}58:5:9;4m")
        assertTrue(Style.underline(c))
        assertEquals(TermColor.DEFAULT_FG, Style.fg(c))
    }

    @Test fun `missing parameters count as 0`() {
        val s = styleAfter("${CSI}31m${CSI};1m")
        assertTrue(Style.bold(s))
        assertEquals(TermColor.DEFAULT_FG, Style.fg(s))
    }

    @Test fun `truncated extended colours do not crash or leak`() {
        assertEquals(Style.DEFAULT, styleAfter("${CSI}38;5m"))
        assertEquals(Style.DEFAULT, styleAfter("${CSI}38;2;1;2m"))
        assertEquals(Style.DEFAULT, styleAfter("${CSI}38m"))
        assertEquals(Style.DEFAULT, styleAfter("${CSI}38:2:1m"))
        assertEquals(Style.DEFAULT, styleAfter("${CSI}38;5;300m"))
    }

    @Test fun `fg and bg are independent`() {
        var s = Style.withFg(Style.DEFAULT, TermColor.ofRgb(0xFFFFFF))
        s = Style.withBg(s, 255)
        assertEquals(0xFFFFFF, TermColor.rgb(Style.fg(s)))
        assertEquals(255, Style.bg(s))
        s = Style.withFg(s, TermColor.DEFAULT_FG)
        assertEquals(TermColor.DEFAULT_FG, Style.fg(s))
        assertEquals(255, Style.bg(s))
    }

    @Test fun `the xterm palette`() {
        assertEquals(0x000000, TermColor.palette256(16))
        assertEquals(0x0000FF, TermColor.palette256(21))
        assertEquals(0xFF0000, TermColor.palette256(196))
        assertEquals(0x5F87AF, TermColor.palette256(67))
        assertEquals(0xFFFFFF, TermColor.palette256(231))
        assertEquals(0x080808, TermColor.palette256(232))
        assertEquals(0xEEEEEE, TermColor.palette256(255))
        assertEquals(0xCD0000, TermColor.palette256(1))
    }

    @Test fun `the theme overrides the first 16`() {
        TermColor.setAnsiPalette(IntArray(16) { it * 0x010101 })
        assertEquals(0x050505, TermColor.palette256(5))
        assertEquals(0xFF0000, TermColor.palette256(196))
        assertEquals(0x050505, TermColor.resolve(5, 0xABCDEF))
        assertEquals(0xABCDEF, TermColor.resolve(TermColor.DEFAULT_FG, 0xABCDEF))
        assertEquals(0x123456, TermColor.resolve(TermColor.ofRgb(0x123456), 0))
        TermColor.resetAnsiPalette()
        assertEquals(0xCD00CD, TermColor.palette256(5))
    }

    @Test fun `the pen colours erased cells but not their attributes`() {
        val t = term()
        t.feed("${CSI}1;4;38;2;1;2;3;48;5;17m${CSI}2J")
        val s = t.row(3).style(4)
        assertEquals(17, Style.bg(s))
        assertEquals(TermColor.DEFAULT_FG, Style.fg(s))
        assertFalse(Style.bold(s) || Style.underline(s))
    }
}
