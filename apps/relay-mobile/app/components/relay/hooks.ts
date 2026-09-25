import { useFocusEffect, useLocalSearchParams } from 'expo-router'
import { useCallback, useEffect, useRef, useState } from 'react'

import {
    BusEvent,
    matchEvent,
    relay,
    StreamFrame,
    useRelayStore,
} from '@lib/engine/Relay/RelayClient'

export type RelayEventOptions = {
    /** Coalesce a burst into one call, made with the last event, this long after it ends. */
    debounceMs?: number
    /** Only events of this project (events without a project_id always pass). */
    projectId?: number | string
    /** False pauses the listener without unmounting. Default true. */
    enabled?: boolean
}

/** The PC screens other screens link to; each is app/screens/RelayScreen/<name>.tsx. */
export type RelayPage =
    | 'Project'
    | 'Board'
    | 'Modules'
    | 'Notes'
    | 'Mailbox'
    | 'Git'
    | 'Files'
    | 'Guardrails'
    | 'Integration'
    | 'Launch'
    | 'ProjectSettings'
    | 'Devices'
    | 'PcSettings'
    | 'AddProject'
    | 'Inbox'
    | 'Hosts'
    | 'Terminal'
    | 'Session'
    | 'Changes'
    | 'Commit'
    | 'FileView'
    | 'Task'
    | 'Module'
    | 'Note'
    | 'Skills'
    | 'Skill'
    | 'Plugins'

/**
 * An href for `router.push`: `router.push(relayHref('Notes', { project_id: '3' }))`.
 * Params are strings; per-project screens take `project_id`.
 */
export const relayHref = (page: RelayPage, params: Record<string, string> = {}) => ({
    pathname: `/screens/RelayScreen/${page}` as const,
    params: params,
})

/**
 * The `project_id` route param of a per-project screen, as a number, and the project it
 * names on the connected PC (undefined until the project list has loaded, or if it is gone).
 */
export const useProjectParam = () => {
    const params = useLocalSearchParams<{ project_id?: string }>()
    const parsed = params.project_id ? Number(params.project_id) : NaN
    const projectId = Number.isFinite(parsed) ? parsed : undefined
    const project = useRelayStore((state) => state.projects.find((item) => item.id === projectId))
    return { projectId, project }
}

/** True while the link to the PC is up. */
export const useRelayOnline = () => useRelayStore((state) => state.status === 'online')

/**
 * Call `handler` for every bus event matching `patterns` (exact, `prefix.*` or `*`, the
 * engine's own syntax) while the component is mounted. The latest `handler` is always the
 * one called, so it needs no dependency list. Only events the phone subscribes to arrive:
 * see `EVENT_PATTERNS` in RelayClient.ts (`resource.sample` needs `useResourceSamples`).
 */
export const useRelayEvent = (
    patterns: string[],
    handler: (event: BusEvent) => void,
    options: RelayEventOptions = {}
) => {
    const latest = useRef(handler)
    useEffect(() => {
        latest.current = handler
    })
    const key = patterns.join('|')
    const { debounceMs = 0, projectId, enabled = true } = options
    const project = projectId === undefined ? undefined : String(projectId)
    useEffect(() => {
        if (!enabled) return
        const wanted = key.split('|')
        let timer: ReturnType<typeof setTimeout> | undefined
        const off = relay.onEvents((event) => {
            if (!matchEvent(wanted, event.ev)) return
            if (
                project !== undefined &&
                event.project_id !== undefined &&
                event.project_id !== null &&
                String(event.project_id) !== project
            )
                return
            if (debounceMs <= 0) {
                latest.current(event)
                return
            }
            if (timer) clearTimeout(timer)
            timer = setTimeout(() => {
                timer = undefined
                latest.current(event)
            }, debounceMs)
        })
        return () => {
            if (timer) clearTimeout(timer)
            off()
        }
    }, [key, debounceMs, project, enabled])
}

/**
 * Frames of a data-plane stream while mounted, e.g. `useRelayStream('logcat', runId, onLine)`.
 * Pass `undefined` as the key to listen to nothing yet.
 */
export const useRelayStream = (
    stream: string,
    key: string | number | undefined,
    handler: (frame: StreamFrame) => void
) => {
    const latest = useRef(handler)
    useEffect(() => {
        latest.current = handler
    })
    useEffect(() => {
        if (key === undefined) return
        return relay.onStream(stream, key, (frame) => latest.current(frame))
    }, [stream, key])
}

/**
 * Live resource samples (`resource.sample`, ~1/s) while the screen is focused; the watch is
 * released on blur and unmount, so the PC stops sampling for a phone that looks elsewhere.
 */
export const useResourceSamples = (handler: (sample: any) => void, enabled = true) => {
    useFocusEffect(
        useCallback(() => {
            if (!enabled) return
            relay.watchResources(true)
            return () => relay.watchResources(false)
        }, [enabled])
    )
    useRelayEvent(['resource.sample'], (event) => handler(event.payload), { enabled })
}

export type BusQueryOptions<T> = {
    /** Event patterns that make the query reload (debounced). */
    events?: string[]
    /** Only reload for events of this project. */
    projectId?: number | string
    /** False skips the query (e.g. a param is not known yet). Default true. */
    enabled?: boolean
    /** Reload whenever the screen regains focus. Default true. */
    refetchOnFocus?: boolean
    /** Default 300 ms. */
    debounceMs?: number
    /** Shape the raw result, e.g. `(r) => r.notes`. */
    select?: (raw: any) => T
}

export type BusQuery<T> = {
    data: T | undefined
    error: Error | undefined
    /** True while a request is in flight (also during a reload with data shown). */
    loading: boolean
    reload: () => Promise<void>
    /** Optimistic local edit; the next reload replaces it. */
    setData: (next: T | undefined | ((prev: T | undefined) => T | undefined)) => void
}

/**
 * A query op as screen state: loads when online, reloads on matching events, on focus, and
 * when the link comes back; skips while offline (keeping the last data). A slow answer to an
 * earlier payload never overwrites a newer one. `payload` may be a fresh object each render;
 * it is compared by its JSON.
 */
export const useBusQuery = <T = any>(
    op: string,
    payload: object = {},
    options: BusQueryOptions<T> = {}
): BusQuery<T> => {
    const {
        events = [],
        projectId,
        enabled = true,
        refetchOnFocus = true,
        debounceMs = 300,
    } = options
    const online = useRelayOnline()
    const body = JSON.stringify(payload)
    const [data, setData] = useState<T | undefined>(undefined)
    const [error, setError] = useState<Error | undefined>(undefined)
    const [loading, setLoading] = useState(false)
    const generation = useRef(0)
    const select = useRef(options.select)
    useEffect(() => {
        select.current = options.select
    })
    // Data belongs to the payload it was fetched for; a new payload starts empty.
    const shownFor = useRef(body)
    const loadedFor = useRef<string | undefined>(undefined)

    const reload = useCallback(async () => {
        if (!enabled || !online) return
        const mine = ++generation.current
        if (shownFor.current !== body) {
            shownFor.current = body
            setData(undefined)
            setError(undefined)
        }
        setLoading(true)
        try {
            const raw = await relay.call(op, JSON.parse(body))
            if (mine !== generation.current) return
            loadedFor.current = body
            setData(select.current ? select.current(raw) : (raw as T))
            setError(undefined)
        } catch (e) {
            if (mine !== generation.current) return
            setError(e as Error)
        } finally {
            if (mine === generation.current) setLoading(false)
        }
    }, [op, body, enabled, online])

    // Focus covers the first load too: it fires on mount for a focused screen, and again
    // whenever `reload` changes (a new payload, the link coming back). Without
    // refetchOnFocus, a return to the screen reloads only what never loaded.
    useFocusEffect(
        useCallback(() => {
            if (refetchOnFocus || loadedFor.current !== body) reload()
        }, [refetchOnFocus, reload, body])
    )

    useRelayEvent(events, () => reload(), {
        debounceMs: debounceMs,
        projectId: projectId,
        enabled: enabled && online && events.length > 0,
    })

    return { data, error, loading, reload, setData }
}
