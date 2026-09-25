import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useFocusEffect, useRouter } from 'expo-router'
import React, { useCallback, useState } from 'react'
import { RefreshControl, ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import Drawer from '@components/views/Drawer'
import HeaderButton from '@components/views/HeaderButton'
import HeaderTitle from '@components/views/HeaderTitle'
import { relay, RelayProject, RelaySession, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { activeHost, isTailnetUrl, useRelayHostsStore } from '@lib/state/RelayHosts'
import { useRelayView } from '@lib/state/RelayView'
import { Theme } from '@lib/theme/ThemeManager'

import { linkColor, palette } from './console'
import RequestSheet from './RequestSheet'
import SessionItem from './SessionItem'
import WorkspaceDrawer, { groupProjects } from './WorkspaceDrawer'

/**
 * The PC tab: the agent wall on the desktop, from a phone. A card says whether the PC is
 * there and how it is reached; a row of counts says what the agents are doing; the sessions
 * follow, grouped by project; and at the bottom, the one thing a person came here to do —
 * hand an agent some work.
 *
 * The header carries the rest: the sidebar (every workspace and project on the PC, and the
 * way to narrow this page to one), and the bell (holds, reviews and notifications).
 */
const RelayScreen = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const paired = useRelayHostsStore((state) => state.hosts.length > 0)
    const status = useRelayStore((state) => state.status)
    const transport = useRelayStore((state) => state.transport)
    const routeUrl = useRelayStore((state) => state.routeUrl)
    const hostName = useRelayStore((state) => state.hostName)
    const version = useRelayStore((state) => state.version)
    const error = useRelayStore((state) => state.error)
    const sessions = useRelayStore((state) => state.sessions)
    const projects = useRelayStore((state) => state.projects)
    const workspaces = useRelayStore((state) => state.workspaces)
    const holds = useRelayStore((state) => state.holds)
    const notifications = useRelayStore((state) => state.notifications)
    const inReview = useRelayStore((state) => state.inReview)
    const { scope, setScope } = useRelayView()
    const setDrawer = Drawer.useDrawerStore((state) => state.setShow)
    const [showRequest, setShowRequest] = useState(false)
    const [refreshing, setRefreshing] = useState(false)

    useFocusEffect(
        useCallback(() => {
            // Read the stores directly: this runs on focus, not on every status change.
            const current = useRelayStore.getState()
            const known = useRelayHostsStore.getState().hosts.length > 0
            if (current.status === 'online') relay.refresh().catch(() => {})
            else if (current.status === 'offline' && known && !current.stopped) {
                // Reach the paired PC on arrival, and again after a drop. A Disconnect the
                // person asked for is respected until they tap Connect.
                relay.connect(activeHost()).catch(() => {})
            }
        }, [])
    )

    // The sidebar is closed whenever the tab is left, so it does not greet the next visit.
    useFocusEffect(useCallback(() => () => setDrawer(Drawer.ID.RELAY, false), [setDrawer]))

    const connect = async () => {
        try {
            if (status === 'online') await relay.refresh()
            else if (paired) await relay.connect(activeHost())
        } catch (e) {
            Logger.warnToast(`${(e as Error).message}`)
        }
    }

    const handleRefresh = async () => {
        setRefreshing(true)
        await connect()
        setRefreshing(false)
    }

    const openHosts = () => router.push('/screens/RelayScreen/Hosts')
    const openInbox = () => router.push('/screens/RelayScreen/Inbox')
    const openBoard = (project?: RelayProject) =>
        router.push({
            pathname: '/screens/RelayScreen/Board',
            params: project ? { project: String(project.id) } : {},
        })
    // A project's hub: every surface the desktop has for it.
    const openProject = (project: RelayProject) =>
        router.push({
            pathname: '/screens/RelayScreen/Project',
            params: { project_id: String(project.id) },
        })

    const online = status === 'online'
    const live = sessions.filter((item) => item.state !== 'closed')
    const badge = holds.length + notifications.length + inReview

    // The scope the sidebar picked, if what it points at still exists on this PC.
    const scopeProject =
        scope.kind === 'project' ? projects.find((p) => p.id === scope.id) : undefined
    const scopeWorkspace =
        scope.kind === 'workspace' ? workspaces.find((w) => w.id === scope.id) : undefined
    const scopeLabel = scopeProject?.name ?? scopeWorkspace?.name
    const inScope = (session: RelaySession) => {
        if (scopeProject) return session.project_id === scopeProject.id
        if (scopeWorkspace) {
            const project = projects.find((p) => p.id === session.project_id)
            return project?.workspace_id === scopeWorkspace.id
        }
        return true
    }
    const shown = live.filter(inScope)
    const running = shown.filter((item) => item.state === 'running').length
    const blocked = shown.filter((item) => item.state === 'blocked').length

    // Sessions under their project, projects in the sidebar's order; one with none is left out.
    const sections = groupProjects(workspaces, projects)
        .flatMap((group) => group.projects)
        .map((project) => ({
            project,
            sessions: shown.filter((item) => item.project_id === project.id),
        }))
        .filter((section) => section.sessions.length > 0)
    const orphans = shown.filter((item) => !projects.some((p) => p.id === item.project_id))

    const routeLabel =
        transport === 'via'
            ? 'Through your server'
            : routeUrl && isTailnetUrl(routeUrl)
              ? 'Tailscale · private'
              : 'Same network · private'

    const headerRight = () =>
        paired ? (
            <View style={styles.headerActions}>
                <TouchableOpacity hitSlop={10} onPress={openInbox} disabled={!online}>
                    <AntDesign
                        name="bell"
                        size={22}
                        color={online ? color.text._200 : color.text._600}
                    />
                    {online && badge > 0 && (
                        <View
                            style={[
                                styles.badge,
                                {
                                    backgroundColor:
                                        holds.length > 0 ? color.error._400 : color.primary._500,
                                },
                            ]}>
                            <Text style={styles.badgeText}>{badge > 99 ? '99+' : badge}</Text>
                        </View>
                    )}
                </TouchableOpacity>
                <TouchableOpacity hitSlop={10} onPress={() => setDrawer(Drawer.ID.RELAY, true)}>
                    <AntDesign name="menu" size={22} color={color.text._200} />
                </TouchableOpacity>
            </View>
        ) : null

    if (!paired) {
        return (
            <SafeAreaView edges={['bottom']} style={styles.fill}>
                <HeaderTitle title="PC" />
                <HeaderButton headerRight={headerRight} />
                <View style={styles.empty}>
                    <View style={styles.emptyIcon}>
                        <AntDesign name="desktop" size={40} color={color.text._300} />
                    </View>
                    <Text style={styles.emptyTitle}>Your desktop, from here</Text>
                    <Text style={styles.emptyText}>
                        Pair this phone with the PC that runs Relay. On the same WiFi the link is
                        direct and never leaves your network. Away from home, use Tailscale on both
                        devices, or a server you host.
                    </Text>
                    <ThemedButton label="Pair a PC" iconName="qrcode" onPress={openHosts} />
                </View>
            </SafeAreaView>
        )
    }

    return (
        <Drawer.Gesture
            config={[
                { drawerID: Drawer.ID.RELAY, openDirection: 'right', closeDirection: 'left' },
            ]}>
            <SafeAreaView edges={['bottom']} style={styles.fill}>
                <HeaderTitle title="PC" />
                <HeaderButton headerRight={headerRight} />
                <RequestSheet
                    visible={showRequest}
                    setVisible={setShowRequest}
                    projectId={scopeProject?.id}
                />
                <ScrollView
                    contentContainerStyle={styles.page}
                    refreshControl={
                        <RefreshControl
                            refreshing={refreshing}
                            onRefresh={handleRefresh}
                            tintColor={color.text._300}
                            colors={[color.text._300]}
                        />
                    }>
                    <TouchableOpacity style={styles.hostCard} onPress={openHosts}>
                        <View style={styles.hostIcon}>
                            <AntDesign name="desktop" size={20} color={color.text._200} />
                            <View
                                style={[
                                    styles.hostLamp,
                                    {
                                        backgroundColor: linkColor(status, color),
                                        borderColor: color.neutral._200,
                                    },
                                ]}
                            />
                        </View>
                        <View style={{ flex: 1 }}>
                            <Text numberOfLines={1} style={styles.hostName}>
                                {hostName ?? activeHost()?.name ?? 'PC'}
                            </Text>
                            <Text
                                numberOfLines={2}
                                style={[styles.hostMeta, !online && !!error && styles.error]}>
                                {online
                                    ? `${routeLabel}${version ? ` · v${version}` : ''}`
                                    : status === 'connecting'
                                      ? 'Connecting…'
                                      : (error ?? 'Not connected')}
                            </Text>
                        </View>
                        {status === 'offline' ? (
                            <ThemedButton label="Connect" variant="secondary" onPress={connect} />
                        ) : (
                            <AntDesign name="right" size={14} color={color.text._500} />
                        )}
                    </TouchableOpacity>

                    {online && holds.length > 0 && (
                        <TouchableOpacity
                            style={[styles.alert, { borderColor: color.error._400 }]}
                            onPress={openInbox}>
                            <AntDesign name="safety" size={20} color={color.error._300} />
                            <Text style={styles.alertText}>
                                {holds.length === 1
                                    ? `${holds[0].session ?? 'An agent'} is waiting for permission`
                                    : `${holds.length} agents are waiting for permission`}
                            </Text>
                            <Text style={[styles.alertAction, { color: color.error._300 }]}>
                                Review
                            </Text>
                        </TouchableOpacity>
                    )}

                    {online && (
                        <View style={styles.stats}>
                            <Stat
                                value={running}
                                label="Running"
                                tint={running > 0 ? palette.live : undefined}
                            />
                            <Stat
                                value={blocked}
                                label="Blocked"
                                tint={blocked > 0 ? color.error._300 : undefined}
                            />
                            <Stat
                                value={inReview}
                                label="In review"
                                tint={inReview > 0 ? color.primary._700 : undefined}
                                onPress={() => openBoard(scopeProject)}
                            />
                        </View>
                    )}

                    {online && (
                        <View style={styles.section}>
                            <View style={styles.headingRow}>
                                <Text style={styles.heading}>
                                    {scopeLabel ? scopeLabel : 'All sessions'} · {shown.length}
                                </Text>
                                {scopeProject && (
                                    <TouchableOpacity
                                        hitSlop={8}
                                        style={styles.scopeClear}
                                        onPress={() => openProject(scopeProject)}>
                                        <Text style={styles.link}>Open project</Text>
                                    </TouchableOpacity>
                                )}
                                {scopeLabel ? (
                                    <TouchableOpacity
                                        hitSlop={8}
                                        style={styles.scopeClear}
                                        onPress={() => setScope({ kind: 'all' })}>
                                        <Text style={styles.link}>Show all</Text>
                                    </TouchableOpacity>
                                ) : (
                                    <TouchableOpacity
                                        hitSlop={8}
                                        onPress={() => setDrawer(Drawer.ID.RELAY, true)}>
                                        <Text style={styles.link}>Workspaces</Text>
                                    </TouchableOpacity>
                                )}
                            </View>
                            {shown.length === 0 ? (
                                <View style={styles.none}>
                                    <AntDesign name="code" size={28} color={color.text._600} />
                                    <Text style={styles.note}>
                                        {scopeLabel
                                            ? `Nothing running in ${scopeLabel}.`
                                            : 'Nothing running.'}{' '}
                                        Ask for something below, or launch agents from the desktop;
                                        they appear here as they start.
                                    </Text>
                                </View>
                            ) : (
                                <View style={styles.groups}>
                                    {sections.map(({ project, sessions: items }) => (
                                        <View key={project.id} style={styles.group}>
                                            {!scopeProject && (
                                                <View style={styles.groupHead}>
                                                    <TouchableOpacity
                                                        style={styles.groupOpen}
                                                        onPress={() => openProject(project)}>
                                                        <AntDesign
                                                            name="folder"
                                                            size={14}
                                                            color={color.text._500}
                                                        />
                                                        <Text
                                                            numberOfLines={1}
                                                            style={styles.groupName}>
                                                            {project.name}
                                                        </Text>
                                                        <AntDesign
                                                            name="right"
                                                            size={12}
                                                            color={color.text._500}
                                                        />
                                                    </TouchableOpacity>
                                                    <TouchableOpacity
                                                        hitSlop={8}
                                                        onPress={() => openBoard(project)}>
                                                        <Text style={styles.link}>Board</Text>
                                                    </TouchableOpacity>
                                                </View>
                                            )}
                                            {items.map((session) => (
                                                <SessionItem key={session.name} session={session} />
                                            ))}
                                        </View>
                                    ))}
                                    {orphans.length > 0 && (
                                        <View style={styles.group}>
                                            {orphans.map((session) => (
                                                <SessionItem key={session.name} session={session} />
                                            ))}
                                        </View>
                                    )}
                                </View>
                            )}
                        </View>
                    )}
                </ScrollView>

                {online && (
                    <TouchableOpacity style={styles.composer} onPress={() => setShowRequest(true)}>
                        <AntDesign name="thunderbolt" size={16} color={color.text._500} />
                        <Text numberOfLines={1} style={styles.composerText}>
                            {scopeProject
                                ? `Ask an agent in ${scopeProject.name}…`
                                : 'Ask an agent on the PC…'}
                        </Text>
                        <View style={styles.composerSend}>
                            <AntDesign name="arrow-up" size={16} color={color.text._900} />
                        </View>
                    </TouchableOpacity>
                )}
                <WorkspaceDrawer onOpenInbox={openInbox} onOpenHosts={openHosts} />
            </SafeAreaView>
        </Drawer.Gesture>
    )
}

export default RelayScreen

const Stat: React.FC<{ value: number; label: string; tint?: string; onPress?: () => void }> = ({
    value,
    label,
    tint,
    onPress,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <TouchableOpacity style={styles.stat} disabled={!onPress} onPress={onPress}>
            <Text style={[styles.statValue, { color: tint ?? color.text._300 }]}>{value}</Text>
            <Text style={styles.statLabel}>{label}</Text>
        </TouchableOpacity>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        fill: {
            flex: 1,
        },
        page: {
            padding: spacing.xl,
            rowGap: spacing.xl,
            paddingBottom: spacing.xl3,
        },
        headerActions: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.xl2,
            marginRight: spacing.s,
        },
        badge: {
            position: 'absolute',
            top: -6,
            right: -10,
            minWidth: 18,
            height: 18,
            borderRadius: 9,
            paddingHorizontal: 4,
            alignItems: 'center',
            justifyContent: 'center',
        },
        badgeText: {
            color: '#fff',
            fontSize: 11,
            fontWeight: '700',
        },
        empty: {
            flex: 1,
            alignItems: 'center',
            justifyContent: 'center',
            rowGap: spacing.l,
            paddingHorizontal: spacing.xl2,
            paddingBottom: spacing.xl3,
        },
        emptyIcon: {
            width: 80,
            height: 80,
            borderRadius: 24,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.neutral._200,
            marginBottom: spacing.m,
        },
        emptyTitle: {
            color: color.text._100,
            fontSize: fontSize.xl,
            fontWeight: '600',
        },
        emptyText: {
            color: color.text._400,
            textAlign: 'center',
            lineHeight: 20,
            marginBottom: spacing.m,
        },
        hostCard: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            padding: spacing.l,
            borderRadius: 16,
            backgroundColor: color.neutral._200,
        },
        hostIcon: {
            width: 40,
            height: 40,
            borderRadius: 12,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.neutral._300,
        },
        hostLamp: {
            position: 'absolute',
            right: -2,
            bottom: -2,
            width: 12,
            height: 12,
            borderRadius: 6,
            borderWidth: 2,
        },
        hostName: {
            color: color.text._100,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
        hostMeta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        error: {
            color: color.error._300,
        },
        alert: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            padding: spacing.l,
            borderRadius: 16,
            borderWidth: 1,
            backgroundColor: color.neutral._200,
        },
        alertText: {
            flex: 1,
            color: color.text._100,
        },
        alertAction: {
            fontWeight: '600',
        },
        stats: {
            flexDirection: 'row',
            columnGap: spacing.m,
        },
        stat: {
            flex: 1,
            paddingVertical: spacing.l,
            paddingHorizontal: spacing.l,
            borderRadius: 16,
            backgroundColor: color.neutral._200,
            rowGap: 2,
        },
        statValue: {
            fontSize: fontSize.xl2,
            fontWeight: '700',
            fontVariant: ['tabular-nums'],
        },
        statLabel: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        section: {
            rowGap: spacing.m,
        },
        headingRow: {
            flexDirection: 'row',
            alignItems: 'baseline',
            justifyContent: 'space-between',
        },
        heading: {
            flex: 1,
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
        },
        scopeClear: {
            marginLeft: spacing.m,
        },
        link: {
            color: color.primary._700,
            fontSize: fontSize.s,
        },
        none: {
            alignItems: 'center',
            rowGap: spacing.m,
            paddingVertical: spacing.xl2,
            paddingHorizontal: spacing.xl,
            borderRadius: 16,
            borderWidth: 1,
            borderStyle: 'dashed',
            borderColor: color.neutral._400,
        },
        note: {
            color: color.text._400,
            lineHeight: 20,
            textAlign: 'center',
        },
        groups: {
            rowGap: spacing.xl,
        },
        group: {
            rowGap: spacing.s,
        },
        groupHead: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.s,
        },
        groupOpen: {
            flex: 1,
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        groupName: {
            flexShrink: 1,
            color: color.text._200,
            fontSize: fontSize.m,
            fontWeight: '600',
        },
        composer: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            marginHorizontal: spacing.xl,
            marginBottom: spacing.l,
            paddingLeft: spacing.l,
            paddingRight: spacing.s,
            paddingVertical: spacing.s,
            borderRadius: 24,
            backgroundColor: color.neutral._200,
            borderColor: color.neutral._400,
            borderWidth: 1,
        },
        composerText: {
            flex: 1,
            color: color.text._500,
            fontSize: fontSize.m,
        },
        composerSend: {
            width: 32,
            height: 32,
            borderRadius: 16,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.primary._500,
        },
    })
}
