import { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { useRouter } from 'expo-router'
import React from 'react'
import { View } from 'react-native'

import {
    Badge,
    EmptyState,
    relayHref,
    RelayPage,
    Row,
    Screen,
    Section,
    useProjectParam,
    useRelayStore,
} from '@components/relay'
import { usePeers } from '@components/relay/sessions'
import { Theme } from '@lib/theme/ThemeManager'

import SessionItem from './SessionItem'

type Link = { page: RelayPage; label: string; detail: string; icon: AntDesignIconName }

const GROUPS: { title: string; links: Link[] }[] = [
    {
        title: 'Work',
        links: [
            { page: 'Board', label: 'Board', detail: 'Tasks by column', icon: 'project' },
            { page: 'Modules', label: 'Modules', detail: 'Groups of tasks', icon: 'appstore' },
            {
                page: 'Notes',
                label: 'Notes',
                detail: 'Pinned and standing notes',
                icon: 'file-text',
            },
            {
                page: 'Mailbox',
                label: 'Mailbox',
                detail: 'Messages to and from agents',
                icon: 'mail',
            },
            {
                page: 'Launch',
                label: 'Launch agents',
                detail: 'Solo or review group',
                icon: 'rocket',
            },
        ],
    },
    {
        title: 'Code',
        links: [
            { page: 'Changes', label: 'Changes', detail: 'Stage, commit, push', icon: 'diff' },
            { page: 'Git', label: 'Git', detail: 'History, branches', icon: 'branches' },
            { page: 'Files', label: 'Files', detail: 'Browse, search, edit', icon: 'folder' },
            {
                page: 'Integration',
                label: 'Integration',
                detail: 'Merge sessions and build',
                icon: 'experiment',
            },
        ],
    },
    {
        title: 'Project',
        links: [
            {
                page: 'Guardrails',
                label: 'Guardrails',
                detail: 'Holds and overlaps',
                icon: 'safety',
            },
            { page: 'Devices', label: 'Android devices', detail: 'Run and build', icon: 'mobile' },
            {
                page: 'ProjectSettings',
                label: 'Settings',
                detail: 'Name, commands, guardrails',
                icon: 'setting',
            },
        ],
    },
]

/**
 * One project on the PC: its live sessions, and a way into every surface the desktop has for
 * it. Reached from a project's row on the PC tab and in the sidebar.
 */
const ProjectScreen = () => {
    const router = useRouter()
    const { spacing } = Theme.useTheme()
    const { projectId, project } = useProjectParam()
    const sessions = useRelayStore((state) => state.sessions)
    const live = sessions.filter((item) => item.project_id === projectId && item.state !== 'closed')
    const params = { project_id: String(projectId ?? '') }
    const peers = usePeers(projectId !== undefined && live.length > 0 ? [projectId] : [])

    const open = (page: RelayPage) =>
        router.push(
            // The board predates `project_id` and reads `project`.
            relayHref(page, page === 'Board' ? { ...params, project: params.project_id } : params)
        )

    if (projectId === undefined) {
        return (
            <Screen title="Project">
                <EmptyState
                    icon="folder"
                    title="No project"
                    text="Open a project from the PC tab."
                />
            </Screen>
        )
    }

    return (
        <Screen
            title={project?.name ?? 'Project'}
            actions={[
                { icon: 'setting', label: 'Settings', onPress: () => open('ProjectSettings') },
            ]}>
            {!!project?.path && (
                <Section card={false}>
                    <Row label={project.name} detail={project.path} icon="folder" mono />
                </Section>
            )}
            {live.length > 0 ? (
                <Section
                    title={`Sessions · ${live.length}`}
                    action={{ label: 'Launch agents', onPress: () => open('Launch') }}
                    card={false}>
                    <View style={{ rowGap: spacing.s }}>
                        {live.map((session) => (
                            <SessionItem
                                key={session.name}
                                session={session}
                                peer={peers[session.name]}
                            />
                        ))}
                    </View>
                </Section>
            ) : (
                <Section card={false}>
                    <EmptyState
                        icon="rocket"
                        title="No agents running"
                        text="Launch one or more agents, alone or as a review group."
                        action={{ label: 'Launch agents', onPress: () => open('Launch') }}
                    />
                </Section>
            )}
            {GROUPS.map((group) => (
                <Section key={group.title} title={group.title}>
                    {group.links.map((link) => (
                        <Row
                            key={link.page}
                            label={link.label}
                            detail={link.detail}
                            icon={link.icon}
                            right={
                                link.page === 'Launch' ? <Badge value={live.length} /> : undefined
                            }
                            onPress={() => open(link.page)}
                        />
                    ))}
                </Section>
            ))}
        </Screen>
    )
}

export default ProjectScreen
