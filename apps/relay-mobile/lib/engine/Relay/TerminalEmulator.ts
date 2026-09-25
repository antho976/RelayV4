import './xtermEnv'

import { IBufferCell, IBufferLine, Terminal } from '@xterm/headless'

/**
 * A session's PTY as a screen, the way the desktop draws it. Agent CLIs (Claude Code, Codex)
 * redraw in place: cursor addressing, erase, scroll regions, the alternate screen. A list of
 * lines cannot follow that, so the bytes go through a real terminal emulator (xterm.js,
 * headless: its parser and buffers, no DOM) and the phone draws the emulator's rows.
 *
 * The emulator must have the PTY's size, or wrapped lines and cursor-up redraws land in the
 * wrong rows. The engine does not report it (`session.get` has no cols/rows), so it starts at
 * the engine's spawn size, 120x40, and grows when the stream proves the PTY is larger: a
 * cursor addressed past the edge, a scroll region's bottom, a full-width rule. Growing wider
 * than the real PTY only leaves blank columns; narrower would break every redraw. It never
 * shrinks, and the phone never resizes the PTY: the desktop owns its size.
 */

export const DEFAULT_COLS = 120
export const DEFAULT_ROWS = 40
/** Rows kept above the screen. */
export const TERMINAL_SCROLLBACK = 3000
/** Sizes past these are cursor probes (`CSI 999;999 H`), not evidence of a real screen. */
const MAX_COLS = 400
const MAX_ROWS = 200

/** One run of cells drawn alike. Colours are CSS strings; absent means the default. */
export type TermSpan = {
    text: string
    fg?: string
    bg?: string
    bold?: boolean
    dim?: boolean
    italic?: boolean
    underline?: boolean
    strike?: boolean
}

export type TermRow = {
    /** Plain text of the row, right-trimmed. */
    text: string
    spans: TermSpan[]
    /** Equal for two rows that draw the same; a changed row is a new object. */
    sig: string
}

export type TermSnapshot = {
    rows: TermRow[]
    cols: number
    /** The program is on the alternate screen (a full-screen UI with no history of its own). */
    alt: boolean
}

/** The sixteen ANSI colours, tuned for the console's near-black ink. */
export const ANSI16 = [
    '#1d1d1f',
    '#e5534b',
    '#57ab5a',
    '#c69026',
    '#539bf5',
    '#b083f0',
    '#39c5cf',
    '#c9c9c4',
    '#6e6e6a',
    '#ff7b72',
    '#7ee787',
    '#e3b341',
    '#79c0ff',
    '#d2a8ff',
    '#56d4dd',
    '#f5f5f2',
]

const hex2 = (value: number) => value.toString(16).padStart(2, '0')
const rgb = (value: number) =>
    `#${hex2((value >> 16) & 255)}${hex2((value >> 8) & 255)}${hex2(value & 255)}`

/** xterm's 256-colour table past the first sixteen: a 6x6x6 cube, then 24 greys. */
const palette256 = (index: number): string => {
    if (index < 16) return ANSI16[index]
    if (index < 232) {
        const n = index - 16
        const level = (v: number) => (v === 0 ? 0 : 55 + v * 40)
        return `#${hex2(level(Math.floor(n / 36)))}${hex2(level(Math.floor(n / 6) % 6))}${hex2(level(n % 6))}`
    }
    const grey = 8 + (index - 232) * 10
    return `#${hex2(grey)}${hex2(grey)}${hex2(grey)}`
}

const fgOf = (cell: IBufferCell): string | undefined =>
    cell.isFgRGB()
        ? rgb(cell.getFgColor())
        : cell.isFgPalette()
          ? palette256(cell.getFgColor())
          : undefined
const bgOf = (cell: IBufferCell): string | undefined =>
    cell.isBgRGB()
        ? rgb(cell.getBgColor())
        : cell.isBgPalette()
          ? palette256(cell.getBgColor())
          : undefined

/** Size evidence in a chunk of output. */
const CUP = /\x1b\[(\d+);(\d+)[Hf]/g
const MARGINS = /\x1b\[\d+;(\d+)r/g
const CHA = /\x1b\[(\d+)[G`]/g
const VPA = /\x1b\[(\d+)d/g
/** Box drawing that full-width rules and boxes are made of; a run of 40 or more is a rule. */
const RULE_CHARS = new Set([...'─━═╭╮╰╯┌┐└┘├┤╔╗╚╝'])
const RULE_MIN = 40

export class TerminalEmulator {
    private term: Terminal
    private cols: number
    private rows: number
    private cell: IBufferCell | undefined
    /** Rows of the normal buffer above its screen, oldest first; `history.length === baseY`. */
    private history: TermRow[] = []
    /** Lines xterm dropped off the top of the normal buffer since the last snapshot. */
    private trimmed = 0
    /** Set when history can no longer be patched (resize, reset, an untracked trim). */
    private stale = true
    private screen: TermRow[] = []
    private trimHook?: { dispose(): void }
    private listeners = new Set<() => void>()
    /** The end of the previous write, so a sequence cut between two frames is still seen. */
    private tail = ''
    /** Length of the box-drawing run the previous write ended in. */
    private ruleRun = 0

    constructor(cols = DEFAULT_COLS, rows = DEFAULT_ROWS) {
        this.cols = cols
        this.rows = rows
        this.term = this.create()
    }

    private create(): Terminal {
        const term = new Terminal({
            cols: this.cols,
            rows: this.rows,
            scrollback: TERMINAL_SCROLLBACK,
            allowProposedApi: true,
        })
        term.onWriteParsed(() => {
            for (const listener of this.listeners) listener()
        })
        this.cell = undefined
        this.hookTrim(term)
        return term
    }

    /**
     * xterm reuses the oldest line object for the newest once the scrollback is full, and says
     * so only through its buffer's internal trim event. Counting trims keeps the cached history
     * rows aligned; without the hook (another xterm build), a full buffer rebuilds instead.
     */
    private hookTrim(term: Terminal) {
        this.trimHook?.dispose()
        this.trimHook = undefined
        try {
            const lines = (term as any)._core?.buffers?.normal?.lines
            if (lines && typeof lines.onTrim === 'function') {
                this.trimHook = lines.onTrim((amount: number) => {
                    this.trimmed += amount
                })
            }
        } catch {}
        this.stale = true
    }

    /** Called after each parsed write; the screen throttles its own redraws. */
    onChange(listener: () => void): () => void {
        this.listeners.add(listener)
        return () => this.listeners.delete(listener)
    }

    /** The program asked for bracketed paste (`CSI ? 2004 h`), as agent CLIs and shells do. */
    get bracketedPaste(): boolean {
        return this.term.modes.bracketedPasteMode
    }

    get size() {
        return { cols: this.cols, rows: this.rows }
    }

    /** Output from the PTY, as text (the client already joins UTF-8 across frames). */
    write(data: string, done?: () => void) {
        if (!data) {
            if (done) this.term.write('', done)
            return
        }
        const [cols, rows] = this.evidence(data)
        if (cols > this.cols || rows > this.rows) {
            const next = { cols: Math.max(cols, this.cols), rows: Math.max(rows, this.rows) }
            // Writes are parsed asynchronously: resize in order, after what came before.
            this.term.write('', () => this.resize(next.cols, next.rows))
            this.cols = next.cols
            this.rows = next.rows
        }
        this.term.write(data, done)
    }

    private resize(cols: number, rows: number) {
        this.term.resize(cols, rows)
        this.hookTrim(this.term)
    }

    private evidence(data: string): [number, number] {
        let cols = 0
        let rows = 0
        const col = (value: number) => {
            if (value <= MAX_COLS && value > cols) cols = value
        }
        const row = (value: number) => {
            if (value <= MAX_ROWS && value > rows) rows = value
        }
        const scan = this.tail + data
        this.tail = data.slice(-24)
        if (scan.indexOf('\x1b[') !== -1) {
            for (const m of scan.matchAll(CUP)) {
                row(Number(m[1]))
                col(Number(m[2]))
            }
            for (const m of scan.matchAll(MARGINS)) row(Number(m[1]))
            for (const m of scan.matchAll(CHA)) col(Number(m[1]))
            for (const m of scan.matchAll(VPA)) row(Number(m[1]))
        }
        // A rule is as wide as the screen, and often longer than one frame.
        if (this.ruleRun > 0 || /[─━═]/.test(data)) {
            let run = this.ruleRun
            for (const ch of data) {
                if (RULE_CHARS.has(ch)) {
                    run++
                    continue
                }
                if (run >= RULE_MIN) col(run)
                run = 0
            }
            if (run >= RULE_MIN) col(run)
            this.ruleRun = run
        }
        return [cols, rows]
    }

    /** A new process (a new epoch): forget everything, keep the size learned so far. */
    reset() {
        this.trimHook?.dispose()
        this.term.dispose()
        this.term = this.create()
        this.history = []
        this.screen = []
        this.trimmed = 0
        this.stale = true
        this.tail = ''
        this.ruleRun = 0
        for (const listener of this.listeners) listener()
    }

    dispose() {
        this.trimHook?.dispose()
        this.listeners.clear()
        this.term.dispose()
    }

    /**
     * The rows to draw: history, then the screen. Rows that did not change keep their object,
     * so a list can skip them. Only the screen is re-read from cells on every call; history
     * rows are read once, when they scroll off the top.
     */
    snapshot(): TermSnapshot {
        const buffer = this.term.buffer.active
        const alt = buffer.type === 'alternate'
        const normal = this.term.buffer.normal
        this.syncHistory(normal)
        const base = buffer.baseY
        const screen: TermRow[] = []
        // A program that draws its own cursor (Claude Code, Codex) hides the terminal's.
        const cursorShown = !this.cursorHidden()
        for (let y = 0; y < this.rows; y++) {
            const line = buffer.getLine(base + y)
            const cursor =
                cursorShown && y === buffer.cursorY ? Math.min(buffer.cursorX, this.cols - 1) : -1
            const row = line ? this.readRow(line, cursor) : EMPTY_ROW
            const before = this.screen[y]
            screen.push(before && before.sig === row.sig ? before : row)
        }
        this.screen = screen
        let shown = screen
        if (!alt) {
            // A shell that printed three lines is three rows, not three and thirty-seven blanks.
            let last = screen.length - 1
            while (
                last > buffer.cursorY &&
                screen[last].text === '' &&
                screen[last].spans.every((s) => !s.bg)
            )
                last--
            shown = screen.slice(0, last + 1)
        }
        return { rows: alt ? shown : this.history.concat(shown), cols: this.cols, alt: alt }
    }

    private syncHistory(normal: Terminal['buffer']['normal']) {
        const base = normal.baseY
        const full = normal.length >= TERMINAL_SCROLLBACK + this.rows
        if (this.stale || (!this.trimHook && full)) {
            this.history = []
            this.stale = false
            this.trimmed = 0
        } else if (this.trimmed > 0) {
            this.history.splice(0, this.trimmed)
            this.trimmed = 0
        }
        // Lines pulled back into the screen (a taller resize) or erased with the scrollback.
        if (this.history.length > base) this.history.length = base
        for (let y = this.history.length; y < base; y++) {
            const line = normal.getLine(y)
            this.history.push(line ? this.readRow(line, -1) : EMPTY_ROW)
        }
    }

    private cursorHidden(): boolean {
        try {
            return !!(this.term as any)._core?.coreService?.isCursorHidden
        } catch {
            return true
        }
    }

    private readRow(line: IBufferLine, cursor: number): TermRow {
        const cell = (this.cell = line.getCell(0, this.cell) ?? this.cell)
        if (!cell) return EMPTY_ROW
        // Right-trim: blank default-background cells past the last glyph draw nothing.
        let end = line.length
        while (end > 0 && end - 1 !== cursor) {
            line.getCell(end - 1, cell)
            const chars = cell.getChars()
            if ((chars !== '' && chars !== ' ') || !cell.isBgDefault() || cell.isInverse()) break
            end--
        }
        const spans: TermSpan[] = []
        let sig = ''
        let text = ''
        let run: TermSpan | undefined
        let runKey = ''
        for (let x = 0; x < end; x++) {
            line.getCell(x, cell)
            const width = cell.getWidth()
            if (width === 0) continue
            let chars = cell.getChars() || ' '
            if (cell.isInvisible()) chars = ' '.repeat(width)
            let fg = fgOf(cell)
            let bg = bgOf(cell)
            if ((cell.isInverse() !== 0) !== (x === cursor)) {
                const swap = fg
                fg = bg ?? INK
                bg = swap ?? PAPER
            }
            const bold = cell.isBold() !== 0
            const dim = cell.isDim() !== 0
            const italic = cell.isItalic() !== 0
            const underline = cell.isUnderline() !== 0
            const strike = cell.isStrikethrough() !== 0
            const key = `${fg ?? ''}|${bg ?? ''}|${+bold}${+dim}${+italic}${+underline}${+strike}`
            if (!run || key !== runKey) {
                run = { text: '' }
                if (fg) run.fg = fg
                if (bg) run.bg = bg
                if (bold) run.bold = true
                if (dim) run.dim = true
                if (italic) run.italic = true
                if (underline) run.underline = true
                if (strike) run.strike = true
                spans.push(run)
                runKey = key
                sig += `\x00${key}\x01`
            }
            run.text += chars
            sig += chars
            text += chars
        }
        return { text: text.trimEnd(), spans: spans, sig: sig }
    }

    /** What the screen says, as plain text: history and screen, wrapped lines joined. */
    plainText(maxLines = 800): string {
        const out: string[] = []
        const read = (buffer: Terminal['buffer']['normal'], from: number, to: number) => {
            for (let y = from; y < to; y++) {
                const line = buffer.getLine(y)
                if (!line) continue
                const text = line.translateToString(true)
                if (line.isWrapped && out.length > 0) out[out.length - 1] += text
                else out.push(text)
            }
        }
        const normal = this.term.buffer.normal
        read(normal, 0, normal.length)
        const active = this.term.buffer.active
        if (active.type === 'alternate') read(active, 0, active.length)
        while (out.length > 0 && out[out.length - 1].trim() === '') out.pop()
        return out.slice(-maxLines).join('\n')
    }
}

/** The console's ink and paper, for inverse video over default colours. */
const INK = '#0a0a0b'
const PAPER = '#ececea'
const EMPTY_ROW: TermRow = { text: '', spans: [], sig: '' }
