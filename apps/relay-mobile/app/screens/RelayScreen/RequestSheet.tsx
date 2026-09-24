import { useRouter } from 'expo-router'
import React, { useEffect, useState } from 'react'
import { Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import DropdownSheet from '@components/input/DropdownSheet'
import HorizontalSelector from '@components/input/HorizontalSelector'
import ThemedSwitch from '@components/input/ThemedSwitch'
import ThemedTextInput from '@components/input/ThemedTextInput'
import BottomSheet, { useBottomSheetRef } from '@components/views/BottomSheet'
import { relay, RelayProject, RelaySession, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type RequestSheetProps = {
    visible: boolean
    setVisible: (visible: boolean) => void
    /** The project the PC tab is narrowed to, preselected when the sheet opens. */
    projectId?: number
}

type Provider = 'claude' | 'codex'
type Role = 'builder' | 'docs'

type ProviderInfo = { provider: Provider; installed: boolean; signed_in_as?: string | null }

/**
 * Hand an agent on the PC a piece of work from the phone. The request becomes a task on the
 * project's board and a fresh session is launched for it — the same `task.dispatch` the desktop
 * uses — so the desktop wall, the audit log and the review column all see it as yours.
 */
const RequestSheet: React.FC<RequestSheetProps> = ({ visible, setVisible, projectId }) => {
    const { color, spacing, fontSize } = Theme.useTheme()
    const router = useRouter()
    const projects = useRelayStore((state) => state.projects)
    // A pick is remembered with the scope it was made under, so narrowing the PC tab to
    // another project starts the sheet there instead of on an older choice.
    const [choice, setChoice] = useState<{ scope?: number; project?: RelayProject }>({})
    const [provider, setProvider] = useState<Provider>('claude')
    const [role, setRole] = useState<Role>('builder')
    const [onBoard, setOnBoard] = useState(true)
    const [text, setText] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState('')
    const [providers, setProviders] = useState<ProviderInfo[]>([])
    const sheet = useBottomSheetRef()

    // The person's pick under the current scope, else the scoped project, else the first one;
    // a pick of a project the PC no longer has falls through.
    const picked = choice.scope === projectId ? choice.project : undefined
    const current =
        (picked && projects.find((item) => item.id === picked.id)) ??
        projects.find((item) => item.id === projectId) ??
        projects[0]

    useEffect(() => {
        if (visible) sheet.current?.open()
        else sheet.current?.close()
    }, [visible, sheet])

    useEffect(() => {
        if (!visible) return
        // Runs when the sheet opens; the selection is the person's afterwards.
        relay
            .request<{ providers: ProviderInfo[] }>('provider.list', {})
            .then((result) => {
                setProviders(result.providers)
                const installed = result.providers.filter((item) => item.installed)
                if (installed.length === 0) return
                setProvider((current) =>
                    installed.some((item) => item.provider === current)
                        ? current
                        : installed[0].provider
                )
            })
            .catch(() => {})
    }, [visible])

    const providerLabel = (name: Provider) => {
        const info = providers.find((item) => item.provider === name)
        if (!info) return name
        if (!info.installed) return `${name} (not installed)`
        return name
    }

    const send = async () => {
        const body = text.trim()
        const project = current
        if (!project) {
            setError('Pick a project on the PC first.')
            return
        }
        if (!body) {
            setError('Say what you want done.')
            return
        }
        setBusy(true)
        setError('')
        try {
            let session: RelaySession
            if (onBoard) {
                const firstLine = body.split('\n')[0].trim()
                const title = firstLine.length > 80 ? firstLine.slice(0, 77) + '…' : firstLine
                const task = await relay.request<{ id: number }>('task.create', {
                    project_id: project.id,
                    title: title,
                    body: body,
                })
                const dispatched = await relay.request<{ session: RelaySession }>('task.dispatch', {
                    task_id: task.id,
                    create: { project_id: project.id, provider: provider, role: role },
                    start: true,
                })
                session = dispatched.session
            } else {
                const created = await relay.request<RelaySession>('session.create', {
                    project_id: project.id,
                    provider: provider,
                    role: role,
                    prompt: body,
                })
                session = await relay.request<RelaySession>('session.spawn', {
                    session: created.name,
                })
            }
            Logger.infoToast(`Sent to ${session.name}`)
            setText('')
            setVisible(false)
            relay.refresh().catch(() => {})
            router.push({
                pathname: '/screens/RelayScreen/Terminal',
                params: { session: session.name },
            })
        } catch (e) {
            setError(`${(e as Error).message}`)
        } finally {
            setBusy(false)
        }
    }

    return (
        <BottomSheet
            ref={sheet}
            onClose={() => setVisible(false)}
            sheetStyle={{ maxHeight: '90%' }}>
            <View style={{ rowGap: spacing.l }}>
                <Text style={{ color: color.text._100, fontSize: fontSize.l }}>New request</Text>
                <Text style={{ color: color.text._400 }}>
                    An agent on the PC starts on this in its own worktree. Watch it in the terminal
                    that opens, and answer its questions there.
                </Text>

                <View>
                    <Text style={{ color: color.text._100, marginBottom: spacing.m }}>Project</Text>
                    <DropdownSheet
                        data={projects}
                        selected={current}
                        onChangeValue={(item) => setChoice({ scope: projectId, project: item })}
                        labelExtractor={(item) => item.name}
                        placeholder={
                            projects.length === 0 ? 'No projects on the PC' : 'Pick a project'
                        }
                        modalTitle="Project"
                        search={projects.length > 6}
                    />
                </View>

                <HorizontalSelector
                    style={{ flex: 0 }}
                    label="Agent"
                    values={[
                        { label: providerLabel('claude'), value: 'claude' as Provider },
                        { label: providerLabel('codex'), value: 'codex' as Provider },
                    ]}
                    selected={provider}
                    onPress={setProvider}
                />

                <HorizontalSelector
                    style={{ flex: 0 }}
                    label="Role"
                    values={[
                        { label: 'Builder', value: 'builder' as Role },
                        { label: 'Docs', value: 'docs' as Role },
                    ]}
                    selected={role}
                    onPress={setRole}
                />

                <ThemedTextInput
                    label="What should it do?"
                    value={text}
                    onChangeText={setText}
                    placeholder="Fix the flaky login test and explain what was wrong."
                    multiline
                    numberOfLines={5}
                    autoUnfocus={false}
                />

                <ThemedSwitch
                    label="Track it on the board"
                    value={onBoard}
                    onChangeValue={setOnBoard}
                    description="On: a task is created and dispatched, so it lands in review when the agent reports done. Off: a one-off session with this as its opening prompt."
                />

                {!!error && <Text style={{ color: color.error._300 }}>{error}</Text>}

                <View style={{ flexDirection: 'row', justifyContent: 'space-between' }}>
                    <ThemedButton
                        label="Cancel"
                        variant="secondary"
                        onPress={() => setVisible(false)}
                    />
                    <ThemedButton
                        label={busy ? 'Sending…' : 'Send to PC'}
                        iconName="right"
                        variant={busy ? 'disabled' : 'primary'}
                        onPress={send}
                    />
                </View>
            </View>
        </BottomSheet>
    )
}

export default RequestSheet
