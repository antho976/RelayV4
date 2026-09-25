import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useState } from 'react'
import { View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    EmptyState,
    ErrorState,
    Field,
    isCancelled,
    LoadingState,
    Mono,
    relay,
    Row,
    Screen,
    Section,
    SwitchRow,
    useBusQuery,
    useRelayStore,
} from '@components/relay'
import {
    effortChoices,
    lifecycleActions,
    terminalHref,
    useSessionLifecycle,
} from '@components/relay/sessions'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

/** `session.get`: the Session row (relay-bus types.rs). */
type SessionFull = {
    id: number
    name: string
    intent: string | null
    project_id: number
    provider: string
    role: string
    model: string | null
    effort: string | null
    branch: string
    worktree: string
    task_id: number | null
    module_id: number | null
    pair_with: string | null
    bus_writes: boolean
    allow_ui: boolean
    state: string
    pid: number | null
    exit_code: number | null
    spawned_at: string | null
    last_output_at: string | null
    created_at: string
    updated_at: string
    closed_at: string | null
}

type Draft = { model: string; effort: string; writes: boolean; allowUi: boolean }

const when = (iso: string | null | undefined) => {
    if (!iso) return undefined
    const date = new Date(iso)
    return Number.isNaN(date.getTime()) ? iso : date.toLocaleString()
}

/**
 * One session's facts and settings: what the desktop's session menu shows (shell.rs
 * `session_menu`). Details from `session.get`, the launch brief from `session.brief`,
 * model/effort (before the first spawn only) and permissions through `session.update`, and
 * the same lifecycle actions as the terminal strip.
 */
const SessionScreen = () => {
    const router = useRouter()
    const { spacing } = Theme.useTheme()
    const params = useLocalSearchParams<{ session: string }>()
    const name = typeof params.session === 'string' ? params.session : ''
    const project = useRelayStore((state) => {
        const id = state.sessions.find((item) => item.name === name)?.project_id
        return state.projects.find((item) => item.id === id)
    })
    const query = useBusQuery<SessionFull>(
        'session.get',
        { session: name },
        {
            enabled: !!name,
            events: ['session.changed'],
        }
    )
    const session = query.data
    const lifecycle = useSessionLifecycle(session, { onClosed: () => router.back() })

    const [showBrief, setShowBrief] = useState(false)
    const brief = useBusQuery<{ text: string }>(
        'session.brief',
        { session: name },
        {
            enabled: !!name && showBrief,
            refetchOnFocus: false,
        }
    )

    // The form shows the session until the person edits something; then their draft.
    const [draft, setDraft] = useState<Draft | undefined>(undefined)
    const [saving, setSaving] = useState(false)

    if (!name) {
        return (
            <Screen title="Session">
                <EmptyState icon="code" title="No session" text="Open a session from the PC tab." />
            </Screen>
        )
    }

    if (!session) {
        return (
            <Screen title={name}>
                {query.error ? (
                    <ErrorState error={query.error} onRetry={query.reload} />
                ) : (
                    <LoadingState />
                )}
            </Screen>
        )
    }

    const spawned = !!session.spawned_at
    const closed = session.state === 'closed'
    const form: Draft = draft ?? {
        model: session.model ?? '',
        effort: session.effort ?? '',
        writes: session.bus_writes,
        allowUi: session.allow_ui,
    }
    const { model, effort, writes, allowUi } = form
    const edited = draft !== undefined
    const edit = (next: Partial<Draft>) => setDraft({ ...form, ...next })

    const save = async () => {
        const payload: Record<string, unknown> = {
            session: name,
            bus_writes: writes,
            allow_ui: allowUi,
        }
        // Model and effort are fixed once the agent has started (a conflict otherwise), and
        // neither can be set back to empty: only a real change is sent.
        if (!spawned) {
            if (model.trim() && model.trim() !== (session.model ?? '')) payload.model = model.trim()
            if (effort && effort !== (session.effort ?? '')) payload.effort = effort
        }
        setSaving(true)
        try {
            const next = await relay.guarded<SessionFull>('session.update', payload)
            query.setData(next)
            setDraft(undefined)
            Logger.infoToast('Session settings saved')
        } catch (e) {
            if (!isCancelled(e)) Logger.errorToast(`${(e as Error).message}`)
        } finally {
            setSaving(false)
        }
    }

    const facts: { label: string; value?: string | null; mono?: boolean }[] = [
        { label: 'Intent', value: session.intent },
        { label: 'Branch', value: session.branch, mono: true },
        { label: 'Worktree', value: session.worktree, mono: true },
        { label: 'Task', value: session.task_id ? `#${session.task_id}` : undefined },
        { label: 'Paired with', value: session.pair_with },
        { label: 'Model', value: session.model || 'Provider default' },
        { label: 'Effort', value: session.effort || 'Provider default' },
        { label: 'Process', value: session.pid ? `pid ${session.pid}` : undefined },
        {
            label: 'Exit code',
            value: session.exit_code !== null ? String(session.exit_code) : undefined,
        },
        { label: 'Created', value: when(session.created_at) },
        { label: 'Started', value: when(session.spawned_at) },
        { label: 'Last output', value: when(session.last_output_at) },
        { label: 'Closed', value: when(session.closed_at) },
    ]
    const choices = effortChoices(session.provider)

    return (
        <Screen
            title={name}
            onRefresh={query.reload}
            refreshing={query.loading}
            actions={
                closed
                    ? []
                    : [
                          {
                              icon: 'code',
                              label: 'Terminal',
                              onPress: () => router.push(terminalHref(name)),
                          },
                      ]
            }>
            {lifecycle.sheets}
            <Section card={false}>
                <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: spacing.s }}>
                    <Chip
                        label={session.state}
                        tone={
                            session.state === 'running'
                                ? 'live'
                                : session.state === 'blocked'
                                  ? 'danger'
                                  : 'neutral'
                        }
                        selected
                    />
                    <Chip label={session.provider} />
                    <Chip label={session.role} />
                    {!!project && <Chip label={project.name} icon="folder" />}
                </View>
            </Section>

            {lifecycleActions(session.state).length > 0 && (
                <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: spacing.m }}>
                    {!closed && (
                        <ThemedButton
                            label="Terminal"
                            iconName="code"
                            variant="secondary"
                            onPress={() => router.push(terminalHref(name))}
                        />
                    )}
                    {lifecycle.actions.map((action) => (
                        <ThemedButton
                            key={action.key}
                            label={
                                lifecycle.busy === action.key ? `${action.label}…` : action.label
                            }
                            variant={
                                lifecycle.busy
                                    ? 'disabled'
                                    : action.destructive
                                      ? 'tertiary'
                                      : 'secondary'
                            }
                            onPress={() => lifecycle.run(action.key)}
                        />
                    ))}
                </View>
            )}

            <Section title="Details">
                {facts
                    .filter((fact) => !!fact.value)
                    .map((fact) => (
                        <Row
                            key={fact.label}
                            label={fact.label}
                            detail={fact.value ?? undefined}
                            mono={fact.mono}
                            detailLines={4}
                        />
                    ))}
            </Section>

            <Section
                title="Launch brief"
                action={{
                    label: showBrief ? 'Hide' : 'Show',
                    onPress: () => setShowBrief((on) => !on),
                }}>
                {!showBrief ? (
                    <Row
                        label="What the agent is told at launch"
                        detail="State, peers, notes, adjacent work and skills."
                        onPress={() => setShowBrief(true)}
                    />
                ) : brief.data ? (
                    <View style={{ paddingVertical: spacing.l }}>
                        <Mono>{brief.data.text || 'The brief is empty.'}</Mono>
                    </View>
                ) : brief.error ? (
                    <ErrorState error={brief.error} onRetry={brief.reload} />
                ) : (
                    <LoadingState />
                )}
            </Section>

            {!closed && (
                <Section title="Settings">
                    <View style={{ rowGap: spacing.l, paddingVertical: spacing.l }}>
                        <Field
                            label="Model"
                            value={model}
                            onChangeText={(text) => edit({ model: text })}
                            placeholder="Provider default, or a model ID"
                            autoCapitalize="none"
                            autoCorrect={false}
                            editable={!spawned}
                            description={
                                spawned
                                    ? 'Model and effort are fixed once the agent has started.'
                                    : undefined
                            }
                        />
                        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: spacing.s }}>
                            {choices.map((choice) => (
                                <Chip
                                    key={choice}
                                    label={choice}
                                    tone="primary"
                                    selected={effort === choice}
                                    onPress={spawned ? undefined : () => edit({ effort: choice })}
                                />
                            ))}
                        </View>
                    </View>
                    <SwitchRow
                        label="Allow agent bus writes"
                        description="Let the agent create tasks, notes and mail."
                        value={writes}
                        onChange={(value) => edit({ writes: value })}
                    />
                    <SwitchRow
                        label="Allow UI control"
                        description="Let the agent drive the desktop's panes and pages."
                        value={allowUi}
                        onChange={(value) => edit({ allowUi: value })}
                    />
                    <View style={{ paddingVertical: spacing.l, alignItems: 'flex-end' }}>
                        <ThemedButton
                            label={saving ? 'Saving…' : 'Save settings'}
                            variant={saving || !edited ? 'disabled' : 'primary'}
                            onPress={save}
                        />
                    </View>
                </Section>
            )}
        </Screen>
    )
}

export default SessionScreen
