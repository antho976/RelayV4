import { useFocusEffect, useLocalSearchParams, useRouter } from 'expo-router'
import React, { useCallback, useEffect, useState } from 'react'
import { RefreshControl, ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import DropdownSheet from '@components/input/DropdownSheet'
import SectionTitle from '@components/text/SectionTitle'
import Alert from '@components/views/Alert'
import HeaderTitle from '@components/views/HeaderTitle'
import { relay, RelayProject, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type Task = {
    id: number
    project_id: number
    title: string
    body: string
    changelog: string
    column: 'backlog' | 'ready' | 'active' | 'in_review' | 'done'
    state: string
    priority: string
    labels: string[]
    sessions: string[]
    commits: { sha: string }[]
    updated_at: string
}

const COLUMNS: { key: Task['column']; title: string; limit?: number }[] = [
    { key: 'in_review', title: 'In review' },
    { key: 'active', title: 'Active' },
    { key: 'ready', title: 'Ready' },
    { key: 'backlog', title: 'Backlog' },
    { key: 'done', title: 'Done', limit: 8 },
]

/**
 * The project board from the phone: what is in review (approve it here), what is running,
 * what is waiting. Dispatching a waiting task launches an agent on the PC, exactly as a
 * request from the PC tab does.
 */
const BoardScreen = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const projects = useRelayStore((state) => state.projects)
    const status = useRelayStore((state) => state.status)
    const params = useLocalSearchParams<{ project?: string }>()
    const [project, setProject] = useState<RelayProject | undefined>(() =>
        useRelayStore.getState().projects.find((item) => String(item.id) === params.project)
    )
    // Tagged with the project they came from, so a slow answer for a project the person has
    // since switched away from never shows under the new one.
    const [loaded, setLoaded] = useState<{ projectId?: number; tasks: Task[] }>({ tasks: [] })
    const [loading, setLoading] = useState(false)
    const [open, setOpen] = useState<number | undefined>(undefined)

    // The chosen project, or the first one when the choice is stale (removed on the PC) or unmade.
    const current = (project && projects.find((item) => item.id === project.id)) ?? projects[0]
    const currentId = current?.id
    const tasks = loaded.projectId === currentId ? loaded.tasks : []

    // Keyed on the id, not the object: every refresh hands out new project objects, and the
    // board should not refetch for those.
    const load = useCallback(async () => {
        if (currentId === undefined || status !== 'online') return
        setLoading(true)
        try {
            const result = await relay.request<{ tasks: Task[] }>('task.list', {
                project_id: currentId,
                sort: 'updated',
            })
            setLoaded({ projectId: currentId, tasks: result.tasks })
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
        } finally {
            setLoading(false)
        }
    }, [currentId, status])

    useFocusEffect(
        useCallback(() => {
            load()
        }, [load])
    )

    // Board events refresh the list; the engine emits one per transition, not per byte.
    useEffect(
        () =>
            relay.onEvents((event) => {
                if (event.ev === 'task.changed' || event.ev === 'task.deleted') load()
            }),
        [load]
    )

    const approve = (task: Task) => {
        Alert.alert({
            title: 'Approve task',
            description: `Move "${task.title}" to done? The branch head is linked as its commit.`,
            buttons: [
                { label: 'Cancel' },
                {
                    label: 'Approve',
                    onPress: async () => {
                        try {
                            await relay.request('task.approve', { task_id: task.id })
                            Logger.infoToast('Approved')
                            load()
                        } catch (e) {
                            Logger.errorToast(`${(e as Error).message}`)
                        }
                    },
                },
            ],
        })
    }

    const dispatch = (task: Task) => {
        const launch = async (provider: 'claude' | 'codex') => {
            try {
                const result = await relay.request<{ session: { name: string } }>('task.dispatch', {
                    task_id: task.id,
                    create: { project_id: task.project_id, provider: provider, role: 'builder' },
                    start: true,
                })
                Logger.infoToast(`Sent to ${result.session.name}`)
                router.push({
                    pathname: '/screens/RelayScreen/Terminal',
                    params: { session: result.session.name },
                })
            } catch (e) {
                Logger.errorToast(`${(e as Error).message}`)
            }
        }
        Alert.alert({
            title: 'Dispatch task',
            description: `Launch an agent on "${task.title}"?`,
            buttons: [
                { label: 'Cancel' },
                { label: 'Codex', onPress: () => launch('codex') },
                { label: 'Claude', onPress: () => launch('claude') },
            ],
        })
    }

    const openSession = (task: Task) => {
        const name = task.sessions[task.sessions.length - 1]
        if (!name) return
        router.push({ pathname: '/screens/RelayScreen/Terminal', params: { session: name } })
    }

    const openChanges = (task: Task) => {
        const name = task.sessions[task.sessions.length - 1]
        if (!name) return
        router.push({ pathname: '/screens/RelayScreen/Changes', params: { session: name } })
    }

    return (
        <SafeAreaView edges={['bottom']} style={{ flex: 1 }}>
            <HeaderTitle title="Board" />
            <ScrollView
                contentContainerStyle={styles.page}
                refreshControl={
                    <RefreshControl
                        refreshing={loading}
                        onRefresh={load}
                        tintColor={color.text._300}
                        colors={[color.text._300]}
                    />
                }>
                <DropdownSheet
                    data={projects}
                    selected={current}
                    onChangeValue={setProject}
                    labelExtractor={(item) => item.name}
                    placeholder="Project"
                    modalTitle="Project"
                    search={projects.length > 6}
                />
                {status !== 'online' && <Text style={styles.note}>Not connected to the PC.</Text>}
                {COLUMNS.map((column) => {
                    const items = tasks.filter((task) => task.column === column.key)
                    if (items.length === 0) return null
                    const shown = column.limit ? items.slice(0, column.limit) : items
                    return (
                        <View key={column.key} style={styles.section}>
                            <SectionTitle>
                                {column.title} · {items.length}
                            </SectionTitle>
                            {shown.map((task) => {
                                const expanded = open === task.id
                                const session = task.sessions[task.sessions.length - 1]
                                return (
                                    <TouchableOpacity
                                        key={task.id}
                                        style={styles.card}
                                        onPress={() => setOpen(expanded ? undefined : task.id)}>
                                        <View style={styles.cardHead}>
                                            <Text style={styles.id}>#{task.id}</Text>
                                            <Text
                                                numberOfLines={expanded ? undefined : 2}
                                                style={styles.title}>
                                                {task.title}
                                            </Text>
                                        </View>
                                        <Text style={styles.meta}>
                                            {task.priority}
                                            {task.state !== 'none'
                                                ? ` · ${task.state.replace('_', ' ')}`
                                                : ''}
                                            {session ? ` · ${session}` : ''}
                                            {task.labels.length > 0
                                                ? ` · ${task.labels.join(', ')}`
                                                : ''}
                                        </Text>
                                        {expanded && (
                                            <View style={styles.detail}>
                                                {!!task.body && (
                                                    <Text selectable style={styles.body}>
                                                        {task.body}
                                                    </Text>
                                                )}
                                                {!!task.changelog && (
                                                    <View>
                                                        <Text style={styles.label}>CHANGELOG</Text>
                                                        <Text selectable style={styles.body}>
                                                            {task.changelog}
                                                        </Text>
                                                    </View>
                                                )}
                                                <View style={styles.actions}>
                                                    {session && (
                                                        <ThemedButton
                                                            label="Terminal"
                                                            variant="secondary"
                                                            onPress={() => openSession(task)}
                                                        />
                                                    )}
                                                    {session && task.column !== 'done' && (
                                                        <ThemedButton
                                                            label="Changes"
                                                            variant="secondary"
                                                            onPress={() => openChanges(task)}
                                                        />
                                                    )}
                                                    {task.column === 'in_review' && (
                                                        <ThemedButton
                                                            label="Approve"
                                                            onPress={() => approve(task)}
                                                        />
                                                    )}
                                                    {(task.column === 'backlog' ||
                                                        task.column === 'ready') && (
                                                        <ThemedButton
                                                            label="Dispatch"
                                                            onPress={() => dispatch(task)}
                                                        />
                                                    )}
                                                </View>
                                            </View>
                                        )}
                                    </TouchableOpacity>
                                )
                            })}
                            {shown.length < items.length && (
                                <Text style={styles.note}>
                                    {items.length - shown.length} more on the desktop.
                                </Text>
                            )}
                        </View>
                    )
                })}
                {status === 'online' && tasks.length === 0 && !loading && (
                    <Text style={styles.note}>No tasks in this project yet.</Text>
                )}
            </ScrollView>
        </SafeAreaView>
    )
}

export default BoardScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        page: {
            padding: spacing.xl,
            rowGap: spacing.xl,
            paddingBottom: spacing.xl3,
        },
        section: {
            rowGap: spacing.s,
        },
        card: {
            backgroundColor: color.neutral._300,
            padding: spacing.l,
            rowGap: spacing.s,
        },
        cardHead: {
            flexDirection: 'row',
            columnGap: spacing.m,
            alignItems: 'flex-start',
        },
        id: {
            color: color.text._500,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
            paddingTop: 2,
        },
        title: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.m,
        },
        meta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        detail: {
            rowGap: spacing.m,
            marginTop: spacing.s,
        },
        label: {
            color: color.text._500,
            fontSize: fontSize.s - 1,
            letterSpacing: 1,
            marginBottom: 2,
        },
        body: {
            color: color.text._200,
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        actions: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
            rowGap: spacing.s,
        },
        note: {
            color: color.text._400,
        },
    })
}
