import { create } from 'zustand'

/**
 * What the PC tab is looking at: every session, one workspace, or one project. Picked in the
 * sidebar; not persisted, so the tab opens on everything.
 */
export type RelayScope =
    | { kind: 'all' }
    | { kind: 'workspace'; id: number }
    | { kind: 'project'; id: number }

type RelayViewState = {
    scope: RelayScope
    setScope: (scope: RelayScope) => void
}

export const useRelayView = create<RelayViewState>()((set) => ({
    scope: { kind: 'all' },
    setScope: (scope) => set({ scope }),
}))
