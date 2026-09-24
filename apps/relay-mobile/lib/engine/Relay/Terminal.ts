/**
 * A plain-text view of a PTY stream. Not a terminal emulator: escape sequences are removed,
 * carriage returns overwrite the current line, backspaces erase, and the screen is a bounded
 * list of lines. That is enough to read what an agent prints and to answer it; a full
 * emulator on a phone screen would be neither readable nor worth its weight here.
 */

/** Longest a screen keeps; older lines fall off the top. */
export const TERMINAL_MAX_LINES = 1500

const ESC = '\x1b'

/**
 * Longest escape sequence kept waiting for its end. An OSC or DCS that never terminates
 * (a program killed mid-sequence) would otherwise hold the screen and grow without bound.
 */
const MAX_ESCAPE = 4096

export class TerminalText {
    private lines: string[] = ['']
    private cursor = 0
    /** Bytes of an escape sequence that arrived split across two frames. */
    private pending = ''
    /** Inside an over-long OSC (`osc`, ends at BEL or ST) or DCS/PM/APC (`st`, ends at ST). */
    private swallow?: 'osc' | 'st'

    constructor(private readonly maxLines = TERMINAL_MAX_LINES) {}

    /** Start over from a block of scrollback text. */
    reset(text: string) {
        this.lines = ['']
        this.cursor = 0
        this.pending = ''
        this.swallow = undefined
        this.feed(text)
    }

    feed(chunk: string) {
        const data = this.pending + chunk
        this.pending = ''
        let i = 0
        if (this.swallow) {
            // The rest of an abandoned sequence is dropped up to its terminator.
            const end = this.swallowEnd(data)
            if (end === -1) {
                // An ESC at the very end may be the first half of ST.
                if (data.endsWith(ESC)) this.pending = ESC
                return
            }
            this.swallow = undefined
            i = end
        }
        while (i < data.length) {
            const ch = data[i]
            if (ch === ESC) {
                const consumed = this.skipEscape(data, i)
                if (consumed === -1) {
                    if (data.length - i > MAX_ESCAPE) {
                        // Too long to be real: drop what came so far, and for a string
                        // sequence the rest of it too, instead of waiting on it forever.
                        const kind = data[i + 1]
                        if (kind === ']') this.swallow = 'osc'
                        else if (kind === 'P' || kind === '^' || kind === '_') this.swallow = 'st'
                        return
                    }
                    // Sequence continues in the next frame.
                    this.pending = data.slice(i)
                    return
                }
                i += consumed
                continue
            }
            if (ch === '\r') {
                this.cursor = 0
                i++
                continue
            }
            if (ch === '\n') {
                this.lines.push('')
                this.cursor = 0
                if (this.lines.length > this.maxLines) {
                    this.lines.splice(0, this.lines.length - this.maxLines)
                }
                i++
                continue
            }
            if (ch === '\b') {
                if (this.cursor > 0) this.cursor--
                i++
                continue
            }
            if (ch === '\x07' || ch === '\x00' || ch === '\x0f' || ch === '\x0e') {
                i++
                continue
            }
            if (ch === '\t') {
                this.put(' '.repeat(8 - (this.cursor % 8)))
                i++
                continue
            }
            // Fast path: copy a run of ordinary characters at once.
            let end = i + 1
            while (end < data.length) {
                const c = data[end]
                if (c === ESC || c === '\r' || c === '\n' || c === '\b' || c === '\t' || c < ' ')
                    break
                end++
            }
            this.put(data.slice(i, end))
            i = end
        }
    }

    /** The screen, top to bottom. */
    text(): string {
        return this.lines.join('\n')
    }

    private put(run: string) {
        const line = this.lines[this.lines.length - 1]
        if (this.cursor >= line.length) {
            this.lines[this.lines.length - 1] = line + run
        } else {
            this.lines[this.lines.length - 1] =
                line.slice(0, this.cursor) + run + line.slice(this.cursor + run.length)
        }
        this.cursor += run.length
    }

    /** Index just past the terminator of the sequence being swallowed, or -1. */
    private swallowEnd(data: string): number {
        for (let j = 0; j < data.length; j++) {
            if (this.swallow === 'osc' && data[j] === '\x07') return j + 1
            if (data[j] === ESC && data[j + 1] === '\\') return j + 2
        }
        return -1
    }

    /**
     * Length of the escape sequence starting at `i`, or -1 if the buffer ends inside one.
     * Covers CSI (`ESC [ … final`), OSC (`ESC ] … BEL|ST`), and two-byte escapes.
     */
    private skipEscape(data: string, i: number): number {
        if (i + 1 >= data.length) return -1
        const kind = data[i + 1]
        if (kind === '[') {
            let j = i + 2
            while (j < data.length) {
                const code = data.charCodeAt(j)
                if (code >= 0x40 && code <= 0x7e) {
                    // Erase-in-line clears to the end of the current line; everything else is
                    // cursor movement or colour, which the plain view has no use for.
                    if (data[j] === 'K') {
                        const line = this.lines[this.lines.length - 1]
                        this.lines[this.lines.length - 1] = line.slice(0, this.cursor)
                    } else if (data[j] === 'J' && data.slice(i + 2, j) === '2') {
                        this.lines = ['']
                        this.cursor = 0
                    }
                    return j - i + 1
                }
                j++
            }
            return -1
        }
        if (kind === ']') {
            let j = i + 2
            while (j < data.length) {
                if (data[j] === '\x07') return j - i + 1
                if (data[j] === ESC && data[j + 1] === '\\') return j - i + 2
                j++
            }
            return -1
        }
        if (kind === 'P' || kind === '^' || kind === '_') {
            // DCS / PM / APC end with ST.
            let j = i + 2
            while (j < data.length) {
                if (data[j] === ESC && data[j + 1] === '\\') return j - i + 2
                j++
            }
            return -1
        }
        if (kind === '(' || kind === ')' || kind === '#' || kind === '%') {
            return i + 2 < data.length ? 3 : -1
        }
        return 2
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
