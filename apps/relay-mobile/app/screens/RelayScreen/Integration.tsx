import AntDesign from '@react-native-vector-icons/ant-design/static'
import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    QueryView,
    Row,
    Screen,
    Section,
    Sheet,
    Tone,
    useBusQuery,
    useProjectParam,
    useRelayStore,
} from '@components/relay'
import { afterSheet, Integration, IntegrationState, useGuardedAction } from '@components/relay/git'
import { useSheetStyles } from '@components/relay/Sheet'
import { Theme } from '@lib/theme/ThemeManager'

import { ago, palette } from './console'

const RUNNING: IntegrationState[] = ['queued', 'merging', 'building', 'deploying']

const tone = (state: IntegrationState): Tone => {
    switch (state) {
        case 'passed':
            return 'live'
        case 'failed':
        case 'conflict':
            return 'danger'
        case 'discarded':
            return 'neutral'
        default:
            return 'warn'
    }
}

/**
 * Test agents' branches together: octopus-merge two or more sessions' branches into a
 * throwaway worktree and build it, without touching anyone's own branch. Runs update live from
 * integration.* events; a finished run can be inspected (conflict, build log) and discarded.
 * Param: `project_id`. Desktop: relay-native code_git.rs, "Merge test".
 */
const IntegrationScreen = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const { projectId, project } = useProjectParam()
    const sessions = useRelayStore((state) => state.sessions)
    const open = sessions.filter((item) => item.project_id === projectId && item.state !== 'closed')
    const [picked, setPicked] = useState<string[]>([])
    const [detail, setDetail] = useState<number | undefined>(undefined)
    const { busy, run } = useGuardedAction()
    const runs = useBusQuery<Integration[]>(
        'integration.list',
        { project_id: projectId ?? 0 },
        {
            enabled: projectId !== undefined,
            events: ['integration.*'],
            projectId: projectId,
            debounceMs: 200,
            select: (raw) => [...raw.integrations].sort((a, b) => b.id - a.id),
        }
    )

    if (projectId === undefined) {
        return (
            <Screen title="Integration">
                <EmptyState icon="folder" title="No project" text="Open this from a project." />
            </Screen>
        )
    }

    const chosen = picked.filter((name) => open.some((item) => item.name === name))
    const toggle = (name: string) =>
        setPicked((now) =>
            now.includes(name) ? now.filter((item) => item !== name) : [...now, name]
        )
    const start = async () => {
        if (chosen.length < 2) return
        const result = await run<Integration>(
            'integration.request',
            { project_id: projectId, sessions: chosen, build: true },
            (out) => `Integration #${out.id} started`
        )
        if (result) {
            setPicked([])
            runs.reload()
        }
    }

    const discard = async (item: Integration) => {
        const yes = await confirm({
            title: `Discard integration #${item.id}?`,
            message: RUNNING.includes(item.state)
                ? 'It is still running. Discarding removes its throwaway worktree.'
                : 'Removes its throwaway worktree.',
            confirmLabel: 'Discard',
            destructive: true,
        })
        if (!yes) return
        const done = await run(
            'integration.discard',
            { integration_id: item.id },
            `Discarded #${item.id}`
        )
        if (done) runs.reload()
    }

    return (
        <Screen
            title={project ? `${project.name} · integration` : 'Integration'}
            onRefresh={runs.reload}
            refreshing={runs.loading && !!runs.data}>
            <Section title="Merge test">
                <Text style={styles.hint}>
                    Pick at least two sessions. Their branches are merged in a disposable worktree
                    and built there; your branches are left alone.
                </Text>
                {open.length === 0 && <Row label="No open sessions in this project" />}
                {open.map((session) => {
                    const on = chosen.includes(session.name)
                    return (
                        <Row
                            key={session.name}
                            label={session.name}
                            detail={session.branch}
                            mono
                            chevron={false}
                            onPress={() => toggle(session.name)}
                            right={
                                <AntDesign
                                    name={on ? 'check-square' : 'border'}
                                    size={20}
                                    color={on ? color.primary._700 : color.text._500}
                                />
                            }
                        />
                    )
                })}
            </Section>
            <ThemedButton
                label={
                    busy
                        ? 'Starting…'
                        : chosen.length >= 2
                          ? `Test merge + build · ${chosen.length}`
                          : 'Select at least 2 sessions'
                }
                variant={chosen.length >= 2 && !busy ? 'primary' : 'disabled'}
                onPress={start}
            />
            <QueryView
                query={runs}
                isEmpty={(list) => list.length === 0}
                empty={<EmptyState icon="experiment" title="No integration runs yet" />}>
                {(list) => (
                    <Section title={`Runs · ${list.length}`}>
                        {list.map((item) => (
                            <Row
                                key={item.id}
                                label={`#${item.id} · ${item.branches.join(' + ')}`}
                                detail={
                                    item.conflict
                                        ? `Conflict between ${item.conflict[0]} and ${item.conflict[1]}`
                                        : item.finished_at
                                          ? `Finished ${ago(item.finished_at)} ago`
                                          : item.started_at
                                            ? `Started ${ago(item.started_at)} ago`
                                            : 'Waiting'
                                }
                                right={<Chip label={item.state} tone={tone(item.state)} />}
                                onPress={() => setDetail(item.id)}
                            />
                        ))}
                    </Section>
                )}
            </QueryView>
            <DetailSheet
                id={detail}
                onDismiss={() => setDetail(undefined)}
                onDiscard={(item) => {
                    setDetail(undefined)
                    afterSheet(() => discard(item))
                }}
                busy={!!busy}
            />
        </Screen>
    )
}

export default IntegrationScreen

/** One run in full, kept live: state, branches, the conflicting pair, the build log's tail. */
const DetailSheet: React.FC<{
    id: number | undefined
    busy: boolean
    onDismiss: () => void
    onDiscard: (item: Integration) => void
}> = ({ id, busy, onDismiss, onDiscard }) => {
    const sheet = useSheetStyles()
    const styles = useStyles()
    const query = useBusQuery<Integration>(
        'integration.get',
        { integration_id: id ?? 0 },
        { enabled: id !== undefined, events: ['integration.*'], debounceMs: 200 }
    )
    const item = query.data
    return (
        <Sheet visible={id !== undefined} onDismiss={onDismiss}>
            <View style={sheet.body}>
                <Text style={sheet.title}>Integration #{id}</Text>
                <QueryView query={query}>
                    {(data) => (
                        <View style={styles.detail}>
                            <View style={styles.chips}>
                                <Chip label={data.state} tone={tone(data.state)} selected />
                                {data.branches.map((branch) => (
                                    <Chip key={branch} label={branch} icon="branches" />
                                ))}
                            </View>
                            {data.conflict && (
                                <Text style={styles.conflict}>
                                    {data.conflict[0]} conflicts with {data.conflict[1]}. Resolve it
                                    in one of those sessions, then test again.
                                </Text>
                            )}
                            {!!data.worktree && (
                                <Text selectable style={styles.path}>
                                    {data.worktree}
                                </Text>
                            )}
                            <Text style={styles.hint}>
                                {data.started_at ? `Started ${ago(data.started_at)} ago` : 'Queued'}
                                {data.finished_at ? ` · finished ${ago(data.finished_at)} ago` : ''}
                                {RUNNING.includes(data.state) ? ' · running' : ''}
                            </Text>
                            {!!data.log_tail && (
                                <ScrollView style={styles.logBox} nestedScrollEnabled>
                                    <ScrollView horizontal>
                                        <Text selectable style={styles.log}>
                                            {data.log_tail}
                                        </Text>
                                    </ScrollView>
                                </ScrollView>
                            )}
                        </View>
                    )}
                </QueryView>
                <View style={sheet.actions}>
                    <ThemedButton label="Close" variant="secondary" onPress={onDismiss} />
                    <ThemedButton
                        label="Discard"
                        variant={
                            item && item.state !== 'discarded' && !busy ? 'critical' : 'disabled'
                        }
                        onPress={() => {
                            if (item && item.state !== 'discarded' && !busy) onDiscard(item)
                        }}
                    />
                </View>
            </View>
        </Sheet>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        hint: {
            color: color.text._400,
            fontSize: fontSize.s,
            lineHeight: 18,
            paddingVertical: spacing.s,
        },
        detail: {
            rowGap: spacing.m,
        },
        chips: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            gap: spacing.s,
        },
        conflict: {
            color: color.error._300,
            lineHeight: 20,
        },
        path: {
            color: color.text._400,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        logBox: {
            maxHeight: 280,
            backgroundColor: palette.ink,
            borderRadius: 8,
            padding: spacing.m,
        },
        log: {
            color: palette.paper,
            fontFamily: 'monospace',
            fontSize: 11,
            lineHeight: 15,
        },
    })
}
