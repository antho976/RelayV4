package com.quietsoftware.relay.core.term

/**
 * One row of cells. Screen rows are [Terminal.cols] long; a scrollback row keeps the width it had
 * when it scrolled off (there is no reflow), so read [length] per row.
 *
 * A wide character occupies its cell and the next, which holds [WIDE_TAIL]. Every write keeps the
 * pair whole: overwriting either half blanks the other.
 */
class Line internal constructor(cols: Int) {
    internal var cp = IntArray(cols)
    internal var st = LongArray(cols)
    /** Combining marks per cell, allocated the first time a row needs one. */
    internal var ex: Array<String?>? = null

    val length: Int get() = cp.size

    /** This line continues on the next: auto-wrap happened at its end. */
    var wrapped: Boolean = false
        internal set

    /** Unicode code point; 0 for an empty cell; [WIDE_TAIL] for the second half of a wide char. */
    fun char(col: Int): Int = cp[col]

    /** Combining marks or a ZWJ sequence's continuation, drawn with the cell's base char. */
    fun extra(col: Int): String? = ex?.get(col)

    fun style(col: Int): Long = st[col]

    /** The cell's text: base char plus extras, "" for an empty cell or a wide char's tail. */
    fun text(col: Int): String {
        val c = cp[col]
        if (c <= 0) return ""
        val e = extra(col)
        return if (e == null) String(Character.toChars(c)) else String(Character.toChars(c)) + e
    }

    internal fun put(col: Int, c: Int, style: Long) {
        val cp = cp
        if (cp[col] == WIDE_TAIL) {
            if (col > 0) blank(col - 1)
        } else if (col + 1 < cp.size && cp[col + 1] == WIDE_TAIL) {
            blank(col + 1)
        }
        cp[col] = c
        st[col] = style
        ex?.set(col, null)
    }

    internal fun putWide(col: Int, c: Int, style: Long) {
        val cp = cp
        if (cp[col] == WIDE_TAIL && col > 0) blank(col - 1)
        if (col + 2 < cp.size && cp[col + 2] == WIDE_TAIL) blank(col + 2)
        cp[col] = c
        cp[col + 1] = WIDE_TAIL
        st[col] = style
        st[col + 1] = style
        ex?.let { it[col] = null; it[col + 1] = null }
    }

    /** Empties a cell left over from a broken wide pair, keeping its colours. */
    internal fun blank(col: Int) {
        cp[col] = 0
        ex?.set(col, null)
    }

    internal fun appendExtra(col: Int, s: String) {
        val ex = ex ?: arrayOfNulls<String>(cp.size).also { ex = it }
        val old = ex[col]
        if (old == null) ex[col] = s else if (old.length < MAX_EXTRA) ex[col] = old + s
    }

    /** Erases [from, to) to blanks in [style]. */
    internal fun erase(from: Int, to: Int, style: Long) {
        val cp = cp
        val f = from.coerceAtLeast(0)
        val t = to.coerceAtMost(cp.size)
        if (f >= t) return
        if (cp[f] == WIDE_TAIL && f > 0) blank(f - 1)
        if (t < cp.size && cp[t] == WIDE_TAIL) blank(t)
        cp.fill(0, f, t)
        st.fill(style, f, t)
        ex?.fill(null, f, t)
    }

    /** Shifts [at, end) right by [n], dropping what falls off, and blanks the gap (ICH, IRM). */
    internal fun insertBlanks(at: Int, n: Int, style: Long) {
        val len = cp.size
        if (at !in 0 until len || n <= 0) return
        if (n >= len - at) return erase(at, len, style)
        if (cp[at] == WIDE_TAIL && at > 0) { blank(at - 1); blank(at) }
        val firstDropped = len - n
        if (cp[firstDropped] == WIDE_TAIL) blank(firstDropped - 1)
        System.arraycopy(cp, at, cp, at + n, len - n - at)
        System.arraycopy(st, at, st, at + n, len - n - at)
        ex?.let { System.arraycopy(it, at, it, at + n, len - n - at) }
        cp.fill(0, at, at + n)
        st.fill(style, at, at + n)
        ex?.fill(null, at, at + n)
    }

    /** Removes [n] cells at [at], pulling the rest left and blanking the end (DCH). */
    internal fun deleteCells(at: Int, n: Int, style: Long) {
        val len = cp.size
        if (at !in 0 until len || n <= 0) return
        if (n >= len - at) return erase(at, len, style)
        if (cp[at] == WIDE_TAIL && at > 0) blank(at - 1)
        if (cp[at + n] == WIDE_TAIL) blank(at + n)
        System.arraycopy(cp, at + n, cp, at, len - n - at)
        System.arraycopy(st, at + n, st, at, len - n - at)
        ex?.let { System.arraycopy(it, at + n, it, at, len - n - at) }
        cp.fill(0, len - n, len)
        st.fill(style, len - n, len)
        ex?.fill(null, len - n, len)
    }

    /** Blanks the whole line at [cols] wide, reusing its arrays when the width matches. */
    internal fun reset(cols: Int, style: Long) {
        if (cp.size != cols) {
            cp = IntArray(cols)
            st = LongArray(cols)
            ex = null
        } else {
            cp.fill(0)
            ex?.fill(null)
        }
        st.fill(style)
        wrapped = false
    }

    /** Truncates or pads to [cols]; a wide char cut in half is dropped. */
    internal fun resize(cols: Int) {
        val len = cp.size
        if (cols == len) return
        val cutTail = cols in 1 until len && cp[cols] == WIDE_TAIL
        cp = cp.copyOf(cols)
        st = st.copyOf(cols)
        ex = ex?.copyOf(cols)
        if (cutTail) blank(cols - 1)
    }

    internal fun fill(c: Int, style: Long) {
        cp.fill(c)
        st.fill(style)
        ex?.fill(null)
        wrapped = false
    }

    /** Index after the last cell with a character in it, 0 for an empty line. */
    internal fun contentEnd(): Int {
        var i = cp.size
        while (i > 0 && cp[i - 1] == 0) i--
        return i
    }

    companion object {
        /** [char] of the cell after a wide character. */
        const val WIDE_TAIL: Int = -1

        private const val MAX_EXTRA = 32
    }
}
