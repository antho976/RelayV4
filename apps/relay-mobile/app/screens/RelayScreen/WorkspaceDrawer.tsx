import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import { confirm, relay, relayHref, RelayPage } from '@components/relay'
import { attempt, MenuItem, MenuSheet, PromptSheet } from '@components/relay/settings/common'
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

/** `project.list` rows carry `pinned`, which the store's type leaves out. */
export const isPinned = (project: RelayProject) =>
    !!(project as RelayProject & { pinned?: boolean }).pinned

/**
 * Projects under their workspaces, pinned first as on the desktop, then by name; strays
 * last, under "Other".
 */
export const groupProjects = (workspaces: RelayWorkspace[], projects: RelayProject[]): Group[] => {
    const known = new Set(workspaces.map((item) => item.id))
    const byName = (a: RelayProject, b: RelayProject) =>
        Number(isPinned(b)) - Number(isPinned(a)) || a.name.localeCompare(b.name)
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
    pinned?: boolean
    onBoard?: () => void
    onOpen?: () => void
    onLongPress?: () => void
}

const Row: React.FC<RowProps> = ({
    active,
    onPress,
    icon,
    label,
    count,
    lamp,
    indent,
    pinned,
    onBoard,
    onOpen,
    onLongPress,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <TouchableOpacity
            style={[styles.row, indent && styles.indent, active && styles.rowActive]}
            onPress={onPress}
            onLongPress={onLongPress}>
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
            {pinned && <AntDesign name="pushpin" size={12} color={color.text._500} />}
            {count > 0 && <Text style={styles.count}>{count}</Text>}
            {onBoard && (
                <TouchableOpacity hitSlop={10} onPress={onBoard}>
                    <AntDesign name="project" size={16} color={color.text._500} />
                </TouchableOpacity>
            )}
            {onOpen && (
                <TouchableOpacity hitSlop={10} onPress={onOpen}>
                    <AntDesign name="right" size={16} color={color.text._500} />
                </TouchableOpacity>
            )}
        </TouchableOpacity>
    )
}

type Menu =
    | { kind: 'project'; project: RelayProject }
    | { kind: 'workspace'; workspace: RelayWorkspace }

/**
 * The home screen's workspaces sidebar, on the right (the app's drawer has the left): every
 * workspace on the PC and the projects in it, each with how many agents are live there.
 * Tapping one narrows the home screen to it; the board icon opens that project's board, the
 * arrow its hub. A long press offers what the desktop's sidebar menu
 * does: pin, rename, settings and remove. The rest of the PC's surfaces sit at the bottom.
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
    const [menu, setMenu] = useState<Menu | undefined>(undefined)
    const [renaming, setRenaming] = useState<Menu | undefined>(undefined)

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

    const go = (page: RelayPage, params: Record<string, string> = {}) => {
        close()
        router.push(relayHref(page, params))
    }
    const openProject = (project: RelayProject) => go('Project', { project_id: String(project.id) })

    const rename = async (target: Menu, name: string) => {
        setRenaming(undefined)
        const done = await attempt(() =>
            target.kind === 'project'
                ? relay.guarded('project.update', { project_id: target.project.id, name: name })
                : relay.guarded('workspace.update', {
                      workspace_id: target.workspace.id,
                      name: name,
                  })
        )
        if (done !== undefined) relay.refreshProjects().catch(() => {})
    }

    const removeProject = async (project: RelayProject) => {
        const yes = await confirm({
            title: `Remove ${project.name}?`,
            message:
                'Relay forgets this project: its board, notes and history on the PC go with it. The folder on disk is not touched. A project with live sessions cannot be removed.',
            confirmLabel: 'Remove',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(
            () => relay.guarded('project.remove', { project_id: project.id }),
            `${project.name} removed`
        )
        if (done === undefined) return
        if (scope.kind === 'project' && scope.id === project.id) setScope({ kind: 'all' })
        relay.refreshProjects().catch(() => {})
    }

    const removeWorkspace = async (workspace: RelayWorkspace) => {
        const yes = await confirm({
            title: `Remove ${workspace.name}?`,
            message:
                'Relay forgets this workspace. Its folder stays on disk. Remove its projects first; a workspace that still has projects cannot be removed.',
            confirmLabel: 'Remove',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(
            () => relay.guarded('workspace.remove', { workspace_id: workspace.id }),
            `${workspace.name} removed`
        )
        if (done === undefined) return
        if (scope.kind === 'workspace' && scope.id === workspace.id) setScope({ kind: 'all' })
        relay.refreshProjects().catch(() => {})
    }

    const menuItems = (target: Menu): MenuItem[] => {
        if (target.kind === 'workspace')
            return [
                { label: 'Rename', icon: 'edit', onPress: () => setRenaming(target) },
                {
                    label: 'Remove workspace',
                    icon: 'delete',
                    destructive: true,
                    onPress: () => removeWorkspace(target.workspace),
                },
            ]
        const project = target.project
        const pinned = isPinned(project)
        return [
            { label: 'Open', icon: 'right-circle', onPress: () => openProject(project) },
            {
                label: pinned ? 'Unpin' : 'Pin',
                icon: 'pushpin',
                onPress: () =>
                    attempt(() =>
                        relay.guarded('project.update', { project_id: project.id, pinned: !pinned })
                    ).then((done) => {
                        if (done !== undefined) relay.refreshProjects().catch(() => {})
                    }),
            },
            { label: 'Rename', icon: 'edit', onPress: () => setRenaming(target) },
            {
                label: 'Settings',
                icon: 'setting',
                onPress: () => go('ProjectSettings', { project_id: String(project.id) }),
            },
            {
                label: 'Remove project',
                icon: 'delete',
                destructive: true,
                onPress: () => removeProject(project),
            },
        ]
    }

    const menuName = (target?: Menu) =>
        target?.kind === 'project' ? target.project.name : target?.workspace.name

    const openBoard = (project: RelayProject) => {
        close()
        router.push({
            pathname: '/screens/RelayScreen/Board',
            params: { project: String(project.id) },
        })
    }

    return (
        <>
            <Drawer.Body drawerID={Drawer.ID.RELAY} direction="right" drawerStyle={styles.drawer}>
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
                                        active={selected({
                                            kind: 'workspace',
                                            id: group.workspace.id,
                                        })}
                                        onPress={() =>
                                            pick({ kind: 'workspace', id: group.workspace!.id })
                                        }
                                        icon="folder"
                                        label={group.workspace.name}
                                        count={liveIn(ids).length}
                                        onLongPress={() =>
                                            setMenu({
                                                kind: 'workspace',
                                                workspace: group.workspace!,
                                            })
                                        }
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
                                            onPress={() =>
                                                pick({ kind: 'project', id: project.id })
                                            }
                                            label={project.name}
                                            pinned={isPinned(project)}
                                            count={here.length}
                                            lamp={loudest(here)}
                                            indent
                                            onBoard={() => openBoard(project)}
                                            onOpen={() => openProject(project)}
                                            onLongPress={() =>
                                                setMenu({ kind: 'project', project: project })
                                            }
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
                        <Text style={styles.empty}>No projects on this PC yet.</Text>
                    )}
                    <Row
                        active={false}
                        onPress={() => go('AddProject')}
                        icon="folder-add"
                        label="Add a project"
                        count={0}
                    />
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
                        <AntDesign name="desktop" size={18} color={color.text._300} />
                        <Text style={styles.footerText}>Paired PCs</Text>
                    </TouchableOpacity>
                    <TouchableOpacity style={styles.footerItem} onPress={() => go('PcSettings')}>
                        <AntDesign name="setting" size={18} color={color.text._300} />
                        <Text style={styles.footerText}>Settings</Text>
                    </TouchableOpacity>
                </View>
            </Drawer.Body>
            <MenuSheet
                visible={!!menu}
                title={menuName(menu)}
                detail={menu?.kind === 'project' ? menu.project.path : menu?.workspace.path}
                items={menu ? menuItems(menu) : []}
                onDismiss={() => setMenu(undefined)}
            />
            <PromptSheet
                visible={!!renaming}
                title={renaming?.kind === 'workspace' ? 'Rename workspace' : 'Rename project'}
                label="Name"
                initial={menuName(renaming) ?? ''}
                confirmLabel="Rename"
                onSubmit={(name) => renaming && rename(renaming, name)}
                onDismiss={() => setRenaming(undefined)}
            />
        </>
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
            right: 0,
            elevation: 20,
            shadowColor: color.shadow,
            paddingTop: spacing.l,
            borderTopLeftRadius: 20,
            borderBottomLeftRadius: 20,
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
