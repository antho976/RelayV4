import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useFocusEffect, useRouter } from 'expo-router'
import React, { useCallback, useState } from 'react'
import { RefreshControl, ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { SafeAreaView, useSafeAreaInsets } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    isStopped,
    StatsStrip,
    StoppedAgents,
    UsageRow,
    usePeers,
} from '@components/relay/sessions'
import Drawer from '@components/views/Drawer'
import HeaderButton from '@components/views/HeaderButton'
import HeaderTitle from '@components/views/HeaderTitle'
import SettingsDrawer from '@components/views/SettingsDrawer'
import { relay, RelayProject, RelaySession, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { activeHost, isTailnetUrl, useRelayHostsStore } from '@lib/state/RelayHosts'
import { useRelayView } from '@lib/state/RelayView'
import { Theme } from '@lib/theme/ThemeManager'

import { linkColor } from './console'
import NewTerminalSheet from './NewTerminalSheet'
import SessionItem from './SessionItem'
import WorkspaceDrawer, { groupProjects } from './WorkspaceDrawer'

/**
 * The home screen: the agent wall on the desktop, from a phone. One line says whether the PC
 * is there; another appears only when something needs the person; a slim card carries the
 * figures and provider usage. Then the live agents, grouped by project, and at the end every
 * stopped agent folded into one row. The New terminal button starts another agent.
 *
 * Everything else lives once, elsewhere. On the left, the app's drawer: the PC first, then
 * what runs on the phone itself (characters, models, recent chats). On the right, the bell
 * (holds, reviews and notifications) and the workspaces sidebar, which picks the workspace or
 * project this page shows, lists the PC's other surfaces, and slides in from that side.
 */
const RelayScreen = () => {
    const styles = useStyles()
    const { color, spacing } = Theme.useTheme()
    const insets = useSafeAreaInsets()
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
    const { scope } = useRelayView()
    const setDrawer = Drawer.useDrawerStore((state) => state.setShow)
    const [showNew, setShowNew] = useState(false)
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
    const badge = holds.length + notifications.length + inReview

    // The scope the sidebar picked, if what it points at still exists on this PC.
    const scopeProject =
        scope.kind === 'project' ? projects.find((p) => p.id === scope.id) : undefined
    const scopeWorkspace =
        scope.kind === 'workspace' ? workspaces.find((w) => w.id === scope.id) : undefined
    const inScope = (projectId: number | undefined) => {
        if (scopeProject) return projectId === scopeProject.id
        if (scopeWorkspace) {
            const project = projects.find((p) => p.id === projectId)
            return project?.workspace_id === scopeWorkspace.id
        }
        return true
    }
    // Live is everything on the wall that is not stopped; stopped agents have a row of their own.
    const shown = sessions.filter(
        (item) => item.state !== 'closed' && !isStopped(item.state) && inScope(item.project_id)
    )
    const running = shown.filter((item) => item.state === 'running').length
    const blocked = shown.filter((item) => item.state === 'blocked').length

    // Sessions under their project, projects in the sidebar's order. A project with none is
    // left out, except the one the tab is narrowed to: its header is the page's title.
    const sections = groupProjects(workspaces, projects)
        .flatMap((group) => group.projects)
        .map((project) => ({
            project: project,
            sessions: shown.filter((item) => item.project_id === project.id),
        }))
        .filter((section) => section.sessions.length > 0 || section.project.id === scopeProject?.id)
    const orphans = shown.filter((item) => !projects.some((p) => p.id === item.project_id))
    // What each visible agent says it is doing: one peer table per project on screen.
    const peers = usePeers(
        online ? sections.filter((s) => s.sessions.length > 0).map((s) => s.project.id) : []
    )

    const routeLabel =
        transport === 'via'
            ? 'via server'
            : routeUrl && isTailnetUrl(routeUrl)
              ? 'Tailscale'
              : 'same network'

    const attention = [
        holds.length > 0 && `${holds.length} ${holds.length === 1 ? 'hold' : 'holds'}`,
        blocked > 0 && `${blocked} blocked`,
        inReview > 0 && `${inReview} to review`,
    ].filter(Boolean) as string[]

    const headerLeft = () => <Drawer.Button drawerID={Drawer.ID.SETTINGS} />
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
                <TouchableOpacity
                    hitSlop={10}
                    accessibilityLabel="Workspaces"
                    onPress={() => setDrawer(Drawer.ID.RELAY, true)}>
                    <AntDesign name="folder" size={22} color={color.text._200} />
                </TouchableOpacity>
            </View>
        ) : null

    if (!paired) {
        return (
            <Drawer.Gesture
                config={[
                    {
                        drawerID: Drawer.ID.SETTINGS,
                        openDirection: 'right',
                        closeDirection: 'left',
                    },
                ]}>
                <SafeAreaView edges={['bottom']} style={styles.fill}>
                    <HeaderTitle title="Relay" />
                    <HeaderButton headerLeft={headerLeft} headerRight={headerRight} />
                    <View style={styles.empty}>
                        <View style={styles.emptyIcon}>
                            <AntDesign name="desktop" size={40} color={color.text._300} />
                        </View>
                        <Text style={styles.emptyTitle}>Your desktop, from here</Text>
                        <Text style={styles.emptyText}>
                            Pair this phone with the PC that runs Relay. On the same WiFi the link
                            is direct and never leaves your network. Away from home, use Tailscale
                            on both devices, or a server you host.
                        </Text>
                        <ThemedButton label="Pair a PC" iconName="qrcode" onPress={openHosts} />
                        <TouchableOpacity
                            hitSlop={8}
                            style={styles.localLink}
                            onPress={() => router.push('/screens/CharacterListScreen')}>
                            <Text style={styles.localLinkText}>
                                Or chat with a model on this phone
                            </Text>
                        </TouchableOpacity>
                    </View>
                    <SettingsDrawer />
                </SafeAreaView>
            </Drawer.Gesture>
        )
    }

    const projectHead = (project: RelayProject) => (
        <View style={styles.groupHead}>
            <TouchableOpacity style={styles.groupOpen} onPress={() => openProject(project)}>
                <Text numberOfLines={1} style={styles.groupName}>
                    {project.name}
                </Text>
                <AntDesign name="right" size={11} color={color.text._500} />
            </TouchableOpacity>
            <TouchableOpacity
                hitSlop={10}
                style={styles.iconButton}
                accessibilityLabel={`${project.name} board`}
                onPress={() => openBoard(project)}>
                <AntDesign name="project" size={15} color={color.text._400} />
            </TouchableOpacity>
        </View>
    )

    return (
        <Drawer.Gesture
            config={[
                { drawerID: Drawer.ID.SETTINGS, openDirection: 'right', closeDirection: 'left' },
                { drawerID: Drawer.ID.RELAY, openDirection: 'left', closeDirection: 'right' },
            ]}>
            <View style={styles.fill}>
                <HeaderTitle title="Relay" />
                <HeaderButton headerLeft={headerLeft} headerRight={headerRight} />
                <NewTerminalSheet
                    visible={showNew}
                    setVisible={setShowNew}
                    projectId={scopeProject?.id}
                />
                <ScrollView
                    contentContainerStyle={[
                        styles.page,
                        { paddingBottom: insets.bottom + (online ? 96 : spacing.xl3) },
                    ]}
                    refreshControl={
                        <RefreshControl
                            refreshing={refreshing}
                            onRefresh={handleRefresh}
                            tintColor={color.text._300}
                            colors={[color.text._300]}
                        />
                    }>
                    <TouchableOpacity style={styles.hostRow} onPress={openHosts}>
                        <View
                            style={[styles.lamp, { backgroundColor: linkColor(status, color) }]}
                        />
                        <Text numberOfLines={1} style={styles.hostName}>
                            {hostName ?? activeHost()?.name ?? 'PC'}
                        </Text>
                        <Text
                            numberOfLines={1}
                            style={[styles.hostMeta, !online && !!error && styles.error]}>
                            {online
                                ? `${routeLabel}${version ? ` · v${version}` : ''}`
                                : status === 'connecting'
                                  ? 'Connecting…'
                                  : 'Not connected'}
                        </Text>
                        <AntDesign name="right" size={11} color={color.text._500} />
                    </TouchableOpacity>

                    {!online && (
                        <View style={styles.none}>
                            <AntDesign name="disconnect" size={26} color={color.text._500} />
                            <Text style={styles.noneTitle}>
                                {status === 'connecting' ? 'Reaching the PC…' : 'Not connected'}
                            </Text>
                            {status === 'offline' && (
                                <>
                                    <Text style={styles.note}>
                                        {error ?? 'The PC is paired but not reachable right now.'}
                                    </Text>
                                    <ThemedButton
                                        label="Connect"
                                        iconName="reload"
                                        variant="secondary"
                                        onPress={connect}
                                    />
                                </>
                            )}
                        </View>
                    )}

                    {online && attention.length > 0 && (
                        <TouchableOpacity
                            style={[
                                styles.attention,
                                holds.length > 0 && { borderColor: color.error._400 },
                            ]}
                            onPress={openInbox}>
                            <AntDesign
                                name={holds.length > 0 ? 'safety' : 'bell'}
                                size={14}
                                color={holds.length > 0 ? color.error._300 : color.quote}
                            />
                            <Text numberOfLines={1} style={styles.attentionText}>
                                {attention.join(' · ')}
                            </Text>
                            <AntDesign name="right" size={11} color={color.text._500} />
                        </TouchableOpacity>
                    )}

                    {online && (
                        <View style={styles.overview}>
                            <StatsStrip
                                open={shown.length}
                                running={running}
                                onBoard={() => openBoard(scopeProject)}
                            />
                            <View style={styles.divider} />
                            <UsageRow />
                        </View>
                    )}

                    {online && scopeWorkspace && (
                        <Text numberOfLines={1} style={styles.scopeTitle}>
                            {scopeWorkspace.name}
                        </Text>
                    )}

                    {online && (
                        <View style={styles.groups}>
                            {sections.map(({ project, sessions: items }) => (
                                <View key={project.id} style={styles.group}>
                                    {projectHead(project)}
                                    {items.map((session) => (
                                        <SessionItem
                                            key={session.name}
                                            session={session}
                                            peer={peers[session.name]}
                                        />
                                    ))}
                                </View>
                            ))}
                            {orphans.length > 0 && (
                                <View style={styles.group}>
                                    {orphans.map((session: RelaySession) => (
                                        <SessionItem key={session.name} session={session} />
                                    ))}
                                </View>
                            )}
                            {shown.length === 0 && (
                                <View style={styles.none}>
                                    <AntDesign name="code" size={26} color={color.text._500} />
                                    <Text style={styles.noneTitle}>No agents running</Text>
                                    <Text style={styles.note}>
                                        {scopeProject || scopeWorkspace
                                            ? `Nothing is running in ${(scopeProject ?? scopeWorkspace)!.name}. `
                                            : ''}
                                        Start one in a terminal; it shows up here as it runs.
                                    </Text>
                                    <ThemedButton
                                        label="New terminal"
                                        iconName="plus"
                                        variant="secondary"
                                        onPress={() => setShowNew(true)}
                                    />
                                </View>
                            )}
                            <StoppedAgents sessions={sessions} inScope={inScope} />
                        </View>
                    )}
                </ScrollView>

                {online && (
                    <TouchableOpacity
                        style={[styles.fab, { bottom: insets.bottom + spacing.xl }]}
                        onPress={() => setShowNew(true)}>
                        <AntDesign name="plus" size={16} color={color.primary._100} />
                        <Text style={styles.fabText}>New terminal</Text>
                    </TouchableOpacity>
                )}
                <WorkspaceDrawer onOpenInbox={openInbox} onOpenHosts={openHosts} />
                <SettingsDrawer />
            </View>
        </Drawer.Gesture>
    )
}

export default RelayScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        fill: {
            flex: 1,
        },
        localLink: {
            marginTop: 12,
        },
        localLinkText: {
            color: color.primary._700,
            fontSize: fontSize.s,
        },
        page: {
            paddingHorizontal: spacing.xl,
            paddingTop: spacing.m,
            rowGap: spacing.l,
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
            fontFamily: 'serif',
            fontSize: fontSize.xl2,
        },
        emptyText: {
            color: color.text._400,
            textAlign: 'center',
            lineHeight: 20,
            marginBottom: spacing.m,
        },
        hostRow: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.s,
            paddingVertical: spacing.s,
        },
        lamp: {
            width: 8,
            height: 8,
            borderRadius: 4,
        },
        hostName: {
            flexShrink: 1,
            color: color.text._100,
            fontSize: fontSize.m,
            fontWeight: '600',
        },
        hostMeta: {
            flex: 1,
            color: color.text._500,
            fontSize: fontSize.s,
        },
        error: {
            color: color.error._300,
        },
        attention: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            borderRadius: 999,
            borderWidth: 1,
            borderColor: color.neutral._400,
            backgroundColor: color.neutral._200,
        },
        attentionText: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.s,
        },
        overview: {
            rowGap: spacing.m,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            borderRadius: 14,
            backgroundColor: color.neutral._200,
        },
        divider: {
            height: StyleSheet.hairlineWidth,
            backgroundColor: color.neutral._400,
        },
        scopeTitle: {
            color: color.text._100,
            fontFamily: 'serif',
            fontSize: fontSize.xl,
            paddingHorizontal: spacing.s,
        },
        none: {
            alignItems: 'center',
            rowGap: spacing.m,
            paddingVertical: spacing.xl2,
            paddingHorizontal: spacing.xl,
            borderRadius: 14,
            backgroundColor: color.neutral._200,
        },
        noneTitle: {
            color: color.text._100,
            fontFamily: 'serif',
            fontSize: fontSize.xl,
        },
        note: {
            color: color.text._400,
            lineHeight: 20,
            textAlign: 'center',
        },
        groups: {
            rowGap: spacing.l,
        },
        group: {
            rowGap: 6,
        },
        groupHead: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingLeft: spacing.s,
        },
        groupOpen: {
            flex: 1,
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.s,
            paddingVertical: 2,
        },
        groupName: {
            flexShrink: 1,
            color: color.text._200,
            fontFamily: 'serif',
            fontSize: fontSize.l,
        },
        iconButton: {
            width: 30,
            height: 30,
            borderRadius: 15,
            alignItems: 'center',
            justifyContent: 'center',
        },
        fab: {
            position: 'absolute',
            right: spacing.xl,
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.xl,
            paddingVertical: spacing.l,
            borderRadius: 999,
            backgroundColor: color.primary._500,
            elevation: 4,
            shadowColor: '#000',
            shadowOpacity: 0.3,
            shadowRadius: 8,
            shadowOffset: { width: 0, height: 3 },
        },
        fabText: {
            color: color.primary._100,
            fontSize: fontSize.m,
            fontWeight: '600',
        },
    })
}
