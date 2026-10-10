package com.quietsoftware.relay.core.term

/**
 * A cell's attributes packed in a Long, so a row's styles are one LongArray.
 *
 * Bits 0..24 hold the foreground, 25..49 the background, 50.. the flags. A colour field is 0 for
 * the default colour, `index + 1` for an indexed one, or the RGB code itself (bit 24 set): the
 * all-default style is 0, and a zero-filled row is a blank one.
 */
object Style {
    const val DEFAULT: Long = 0L

    private const val FIELD = 0x1FF_FFFFL
    private const val BG_SHIFT = 25

    internal const val BOLD = 1L shl 50
    internal const val DIM = 1L shl 51
    internal const val ITALIC = 1L shl 52
    internal const val UNDERLINE = 1L shl 53
    internal const val BLINK = 1L shl 54
    internal const val INVERSE = 1L shl 55
    internal const val INVISIBLE = 1L shl 56
    internal const val STRIKE = 1L shl 57

    internal const val FG_MASK = FIELD
    internal const val BG_MASK = FIELD shl BG_SHIFT

    fun fg(style: Long): Int = decode(style and FIELD, TermColor.DEFAULT_FG)
    fun bg(style: Long): Int = decode((style ushr BG_SHIFT) and FIELD, TermColor.DEFAULT_BG)

    fun bold(style: Long): Boolean = style and BOLD != 0L
    fun dim(style: Long): Boolean = style and DIM != 0L
    fun italic(style: Long): Boolean = style and ITALIC != 0L
    fun underline(style: Long): Boolean = style and UNDERLINE != 0L
    fun blink(style: Long): Boolean = style and BLINK != 0L
    fun inverse(style: Long): Boolean = style and INVERSE != 0L
    fun strikethrough(style: Long): Boolean = style and STRIKE != 0L
    fun invisible(style: Long): Boolean = style and INVISIBLE != 0L

    fun withFg(style: Long, code: Int): Long = (style and FIELD.inv()) or encode(code)
    fun withBg(style: Long, code: Int): Long = (style and BG_MASK.inv()) or (encode(code) shl BG_SHIFT)

    /** What an erase leaves behind: the pen's background and nothing else (xterm's BCE). */
    internal fun erased(pen: Long): Long = pen and BG_MASK

    private fun encode(code: Int): Long = when {
        code and TermColor.RGB_FLAG != 0 -> (code and 0x1FF_FFFF).toLong()
        code in 0..255 -> (code + 1).toLong()
        else -> 0L
    }

    private fun decode(field: Long, default: Int): Int {
        val f = field.toInt()
        return when {
            f == 0 -> default
            f and TermColor.RGB_FLAG != 0 -> f
            else -> f - 1
        }
    }
}
