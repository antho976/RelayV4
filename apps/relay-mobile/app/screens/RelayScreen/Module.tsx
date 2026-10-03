import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useState } from 'react'
import { ScrollView, Share, StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    ErrorState,
    Field,
    isCancelled,
    LoadingState,
    Mono,
    relay,
    RelayRequestError,
    Screen,
    Section,
    Segmented,
    Sheet,
    useBusQuery,
} from '@components/relay'
import { useSheetStyles } from '@components/relay/Sheet'
import {
    attempt,
    Column,
    columnLabel,
    errorText,
    isConflict,
    Module,
    offerUndo,
    Priority,
    PRIORITIES,
    priorityTone,
    shortTime,
    Task,
    TaskCard,
    TaskForm,
    taskHref,
    UndoBar,
    usePieceStyles,
} from '@components/relay/tasks'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type ModuleDetail = Module & { tasks_by_state: Partial<Record<Column, Task[]>> }

/** Review first, then what is moving, then what waits, then what shipped. */
const ORDER: Column[] = ['in_review', 'active', 'ready', 'backlog', 'done']

/**
 * One module: its tasks by column, edit (with conflict detection), complete or reopen,
 * delete with undo, a new task inside it, and patch notes drafted from its done tasks.
 */
const ModuleScreen = () => {
    const router = useRouter()
    const styles = useStyles()
    const pieces = usePieceStyles()
    const sheet = useSheetStyles()
    const params = useLocalSearchParams<{ module_id?: string }>()
    const parsed = Number(params.module_id)
    const moduleId = Number.isFinite(parsed) ? parsed : undefined
    const query = useBusQuery<ModuleDetail>(
        'module.get',
        { module_id: moduleId },
        { events: ['module.*', 'task.*'], enabled: moduleId !== undefined }
    )
    const module = query.data
    const [editing, setEditing] = useState(false)
    const [name, setName] = useState('')
    const [icon, setIcon] = useState('')
    const [priority, setPriority] = useState<Priority>('medium')
    const [adding, setAdding] = useState(false)
    const [notes, setNotes] = useState<string | undefined>(undefined)
    const [drafting, setDrafting] = useState(false)
    const [gone, setGone] = useState(false)

    if (moduleId === undefined) {
        return (
            <Screen title="Module">
                <EmptyState icon="appstore" title="No module" />
            </Screen>
        )
    }

    const notFound =
        query.error instanceof RelayRequestError && query.error.error.kind === 'not_found'
    if (gone || notFound) {
        return (
            <Screen title="Module" footer={<UndoBar />}>
                <EmptyState
                    icon="delete"
                    title="Module deleted"
                    text="Its tasks are still on the board."
                    action={{
                        label: 'Restore',
                        onPress: async () => {
                            const back = await attempt(
                                () => relay.guarded('module.restore', { module_id: moduleId }),
                                'Restored'
                            )
                            if (back === undefined) return
                            setGone(false)
                            query.reload()
                        },
                    }}
                />
            </Screen>
        )
    }
    if (!module) {
        return (
            <Screen title="Module">
                {query.error ? (
                    <ErrorState error={query.error} onRetry={query.reload} />
                ) : (
                    <LoadingState />
                )}
            </Screen>
        )
    }

    const tasks = ORDER.flatMap((column) => module.tasks_by_state[column] ?? [])
    const done = (module.tasks_by_state.done ?? []).length
    const archived = !!module.completed_at

    const startEdit = () => {
        setName(module.name)
        setIcon(module.icon ?? '')
        setPriority(module.priority)
        setEditing(true)
    }

    const saveEdit = async () => {
        const patch: Record<string, unknown> = {}
        const expected: Record<string, unknown> = {}
        const change = (field: string, next: unknown, before: unknown) => {
            if (next === before) return
            patch[field] = next
            expected[field] = before
        }
        change('name', name.trim(), module.name)
        change('icon', icon.trim() || null, module.icon)
        change('priority', priority, module.priority)
        if (Object.keys(patch).length === 0) return setEditing(false)
        if (!name.trim()) return Logger.errorToast('A module needs a name')
        try {
            await relay.guarded('module.update', {
                module_id: module.id,
                ...patch,
                expected: expected,
            })
            Logger.infoToast('Saved')
            setEditing(false)
            query.reload()
        } catch (e) {
            if (isConflict(e)) {
                Logger.errorToast(`${errorText(e)}. Reloaded the module; edit again.`)
                setEditing(false)
                query.reload()
            } else if (!isCancelled(e)) Logger.errorToast(errorText(e))
        }
    }

    const toggleComplete = async () => {
        const op = archived ? 'module.reopen' : 'module.complete'
        const result = await attempt(
            () => relay.guarded(op, { module_id: module.id }),
            archived ? 'Reopened' : 'Completed'
        )
        if (!result) return
        query.reload()
        offerUndo(archived ? 'Module reopened' : 'Module completed', () =>
            relay.guarded(archived ? 'module.complete' : 'module.reopen', { module_id: module.id })
        )
    }

    const remove = async () => {
        const yes = await confirm({
            title: `Delete ${module.name}?`,
            message: 'Its tasks stay on the board, without a module.',
            confirmLabel: 'Delete',
            destructive: true,
        })
        if (!yes) return
        const result = await attempt(() => relay.guarded('module.delete', { module_id: module.id }))
        if (result === undefined) return
        offerUndo(`${module.name} deleted`, () =>
            relay.guarded('module.restore', { module_id: module.id })
        )
        if (router.canGoBack()) router.back()
        else setGone(true)
    }

    const draft = async () => {
        setDrafting(true)
        const result = await attempt(() =>
            relay.call<{ markdown: string; tasks: number[] }>('module.changelog.draft', {
                module_id: module.id,
            })
        )
        setDrafting(false)
        if (result) setNotes(result.markdown)
    }

    return (
        <Screen
            title={module.icon ? `${module.icon} ${module.name}` : module.name}
            onRefresh={query.reload}
            refreshing={query.loading}
            footer={<UndoBar />}
            actions={[
                { icon: 'edit', label: 'Edit', onPress: startEdit },
                { icon: 'plus', label: 'Add task', onPress: () => setAdding(true) },
            ]}>
            <View style={pieces.chips}>
                <Chip label={module.priority} tone={priorityTone(module.priority)} />
                <Chip label={`${done}/${tasks.length} done`} />
                {archived && (
                    <Chip
                        label={`completed ${shortTime(module.completed_at)}`}
                        tone="primary"
                        icon="check"
                    />
                )}
            </View>
            <View style={pieces.buttons}>
                <ThemedButton
                    label={archived ? 'Reopen' : 'Complete'}
                    variant="secondary"
                    onPress={toggleComplete}
                />
                <ThemedButton
                    label={drafting ? 'Drafting…' : 'Patch notes'}
                    variant="secondary"
                    onPress={drafting ? undefined : draft}
                />
                <ThemedButton label="Delete" variant="critical" onPress={remove} />
            </View>
            {tasks.length === 0 ? (
                <EmptyState
                    icon="project"
                    title="No tasks in this module"
                    action={{ label: 'Add task', onPress: () => setAdding(true) }}
                />
            ) : (
                ORDER.map((column) => {
                    const items = module.tasks_by_state[column] ?? []
                    if (items.length === 0) return null
                    return (
                        <Section
                            key={column}
                            title={`${columnLabel(column)} · ${items.length}`}
                            card={false}>
                            <View style={styles.list}>
                                {items.map((task) => (
                                    <TaskCard
                                        key={task.id}
                                        task={task}
                                        onPress={() => router.push(taskHref(task.id))}
                                    />
                                ))}
                            </View>
                        </Section>
                    )
                })
            )}

            <TaskForm
                visible={adding}
                onDismiss={() => setAdding(false)}
                projectId={module.project_id}
                defaults={{ module_id: module.id, column: 'backlog' }}
                onSaved={() => query.reload()}
            />
            <Sheet visible={editing} onDismiss={() => setEditing(false)}>
                <View style={[sheet.body, pieces.shrink]}>
                    <Text style={sheet.title}>Edit module</Text>
                    <Field label="Name" value={name} onChangeText={setName} />
                    <Field
                        label="Icon"
                        value={icon}
                        onChangeText={setIcon}
                        placeholder="An emoji, or empty"
                    />
                    <Text style={pieces.label}>Priority</Text>
                    <Segmented options={PRIORITIES} value={priority} onChange={setPriority} />
                    <View style={sheet.actions}>
                        <ThemedButton
                            label="Cancel"
                            variant="secondary"
                            onPress={() => setEditing(false)}
                        />
                        <ThemedButton label="Save" onPress={saveEdit} />
                    </View>
                </View>
            </Sheet>
            <Sheet visible={notes !== undefined} onDismiss={() => setNotes(undefined)}>
                <View style={[sheet.body, pieces.shrink]}>
                    <Text style={sheet.title}>Patch notes</Text>
                    <ScrollView style={[sheet.scroll, pieces.shrink]}>
                        {notes?.trim() ? (
                            <Mono>{notes}</Mono>
                        ) : (
                            <Text style={pieces.meta}>
                                No done task in this module has a changelog line yet.
                            </Text>
                        )}
                    </ScrollView>
                    <View style={sheet.actions}>
                        <ThemedButton
                            label="Close"
                            variant="secondary"
                            onPress={() => setNotes(undefined)}
                        />
                        <ThemedButton
                            label="Share"
                            variant={notes?.trim() ? 'primary' : 'disabled'}
                            onPress={
                                notes?.trim()
                                    ? () =>
                                          Share.share({
                                              title: `${module.name} patch notes`,
                                              message: notes,
                                          }).catch((e) => Logger.errorToast(errorText(e)))
                                    : undefined
                            }
                        />
                    </View>
                </View>
            </Sheet>
        </Screen>
    )
}

export default ModuleScreen

const useStyles = () => {
    const { spacing } = Theme.useTheme()
    return StyleSheet.create({
        list: {
            rowGap: spacing.m,
        },
    })
}
