import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    HeldRequest,
    HoldDetails,
    QueryView,
    relay,
    RelayHoldFull,
    Row,
    Screen,
    Section,
    Sheet,
    Tone,
    useBusQuery,
    useProjectParam,
} from '@components/relay'
import { attempt } from '@components/relay/notes/types'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { ago } from './console'
import HoldItem from './HoldItem'

type Overlap = {
    id: number
    project_id: number
    sessions: string[]
    path: string
    symbol: string | null
    kind: 'file' | 'symbol' | 'claim'
    acked_by: string[]
    first_seen: string
    last_seen: string
}

const STATE_TONE: Record<string, Tone> = {
    open: 'warn',
    confirmed: 'live',
    rejected: 'danger',
    expired: 'neutral',
}

const STATE_LABEL: Record<string, string> = {
    open: 'Waiting',
    confirmed: 'Allowed',
    rejected: 'Denied',
    expired: 'Expired',
}

/**
 * A project's guardrails, as the desktop's page: holds waiting for a decision (inspect, then
 * Allow once or Deny — HoldItem), every earlier hold with how it ended, and shared file
 * activity: files or symbols two sessions both changed or claimed.
 */
const GuardrailsScreen = () => {
    const styles = useStyles()
    const { projectId, project } = useProjectParam()
    const enabled = projectId !== undefined
    const holds = useBusQuery<RelayHoldFull[]>(
        'guardrail.holds.list',
        { project_id: projectId, open_only: false },
        { events: ['guardrail.*'], projectId: projectId, enabled: enabled, select: (r) => r.holds }
    )
    const overlaps = useBusQuery<Overlap[]>(
        'overlap.list',
        { project_id: projectId },
        {
            events: ['overlap.*'],
            projectId: projectId,
            enabled: enabled,
            select: (r) => r.overlaps,
        }
    )
    const [shown, setShown] = useState<RelayHoldFull | undefined>(undefined)

    const reload = () => {
        holds.reload()
        overlaps.reload()
    }

    const tell = async (overlap: Overlap) => {
        if (projectId === undefined) return
        const subject = overlap.symbol ? `${overlap.symbol} in ${overlap.path}` : overlap.path
        const yes = await confirm({
            title: 'Tell these sessions?',
            message: `Sends each of ${overlap.sessions.join(', ')} a message that the others are changing ${subject} too.`,
            confirmLabel: 'Send',
        })
        if (!yes) return
        let sent = 0
        for (const session of overlap.sessions) {
            const others = overlap.sessions.filter((name) => name !== session).join(', ')
            const done = await attempt(() =>
                relay.guarded('mailbox.send', {
                    project_id: projectId,
                    to: session,
                    text: `Heads up: ${others} ${overlap.sessions.length > 2 ? 'are' : 'is'} also changing ${subject}. Coordinate before you commit.`,
                })
            )
            if (done) sent++
        }
        if (sent > 0) Logger.infoToast(`Told ${sent} session${sent === 1 ? '' : 's'}`)
    }

    return (
        <Screen
            title={project ? `Guardrails · ${project.name}` : 'Guardrails'}
            onRefresh={reload}
            refreshing={holds.loading && holds.data !== undefined}>
            <QueryView query={holds}>
                {(list) => {
                    const open = list.filter((hold) => hold.state === 'open')
                    const past = list.filter((hold) => hold.state !== 'open')
                    return (
                        <>
                            <Section
                                title={`Waiting for you${open.length ? ` · ${open.length}` : ''}`}
                                card={false}>
                                {open.length === 0 ? (
                                    <EmptyState
                                        icon="safety"
                                        text="No agent in this project is waiting on a guardrail."
                                    />
                                ) : (
                                    open.map((hold) => <HoldItem key={hold.id} hold={hold} />)
                                )}
                            </Section>
                            {past.length > 0 && (
                                <Section title="History">
                                    {past.map((hold) => (
                                        <Row
                                            key={hold.id}
                                            label={hold.op}
                                            mono
                                            detail={[
                                                hold.session ?? hold.actor,
                                                hold.policy,
                                                `${ago(hold.created_at)} ago`,
                                            ].join(' · ')}
                                            right={
                                                <Chip
                                                    label={STATE_LABEL[hold.state] ?? hold.state}
                                                    tone={STATE_TONE[hold.state] ?? 'neutral'}
                                                />
                                            }
                                            onPress={() => setShown(hold)}
                                        />
                                    ))}
                                </Section>
                            )}
                        </>
                    )
                }}
            </QueryView>

            <Section title="Shared file activity" card={false}>
                <QueryView
                    query={overlaps}
                    isEmpty={(list) => list.length === 0}
                    empty={
                        <EmptyState
                            icon="swap"
                            text="No two sessions are changing the same files right now."
                        />
                    }>
                    {(list) => (
                        <View style={styles.card}>
                            {list.map((overlap, index) => (
                                <View
                                    key={overlap.id}
                                    style={[styles.overlap, index > 0 && styles.divider]}>
                                    <View style={styles.overlapHead}>
                                        <Text style={styles.path} numberOfLines={2}>
                                            {overlap.path}
                                            {overlap.symbol ? ` · ${overlap.symbol}` : ''}
                                        </Text>
                                        <Chip label={overlap.kind} />
                                    </View>
                                    <Text style={styles.detail}>
                                        {overlap.sessions.join(' ↔ ')} · since{' '}
                                        {ago(overlap.first_seen)} ago
                                    </Text>
                                    {overlap.acked_by.length > 0 && (
                                        <Text style={styles.detail}>
                                            Seen by {overlap.acked_by.join(', ')}
                                        </Text>
                                    )}
                                    <View style={styles.actions}>
                                        <ThemedButton
                                            label="Tell them"
                                            iconName="mail"
                                            variant="tertiary"
                                            onPress={() => tell(overlap)}
                                        />
                                    </View>
                                </View>
                            ))}
                        </View>
                    )}
                </QueryView>
            </Section>

            <HoldSheet hold={shown} onDismiss={() => setShown(undefined)} />
        </Screen>
    )
}

export default GuardrailsScreen

/** A decided hold's facts, with the frozen request it held once it loads. */
const HoldSheet: React.FC<{ hold?: RelayHoldFull; onDismiss: () => void }> = ({
    hold,
    onDismiss,
}) => {
    const styles = useStyles()
    const detail = useBusQuery<{ hold: RelayHoldFull; request: HeldRequest }>(
        'guardrail.hold.get',
        { hold_id: hold?.id },
        { enabled: hold !== undefined, refetchOnFocus: false }
    )
    const loaded = detail.data && detail.data.hold.id === hold?.id ? detail.data : undefined
    return (
        <Sheet visible={hold !== undefined} onDismiss={onDismiss}>
            {hold && (
                <View style={styles.sheet}>
                    <View style={styles.overlapHead}>
                        <Text style={styles.sheetTitle}>Hold #{hold.id}</Text>
                        <Chip
                            label={STATE_LABEL[hold.state] ?? hold.state}
                            tone={STATE_TONE[hold.state] ?? 'neutral'}
                        />
                    </View>
                    <HoldDetails
                        hold={loaded?.hold ?? hold}
                        request={loaded?.request}
                        message={
                            hold.resolved_at
                                ? `${STATE_LABEL[hold.state] ?? hold.state} by ${hold.resolved_by ?? 'the engine'}, ${ago(hold.resolved_at)} ago.`
                                : undefined
                        }
                    />
                    <View style={styles.actions}>
                        <ThemedButton label="Close" variant="tertiary" onPress={onDismiss} />
                    </View>
                </View>
            )}
        </Sheet>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        card: {
            backgroundColor: color.neutral._200,
            borderRadius: 16,
            paddingHorizontal: spacing.l,
        },
        overlap: {
            rowGap: spacing.s,
            paddingVertical: spacing.l,
        },
        divider: {
            borderTopWidth: 1,
            borderTopColor: color.neutral._300,
        },
        overlapHead: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        path: {
            flex: 1,
            color: color.text._100,
            fontFamily: 'monospace',
        },
        detail: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
        sheet: {
            rowGap: spacing.l,
        },
        sheetTitle: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
    })
}
