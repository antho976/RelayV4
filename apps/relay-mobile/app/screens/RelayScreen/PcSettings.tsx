import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Caption,
    ErrorState,
    LoadingState,
    relay,
    relayHref,
    Row,
    Screen,
    Section,
    SwitchRow,
    useBusQuery,
    useProjectParam,
} from '@components/relay'
import { attempt, formatBytes, formatTime, settingsHref } from '@components/relay/settings/common'
import GuardrailEditor from '@components/relay/settings/GuardrailEditor'
import Providers from '@components/relay/settings/Providers'
import { Theme } from '@lib/theme/ThemeManager'

/** The desktop's notification categories (tools_settings.rs), in its order. */
const CATEGORIES: { key: string; label: string; detail: string }[] = [
    { key: 'agent_done', label: 'Agent finished', detail: 'An agent reports its work done.' },
    { key: 'agent_blocked', label: 'Agent blocked', detail: 'An agent waits on you.' },
    { key: 'guardrail', label: 'Guardrail holds', detail: 'An action waits for your Allow.' },
    { key: 'integration', label: 'Integration', detail: 'Merges and builds finish or fail.' },
    { key: 'provider', label: 'Providers', detail: 'Provider updates and sign-in problems.' },
    { key: 'disk', label: 'Disk', detail: 'Worktrees and builds use a lot of space.' },
    { key: 'system', label: 'System', detail: 'Crashes on a device and other engine news.' },
]

type NotifySettings = { sound?: string; volume?: number; categories?: Record<string, boolean> }
type Backup = { path: string; bytes: number; created_at: string; reason: string }

/**
 * Settings of the PC itself that make sense from a phone: which notifications the PC raises,
 * the guardrail values, the agent providers, skills and plugins, and database backups.
 * Appearance, fonts, shortcuts, sounds and SDK paths stay on the desktop.
 */
const PcSettingsScreen = () => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const router = useRouter()
    const { projectId, project } = useProjectParam()
    const notify = useBusQuery<NotifySettings>(
        'notify.settings.get',
        {},
        {
            events: ['settings.changed'],
        }
    )
    const backups = useBusQuery<Backup[]>(
        'app.backup.list',
        {},
        {
            select: (raw) => raw.backups,
        }
    )
    const [guardrails, setGuardrails] = useState(projectId !== undefined)
    const [backingUp, setBackingUp] = useState(false)
    const [lastBackup, setLastBackup] = useState<{ path: string; bytes: number } | undefined>(
        undefined
    )

    const categories = notify.data?.categories ?? {}
    const toggle = async (key: string, value: boolean) => {
        notify.setData((current) => ({
            ...current,
            categories: { ...(current?.categories ?? {}), [key]: value },
        }))
        const next = await attempt(() =>
            relay.guarded<NotifySettings>('notify.settings.set', {
                patch: { categories: { [key]: value } },
            })
        )
        if (next) notify.setData(next)
        else notify.reload()
    }

    const backUp = async () => {
        setBackingUp(true)
        const result = await attempt(() =>
            relay.guarded<{ path: string; bytes: number }>('app.backup.now')
        )
        setBackingUp(false)
        if (result) {
            setLastBackup(result)
            backups.reload()
        }
    }

    const refresh = () => {
        notify.reload()
        backups.reload()
    }

    return (
        <Screen title="PC settings" onRefresh={refresh}>
            <Section title="Notifications">
                {notify.data === undefined ? (
                    notify.error ? (
                        <ErrorState error={notify.error} onRetry={notify.reload} />
                    ) : (
                        <LoadingState />
                    )
                ) : (
                    CATEGORIES.map((category) => (
                        <SwitchRow
                            key={category.key}
                            label={category.label}
                            description={category.detail}
                            value={categories[category.key] ?? true}
                            onChange={(value) => toggle(category.key, value)}
                        />
                    ))
                )}
            </Section>
            <Text style={styles.note}>
                These are the PC’s notifications, which this phone also shows. Their sound and
                volume are set on the PC.
            </Text>

            <Section title={project ? `Guardrails · ${project.name}` : 'Guardrails'}>
                <Row
                    label={guardrails ? 'Hide guardrail values' : 'Guardrail values'}
                    detail={
                        project
                            ? 'This project’s override of the PC-wide values'
                            : 'Caps, destructive-write thresholds, protected paths, denied commands'
                    }
                    icon="safety"
                    chevron={false}
                    onPress={() => setGuardrails(!guardrails)}
                />
                {guardrails && <GuardrailEditor projectId={projectId} />}
            </Section>

            <Providers />

            <Section title="Agents">
                <Row
                    label="Skills"
                    detail="Instruction files agents load for a kind of work"
                    icon="book"
                    onPress={() => router.push(settingsHref('Skills'))}
                />
                <Row
                    label="Plugins"
                    detail="Bundled skills, instructions and tools"
                    icon="appstore-add"
                    onPress={() => router.push(settingsHref('Plugins'))}
                />
            </Section>

            <Section title="Backups">
                <View style={{ rowGap: spacing.m, paddingVertical: spacing.l }}>
                    <Text style={styles.note}>
                        A copy of the PC’s Relay database (boards, notes, history). The PC keeps the
                        last five.
                    </Text>
                    {lastBackup && (
                        <Text style={styles.done} selectable>
                            Saved {formatBytes(lastBackup.bytes)} to {lastBackup.path}
                        </Text>
                    )}
                    <View style={styles.actions}>
                        <ThemedButton
                            label={backingUp ? 'Backing up…' : 'Back up now'}
                            iconName="save"
                            variant={backingUp ? 'disabled' : 'primary'}
                            onPress={backUp}
                        />
                    </View>
                </View>
                {backups.data && backups.data.length > 0 && <Caption>On the PC</Caption>}
                {backups.data?.map((backup) => (
                    <Row
                        key={backup.path}
                        label={formatTime(backup.created_at)}
                        detail={`${formatBytes(backup.bytes)} · ${backup.reason}\n${backup.path}`}
                        icon="database"
                        mono
                    />
                ))}
            </Section>

            <Section title="This phone">
                <Row
                    label="Paired PCs"
                    detail="How this phone reaches the PC"
                    icon="desktop"
                    onPress={() => router.push(relayHref('Hosts'))}
                />
            </Section>
        </Screen>
    )
}

export default PcSettingsScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        done: {
            color: color.text._200,
            fontSize: fontSize.s,
            fontFamily: 'monospace',
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
    })
}
