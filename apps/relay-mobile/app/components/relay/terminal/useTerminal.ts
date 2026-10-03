import { useEffect, useRef, useState } from 'react'
import { AppState } from 'react-native'

import { relay, RelayRequestError } from '@lib/engine/Relay/RelayClient'
import { DEFAULT_COLS, TerminalEmulator, TermSnapshot } from '@lib/engine/Relay/TerminalEmulator'
import { Logger } from '@lib/state/Logger'

/** Redraws while output streams: about twenty a second. */
const FRAME_MS = 50

/** A new size settles this long before the PTY hears of it: one rotation is several layouts. */
const FIT_SETTLE_MS = 200

export type TermSize = { cols: number; rows: number }

export type TerminalFeed = {
    snapshot: TermSnapshot
    /** The emulator behind the screen (plain text for a summary, the paste mode). */
    emulator: () => TerminalEmulator
    /** Live: keystrokes reach the PTY. */
    attached: boolean
    /** Waiting for the first bytes. */
    loading: boolean
    /** No PTY: the screen shows the output the engine saved when the process stopped. */
    saved: boolean
    problem: string
    /** The PTY's size on the PC, when the engine reports it; only then can the phone borrow it. */
    pcSize?: TermSize
}

const EMPTY: TermSnapshot = { rows: [], cols: DEFAULT_COLS, alt: false }

/**
 * One session's PTY through a terminal emulator. `session.attach` with no position makes the
 * engine replay the newest raw bytes it holds (up to 256 KiB, from a frame boundary) and then
 * stream live frames, so the emulator rebuilds the screen from the same bytes the desktop drew
 * it from. A session with no PTY (parked, exited) shows the scrollback the engine saved, which
 * is text by lines, so redraws in it are approximate. A new epoch is a new process: the screen
 * starts over. `key` changes (reconnect, a new pid) re-attach from scratch.
 *
 * With `want`, the PTY is borrowed at that size while attached (`session.resize` with
 * `until_detach`), so the agent lays itself out for the phone. The engine hands the PC's size
 * back when this detaches or the link drops; the phone also hands it back when it is put in
 * the background, and borrows again when it is back. Without `want` the PTY keeps the PC's
 * size, and a borrow still held is ended.
 */
export const useTerminal = (
    name: string,
    active: boolean,
    key: unknown,
    want?: TermSize
): TerminalFeed => {
    const emulator = useRef<TerminalEmulator | null>(null)
    const get = () => {
        if (!emulator.current) emulator.current = new TerminalEmulator()
        return emulator.current
    }
    const [snapshot, setSnapshot] = useState<TermSnapshot>(EMPTY)
    const [attached, setAttached] = useState(false)
    const [loading, setLoading] = useState(true)
    const [saved, setSaved] = useState(false)
    const [problem, setProblem] = useState('')
    const [pcSize, setPcSize] = useState<TermSize>()
    const [foreground, setForeground] = useState(AppState.currentState === 'active')
    /** The PTY is held at the phone's size by this attachment. */
    const borrowed = useRef(false)

    useEffect(
        () => () => {
            emulator.current?.dispose()
            emulator.current = null
        },
        []
    )

    useEffect(() => {
        const em = get()
        if (!name || !active) return
        em.reset()
        let cancelled = false
        let dirty = true
        let detach: (() => void) | undefined
        let epoch: number | undefined
        const off = em.onChange(() => {
            dirty = true
        })
        const timer = setInterval(() => {
            if (!dirty) return
            dirty = false
            setSnapshot(em.snapshot())
        }, FRAME_MS)
        ;(async () => {
            try {
                // The PTY's size first: the replay that follows was drawn for it. An engine too
                // old to report it leaves the screen to learn it from the output.
                const probe = await relay
                    .request<{ cols?: number; rows?: number }>('session.scrollback', {
                        session: name,
                        lines: 1,
                    })
                    .catch(() => undefined)
                if (cancelled) return
                if (probe?.cols && probe.rows) {
                    em.setSize(probe.cols, probe.rows)
                    setPcSize({ cols: probe.cols, rows: probe.rows })
                }
                detach = await relay.attach(name, (frame, chunk) => {
                    if (epoch !== undefined && frame.epoch !== undefined && frame.epoch !== epoch)
                        em.reset()
                    if (frame.epoch !== undefined) epoch = frame.epoch
                    em.write(chunk)
                    setLoading(false)
                })
                if (cancelled) {
                    detach()
                    return
                }
                setAttached(true)
                // A PTY that has printed nothing yet sends no catch-up.
                setTimeout(() => !cancelled && setLoading(false), 800)
            } catch (e) {
                if (cancelled) return
                const code = e instanceof RelayRequestError ? e.error.code : undefined
                if (code !== 'session.not_spawned' && code !== 'session.exited') {
                    setProblem(`${(e as Error).message}`)
                    setLoading(false)
                    return
                }
                try {
                    const back = await relay.request<{ text: string }>('session.scrollback', {
                        session: name,
                        lines: 2000,
                    })
                    if (cancelled) return
                    // Saved by lines: each line starts at the left edge again.
                    em.write(back.text.replace(/\r?\n/g, '\r\n'))
                    setSaved(true)
                } catch (inner) {
                    if (!cancelled) setProblem(`${(inner as Error).message}`)
                } finally {
                    if (!cancelled) setLoading(false)
                }
            }
        })()
        return () => {
            cancelled = true
            clearInterval(timer)
            off()
            // Detaching is what hands a borrowed size back; a dropped link does it as well.
            detach?.()
            borrowed.current = false
            setPcSize(undefined)
            // The next attachment starts from a blank screen.
            setAttached(false)
            setSnapshot(EMPTY)
            setLoading(true)
            setSaved(false)
            setProblem('')
        }
    }, [name, active, key])

    const lend = !!want && !!pcSize
    const target = attached && pcSize ? (want ?? pcSize) : undefined
    const targetCols = target?.cols
    const targetRows = target?.rows

    // Borrowed, the size goes back to the PC by itself when this detaches; set for good, it
    // ends the loan, which is what "Use the PC's width" does.
    useEffect(() => {
        if (!targetCols || !targetRows || !foreground) return
        const timer = setTimeout(() => {
            emulator.current?.setSize(targetCols, targetRows)
            if (!lend && !borrowed.current) return
            borrowed.current = lend
            relay
                .request('session.resize', {
                    session: name,
                    cols: targetCols,
                    rows: targetRows,
                    ...(lend ? { until_detach: true } : {}),
                })
                .catch((e) => Logger.warn(`Could not fit the terminal: ${(e as Error).message}`))
        }, FIT_SETTLE_MS)
        return () => clearTimeout(timer)
    }, [name, targetCols, targetRows, lend, foreground])

    // Put away, the phone hands the terminal back at once, so whoever sits down at the PC
    // finds it as they left it. Back in front, the effect above borrows it again.
    useEffect(() => {
        const subscription = AppState.addEventListener('change', (next) => {
            setForeground(next === 'active')
            if (next !== 'background' || !borrowed.current || !pcSize) return
            borrowed.current = false
            relay
                .request('session.resize', { session: name, cols: pcSize.cols, rows: pcSize.rows })
                .catch(() => {})
        })
        return () => subscription.remove()
    }, [name, pcSize])

    return {
        snapshot: snapshot,
        emulator: get,
        attached: attached,
        loading: active && loading,
        saved: saved,
        problem: problem,
        pcSize: pcSize,
    }
}
