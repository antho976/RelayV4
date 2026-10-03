import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { createMMKVStorage } from '@lib/storage/MMKV'

/**
 * What the New terminal sheet last started with, so the next one opens on the same project
 * and agent. Kept on the phone; a project the PC no longer has falls back to the first one.
 */
type NewTerminalPrefs = {
    projectId?: number
    provider: 'claude' | 'codex'
    remember: (projectId: number, provider: 'claude' | 'codex') => void
}

export const useNewTerminalPrefs = create<NewTerminalPrefs>()(
    persist(
        (set) => ({
            projectId: undefined,
            provider: 'claude',
            remember: (projectId, provider) => set({ projectId: projectId, provider: provider }),
        }),
        {
            name: 'relay-new-terminal',
            storage: createMMKVStorage(),
            partialize: (state) => ({ projectId: state.projectId, provider: state.provider }),
            version: 1,
        }
    )
)
