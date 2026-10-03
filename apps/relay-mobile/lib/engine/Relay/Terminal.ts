/**
 * Bytes of a PTY stream: base64 frames to bytes, and UTF-8 across frame boundaries. The
 * screen itself is `TerminalEmulator`.
 */

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
