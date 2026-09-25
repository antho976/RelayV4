import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import DropdownSheet from '@components/input/DropdownSheet'
import {
    Chip,
    confirm,
    EmptyState,
    Field,
    QueryView,
    relay,
    RelayProject,
    Screen,
    Segmented,
    useBusQuery,
    useRelayStore,
} from '@components/relay'
import {
    attempt,
    Choice,
    ChoiceSheet,
    ChipPicker,
    Column,
    COLUMNS,
    columnLabel,
    DispatchSheet,
    Module,
    offerUndo,
    Priority,
    PRIORITIES,
    Task,
    TaskCard,
    TaskForm,
    taskHref,
    TaskType,
    TYPES,
    UndoBar,
    undoLatest,
} from '@components/relay/tasks'
import { Theme } from '@lib/theme/ThemeManager'

const PAGE = 40

type Filters = {
    search: string
    type?: TaskType
    priority?: Priority
    module?: number
    label?: string
}

/**
 * The project board from the phone: one column at a time with its count, filters, and every
 * board action the desktop has — create, move, dispatch, approve, delete (with undo). A tap
 * opens the task; a long press opens its menu.
 */
const BoardScreen = () => {
    const styles = useStyles()
    const router = useRouter()
    const projects = useRelayStore((state) => state.projects)
    const params = useLocalSearchParams<{ project?: string; project_id?: string }>()
    const wanted = params.project_id ?? params.project
    const [project, setProject] = useState<RelayProject | undefined>(() =>
        useRelayStore.getState().projects.find((item) => String(item.id) === wanted)
    )
    // The chosen project, or the first one when the choice is stale (removed on the PC) or unmade.
    const current =
        (project && projects.find((item) => item.id === project.id)) ??
        projects.find((item) => String(item.id) === wanted) ??
        projects[0]
    const projectId = current?.id

    const tasks = useBusQuery<Task[]>(
        'task.list',
        { project_id: projectId },
        {
            select: (r) => r.tasks,
            events: ['task.*', 'module.*'],
            projectId: projectId,
            enabled: projectId !== undefined,
        }
    )
    const modules = useBusQuery<Module[]>(
        'module.list',
        { project_id: projectId },
        {
            select: (r) => r.modules,
            events: ['module.*'],
            projectId: projectId,
            enabled: projectId !== undefined,
        }
    )

    const [column, setColumn] = useState<Column | undefined>(undefined)
    const [filters, setFilters] = useState<Filters>({ search: '' })
    const [showFilters, setShowFilters] = useState(false)
    const [limit, setLimit] = useState(PAGE)
    const [creating, setCreating] = useState(false)
    const [dispatching, setDispatching] = useState<Task | undefined>(undefined)
    const [menu, setMenu] = useState<{ title: string; choices: Choice[] } | undefined>(undefined)

    const all = tasks.data ?? []
    const needle = filters.search.trim().toLowerCase().replace(/^#/, '')
    const filtered = all.filter(
        (task) =>
            (!needle ||
                String(task.id) === needle ||
                task.title.toLowerCase().includes(needle) ||
                task.body.toLowerCase().includes(needle)) &&
            (!filters.type || task.type === filters.type) &&
            (!filters.priority || task.priority === filters.priority) &&
            (filters.module === undefined || task.module_id === filters.module) &&
            (!filters.label || task.labels.includes(filters.label))
    )
    const count = (key: Column) => filtered.filter((task) => task.column === key).length
    // Until the person picks, open on the first column with work in it, review first.
    const shown =
        column ??
        (['in_review', 'active', 'ready', 'backlog'] as Column[]).find((key) => count(key) > 0) ??
        'backlog'
    const inColumn = filtered.filter((task) => task.column === shown)
    const labels = Array.from(new Set(all.flatMap((task) => task.labels))).sort()
    const active =
        (filters.type ? 1 : 0) +
        (filters.priority ? 1 : 0) +
        (filters.module !== undefined ? 1 : 0) +
        (filters.label ? 1 : 0)

    const move = async (task: Task, to: Column) => {
        const from = task.column
        const position = task.position
        const moved = await attempt(() =>
            relay.guarded<Task>('task.move', { task_id: task.id, column: to })
        )
        if (!moved) return
        tasks.reload()
        offerUndo(`#${task.id} moved to ${columnLabel(to)}`, () =>
            relay.guarded('task.move', { task_id: task.id, column: from, position: position })
        )
    }

    const approve = async (task: Task) => {
        const yes = await confirm({
            title: `Approve #${task.id}?`,
            message: `"${task.title}" moves to Done; the branch head is linked as its commit.`,
            confirmLabel: 'Approve',
        })
        if (!yes) return
        const done = await attempt(() => relay.guarded<Task>('task.approve', { task_id: task.id }))
        if (!done) return
        tasks.reload()
        offerUndo(`#${task.id} approved`, () => undoLatest(task.project_id, 'task.approve'))
    }

    const remove = async (task: Task) => {
        const yes = await confirm({
            title: `Delete #${task.id}?`,
            message: task.title,
            confirmLabel: 'Delete',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(() => relay.guarded('task.delete', { task_id: task.id }))
        if (done === undefined) return
        tasks.reload()
        offerUndo(`#${task.id} deleted`, () => relay.guarded('task.restore', { task_id: task.id }))
    }

    const openMenu = (task: Task) => {
        const choices: Choice[] = [
            { label: 'Open', icon: 'file-text', onPress: () => router.push(taskHref(task.id)) },
            {
                label: 'Move to…',
                icon: 'swap',
                onPress: () =>
                    setMenu({
                        title: `Move #${task.id} to`,
                        choices: COLUMNS.filter(
                            (c) => c.value !== task.column && c.value !== 'done'
                        ).map((c) => ({
                            label: columnLabel(c.value),
                            onPress: () => move(task, c.value),
                        })),
                    }),
            },
        ]
        if (task.column === 'in_review')
            choices.push({ label: 'Approve', icon: 'check', onPress: () => approve(task) })
        if (task.column !== 'done')
            choices.push({
                label: 'Dispatch…',
                icon: 'rocket',
                onPress: () => setDispatching(task),
            })
        const session = task.sessions[task.sessions.length - 1]
        if (session)
            choices.push({
                label: 'Terminal',
                detail: session,
                icon: 'code',
                onPress: () =>
                    router.push({
                        pathname: '/screens/RelayScreen/Terminal',
                        params: { session: session },
                    }),
            })
        choices.push({
            label: 'Delete',
            icon: 'delete',
            destructive: true,
            onPress: () => remove(task),
        })
        setMenu({ title: `#${task.id} ${task.title}`, choices: choices })
    }

    const cardActions = (task: Task) =>
        task.column === 'in_review'
            ? [{ label: 'Approve', onPress: () => approve(task) }]
            : task.column === 'backlog' || task.column === 'ready'
              ? [{ label: 'Dispatch', onPress: () => setDispatching(task) }]
              : undefined

    return (
        <Screen
            title="Board"
            onRefresh={tasks.reload}
            refreshing={tasks.loading && tasks.data !== undefined}
            footer={<UndoBar />}
            actions={[
                {
                    icon: 'filter',
                    label: 'Filters',
                    onPress: () => setShowFilters((v) => !v),
                },
                {
                    icon: 'plus',
                    label: 'New task',
                    disabled: projectId === undefined,
                    onPress: () => setCreating(true),
                },
            ]}>
            {!wanted && projects.length > 1 && (
                <DropdownSheet
                    data={projects}
                    selected={current}
                    onChangeValue={setProject}
                    labelExtractor={(item) => item.name}
                    placeholder="Project"
                    modalTitle="Project"
                    search={projects.length > 6}
                />
            )}
            {projectId === undefined ? (
                <EmptyState icon="project" title="No project" text="Add a project on the PC." />
            ) : (
                <>
                    <Segmented
                        options={COLUMNS.map((c) => ({
                            value: c.value,
                            label: `${c.label} ${count(c.value)}`,
                        }))}
                        value={shown}
                        onChange={(value) => {
                            setColumn(value)
                            setLimit(PAGE)
                        }}
                    />
                    <Field
                        value={filters.search}
                        onChangeText={(text) => setFilters((f) => ({ ...f, search: text }))}
                        placeholder="Search title, description or #id"
                        autoCorrect={false}
                    />
                    {showFilters && (
                        <View style={styles.filters}>
                            <Text style={styles.label}>Type</Text>
                            <ChipPicker
                                allowNone
                                options={TYPES}
                                value={filters.type}
                                onChange={(value) => setFilters((f) => ({ ...f, type: value }))}
                            />
                            <Text style={styles.label}>Priority</Text>
                            <ChipPicker
                                allowNone
                                options={PRIORITIES}
                                value={filters.priority}
                                onChange={(value) => setFilters((f) => ({ ...f, priority: value }))}
                            />
                            {!!modules.data && modules.data.length > 0 && (
                                <>
                                    <Text style={styles.label}>Module</Text>
                                    <ChipPicker
                                        allowNone
                                        options={modules.data.map((m) => ({
                                            value: m.id,
                                            label: m.name,
                                        }))}
                                        value={filters.module}
                                        onChange={(value) =>
                                            setFilters((f) => ({ ...f, module: value }))
                                        }
                                    />
                                </>
                            )}
                            {labels.length > 0 && (
                                <>
                                    <Text style={styles.label}>Label</Text>
                                    <ChipPicker
                                        allowNone
                                        options={labels.map((l) => ({ value: l, label: l }))}
                                        value={filters.label}
                                        onChange={(value) =>
                                            setFilters((f) => ({ ...f, label: value }))
                                        }
                                    />
                                </>
                            )}
                        </View>
                    )}
                    {!showFilters && active > 0 && (
                        <View style={styles.chipRow}>
                            <Chip
                                label={`${active} filter${active > 1 ? 's' : ''} on · clear`}
                                icon="close"
                                tone="primary"
                                selected
                                onPress={() => setFilters((f) => ({ search: f.search }))}
                            />
                        </View>
                    )}
                    <QueryView
                        query={tasks}
                        isEmpty={() => inColumn.length === 0}
                        empty={
                            <EmptyState
                                icon="project"
                                title={`Nothing in ${columnLabel(shown)}`}
                                text={
                                    filtered.length < all.length
                                        ? 'Some tasks are hidden by the filters.'
                                        : undefined
                                }
                                action={
                                    shown !== 'done'
                                        ? { label: 'New task', onPress: () => setCreating(true) }
                                        : undefined
                                }
                            />
                        }>
                        {() => (
                            <View style={styles.list}>
                                {inColumn.slice(0, limit).map((task) => (
                                    <TaskCard
                                        key={task.id}
                                        task={task}
                                        onPress={() => router.push(taskHref(task.id))}
                                        onLongPress={() => openMenu(task)}
                                        actions={cardActions(task)}
                                    />
                                ))}
                                {inColumn.length > limit && (
                                    <Chip
                                        label={`Show ${Math.min(PAGE, inColumn.length - limit)} more of ${inColumn.length - limit}`}
                                        onPress={() => setLimit((n) => n + PAGE)}
                                    />
                                )}
                                <Text style={styles.hint}>Long-press a task to move it.</Text>
                            </View>
                        )}
                    </QueryView>
                </>
            )}
            {projectId !== undefined && (
                <TaskForm
                    visible={creating}
                    onDismiss={() => setCreating(false)}
                    projectId={projectId}
                    defaults={{
                        column: shown === 'done' || shown === 'active' ? 'backlog' : shown,
                        module_id: filters.module,
                    }}
                    onSaved={() => tasks.reload()}
                />
            )}
            <DispatchSheet task={dispatching} onDismiss={() => setDispatching(undefined)} />
            <ChoiceSheet
                title={menu?.title}
                choices={menu?.choices}
                onDismiss={() => setMenu(undefined)}
            />
        </Screen>
    )
}

export default BoardScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        filters: {
            rowGap: spacing.m,
        },
        label: {
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
        },
        chipRow: {
            flexDirection: 'row',
        },
        list: {
            rowGap: spacing.m,
        },
        hint: {
            color: color.text._500,
            fontSize: fontSize.s,
            textAlign: 'center',
        },
    })
}
