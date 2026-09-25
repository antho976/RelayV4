import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useRouter } from 'expo-router'
import React, { useRef, useState } from 'react'
import { ActivityIndicator, StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { useBusQuery } from '@components/relay/hooks'
import { MenuItem, MenuSheet } from '@components/relay/settings/common'
import { confirm } from '@components/relay/Sheet'
import { isCancelled, relay, RelaySession, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { sessionHref, terminalHref } from './links'
import { Restorable } from './Overview'

/** A session that is not running but can be brought back: restorable or exited. */
export const isStopped = (state: string) => state === 'restorable' || state === 'exited'

type Stopped = {
    name: string
    provider: string
    role: string
    projectId?: number
    /** Offered by `session.restorable`: Discard declines it, Clear context starts it fresh. */
    restorable: boolean
    dirty: boolean
}

/**
 * Every stopped agent, once: the wall's restorable and exited sessions merged with what
 * `session.restorable` offers after a restart, keyed by name. Folded into a single row —
 * "12 stopped agents · Resume all" — that opens to one line per agent with a resume button;
 * a long press offers details, a fresh start and discard.
 *
 * `inScope` narrows it to what the PC tab shows; an offer the wall does not know yet (so its
 * project is unknown) is shown only when the tab is not narrowed.
 */
export const StoppedAgents: React.FC<{
    sessions: RelaySession[]
    inScope: (projectId: number | undefined) => boolean
}> = ({ sessions, inScope }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const projects = useRelayStore((state) => state.projects)
    const [open, setOpen] = useState(false)
    const [busy, setBusy] = useState<string | undefined>(undefined)
    const [progress, setProgress] = useState<{ done: number; total: number } | undefined>()
    const [menu, setMenu] = useState<Stopped | undefined>(undefined)
    const stop = useRef(false)
    const query = useBusQuery<Restorable[]>(
        'session.restorable',
        {},
        { events: ['session.changed'], debounceMs: 800, select: (raw) => raw.sessions ?? [] }
    )

    const offers = new Map((query.data ?? []).map((entry) => [entry.session.name, entry]))
    const known = new Map(sessions.map((session) => [session.name, session]))
    const list: Stopped[] = []
    for (const session of sessions) {
        if (!isStopped(session.state) || !inScope(session.project_id)) continue
        const offer = offers.get(session.name)
        list.push({
            name: session.name,
            provider: session.provider,
            role: session.role,
            projectId: session.project_id,
            restorable: !!offer || session.state === 'restorable',
            dirty: !!offer?.worktree_dirty,
        })
    }
    for (const [name, offer] of offers) {
        // A session the wall knows is listed above, or is live and the offer is stale.
        if (known.has(name) || !inScope(undefined)) continue
        list.push({
            name: name,
            provider: offer.session.provider,
            role: offer.session.role,
            restorable: true,
            dirty: offer.worktree_dirty,
        })
    }
    if (list.length === 0) return null

    const projectName = (id?: number) => projects.find((item) => item.id === id)?.name
    const after = () => {
        relay.refresh().catch(() => {})
        query.reload()
    }
    const report = (e: unknown) => {
        if (!isCancelled(e)) Logger.errorToast(`${(e as Error).message}`)
    }

    const resume = async (item: Stopped) => {
        if (busy || progress) return
        setBusy(item.name)
        try {
            await relay.guarded('session.resume', { session: item.name })
            router.push(terminalHref(item.name))
        } catch (e) {
            report(e)
        } finally {
            setBusy(undefined)
            after()
        }
    }

    const resumeAll = async () => {
        if (busy || progress) return
        const yes = await confirm({
            title: `Resume ${list.length === 1 ? 'one agent' : `all ${list.length} agents`}?`,
            message:
                'Each agent starts its CLI again and picks up its conversation, which uses provider quota. They are resumed one after another; tap the row to stop after the current one.',
            confirmLabel: 'Resume all',
        })
        if (!yes) return
        const names = list.map((item) => item.name)
        stop.current = false
        let resumed = 0
        for (const [index, name] of names.entries()) {
            if (stop.current) break
            setBusy(name)
            setProgress({ done: index, total: names.length })
            try {
                await relay.guarded('session.resume', { session: name })
                resumed++
            } catch (e) {
                // A Deny means the person does not want this run; any other failure is
                // reported and the next agent is tried.
                if (isCancelled(e)) break
                report(e)
            }
        }
        setBusy(undefined)
        setProgress(undefined)
        Logger.infoToast(`Resumed ${resumed} of ${names.length}`)
        after()
    }

    const discard = async (item: Stopped) => {
        const yes = await confirm({
            title: `Discard ${item.name}?`,
            message:
                (item.dirty ? 'Its worktree has uncommitted changes. ' : '') +
                'The session is cleaned up and taken off the PC; its branch is kept.',
            confirmLabel: 'Discard',
            destructive: true,
        })
        if (!yes) return
        setBusy(item.name)
        try {
            if (item.restorable)
                await relay.guarded('session.discard_restorable', { session: item.name })
            else await relay.guarded('session.close', { session: item.name })
        } catch (e) {
            report(e)
        } finally {
            setBusy(undefined)
            after()
        }
    }

    const clear = async (item: Stopped) => {
        const yes = await confirm({
            title: 'Start fresh?',
            message:
                'Start again in this same session and worktree. The saved provider conversation is cleared.',
            confirmLabel: 'Clear and start',
            destructive: true,
        })
        if (!yes) return
        setBusy(item.name)
        try {
            await relay.guarded('session.clear_restorable', { session: item.name })
            router.push(terminalHref(item.name))
        } catch (e) {
            report(e)
        } finally {
            setBusy(undefined)
            after()
        }
    }

    const menuItems = (item: Stopped): MenuItem[] => [
        { label: 'Resume', icon: 'caret-right', onPress: () => resume(item) },
        {
            label: 'Details',
            icon: 'info-circle',
            onPress: () => router.push(sessionHref(item.name)),
        },
        ...(item.restorable
            ? [{ label: 'Start fresh', icon: 'reload' as const, onPress: () => clear(item) }]
            : []),
        { label: 'Discard…', icon: 'delete', destructive: true, onPress: () => discard(item) },
    ]

    return (
        <View style={styles.card}>
            <TouchableOpacity
                style={styles.head}
                onPress={() => (progress ? (stop.current = true) : setOpen((on) => !on))}>
                <AntDesign name="pause-circle" size={16} color={color.text._400} />
                <Text numberOfLines={1} style={styles.title}>
                    {progress
                        ? `Resuming ${Math.min(progress.done + 1, progress.total)} of ${progress.total}…`
                        : `${list.length} stopped ${list.length === 1 ? 'agent' : 'agents'}`}
                </Text>
                {progress ? (
                    <ActivityIndicator size="small" color={color.text._400} />
                ) : (
                    <>
                        <TouchableOpacity hitSlop={8} disabled={!!busy} onPress={resumeAll}>
                            <Text style={[styles.action, !!busy && styles.muted]}>
                                {list.length === 1 ? 'Resume' : 'Resume all'}
                            </Text>
                        </TouchableOpacity>
                        <AntDesign name={open ? 'up' : 'down'} size={12} color={color.text._500} />
                    </>
                )}
            </TouchableOpacity>
            {open &&
                list.map((item) => (
                    <TouchableOpacity
                        key={item.name}
                        style={styles.row}
                        onPress={() => router.push(sessionHref(item.name))}
                        onLongPress={() => setMenu(item)}>
                        <View style={styles.dot} />
                        <Text numberOfLines={1} style={styles.line}>
                            <Text style={styles.name}>{item.name}</Text>
                            {[projectName(item.projectId), item.provider]
                                .filter(Boolean)
                                .map((part) => ` · ${part}`)
                                .join('')}
                            {item.dirty && <Text style={{ color: color.quote }}> · changes</Text>}
                        </Text>
                        <TouchableOpacity
                            hitSlop={6}
                            style={styles.resume}
                            disabled={!!busy || !!progress}
                            onPress={() => resume(item)}>
                            {busy === item.name ? (
                                <ActivityIndicator size="small" color={color.text._300} />
                            ) : (
                                <AntDesign name="caret-right" size={14} color={color.text._200} />
                            )}
                        </TouchableOpacity>
                    </TouchableOpacity>
                ))}
            {open && <Text style={styles.hint}>Long-press an agent to discard it.</Text>}
            <MenuSheet
                visible={!!menu}
                title={menu?.name}
                detail={menu ? `${menu.provider} · ${menu.role}` : undefined}
                items={menu ? menuItems(menu) : []}
                onDismiss={() => setMenu(undefined)}
            />
        </View>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        card: {
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            borderRadius: 14,
            backgroundColor: color.neutral._200,
        },
        head: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            minHeight: 32,
        },
        title: {
            flex: 1,
            color: color.text._200,
            fontSize: fontSize.m,
        },
        action: {
            color: color.primary._700,
            fontSize: fontSize.s,
            fontWeight: '600',
        },
        muted: {
            color: color.text._600,
        },
        row: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingVertical: spacing.s,
        },
        dot: {
            width: 6,
            height: 6,
            borderRadius: 3,
            marginLeft: 5,
            marginRight: 5,
            backgroundColor: color.neutral._700,
        },
        line: {
            flex: 1,
            color: color.text._500,
            fontSize: fontSize.s,
        },
        name: {
            color: color.text._200,
            fontSize: fontSize.m,
        },
        resume: {
            width: 30,
            height: 30,
            borderRadius: 15,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.neutral._300,
        },
        hint: {
            color: color.text._600,
            fontSize: fontSize.s - 1,
            paddingTop: spacing.s,
        },
    })
}
