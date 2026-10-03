/**
 * A terminal screen for a phone: enough of a VT emulator that an agent CLI which redraws in
 * place — Claude Code's prompt box, a spinner, a menu — reads as it looks on the PC, instead
 * of every redraw stacked under the last. A grid of cells the size of the PTY, a cursor the
 * program moves, lines that scroll off the top into a bounded history, and the alternate
 * screen full-screen programs draw on. Colours and other attributes are dropped: the phone
 * shows text.
 */

/** History kept above the screen; older lines fall off the top. */
export const TERMINAL_MAX_LINES = 1500

// Parser states.
const GROUND = 0
const ESCAPE = 1
const ESCAPE_INTERMEDIATE = 2
const CSI = 3
const OSC = 4
/** DCS, SOS, PM and APC: strings that end at ST and mean nothing to a text view. */
const STRING = 5

/** A CSI's parameters longer than this are not a real sequence; the rest is ignored. */
const MAX_PARAMS = 64

/** DEC special graphics, which some TUIs draw their boxes in after `ESC ( 0`. */
const DEC_GRAPHICS: Record<string, string> = {
    '`': '◆',
    a: '▒',
    f: '°',
    g: '±',
    j: '┘',
    k: '┐',
    l: '┌',
    m: '└',
    n: '┼',
    o: '⎺',
    p: '⎻',
    q: '─',
    r: '⎼',
    s: '⎽',
    t: '├',
    u: '┤',
    v: '┴',
    w: '┬',
    x: '│',
    y: '≤',
    z: '≥',
    '{': 'π',
    '|': '≠',
    '}': '£',
    '~': '·',
}

/** Code points that take two cells: East Asian wide and fullwidth, and emoji. Sorted. */
const WIDE: [number, number][] = [
    [0x1100, 0x115f],
    [0x231a, 0x231b],
    [0x2329, 0x232a],
    [0x23e9, 0x23ec],
    [0x23f0, 0x23f0],
    [0x23f3, 0x23f3],
    [0x25fd, 0x25fe],
    [0x2614, 0x2615],
    [0x2648, 0x2653],
    [0x267f, 0x267f],
    [0x2693, 0x2693],
    [0x26a1, 0x26a1],
    [0x26aa, 0x26ab],
    [0x26bd, 0x26be],
    [0x26c4, 0x26c5],
    [0x26ce, 0x26ce],
    [0x26d4, 0x26d4],
    [0x26ea, 0x26ea],
    [0x26f2, 0x26f3],
    [0x26f5, 0x26f5],
    [0x26fa, 0x26fa],
    [0x26fd, 0x26fd],
    [0x2705, 0x2705],
    [0x270a, 0x270b],
    [0x2728, 0x2728],
    [0x274c, 0x274c],
    [0x274e, 0x274e],
    [0x2753, 0x2755],
    [0x2757, 0x2757],
    [0x2795, 0x2797],
    [0x27b0, 0x27b0],
    [0x27bf, 0x27bf],
    [0x2b1b, 0x2b1c],
    [0x2b50, 0x2b50],
    [0x2b55, 0x2b55],
    [0x2e80, 0x303e],
    [0x3041, 0x33ff],
    [0x3400, 0x4dbf],
    [0x4e00, 0x9fff],
    [0xa000, 0xa4cf],
    [0xa960, 0xa97f],
    [0xac00, 0xd7a3],
    [0xf900, 0xfaff],
    [0xfe10, 0xfe19],
    [0xfe30, 0xfe6f],
    [0xff00, 0xff60],
    [0xffe0, 0xffe6],
    [0x16fe0, 0x18cff],
    [0x1b000, 0x1b2ff],
    [0x1f004, 0x1f004],
    [0x1f0cf, 0x1f0cf],
    [0x1f18e, 0x1f18e],
    [0x1f191, 0x1f19a],
    [0x1f200, 0x1f265],
    [0x1f300, 0x1f64f],
    [0x1f680, 0x1f6ff],
    [0x1f7e0, 0x1f7f0],
    [0x1f900, 0x1f9ff],
    [0x1fa70, 0x1faff],
    [0x20000, 0x3fffd],
]

/** Cells a code point takes: 0 for a combining mark, 2 for a wide character, else 1. */
export const cellWidth = (cp: number): number => {
    if (cp < 0x300) return 1
    if (
        cp <= 0x36f ||
        (cp >= 0x1ab0 && cp <= 0x1aff) ||
        (cp >= 0x1dc0 && cp <= 0x1dff) ||
        (cp >= 0x200b && cp <= 0x200f) ||
        (cp >= 0x20d0 && cp <= 0x20ff) ||
        (cp >= 0xfe00 && cp <= 0xfe0f) ||
        (cp >= 0xfe20 && cp <= 0xfe2f) ||
        (cp >= 0xe0100 && cp <= 0xe01ef)
    )
        return 0
    if (cp < 0x1100) return 1
    for (const [low, high] of WIDE) {
        if (cp < low) return 1
        if (cp <= high) return 2
    }
    return 1
}

/** A row's cells: one character each, `''` for the right half of a wide one. */
type Row = string[]

const blankRow = (cols: number): Row => new Array<string>(cols).fill(' ')

const isBlank = (row: Row): boolean => {
    for (const cell of row) if (cell !== ' ' && cell !== '') return false
    return true
}

const render = (row: Row): string => row.join('').replace(/ +$/, '')

export class VtScreen {
    private cols: number
    private rows: number
    private history: string[] = []
    private main: Row[]
    /** The alternate screen, while a full-screen program has it. It keeps no history. */
    private alt?: Row[]
    private x = 0
    private y = 0
    /** The cursor sits past the last column: the next character wraps first. */
    private wrapNext = false
    /** The scroll region, inclusive. */
    private top = 0
    private bottom: number
    private autowrap = true
    private insert = false
    private origin = false
    /** G0 is DEC special graphics. */
    private graphics = false
    private saved = { x: 0, y: 0, graphics: false, origin: false }
    /** Line feed returns the carriage too: scrollback text has lost its carriage returns. */
    private newline = false
    /** The last character printed, for REP. */
    private last = ' '

    private state = GROUND
    private params = ''
    private intermediates = ''
    /** A CSI too long or malformed to act on: consumed up to its final byte, then dropped. */
    private bad = false
    /** Inside a string sequence, the previous character was ESC: `\` ends it. */
    private stringEsc = false

    constructor(
        cols = 80,
        rows = 24,
        private readonly maxLines = TERMINAL_MAX_LINES
    ) {
        this.cols = Math.max(2, cols)
        this.rows = Math.max(2, rows)
        this.main = this.blankScreen()
        this.bottom = this.rows - 1
    }

    get size(): { cols: number; rows: number } {
        return { cols: this.cols, rows: this.rows }
    }

    /**
     * Start over at a size, from a block of scrollback text. That text comes from the engine
     * line by line, without the carriage returns that ended them, so a line feed in it starts
     * the next line at its left edge.
     */
    reset(text: string, cols = this.cols, rows = this.rows) {
        this.cols = Math.max(2, cols)
        this.rows = Math.max(2, rows)
        this.history = []
        this.clear()
        this.state = GROUND
        this.newline = true
        this.feed(text)
        this.newline = false
        // A sequence the scrollback cut short is not continued by the live stream.
        this.state = GROUND
    }

    feed(chunk: string) {
        for (const ch of chunk) {
            const cp = ch.codePointAt(0)!
            switch (this.state) {
                case GROUND:
                    if (cp < 0x20 || cp === 0x7f) this.control(cp)
                    else if (cp >= 0x80 && cp < 0xa0) {
                        // C1 controls: nothing a UTF-8 program sends on purpose.
                    } else this.print(ch, cp)
                    break
                case ESCAPE:
                    this.escape(ch, cp)
                    break
                case ESCAPE_INTERMEDIATE:
                    if (cp < 0x20) this.control(cp)
                    else if (cp < 0x30) this.intermediates += ch
                    else {
                        if (this.intermediates === '(') this.graphics = ch === '0'
                        this.state = GROUND
                    }
                    break
                case CSI:
                    if (cp < 0x20) this.control(cp)
                    else if (cp < 0x30) this.intermediates += ch
                    else if (cp < 0x40) {
                        if (this.intermediates || this.params.length >= MAX_PARAMS) this.bad = true
                        else this.params += ch
                    } else if (cp < 0x7f) {
                        if (!this.bad) this.csi(ch)
                        this.state = GROUND
                    }
                    break
                case OSC:
                case STRING:
                    if (this.stringEsc) {
                        this.stringEsc = false
                        if (ch === '\\') {
                            this.state = GROUND
                            break
                        }
                    }
                    if (cp === 0x1b) this.stringEsc = true
                    else if (cp === 0x07 && this.state === OSC) this.state = GROUND
                    else if (cp === 0x18 || cp === 0x1a) this.state = GROUND
                    break
            }
        }
    }

    /** Change size, as the PTY just did. Rows that no longer fit go to the history. */
    resize(cols: number, rows: number) {
        cols = Math.max(2, cols)
        rows = Math.max(2, rows)
        if (cols === this.cols && rows === this.rows) return
        this.cols = cols
        this.rows = rows
        if (this.alt) {
            this.y = this.fit(this.alt, this.y, false)
            this.saved.y = this.fit(this.main, this.saved.y, true)
        } else {
            this.y = this.fit(this.main, this.y, true)
        }
        this.x = Math.min(this.x, cols - 1)
        this.saved.x = Math.min(this.saved.x, cols - 1)
        this.wrapNext = false
        this.top = 0
        this.bottom = rows - 1
    }

    /** The history and the screen, top to bottom; the alternate screen alone while it is up. */
    text(): string {
        const screen = this.screen
        // The screen down to the cursor, or to the last row with anything on it if lower.
        let end = this.y
        for (let i = screen.length - 1; i > end; i--) {
            if (!isBlank(screen[i])) {
                end = i
                break
            }
        }
        const lines: string[] = this.alt ? [] : this.history.slice(-this.maxLines)
        for (let i = 0; i <= end; i++) lines.push(render(screen[i]))
        return lines.join('\n')
    }

    private get screen(): Row[] {
        return this.alt ?? this.main
    }

    /** A blank screen and every mode as it starts. What scrolled away stays. */
    private clear() {
        this.main = this.blankScreen()
        this.alt = undefined
        this.x = 0
        this.y = 0
        this.wrapNext = false
        this.softReset()
    }

    private blankScreen(): Row[] {
        return Array.from({ length: this.rows }, () => blankRow(this.cols))
    }

    /** Fit a grid to the current size; returns where the cursor row went. */
    private fit(grid: Row[], cursorY: number, keep: boolean): number {
        for (const row of grid) {
            if (row.length > this.cols) {
                // A wide character cut in half leaves a blank.
                if (row[this.cols] === '') row[this.cols - 1] = ' '
                row.length = this.cols
            } else {
                while (row.length < this.cols) row.push(' ')
            }
        }
        let y = cursorY
        // Shorter: blank rows under the cursor go first, then rows off the top.
        while (grid.length > this.rows && grid.length - 1 > y && isBlank(grid[grid.length - 1]))
            grid.pop()
        if (grid.length > this.rows) {
            const gone = grid.splice(0, grid.length - this.rows)
            if (keep) this.keep(gone)
            y -= gone.length
        }
        while (grid.length < this.rows) grid.push(blankRow(this.cols))
        return Math.max(0, Math.min(this.rows - 1, y))
    }

    /** Rows scrolled off the top of the main screen, into the history. */
    private keep(rows: Row[]) {
        for (const row of rows) this.history.push(render(row))
        // Trimmed in batches: one splice per line would copy the history for every line.
        if (this.history.length > this.maxLines + 256)
            this.history.splice(0, this.history.length - this.maxLines)
    }

    private control(cp: number) {
        switch (cp) {
            case 0x08:
                this.wrapNext = false
                if (this.x > 0) this.x--
                break
            case 0x09:
                this.wrapNext = false
                this.x = Math.min(this.cols - 1, (Math.floor(this.x / 8) + 1) * 8)
                break
            case 0x0a:
            case 0x0b:
            case 0x0c:
                this.wrapNext = false
                if (this.newline) this.x = 0
                this.index()
                break
            case 0x0d:
                this.wrapNext = false
                this.x = 0
                break
            case 0x18:
            case 0x1a:
                this.state = GROUND
                break
            case 0x1b:
                this.state = ESCAPE
                this.intermediates = ''
                break
        }
    }

    private escape(ch: string, cp: number) {
        this.state = GROUND
        if (cp < 0x20) {
            this.control(cp)
            return
        }
        if (cp < 0x30) {
            this.intermediates = ch
            this.state = ESCAPE_INTERMEDIATE
            return
        }
        switch (ch) {
            case '[':
                this.state = CSI
                this.params = ''
                this.intermediates = ''
                this.bad = false
                break
            case ']':
                this.state = OSC
                this.stringEsc = false
                break
            case 'P':
            case 'X':
            case '^':
            case '_':
                this.state = STRING
                this.stringEsc = false
                break
            case '7':
                this.saveCursor()
                break
            case '8':
                this.restoreCursor()
                break
            case 'D':
                this.wrapNext = false
                this.index()
                break
            case 'E':
                this.wrapNext = false
                this.x = 0
                this.index()
                break
            case 'M':
                this.wrapNext = false
                this.reverseIndex()
                break
            case 'c':
                this.clear()
                break
        }
    }

    private csi(final: string) {
        const lead = this.params.charAt(0)
        const priv = lead === '?' || lead === '>' || lead === '<' || lead === '=' ? lead : ''
        const body = priv ? this.params.slice(1) : this.params
        const args = body === '' ? [] : body.split(';').map((item) => parseInt(item, 10) || 0)
        /** The i-th parameter, where 0 and absent both mean `fallback`. */
        const n = (i: number, fallback = 1) => args[i] || fallback
        if (this.intermediates) {
            // `CSI ! p`: a soft reset. Every other sequence with an intermediate is a style.
            if (this.intermediates === '!' && final === 'p') this.softReset()
            return
        }
        if (priv && priv !== '?') return
        switch (final) {
            case '@':
                if (!priv) this.insertChars(n(0))
                break
            case 'A':
                this.moveTo(this.x, Math.max(this.y >= this.top ? this.top : 0, this.y - n(0)))
                break
            case 'B':
            case 'e':
                this.moveTo(
                    this.x,
                    Math.min(this.y <= this.bottom ? this.bottom : this.rows - 1, this.y + n(0))
                )
                break
            case 'C':
            case 'a':
                this.moveTo(this.x + n(0), this.y)
                break
            case 'D':
                this.moveTo(this.x - n(0), this.y)
                break
            case 'E':
                this.moveTo(
                    0,
                    Math.min(this.y <= this.bottom ? this.bottom : this.rows - 1, this.y + n(0))
                )
                break
            case 'F':
                this.moveTo(0, Math.max(this.y >= this.top ? this.top : 0, this.y - n(0)))
                break
            case 'G':
            case '`':
                this.moveTo(n(0) - 1, this.y)
                break
            case 'H':
            case 'f':
                this.moveTo(n(1) - 1, (this.origin ? this.top : 0) + n(0) - 1)
                break
            case 'd':
                this.moveTo(this.x, (this.origin ? this.top : 0) + n(0) - 1)
                break
            case 'I':
                for (let i = 0; i < n(0); i++) this.control(0x09)
                break
            case 'Z':
                this.moveTo(Math.max(0, (Math.ceil(this.x / 8) - n(0)) * 8), this.y)
                break
            case 'J':
                this.eraseDisplay(args[0] ?? 0)
                break
            case 'K':
                this.eraseLine(args[0] ?? 0)
                break
            case 'L':
                if (!priv) this.insertLines(n(0))
                break
            case 'M':
                if (!priv) this.deleteLines(n(0))
                break
            case 'P':
                if (!priv) this.deleteChars(n(0))
                break
            case 'S':
                if (!priv) this.scrollUp(n(0))
                break
            case 'T':
                if (!priv && args.length <= 1) this.scrollDown(n(0))
                break
            case 'X':
                if (!priv) this.eraseChars(n(0))
                break
            case 'b':
                if (!priv)
                    for (let i = 0; i < Math.min(n(0), this.cols * this.rows); i++) {
                        this.print(this.last, this.last.codePointAt(0)!)
                    }
                break
            case 'r':
                if (!priv) this.setRegion(n(0), n(1, this.rows))
                break
            case 's':
                if (!priv) this.saveCursor()
                break
            case 'u':
                if (!priv) this.restoreCursor()
                break
            case 'h':
            case 'l':
                for (const mode of args) this.setMode(priv, mode, final === 'h')
                break
        }
    }

    private setMode(priv: string, mode: number, on: boolean) {
        if (!priv) {
            if (mode === 4) this.insert = on
            return
        }
        switch (mode) {
            case 6:
                this.origin = on
                this.moveTo(0, on ? this.top : 0)
                break
            case 7:
                this.autowrap = on
                break
            case 47:
            case 1047:
            case 1049:
                if (on) {
                    if (mode === 1049) this.saveCursor()
                    if (!this.alt || mode !== 47) this.alt = this.blankScreen()
                } else if (this.alt) {
                    this.alt = undefined
                    if (mode === 1049) this.restoreCursor()
                }
                this.top = 0
                this.bottom = this.rows - 1
                this.wrapNext = false
                break
        }
    }

    private softReset() {
        this.autowrap = true
        this.insert = false
        this.origin = false
        this.graphics = false
        this.top = 0
        this.bottom = this.rows - 1
        this.saved = { x: 0, y: 0, graphics: false, origin: false }
    }

    private print(ch: string, cp: number) {
        let width = cellWidth(cp)
        if (this.graphics && cp >= 0x60 && cp <= 0x7e) {
            ch = DEC_GRAPHICS[ch] ?? ch
            width = 1
        }
        if (width === 0) {
            this.combine(ch)
            return
        }
        if (this.wrapNext && this.autowrap) {
            this.x = 0
            this.index()
        }
        this.wrapNext = false
        if (width === 2 && this.x === this.cols - 1) {
            // No room for both halves on this row.
            if (!this.autowrap) return
            this.screen[this.y][this.x] = ' '
            this.x = 0
            this.index()
        }
        if (this.insert) this.insertChars(width)
        const row = this.screen[this.y]
        this.unsplit(row, this.x)
        if (width === 2) this.unsplit(row, this.x + 1)
        row[this.x] = ch
        if (width === 2) row[this.x + 1] = ''
        this.last = ch
        if (this.x + width >= this.cols) {
            this.x = this.cols - 1
            this.wrapNext = this.autowrap
        } else {
            this.x += width
        }
    }

    /** A combining mark joins the character before the cursor. */
    private combine(mark: string) {
        const row = this.screen[this.y]
        let at = this.wrapNext ? this.x : this.x - 1
        if (at > 0 && row[at] === '') at--
        if (at >= 0) row[at] += mark
    }

    /** Overwriting half of a wide character blanks the other half. */
    private unsplit(row: Row, i: number) {
        if (row[i] === '' && i > 0) row[i - 1] = ' '
        if (row[i + 1] === '') row[i + 1] = ' '
    }

    private moveTo(x: number, y: number) {
        this.x = Math.max(0, Math.min(this.cols - 1, x))
        this.y = Math.max(0, Math.min(this.rows - 1, y))
        this.wrapNext = false
    }

    private saveCursor() {
        this.saved = { x: this.x, y: this.y, graphics: this.graphics, origin: this.origin }
    }

    private restoreCursor() {
        this.graphics = this.saved.graphics
        this.origin = this.saved.origin
        this.moveTo(this.saved.x, this.saved.y)
    }

    private setRegion(top: number, bottom: number) {
        if (top >= bottom || bottom > this.rows) return
        this.top = top - 1
        this.bottom = bottom - 1
        this.moveTo(0, this.origin ? this.top : 0)
    }

    private index() {
        if (this.y === this.bottom) this.scrollUp(1)
        else if (this.y < this.rows - 1) this.y++
    }

    private reverseIndex() {
        if (this.y === this.top) this.scrollDown(1)
        else if (this.y > 0) this.y--
    }

    private blankRows(count: number): Row[] {
        return Array.from({ length: count }, () => blankRow(this.cols))
    }

    private scrollUp(count: number) {
        count = Math.min(count, this.bottom - this.top + 1)
        const screen = this.screen
        const gone = screen.splice(this.top, count)
        screen.splice(this.bottom - count + 1, 0, ...this.blankRows(count))
        // What leaves the top row of the main screen is history, as in xterm, even with a
        // status line held below; a region that starts lower down is the program redrawing.
        if (!this.alt && this.top === 0) this.keep(gone)
    }

    private scrollDown(count: number) {
        count = Math.min(count, this.bottom - this.top + 1)
        const screen = this.screen
        screen.splice(this.bottom - count + 1, count)
        screen.splice(this.top, 0, ...this.blankRows(count))
    }

    private insertLines(count: number) {
        if (this.y < this.top || this.y > this.bottom) return
        count = Math.min(count, this.bottom - this.y + 1)
        const screen = this.screen
        screen.splice(this.bottom - count + 1, count)
        screen.splice(this.y, 0, ...this.blankRows(count))
        this.moveTo(0, this.y)
    }

    private deleteLines(count: number) {
        if (this.y < this.top || this.y > this.bottom) return
        count = Math.min(count, this.bottom - this.y + 1)
        const screen = this.screen
        screen.splice(this.y, count)
        screen.splice(this.bottom - count + 1, 0, ...this.blankRows(count))
        this.moveTo(0, this.y)
    }

    private insertChars(count: number) {
        const row = this.screen[this.y]
        this.unsplit(row, this.x)
        row.splice(this.x, 0, ...new Array<string>(Math.min(count, this.cols)).fill(' '))
        if (row[this.cols] === '') row[this.cols - 1] = ' '
        row.length = this.cols
    }

    private deleteChars(count: number) {
        const row = this.screen[this.y]
        count = Math.min(count, this.cols - this.x)
        this.unsplit(row, this.x)
        this.unsplit(row, this.x + count - 1)
        row.splice(this.x, count)
        while (row.length < this.cols) row.push(' ')
    }

    private eraseChars(count: number) {
        const row = this.screen[this.y]
        const end = Math.min(this.cols, this.x + count)
        this.unsplit(row, this.x)
        this.unsplit(row, end - 1)
        for (let i = this.x; i < end; i++) row[i] = ' '
    }

    private eraseLine(mode: number) {
        const row = this.screen[this.y]
        const [from, to] =
            mode === 0 ? [this.x, this.cols] : mode === 1 ? [0, this.x + 1] : [0, this.cols]
        this.unsplit(row, from)
        this.unsplit(row, to - 1)
        for (let i = from; i < to; i++) row[i] = ' '
    }

    private eraseDisplay(mode: number) {
        const screen = this.screen
        if (mode === 3) {
            this.history = []
            return
        }
        if (mode === 0) {
            this.eraseLine(0)
            for (let i = this.y + 1; i < this.rows; i++) screen[i] = blankRow(this.cols)
        } else if (mode === 1) {
            this.eraseLine(1)
            for (let i = 0; i < this.y; i++) screen[i] = blankRow(this.cols)
        } else if (mode === 2) {
            for (let i = 0; i < this.rows; i++) screen[i] = blankRow(this.cols)
        }
    }
}

/** UTF-8 to string without relying on `TextDecoder`, which not every JS engine ships. */
export const utf8Decode = (bytes: Uint8Array): string => {
    if (typeof TextDecoder !== 'undefined') {
        try {
            return new TextDecoder('utf-8').decode(bytes)
        } catch {}
    }
    let out = ''
    let i = 0
    while (i < bytes.length) {
        const b0 = bytes[i]
        if (b0 < 0x80) {
            out += String.fromCharCode(b0)
            i += 1
        } else if (b0 >= 0xc0 && b0 < 0xe0 && i + 1 < bytes.length) {
            out += String.fromCharCode(((b0 & 0x1f) << 6) | (bytes[i + 1] & 0x3f))
            i += 2
        } else if (b0 >= 0xe0 && b0 < 0xf0 && i + 2 < bytes.length) {
            out += String.fromCharCode(
                ((b0 & 0x0f) << 12) | ((bytes[i + 1] & 0x3f) << 6) | (bytes[i + 2] & 0x3f)
            )
            i += 3
        } else if (b0 >= 0xf0 && i + 3 < bytes.length) {
            const cp =
                ((b0 & 0x07) << 18) |
                ((bytes[i + 1] & 0x3f) << 12) |
                ((bytes[i + 2] & 0x3f) << 6) |
                (bytes[i + 3] & 0x3f)
            out += String.fromCodePoint(cp)
            i += 4
        } else {
            out += '\ufffd'
            i += 1
        }
    }
    return out
}

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

/** Base64 to bytes; `atob` is used when the runtime has it. */
export const base64Decode = (b64: string): Uint8Array => {
    if (typeof globalThis.atob === 'function') {
        const binary = globalThis.atob(b64)
        const bytes = new Uint8Array(binary.length)
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
        return bytes
    }
    const clean = b64.replace(/[^A-Za-z0-9+/]/g, '')
    const out: number[] = []
    let buffer = 0
    let bits = 0
    for (const ch of clean) {
        buffer = (buffer << 6) | B64.indexOf(ch)
        bits += 6
        if (bits >= 8) {
            bits -= 8
            out.push((buffer >> bits) & 0xff)
        }
    }
    return Uint8Array.from(out)
}

/**
 * UTF-8 across PTY frames. The engine cuts a frame wherever its buffer ends, which can be in
 * the middle of a character: the bytes of an unfinished character wait for the next frame
 * instead of turning into `\ufffd`. A stream that starts in the middle of a character (a
 * catch-up from partway through the engine's buffer) drops the stray continuation bytes.
 * One per attachment, reset when the session's epoch changes.
 */
export class Utf8Stream {
    private carry = new Uint8Array(0)
    private fresh = true

    reset() {
        this.carry = new Uint8Array(0)
        this.fresh = true
    }

    decode(chunk: Uint8Array): string {
        let bytes = chunk
        if (this.carry.length > 0) {
            bytes = new Uint8Array(this.carry.length + chunk.length)
            bytes.set(this.carry)
            bytes.set(chunk, this.carry.length)
        }
        let start = 0
        if (this.fresh) {
            // A character is at most four bytes, so at most three can belong to one begun
            // before the stream did.
            while (start < bytes.length && start < 3 && (bytes[start] & 0xc0) === 0x80) start++
            if (start < bytes.length || start === 3) this.fresh = false
        }
        // Hold back a lead byte whose continuation bytes have not all arrived.
        let end = bytes.length
        for (let back = 1; back <= 3 && back <= end - start; back++) {
            const b = bytes[end - back]
            if ((b & 0xc0) === 0x80) continue
            const need = b >= 0xf0 ? 4 : b >= 0xe0 ? 3 : b >= 0xc0 ? 2 : 1
            if (need > back) end -= back
            break
        }
        this.carry = bytes.slice(end)
        return start < end ? utf8Decode(bytes.subarray(start, end)) : ''
    }
}
