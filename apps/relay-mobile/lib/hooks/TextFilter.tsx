import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { Storage } from '@lib/enums/Storage'
import { createMMKVStorage } from '@lib/storage/MMKV'

type TextFilterStateProps = {
    filter: string[]
    sendFilteredText: boolean
    setFilter: (arr: string[]) => void
    setSendFilteredText: (b: boolean) => void
}

export const useTextFilterStore = create<TextFilterStateProps>()(
    persist(
        (set) => ({
            filter: [],
            sendFilteredText: true,
            setFilter: (arr) => set({ filter: arr }),
            setSendFilteredText: (b) => {
                set({ sendFilteredText: b })
            },
        }),
        {
            name: Storage.TextFilter,
            version: 1,
            storage: createMMKVStorage(),
        }
    )
)

/**
 * A filter word is a regular expression; one that does not compile (`c++`, `(`)
 * is matched as plain text instead.
 */
export const filterRegex = (item: string) => {
    try {
        return new RegExp(item, 'gi')
    } catch {
        return new RegExp(item.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi')
    }
}

type RegexResult = {
    result: string
    found: boolean
}

export const useTextFilter = (inputString: string): RegexResult => {
    const filters = useTextFilterStore((state) => state.filter)
    if (filters.length === 0) return { result: inputString, found: false }
    let newString = inputString
    filters.forEach((item) => {
        if (item) newString = newString.replace(filterRegex(item), '')
    })
    return { result: newString, found: newString.length !== inputString.length }
}
