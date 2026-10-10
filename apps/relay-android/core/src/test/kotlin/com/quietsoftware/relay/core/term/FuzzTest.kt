package com.quietsoftware.relay.core.term

import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.random.Random

/** Arbitrary input never throws and never leaves the screen inconsistent. */
class FuzzTest {
    @Test fun `random bytes`() {
        val rnd = Random(20261009)
        repeat(200) { run ->
            val t = term(cols = rnd.nextInt(1, 120), rows = rnd.nextInt(1, 50), scrollback = rnd.nextInt(0, 200))
            val bytes = rnd.nextBytes(10 * 1024)
            var i = 0
            while (i < bytes.size) {
                val n = rnd.nextInt(1, 2048).coerceAtMost(bytes.size - i)
                t.feed(bytes, i, n)
                i += n
            }
            check(t, "run $run")
        }
    }

    private val fragments = listOf(
        ESC, CSI, "$CSI?", "$CSI>", "$CSI=", "$CSI<", "$ESC]", "${ESC}P", "${ESC}_", "${ESC}^", "${ESC}X",
        "$ESC\\", "\u0007", "\u0018", "\u001a", ";", ":", "::", "\r", "\n", "\b", "\t", "\u000b", "\u000c",
        "\u000e", "\u000f", "$ESC(0", "$ESC(B", "$ESC)0", "$ESC#8", "${ESC}7", "${ESC}8", "${ESC}D", "${ESC}E",
        "${ESC}H", "${ESC}M", "${ESC}c", "$ESC=", "$ESC>", "\u009b", "\u009d", "\u0090", "\u009c", "\u0085",
        "中", "文", "😀", "👍🏽", "‍", "́", "️", "é", "x", "hello ", "─", "�",
        "${CSI}?1049h", "${CSI}?1049l", "${CSI}?47h", "${CSI}?1047l", "${CSI}?6h", "${CSI}?6l", "${CSI}?7l",
        "${CSI}?7h", "${CSI}4h", "${CSI}4l", "${CSI}20h", "${CSI}20l", "${CSI}?2026h", "${CSI}?2026l",
        "${CSI}38:2::1:2:3m", "${CSI}48;5;200m", "${CSI}38;2;1m", "${CSI}m", "${CSI}3J", "${CSI}!p",
    )

    private val finals = "@ABCDEFGHIJKLMPSTXZ`abcdefghlmnqrstu"

    private fun noise(rnd: Random): String {
        val sb = StringBuilder()
        while (sb.length < 10 * 1024) {
            when (rnd.nextInt(10)) {
                in 0..4 -> sb.append(fragments[rnd.nextInt(fragments.size)])
                5, 6 -> {
                    // A well-formed CSI with random parameters, so the dispatchers see real work.
                    sb.append(CSI)
                    if (rnd.nextInt(4) == 0) sb.append("?>=<"[rnd.nextInt(4)])
                    repeat(rnd.nextInt(0, 6)) { k ->
                        if (k > 0) sb.append(if (rnd.nextInt(5) == 0) ':' else ';')
                        if (rnd.nextInt(6) > 0) sb.append(randomNumber(rnd))
                    }
                    if (rnd.nextInt(8) == 0) sb.append(" !\$\"'"[rnd.nextInt(5)])
                    sb.append(finals[rnd.nextInt(finals.length)])
                }
                7 -> sb.append(randomNumber(rnd))
                8 -> sb.appendCodePoint(rnd.nextInt(0x20, 0x7F))
                else -> {
                    val cp = rnd.nextInt(0, 0x30000)
                    if (cp !in 0xD800..0xDFFF) sb.appendCodePoint(cp)
                }
            }
        }
        return sb.toString()
    }

    private fun randomNumber(rnd: Random): String = when (rnd.nextInt(6)) {
        0 -> "0"
        1 -> rnd.nextInt(1, 10).toString()
        2 -> rnd.nextInt(1, 300).toString()
        3 -> rnd.nextInt(1000, 3000).toString()
        4 -> "99999999999"
        else -> rnd.nextInt(1, 60).toString()
    }

    @Test fun `random escape sequence fragments`() {
        val rnd = Random(4242)
        repeat(200) { run ->
            val t = term(cols = rnd.nextInt(1, 100), rows = rnd.nextInt(1, 40), scrollback = rnd.nextInt(0, 50))
            val bytes = noise(rnd).toByteArray(Charsets.UTF_8)
            var i = 0
            while (i < bytes.size) {
                val n = rnd.nextInt(1, 512).coerceAtMost(bytes.size - i)
                t.feed(bytes, i, n)
                i += n
                if (rnd.nextInt(40) == 0) t.resize(rnd.nextInt(1, 100), rnd.nextInt(1, 40))
                if (rnd.nextInt(10) == 0) check(t, "run $run at $i")
            }
            check(t, "run $run")
        }
    }

    @Test fun `the guard recovers when not strict`() {
        val t = Terminal(20, 5)
        val rnd = Random(7)
        repeat(20) { t.feed(noise(rnd)) }
        t.feed("${ESC}cok")
        assertTrue(t.line(0) == "ok")
    }

    private fun check(t: Terminal, where: String) {
        try {
            t.assertInvariants()
            t.plainText(50)
            t.textBetween(0, 0, t.scrollbackSize + t.rows - 1, t.cols)
            for (r in 0 until t.scrollbackSize) t.scrollbackRow(r).length
        } catch (e: AssertionError) {
            throw AssertionError("$where: ${e.message}", e)
        }
    }
}
