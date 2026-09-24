import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { useRouter } from 'expo-router'
import React from 'react'
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import Drawer from '@components/views/Drawer'
import {
    RelayProject,
    RelaySession,
    RelayWorkspace,
    useRelayStore,
} from '@lib/engine/Relay/RelayClient'
import { RelayScope, useRelayView } from '@lib/state/RelayView'
import { Theme } from '@lib/theme/ThemeManager'

import Lamp from './Lamp'

/** The session state a group of sessions shows as one lamp: the one that most needs a look. */
const loudest = (sessions: RelaySession[]): string | undefined => {
    for (const state of ['blocked', 'running', 'spawning', 'idle']) {
        if (sessions.some((item) => item.state === state)) return state
    }
    return sessions[0]?.state
}

type Group = { workspace?: RelayWorkspace; projects: RelayProject[] }

/** Projects under their workspaces, in the desktop's order; strays last, under "Other". */
export const groupProjects = (workspaces: RelayWorkspace[], projects: RelayProject[]): Group[] => {
    const known = new Set(workspaces.map((item) => item.id))
    const byName = (a: RelayProject, b: RelayProject) => a.name.localeCompare(b.name)
    const groups: Group[] = workspaces.map((workspace) => ({
        workspace,
        projects: projects.filter((item) => item.workspace_id === workspace.id).sort(byName),
    }))
    const strays = projects.filter((item) => !known.has(item.workspace_id)).sort(byName)
    if (strays.length > 0) groups.push({ projects: strays })
    return groups
}

type RowProps = {
    active: boolean
    onPress: () => void
    icon?: AntDesignIconName
    label: string
    count: number
    lamp?: string
    indent?: boolean
    onBoard?: () => void
}

const Row: React.FC<RowProps> = ({
    active,
    onPress,
    icon,
    label,
    count,
    lamp,
    indent,
    onBoard,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <TouchableOpacity
            style={[styles.row, indent && styles.indent, active && styles.rowActive]}
            onPress={onPress}>
            {icon ? (
                <AntDesign
                    name={icon}
                    size={16}
                    color={active ? color.text._100 : color.text._400}
                />
            ) : (
                <View style={styles.lampSlot}>
                    {lamp ? <Lamp state={lamp} /> : <View style={styles.lampOff} />}
                </View>
            )}
            <Text numberOfLines={1} style={[styles.rowLabel, active && styles.rowLabelActive]}>
                {label}
            </Text>
            {count > 0 && <Text style={styles.count}>{count}</Text>}
            {onBoard && (
                <TouchableOpacity hitSlop={10} onPress={onBoard}>
                    <AntDesign name="project" size={16} color={color.text._500} />
                </TouchableOpacity>
            )}
        </TouchableOpacity>
    )
}

/**
 * The PC tab's sidebar: every workspace on the PC and the projects in it, each with how many
 * agents are live there. Tapping one narrows the tab to it; the board icon opens that
 * project's board. The rest of the PC's surfaces sit at the bottom.
 */
const WorkspaceDrawer: React.FC<{ onOpenInbox: () => void; onOpenHosts: () => void }> = ({
    onOpenInbox,
    onOpenHosts,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const insets = useSafeAreaInsets()
    const router = useRouter()
    const workspaces = useRelayStore((state) => state.workspaces)
    const projects = useRelayStore((state) => state.projects)
    const sessions = useRelayStore((state) => state.sessions)
    const hostName = useRelayStore((state) => state.hostName)
    const { scope, setScope } = useRelayView()
    const setShow = Drawer.useDrawerStore((state) => state.setShow)

    const live = sessions.filter((item) => item.state !== 'closed')
    const liveIn = (ids: number[]) => live.filter((item) => ids.includes(item.project_id))
    const groups = groupProjects(workspaces, projects)

    const close = () => setShow(Drawer.ID.RELAY, false)
    const pick = (next: RelayScope) => {
        setScope(next)
        close()
    }
    const selected = (candidate: RelayScope) =>
        candidate.kind === scope.kind &&
        (candidate.kind === 'all' || (scope.kind !== 'all' && candidate.id === scope.id))

    const openBoard = (project: RelayProject) => {
        close()
        router.push({
            pathname: '/screens/RelayScreen/Board',
            params: { project: String(project.id) },
        })
    }

    return (
        <Drawer.Body drawerID={Drawer.ID.RELAY} direction="left" drawerStyle={styles.drawer}>
            <View style={styles.head}>
                <Text style={styles.title} numberOfLines={1}>
                    {hostName ?? 'PC'}
                </Text>
                <TouchableOpacity hitSlop={12} onPress={close}>
                    <AntDesign name="close" size={20} color={color.text._400} />
                </TouchableOpacity>
            </View>
            <ScrollView contentContainerStyle={styles.list}>
                <Row
                    active={selected({ kind: 'all' })}
                    onPress={() => pick({ kind: 'all' })}
                    icon="appstore"
                    label="All sessions"
                    count={live.length}
                />
                {groups.map((group) => {
                    const ids = group.projects.map((item) => item.id)
                    return (
                        <View key={group.workspace?.id ?? 'other'} style={styles.group}>
                            {group.workspace ? (
                                <Row
                                    active={selected({ kind: 'workspace', id: group.workspace.id })}
                                    onPress={() =>
                                        pick({ kind: 'workspace', id: group.workspace!.id })
                                    }
                                    icon="folder"
                                    label={group.workspace.name}
                                    count={liveIn(ids).length}
                                />
                            ) : (
                                <Text style={styles.groupTitle}>Other projects</Text>
                            )}
                            {group.projects.map((project) => {
                                const here = liveIn([project.id])
                                return (
                                    <Row
                                        key={project.id}
                                        active={selected({ kind: 'project', id: project.id })}
                                        onPress={() => pick({ kind: 'project', id: project.id })}
                                        label={project.name}
                                        count={here.length}
                                        lamp={loudest(here)}
                                        indent
                                        onBoard={() => openBoard(project)}
                                    />
                                )
                            })}
                            {group.projects.length === 0 && (
                                <Text style={[styles.empty, styles.indent]}>No projects</Text>
                            )}
                        </View>
                    )
                })}
                {projects.length === 0 && (
                    <Text style={styles.empty}>
                        No projects on this PC yet. Add one from the desktop.
                    </Text>
                )}
            </ScrollView>
            <View style={[styles.footer, { paddingBottom: insets.bottom + 8 }]}>
                <TouchableOpacity
                    style={styles.footerItem}
                    onPress={() => {
                        close()
                        onOpenInbox()
                    }}>
                    <AntDesign name="bell" size={18} color={color.text._300} />
                    <Text style={styles.footerText}>Inbox</Text>
                </TouchableOpacity>
                <TouchableOpacity
                    style={styles.footerItem}
                    onPress={() => {
                        close()
                        router.push('/screens/RelayScreen/Board')
                    }}>
                    <AntDesign name="project" size={18} color={color.text._300} />
                    <Text style={styles.footerText}>Board</Text>
                </TouchableOpacity>
                <TouchableOpacity
                    style={styles.footerItem}
                    onPress={() => {
                        close()
                        onOpenHosts()
                    }}>
                    <AntDesign name="setting" size={18} color={color.text._300} />
                    <Text style={styles.footerText}>Paired PCs</Text>
                </TouchableOpacity>
            </View>
        </Drawer.Body>
    )
}

export default WorkspaceDrawer

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        drawer: {
            backgroundColor: color.neutral._100,
            width: '82%',
            height: '100%',
            position: 'absolute',
            left: 0,
            elevation: 20,
            shadowColor: color.shadow,
            paddingTop: spacing.l,
        },
        head: {
            flexDirection: 'row',
            alignItems: 'center',
            justifyContent: 'space-between',
            paddingHorizontal: spacing.xl,
            paddingBottom: spacing.l,
        },
        title: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.xl,
            fontWeight: '600',
        },
        list: {
            paddingHorizontal: spacing.m,
            paddingBottom: spacing.xl,
            rowGap: spacing.s,
        },
        group: {
            marginTop: spacing.m,
            rowGap: 2,
        },
        groupTitle: {
            color: color.text._500,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.s,
        },
        row: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            borderRadius: 10,
        },
        indent: {
            paddingLeft: spacing.xl2,
        },
        rowActive: {
            backgroundColor: color.neutral._300,
        },
        rowLabel: {
            flex: 1,
            color: color.text._300,
            fontSize: fontSize.m,
        },
        rowLabelActive: {
            color: color.text._100,
            fontWeight: '600',
        },
        lampSlot: {
            width: 16,
            alignItems: 'center',
        },
        lampOff: {
            width: 6,
            height: 6,
            borderRadius: 3,
            backgroundColor: color.neutral._500,
        },
        count: {
            minWidth: 22,
            textAlign: 'center',
            color: color.text._300,
            fontSize: fontSize.s,
            fontVariant: ['tabular-nums'],
            paddingHorizontal: 6,
            paddingVertical: 1,
            borderRadius: 10,
            overflow: 'hidden',
            backgroundColor: color.neutral._300,
        },
        empty: {
            color: color.text._500,
            fontSize: fontSize.s,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.s,
        },
        footer: {
            flexDirection: 'row',
            justifyContent: 'space-around',
            paddingTop: spacing.m,
            borderTopWidth: 1,
            borderTopColor: color.neutral._300,
        },
        footerItem: {
            alignItems: 'center',
            rowGap: 4,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.s,
        },
        footerText: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
    })
}
