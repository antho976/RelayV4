import { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import { Field, isCancelled, RelayRequestError, Row, Sheet } from '@components/relay'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

/** The settings screens that are not in `RelayPage` (the kit's list of linked pages). */
export type SettingsPage = 'Skills' | 'Skill' | 'Plugins'

/** An href for `router.push`, like `relayHref`, for the settings pages this folder adds. */
export const settingsHref = (page: SettingsPage, params: Record<string, string> = {}) => ({
    pathname: `/screens/RelayScreen/${page}` as const,
    params: params,
})

/** The bus error code of a failed request, if it was one. */
export const errorCode = (e: unknown) => (e instanceof RelayRequestError ? e.error.code : undefined)

/** What to tell a person about a failed request: the engine's message, not its code. */
export const errorText = (e: unknown) =>
    e instanceof RelayRequestError
        ? e.error.message + (e.error.hint ? ` ${e.error.hint}` : '')
        : `${(e as Error)?.message ?? e}`

/**
 * Run a mutation and report its failure as a toast; a Deny on a held action stays quiet.
 * Resolves with the result, or undefined when it failed or was denied.
 */
export const attempt = async <T,>(
    action: () => Promise<T>,
    success?: string
): Promise<T | undefined> => {
    try {
        const result = await action()
        if (success) Logger.infoToast(success)
        return result
    } catch (e) {
        if (!isCancelled(e)) Logger.errorToast(errorText(e))
        return undefined
    }
}

/** One line of text per entry, blanks dropped: how the desktop edits lists of patterns. */
export const splitLines = (text: string) =>
    text
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.length > 0)

export const formatBytes = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
    return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`
}

/** `2026-09-25T10:12:00Z` as the phone's local date and time. */
export const formatTime = (ts?: string | null) => {
    if (!ts) return ''
    const date = new Date(ts)
    return Number.isNaN(date.getTime()) ? ts : date.toLocaleString()
}

export type MenuItem = {
    label: string
    icon?: AntDesignIconName
    destructive?: boolean
    onPress: () => void
}

/** A bottom sheet of actions, for a long press. The sheet closes before the action runs. */
export const MenuSheet: React.FC<{
    visible: boolean
    title?: string
    detail?: string
    items: MenuItem[]
    onDismiss: () => void
}> = ({ visible, title, detail, items, onDismiss }) => {
    const styles = useStyles()
    return (
        <Sheet visible={visible} onDismiss={onDismiss}>
            <View style={styles.body}>
                {!!title && <Text style={styles.title}>{title}</Text>}
                {!!detail && <Text style={styles.detail}>{detail}</Text>}
                <View>
                    {items.map((item) => (
                        <Row
                            key={item.label}
                            label={item.label}
                            icon={item.icon}
                            destructive={item.destructive}
                            chevron={false}
                            onPress={() => {
                                onDismiss()
                                item.onPress()
                            }}
                        />
                    ))}
                </View>
            </View>
        </Sheet>
    )
}

/** Ask for one line of text (a new name) in a bottom sheet. */
type PromptProps = {
    visible: boolean
    title: string
    label?: string
    initial?: string
    description?: string
    confirmLabel?: string
    onSubmit: (text: string) => void
    onDismiss: () => void
}

export const PromptSheet: React.FC<PromptProps> = (props) => (
    <Sheet visible={props.visible} onDismiss={props.onDismiss}>
        {/* Mounted per opening, so each starts from `initial`. */}
        {props.visible && <PromptBody {...props} />}
    </Sheet>
)

const PromptBody: React.FC<PromptProps> = ({
    title,
    label,
    initial = '',
    description,
    confirmLabel,
    onSubmit,
    onDismiss,
}) => {
    const styles = useStyles()
    const [text, setText] = useState(initial)
    const ready = text.trim().length > 0 && text.trim() !== initial.trim()
    return (
        <View style={styles.body}>
            <Text style={styles.title}>{title}</Text>
            <Field
                label={label}
                value={text}
                onChangeText={setText}
                description={description}
                autoFocus
                onSubmitEditing={() => ready && onSubmit(text.trim())}
            />
            <View style={styles.actions}>
                <ThemedButton label="Cancel" variant="secondary" onPress={onDismiss} />
                <ThemedButton
                    label={confirmLabel ?? 'Save'}
                    variant={ready ? 'primary' : 'disabled'}
                    onPress={() => ready && onSubmit(text.trim())}
                />
            </View>
        </View>
    )
}

const useStyles = () => {
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
        detail: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
    })
}
