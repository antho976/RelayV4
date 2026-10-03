import { Hunk } from './types'

type Op = { tag: ' ' | '-' | '+'; line: string }

/** Past this many edits the diff is shown as "everything removed, everything added". */
const MAX_EDITS = 2000

const splitLines = (text: string) => {
    if (text === '') return []
    const lines = text.split('\n')
    if (lines[lines.length - 1] === '') lines.pop()
    return lines
}

/** Myers' shortest edit script between two line arrays (the common ends already trimmed). */
const myers = (a: string[], b: string[]): Op[] | undefined => {
    const n = a.length
    const m = b.length
    const max = n + m
    const offset = max + 1
    const v = new Int32Array(2 * max + 3)
    // Each round keeps only the diagonals it can reach, so memory is O(D²), not O(D·(N+M)).
    const trace: Int32Array[] = []
    for (let d = 0; d <= max; d++) {
        if (d > MAX_EDITS) return undefined
        trace.push(v.slice(offset - d, offset + d + 1))
        for (let k = -d; k <= d; k += 2) {
            let x =
                k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1])
                    ? v[offset + k + 1]
                    : v[offset + k - 1] + 1
            let y = x - k
            while (x < n && y < m && a[x] === b[y]) {
                x++
                y++
            }
            v[offset + k] = x
            if (x >= n && y >= m) {
                const ops: Op[] = []
                let cx = n
                let cy = m
                for (let back = d; back > 0; back--) {
                    const prev = trace[back]
                    const at = (kk: number) => prev[kk + back]
                    const ck = cx - cy
                    const down = ck === -back || (ck !== back && at(ck - 1) < at(ck + 1))
                    const pk = down ? ck + 1 : ck - 1
                    const px = at(pk)
                    const py = px - pk
                    while (cx > px && cy > py) {
                        ops.push({ tag: ' ', line: a[cx - 1] })
                        cx--
                        cy--
                    }
                    if (down) ops.push({ tag: '+', line: b[cy - 1] })
                    else ops.push({ tag: '-', line: a[cx - 1] })
                    cx = px
                    cy = py
                }
                while (cx > 0 && cy > 0) {
                    ops.push({ tag: ' ', line: a[cx - 1] })
                    cx--
                    cy--
                }
                return ops.reverse()
            }
        }
    }
    return undefined
}

const lineOps = (a: string[], b: string[]): Op[] => {
    let start = 0
    while (start < a.length && start < b.length && a[start] === b[start]) start++
    let endA = a.length
    let endB = b.length
    while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
        endA--
        endB--
    }
    const middleA = a.slice(start, endA)
    const middleB = b.slice(start, endB)
    const middle = myers(middleA, middleB) ?? [
        ...middleA.map((line): Op => ({ tag: '-', line })),
        ...middleB.map((line): Op => ({ tag: '+', line })),
    ]
    return [
        ...a.slice(0, start).map((line): Op => ({ tag: ' ', line })),
        ...middle,
        ...a.slice(endA).map((line): Op => ({ tag: ' ', line })),
    ]
}

/**
 * A unified diff of two texts, in the engine's own shape: one Hunk whose text carries the
 * `@@ -a,b +c,d @@` headers, three lines of context. Used where the engine hands over two
 * texts but no hunks (a file as one commit changed it). Empty when the texts are equal.
 */
export const unifiedDiff = (oldText: string, newText: string, context = 3): Hunk[] => {
    if (oldText === newText) return []
    const ops = lineOps(splitLines(oldText), splitLines(newText))
    const out: string[] = []
    let i = 0
    let oldLine = 1
    let newLine = 1
    const positions = ops.map((op) => {
        const here = { old: oldLine, new: newLine }
        if (op.tag !== '+') oldLine++
        if (op.tag !== '-') newLine++
        return here
    })
    while (i < ops.length) {
        if (ops[i].tag === ' ') {
            i++
            continue
        }
        const from = Math.max(0, i - context)
        let to = i
        // Extend while the next change is within two contexts of this one.
        while (to < ops.length) {
            if (ops[to].tag !== ' ') {
                to++
                continue
            }
            let gap = to
            while (gap < ops.length && ops[gap].tag === ' ') gap++
            if (gap < ops.length && gap - to <= context * 2) to = gap
            else {
                to = Math.min(ops.length, to + context)
                break
            }
        }
        const slice = ops.slice(from, to)
        const oldCount = slice.filter((op) => op.tag !== '+').length
        const newCount = slice.filter((op) => op.tag !== '-').length
        const oldStart = oldCount === 0 ? positions[from].old - 1 : positions[from].old
        const newStart = newCount === 0 ? positions[from].new - 1 : positions[from].new
        out.push(`@@ -${oldStart},${oldCount} +${newStart},${newCount} @@`)
        for (const op of slice) out.push(`${op.tag}${op.line}`)
        i = to
    }
    return [
        {
            old_start: 1,
            new_start: 1,
            old_lines: splitLines(oldText).length,
            new_lines: splitLines(newText).length,
            text: out.join('\n') + '\n',
        },
    ]
}

export type DiffLine = {
    kind: 'head' | 'add' | 'del' | 'ctx' | 'note'
    text: string
    /** The line's number in the new file (old file for a removal); undefined for headers. */
    number?: number
}

/**
 * The lines of a set of hunks, numbered. Hunk text from the engine carries its own `@@`
 * headers; a hunk without them gets one from its starts.
 */
export const diffLines = (hunks: Hunk[]): DiffLine[] => {
    const lines: DiffLine[] = []
    for (const hunk of hunks) {
        const raw = hunk.text.endsWith('\n') ? hunk.text.slice(0, -1) : hunk.text
        const body = raw.split('\n')
        let oldLine = hunk.old_start
        let newLine = hunk.new_start
        if (!body[0]?.startsWith('@@'))
            lines.push({ kind: 'head', text: `@@ -${hunk.old_start} +${hunk.new_start} @@` })
        for (const text of body) {
            const header = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text)
            if (header) {
                oldLine = Number(header[1])
                newLine = Number(header[2])
                lines.push({ kind: 'head', text })
            } else if (text.startsWith('+')) {
                lines.push({ kind: 'add', text, number: newLine++ })
            } else if (text.startsWith('-')) {
                lines.push({ kind: 'del', text, number: oldLine++ })
            } else if (text.startsWith('\\')) {
                lines.push({ kind: 'note', text })
            } else {
                oldLine++
                lines.push({ kind: 'ctx', text, number: newLine++ })
            }
        }
    }
    return lines
}
