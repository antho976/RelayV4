import { create } from 'zustand'

type ChatsDrawerStoreProps = {
    /**
     * Whether hidden chats are currently listed in the chats drawer.
     * Intentionally not persisted so hidden chats stay hidden after a restart.
     */
    revealHidden: boolean
    setRevealHidden: (b: boolean) => void
}

export const useChatsDrawerStore = create<ChatsDrawerStoreProps>()((set) => ({
    revealHidden: false,
    setRevealHidden: (b) => set({ revealHidden: b }),
}))
