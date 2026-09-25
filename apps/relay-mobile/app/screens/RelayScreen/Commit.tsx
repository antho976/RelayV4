import { useLocalSearchParams } from 'expo-router'
import React, { useRef, useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import {
    Chip,
    EmptyState,
    Mono,
    QueryView,
    relay,
    Row,
    Screen,
    Section,
    useBusQuery,
} from '@components/relay'
import {
    Commit,
    DiffBlock,
    DiffFile,
    DiffFileRow,
    Hunk,
    problemText,
    shortSha,
    unifiedDiff,
} from '@components/relay/git'
import { Theme } from '@lib/theme/ThemeManager'

type Opened = { path: string; hunks?: Hunk[]; error?: string }

/** Past this size (both sides together) a commit's file is not diffed on the phone. */
const MAX_DIFF_CHARS = 1024 * 1024

/**
 * One commit: its message, author and the files it touched. A file's diff is the file at the
 * commit's parent against the file at the commit, both read with `git.diff.file {base}` and
 * compared here, since the engine has no per-commit file diff.
 * Params: `sha`, `project_id`, optional `worktree`.
 */
const CommitScreen = () => {
    const styles = useStyles()
    const params = useLocalSearchParams<{ sha?: string; project_id?: string; worktree?: string }>()
    const sha = typeof params.sha === 'string' ? params.sha : ''
    const parsed = params.project_id ? Number(params.project_id) : NaN
    const projectId = Number.isFinite(parsed) ? parsed : undefined
    const worktree =
        typeof params.worktree === 'string' && params.worktree ? params.worktree : undefined
    const query = useBusQuery<{ commit: Commit; files: DiffFile[] }>(
        'git.show',
        { project_id: projectId ?? 0, sha: sha },
        { enabled: projectId !== undefined && !!sha, refetchOnFocus: false }
    )
    const [opened, setOpened] = useState<Opened | undefined>(undefined)
    const wanted = useRef<string | undefined>(undefined)

    const textAt = async (revision: string, path: string) => {
        const payload: Record<string, unknown> = {
            project_id: projectId,
            path: path,
            base: revision,
        }
        if (worktree) payload.worktree = worktree
        const result = await relay.call<{ old: string }>('git.diff.file', payload)
        return result.old
    }

    const toggle = async (commit: Commit, file: DiffFile) => {
        if (opened?.path === file.path) {
            wanted.current = undefined
            setOpened(undefined)
            return
        }
        wanted.current = file.path
        setOpened({ path: file.path })
        if (file.binary) return
        try {
            const parent = commit.parents[0]
            const [before, after] = await Promise.all([
                parent ? textAt(parent, file.old_path ?? file.path) : Promise.resolve(''),
                textAt(commit.sha, file.path),
            ])
            if (wanted.current !== file.path) return
            if (before.length + after.length > MAX_DIFF_CHARS) {
                setOpened({ path: file.path, error: 'Too large to diff on the phone.' })
                return
            }
            setOpened({ path: file.path, hunks: unifiedDiff(before, after) })
        } catch (e) {
            if (wanted.current === file.path) setOpened({ path: file.path, error: problemText(e) })
        }
    }

    return (
        <Screen
            title={sha ? `Commit ${shortSha(sha)}` : 'Commit'}
            onRefresh={query.reload}
            refreshing={query.loading && !!query.data}>
            {!sha || projectId === undefined ? (
                <EmptyState
                    icon="question-circle"
                    title="No commit"
                    text="Open one from History."
                />
            ) : (
                <QueryView query={query}>
                    {({ commit, files }) => {
                        const added = files.reduce((sum, file) => sum + file.added, 0)
                        const removed = files.reduce((sum, file) => sum + file.removed, 0)
                        return (
                            <>
                                <View style={styles.head}>
                                    <Text selectable style={styles.subject}>
                                        {commit.subject}
                                    </Text>
                                    {!!commit.body.trim() && (
                                        <Text selectable style={styles.body}>
                                            {commit.body.trim()}
                                        </Text>
                                    )}
                                    {commit.refs.length > 0 && (
                                        <View style={styles.refs}>
                                            {commit.refs.map((ref) => (
                                                <Chip key={ref} label={ref} icon="tag" />
                                            ))}
                                        </View>
                                    )}
                                </View>
                                <Section>
                                    <Row label={commit.author} detail={commit.email} icon="user" />
                                    <Row
                                        label={new Date(commit.at).toLocaleString()}
                                        icon="clock-circle"
                                    />
                                    <Row label="Commit" detail={commit.sha} icon="number" mono />
                                    {commit.parents.length > 0 && (
                                        <Row
                                            label={commit.parents.length > 1 ? 'Parents' : 'Parent'}
                                            detail={commit.parents.map(shortSha).join('  ')}
                                            icon="branches"
                                            mono
                                        />
                                    )}
                                </Section>
                                <Section
                                    title={`Files · ${files.length} · +${added} -${removed}`}
                                    card={false}>
                                    {files.length === 0 ? (
                                        <Mono>No file changes (a merge or an empty commit).</Mono>
                                    ) : (
                                        <View style={styles.list}>
                                            {files.map((file) => (
                                                <DiffFileRow
                                                    key={file.path}
                                                    path={file.path}
                                                    oldPath={file.old_path}
                                                    status={file.status}
                                                    added={file.added}
                                                    removed={file.removed}
                                                    binary={file.binary}
                                                    open={opened?.path === file.path}
                                                    onPress={() => toggle(commit, file)}>
                                                    <DiffBlock
                                                        hunks={opened?.hunks}
                                                        error={opened?.error}
                                                        binary={file.binary}
                                                    />
                                                </DiffFileRow>
                                            ))}
                                        </View>
                                    )}
                                </Section>
                            </>
                        )
                    }}
                </QueryView>
            )}
        </Screen>
    )
}

export default CommitScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        head: {
            rowGap: spacing.m,
        },
        subject: {
            color: color.text._100,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
        body: {
            color: color.text._300,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        refs: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            gap: spacing.s,
        },
        list: {
            rowGap: 2,
        },
    })
}
