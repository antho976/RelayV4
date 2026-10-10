package com.quietsoftware.relay.core.term

/** What the [Parser] recognises, in the vocabulary of Paul Williams' DEC ANSI parser. */
internal interface ParserSink {
    /** A graphic character in the ground state. */
    fun print(cp: Int)

    /** A C0 or C1 control. */
    fun execute(code: Int)

    /** A complete control sequence; read its parameters from [p]. */
    fun csi(p: Parser, final: Int)

    /** An escape sequence; [intermediates] packs up to two intermediate bytes, first in the high byte. */
    fun esc(final: Int, intermediates: Int)

    /** An operating system command's text, without its terminator. */
    fun osc(data: CharSequence)
}

/**
 * The state machine of vt100.net/emu/dec_ansi_parser, fed code points (UTF-8 is already decoded,
 * so C1 controls arrive as U+0080..U+009F). Never throws; whatever it does not recognise it drops.
 *
 * Departures from the diagram, as in xterm and VTE: `:` separates sub-parameters instead of
 * spoiling the sequence, BEL ends an OSC string, and DCS content is never passed on.
 */
internal class Parser(private val sink: ParserSink) {
    var state = GROUND
        private set

    private val params = IntArray(MAX_PARAMS)
    var paramCount = 0
        private set
    private var subMask = 0L
    private var cur = -1
    private var pendingSub = false
    private var sawParam = false

    /** The private marker (`?`, `>`, `=`, `<`) that opened the sequence, or 0. */
    var prefix = 0
        private set
    /** Up to two intermediate bytes, the first in the high byte. */
    var intermediates = 0
        private set
    private var interCount = 0

    private val oscBuf = StringBuilder()

    fun reset() {
        state = GROUND
        clear()
        oscBuf.setLength(0)
    }

    /** Parameter [i], or [default] when it is missing or 0 (the usual rule: 0 means the default). */
    fun param(i: Int, default: Int): Int {
        val v = if (i < paramCount) params[i] else -1
        return if (v <= 0) default else v
    }

    /** Parameter [i] as sent, -1 when missing. */
    fun raw(i: Int): Int = if (i < paramCount) params[i] else -1

    /** True when parameter [i] followed a `:`, a sub-parameter of the one before it. */
    fun isSub(i: Int): Boolean = i in 1 until paramCount && (subMask ushr i) and 1L != 0L

    fun advance(cp: Int) {
        val s = state
        if (s == GROUND && cp >= 0x20 && cp != 0x7F && (cp < 0x80 || cp > 0x9F)) return sink.print(cp)
        when {
            cp == 0x1B -> { exit(true); clear(); state = ESCAPE; return }
            cp == 0x18 || cp == 0x1A -> { exit(false); state = GROUND; return }
            cp in 0x80..0x9F -> { c1(cp); return }
        }
        when (s) {
            GROUND -> if (cp < 0x20) sink.execute(cp)
            ESCAPE -> escape(cp)
            ESCAPE_INTERMEDIATE -> when {
                cp < 0x20 -> sink.execute(cp)
                cp <= 0x2F -> collect(cp)
                cp <= 0x7E -> { state = GROUND; if (interCount <= 2) sink.esc(cp, intermediates) }
                cp > 0x7F -> state = GROUND
            }
            CSI_ENTRY, CSI_PARAM -> csiParam(cp, s == CSI_ENTRY)
            CSI_INTERMEDIATE -> when {
                cp < 0x20 -> sink.execute(cp)
                cp <= 0x2F -> collect(cp)
                cp <= 0x3F -> state = CSI_IGNORE
                cp <= 0x7E -> dispatchCsi(cp)
                cp > 0x7F -> state = CSI_IGNORE
            }
            CSI_IGNORE -> when {
                cp < 0x20 -> sink.execute(cp)
                cp in 0x40..0x7E -> state = GROUND
            }
            DCS_ENTRY -> when {
                cp < 0x20 || cp == 0x7F -> {}
                cp <= 0x2F -> { collect(cp); state = DCS_INTERMEDIATE }
                cp <= 0x3F -> state = DCS_PARAM
                cp <= 0x7E -> state = DCS_PASSTHROUGH
                else -> state = DCS_IGNORE
            }
            DCS_PARAM -> when {
                cp < 0x20 || cp == 0x7F -> {}
                cp <= 0x2F -> { collect(cp); state = DCS_INTERMEDIATE }
                cp <= 0x3B -> {}
                cp <= 0x3F -> state = DCS_IGNORE
                cp <= 0x7E -> state = DCS_PASSTHROUGH
                else -> state = DCS_IGNORE
            }
            DCS_INTERMEDIATE -> when {
                cp < 0x20 || cp == 0x7F -> {}
                cp <= 0x2F -> collect(cp)
                cp <= 0x3F -> state = DCS_IGNORE
                cp <= 0x7E -> state = DCS_PASSTHROUGH
                else -> state = DCS_IGNORE
            }
            DCS_PASSTHROUGH, DCS_IGNORE, SOS_PM_APC -> {}
            OSC_STRING -> when {
                cp == 0x07 -> { state = GROUND; sink.osc(oscBuf) }
                cp < 0x20 || cp == 0x7F -> {}
                oscBuf.length < OSC_MAX -> oscBuf.appendCodePoint(cp)
            }
        }
    }

    private fun escape(cp: Int) {
        when {
            cp < 0x20 -> sink.execute(cp)
            cp <= 0x2F -> { collect(cp); state = ESCAPE_INTERMEDIATE }
            cp == '['.code -> { clear(); state = CSI_ENTRY }
            cp == ']'.code -> { oscBuf.setLength(0); state = OSC_STRING }
            cp == 'P'.code -> { clear(); state = DCS_ENTRY }
            cp == 'X'.code || cp == '^'.code || cp == '_'.code -> state = SOS_PM_APC
            cp <= 0x7E -> { state = GROUND; sink.esc(cp, 0) }
            cp > 0x7F -> state = GROUND
        }
    }

    private fun csiParam(cp: Int, entry: Boolean) {
        when {
            cp < 0x20 -> sink.execute(cp)
            cp in 0x30..0x39 -> {
                if (cur < 0) cur = 0
                if (cur < PARAM_MAX) cur = cur * 10 + (cp - 0x30)
                sawParam = true
                state = CSI_PARAM
            }
            cp == 0x3A || cp == 0x3B -> {
                pushParam()
                pendingSub = cp == 0x3A
                sawParam = true
                state = CSI_PARAM
            }
            cp in 0x3C..0x3F -> if (entry) { prefix = cp; state = CSI_PARAM } else state = CSI_IGNORE
            cp <= 0x2F -> { collect(cp); state = CSI_INTERMEDIATE }
            cp <= 0x7E -> dispatchCsi(cp)
            cp > 0x7F -> state = CSI_IGNORE
        }
    }

    private fun dispatchCsi(final: Int) {
        state = GROUND
        if (sawParam) pushParam()
        if (interCount > 2) return
        sink.csi(this, final)
    }

    private fun pushParam() {
        if (paramCount < MAX_PARAMS) {
            params[paramCount] = cur.coerceAtMost(PARAM_MAX)
            if (pendingSub) subMask = subMask or (1L shl paramCount)
            paramCount++
        }
        cur = -1
        pendingSub = false
    }

    private fun collect(cp: Int) {
        if (interCount < 2) intermediates = (intermediates shl 8) or cp
        interCount++
    }

    private fun clear() {
        paramCount = 0
        subMask = 0L
        cur = -1
        pendingSub = false
        sawParam = false
        prefix = 0
        intermediates = 0
        interCount = 0
    }

    /** Leaving a string state: an OSC ended by ESC or ST is complete, one cancelled by CAN/SUB is not. */
    private fun exit(complete: Boolean) {
        if (state == OSC_STRING && complete) {
            state = GROUND
            sink.osc(oscBuf)
        }
    }

    private fun c1(cp: Int) {
        when (cp) {
            0x90 -> { exit(true); clear(); state = DCS_ENTRY }
            0x9B -> { exit(true); clear(); state = CSI_ENTRY }
            0x9D -> { exit(true); oscBuf.setLength(0); state = OSC_STRING }
            0x98, 0x9E, 0x9F -> { exit(true); state = SOS_PM_APC }
            0x9C -> { exit(true); state = GROUND }
            else -> { exit(true); state = GROUND; sink.execute(cp) }
        }
    }

    companion object {
        const val GROUND = 0
        const val ESCAPE = 1
        const val ESCAPE_INTERMEDIATE = 2
        const val CSI_ENTRY = 3
        const val CSI_PARAM = 4
        const val CSI_INTERMEDIATE = 5
        const val CSI_IGNORE = 6
        const val DCS_ENTRY = 7
        const val DCS_PARAM = 8
        const val DCS_INTERMEDIATE = 9
        const val DCS_PASSTHROUGH = 10
        const val DCS_IGNORE = 11
        const val OSC_STRING = 12
        const val SOS_PM_APC = 13

        const val MAX_PARAMS = 32
        private const val PARAM_MAX = 99_999
        private const val OSC_MAX = 4096
    }
}
