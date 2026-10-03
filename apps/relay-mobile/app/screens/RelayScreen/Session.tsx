import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { setStringAsync } from 'expo-clipboard'
import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'

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
    Screen,
    Section,
    SwitchRow,
    useBusQuery,
    useRelayStore,
} from '@components/relay'
import MailThread from '@components/relay/mail/MailThread'
import { effortChoices, terminalHref, useSessionLifecycle } from '@components/relay/sessions'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { ago, stateColor } from './console'

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
 * `session_menu`). A header card with its state, the actions in one row of pills, then the
 * details from `session.get`, the launch brief from `session.brief` (folded until asked for),
 * model/effort (before the first spawn only) and permissions through `session.update`, and
 * its mail.
 */
const SessionScreen = () => {
    const router = useRouter()
    const styles = useStyles()
    const { color, spacing } = Theme.useTheme()
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

    const facts: Fact[] = [
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
    const tint = stateColor(session.state, color)
    const quiet = ago(session.last_output_at)
    const pills: Pill[] = [
        ...(closed
            ? []
            : [
                  {
                      key: 'terminal',
                      label: 'Terminal',
                      icon: 'code' as const,
                      primary: true,
                      onPress: () => router.push(terminalHref(name)),
                  },
              ]),
        ...lifecycle.actions.map((action) => ({
            key: action.key,
            label: lifecycle.busy === action.key ? `${action.label}…` : action.label,
            icon: ICONS[action.key],
            destructive: action.destructive,
            onPress: () => lifecycle.run(action.key),
        })),
    ]

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
            <View style={styles.head}>
                <View style={styles.headLine}>
                    <View style={[styles.state, { borderColor: tint }]}>
                        <View style={[styles.dot, { backgroundColor: tint }]} />
                        <Text style={[styles.stateText, { color: tint }]}>{session.state}</Text>
                    </View>
                    <Text numberOfLines={1} style={styles.who}>
                        {session.provider} · {session.role}
                    </Text>
                </View>
                {!!project && (
                    <View style={styles.headLine}>
                        <AntDesign name="folder" size={13} color={color.text._500} />
                        <Text numberOfLines={1} style={styles.meta}>
                            {project.name}
                            {session.pid ? ` · pid ${session.pid}` : ''}
                            {quiet ? ` · output ${quiet} ago` : ''}
                        </Text>
                    </View>
                )}
                {!!session.intent && <Text style={styles.intent}>{session.intent}</Text>}
            </View>

            {pills.length > 0 && (
                <ScrollView
                    horizontal
                    showsHorizontalScrollIndicator={false}
                    contentContainerStyle={styles.pills}>
                    {pills.map((pill) => (
                        <TouchableOpacity
                            key={pill.key}
                            disabled={!!lifecycle.busy}
                            style={[
                                styles.pill,
                                pill.primary && styles.pillPrimary,
                                !!lifecycle.busy && styles.dim,
                            ]}
                            onPress={pill.onPress}>
                            <AntDesign
                                name={pill.icon}
                                size={14}
                                color={
                                    pill.primary
                                        ? color.primary._100
                                        : pill.destructive
                                          ? color.error._300
                                          : color.text._200
                                }
                            />
                            <Text
                                style={[
                                    styles.pillText,
                                    pill.primary && styles.pillTextPrimary,
                                    pill.destructive && styles.danger,
                                ]}>
                                {pill.label}
                            </Text>
                        </TouchableOpacity>
                    ))}
                </ScrollView>
            )}

            <Section title="Details">
                <View style={styles.facts}>
                    {facts
                        .filter((fact) => !!fact.value)
                        .map((fact) => (
                            <FactRow key={fact.label} fact={fact} />
                        ))}
                </View>
            </Section>

            <View style={styles.fold}>
                <TouchableOpacity style={styles.foldHead} onPress={() => setShowBrief((on) => !on)}>
                    <View style={{ flex: 1, rowGap: 2 }}>
                        <Text style={styles.foldTitle}>Launch brief</Text>
                        <Text style={styles.meta}>
                            What the agent is told at launch: state, peers, notes and skills.
                        </Text>
                    </View>
                    <AntDesign name={showBrief ? 'up' : 'down'} size={12} color={color.text._500} />
                </TouchableOpacity>
                {showBrief && (
                    <View style={{ paddingTop: spacing.m }}>
                        {brief.data ? (
                            <Mono>{brief.data.text || 'The brief is empty.'}</Mono>
                        ) : brief.error ? (
                            <ErrorState error={brief.error} onRetry={brief.reload} />
                        ) : (
                            <LoadingState />
                        )}
                    </View>
                )}
            </View>

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
            {session && (
                <Section title="Mail">
                    <MailThread project_id={session.project_id} session={name} />
                </Section>
            )}
        </Screen>
    )
}

export default SessionScreen

type Fact = { label: string; value?: string | null; mono?: boolean }

type Pill = {
    key: string
    label: string
    icon: AntDesignIconName
    primary?: boolean
    destructive?: boolean
    onPress: () => void
}

const ICONS: Record<string, AntDesignIconName> = {
    start: 'caret-right',
    wake: 'caret-right',
    resume: 'caret-right',
    park: 'pause',
    clear: 'reload',
    close: 'close',
}

/**
 * One fact: a small label and its value. Paths and branches are monospace, cut in the middle
 * so both ends stay readable; a long press copies the whole value.
 */
const FactRow: React.FC<{ fact: Fact }> = ({ fact }) => {
    const styles = useStyles()
    const value = fact.value ?? ''
    return (
        <TouchableOpacity
            style={styles.fact}
            disabled={!fact.mono}
            onLongPress={() => {
                setStringAsync(value)
                    .then(() => Logger.infoToast(`Copied ${fact.label.toLowerCase()}`))
                    .catch(() => {})
            }}>
            <Text style={styles.factLabel}>{fact.label}</Text>
            <Text
                numberOfLines={fact.mono ? 1 : 3}
                ellipsizeMode={fact.mono ? 'middle' : 'tail'}
                style={[styles.factValue, fact.mono && styles.mono]}>
                {value}
            </Text>
        </TouchableOpacity>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        head: {
            rowGap: spacing.m,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.l,
            borderRadius: 14,
            backgroundColor: color.neutral._200,
        },
        headLine: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        state: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: 6,
            paddingHorizontal: spacing.m,
            paddingVertical: 2,
            borderRadius: 999,
            borderWidth: 1,
        },
        dot: {
            width: 7,
            height: 7,
            borderRadius: 4,
        },
        stateText: {
            fontSize: fontSize.s,
            fontWeight: '600',
        },
        who: {
            flex: 1,
            color: color.text._300,
            fontSize: fontSize.s,
        },
        meta: {
            flex: 1,
            color: color.text._500,
            fontSize: fontSize.s - 1,
        },
        intent: {
            color: color.text._100,
            fontSize: fontSize.m,
            lineHeight: 21,
        },
        pills: {
            columnGap: spacing.s,
        },
        pill: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: 6,
            height: 34,
            paddingHorizontal: spacing.l,
            borderRadius: 999,
            backgroundColor: color.neutral._200,
        },
        pillPrimary: {
            backgroundColor: color.primary._500,
        },
        pillText: {
            color: color.text._200,
            fontSize: fontSize.s,
            fontWeight: '600',
        },
        pillTextPrimary: {
            color: color.primary._100,
        },
        danger: {
            color: color.error._300,
        },
        dim: {
            opacity: 0.5,
        },
        facts: {
            paddingVertical: spacing.s,
        },
        fact: {
            flexDirection: 'row',
            alignItems: 'baseline',
            columnGap: spacing.l,
            paddingVertical: 7,
        },
        factLabel: {
            width: 88,
            color: color.text._500,
            fontSize: fontSize.s - 1,
        },
        factValue: {
            flex: 1,
            color: color.text._200,
            fontSize: fontSize.s,
        },
        mono: {
            fontFamily: 'monospace',
            fontSize: fontSize.s - 1,
        },
        fold: {
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            borderRadius: 14,
            backgroundColor: color.neutral._200,
        },
        foldHead: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        foldTitle: {
            color: color.text._100,
            fontSize: fontSize.m,
        },
    })
}
