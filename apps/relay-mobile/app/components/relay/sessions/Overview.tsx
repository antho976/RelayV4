import AntDesign from '@react-native-vector-icons/ant-design/static'
import React, { useState } from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { useBusQuery, useResourceSamples } from '@components/relay/hooks'
import { Theme } from '@lib/theme/ThemeManager'

import { megabytes } from './links'

type Resources = {
    panes: { session: string; cpu_pct: number; rss_mb: number }[]
    worktrees: { disk_mb?: number | null }[]
    total_rss_mb: number
}

type Dashboard = {
    projects: { tasks_open: number; done_recent: number }[]
    resources: Resources
}

const cpuOf = (resources?: Resources) =>
    (resources?.panes ?? []).reduce((sum, pane) => sum + (pane.cpu_pct || 0), 0)

/**
 * One slim row of figures from `dashboard.get` (the desktop dashboard's tiles, relay-native
 * tools.rs `dashboard`): the agents open on the wall, what got done this week (tap for the
 * board), and what the agents use (tap to follow live samples, which the PC sends only while
 * this screen is in front). What needs a decision is the attention row's job, not this one.
 */
export const StatsStrip: React.FC<{ open: number; running: number; onBoard: () => void }> = ({
    open,
    running,
    onBoard,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const [live, setLive] = useState(false)
    const [sample, setSample] = useState<Resources | undefined>(undefined)
    const query = useBusQuery<Dashboard>(
        'dashboard.get',
        {},
        { events: ['session.changed', 'task.changed'], debounceMs: 1000 }
    )
    useResourceSamples((payload) => setSample(payload as Resources), live)
    const data = query.data
    const done = (data?.projects ?? []).reduce((sum, item) => sum + (item.done_recent || 0), 0)
    const resources = (live && sample) || data?.resources
    return (
        <View style={styles.strip}>
            <Figure
                value={String(open)}
                caption={running > 0 ? `agents · ${running} running` : 'agents'}
                tint={running > 0 ? '#2ec469' : undefined}
            />
            <View style={styles.rule} />
            <Figure value={data ? String(done) : '–'} caption="done this week" onPress={onBoard} />
            <View style={styles.rule} />
            <Figure
                value={resources ? megabytes(resources.total_rss_mb ?? 0) : '–'}
                caption={`${cpuOf(resources).toFixed(0)}% CPU${live ? ' · live' : ''}`}
                tint={live ? color.primary._700 : undefined}
                onPress={() => {
                    setLive((on) => !on)
                    setSample(undefined)
                }}
            />
        </View>
    )
}

const Figure: React.FC<{
    value: string
    caption: string
    tint?: string
    onPress?: () => void
}> = ({ value, caption, tint, onPress }) => {
    const styles = useStyles()
    return (
        <TouchableOpacity style={styles.figure} disabled={!onPress} onPress={onPress}>
            <Text numberOfLines={1} style={[styles.value, !!tint && { color: tint }]}>
                {value}
            </Text>
            <Text numberOfLines={1} style={styles.caption}>
                {caption}
            </Text>
        </TouchableOpacity>
    )
}

type UsageWindow = { used_pct?: number; pct?: number; resets_in?: string | null }
type Usage = { provider: string; windows: Record<string, UsageWindow> | null; taken_at: string }

/**
 * Provider usage (`usage.get`, the desktop's usage window, shell.rs `usage`) folded into one
 * row: the fullest window as a mini bar. Tap to see a bar per window with when it resets.
 */
export const UsageRow: React.FC = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const [open, setOpen] = useState(false)
    const query = useBusQuery<Usage[]>(
        'usage.get',
        {},
        { events: ['usage.changed'], debounceMs: 1000, select: (raw) => raw.usage ?? [] }
    )
    const rows = (query.data ?? []).flatMap((item) =>
        Object.entries(item.windows ?? {})
            .map(([name, window]) => ({
                provider: item.provider,
                name: name.replace(/_/g, ' '),
                pct: Math.min(100, Math.max(0, Number(window?.used_pct ?? window?.pct ?? NaN))),
                resets: window?.resets_in ?? undefined,
            }))
            .filter((row) => Number.isFinite(row.pct))
    )
    const top = rows.reduce<(typeof rows)[number] | undefined>(
        (best, row) => (!best || row.pct > best.pct ? row : best),
        undefined
    )
    const tone = (pct: number) =>
        pct >= 90 ? color.error._300 : pct >= 70 ? color.quote : color.primary._500

    return (
        <View>
            <TouchableOpacity
                style={styles.usageHead}
                disabled={rows.length === 0}
                onPress={() => setOpen((on) => !on)}>
                <Text style={styles.usageLabel}>Usage</Text>
                {top ? (
                    <>
                        <View style={[styles.track, styles.mini]}>
                            <View
                                style={[
                                    styles.bar,
                                    { width: `${top.pct}%`, backgroundColor: tone(top.pct) },
                                ]}
                            />
                        </View>
                        <Text numberOfLines={1} style={styles.usageTop}>
                            {top.provider} {top.name} · {top.pct.toFixed(0)}%
                        </Text>
                        <AntDesign name={open ? 'up' : 'down'} size={12} color={color.text._500} />
                    </>
                ) : (
                    <Text style={styles.usageTop}>{query.data ? 'Nothing reported yet' : '…'}</Text>
                )}
            </TouchableOpacity>
            {open &&
                rows.map((row) => (
                    <View key={`${row.provider}-${row.name}`} style={styles.usageRow}>
                        <View style={styles.usageLine}>
                            <Text style={styles.usageName}>
                                {row.provider} · {row.name}
                            </Text>
                            <Text style={styles.caption}>
                                {row.pct.toFixed(0)}%
                                {row.resets ? ` · resets in ${row.resets}` : ''}
                            </Text>
                        </View>
                        <View style={styles.track}>
                            <View
                                style={[
                                    styles.bar,
                                    { width: `${row.pct}%`, backgroundColor: tone(row.pct) },
                                ]}
                            />
                        </View>
                    </View>
                ))}
        </View>
    )
}

/** Where a stopped agent can come back from: one row of `session.restorable`. */
export type Restorable = {
    session: { name: string; provider: string; role: string; branch: string; worktree: string }
    reason: string
    worktree_dirty: boolean
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        strip: {
            flexDirection: 'row',
            alignItems: 'center',
        },
        figure: {
            flex: 1,
            alignItems: 'center',
            paddingVertical: 2,
        },
        rule: {
            width: StyleSheet.hairlineWidth,
            alignSelf: 'stretch',
            backgroundColor: color.neutral._400,
        },
        value: {
            color: color.text._100,
            fontSize: fontSize.l,
            fontWeight: '600',
            fontVariant: ['tabular-nums'],
        },
        caption: {
            color: color.text._500,
            fontSize: fontSize.s - 1,
        },
        usageHead: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        usageLabel: {
            color: color.text._300,
            fontSize: fontSize.s,
        },
        usageTop: {
            flex: 1,
            color: color.text._500,
            fontSize: fontSize.s - 1,
            textTransform: 'capitalize',
        },
        mini: {
            width: 56,
        },
        usageRow: {
            rowGap: 4,
            marginTop: spacing.m,
        },
        usageLine: {
            flexDirection: 'row',
            justifyContent: 'space-between',
            columnGap: spacing.m,
        },
        usageName: {
            flexShrink: 1,
            color: color.text._200,
            fontSize: fontSize.s,
            textTransform: 'capitalize',
        },
        track: {
            height: 4,
            borderRadius: 2,
            overflow: 'hidden',
            backgroundColor: color.neutral._400,
        },
        bar: {
            height: 4,
            borderRadius: 2,
        },
    })
}
