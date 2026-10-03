import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { ScrollView, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    Field,
    relay,
    Row,
    Segmented,
    Sheet,
    SwitchRow,
    useBusQuery,
    useRelayStore,
} from '@components/relay'
import { useSheetStyles } from '@components/relay/Sheet'
import { Logger } from '@lib/state/Logger'

import { usePieceStyles } from './Pieces'
import { attempt, DISPATCHABLE, Task } from './types'

type Provider = 'claude' | 'codex'
type Role = 'builder' | 'reviewer' | 'docs'

/** launch.rs: codex has no `max`, claude has no `minimal`; both default to `high`. */
const EFFORTS: Record<Provider, string[]> = {
    claude: ['low', 'medium', 'high', 'xhigh', 'max'],
    codex: ['minimal', 'low', 'medium', 'high', 'xhigh'],
}

const ROLES: { value: Role; label: string }[] = [
    { value: 'builder', label: 'Builder' },
    { value: 'reviewer', label: 'Reviewer' },
    { value: 'docs', label: 'Docs' },
]

type ProviderInfo = { provider: Provider; installed: boolean; version: string | null }

/**
 * Send a task to an agent: an existing session of the project, or a new one with the
 * launcher's options (task.dispatch {session} or {create, start}). "Stage only" assigns the
 * task without starting the agent (start: false).
 */
export const DispatchSheet: React.FC<{ task?: Task; onDismiss: () => void }> = ({
    task,
    onDismiss,
}) => {
    const router = useRouter()
    const sheet = useSheetStyles()
    const styles = usePieceStyles()
    const sessions = useRelayStore((state) => state.sessions)
    const live = sessions.filter(
        (s) => s.project_id === task?.project_id && DISPATCHABLE.includes(s.state)
    )
    const providers = useBusQuery<ProviderInfo[]>(
        'provider.list',
        {},
        { select: (r) => r.providers, enabled: !!task, refetchOnFocus: false }
    )
    const [mode, setMode] = useState<'new' | 'existing'>('new')
    const [target, setTarget] = useState<string | undefined>(undefined)
    const [provider, setProvider] = useState<Provider>('claude')
    const [role, setRole] = useState<Role>('builder')
    const [model, setModel] = useState('')
    const [effort, setEffort] = useState('high')
    const [worktree, setWorktree] = useState('new')
    const [busWrites, setBusWrites] = useState(true)
    const [allowUi, setAllowUi] = useState(false)
    const [fanout, setFanout] = useState(false)
    const [busy, setBusy] = useState(false)

    // A fresh choice for each task the sheet opens on.
    const [openedFor, setOpenedFor] = useState<number | undefined>(undefined)
    if (task && task.id !== openedFor) {
        setOpenedFor(task.id)
        setMode('new')
        setTarget(undefined)
        setFanout(false)
    }
    // Switching provider keeps the effort when the new one has it, else its default.
    const shownEffort = EFFORTS[provider].includes(effort) ? effort : 'high'

    const installed = (providers.data ?? []).filter((p) => p.installed)
    const providerOptions = (
        installed.length > 0 ? installed.map((p) => p.provider) : ['claude', 'codex']
    ).map((p) => ({ value: p as Provider, label: p === 'claude' ? 'Claude' : 'Codex' }))

    const send = async (start: boolean) => {
        if (!task) return
        let payload: Record<string, unknown>
        if (mode === 'existing') {
            if (!target) return
            payload = { task_id: task.id, session: target, start: start }
        } else {
            payload = {
                task_id: task.id,
                create: {
                    project_id: task.project_id,
                    provider: provider,
                    role: role,
                    ...(model.trim() ? { model: model.trim() } : {}),
                    effort: shownEffort,
                    worktree: worktree.trim() || 'new',
                    bus_writes: busWrites,
                    allow_ui: allowUi,
                },
                start: start,
                ...(fanout ? { fanout: true } : {}),
            }
        }
        setBusy(true)
        const result = await attempt(() =>
            relay.guarded<{ session: { name: string }; fanned: unknown[] }>(
                'task.dispatch',
                payload
            )
        )
        setBusy(false)
        if (!result) return
        onDismiss()
        const more = result.fanned?.length ? ` (+${result.fanned.length} sub-tasks)` : ''
        if (start) {
            router.push({
                pathname: '/screens/RelayScreen/Terminal',
                params: { session: result.session.name },
            })
        }
        Logger.infoToast(
            start
                ? `Sent to ${result.session.name}${more}`
                : `Staged on ${result.session.name}${more}`
        )
    }

    const ready = mode === 'new' || !!target
    return (
        <Sheet visible={!!task} onDismiss={onDismiss}>
            <View style={[sheet.body, styles.shrink]}>
                <Text numberOfLines={2} style={sheet.title}>
                    Dispatch #{task?.id}
                </Text>
                <Segmented
                    options={[
                        { value: 'new', label: 'New agent' },
                        { value: 'existing', label: `Existing · ${live.length}` },
                    ]}
                    value={mode}
                    onChange={setMode}
                />
                <ScrollView
                    style={[sheet.scroll, styles.shrink]}
                    keyboardShouldPersistTaps="handled">
                    {mode === 'existing' ? (
                        live.length === 0 ? (
                            <Text style={styles.meta}>
                                No live session in this project. Start a new agent instead.
                            </Text>
                        ) : (
                            live.map((s) => (
                                <Row
                                    key={s.name}
                                    label={s.name}
                                    detail={`${s.provider} · ${s.role} · ${s.state}${s.task_id ? ` · on #${s.task_id}` : ''}`}
                                    icon={target === s.name ? 'check-circle' : 'user'}
                                    chevron={false}
                                    onPress={() => setTarget(s.name)}
                                />
                            ))
                        )
                    ) : (
                        <View style={{ rowGap: 12 }}>
                            <Text style={styles.label}>Provider</Text>
                            <Segmented
                                options={providerOptions}
                                value={provider}
                                onChange={setProvider}
                            />
                            <Text style={styles.label}>Role</Text>
                            <Segmented options={ROLES} value={role} onChange={setRole} />
                            <Field
                                label="Model"
                                value={model}
                                onChangeText={setModel}
                                placeholder="Provider default, or a model ID"
                                autoCapitalize="none"
                                autoCorrect={false}
                            />
                            <Text style={styles.label}>Reasoning effort</Text>
                            <View style={styles.chips}>
                                {EFFORTS[provider].map((value) => (
                                    <Chip
                                        key={value}
                                        label={value}
                                        tone={value === shownEffort ? 'primary' : 'neutral'}
                                        selected={value === shownEffort}
                                        onPress={() => setEffort(value)}
                                    />
                                ))}
                            </View>
                            <Field
                                label="Worktree"
                                value={worktree}
                                onChangeText={setWorktree}
                                description="new, primary, or an absolute worktree path on the PC"
                                autoCapitalize="none"
                                autoCorrect={false}
                            />
                            <SwitchRow
                                label="Allow agent bus writes"
                                value={busWrites}
                                onChange={setBusWrites}
                            />
                            <SwitchRow
                                label="Allow UI control"
                                value={allowUi}
                                onChange={setAllowUi}
                            />
                            {!!task && task.children.length > 0 && (
                                <SwitchRow
                                    label="Sub-tasks too"
                                    description="One new agent per open sub-task, with the same options"
                                    value={fanout}
                                    onChange={setFanout}
                                />
                            )}
                        </View>
                    )}
                </ScrollView>
                <View style={sheet.actions}>
                    <ThemedButton
                        label="Stage only"
                        variant={busy || !ready ? 'disabled' : 'secondary'}
                        onPress={busy || !ready ? undefined : () => send(false)}
                    />
                    <ThemedButton
                        label={busy ? 'Sending…' : 'Dispatch'}
                        variant={busy || !ready ? 'disabled' : 'primary'}
                        onPress={busy || !ready ? undefined : () => send(true)}
                    />
                </View>
            </View>
        </Sheet>
    )
}
