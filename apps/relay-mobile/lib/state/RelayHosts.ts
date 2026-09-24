import * as SecureStore from 'expo-secure-store'
import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { Storage } from '@lib/enums/Storage'
import { createMMKVStorage } from '@lib/storage/MMKV'

/** How the phone prefers to reach a PC. `auto` tries the LAN first and the server after. */
export type RelayRoute = 'auto' | 'direct' | 'via'

/** A PC this phone paired with. The token is the credential; it never leaves the device. */
export type RelayHost = {
    id: string
    name: string
    instance: string
    /** Direct LAN addresses, `ws://ip:port`, in the order the PC listed them. */
    direct: string[]
    /**
     * Addresses the person added by hand — a Tailscale address, a hostname — tried alongside
     * `direct`. Kept apart so re-pairing, which replaces `direct`, does not lose them.
     */
    extra?: string[]
    /** Rendezvous join URL, if the PC has one configured. */
    via?: string
    deviceId: string
    token: string
    route: RelayRoute
    pairedAt: number
    lastConnectedAt?: number
    lastTransport?: 'direct' | 'via'
}

/**
 * The device token lives in the Android keystore (SecureStore), not beside the rest of the
 * record in MMKV: MMKV files go into Android's cloud backup, and a restored backup would
 * otherwise carry a working credential to another phone. A keystore key is never backed up,
 * so after a restore the token is gone and the PC has to be paired again — on purpose.
 */
const tokenKey = (hostId: string) => `relay-token-${hostId.replace(/[^A-Za-z0-9._-]/g, '_')}`

const saveToken = (hostId: string, token: string) => {
    try {
        SecureStore.setItem(tokenKey(hostId), token)
    } catch {}
}

const loadToken = (hostId: string): string => {
    try {
        return SecureStore.getItem(tokenKey(hostId)) ?? ''
    } catch {
        return ''
    }
}

const dropToken = (hostId: string) => {
    SecureStore.deleteItemAsync(tokenKey(hostId)).catch(() => {})
}

type RelayHostsState = {
    hosts: RelayHost[]
    activeHostId?: string
    addHost: (host: RelayHost) => void
    updateHost: (id: string, patch: Partial<RelayHost>) => void
    removeHost: (id: string) => void
    setActiveHost: (id: string | undefined) => void
}

export const useRelayHostsStore = create<RelayHostsState>()(
    persist(
        (set, get) => ({
            hosts: [],
            activeHostId: undefined,
            addHost: (host) => {
                // Re-pairing the same PC replaces its record: one card per machine.
                const previous = get().hosts.find((item) => item.id === host.id)
                const others = get().hosts.filter((item) => item.id !== host.id)
                const merged = previous?.extra?.length
                    ? { ...host, extra: host.extra ?? previous.extra }
                    : host
                saveToken(host.id, host.token)
                set({ hosts: [...others, merged], activeHostId: host.id })
            },
            updateHost: (id, patch) => {
                if (patch.token !== undefined) saveToken(id, patch.token)
                set({
                    hosts: get().hosts.map((item) =>
                        item.id === id ? { ...item, ...patch } : item
                    ),
                })
            },
            removeHost: (id) => {
                const hosts = get().hosts.filter((item) => item.id !== id)
                dropToken(id)
                set({
                    hosts: hosts,
                    activeHostId: get().activeHostId === id ? hosts[0]?.id : get().activeHostId,
                })
            },
            setActiveHost: (id) => set({ activeHostId: id }),
        }),
        {
            name: Storage.RelayHosts,
            storage: createMMKVStorage(),
            version: 2,
            // the token is stored apart (see tokenKey) and put back on load
            partialize: (state) => ({
                hosts: state.hosts.map((host) => ({ ...host, token: '' })),
                activeHostId: state.activeHostId,
            }),
            migrate: (persisted: any, version) => {
                // version 1 kept the token in MMKV: move it to the keystore
                if (version < 2)
                    for (const host of persisted?.hosts ?? [])
                        if (host.token) saveToken(host.id, host.token)
                return persisted
            },
            merge: (persisted: any, current) => ({
                ...current,
                ...persisted,
                hosts: (persisted?.hosts ?? []).map((host: RelayHost) => ({
                    ...host,
                    token: loadToken(host.id),
                })),
            }),
        }
    )
)

export const activeHost = () => {
    const { hosts, activeHostId } = useRelayHostsStore.getState()
    return hosts.find((item) => item.id === activeHostId) ?? hosts[0]
}

/** Every direct address of a host: what the PC listed, then what the person added. */
export const directRoutes = (host: RelayHost): string[] => {
    const all = [...host.direct, ...(host.extra ?? [])]
    return all.filter((url, index) => all.indexOf(url) === index)
}

/**
 * A Tailscale address: 100.64.0.0/10 (CGNAT, where tailnets live) or a MagicDNS name.
 * Reaching one is still a direct link, only over the person's tailnet instead of the WiFi.
 */
export const isTailnetUrl = (url: string): boolean => {
    const host = /^wss?:\/\/\[?([^/:\]]+)/i.exec(url)?.[1]?.toLowerCase() ?? ''
    if (host.endsWith('.ts.net')) return true
    const octets = host.split('.').map(Number)
    if (octets.length !== 4 || octets.some((part) => Number.isNaN(part))) return false
    return octets[0] === 100 && octets[1] >= 64 && octets[1] <= 127
}
