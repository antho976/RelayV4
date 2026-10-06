/**
 * The question `relay.guarded()` puts to the person when the PC holds one of the phone's own
 * mutations: what is held, why, and the exact frozen request. The answer is a plain boolean.
 *
 * The sheet that shows it (app/components/relay/HoldSheet.tsx) is mounted once in the root
 * layout and reads this store; asks queue, so two held actions are answered one at a time.
 */
import { create } from 'zustand'

import type { BusError, HeldRequest, RelayHoldFull } from './RelayClient'

export type HoldAsk = {
    /** A key for React; the hold id is unique per PC. */
    key: string
    /** Why the engine held it: the `held` error the op answered with. */
    error: BusError
    hold: RelayHoldFull
    request: HeldRequest
    answer: (allow: boolean) => void
}

type HoldPromptState = {
    queue: HoldAsk[]
    /** Ask the person; resolves with their answer. */
    ask: (item: Omit<HoldAsk, 'answer' | 'key'>) => Promise<boolean>
    /** Answer the ask at the head of the queue. */
    answer: (allow: boolean) => void
    /** Deny everything still waiting, e.g. when the link drops. */
    clear: () => void
}

export const useHoldPromptStore = create<HoldPromptState>()((set, get) => ({
    queue: [],
    ask: (item) =>
        new Promise<boolean>((resolve) => {
            const entry: HoldAsk = {
                ...item,
                key: `${item.hold.id}`,
                answer: (allow) => {
                    set((state) => ({ queue: state.queue.filter((q) => q !== entry) }))
                    resolve(allow)
                },
            }
            set((state) => ({ queue: [...state.queue, entry] }))
        }),
    answer: (allow) => get().queue[0]?.answer(allow),
    clear: () => {
        for (const item of get().queue) item.answer(false)
    },
}))
