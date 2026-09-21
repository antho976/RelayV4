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
    /** Rendezvous join URL, if the PC has one configured. */
    via?: string
    deviceId: string
    token: string
    route: RelayRoute
    pairedAt: number
    lastConnectedAt?: number
    lastTransport?: 'direct' | 'via'
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
                const others = get().hosts.filter((item) => item.id !== host.id)
                set({ hosts: [...others, host], activeHostId: host.id })
            },
            updateHost: (id, patch) => {
                set({
                    hosts: get().hosts.map((item) =>
                        item.id === id ? { ...item, ...patch } : item
                    ),
                })
            },
            removeHost: (id) => {
                const hosts = get().hosts.filter((item) => item.id !== id)
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
            version: 1,
            partialize: (state) => ({ hosts: state.hosts, activeHostId: state.activeHostId }),
        }
    )
)

export const activeHost = () => {
    const { hosts, activeHostId } = useRelayHostsStore.getState()
    return hosts.find((item) => item.id === activeHostId) ?? hosts[0]
}
