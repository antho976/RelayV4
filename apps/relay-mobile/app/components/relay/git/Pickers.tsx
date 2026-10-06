import { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import React, { useEffect, useState } from 'react'
import { ScrollView, StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import { useBusQuery } from '@components/relay/hooks'
import { Chip, Field, Row } from '@components/relay/Kit'
import { Sheet, useSheetStyles } from '@components/relay/Sheet'
import { Theme } from '@lib/theme/ThemeManager'

import { baseName, Worktree } from './types'

/** Bottom sheets are modals; one opening while another closes can be dropped, so wait it out. */
export const afterSheet = (fn: () => void) => setTimeout(fn, 350)

/**
 * The project's worktrees as chips: the checkout and each session's own. Picks the first
 * (the project checkout) when nothing is chosen yet, or when the chosen one is gone.
 */
export const WorktreePicker: React.FC<{
    projectId: number
    value: string | undefined
    onChange: (path: string, worktree: Worktree) => void
}> = ({ projectId, value, onChange }) => {
    const styles = useStyles()
    const query = useBusQuery<Worktree[]>(
        'worktree.list',
        { project_id: projectId },
        {
            events: ['worktree.*', 'session.*'],
            projectId: projectId,
            select: (raw) => raw.worktrees,
        }
    )
    const worktrees = query.data
    useEffect(() => {
        if (!worktrees || worktrees.length === 0) return
        if (value === undefined || !worktrees.some((item) => item.path === value))
            onChange(worktrees[0].path, worktrees[0])
    }, [worktrees, value, onChange])

    if (!worktrees) {
        return query.error ? (
            <Text style={styles.problem}>{query.error.message}</Text>
        ) : (
            <Text style={styles.note}>Loading worktrees…</Text>
        )
    }
    return (
        <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={styles.chips}>
            {worktrees.map((item, index) => (
                <Chip
                    key={item.path}
                    label={`${item.session ?? (index === 0 ? 'checkout' : baseName(item.path))} · ${
                        item.branch || 'detached'
                    }${item.dirty ? ' •' : ''}`}
                    icon={item.session ? 'user' : 'branches'}
                    tone={item.path === value ? 'primary' : 'neutral'}
                    selected={item.path === value}
                    onPress={() => onChange(item.path, item)}
                />
            ))}
        </ScrollView>
    )
}

export type SheetAction = {
    label: string
    icon?: AntDesignIconName
    detail?: string
    destructive?: boolean
    disabled?: boolean
    onPress: () => void
}

/** A list of actions in a bottom sheet; choosing one closes the sheet, then runs it. */
export const ActionSheet: React.FC<{
    visible: boolean
    title: string
    detail?: string
    actions: SheetAction[]
    onDismiss: () => void
}> = ({ visible, title, detail, actions, onDismiss }) => {
    const styles = useSheetStyles()
    return (
        <Sheet visible={visible} onDismiss={onDismiss}>
            <View style={styles.body}>
                <Text numberOfLines={2} style={styles.title}>
                    {title}
                </Text>
                {!!detail && <Text style={styles.message}>{detail}</Text>}
                <View>
                    {actions.map((action) => (
                        <Row
                            key={action.label}
                            label={action.label}
                            detail={action.detail}
                            icon={action.icon}
                            destructive={action.destructive}
                            disabled={action.disabled}
                            chevron={false}
                            onPress={() => {
                                onDismiss()
                                afterSheet(action.onPress)
                            }}
                        />
                    ))}
                </View>
            </View>
        </Sheet>
    )
}

/**
 * Ask for one line of text (a name, a path) in a bottom sheet. `onSubmit` returns true to
 * close; a failure keeps the sheet and the typed text.
 */
export const PromptSheet: React.FC<{
    visible: boolean
    title: string
    message?: string
    label?: string
    initial?: string
    placeholder?: string
    confirmLabel?: string
    busy?: boolean
    onSubmit: (value: string) => Promise<boolean> | boolean
    onDismiss: () => void
}> = ({
    visible,
    title,
    message,
    label,
    initial = '',
    placeholder,
    confirmLabel = 'Save',
    busy,
    onSubmit,
    onDismiss,
}) => {
    const styles = useSheetStyles()
    const [value, setValue] = useState(initial)
    // Each opening starts from `initial` again.
    const [shown, setShown] = useState(false)
    if (visible !== shown) {
        setShown(visible)
        if (visible) setValue(initial)
    }
    const submit = async () => {
        if (!value.trim() || busy) return
        if (await onSubmit(value.trim())) onDismiss()
    }
    return (
        <Sheet visible={visible} onDismiss={onDismiss}>
            <View style={styles.body}>
                <Text style={styles.title}>{title}</Text>
                {!!message && <Text style={styles.message}>{message}</Text>}
                <Field
                    label={label}
                    value={value}
                    onChangeText={setValue}
                    placeholder={placeholder}
                    autoCapitalize="none"
                    autoCorrect={false}
                    autoFocus
                    mono
                    onSubmitEditing={submit}
                />
                <View style={styles.actions}>
                    <ThemedButton label="Cancel" variant="secondary" onPress={onDismiss} />
                    <ThemedButton
                        label={busy ? 'Working…' : confirmLabel}
                        variant={value.trim() && !busy ? 'primary' : 'disabled'}
                        onPress={submit}
                    />
                </View>
            </View>
        </Sheet>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        chips: {
            columnGap: spacing.s,
            paddingVertical: 2,
        },
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        problem: {
            color: color.error._300,
            fontSize: fontSize.s,
        },
    })
}
