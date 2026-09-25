import { useFocusEffect } from 'expo-router'
import { useCallback, useRef, useState } from 'react'

import { useRelayEvent, useRelayOnline } from '@components/relay/hooks'
import { relay } from '@lib/engine/Relay/RelayClient'

/** One row of `session.peers`: what an agent says it is doing, and what it holds. */
export type RelayPeer = {
    session: string
    provider: string
    role: string
    branch: string
    state: string
    task_title: string | null
    claimed: string[]
    last_output_at: string | null
    intent: string | null
}

/**
 * The peer table of every project in `projectIds`, keyed by session name: one
 * `session.peers {project_id}` per project, reloaded on focus, when the list of projects
 * changes and after session or overlap events. A project that fails keeps its last rows.
 */
export const usePeers = (projectIds: number[]): Record<string, RelayPeer> => {
    const online = useRelayOnline()
    const key = [...new Set(projectIds)].sort((a, b) => a - b).join(',')
    const [peers, setPeers] = useState<Record<string, RelayPeer>>({})
    const generation = useRef(0)

    const load = useCallback(async () => {
        if (!online || !key) return
        const mine = ++generation.current
        const ids = key.split(',').map(Number)
        const results = await Promise.all(
            ids.map((id) =>
                relay
                    .call<{ peers: RelayPeer[] }>('session.peers', { project_id: id })
                    .then((result) => result.peers)
                    .catch(() => undefined)
            )
        )
        if (mine !== generation.current) return
        setPeers((previous) => {
            const next: Record<string, RelayPeer> = {}
            // Keep the rows of a project whose request failed, drop those of any other.
            ids.forEach((id, index) => {
                const rows = results[index]
                if (rows) rows.forEach((peer) => (next[peer.session] = peer))
            })
            if (results.some((rows) => rows === undefined)) {
                for (const [name, peer] of Object.entries(previous)) {
                    if (!(name in next)) next[name] = peer
                }
            }
            return next
        })
    }, [online, key])

    // Runs on focus, and again while focused whenever `load` changes (other projects, the
    // link coming back).
    useFocusEffect(
        useCallback(() => {
            load()
        }, [load])
    )
    useRelayEvent(['session.changed', 'overlap.changed'], () => load(), {
        debounceMs: 800,
        enabled: online && !!key,
    })
    return peers
}
