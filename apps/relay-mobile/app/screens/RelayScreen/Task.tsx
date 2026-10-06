import {
    ImagePickerOptions,
    ImagePickerResult,
    launchCameraAsync,
    launchImageLibraryAsync,
    requestCameraPermissionsAsync,
    requestMediaLibraryPermissionsAsync,
} from 'expo-image-picker'
import { useFocusEffect, useLocalSearchParams, useRouter } from 'expo-router'
import React, { useCallback, useRef, useState } from 'react'
import { ScrollView, StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Caption,
    Chip,
    confirm,
    EmptyState,
    ErrorState,
    Field,
    LoadingState,
    relay,
    RelayRequestError,
    Row,
    Screen,
    Section,
    Segmented,
    Sheet,
    useBusQuery,
    useRelayEvent,
    useRelayOnline,
} from '@components/relay'
import { useSheetStyles } from '@components/relay/Sheet'
import {
    attempt,
    Choice,
    ChoiceSheet,
    COLUMNS,
    columnLabel,
    DispatchSheet,
    formatBytes,
    moduleHref,
    offerUndo,
    priorityTone,
    Relation,
    shortTime,
    stateLabel,
    stateTone,
    Task,
    TaskCard,
    TaskForm,
    taskHref,
    TaskSearch,
    TYPES,
    UndoBar,
    undoLatest,
    usePieceStyles,
} from '@components/relay/tasks'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type Message = {
    id: number
    from: string
    to: string
    text: string
    sent_at: string
    priority: boolean
}
type AuditRow = {
    id: number
    ts: string
    actor: string
    op: string
    kind: string
    payload: any
    result_summary: any
    undo_op: any
    undone_by: number | null
}
type Activity = {
    history: AuditRow[]
    messages: Message[]
    next_audit: number | null
    next_message: number | null
}

/** Sub-tasks nest this deep at most (TASK_DEPTH_MAX): a root and two levels of children. */
const DEPTH_MAX = 3

/** One line for an audit row: a column change reads as `from → to`, anything else as its op. */
const describe = (row: AuditRow) => {
    const before = row.undo_op?.payload?.column
    const after =
        row.payload?.column ?? row.result_summary?.column ?? row.result_summary?.task?.column
    if (typeof before === 'string' && typeof after === 'string' && before !== after)
        return `${columnLabel(before as any)} → ${columnLabel(after as any)}`
    return row.op.replace(/^task\./, '').replace(/[._]/g, ' ')
}

/**
 * Task activity, newest first, paged by the two cursors task.activity hands back. A task or
 * mailbox event reloads the first page.
 */
const useActivity = (taskId: number | undefined) => {
    const online = useRelayOnline()
    const [data, setData] = useState<Activity | undefined>(undefined)
    const [error, setError] = useState<Error | undefined>(undefined)
    const [busy, setBusy] = useState(false)
    const generation = useRef(0)
    const load = useCallback(
        async (older?: Activity) => {
            if (taskId === undefined || !online) return
            const mine = ++generation.current
            setBusy(true)
            try {
                const page = await relay.call<Activity>('task.activity', {
                    task_id: taskId,
                    limit: 30,
                    ...(older?.next_audit ? { before_audit: older.next_audit } : {}),
                    ...(older?.next_message ? { before_message: older.next_message } : {}),
                })
                if (mine !== generation.current) return
                setData(
                    older
                        ? {
                              history: older.next_audit
                                  ? [...older.history, ...page.history]
                                  : older.history,
                              messages: older.next_message
                                  ? [...older.messages, ...page.messages]
                                  : older.messages,
                              next_audit: older.next_audit ? page.next_audit : null,
                              next_message: older.next_message ? page.next_message : null,
                          }
                        : page
                )
                setError(undefined)
            } catch (e) {
                if (mine === generation.current) setError(e as Error)
            } finally {
                if (mine === generation.current) setBusy(false)
            }
        },
        [taskId, online]
    )
    useFocusEffect(
        useCallback(() => {
            load()
        }, [load])
    )
    useRelayEvent(['task.changed', 'mailbox.new'], () => load(), { debounceMs: 500 })
    return { data, error, busy, reload: () => load(), more: () => data && load(data) }
}

const TaskScreen = () => {
    const router = useRouter()
    const styles = useStyles()
    const pieces = usePieceStyles()
    const sheet = useSheetStyles()
    const params = useLocalSearchParams<{ task_id?: string }>()
    const parsed = Number(params.task_id)
    const taskId = Number.isFinite(parsed) ? parsed : undefined

    const query = useBusQuery<Task>(
        'task.get',
        { task_id: taskId },
        { events: ['task.*'], enabled: taskId !== undefined }
    )
    const task = query.data
    const projectId = task?.project_id
    const children = useBusQuery<Task[]>(
        'task.children',
        { task_id: taskId },
        { select: (r) => r.tasks, events: ['task.*'], enabled: !!task && task.children.length > 0 }
    )
    // Titles for parent and relations, and the module name.
    const others = useBusQuery<Task[]>(
        'task.list',
        { project_id: projectId },
        { select: (r) => r.tasks, events: ['task.*'], enabled: projectId !== undefined }
    )
    const module = useBusQuery<{ name: string }>(
        'module.get',
        { module_id: task?.module_id },
        { events: ['module.*'], enabled: !!task?.module_id }
    )
    const suggestions = useBusQuery<string[]>(
        'task.label.list',
        { project_id: projectId },
        {
            select: (r) => r.labels.map((l: { name: string }) => l.name),
            events: ['task.*'],
            enabled: projectId !== undefined,
        }
    )
    const activity = useActivity(taskId)

    const [editing, setEditing] = useState(false)
    const [adding, setAdding] = useState(false)
    const [dispatching, setDispatching] = useState<Task | undefined>(undefined)
    const [menu, setMenu] = useState<{ title: string; choices: Choice[] } | undefined>(undefined)
    const [relating, setRelating] = useState(false)
    const [relation, setRelation] = useState<Relation>('blocked_by')
    const [label, setLabel] = useState('')
    const [sha, setSha] = useState('')
    const [branch, setBranch] = useState('')
    const [changelog, setChangelog] = useState<string | undefined>(undefined)
    const [message, setMessage] = useState('')
    const [recipient, setRecipient] = useState<string | undefined>(undefined)
    const [uploading, setUploading] = useState(false)
    const [deleted, setDeleted] = useState(false)

    const title = (id: number) => others.data?.find((t) => t.id === id)?.title ?? ''
    const session = task?.sessions[task.sessions.length - 1]
    const to = recipient ?? session

    const run = async <T,>(op: string, payload: object, done?: string) => {
        const result = await attempt(() => relay.guarded<T>(op, payload), done)
        if (result !== undefined) query.reload()
        return result
    }

    if (taskId === undefined) {
        return (
            <Screen title="Task">
                <EmptyState icon="file-text" title="No task" />
            </Screen>
        )
    }

    const notFound =
        query.error instanceof RelayRequestError && query.error.error.kind === 'not_found'
    if (deleted || notFound) {
        return (
            <Screen title={`#${taskId}`} footer={<UndoBar />}>
                <EmptyState
                    icon="delete"
                    title="Task deleted"
                    text="It is in the PC's trash; restore it to keep working on it."
                    action={{
                        label: 'Restore',
                        onPress: async () => {
                            const back = await attempt(
                                () => relay.guarded('task.restore', { task_id: taskId }),
                                'Restored'
                            )
                            if (back === undefined) return
                            setDeleted(false)
                            query.reload()
                        },
                    }}
                />
            </Screen>
        )
    }

    if (!task) {
        return (
            <Screen title={`#${taskId}`}>
                {query.error ? (
                    <ErrorState error={query.error} onRetry={query.reload} />
                ) : (
                    <LoadingState />
                )}
            </Screen>
        )
    }

    const move = () =>
        setMenu({
            title: `Move #${task.id} to`,
            choices: COLUMNS.filter((c) => c.value !== task.column && c.value !== 'done').map(
                (c) => ({
                    label: columnLabel(c.value),
                    onPress: async () => {
                        const from = task.column
                        const moved = await run('task.move', {
                            task_id: task.id,
                            column: c.value,
                        })
                        if (moved)
                            offerUndo(`Moved to ${columnLabel(c.value)}`, () =>
                                relay.guarded('task.move', {
                                    task_id: task.id,
                                    column: from,
                                    position: task.position,
                                })
                            )
                    },
                })
            ),
        })

    const approve = async () => {
        const yes = await confirm({
            title: `Approve #${task.id}?`,
            message: 'It moves to Done; the branch head is linked as its commit.',
            confirmLabel: 'Approve',
        })
        if (!yes) return
        if (await run('task.approve', { task_id: task.id }, 'Approved'))
            offerUndo('Approved', () => undoLatest(task.project_id, 'task.approve'))
    }

    const remove = async () => {
        const yes = await confirm({
            title: `Delete #${task.id}?`,
            message: task.title,
            confirmLabel: 'Delete',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(() => relay.guarded('task.delete', { task_id: task.id }))
        if (done === undefined) return
        offerUndo(`#${task.id} deleted`, () => relay.guarded('task.restore', { task_id: task.id }))
        if (router.canGoBack()) router.back()
        else setDeleted(true)
    }

    const addLabel = async (name: string) => {
        const value = name.trim()
        if (!value) return
        if (await run('task.label.add', { task_id: task.id, label: value })) setLabel('')
    }

    const removeLabel = (name: string) =>
        run('task.label.remove', { task_id: task.id, label: name }).then((done) => {
            if (done)
                offerUndo(`Label ${name} removed`, () =>
                    relay.guarded('task.label.add', { task_id: task.id, label: name })
                )
        })

    const unrelate = async (kind: Relation, other: number) => {
        const yes = await confirm({
            title: kind === 'blocked_by' ? `No longer blocked by #${other}?` : 'Not a duplicate?',
            confirmLabel: 'Remove',
        })
        if (yes) run('task.unrelate', { task_id: task.id, relation: kind, other_id: other })
    }

    const linkCommit = async () => {
        const value = sha.trim()
        if (!value) return
        const done = await run(
            'task.link_commit',
            { task_id: task.id, sha: value, ...(branch.trim() ? { branch: branch.trim() } : {}) },
            'Commit linked'
        )
        if (done) {
            setSha('')
            setBranch('')
        }
    }

    const saveChangelog = async () => {
        if (changelog === undefined) return
        const done = await run(
            'task.changelog.write',
            { task_id: task.id, text: changelog },
            'Changelog saved'
        )
        if (done) setChangelog(undefined)
    }

    const attach = async (source: 'camera' | 'library') => {
        const permission =
            source === 'camera'
                ? await requestCameraPermissionsAsync()
                : await requestMediaLibraryPermissionsAsync()
        if (!permission.granted) {
            Logger.errorToast(
                source === 'camera' ? 'Camera permission is needed' : 'Photo access is needed'
            )
            return
        }
        const options: ImagePickerOptions = { mediaTypes: ['images'], base64: true, quality: 0.7 }
        let picked: ImagePickerResult
        try {
            picked =
                source === 'camera'
                    ? await launchCameraAsync(options)
                    : await launchImageLibraryAsync(options)
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
            return
        }
        if (picked.canceled) return
        const asset = picked.assets[0]
        if (!asset?.base64) {
            Logger.errorToast('Could not read the photo')
            return
        }
        const mime = asset.mimeType ?? 'image/jpeg'
        const extension = mime.split('/')[1]?.replace('jpeg', 'jpg') ?? 'jpg'
        const name = asset.fileName?.trim() || `photo-${Date.now()}.${extension}`
        setUploading(true)
        await run(
            'task.attach',
            { task_id: task.id, name: name, mime: mime, bytes_b64: asset.base64 },
            'Photo attached'
        )
        setUploading(false)
    }

    const detach = async (id: number, name: string) => {
        const yes = await confirm({
            title: 'Remove attachment?',
            message: name,
            confirmLabel: 'Remove',
            destructive: true,
        })
        if (!yes) return
        if (await run('task.detach', { task_id: task.id, attachment_id: id }))
            offerUndo(`${name} removed`, () => undoLatest(task.project_id, 'task.detach'))
    }

    const send = async () => {
        const text = message.trim()
        if (!text || !to) return
        const sent = await attempt(() =>
            relay.guarded<{ delivery: string }>('mailbox.send', {
                project_id: task.project_id,
                to: to,
                text: text,
                re_task: task.id,
            })
        )
        if (!sent) return
        setMessage('')
        Logger.infoToast(
            sent.delivery === 'queued'
                ? `Sent to ${to}`
                : sent.delivery === 'session_parked'
                  ? `${to} is parked; it reads this when it wakes`
                  : `${to} is not running; it reads this when it starts`
        )
        activity.reload()
    }

    const typeLabel = TYPES.find((t) => t.value === task.type)?.label ?? task.type
    const canNest = task.depth + 1 < DEPTH_MAX

    return (
        <Screen
            title={`#${task.id}`}
            onRefresh={() => {
                query.reload()
                activity.reload()
            }}
            refreshing={query.loading}
            footer={<UndoBar />}
            actions={[{ icon: 'edit', label: 'Edit', onPress: () => setEditing(true) }]}>
            <View style={styles.head}>
                <Text selectable style={styles.title}>
                    {task.title}
                </Text>
                <View style={pieces.chips}>
                    <Chip label={columnLabel(task.column)} tone="primary" selected />
                    {task.state !== 'none' && (
                        <Chip label={stateLabel(task.state)} tone={stateTone(task.state)} />
                    )}
                    <Chip label={typeLabel} tone={task.type === 'bug' ? 'danger' : 'neutral'} />
                    <Chip label={task.priority} tone={priorityTone(task.priority)} />
                    {!!task.size && <Chip label={`size ${task.size}`} />}
                    {task.rollup.total > 0 && (
                        <Chip label={`${task.rollup.done}/${task.rollup.total} done`} />
                    )}
                </View>
            </View>

            <View style={pieces.buttons}>
                <ThemedButton label="Move" variant="secondary" onPress={move} />
                {task.column !== 'done' && (
                    <ThemedButton
                        label="Dispatch"
                        variant="secondary"
                        onPress={() => setDispatching(task)}
                    />
                )}
                {task.column === 'in_review' && <ThemedButton label="Approve" onPress={approve} />}
                <ThemedButton label="Delete" variant="critical" onPress={remove} />
            </View>

            <Section>
                <Row
                    label="Module"
                    detail={task.module_id ? (module.data?.name ?? `#${task.module_id}`) : 'None'}
                    icon="appstore"
                    onPress={
                        task.module_id ? () => router.push(moduleHref(task.module_id!)) : undefined
                    }
                    onLongPress={() => setEditing(true)}
                />
                {task.parent_id !== null && (
                    <Row
                        label="Parent"
                        detail={`#${task.parent_id} ${title(task.parent_id)}`}
                        icon="apartment"
                        onPress={() => router.push(taskHref(task.parent_id!))}
                        onLongPress={async () => {
                            const yes = await confirm({
                                title: 'Detach from parent?',
                                message: 'It becomes a top-level task.',
                                confirmLabel: 'Detach',
                            })
                            if (yes) run('task.parent.set', { task_id: task.id, parent_id: null })
                        }}
                    />
                )}
                <Row
                    label="Agent"
                    detail={
                        session
                            ? task.sessions.length > 1
                                ? `${session} (and ${task.sessions.length - 1} before)`
                                : session
                            : 'Not dispatched'
                    }
                    icon="user"
                    onPress={
                        session
                            ? () =>
                                  router.push({
                                      pathname: '/screens/RelayScreen/Terminal',
                                      params: { session: session },
                                  })
                            : undefined
                    }
                />
                {!!session && task.column !== 'done' && (
                    <Row
                        label="Changes"
                        detail="What the agent changed"
                        icon="diff"
                        onPress={() =>
                            router.push({
                                pathname: '/screens/RelayScreen/Changes',
                                params: { session: session },
                            })
                        }
                    />
                )}
                <Row
                    label="Updated"
                    detail={`${shortTime(task.updated_at)} · created ${shortTime(task.created_at)}`}
                    icon="clock-circle"
                />
            </Section>

            <Section
                title="Description"
                action={{ label: 'Edit', onPress: () => setEditing(true) }}>
                <Text selectable style={[pieces.body, styles.pad]}>
                    {task.body || 'No description.'}
                </Text>
            </Section>

            <Section title="Labels">
                <View style={[styles.pad, { rowGap: 10 }]}>
                    {task.labels.length > 0 ? (
                        <View style={pieces.chips}>
                            {task.labels.map((name) => (
                                <Chip
                                    key={name}
                                    label={name}
                                    icon="close"
                                    onPress={() => removeLabel(name)}
                                />
                            ))}
                        </View>
                    ) : (
                        <Text style={pieces.meta}>No labels.</Text>
                    )}
                    <View style={styles.inline}>
                        <View style={{ flex: 1 }}>
                            <Field
                                value={label}
                                onChangeText={setLabel}
                                placeholder="Add a label"
                                autoCapitalize="none"
                                onSubmitEditing={() => addLabel(label)}
                            />
                        </View>
                        <ThemedButton
                            label="Add"
                            variant="secondary"
                            onPress={() => addLabel(label)}
                        />
                    </View>
                    {(suggestions.data ?? []).filter(
                        (name) =>
                            !task.labels.includes(name) &&
                            name.toLowerCase().includes(label.trim().toLowerCase())
                    ).length > 0 && (
                        <View style={pieces.chips}>
                            {(suggestions.data ?? [])
                                .filter(
                                    (name) =>
                                        !task.labels.includes(name) &&
                                        name.toLowerCase().includes(label.trim().toLowerCase())
                                )
                                .slice(0, 12)
                                .map((name) => (
                                    <Chip
                                        key={name}
                                        label={name}
                                        icon="plus"
                                        onPress={() => addLabel(name)}
                                    />
                                ))}
                        </View>
                    )}
                </View>
            </Section>

            <Section
                title={`Sub-tasks · ${task.children.length}`}
                card={false}
                action={canNest ? { label: 'Add', onPress: () => setAdding(true) } : undefined}>
                {task.children.length === 0 ? (
                    <Text style={pieces.meta}>
                        {canNest
                            ? 'No sub-tasks.'
                            : 'No sub-tasks; this one is nested as deep as tasks go.'}
                    </Text>
                ) : (
                    <View style={{ rowGap: 8 }}>
                        {(children.data ?? []).map((child) => (
                            <TaskCard
                                key={child.id}
                                task={child}
                                showColumn
                                onPress={() => router.push(taskHref(child.id))}
                            />
                        ))}
                        {!children.data && <LoadingState />}
                    </View>
                )}
            </Section>

            <Section title="Relations" action={{ label: 'Add', onPress: () => setRelating(true) }}>
                {task.blocked_by.map((id) => (
                    <Row
                        key={`b${id}`}
                        label={`Blocked by #${id}`}
                        detail={title(id)}
                        icon="lock"
                        onPress={() => router.push(taskHref(id))}
                        onLongPress={() => unrelate('blocked_by', id)}
                    />
                ))}
                {task.blocks.map((id) => (
                    <Row
                        key={`k${id}`}
                        label={`Blocks #${id}`}
                        detail={title(id)}
                        icon="unlock"
                        onPress={() => router.push(taskHref(id))}
                    />
                ))}
                {task.duplicate_of !== null && (
                    <Row
                        label={`Duplicate of #${task.duplicate_of}`}
                        detail={title(task.duplicate_of)}
                        icon="copy"
                        onPress={() => router.push(taskHref(task.duplicate_of!))}
                        onLongPress={() => unrelate('duplicate_of', task.duplicate_of!)}
                    />
                )}
                {task.blocked_by.length + task.blocks.length === 0 && task.duplicate_of === null ? (
                    <Row label="None" detail="Blocked by, or duplicate of, another task" />
                ) : (
                    <Text style={[pieces.meta, styles.note]}>
                        Long-press a relation to remove it.
                    </Text>
                )}
            </Section>

            <Section title="Changelog">
                <View style={[styles.pad, { rowGap: 8 }]}>
                    <Field
                        value={changelog ?? task.changelog}
                        onChangeText={setChangelog}
                        placeholder="The sentence that ships in the patch notes"
                        multiline
                        lines={2}
                    />
                    {changelog !== undefined && changelog !== task.changelog && (
                        <View style={styles.right}>
                            <ThemedButton
                                label="Revert"
                                variant="secondary"
                                onPress={() => setChangelog(undefined)}
                            />
                            <ThemedButton label="Save" onPress={saveChangelog} />
                        </View>
                    )}
                </View>
            </Section>

            <Section title={`Commits · ${task.commits.length}`}>
                {task.commits.map((commit) => (
                    <Row
                        key={commit.sha}
                        label={commit.sha.slice(0, 10)}
                        detail={`${commit.branch ?? 'no branch'} · ${shortTime(commit.linked_at)}`}
                        icon="branches"
                        mono
                    />
                ))}
                <View style={[styles.pad, { rowGap: 8 }]}>
                    <Field
                        value={sha}
                        onChangeText={setSha}
                        placeholder="Commit sha"
                        autoCapitalize="none"
                        autoCorrect={false}
                        mono
                    />
                    {!!sha.trim() && (
                        <>
                            <Field
                                value={branch}
                                onChangeText={setBranch}
                                placeholder="Branch (optional)"
                                autoCapitalize="none"
                                autoCorrect={false}
                                mono
                            />
                            <View style={styles.right}>
                                <ThemedButton label="Link commit" onPress={linkCommit} />
                            </View>
                        </>
                    )}
                </View>
            </Section>

            <Section title={`Attachments · ${task.attachments.length}`}>
                {task.attachments.map((file) => (
                    <Row
                        key={file.id}
                        label={file.name}
                        detail={`${file.mime} · ${formatBytes(file.bytes)} · ${shortTime(file.created_at)}`}
                        icon={file.mime.startsWith('image/') ? 'picture' : 'paper-clip'}
                        right={<Chip label="Remove" onPress={() => detach(file.id, file.name)} />}
                    />
                ))}
                <View style={[styles.right, styles.pad]}>
                    <ThemedButton
                        label="Gallery"
                        variant={uploading ? 'disabled' : 'secondary'}
                        onPress={uploading ? undefined : () => attach('library')}
                    />
                    <ThemedButton
                        label={uploading ? 'Uploading…' : 'Camera'}
                        variant={uploading ? 'disabled' : 'secondary'}
                        onPress={uploading ? undefined : () => attach('camera')}
                    />
                </View>
            </Section>

            {task.sessions.length > 0 && (
                <Section title="Ask the agent">
                    <View style={[styles.pad, { rowGap: 8 }]}>
                        {task.sessions.length > 1 && (
                            <View style={pieces.chips}>
                                {Array.from(new Set(task.sessions)).map((name) => (
                                    <Chip
                                        key={name}
                                        label={name}
                                        tone={name === to ? 'primary' : 'neutral'}
                                        selected={name === to}
                                        onPress={() => setRecipient(name)}
                                    />
                                ))}
                            </View>
                        )}
                        <Field
                            value={message}
                            onChangeText={setMessage}
                            placeholder={`Message ${to} about this task`}
                            multiline
                            lines={2}
                        />
                        <View style={styles.right}>
                            <ThemedButton
                                label="Send"
                                variant={message.trim() ? 'primary' : 'disabled'}
                                onPress={message.trim() ? send : undefined}
                            />
                        </View>
                    </View>
                </Section>
            )}

            <Section title="Activity" card={false}>
                {activity.error && !activity.data && (
                    <ErrorState error={activity.error} onRetry={activity.reload} />
                )}
                {!activity.data && !activity.error && <LoadingState />}
                {activity.data && (
                    <View style={{ rowGap: 12 }}>
                        <View style={styles.card}>
                            <Caption>Messages</Caption>
                            {activity.data.messages.length === 0 && (
                                <Text style={pieces.meta}>
                                    No messages linked to this task yet.
                                </Text>
                            )}
                            {activity.data.messages.map((m) => (
                                <View key={m.id} style={styles.entry}>
                                    <Text style={pieces.meta}>
                                        {m.from} → {m.to} · {shortTime(m.sent_at)}
                                        {m.priority ? ' · priority' : ''}
                                    </Text>
                                    <Text selectable style={pieces.body}>
                                        {m.text}
                                    </Text>
                                </View>
                            ))}
                        </View>
                        <View style={styles.card}>
                            <Caption>History</Caption>
                            {activity.data.history.length === 0 && (
                                <Text style={pieces.meta}>No recorded activity.</Text>
                            )}
                            {activity.data.history.map((row) => (
                                <View key={row.id} style={styles.entry}>
                                    <Text style={styles.entryTitle}>
                                        {describe(row)}
                                        {row.undone_by ? ' (undone)' : ''}
                                    </Text>
                                    <Text style={pieces.meta}>
                                        {shortTime(row.ts)} · {row.actor} · {row.kind}
                                    </Text>
                                </View>
                            ))}
                        </View>
                        {(activity.data.next_audit || activity.data.next_message) && (
                            <ThemedButton
                                label={activity.busy ? 'Loading…' : 'Load older'}
                                variant="secondary"
                                onPress={activity.busy ? undefined : activity.more}
                            />
                        )}
                    </View>
                )}
            </Section>

            <TaskForm
                visible={editing}
                onDismiss={() => setEditing(false)}
                projectId={task.project_id}
                task={task}
                onSaved={() => query.reload()}
                onConflict={() => query.reload()}
            />
            <TaskForm
                visible={adding}
                onDismiss={() => setAdding(false)}
                projectId={task.project_id}
                defaults={{
                    parent_id: task.id,
                    module_id: task.module_id ?? undefined,
                    column: 'backlog',
                }}
                onSaved={() => {
                    query.reload()
                    children.reload()
                }}
            />
            <DispatchSheet task={dispatching} onDismiss={() => setDispatching(undefined)} />
            <ChoiceSheet
                title={menu?.title}
                choices={menu?.choices}
                onDismiss={() => setMenu(undefined)}
            />
            <Sheet visible={relating} onDismiss={() => setRelating(false)}>
                <View style={[sheet.body, pieces.shrink]}>
                    <Text style={sheet.title}>Relate #{task.id}</Text>
                    <Segmented
                        options={[
                            { value: 'blocked_by', label: 'Blocked by' },
                            { value: 'duplicate_of', label: 'Duplicate of' },
                        ]}
                        value={relation}
                        onChange={setRelation}
                    />
                    <ScrollView
                        style={[sheet.scroll, pieces.shrink]}
                        keyboardShouldPersistTaps="handled">
                        <TaskSearch
                            projectId={task.project_id}
                            exclude={[task.id, ...task.blocked_by]}
                            onPick={async (other) => {
                                setRelating(false)
                                await run(
                                    'task.relate',
                                    { task_id: task.id, relation: relation, other_id: other.id },
                                    relation === 'blocked_by'
                                        ? `Blocked by #${other.id}`
                                        : `Duplicate of #${other.id}`
                                )
                            }}
                        />
                    </ScrollView>
                </View>
            </Sheet>
        </Screen>
    )
}

export default TaskScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        head: {
            rowGap: spacing.m,
        },
        title: {
            color: color.text._100,
            fontSize: fontSize.xl,
            fontWeight: '600',
        },
        pad: {
            paddingVertical: spacing.l,
        },
        inline: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        right: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
        note: {
            paddingBottom: spacing.m,
        },
        card: {
            backgroundColor: color.neutral._200,
            borderRadius: 16,
            padding: spacing.l,
            rowGap: spacing.m,
        },
        entry: {
            rowGap: 2,
        },
        entryTitle: {
            color: color.text._200,
        },
    })
}
