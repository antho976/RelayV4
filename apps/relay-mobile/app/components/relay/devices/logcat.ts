import { useSyncExternalStore } from 'react'

import { relay, StreamFrame } from '@lib/engine/Relay/RelayClient'

/** Lines kept per run; older ones drop off the top. */
export const MAX_LINES = 2000
/** Runs whose log is kept; the oldest is forgotten first. */
const MAX_RUNS = 8
/** The screen redraws at most this often while lines pour in. */
const FLUSH_MS = 200

/** What a screen draws: an immutable copy, made once per redraw of a run that changed. */
export type LogView = { lines: string[]; error?: string; tracked: boolean }

type Log = { lines: string[]; error?: string; dirty: boolean; view: LogView }

const EMPTY: LogView = { lines: [], tracked: false }

const logs = new Map<number, Log>()
const listeners = new Set<() => void>()
let listening = false
let timer: ReturnType<typeof setTimeout> | undefined

const flush = () => {
    timer = undefined
    for (const log of logs.values()) {
        if (!log.dirty) continue
        log.dirty = false
        log.view = { lines: [...log.lines], error: log.error, tracked: true }
    }
    for (const listener of [...listeners]) listener()
}

const schedule = () => {
    if (!timer) timer = setTimeout(flush, FLUSH_MS)
}

const logOf = (runId: number) => {
    let log = logs.get(runId)
    if (!log) {
        log = { lines: [], dirty: false, view: { lines: [], tracked: true } }
        logs.set(runId, log)
        while (logs.size > MAX_RUNS) logs.delete(logs.keys().next().value!)
    }
    return log
}

const onFrame = (frame: StreamFrame) => {
    if (typeof frame.run_id !== 'number') return
    const log = logOf(frame.run_id)
    if (typeof frame.data === 'string') {
        for (const line of frame.data.replace(/\n$/, '').split('\n')) log.lines.push(line)
        if (log.lines.length > MAX_LINES + 200) log.lines.splice(0, log.lines.length - MAX_LINES)
    } else if (frame.data?.error) {
        log.error = frame.data.error
    }
    log.dirty = true
    schedule()
}

/**
 * Start keeping logcat lines. The engine sends a run's frames only to the connection that
 * started it, right after its answer, so the listener is on before the first run and stays
 * on: the log survives leaving the screen and coming back.
 */
export const listenToLogcat = () => {
    if (listening) return
    listening = true
    relay.onStream('logcat', '*', onFrame)
}

/** Remember a run this phone started, so it shows as streaming even before its first line. */
export const trackRun = (runId: number) => {
    listenToLogcat()
    logOf(runId)
    schedule()
}

/** True when this phone started the run (in this app session) and so receives its lines. */
export const isTracked = (runId: number) => logs.has(runId)

export const clearLog = (runId: number) => {
    const log = logs.get(runId)
    if (!log) return
    log.lines = []
    log.error = undefined
    log.dirty = true
    schedule()
}

const subscribe = (listener: () => void) => {
    listeners.add(listener)
    return () => {
        listeners.delete(listener)
    }
}

/** The kept lines of one run, redrawn as they arrive (batched every 200 ms). */
export const useLogcat = (runId: number | undefined): LogView =>
    useSyncExternalStore(subscribe, () =>
        runId === undefined ? EMPTY : (logs.get(runId)?.view ?? EMPTY)
    )
