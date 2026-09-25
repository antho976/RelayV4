import { useRouter } from 'expo-router'
import React, { useEffect, useState } from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import DropdownSheet from '@components/input/DropdownSheet'
import { Field, Segmented } from '@components/relay/Kit'
import { launchHref, terminalHref, useNewTerminalPrefs } from '@components/relay/sessions'
import BottomSheet, { useBottomSheetRef } from '@components/views/BottomSheet'
import {
    isCancelled,
    relay,
    RelayProject,
    RelaySession,
    useRelayStore,
} from '@lib/engine/Relay/RelayClient'
import { Theme } from '@lib/theme/ThemeManager'

type NewTerminalSheetProps = {
    visible: boolean
    setVisible: (visible: boolean) => void
    /** The project the PC tab is narrowed to, preselected when the sheet opens. */
    projectId?: number
}

type Provider = 'claude' | 'codex'
type Role = 'builder' | 'docs'

type ProviderInfo = { provider: Provider; installed: boolean; signed_in_as?: string | null }

/**
 * Start one agent on the PC and open its terminal: `session.create` in a fresh worktree, then
 * `session.spawn`, with the first message as its prompt when one is given. The project
 * defaults to the one the tab is narrowed to, else the last one used here; the agent to the
 * last one used. Several agents, review groups, models and worktrees are the launcher's.
 */
const NewTerminalSheet: React.FC<NewTerminalSheetProps> = ({ visible, setVisible, projectId }) => {
    const styles = useStyles()
    const router = useRouter()
    const sheet = useBottomSheetRef()
    const projects = useRelayStore((state) => state.projects)
    const prefs = useNewTerminalPrefs()
    const [picked, setPicked] = useState<RelayProject | undefined>(undefined)
    const [provider, setProvider] = useState<Provider>(prefs.provider)
    const [role, setRole] = useState<Role>('builder')
    const [text, setText] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState('')
    const [providers, setProviders] = useState<ProviderInfo[]>([])

    const project =
        (picked && projects.find((item) => item.id === picked.id)) ??
        projects.find((item) => item.id === projectId) ??
        projects.find((item) => item.id === prefs.projectId) ??
        projects[0]

    useEffect(() => {
        if (visible) sheet.current?.open()
        else sheet.current?.close()
    }, [visible, sheet])

    useEffect(() => {
        if (!visible) return
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

    // Each opening starts from the scope and the last choices, not an abandoned pick.
    const dismiss = () => {
        setVisible(false)
        setPicked(undefined)
        setError('')
        setRole('builder')
    }

    const providerLabel = (name: Provider) =>
        providers.find((item) => item.provider === name)?.installed === false
            ? `${name} (not installed)`
            : name

    const start = async () => {
        if (!project) {
            setError('Add a project on the PC first.')
            return
        }
        const prompt = text.trim()
        setBusy(true)
        setError('')
        try {
            const created = await relay.guarded<RelaySession>('session.create', {
                project_id: project.id,
                provider: provider,
                role: role,
            })
            const session = await relay.guarded<RelaySession>(
                'session.spawn',
                prompt ? { session: created.name, prompt: prompt } : { session: created.name }
            )
            prefs.remember(project.id, provider)
            setText('')
            dismiss()
            relay.refresh().catch(() => {})
            router.push(terminalHref(session.name ?? created.name))
        } catch (e) {
            if (!isCancelled(e)) setError(`${(e as Error).message}`)
        } finally {
            setBusy(false)
        }
    }

    // The full launcher: several agents, a review group, models, worktrees, staged tasks.
    // What was typed so far becomes the first agent's instructions there.
    const moreOptions = () => {
        dismiss()
        router.push(launchHref(project?.id, { prompt: text.trim() || undefined }))
    }

    return (
        <BottomSheet ref={sheet} onClose={dismiss} sheetStyle={{ maxHeight: '90%' }}>
            <View style={styles.body}>
                <Text style={styles.title}>New terminal</Text>

                <DropdownSheet
                    data={projects}
                    selected={project}
                    onChangeValue={setPicked}
                    labelExtractor={(item) => item.name}
                    placeholder={projects.length === 0 ? 'No projects on the PC' : 'Pick a project'}
                    modalTitle="Project"
                    search={projects.length > 6}
                />

                <Segmented
                    options={[
                        { value: 'claude' as Provider, label: providerLabel('claude') },
                        { value: 'codex' as Provider, label: providerLabel('codex') },
                    ]}
                    value={provider}
                    onChange={setProvider}
                />

                <Segmented
                    options={[
                        { value: 'builder' as Role, label: 'Builder' },
                        { value: 'docs' as Role, label: 'Docs' },
                    ]}
                    value={role}
                    onChange={setRole}
                />

                <Field
                    value={text}
                    onChangeText={setText}
                    placeholder="First message (optional)"
                    multiline
                    lines={3}
                />

                {!!error && <Text style={styles.error}>{error}</Text>}

                <View style={styles.actions}>
                    <TouchableOpacity hitSlop={8} onPress={moreOptions}>
                        <Text style={styles.link}>More options</Text>
                    </TouchableOpacity>
                    <ThemedButton
                        label={busy ? 'Starting…' : 'Start'}
                        iconName="caret-right"
                        variant={busy || !project ? 'disabled' : 'primary'}
                        onPress={start}
                    />
                </View>
            </View>
        </BottomSheet>
    )
}

export default NewTerminalSheet

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        body: {
            rowGap: spacing.l,
        },
        title: {
            color: color.text._100,
            fontFamily: 'serif',
            fontSize: fontSize.xl2,
        },
        error: {
            color: color.error._300,
        },
        actions: {
            flexDirection: 'row',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginTop: spacing.s,
        },
        link: {
            color: color.primary._700,
            fontSize: fontSize.m,
        },
    })
}
