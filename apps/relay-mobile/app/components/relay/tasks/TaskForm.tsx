import React, { useState } from 'react'
import { ScrollView, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import { Chip, Field, isCancelled, relay, Segmented, Sheet, useBusQuery } from '@components/relay'
import { useSheetStyles } from '@components/relay/Sheet'
import { Logger } from '@lib/state/Logger'

import { ChipPicker, TaskSearch, usePieceStyles } from './Pieces'
import {
    Column,
    COLUMNS,
    errorText,
    isConflict,
    Module,
    Priority,
    PRIORITIES,
    Size,
    SIZES,
    Task,
    TaskType,
    TYPES,
} from './types'

export type TaskDefaults = { column?: Column; module_id?: number; parent_id?: number }

type Draft = {
    title: string
    body: string
    type: TaskType
    priority: Priority
    size: Size | 'none'
    column: Column
    module_id?: number
    parent?: { id: number; title: string }
    labels: string
}

const fromTask = (task?: Task, defaults: TaskDefaults = {}): Draft => ({
    title: task?.title ?? '',
    body: task?.body ?? '',
    type: task?.type ?? 'task',
    priority: task?.priority ?? 'medium',
    size: task?.size ?? 'none',
    column: task?.column ?? defaults.column ?? 'backlog',
    module_id: task ? (task.module_id ?? undefined) : defaults.module_id,
    parent: defaults.parent_id ? { id: defaults.parent_id, title: '' } : undefined,
    labels: '',
})

const splitLabels = (text: string) =>
    Array.from(
        new Set(
            text
                .split(',')
                .map((label) => label.trim())
                .filter(Boolean)
        )
    )

/**
 * New task, or edit one (`task` given). An edit sends only what changed, with the values it
 * started from as `expected`: if someone changed the same field on the PC meanwhile, the
 * engine refuses (`task.edit_conflict`) and the sheet reloads the task and says so.
 */
export const TaskForm: React.FC<{
    visible: boolean
    onDismiss: () => void
    projectId: number
    task?: Task
    defaults?: TaskDefaults
    onSaved?: (task: Task) => void
    /** Called on an edit conflict so the owner reloads the task. */
    onConflict?: () => void
}> = ({ visible, onDismiss, projectId, task, defaults, onSaved, onConflict }) => {
    const sheet = useSheetStyles()
    const styles = usePieceStyles()
    const [draft, setDraft] = useState<Draft>(() => fromTask(task, defaults))
    const [busy, setBusy] = useState(false)
    const [pickParent, setPickParent] = useState(false)
    const editing = !!task
    const modules = useBusQuery<Module[]>(
        'module.list',
        { project_id: projectId },
        { select: (r) => r.modules, enabled: visible, refetchOnFocus: false }
    )
    const labels = useBusQuery<string[]>(
        'task.label.list',
        { project_id: projectId },
        {
            select: (r) => r.labels.map((l: { name: string }) => l.name),
            enabled: visible && !editing,
            refetchOnFocus: false,
        }
    )

    // A fresh draft each time the sheet opens, from the task as it is then. A change on the PC
    // while the sheet is open does not overwrite the draft; `expected` catches it on save.
    const key = visible ? `open:${task?.id}` : 'closed'
    const [draftFor, setDraftFor] = useState(key)
    if (key !== draftFor) {
        setDraftFor(key)
        if (visible) {
            setDraft(fromTask(task, defaults))
            setPickParent(false)
        }
    }

    const set = (patch: Partial<Draft>) => setDraft((prev) => ({ ...prev, ...patch }))

    const save = async () => {
        const title = draft.title.trim()
        if (!title) return Logger.errorToast('A task needs a title')
        const size = draft.size === 'none' ? null : draft.size
        setBusy(true)
        try {
            let saved: Task
            if (task) {
                const patch: Record<string, unknown> = {}
                const expected: Record<string, unknown> = {}
                const change = (field: string, next: unknown, before: unknown) => {
                    if (next === before) return
                    patch[field] = next
                    expected[field] = before
                }
                change('title', title, task.title)
                change('body', draft.body, task.body)
                change('type', draft.type, task.type)
                change('priority', draft.priority, task.priority)
                change('size', size, task.size)
                change('module_id', draft.module_id ?? null, task.module_id)
                if (Object.keys(patch).length === 0) {
                    onDismiss()
                    return
                }
                saved = await relay.guarded<Task>('task.update', {
                    task_id: task.id,
                    ...patch,
                    expected: expected,
                })
                Logger.infoToast('Saved')
            } else {
                const labelList = splitLabels(draft.labels)
                saved = await relay.guarded<Task>('task.create', {
                    project_id: projectId,
                    title: title,
                    body: draft.body,
                    type: draft.type,
                    priority: draft.priority,
                    ...(size ? { size: size } : {}),
                    column: draft.column,
                    ...(draft.module_id !== undefined ? { module_id: draft.module_id } : {}),
                    ...(draft.parent ? { parent_id: draft.parent.id } : {}),
                    ...(labelList.length > 0 ? { labels: labelList } : {}),
                })
                Logger.infoToast(`Created #${saved.id}`)
            }
            onSaved?.(saved)
            onDismiss()
        } catch (e) {
            if (isConflict(e)) {
                Logger.errorToast(`${errorText(e)}. Reloaded the task; check it and edit again.`)
                onConflict?.()
                onDismiss()
            } else if (!isCancelled(e)) Logger.errorToast(errorText(e))
        } finally {
            setBusy(false)
        }
    }

    const addLabel = (label: string) => {
        const list = splitLabels(draft.labels)
        if (!list.includes(label)) set({ labels: [...list, label].join(', ') })
    }

    return (
        <Sheet visible={visible} onDismiss={onDismiss}>
            <View style={[sheet.body, styles.shrink]}>
                <Text style={sheet.title}>{editing ? `Edit #${task?.id}` : 'New task'}</Text>
                <ScrollView
                    style={[sheet.scroll, styles.shrink]}
                    keyboardShouldPersistTaps="handled">
                    <View style={{ rowGap: 12 }}>
                        <Field
                            label="Title"
                            value={draft.title}
                            onChangeText={(text) => set({ title: text })}
                            autoFocus={!editing}
                        />
                        <Field
                            label="Description"
                            value={draft.body}
                            onChangeText={(text) => set({ body: text })}
                            multiline
                            lines={4}
                        />
                        <Text style={styles.label}>Type</Text>
                        <ChipPicker
                            options={TYPES}
                            value={draft.type}
                            onChange={(value) => value && set({ type: value })}
                        />
                        <Text style={styles.label}>Priority</Text>
                        <Segmented
                            options={PRIORITIES}
                            value={draft.priority}
                            onChange={(value) => set({ priority: value })}
                        />
                        <Text style={styles.label}>Size</Text>
                        <Segmented
                            options={SIZES}
                            value={draft.size}
                            onChange={(value) => set({ size: value })}
                        />
                        {!editing && (
                            <>
                                <Text style={styles.label}>Column</Text>
                                <Segmented
                                    options={COLUMNS.filter((c) => c.value !== 'done')}
                                    value={draft.column}
                                    onChange={(value) => set({ column: value })}
                                />
                            </>
                        )}
                        <Text style={styles.label}>Module</Text>
                        {modules.data && modules.data.length > 0 ? (
                            <ChipPicker
                                allowNone
                                options={modules.data.map((m) => ({ value: m.id, label: m.name }))}
                                value={draft.module_id}
                                onChange={(value) => set({ module_id: value })}
                            />
                        ) : (
                            <Text style={styles.meta}>
                                {modules.data ? 'No open modules.' : 'Loading modules…'}
                            </Text>
                        )}
                        {!editing && (
                            <>
                                <Field
                                    label="Labels"
                                    value={draft.labels}
                                    onChangeText={(text) => set({ labels: text })}
                                    placeholder="Comma separated"
                                    autoCapitalize="none"
                                />
                                {!!labels.data && labels.data.length > 0 && (
                                    <View style={styles.chips}>
                                        {labels.data
                                            .filter(
                                                (label) =>
                                                    !splitLabels(draft.labels).includes(label)
                                            )
                                            .slice(0, 12)
                                            .map((label) => (
                                                <Chip
                                                    key={label}
                                                    label={label}
                                                    icon="plus"
                                                    onPress={() => addLabel(label)}
                                                />
                                            ))}
                                    </View>
                                )}
                                <Text style={styles.label}>Parent task</Text>
                                {draft.parent ? (
                                    <View style={styles.chips}>
                                        <Chip
                                            label={`#${draft.parent.id} ${draft.parent.title}`.trim()}
                                            icon="close"
                                            onPress={() => set({ parent: undefined })}
                                        />
                                    </View>
                                ) : pickParent ? (
                                    <TaskSearch
                                        projectId={projectId}
                                        onPick={(picked) => {
                                            set({ parent: { id: picked.id, title: picked.title } })
                                            setPickParent(false)
                                        }}
                                    />
                                ) : (
                                    <View style={styles.chips}>
                                        <Chip
                                            label="None · choose"
                                            icon="apartment"
                                            onPress={() => setPickParent(true)}
                                        />
                                    </View>
                                )}
                            </>
                        )}
                    </View>
                </ScrollView>
                <View style={sheet.actions}>
                    <ThemedButton label="Cancel" variant="secondary" onPress={onDismiss} />
                    <ThemedButton
                        label={busy ? 'Saving…' : editing ? 'Save' : 'Create'}
                        variant={busy ? 'disabled' : 'primary'}
                        onPress={busy ? undefined : save}
                    />
                </View>
            </View>
        </Sheet>
    )
}
