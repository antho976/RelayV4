import { useFocusEffect, useLocalSearchParams } from 'expo-router'
import React, { useCallback, useState } from 'react'
import { RefreshControl, ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import HeaderTitle from '@components/views/HeaderTitle'
import { relay, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Theme } from '@lib/theme/ThemeManager'

type DiffFile = {
    path: string
    old_path: string | null
    status: string
    added: number
    removed: number
    binary: boolean
}

type Status = {
    branch: string
    upstream: string | null
    ahead: number | null
    behind: number | null
    files: { path: string; index: string; worktree: string }[]
}

type Hunk = {
    old_start: number
    new_start: number
    text: string
}

/**
 * What an agent changed, read from its worktree: the files with their counts, and one
 * file's hunks when tapped. Reviewing from the phone means reading a diff; this is the diff.
 */
const ChangesScreen = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const params = useLocalSearchParams<{ session: string }>()
    const name = typeof params.session === 'string' ? params.session : ''
    const session = useRelayStore((state) => state.sessions.find((item) => item.name === name))
    const status = useRelayStore((state) => state.status)
    const [git, setGit] = useState<Status | undefined>(undefined)
    const [files, setFiles] = useState<DiffFile[]>([])
    const [selected, setSelected] = useState<string | undefined>(undefined)
    const [hunks, setHunks] = useState<Hunk[]>([])
    const [loading, setLoading] = useState(false)
    const [problem, setProblem] = useState('')

    // Only the scope matters here; the session object itself changes on every state flip.
    const projectId = session?.project_id
    const worktree = session?.worktree
    const load = useCallback(async () => {
        if (projectId === undefined || worktree === undefined || status !== 'online') return
        setLoading(true)
        setProblem('')
        try {
            const scope = { project_id: projectId, worktree: worktree }
            const [gitStatus, diff] = await Promise.all([
                relay.request<Status>('git.status', scope),
                relay.request<{ files: DiffFile[] }>('git.diff', scope),
            ])
            setGit(gitStatus)
            setFiles(diff.files)
        } catch (e) {
            setProblem(`${(e as Error).message}`)
        } finally {
            setLoading(false)
        }
    }, [projectId, worktree, status])

    useFocusEffect(
        useCallback(() => {
            load()
        }, [load])
    )

    const show = async (file: DiffFile) => {
        if (!session) return
        if (selected === file.path) {
            setSelected(undefined)
            setHunks([])
            return
        }
        setSelected(file.path)
        setHunks([])
        if (file.binary) return
        try {
            const result = await relay.request<{ hunks: Hunk[] }>('git.diff.file', {
                project_id: session.project_id,
                worktree: session.worktree,
                path: file.path,
            })
            setHunks(result.hunks)
        } catch (e) {
            setProblem(`${(e as Error).message}`)
        }
    }

    const untracked = git?.files.filter((item) => item.worktree === '?') ?? []

    return (
        <SafeAreaView edges={['bottom']} style={{ flex: 1 }}>
            <HeaderTitle title={name ? `${name} · changes` : 'Changes'} />
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
                {!session && (
                    <Text style={styles.note}>Session not found on the connected PC.</Text>
                )}
                {!!problem && <Text style={styles.problem}>{problem}</Text>}
                {git && (
                    <View style={styles.strip}>
                        <Text style={styles.branch}>{git.branch}</Text>
                        <Text style={styles.meta}>
                            {git.upstream
                                ? `↑${git.ahead ?? 0} ↓${git.behind ?? 0}`
                                : 'no upstream'}
                            {' · '}
                            {files.length} changed
                            {untracked.length > 0 ? ` · ${untracked.length} untracked` : ''}
                        </Text>
                    </View>
                )}
                {git && files.length === 0 && untracked.length === 0 && (
                    <Text style={styles.note}>Nothing changed in this worktree yet.</Text>
                )}
                {files.map((file) => (
                    <View key={file.path}>
                        <TouchableOpacity style={styles.file} onPress={() => show(file)}>
                            <Text style={styles.status}>{file.status}</Text>
                            <Text numberOfLines={1} style={styles.path}>
                                {file.old_path ? `${file.old_path} → ` : ''}
                                {file.path}
                            </Text>
                            {file.binary ? (
                                <Text style={styles.meta}>binary</Text>
                            ) : (
                                <Text style={styles.counts}>
                                    <Text style={{ color: '#2ec469' }}>+{file.added}</Text>{' '}
                                    <Text style={{ color: color.error._300 }}>-{file.removed}</Text>
                                </Text>
                            )}
                        </TouchableOpacity>
                        {selected === file.path && (
                            <View style={styles.diff}>
                                {hunks.length === 0 && !file.binary && (
                                    <Text style={styles.meta}>Loading…</Text>
                                )}
                                {hunks.map((hunk, index) => (
                                    <View key={index}>
                                        <Text style={styles.hunkHead}>
                                            @@ -{hunk.old_start} +{hunk.new_start} @@
                                        </Text>
                                        {hunk.text.split('\n').map((line, i) => (
                                            <Text
                                                key={i}
                                                selectable
                                                style={[
                                                    styles.line,
                                                    line.startsWith('+') && styles.added,
                                                    line.startsWith('-') && styles.removed,
                                                ]}>
                                                {line || ' '}
                                            </Text>
                                        ))}
                                    </View>
                                ))}
                            </View>
                        )}
                    </View>
                ))}
                {untracked.map((file) => (
                    <View key={file.path} style={styles.file}>
                        <Text style={styles.status}>?</Text>
                        <Text numberOfLines={1} style={styles.path}>
                            {file.path}
                        </Text>
                        <Text style={styles.meta}>new</Text>
                    </View>
                ))}
            </ScrollView>
        </SafeAreaView>
    )
}

export default ChangesScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        page: {
            padding: spacing.xl,
            rowGap: 2,
            paddingBottom: spacing.xl3,
        },
        strip: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.m,
            minHeight: 30,
            backgroundColor: color.neutral._200,
            marginBottom: spacing.m,
        },
        branch: {
            color: color.text._100,
            fontFamily: 'monospace',
            fontWeight: '600',
        },
        meta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        file: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            backgroundColor: color.neutral._300,
            paddingHorizontal: spacing.m,
            paddingVertical: spacing.sm,
        },
        status: {
            width: 14,
            color: color.text._400,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        path: {
            flex: 1,
            color: color.text._100,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        counts: {
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        diff: {
            backgroundColor: '#0a0a0b',
            paddingHorizontal: spacing.m,
            paddingVertical: spacing.s,
            marginBottom: spacing.s,
        },
        hunkHead: {
            color: color.text._500,
            fontFamily: 'monospace',
            fontSize: 11,
            marginTop: spacing.s,
        },
        line: {
            color: '#dcdcda',
            fontFamily: 'monospace',
            fontSize: 11,
            lineHeight: 15,
        },
        added: {
            color: '#2ec469',
        },
        removed: {
            color: color.error._200,
        },
        note: {
            color: color.text._400,
        },
        problem: {
            color: color.error._300,
        },
    })
}
