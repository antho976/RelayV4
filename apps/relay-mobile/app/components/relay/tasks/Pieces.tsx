import { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { Chip, Field, relay, Row, Sheet, Tone, useBusQuery } from '@components/relay'
import { useSheetStyles } from '@components/relay/Sheet'
import { Theme } from '@lib/theme/ThemeManager'

import { columnLabel, priorityTone, stateLabel, stateTone, Task, TYPES } from './types'

export type Choice = {
    label: string
    detail?: string
    icon?: AntDesignIconName
    destructive?: boolean
    disabled?: boolean
    onPress: () => void
}

/**
 * A menu in a bottom sheet: a title and a list of rows. Picking one closes the sheet first,
 * then runs it, so the action may open another sheet.
 */
export const ChoiceSheet: React.FC<{
    title?: string
    choices?: Choice[]
    onDismiss: () => void
}> = ({ title, choices, onDismiss }) => {
    const sheet = useSheetStyles()
    const styles = usePieceStyles()
    return (
        <Sheet visible={!!choices} onDismiss={onDismiss}>
            <View style={[sheet.body, styles.shrink]}>
                {!!title && (
                    <Text numberOfLines={2} style={sheet.title}>
                        {title}
                    </Text>
                )}
                <ScrollView style={[sheet.scroll, styles.shrink]}>
                    {(choices ?? []).map((choice) => (
                        <Row
                            key={choice.label}
                            label={choice.label}
                            detail={choice.detail}
                            icon={choice.icon}
                            destructive={choice.destructive}
                            disabled={choice.disabled}
                            chevron={false}
                            onPress={() => {
                                onDismiss()
                                // Let the sheet close before the next one opens.
                                setTimeout(choice.onPress, 250)
                            }}
                        />
                    ))}
                </ScrollView>
            </View>
        </Sheet>
    )
}

/** A wrapped row of chips; one (or none, with `allowNone`) is selected. */
export function ChipPicker<T extends string | number>({
    options,
    value,
    onChange,
    allowNone,
}: {
    options: { value: T; label: string }[]
    value: T | undefined
    onChange: (value: T | undefined) => void
    allowNone?: boolean
}) {
    const styles = usePieceStyles()
    return (
        <View style={styles.chips}>
            {options.map((option) => (
                <Chip
                    key={String(option.value)}
                    label={option.label}
                    tone={option.value === value ? 'primary' : 'neutral'}
                    selected={option.value === value}
                    onPress={() =>
                        onChange(allowNone && option.value === value ? undefined : option.value)
                    }
                />
            ))}
        </View>
    )
}

/** A task as a card: id, title, and the facts that decide what to do with it. */
export const TaskCard: React.FC<{
    task: Task
    onPress?: () => void
    onLongPress?: () => void
    showColumn?: boolean
    actions?: { label: string; tone?: Tone; onPress: () => void }[]
}> = ({ task, onPress, onLongPress, showColumn, actions }) => {
    const styles = usePieceStyles()
    const session = task.sessions[task.sessions.length - 1]
    return (
        <TouchableOpacity
            style={styles.card}
            onPress={onPress}
            onLongPress={onLongPress}
            disabled={!onPress && !onLongPress}>
            <View style={styles.cardHead}>
                <Text style={styles.id}>#{task.id}</Text>
                <Text numberOfLines={2} style={styles.title}>
                    {task.title}
                </Text>
            </View>
            <View style={styles.chips}>
                {showColumn && <Chip label={columnLabel(task.column)} />}
                {task.type !== 'task' && (
                    <Chip
                        label={TYPES.find((t) => t.value === task.type)?.label ?? task.type}
                        tone={task.type === 'bug' ? 'danger' : 'neutral'}
                    />
                )}
                {task.priority !== 'medium' && (
                    <Chip label={task.priority} tone={priorityTone(task.priority)} />
                )}
                {!!task.size && <Chip label={task.size} />}
                {task.state !== 'none' && (
                    <Chip label={stateLabel(task.state)} tone={stateTone(task.state)} />
                )}
                {task.rollup.total > 0 && (
                    <Chip
                        label={`${task.rollup.done}/${task.rollup.total} done`}
                        icon="apartment"
                    />
                )}
                {task.blocked_by.length > 0 && <Chip label="blocked" tone="warn" icon="lock" />}
                {task.labels.map((label) => (
                    <Chip key={label} label={label} icon="tag" />
                ))}
            </View>
            {!!session && <Text style={styles.meta}>{session}</Text>}
            {actions && actions.length > 0 && (
                <View style={styles.actions}>
                    {actions.map((action) => (
                        <Chip
                            key={action.label}
                            label={action.label}
                            tone={action.tone ?? 'primary'}
                            selected
                            onPress={action.onPress}
                        />
                    ))}
                </View>
            )}
        </TouchableOpacity>
    )
}

/**
 * Pick another task of the project by title or #id, inline (it lives inside sheets, which
 * cannot stack another sheet).
 */
export const TaskSearch: React.FC<{
    projectId: number
    exclude?: number[]
    placeholder?: string
    onPick: (task: Task) => void
}> = ({ projectId, exclude = [], placeholder, onPick }) => {
    const styles = usePieceStyles()
    const [query, setQuery] = useState('')
    const tasks = useBusQuery<Task[]>(
        'task.list',
        { project_id: projectId, sort: 'updated' },
        { select: (r) => r.tasks, refetchOnFocus: false }
    )
    const needle = query.trim().toLowerCase().replace(/^#/, '')
    const matches = (tasks.data ?? [])
        .filter((task) => !exclude.includes(task.id))
        .filter(
            (task) =>
                !needle || String(task.id) === needle || task.title.toLowerCase().includes(needle)
        )
        .slice(0, 6)
    return (
        <View style={styles.search}>
            <Field
                value={query}
                onChangeText={setQuery}
                placeholder={placeholder ?? 'Search tasks by title or #id'}
                autoCorrect={false}
            />
            {tasks.data === undefined && <Text style={styles.meta}>Loading tasks…</Text>}
            {matches.map((task) => (
                <Row
                    key={task.id}
                    label={`#${task.id} ${task.title}`}
                    detail={columnLabel(task.column)}
                    chevron={false}
                    onPress={() => onPick(task)}
                />
            ))}
            {tasks.data !== undefined && matches.length === 0 && (
                <Text style={styles.meta}>No matching task.</Text>
            )}
        </View>
    )
}

/** Bytes as a short size. */
export const formatBytes = (bytes: number) =>
    bytes < 1024
        ? `${bytes} B`
        : bytes < 1024 * 1024
          ? `${(bytes / 1024).toFixed(0)} KB`
          : `${(bytes / 1024 / 1024).toFixed(1)} MB`

/** The latest undoable action of the person on this project whose op starts with `prefix`. */
export const undoLatest = async (projectId: number, prefix: string) => {
    const { rows } = await relay.call<{ rows: any[] }>('audit.list', {
        project_id: projectId,
        actor: 'user',
        op_prefix: prefix,
        limit: 20,
    })
    const row = rows.find((item) => item.undo_op && !item.undone_by && item.kind === 'ok')
    if (!row) throw new Error('Nothing to undo')
    return relay.guarded('audit.undo', { audit_id: row.id })
}

export const usePieceStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        shrink: {
            flexShrink: 1,
        },
        chips: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            columnGap: spacing.s,
            rowGap: spacing.s,
        },
        card: {
            backgroundColor: color.neutral._200,
            borderRadius: 14,
            padding: spacing.l,
            rowGap: spacing.m,
        },
        cardHead: {
            flexDirection: 'row',
            columnGap: spacing.m,
            alignItems: 'flex-start',
        },
        id: {
            color: color.text._500,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
            paddingTop: 2,
        },
        title: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.m,
        },
        meta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
        search: {
            rowGap: spacing.xs,
        },
        label: {
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
        },
        body: {
            color: color.text._200,
            lineHeight: 20,
        },
        buttons: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            columnGap: spacing.m,
            rowGap: spacing.m,
        },
    })
}
