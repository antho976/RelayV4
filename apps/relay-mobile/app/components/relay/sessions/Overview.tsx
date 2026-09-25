import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import { useBusQuery, useResourceSamples } from '@components/relay/hooks'
import { confirm } from '@components/relay/Sheet'
import { isCancelled, relay } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { megabytes, sessionHref, terminalHref } from './links'

type Resources = {
    panes: { session: string; cpu_pct: number; rss_mb: number }[]
    worktrees: { disk_mb?: number | null }[]
    total_rss_mb: number
}

type Dashboard = {
    projects: { tasks_open: number; done_recent: number }[]
    sessions_live: { state: string }[]
    in_review: unknown[]
    holds_open: unknown[]
    resources: Resources
}

const cpuOf = (resources?: Resources) =>
    (resources?.panes ?? []).reduce((sum, pane) => sum + (pane.cpu_pct || 0), 0)

/**
 * The desktop dashboard's four tiles (relay-native tools.rs `dashboard`) from one
 * `dashboard.get`: what needs a decision, how many agents are open, what got done this week,
 * and what the agents use. Needs you opens the Inbox, Done this week the board. Tapping the
 * memory tile turns on live samples, which the PC sends only while this screen is in front.
 */
export const DashboardTiles: React.FC<{ onInbox: () => void; onBoard: () => void }> = ({
    onInbox,
    onBoard,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const [live, setLive] = useState(false)
    const [sample, setSample] = useState<Resources | undefined>(undefined)
    const query = useBusQuery<Dashboard>(
        'dashboard.get',
        {},
        {
            events: ['session.changed', 'guardrail.*', 'task.changed', 'notify.*'],
            debounceMs: 1000,
        }
    )
    useResourceSamples((payload) => setSample(payload as Resources), live)
    const data = query.data
    if (!data) return null
    const peers = data.sessions_live ?? []
    const blocked = peers.filter((peer) => peer.state === 'blocked').length
    const holds = data.holds_open?.length ?? 0
    const reviews = data.in_review?.length ?? 0
    const attention = holds + reviews + blocked
    const running = peers.filter((peer) => peer.state === 'running').length
    const idle = peers.filter((peer) => peer.state === 'idle').length
    const done = data.projects.reduce((sum, project) => sum + (project.done_recent || 0), 0)
    const open = data.projects.reduce((sum, project) => sum + (project.tasks_open || 0), 0)
    const resources = (live && sample) || data.resources
    const disk = (data.resources?.worktrees ?? []).reduce(
        (sum, worktree) => sum + (worktree.disk_mb ?? 0),
        0
    )
    return (
        <View style={styles.tiles}>
            <Tile
                icon="bell"
                value={String(attention)}
                caption="Needs you"
                detail={`${holds} holds · ${blocked} blocked · ${reviews} review`}
                tint={attention > 0 ? color.error._300 : undefined}
                onPress={onInbox}
            />
            <Tile
                icon="code"
                value={String(peers.length)}
                caption="Open agents"
                detail={`${running} running · ${idle} idle`}
                tint={running > 0 ? '#2ec469' : undefined}
            />
            <Tile
                icon="check"
                value={String(done)}
                caption="Done this week"
                detail={`${open} tasks still open`}
                onPress={onBoard}
            />
            <Tile
                icon="dashboard"
                value={megabytes(resources?.total_rss_mb ?? 0)}
                caption={live ? 'Agent memory · live' : 'Agent memory'}
                detail={`${cpuOf(resources).toFixed(1)}% CPU · ${megabytes(disk)} worktrees`}
                tint={live ? color.primary._700 : undefined}
                onPress={() => {
                    setLive((on) => !on)
                    setSample(undefined)
                }}
            />
        </View>
    )
}

const Tile: React.FC<{
    icon: AntDesignIconName
    value: string
    caption: string
    detail: string
    tint?: string
    onPress?: () => void
}> = ({ icon, value, caption, detail, tint, onPress }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <TouchableOpacity style={styles.tile} disabled={!onPress} onPress={onPress}>
            <View style={styles.tileHead}>
                <AntDesign name={icon} size={14} color={tint ?? color.text._500} />
                <Text numberOfLines={1} style={styles.caption}>
                    {caption}
                </Text>
            </View>
            <Text numberOfLines={1} style={[styles.value, { color: tint ?? color.text._200 }]}>
                {value}
            </Text>
            <Text numberOfLines={2} style={styles.detail}>
                {detail}
            </Text>
        </TouchableOpacity>
    )
}

type UsageWindow = { used_pct?: number; pct?: number; resets_in?: string | null }
type Usage = { provider: string; windows: Record<string, UsageWindow> | null; taken_at: string }

/**
 * Provider usage, as the desktop's usage window shows it (shell.rs `usage`): one bar per
 * window each provider reports, with when it resets. Reloads on `usage.changed`.
 */
export const UsageCard: React.FC = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const query = useBusQuery<Usage[]>(
        'usage.get',
        {},
        {
            events: ['usage.changed'],
            debounceMs: 1000,
            select: (raw) => raw.usage ?? [],
        }
    )
    const usage = query.data
    if (!usage) return null
    const rows = usage.flatMap((item) =>
        Object.entries(item.windows ?? {})
            .map(([name, window]) => ({
                provider: item.provider,
                name: name.replace(/_/g, ' '),
                pct: Math.min(100, Math.max(0, Number(window?.used_pct ?? window?.pct ?? NaN))),
                resets: window?.resets_in ?? undefined,
            }))
            .filter((row) => Number.isFinite(row.pct))
    )
    return (
        <View style={styles.usage}>
            <Text style={styles.heading}>Usage</Text>
            {rows.length === 0 ? (
                <Text style={styles.detail}>No provider usage has been reported yet.</Text>
            ) : (
                rows.map((row) => (
                    <View key={`${row.provider}-${row.name}`} style={styles.usageRow}>
                        <View style={styles.usageLine}>
                            <Text style={styles.usageName}>
                                {row.provider} · {row.name}
                            </Text>
                            <Text style={styles.detail}>
                                {row.pct.toFixed(0)}%
                                {row.resets ? ` · resets in ${row.resets}` : ''}
                            </Text>
                        </View>
                        <View style={styles.track}>
                            <View
                                style={[
                                    styles.bar,
                                    {
                                        width: `${row.pct}%`,
                                        backgroundColor:
                                            row.pct >= 90
                                                ? color.error._300
                                                : row.pct >= 70
                                                  ? color.quote
                                                  : color.primary._500,
                                    },
                                ]}
                            />
                        </View>
                    </View>
                ))
            )}
        </View>
    )
}

type Restorable = {
    session: { name: string; provider: string; role: string; branch: string; worktree: string }
    reason: string
    worktree_dirty: boolean
}

/**
 * Sessions the PC offers to resume after a restart (`session.restorable`): Resume brings the
 * agent back with its provider context, Discard declines and cleans the session up.
 */
export const RestorableBanner: React.FC = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const [busy, setBusy] = useState<string | undefined>(undefined)
    const query = useBusQuery<Restorable[]>(
        'session.restorable',
        {},
        {
            events: ['session.changed'],
            debounceMs: 800,
            select: (raw) => raw.sessions ?? [],
        }
    )
    const entries = query.data ?? []
    if (entries.length === 0) return null

    const act = async (name: string, op: string) => {
        setBusy(name)
        try {
            await relay.guarded(op, { session: name })
            relay.refresh().catch(() => {})
            query.setData((rows) => rows?.filter((row) => row.session.name !== name))
            if (op === 'session.resume') router.push(terminalHref(name))
        } catch (e) {
            if (!isCancelled(e)) Logger.errorToast(`${(e as Error).message}`)
        } finally {
            setBusy(undefined)
            query.reload()
        }
    }

    const discard = async (entry: Restorable) => {
        const yes = await confirm({
            title: `Discard ${entry.session.name}?`,
            message: entry.worktree_dirty
                ? 'Its worktree has uncommitted changes. Discarding cleans the session up and removes a pooled worktree; the branch is kept.'
                : 'Discarding cleans the session up and removes a pooled worktree; the branch is kept.',
            confirmLabel: 'Discard',
            destructive: true,
        })
        if (yes) await act(entry.session.name, 'session.discard_restorable')
    }

    return (
        <View style={[styles.banner, { borderColor: color.quote }]}>
            <View style={styles.tileHead}>
                <AntDesign name="reload" size={16} color={color.quote} />
                <Text style={styles.bannerTitle}>Resume after restart</Text>
            </View>
            <Text style={styles.detail}>
                {entries.length === 1
                    ? 'One agent was running when the PC stopped.'
                    : `${entries.length} agents were running when the PC stopped.`}
            </Text>
            {entries.map((entry) => (
                <View key={entry.session.name} style={styles.restorable}>
                    <TouchableOpacity
                        style={styles.restorableText}
                        onPress={() => router.push(sessionHref(entry.session.name))}>
                        <Text numberOfLines={1} style={styles.bannerTitle}>
                            {entry.session.name}
                        </Text>
                        <Text numberOfLines={1} style={styles.detail}>
                            {entry.session.provider} · {entry.session.role} ·{' '}
                            {entry.reason.replace(/_/g, ' ')}
                            {entry.worktree_dirty ? ' · uncommitted changes' : ''}
                        </Text>
                    </TouchableOpacity>
                    <ThemedButton
                        label="Discard"
                        variant={busy ? 'disabled' : 'tertiary'}
                        onPress={() => discard(entry)}
                    />
                    <ThemedButton
                        label={busy === entry.session.name ? '…' : 'Resume'}
                        variant={busy ? 'disabled' : 'secondary'}
                        onPress={() => act(entry.session.name, 'session.resume')}
                    />
                </View>
            ))}
        </View>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        tiles: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            gap: spacing.m,
        },
        tile: {
            flexGrow: 1,
            flexBasis: '45%',
            padding: spacing.l,
            borderRadius: 16,
            backgroundColor: color.neutral._200,
            rowGap: 2,
        },
        tileHead: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.s,
        },
        caption: {
            flex: 1,
            color: color.text._400,
            fontSize: fontSize.s - 1,
            letterSpacing: 0.8,
            textTransform: 'uppercase',
        },
        value: {
            fontSize: fontSize.xl2,
            fontWeight: '700',
            fontVariant: ['tabular-nums'],
        },
        detail: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        heading: {
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
        },
        usage: {
            rowGap: spacing.m,
            padding: spacing.l,
            borderRadius: 16,
            backgroundColor: color.neutral._200,
        },
        usageRow: {
            rowGap: 4,
        },
        usageLine: {
            flexDirection: 'row',
            justifyContent: 'space-between',
            columnGap: spacing.m,
        },
        usageName: {
            flexShrink: 1,
            color: color.text._200,
            fontSize: fontSize.m,
            textTransform: 'capitalize',
        },
        track: {
            height: 6,
            borderRadius: 3,
            overflow: 'hidden',
            backgroundColor: color.neutral._400,
        },
        bar: {
            height: 6,
            borderRadius: 3,
        },
        banner: {
            rowGap: spacing.m,
            padding: spacing.l,
            borderRadius: 16,
            borderWidth: 1,
            backgroundColor: color.neutral._200,
        },
        bannerTitle: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.m,
            fontWeight: '600',
        },
        restorable: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.s,
        },
        restorableText: {
            flex: 1,
            rowGap: 2,
        },
    })
}
