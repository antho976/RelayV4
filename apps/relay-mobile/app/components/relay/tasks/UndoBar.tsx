import React, { useEffect } from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { create } from 'zustand'

import { Theme } from '@lib/theme/ThemeManager'

import { attempt } from './types'

type Pending = { key: number; label: string; undo: () => Promise<unknown> }

const useUndoStore = create<{ pending?: Pending }>()(() => ({}))

let counter = 0
const SHOWN_MS = 8000

/**
 * Offer to take back what just happened: `offerUndo('Task deleted', () => relay.guarded(…))`.
 * The bar shows on whichever task or module screen is on top, so a delete that pops back to
 * the board still offers its undo there.
 */
export const offerUndo = (label: string, undo: () => Promise<unknown>) => {
    useUndoStore.setState({ pending: { key: ++counter, label, undo } })
}

/** The pinned Undo bar; pass it as a Screen `footer`. */
export const UndoBar = () => {
    const styles = useStyles()
    const pending = useUndoStore((state) => state.pending)
    useEffect(() => {
        if (!pending) return
        const timer = setTimeout(() => {
            if (useUndoStore.getState().pending?.key === pending.key)
                useUndoStore.setState({ pending: undefined })
        }, SHOWN_MS)
        return () => clearTimeout(timer)
    }, [pending])
    if (!pending) return null
    const undo = async () => {
        useUndoStore.setState({ pending: undefined })
        await attempt(pending.undo, 'Undone')
    }
    return (
        <View style={styles.bar}>
            <Text numberOfLines={1} style={styles.text}>
                {pending.label}
            </Text>
            <TouchableOpacity hitSlop={10} onPress={undo}>
                <Text style={styles.action}>Undo</Text>
            </TouchableOpacity>
            <TouchableOpacity
                hitSlop={10}
                onPress={() => useUndoStore.setState({ pending: undefined })}>
                <Text style={styles.dismiss}>×</Text>
            </TouchableOpacity>
        </View>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        bar: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.xl,
            marginHorizontal: spacing.xl,
            marginBottom: spacing.m,
            paddingHorizontal: spacing.xl,
            paddingVertical: spacing.l,
            borderRadius: 14,
            backgroundColor: color.neutral._400,
        },
        text: {
            flex: 1,
            color: color.text._100,
        },
        action: {
            color: color.primary._700,
            fontWeight: '600',
            fontSize: fontSize.m,
        },
        dismiss: {
            color: color.text._400,
            fontSize: fontSize.l,
        },
    })
}
