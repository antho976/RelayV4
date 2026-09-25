import { useEffect, useRef, useState } from 'react'

import { relay, RelayRequestError } from '@lib/engine/Relay/RelayClient'
import { DEFAULT_COLS, TerminalEmulator, TermSnapshot } from '@lib/engine/Relay/TerminalEmulator'

/** Redraws while output streams: about twenty a second. */
const FRAME_MS = 50

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
}

const EMPTY: TermSnapshot = { rows: [], cols: DEFAULT_COLS, alt: false }

/**
 * One session's PTY through a terminal emulator. `session.attach` with no position makes the
 * engine replay the newest raw bytes it holds (up to 256 KiB, from a frame boundary) and then
 * stream live frames, so the emulator rebuilds the screen from the same bytes the desktop drew
 * it from. A session with no PTY (parked, exited) shows the scrollback the engine saved, which
 * is text by lines, so redraws in it are approximate. A new epoch is a new process: the screen
 * starts over. `key` changes (reconnect, a new pid) re-attach from scratch.
 */
export const useTerminal = (name: string, active: boolean, key: unknown): TerminalFeed => {
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
            detach?.()
            // The next attachment starts from a blank screen.
            setAttached(false)
            setSnapshot(EMPTY)
            setLoading(true)
            setSaved(false)
            setProblem('')
        }
    }, [name, active, key])

    return {
        snapshot: snapshot,
        emulator: get,
        attached: attached,
        loading: active && loading,
        saved: saved,
        problem: problem,
    }
}
