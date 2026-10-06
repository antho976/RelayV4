import React, { ReactNode, useEffect } from 'react'
import { ScrollView, StyleSheet, Text, View } from 'react-native'
import { create } from 'zustand'

import ThemedButton from '@components/buttons/ThemedButton'
import BottomSheet, { useBottomSheetRef } from '@components/views/BottomSheet'
import { Theme } from '@lib/theme/ThemeManager'

/**
 * The app's bottom sheet, driven by a `visible` prop instead of a ref. Back and the backdrop
 * call `onDismiss`; the owner decides whether that closes it.
 */
export const Sheet: React.FC<{
    visible: boolean
    onDismiss: () => void
    children?: ReactNode
}> = ({ visible, onDismiss, children }) => {
    const ref = useBottomSheetRef()
    useEffect(() => {
        if (visible) ref.current?.open()
        else ref.current?.close()
    }, [visible, ref])
    return (
        <BottomSheet ref={ref} onRequestClose={() => onDismiss()}>
            {children}
        </BottomSheet>
    )
}

export type ConfirmOptions = {
    title: string
    message?: string
    /** Default "Continue". */
    confirmLabel?: string
    /** Default "Cancel". */
    cancelLabel?: string
    /** A red confirm button, for deletes and other things that cannot be taken back. */
    destructive?: boolean
    /** Extra content under the message, e.g. a list of what will be deleted. */
    body?: ReactNode
}

type Pending = ConfirmOptions & { resolve: (yes: boolean) => void }

const useConfirmStore = create<{ pending?: Pending }>()(() => ({}))

/**
 * Ask a yes/no question in a bottom sheet; resolves true on confirm, false on cancel, back or
 * a tap outside. Needs `<ConfirmHost />`, mounted once in app/_layout.tsx.
 *
 *     if (!(await confirm({ title: 'Delete note?', destructive: true }))) return
 */
export const confirm = (options: ConfirmOptions): Promise<boolean> =>
    new Promise((resolve) => {
        // A newer question replaces an unanswered one, which counts as cancelled.
        useConfirmStore.getState().pending?.resolve(false)
        useConfirmStore.setState({ pending: { ...options, resolve } })
    })

const settle = (yes: boolean) => {
    const pending = useConfirmStore.getState().pending
    useConfirmStore.setState({ pending: undefined })
    pending?.resolve(yes)
}

export const ConfirmHost = () => {
    const styles = useSheetStyles()
    const pending = useConfirmStore((state) => state.pending)
    return (
        <Sheet visible={!!pending} onDismiss={() => settle(false)}>
            {pending && (
                <View style={styles.body}>
                    <Text style={styles.title}>{pending.title}</Text>
                    {!!pending.message && <Text style={styles.message}>{pending.message}</Text>}
                    {pending.body && <ScrollView style={styles.scroll}>{pending.body}</ScrollView>}
                    <View style={styles.actions}>
                        <ThemedButton
                            label={pending.cancelLabel ?? 'Cancel'}
                            variant="secondary"
                            onPress={() => settle(false)}
                        />
                        <ThemedButton
                            label={pending.confirmLabel ?? 'Continue'}
                            variant={pending.destructive ? 'critical' : 'primary'}
                            onPress={() => settle(true)}
                        />
                    </View>
                </View>
            )}
        </Sheet>
    )
}

export const useSheetStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        body: {
            rowGap: spacing.l,
            paddingRight: spacing.s,
        },
        title: {
            color: color.text._100,
            fontSize: fontSize.xl,
            fontWeight: '600',
        },
        message: {
            color: color.text._300,
            lineHeight: 20,
        },
        scroll: {
            flexGrow: 0,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
            marginTop: spacing.s,
        },
    })
}
