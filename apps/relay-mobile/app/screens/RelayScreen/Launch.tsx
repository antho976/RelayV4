import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Caption,
    Chip,
    EmptyState,
    ErrorState,
    Field,
    isCancelled,
    LoadingState,
    relay,
    relayHref,
    Row,
    Screen,
    Section,
    Segmented,
    SwitchRow,
    useBusQuery,
    useRelayStore,
} from '@components/relay'
import { DEFAULT_EFFORT, effortChoices, terminalHref } from '@components/relay/sessions'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type Provider = 'claude' | 'codex'
type Role = 'builder' | 'reviewer' | 'docs'
type Mode = 'solo' | 'group'

type ProviderInfo = {
    provider: Provider
    installed: boolean
    version: string | null
    signed_in_as: string | null
    guarded: boolean
}
type Worktree = { path: string; branch: string; session: string | null; dirty: boolean }
type Task = { id: number; title: string; body: string; column: string }

/** One agent's launch settings: launch.rs `Profile`. */
type Profile = {
    /** Undefined follows the first installed provider. */
    provider?: Provider
    role: Role
    model: string
    effort: string
    /** `new` (a pooled worktree), `primary`, or the path of an existing worktree. */
    worktree: string
    writes: boolean
    ui: boolean
    prompt: string
    tasks: number[]
}

const MAX_AGENTS = 6

const blank = (): Profile => ({
    role: 'builder',
    model: '',
    effort: DEFAULT_EFFORT,
    worktree: 'new',
    writes: true,
    ui: false,
    prompt: '',
    tasks: [],
})

const PROVIDERS: { value: Provider; label: string }[] = [
    { value: 'claude', label: 'Claude Code' },
    { value: 'codex', label: 'Codex' },
]

const ROLES: { value: Role; label: string }[] = [
    { value: 'builder', label: 'Builder' },
    { value: 'reviewer', label: 'Reviewer' },
    { value: 'docs', label: 'Docs' },
]

const COLUMNS = [
    { value: 'all', label: 'All open' },
    { value: 'backlog', label: 'Backlog' },
    { value: 'ready', label: 'Ready' },
    { value: 'active', label: 'Active' },
    { value: 'in_review', label: 'Review' },
]

/**
 * The profiles a launch uses, in the order the desktop allocates them (launch.rs): solo is
 * agents 1..N; a review group is builder 1, the reviewer (profile 3), then builder 2, so each
 * later member can pair with the one before it.
 */
const launchOrder = (mode: Mode, count: number, builders: number) =>
    mode === 'group'
        ? builders === 2
            ? [0, 2, 1]
            : [0, 2]
        : Array.from({ length: count }, (_, i) => i)

/** The members shown on the rail, in reading order. */
const railOrder = (mode: Mode, count: number, builders: number) =>
    mode === 'group' ? (builders === 2 ? [0, 1, 2] : [0, 2]) : launchOrder(mode, count, builders)

const memberLabel = (mode: Mode, index: number) =>
    mode === 'group' ? (index === 2 ? 'Reviewer' : `Builder ${index + 1}`) : `Agent ${index + 1}`

/**
 * Launch agents on the PC, as the desktop's New session panel does (relay-native launch.rs):
 * solo agents, each in its own worktree, or a review group — one or two builders and a
 * reviewer on one shared branch. Every agent gets its provider, role, model, effort, worktree,
 * permissions and opening instructions; open tasks can be staged onto it. The launch
 * allocates every session first (`session.create`), stages the tasks (`task.dispatch` with
 * `start: false`), then starts the agents, reviewers first so their mailbox is live before the
 * builders publish anything.
 */
const LaunchScreen = () => {
    const styles = useStyles()
    const { color, spacing } = Theme.useTheme()
    const router = useRouter()
    const params = useLocalSearchParams<{ project_id?: string; task?: string; prompt?: string }>()
    const projects = useRelayStore((state) => state.projects)
    const [pickedProject, setPickedProject] = useState<number | undefined>(undefined)
    const paramProject = params.project_id ? Number(params.project_id) : undefined
    const projectId =
        pickedProject ??
        (paramProject !== undefined && Number.isFinite(paramProject) ? paramProject : undefined)
    const project = projects.find((item) => item.id === projectId)

    const [mode, setMode] = useState<Mode>('solo')
    const [count, setCount] = useState(1)
    const [builders, setBuilders] = useState(1)
    // A task or a prompt handed over by another screen lands on the first agent.
    const [profiles, setProfiles] = useState<Profile[]>(() => {
        const all = Array.from({ length: MAX_AGENTS }, blank)
        const task = params.task ? Number(params.task) : NaN
        if (Number.isFinite(task)) all[0].tasks = [task]
        if (typeof params.prompt === 'string') all[0].prompt = params.prompt
        return all
    })
    const [current, setCurrent] = useState(0)
    const [search, setSearch] = useState('')
    const [column, setColumn] = useState('all')
    const [progress, setProgress] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState('')

    const providers = useBusQuery<ProviderInfo[]>(
        'provider.list',
        {},
        {
            events: ['provider.*'],
            refetchOnFocus: false,
            select: (raw) => raw.providers ?? [],
        }
    )
    const tasks = useBusQuery<Task[]>(
        'task.list',
        { project_id: projectId },
        {
            enabled: projectId !== undefined,
            events: ['task.changed'],
            projectId: projectId,
            select: (raw) => (raw.tasks ?? []).filter((task: Task) => task.column !== 'done'),
        }
    )
    const worktrees = useBusQuery<Worktree[]>(
        'worktree.list',
        { project_id: projectId },
        {
            enabled: projectId !== undefined,
            events: ['worktree.changed'],
            projectId: projectId,
            select: (raw) => raw.worktrees ?? [],
        }
    )

    const members = railOrder(mode, count, builders)
    const index = members.includes(current) ? current : members[0]
    const profile = profiles[index]
    const group = mode === 'group'
    // In a group only builder 1 chooses the worktree and the task queue; the rest share them.
    const leads = !group || index === 0
    const roleOf = (i: number): Role =>
        group ? (i === 2 ? 'reviewer' : 'builder') : profiles[i].role

    const patch = (next: Partial<Profile>) =>
        setProfiles((all) => all.map((item, i) => (i === index ? { ...item, ...next } : item)))

    const setProvider = (provider: Provider) => {
        const effort = effortChoices(provider).includes(profile.effort)
            ? profile.effort
            : DEFAULT_EFFORT
        patch({ provider, effort })
    }

    const toggleTask = (id: number) =>
        patch({
            tasks: profile.tasks.includes(id)
                ? profile.tasks.filter((task) => task !== id)
                : [...profile.tasks, id],
        })

    const providerInfo = (provider: Provider) =>
        providers.data?.find((item) => item.provider === provider)
    // An agent nobody switched runs on the first installed provider.
    const fallback = providers.data?.find((item) => item.installed)?.provider ?? 'claude'
    const providerOf = (p: Profile): Provider => p.provider ?? fallback
    const effortOf = (p: Profile) =>
        effortChoices(providerOf(p)).includes(p.effort) ? p.effort : DEFAULT_EFFORT

    const launch = async () => {
        if (busy || projectId === undefined) return
        const order = launchOrder(mode, count, builders)
        for (const i of order) {
            const info = providerInfo(providerOf(profiles[i]))
            if (providers.data && !info?.installed) {
                setError(`Choose an installed provider for ${memberLabel(mode, i)}.`)
                setCurrent(i)
                return
            }
            if ((!group || i === 0) && !profiles[i].worktree.trim()) {
                setError(`Set a worktree for ${memberLabel(mode, i)}.`)
                setCurrent(i)
                return
            }
        }
        setError('')
        setBusy(true)
        const allocated: { name: string; role: string; tasks: number[]; prompt: string }[] = []
        try {
            for (const [step, i] of order.entries()) {
                const p = profiles[i]
                const staged = !group || i === 0 ? p.tasks : []
                const prompt = p.prompt.trim()
                const payload: Record<string, unknown> = {
                    project_id: projectId,
                    provider: providerOf(p),
                    role: roleOf(i),
                    effort: effortOf(p),
                    bus_writes: p.writes,
                    allow_ui: p.ui,
                }
                if (p.model.trim()) payload.model = p.model.trim()
                if (prompt) payload.prompt = prompt
                if (staged.length > 0) payload.task_id = staged[0]
                // Later group members share the first member's worktree through the pair.
                if (group && step > 0) payload.pair_with = allocated[step - 1].name
                else payload.worktree = p.worktree.trim()
                setProgress(`Allocating ${memberLabel(mode, i).toLowerCase()}…`)
                const session = await relay.guarded<{ name: string; role: string }>(
                    'session.create',
                    payload
                )
                allocated.push({
                    name: session.name,
                    role: session.role,
                    tasks: staged,
                    prompt: prompt,
                })
            }
            relay.refresh().catch(() => {})
            for (const session of allocated) {
                for (const task of session.tasks) {
                    setProgress(`Staging task #${task} on ${session.name}…`)
                    await relay.guarded('task.dispatch', {
                        task_id: task,
                        session: session.name,
                        start: false,
                    })
                }
            }
            const starting = [...allocated].sort(
                (a, b) => Number(a.role !== 'reviewer') - Number(b.role !== 'reviewer')
            )
            for (const session of starting) {
                setProgress(`Starting ${session.name}…`)
                // An omitted prompt keeps the assignment stored at allocation.
                await relay.guarded(
                    'session.spawn',
                    session.prompt
                        ? { session: session.name, prompt: session.prompt }
                        : { session: session.name }
                )
            }
            relay.refresh().catch(() => {})
            Logger.infoToast(
                allocated.length === 1
                    ? `Launched ${allocated[0].name}`
                    : `Launched ${allocated.length} agents`
            )
            if (allocated.length === 1) router.replace(terminalHref(allocated[0].name))
            else router.replace(relayHref('Project', { project_id: String(projectId) }))
        } catch (e) {
            relay.refresh().catch(() => {})
            const message = isCancelled(e) ? 'a held action was denied' : (e as Error).message
            if (allocated.length === 0) {
                setError(message)
            } else {
                Logger.errorToast(
                    `Launch incomplete: ${message}. Created sessions and queues are kept; start the rest from their terminals.`
                )
                router.replace(relayHref('Project', { project_id: String(projectId) }))
            }
        } finally {
            setBusy(false)
            setProgress('')
        }
    }

    if (projectId === undefined || !project) {
        return (
            <Screen title="Launch agents">
                {projects.length === 0 ? (
                    <EmptyState
                        icon="folder"
                        title="No projects"
                        text="Add a project on the PC first."
                    />
                ) : (
                    <Section title="Project">
                        {projects.map((item) => (
                            <Row
                                key={item.id}
                                label={item.name}
                                detail={item.path}
                                icon="folder"
                                mono
                                onPress={() => setPickedProject(item.id)}
                            />
                        ))}
                    </Section>
                )}
            </Screen>
        )
    }

    const query = search.trim().toLowerCase()
    const openTasks = tasks.data ?? []
    const shownTasks = openTasks.filter(
        (task) =>
            (column === 'all' || task.column === column) &&
            (!query || `#${task.id} ${task.title} ${task.body}`.toLowerCase().includes(query))
    )
    const worktreeLabel = (value: string) =>
        value === 'new' ? 'New worktree' : value === 'primary' ? 'Primary checkout' : value

    return (
        <Screen
            title="Launch agents"
            footer={
                <View style={styles.footer}>
                    <Text numberOfLines={2} style={[styles.progress, !!error && styles.error]}>
                        {error ||
                            progress ||
                            `${project.name} · ${members.length} ${members.length === 1 ? 'agent' : 'agents'}`}
                    </Text>
                    <ThemedButton
                        label={busy ? 'Launching…' : 'Launch'}
                        iconName="rocket"
                        variant={busy ? 'disabled' : 'primary'}
                        onPress={launch}
                    />
                </View>
            }>
            <Section title="Session type" card={false}>
                <Segmented
                    options={[
                        { value: 'solo' as Mode, label: 'Solo agents' },
                        { value: 'group' as Mode, label: 'Review group' },
                    ]}
                    value={mode}
                    onChange={(value) => {
                        setMode(value)
                        setCurrent(0)
                    }}
                />
                <Text style={styles.note}>
                    {group
                        ? 'Builders and one read-only reviewer share a branch. Choose the worktree and the task queue on builder 1.'
                        : 'Each agent has its own worktree, role, settings and tasks.'}
                </Text>
            </Section>

            <Section title="Agents" card={false}>
                {group ? (
                    <Segmented
                        options={[
                            { value: '1', label: '1 builder' },
                            { value: '2', label: '2 builders' },
                        ]}
                        value={String(builders)}
                        onChange={(value) => setBuilders(Number(value))}
                    />
                ) : (
                    <Segmented
                        options={Array.from({ length: MAX_AGENTS }, (_, i) => ({
                            value: String(i + 1),
                            label: String(i + 1),
                        }))}
                        value={String(count)}
                        onChange={(value) => setCount(Number(value))}
                    />
                )}
                {members.length > 1 && (
                    <View style={styles.rail}>
                        {members.map((i) => (
                            <Chip
                                key={i}
                                label={`${memberLabel(mode, i)} · ${providerOf(profiles[i])}`}
                                tone="primary"
                                selected={i === index}
                                onPress={() => setCurrent(i)}
                            />
                        ))}
                    </View>
                )}
            </Section>

            <Section title={`${memberLabel(mode, index)} · provider`}>
                {providers.data === undefined && providers.error ? (
                    <ErrorState error={providers.error} onRetry={providers.reload} />
                ) : null}
                {PROVIDERS.map((option) => {
                    const info = providerInfo(option.value)
                    const known = providers.data !== undefined
                    const detail = !known
                        ? 'Checking provider…'
                        : !info?.installed
                          ? 'CLI not installed'
                          : [
                                info.signed_in_as
                                    ? `Signed in as ${info.signed_in_as}`
                                    : 'Not signed in',
                                info.version ?? 'Version unavailable',
                                info.guarded ? 'Guarded' : 'Unguarded',
                            ].join(' · ')
                    return (
                        <Row
                            key={option.value}
                            label={option.label}
                            detail={detail}
                            disabled={known && !info?.installed}
                            chevron={false}
                            onPress={() => setProvider(option.value)}
                            right={
                                providerOf(profile) === option.value ? (
                                    <AntDesign
                                        name="check-circle"
                                        size={18}
                                        color={color.primary._500}
                                    />
                                ) : undefined
                            }
                        />
                    )
                })}
            </Section>

            <Section title="Configure" card={false}>
                <View style={styles.block}>
                    <Caption>Role</Caption>
                    {group ? (
                        <Text style={styles.note}>
                            {index === 2
                                ? 'Reviewer: reads the shared branch and reviews what the builders publish.'
                                : 'Builder: works on the shared branch.'}
                        </Text>
                    ) : (
                        <Segmented
                            options={ROLES}
                            value={profile.role}
                            onChange={(role) => patch({ role })}
                        />
                    )}
                </View>
                <Field
                    label="Model"
                    value={profile.model}
                    onChangeText={(model) => patch({ model })}
                    placeholder="Provider default, or a model ID"
                    autoCapitalize="none"
                    autoCorrect={false}
                />
                <View style={styles.block}>
                    <Caption>Reasoning effort</Caption>
                    <View style={styles.chips}>
                        {effortChoices(providerOf(profile)).map((effort) => (
                            <Chip
                                key={effort}
                                label={effort}
                                tone="primary"
                                selected={effortOf(profile) === effort}
                                onPress={() => patch({ effort })}
                            />
                        ))}
                    </View>
                </View>
                <Field
                    label="Additional instructions"
                    value={profile.prompt}
                    onChangeText={(prompt) => patch({ prompt })}
                    placeholder="Optional: the opening prompt for this agent."
                    multiline
                    lines={4}
                />
            </Section>

            <Section title="Worktree and permissions">
                {leads ? (
                    <>
                        {['new', 'primary'].map((value) => (
                            <Row
                                key={value}
                                label={worktreeLabel(value)}
                                detail={
                                    value === 'new'
                                        ? 'A fresh worktree from the pool, on its own branch.'
                                        : "The project's own checkout."
                                }
                                chevron={false}
                                onPress={() => patch({ worktree: value })}
                                right={
                                    profile.worktree === value ? (
                                        <AntDesign
                                            name="check-circle"
                                            size={18}
                                            color={color.primary._500}
                                        />
                                    ) : undefined
                                }
                            />
                        ))}
                        {(worktrees.data ?? []).map((worktree) => (
                            <Row
                                key={worktree.path}
                                label={
                                    worktree.branch ||
                                    worktree.path.split('/').pop() ||
                                    worktree.path
                                }
                                detail={`${worktree.path}${worktree.session ? ` · used by ${worktree.session}` : ''}${worktree.dirty ? ' · uncommitted changes' : ''}`}
                                mono
                                chevron={false}
                                onPress={() => patch({ worktree: worktree.path })}
                                right={
                                    profile.worktree === worktree.path ? (
                                        <AntDesign
                                            name="check-circle"
                                            size={18}
                                            color={color.primary._500}
                                        />
                                    ) : undefined
                                }
                            />
                        ))}
                    </>
                ) : (
                    <Row
                        label="Shared worktree"
                        detail={`Works in ${worktreeLabel(profiles[0].worktree)} with builder 1.`}
                    />
                )}
                <SwitchRow
                    label="Allow agent bus writes"
                    value={profile.writes}
                    onChange={(writes) => patch({ writes })}
                />
                <SwitchRow
                    label="Allow UI control"
                    value={profile.ui}
                    onChange={(ui) => patch({ ui })}
                />
            </Section>

            {leads ? (
                <Section
                    title={`Assign the work · ${profile.tasks.length} selected`}
                    action={
                        profile.tasks.length > 0
                            ? { label: 'Clear', onPress: () => patch({ tasks: [] }) }
                            : undefined
                    }
                    card={false}>
                    <Field
                        value={search}
                        onChangeText={setSearch}
                        placeholder="Search tasks"
                        autoCorrect={false}
                    />
                    <Segmented options={COLUMNS} value={column} onChange={setColumn} />
                    <View style={styles.card}>
                        {tasks.data === undefined ? (
                            tasks.error ? (
                                <ErrorState error={tasks.error} onRetry={tasks.reload} />
                            ) : (
                                <LoadingState label="Loading open tasks…" />
                            )
                        ) : openTasks.length === 0 ? (
                            <Row
                                label="No open tasks"
                                detail="You can launch without an assignment."
                            />
                        ) : shownTasks.length === 0 ? (
                            <Row label="No task matches" />
                        ) : (
                            shownTasks.map((task) => {
                                const on = profile.tasks.includes(task.id)
                                return (
                                    <Row
                                        key={task.id}
                                        label={`#${task.id} ${task.title}`}
                                        detail={`${task.column.replace('_', ' ')}${task.body ? ` · ${task.body}` : ''}`}
                                        chevron={false}
                                        onPress={() => toggleTask(task.id)}
                                        right={
                                            <AntDesign
                                                name={on ? 'check-square' : 'border'}
                                                size={18}
                                                color={on ? color.primary._500 : color.text._500}
                                            />
                                        }
                                    />
                                )
                            })
                        )}
                    </View>
                </Section>
            ) : (
                <Text style={[styles.note, { paddingHorizontal: spacing.s }]}>
                    The group&apos;s tasks are staged on builder 1.
                </Text>
            )}
        </Screen>
    )
}

export default LaunchScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        footer: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            paddingHorizontal: spacing.xl,
            paddingVertical: spacing.m,
            borderTopWidth: 1,
            borderTopColor: color.neutral._300,
        },
        progress: {
            flex: 1,
            color: color.text._400,
            fontSize: fontSize.s,
        },
        error: {
            color: color.error._300,
        },
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        rail: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            gap: spacing.s,
        },
        block: {
            rowGap: spacing.m,
        },
        chips: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            gap: spacing.s,
        },
        card: {
            backgroundColor: color.neutral._200,
            borderRadius: 16,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.xs,
        },
    })
}
