/**
 * PNG text-chunk reading and rewriting, in plain TypeScript.
 *
 * Character cards carry their JSON in a PNG text chunk (`chara`, `ccv3`, ...), usually base64
 * encoded. This module reads those chunks and writes replacements without touching pixel data:
 * the image chunks are copied through byte for byte and only the chunk list around them changes.
 *
 * Nothing here depends on Node's `Buffer` or on `atob`/`btoa`, so it runs unchanged on Hermes.
 * The file has no imports, which also lets Node run it directly for tests.
 */

// ---------------------------------------------------------------------------------------------
// Errors

export type PngErrorCode =
    | 'BAD_BASE64'
    | 'NOT_PNG'
    | 'TRUNCATED'
    | 'BAD_CRC'
    | 'NO_IEND'
    | 'BAD_KEYWORD'
    | 'BAD_TEXT'

export class PngError extends Error {
    readonly code: PngErrorCode
    constructor(code: PngErrorCode, message: string) {
        super(message)
        this.name = 'PngError'
        this.code = code
    }
}

export const isPngError = (e: unknown, code?: PngErrorCode): e is PngError =>
    e instanceof PngError && (code === undefined || e.code === code)

// ---------------------------------------------------------------------------------------------
// Base64 (RFC 4648, standard alphabet)

const B64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
const B64_CODES = Uint8Array.from(B64_ALPHABET, (c) => c.charCodeAt(0))
const B64_LOOKUP = (() => {
    const table = new Int16Array(128).fill(-1)
    for (let i = 0; i < B64_ALPHABET.length; i++) table[B64_ALPHABET.charCodeAt(i)] = i
    // accept the URL-safe alphabet on input as well
    table['-'.charCodeAt(0)] = 62
    table['_'.charCodeAt(0)] = 63
    return table
})()

/**
 * Decodes base64, ignoring ASCII whitespace. Padding is optional, but a malformed length, a
 * character outside the alphabet, or data after the padding is an error.
 */
export const base64ToBytes = (input: string): Uint8Array => {
    const values = new Uint8Array(input.length)
    let count = 0
    let padding = 0
    for (let i = 0; i < input.length; i++) {
        const c = input.charCodeAt(i)
        if (c === 0x20 || c === 0x0a || c === 0x0d || c === 0x09) continue
        if (c === 0x3d /* = */) {
            padding++
            continue
        }
        const v = c < 128 ? B64_LOOKUP[c] : -1
        if (v < 0 || padding > 0)
            throw new PngError('BAD_BASE64', `Invalid base64 character at offset ${i}`)
        values[count++] = v
    }
    const rem = count % 4
    if (rem === 1 || padding > 2 || (padding > 0 && (count + padding) % 4 !== 0))
        throw new PngError('BAD_BASE64', 'Invalid base64 length')

    const out = new Uint8Array(Math.floor(count / 4) * 3 + (rem === 0 ? 0 : rem - 1))
    let o = 0
    let i = 0
    for (; i + 4 <= count; i += 4) {
        const n = (values[i] << 18) | (values[i + 1] << 12) | (values[i + 2] << 6) | values[i + 3]
        out[o++] = n >> 16
        out[o++] = (n >> 8) & 0xff
        out[o++] = n & 0xff
    }
    if (rem >= 2) {
        const n = (values[i] << 18) | (values[i + 1] << 12) | ((rem === 3 ? values[i + 2] : 0) << 6)
        out[o++] = n >> 16
        if (rem === 3) out[o++] = (n >> 8) & 0xff
    }
    return out
}

export const bytesToBase64 = (bytes: Uint8Array): string => {
    // write ASCII codes into a byte array, then turn that into a string in slices
    const out = new Uint8Array(Math.ceil(bytes.length / 3) * 4)
    let o = 0
    let i = 0
    for (; i + 3 <= bytes.length; i += 3) {
        const n = (bytes[i] << 16) | (bytes[i + 1] << 8) | bytes[i + 2]
        out[o++] = B64_CODES[n >> 18]
        out[o++] = B64_CODES[(n >> 12) & 63]
        out[o++] = B64_CODES[(n >> 6) & 63]
        out[o++] = B64_CODES[n & 63]
    }
    if (i < bytes.length) {
        const two = i + 1 < bytes.length
        const n = (bytes[i] << 16) | (two ? bytes[i + 1] << 8 : 0)
        out[o++] = B64_CODES[n >> 18]
        out[o++] = B64_CODES[(n >> 12) & 63]
        out[o++] = two ? B64_CODES[(n >> 6) & 63] : 0x3d
        out[o++] = 0x3d
    }
    return latin1Decode(out)
}

// ---------------------------------------------------------------------------------------------
// Text encodings

/** UTF-8 encode, turning lone surrogates into U+FFFD as TextEncoder does. */
export const utf8Encode = (text: string): Uint8Array => {
    const out = new Uint8Array(text.length * 3)
    let o = 0
    for (let i = 0; i < text.length; i++) {
        let cp = text.charCodeAt(i)
        if (cp >= 0xd800 && cp <= 0xdbff && i + 1 < text.length) {
            const lo = text.charCodeAt(i + 1)
            if (lo >= 0xdc00 && lo <= 0xdfff) {
                cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00)
                i++
            }
        }
        if (cp >= 0xd800 && cp <= 0xdfff) cp = 0xfffd
        if (cp < 0x80) out[o++] = cp
        else if (cp < 0x800) {
            out[o++] = 0xc0 | (cp >> 6)
            out[o++] = 0x80 | (cp & 63)
        } else if (cp < 0x10000) {
            out[o++] = 0xe0 | (cp >> 12)
            out[o++] = 0x80 | ((cp >> 6) & 63)
            out[o++] = 0x80 | (cp & 63)
        } else {
            out[o++] = 0xf0 | (cp >> 18)
            out[o++] = 0x80 | ((cp >> 12) & 63)
            out[o++] = 0x80 | ((cp >> 6) & 63)
            out[o++] = 0x80 | (cp & 63)
        }
    }
    return out.subarray(0, o)
}

/** Strict UTF-8 decode: returns `undefined` for malformed input instead of guessing. */
export const utf8Decode = (bytes: Uint8Array): string | undefined => {
    const units: number[] = []
    const parts: string[] = []
    const flush = () => {
        parts.push(String.fromCharCode.apply(null, units))
        units.length = 0
    }
    let i = 0
    while (i < bytes.length) {
        const b = bytes[i]
        let cp: number
        let need: number
        let min: number
        if (b < 0x80) {
            cp = b
            need = 0
            min = 0
        } else if (b >= 0xc2 && b <= 0xdf) {
            cp = b & 0x1f
            need = 1
            min = 0x80
        } else if (b >= 0xe0 && b <= 0xef) {
            cp = b & 0x0f
            need = 2
            min = 0x800
        } else if (b >= 0xf0 && b <= 0xf4) {
            cp = b & 0x07
            need = 3
            min = 0x10000
        } else return undefined
        if (i + need >= bytes.length) return undefined
        for (let k = 1; k <= need; k++) {
            const c = bytes[i + k]
            if ((c & 0xc0) !== 0x80) return undefined
            cp = (cp << 6) | (c & 0x3f)
        }
        if (cp < min || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) return undefined
        if (cp >= 0x10000) {
            cp -= 0x10000
            units.push(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff))
        } else units.push(cp)
        if (units.length >= 8192) flush()
        i += need + 1
    }
    flush()
    return parts.join('')
}

const latin1Decode = (bytes: Uint8Array): string => {
    const parts: string[] = []
    for (let i = 0; i < bytes.length; i += 8192)
        // apply() takes any array-like, typed arrays included
        parts.push(
            String.fromCharCode.apply(null, bytes.subarray(i, i + 8192) as unknown as number[])
        )
    return parts.join('')
}

/** Returns `undefined` when the text has a character that Latin-1 cannot hold. */
const latin1Encode = (text: string): Uint8Array | undefined => {
    const out = new Uint8Array(text.length)
    for (let i = 0; i < text.length; i++) {
        const c = text.charCodeAt(i)
        if (c > 0xff) return undefined
        out[i] = c
    }
    return out
}

// ---------------------------------------------------------------------------------------------
// CRC-32 (ISO 3309 / PNG)

const CRC_TABLE = (() => {
    const table = new Uint32Array(256)
    for (let n = 0; n < 256; n++) {
        let c = n
        for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
        table[n] = c >>> 0
    }
    return table
})()

const crcUpdate = (crc: number, bytes: Uint8Array): number => {
    let c = crc
    for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8)
    return c
}

/** CRC-32 of the chunk type followed by its data, as stored after every PNG chunk. */
export const chunkCrc = (type: string, data: Uint8Array): number =>
    (crcUpdate(crcUpdate(0xffffffff, typeBytes(type)), data) ^ 0xffffffff) >>> 0

const typeBytes = (type: string) =>
    new Uint8Array([type.charCodeAt(0), type.charCodeAt(1), type.charCodeAt(2), type.charCodeAt(3)])

// ---------------------------------------------------------------------------------------------
// Chunk parsing and building

const SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]

export type PngChunk = {
    type: string
    /** A view into the source bytes; copy it before mutating. */
    data: Uint8Array
}

export type CrcPolicy =
    /** Any chunk with a wrong CRC is an error (default). */
    | 'strict'
    /**
     * libpng's rule: a damaged ancillary chunk (text, metadata) is dropped, a damaged critical
     * chunk (IHDR, PLTE, IDAT, IEND) is still an error.
     */
    | 'drop-ancillary'
    /** Do not check CRCs at all. */
    | 'ignore'

export type ParseOptions = { crc?: CrcPolicy }

const toBytes = (png: string | Uint8Array): Uint8Array =>
    typeof png === 'string' ? base64ToBytes(png) : png

export const isPng = (bytes: Uint8Array): boolean =>
    bytes.length >= 8 && SIGNATURE.every((b, i) => bytes[i] === b)

const readU32 = (b: Uint8Array, at: number) =>
    ((b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3]) >>> 0

const isAncillary = (type: string) => (type.charCodeAt(0) & 0x20) !== 0

/**
 * Splits a PNG into its chunks, up to and including IEND. Anything after IEND is ignored.
 * Throws `PngError` for a bad signature, a truncated chunk, a missing IEND, or (by policy) a CRC
 * mismatch.
 */
export const parsePngChunks = (
    png: string | Uint8Array,
    options: ParseOptions = {}
): PngChunk[] => {
    const bytes = toBytes(png)
    const policy = options.crc ?? 'strict'
    if (!isPng(bytes)) throw new PngError('NOT_PNG', 'Data is not a PNG image')

    const chunks: PngChunk[] = []
    let at = 8
    while (at < bytes.length) {
        if (at + 12 > bytes.length)
            throw new PngError('TRUNCATED', `Truncated chunk header at ${at}`)
        const length = readU32(bytes, at)
        const type = String.fromCharCode(bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7])
        if (!/^[A-Za-z]{4}$/.test(type))
            throw new PngError('TRUNCATED', `Invalid chunk type at ${at}`)
        const dataStart = at + 8
        const dataEnd = dataStart + length
        if (length > 0x7fffffff || dataEnd + 4 > bytes.length)
            throw new PngError('TRUNCATED', `Chunk ${type} at ${at} runs past the end of the data`)
        const data = bytes.subarray(dataStart, dataEnd)
        at = dataEnd + 4

        if (policy !== 'ignore' && readU32(bytes, dataEnd) !== chunkCrc(type, data)) {
            if (policy === 'drop-ancillary' && isAncillary(type)) continue
            throw new PngError('BAD_CRC', `CRC mismatch in ${type} chunk at ${dataStart - 8}`)
        }
        chunks.push({ type, data })
        if (type === 'IEND') return chunks
    }
    throw new PngError('NO_IEND', 'PNG has no IEND chunk')
}

/** Serializes chunks back into a PNG, computing every length and CRC. */
export const buildPng = (chunks: PngChunk[]): Uint8Array => {
    let size = 8
    for (const c of chunks) size += 12 + c.data.length
    const out = new Uint8Array(size)
    out.set(SIGNATURE, 0)
    let at = 8
    const writeU32 = (v: number) => {
        out[at++] = v >>> 24
        out[at++] = (v >>> 16) & 0xff
        out[at++] = (v >>> 8) & 0xff
        out[at++] = v & 0xff
    }
    for (const c of chunks) {
        writeU32(c.data.length)
        out.set(typeBytes(c.type), at)
        at += 4
        out.set(c.data, at)
        at += c.data.length
        writeU32(chunkCrc(c.type, c.data))
    }
    return out
}

// ---------------------------------------------------------------------------------------------
// Text chunks

export type PngTextType = 'tEXt' | 'zTXt' | 'iTXt'

export type PngText = {
    keyword: string
    /** Decoded text (see `ReadTextOptions.decodeBase64`). */
    text: string
    type: PngTextType
    /** True when `text` is the base64-decoded form of what the chunk holds. */
    base64Decoded: boolean
}

export type ReadTextOptions = ParseOptions & {
    /** Only return chunks whose keyword is in this list. All text chunks when omitted. */
    keywords?: readonly string[]
    /**
     * Decode each value as base64 and then UTF-8 (default `true`, as character cards store
     * their JSON this way). A value that is not valid base64 of valid UTF-8 is returned as is.
     */
    decodeBase64?: boolean
    /**
     * Decompressor for zTXt and compressed iTXt chunks (zlib stream in, bytes out). Without it
     * those chunks are skipped, because there is no inflate in React Native.
     */
    inflate?: (data: Uint8Array) => Uint8Array
}

const indexOfZero = (data: Uint8Array, from: number) => {
    for (let i = from; i < data.length; i++) if (data[i] === 0) return i
    return -1
}

/** Latin-1 as the spec says, but many writers put UTF-8 in tEXt, so prefer that when valid. */
const decodeLegacyText = (bytes: Uint8Array) => utf8Decode(bytes) ?? latin1Decode(bytes)

type RawText = { keyword: string; type: PngTextType; value: string }

const decodeTextChunk = (
    chunk: PngChunk,
    inflate: ReadTextOptions['inflate']
): RawText | undefined => {
    const { type, data } = chunk
    if (type !== 'tEXt' && type !== 'zTXt' && type !== 'iTXt') return
    const nul = indexOfZero(data, 0)
    if (nul <= 0) return
    const keyword = latin1Decode(data.subarray(0, nul))

    if (type === 'tEXt') {
        const value = decodeLegacyText(data.subarray(nul + 1))
        return { keyword, type, value }
    }

    if (type === 'zTXt') {
        // keyword \0 method(1) zlib-stream
        if (!inflate || data[nul + 1] !== 0) return
        const value = decodeLegacyText(inflate(data.subarray(nul + 2)))
        return { keyword, type, value }
    }

    // iTXt: keyword \0 flag(1) method(1) language \0 translated-keyword \0 text(UTF-8)
    const compressed = data[nul + 1] === 1
    const langEnd = indexOfZero(data, nul + 3)
    if (langEnd < 0) return
    const transEnd = indexOfZero(data, langEnd + 1)
    if (transEnd < 0) return
    let body = data.subarray(transEnd + 1)
    if (compressed) {
        if (!inflate || data[nul + 2] !== 0) return
        body = inflate(body)
    }
    const value = utf8Decode(body)
    return value === undefined ? undefined : { keyword, type, value }
}

const tryBase64Utf8 = (value: string): string | undefined => {
    try {
        return utf8Decode(base64ToBytes(value))
    } catch {
        return undefined
    }
}

/**
 * Reads the text chunks of a PNG, in file order.
 *
 * @param png base64 string or raw bytes
 */
export const readPngText = (png: string | Uint8Array, options: ReadTextOptions = {}): PngText[] => {
    const { keywords, decodeBase64 = true, inflate } = options
    const wanted = keywords ? new Set(keywords) : undefined
    const result: PngText[] = []
    for (const chunk of parsePngChunks(png, options)) {
        const raw = decodeTextChunk(chunk, inflate)
        if (!raw || (wanted && !wanted.has(raw.keyword))) continue
        const decoded = decodeBase64 ? tryBase64Utf8(raw.value) : undefined
        result.push({
            keyword: raw.keyword,
            type: raw.type,
            text: decoded ?? raw.value,
            base64Decoded: decoded !== undefined,
        })
    }
    return result
}

export type NewPngText = {
    keyword: string
    text: string
    /**
     * Store the UTF-8 bytes of `text` base64 encoded in a tEXt chunk (what character-card
     * readers expect for `chara`). Default `false`.
     */
    base64?: boolean
}

export type WriteTextOptions = ParseOptions & {
    /**
     * Also remove text chunks (tEXt, zTXt and iTXt) with these keywords. Chunks sharing a
     * keyword with a new entry are always removed.
     */
    removeKeywords?: readonly string[]
}

const encodeKeyword = (keyword: string): Uint8Array => {
    const bytes = latin1Encode(keyword)
    if (
        !bytes ||
        bytes.length < 1 ||
        bytes.length > 79 ||
        bytes.includes(0) ||
        keyword.trim() !== keyword ||
        keyword.includes('  ')
    )
        throw new PngError('BAD_KEYWORD', `Invalid PNG text keyword: ${JSON.stringify(keyword)}`)
    return bytes
}

const concat = (...parts: Uint8Array[]) => {
    const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0))
    let at = 0
    for (const p of parts) {
        out.set(p, at)
        at += p.length
    }
    return out
}

/**
 * Builds one text chunk. Base64 values and Latin-1 text go in tEXt; text Latin-1 cannot hold
 * goes in an uncompressed iTXt chunk, which is UTF-8 by definition.
 */
export const makeTextChunk = ({ keyword, text, base64 = false }: NewPngText): PngChunk => {
    const key = encodeKeyword(keyword)
    const zero = new Uint8Array([0])
    if (base64) {
        return {
            type: 'tEXt',
            data: concat(key, zero, latin1Encode(bytesToBase64(utf8Encode(text)))!),
        }
    }
    const latin1 = latin1Encode(text)
    if (latin1) {
        if (latin1.includes(0)) throw new PngError('BAD_TEXT', 'tEXt value may not contain NUL')
        return { type: 'tEXt', data: concat(key, zero, latin1) }
    }
    // keyword \0 flag=0 method=0 language="" \0 translated="" \0 text
    return { type: 'iTXt', data: concat(key, new Uint8Array([0, 0, 0, 0, 0]), utf8Encode(text)) }
}

/**
 * Returns a copy of the PNG with the given text chunks removed and the new ones inserted just
 * before the first IDAT. Image data and every other chunk are kept as they were.
 *
 * @param png base64 string or raw bytes of a PNG. Other image formats throw `PngError` with
 *   code `NOT_PNG`; convert them first (`RelayDevice.convertImageToPng`).
 * @returns the new PNG, base64 encoded
 */
export const writePngText = (
    png: string | Uint8Array,
    entries: NewPngText | readonly NewPngText[],
    options: WriteTextOptions = {}
): string => bytesToBase64(writePngTextBytes(png, entries, options))

/** `writePngText`, returning bytes instead of base64. */
export const writePngTextBytes = (
    png: string | Uint8Array,
    entries: NewPngText | readonly NewPngText[],
    options: WriteTextOptions = {}
): Uint8Array => {
    const list: readonly NewPngText[] = Array.isArray(entries) ? entries : [entries as NewPngText]
    const fresh = list.map(makeTextChunk)
    const remove = new Set<string>([
        ...(options.removeKeywords ?? []),
        ...list.map((e) => e.keyword),
    ])

    const out: PngChunk[] = []
    let inserted = false
    for (const chunk of parsePngChunks(png, options)) {
        if (chunk.type === 'tEXt' || chunk.type === 'zTXt' || chunk.type === 'iTXt') {
            const nul = indexOfZero(chunk.data, 0)
            if (nul > 0 && remove.has(latin1Decode(chunk.data.subarray(0, nul)))) continue
        }
        // IEND guards the degenerate no-IDAT case, so new text is never lost
        if (!inserted && (chunk.type === 'IDAT' || chunk.type === 'IEND')) {
            out.push(...fresh)
            inserted = true
        }
        out.push(chunk)
    }
    return buildPng(out)
}
