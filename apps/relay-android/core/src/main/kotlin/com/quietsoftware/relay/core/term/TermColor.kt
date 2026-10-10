package com.quietsoftware.relay.core.term

/**
 * Colour codes as [Style] stores them: [DEFAULT_FG] / [DEFAULT_BG], an indexed colour 0..255, or
 * 24-bit RGB carrying [RGB_FLAG].
 */
object TermColor {
    const val DEFAULT_FG: Int = 0x100
    const val DEFAULT_BG: Int = 0x101
    const val RGB_FLAG: Int = 0x100_0000

    fun isRgb(code: Int): Boolean = code and RGB_FLAG != 0
    fun isDefault(code: Int): Boolean = code == DEFAULT_FG || code == DEFAULT_BG
    fun isIndexed(code: Int): Boolean = code in 0..255

    /** 0xRRGGBB for an RGB code. */
    fun rgb(code: Int): Int = code and 0xFF_FFFF

    /** 0..255 for an indexed code. */
    fun indexed(code: Int): Int = code and 0xFF

    fun ofRgb(rgb: Int): Int = RGB_FLAG or (rgb and 0xFF_FFFF)
    fun ofRgb(r: Int, g: Int, b: Int): Int = RGB_FLAG or ((r and 0xFF) shl 16) or ((g and 0xFF) shl 8) or (b and 0xFF)

    private val XTERM_ANSI = intArrayOf(
        0x000000, 0xCD0000, 0x00CD00, 0xCDCD00, 0x0000EE, 0xCD00CD, 0x00CDCD, 0xE5E5E5,
        0x7F7F7F, 0xFF0000, 0x00FF00, 0xFFFF00, 0x5C5CFF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
    )

    @Volatile private var ansi: IntArray = XTERM_ANSI.copyOf()

    /** Replace colours 0..15 with the app theme's (16 entries, 0xRRGGBB). */
    fun setAnsiPalette(colors: IntArray) {
        require(colors.size == 16) { "the ANSI palette has 16 colours" }
        ansi = colors.copyOf()
    }

    fun resetAnsiPalette() {
        ansi = XTERM_ANSI.copyOf()
    }

    /** The xterm 256-colour palette as 0xRRGGBB: 16 themeable colours, a 6×6×6 cube, 24 greys. */
    fun palette256(index: Int): Int {
        val i = index and 0xFF
        if (i < 16) return ansi[i]
        if (i < 232) {
            val c = i - 16
            return (level(c / 36) shl 16) or (level(c / 6 % 6) shl 8) or level(c % 6)
        }
        val g = 8 + (i - 232) * 10
        return (g shl 16) or (g shl 8) or g
    }

    private fun level(v: Int): Int = if (v == 0) 0 else 55 + v * 40

    /** The 0xRRGGBB a code resolves to, [default] standing in for DEFAULT_FG / DEFAULT_BG. */
    fun resolve(code: Int, default: Int): Int = when {
        isRgb(code) -> rgb(code)
        code in 0..255 -> palette256(code)
        else -> default
    }
}
