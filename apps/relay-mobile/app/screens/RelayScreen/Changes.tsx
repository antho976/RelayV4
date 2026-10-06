import { useLocalSearchParams } from 'expo-router'
import React, { useEffect, useRef, useState } from 'react'
import { Linking, StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    ErrorState,
    Field,
    relay,
    Row,
    Screen,
    Section,
    Sheet,
    SwitchRow,
    useBusQuery,
    useRelayStore,
} from '@components/relay'
import {
    afterSheet,
    DiffBlock,
    DiffFile,
    DiffFileRow,
    FileStatus,
    GitStatus,
    Hunk,
    problemText,
    PullRequest,
    useGuardedAction,
    WorktreePicker,
} from '@components/relay/git'
import { useSheetStyles } from '@components/relay/Sheet'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type Opened = { key: string; hunks?: Hunk[]; error?: string }

const staged = (file: FileStatus) => !!file.index.trim() && file.index !== '?'
const unstaged = (file: FileStatus) => !!file.worktree.trim()

/**
 * What changed in one worktree, and the way out of it: stage and unstage, commit, push, open a
 * pull request. Reached per session (`session`: its worktree) from the terminal and the board,
 * or per project (`project_id`, optional `worktree`) with a worktree picker.
 * Desktop: relay-native code_git.rs, the Git panel's changes and publish steps.
 */
const ChangesScreen = () => {
    const styles = useStyles()
    const params = useLocalSearchParams<{
        session?: string
        project_id?: string
        worktree?: string
    }>()
    const name = typeof params.session === 'string' ? params.session : ''
    const session = useRelayStore((state) =>
        name ? state.sessions.find((item) => item.name === name) : undefined
    )
    const parsed = params.project_id ? Number(params.project_id) : NaN
    const projectId = session?.project_id ?? (Number.isFinite(parsed) ? parsed : undefined)
    const [picked, setPicked] = useState<string | undefined>(
        typeof params.worktree === 'string' && params.worktree ? params.worktree : undefined
    )
    const worktree = session ? session.worktree : picked
    const ready = projectId !== undefined && !!worktree
    const scope = { project_id: projectId ?? 0, worktree: worktree ?? '' }

    const status = useBusQuery<GitStatus>('git.status', scope, {
        enabled: ready,
        events: ['git.*', 'file.changed', 'worktree.changed'],
        projectId: projectId,
    })
    const diff = useBusQuery<DiffFile[]>('git.diff', scope, {
        enabled: ready,
        events: ['git.*', 'file.changed', 'worktree.changed'],
        projectId: projectId,
        select: (raw) => raw.files,
    })
    // `gh` is slow and may be missing; only on open and after this screen's own push or PR.
    const prs = useBusQuery<{ pull_requests: PullRequest[]; complete: boolean }>(
        'git.pr.list',
        { project_id: projectId ?? 0 },
        { enabled: projectId !== undefined, refetchOnFocus: false }
    )
    const { busy, run } = useGuardedAction()
    const [opened, setOpened] = useState<Opened | undefined>(undefined)
    const wanted = useRef<string | undefined>(undefined)
    const [committing, setCommitting] = useState(false)
    const [prSheet, setPrSheet] = useState(false)

    const git = status.data
    const files = git?.files ?? []
    const counts = new Map((diff.data ?? []).map((file) => [file.path, file]))
    const stagedFiles = files.filter(staged)
    const unstagedFiles = files.filter(unstaged)
    const branch = git?.branch ?? ''
    const pr = (() => {
        const mine = (prs.data?.pull_requests ?? []).filter(
            (item) => item.same_repository && item.branch === branch
        )
        return mine.find((item) => item.state === 'open') ?? mine[0]
    })()

    const reload = () => {
        status.reload()
        diff.reload()
    }

    const toggle = async (key: string, path: string, binary: boolean) => {
        if (opened?.key === key) {
            wanted.current = undefined
            setOpened(undefined)
            return
        }
        wanted.current = key
        setOpened({ key })
        if (binary || projectId === undefined) return
        try {
            const result = await relay.call<{ hunks: Hunk[] }>('git.diff.file', {
                project_id: projectId,
                worktree: worktree,
                path: path,
            })
            if (wanted.current === key) setOpened({ key, hunks: result.hunks })
        } catch (e) {
            if (wanted.current === key) setOpened({ key, error: problemText(e) })
        }
    }

    const stage = (op: 'git.stage' | 'git.unstage', paths: string[]) => {
        if (paths.length === 0 || projectId === undefined) return
        run(op, { project_id: projectId, worktree: worktree, paths: paths }).then(reload)
    }

    const fetch = async () => {
        if (projectId === undefined) return
        const result = await run<{ ahead: number | null; behind: number | null }>(
            'git.fetch',
            { project_id: projectId },
            (out) =>
                out.ahead === null || out.behind === null
                    ? 'Fetched'
                    : `Fetched · ${out.ahead} ahead · ${out.behind} behind`
        )
        if (result) reload()
    }

    const push = async () => {
        if (projectId === undefined || !git) return
        const publish = !git.upstream
        const yes = await confirm({
            title: publish ? 'Publish this branch?' : 'Push this branch?',
            message: publish
                ? `Push ${branch} to origin and set it as the upstream.`
                : `Push ${git.ahead ?? 0} commit${git.ahead === 1 ? '' : 's'} on ${branch} to ${git.upstream}.` +
                  ((git.behind ?? 0) > 0
                      ? ` The remote is ${git.behind} ahead; the push will likely be refused until you pull on the PC.`
                      : ''),
            confirmLabel: publish ? 'Publish' : 'Push',
        })
        if (!yes) return
        const done = await run(
            'git.push',
            publish
                ? { project_id: projectId, worktree: worktree, set_upstream: true }
                : { project_id: projectId, worktree: worktree },
            publish ? `Published ${branch}` : `Pushed ${branch}`
        )
        if (done) {
            reload()
            prs.reload()
        }
    }

    const openPr = async (title: string, body: string) => {
        if (projectId === undefined) return
        const yes = await confirm({
            title: 'Open a pull request?',
            message:
                `GitHub CLI on the PC publishes a pull request for ${branch}.` +
                (title ? '' : ' Title and body are filled from the commits.'),
            confirmLabel: 'Open PR',
        })
        if (!yes) return
        const payload: Record<string, unknown> = { project_id: projectId, worktree: worktree }
        if (title) payload.title = title
        if (body) payload.body = body
        const result = await run<{ url: string }>('git.pr.open', payload, 'Pull request opened')
        if (result?.url) {
            prs.reload()
            Linking.openURL(result.url).catch(() => Logger.infoToast(result.url))
        }
    }

    const upstreamText = !git
        ? ''
        : !git.upstream
          ? 'Not on the remote yet'
          : git.ahead === null || git.behind === null
            ? `${git.upstream} · status unknown, fetch to refresh`
            : git.ahead === 0 && git.behind === 0
              ? `Up to date with ${git.upstream}`
              : `${git.ahead} ahead · ${git.behind} behind ${git.upstream}`
    const canPush = !!git && (!git.upstream || ((git.ahead ?? 0) > 0 && (git.behind ?? 0) === 0))
    const prText = pr
        ? `#${pr.number} · ${pr.draft && pr.state === 'open' ? 'draft' : pr.state} · ${pr.title}`
        : prs.error
          ? 'GitHub unavailable · PR status unknown'
          : !prs.data
            ? 'Looking up pull requests…'
            : !git?.upstream
              ? 'No PR · push the branch first'
              : 'No PR for this branch'

    const fileRow = (file: FileStatus, group: 'staged' | 'unstaged') => {
        // The worktree is part of the key: another worktree is another page, nothing open.
        const key = `${worktree}|${group}:${file.path}`
        const count = counts.get(file.path)
        const letter = group === 'staged' ? file.index : file.worktree
        const untracked = file.worktree === '?'
        return (
            <DiffFileRow
                key={key}
                path={file.path}
                oldPath={file.renamed_from}
                status={letter}
                added={count?.added}
                removed={count?.removed}
                binary={count?.binary}
                meta={untracked && !count ? 'new' : undefined}
                open={opened?.key === key}
                onPress={() => toggle(key, file.path, !!count?.binary)}
                right={
                    <TouchableOpacity
                        hitSlop={8}
                        disabled={!!busy}
                        accessibilityLabel={group === 'staged' ? 'Unstage file' : 'Stage file'}
                        onPress={() =>
                            stage(group === 'staged' ? 'git.unstage' : 'git.stage', [file.path])
                        }
                        style={styles.stageKey}>
                        <Text style={styles.stageText}>{group === 'staged' ? '−' : '+'}</Text>
                    </TouchableOpacity>
                }>
                <DiffBlock
                    hunks={opened?.hunks}
                    error={opened?.error}
                    binary={count?.binary}
                    empty={untracked ? 'Empty file.' : 'No textual changes.'}
                />
            </DiffFileRow>
        )
    }

    const title = name ? `${name} · changes` : 'Changes'
    return (
        <Screen
            title={title}
            onRefresh={() => {
                reload()
                prs.reload()
            }}
            refreshing={status.loading && !!git}
            actions={[
                {
                    icon: 'cloud-download',
                    label: 'Fetch',
                    disabled: !!busy || projectId === undefined,
                    onPress: fetch,
                },
            ]}>
            {name && !session && (
                <EmptyState
                    icon="question-circle"
                    title="Session not found"
                    text="It is not open on the connected PC."
                />
            )}
            {!name && projectId === undefined && (
                <EmptyState icon="folder" title="No project" text="Open this from a project." />
            )}
            {!session && projectId !== undefined && (
                <WorktreePicker projectId={projectId} value={picked} onChange={setPicked} />
            )}
            {status.error && !git && <ErrorState error={status.error} onRetry={reload} />}
            {git && (
                <View style={styles.strip}>
                    <Text numberOfLines={1} style={styles.branch}>
                        {branch || 'detached HEAD'}
                    </Text>
                    <Text style={styles.meta}>
                        {git.upstream
                            ? `↑${git.ahead ?? '?'} ↓${git.behind ?? '?'}`
                            : 'no upstream'}
                    </Text>
                    {pr && (
                        <Chip
                            label={`PR #${pr.number}`}
                            icon="pull-request"
                            tone={pr.state === 'open' ? (pr.draft ? 'warn' : 'live') : 'neutral'}
                            onPress={() => Linking.openURL(pr.url)}
                        />
                    )}
                </View>
            )}
            {git && (
                <Section title="Publish">
                    <Row
                        label={
                            files.length === 0
                                ? 'Nothing to commit'
                                : `Commit… · ${stagedFiles.length} staged of ${files.length}`
                        }
                        detail="Message suggested from the diff"
                        icon="check-circle"
                        disabled={files.length === 0 || !!busy}
                        onPress={() => setCommitting(true)}
                    />
                    <Row
                        label={
                            busy === 'git.push'
                                ? 'Pushing…'
                                : !git.upstream
                                  ? 'Publish branch'
                                  : (git.ahead ?? 0) > 0
                                    ? `Push ${git.ahead}`
                                    : 'Push'
                        }
                        detail={upstreamText}
                        icon="cloud-upload"
                        disabled={!canPush || !!busy}
                        onPress={push}
                    />
                    <Row
                        label={
                            busy === 'git.pr.open'
                                ? 'Opening…'
                                : pr
                                  ? 'View pull request'
                                  : 'Open pull request…'
                        }
                        detail={prText}
                        icon="pull-request"
                        disabled={!!busy || (!pr && !git.upstream)}
                        onPress={() => (pr ? Linking.openURL(pr.url) : setPrSheet(true))}
                    />
                </Section>
            )}
            {git && files.length === 0 && (
                <EmptyState icon="check" title="Clean" text="Nothing changed in this worktree." />
            )}
            {stagedFiles.length > 0 && (
                <Section
                    title={`Staged · ${stagedFiles.length}`}
                    card={false}
                    action={{
                        label: 'Unstage all',
                        onPress: () =>
                            stage(
                                'git.unstage',
                                stagedFiles.map((file) => file.path)
                            ),
                    }}>
                    <View style={styles.list}>
                        {stagedFiles.map((file) => fileRow(file, 'staged'))}
                    </View>
                </Section>
            )}
            {unstagedFiles.length > 0 && (
                <Section
                    title={`Changes · ${unstagedFiles.length}`}
                    card={false}
                    action={{
                        label: 'Stage all',
                        onPress: () =>
                            stage(
                                'git.stage',
                                unstagedFiles.map((file) => file.path)
                            ),
                    }}>
                    <View style={styles.list}>
                        {unstagedFiles.map((file) => fileRow(file, 'unstaged'))}
                    </View>
                </Section>
            )}
            {files.length > 0 && (
                <Text style={styles.meta}>
                    A diff shows the last commit against the working tree, staged or not.
                </Text>
            )}
            <CommitSheet
                visible={committing}
                projectId={projectId}
                worktree={worktree}
                stagedCount={stagedFiles.length}
                changedCount={files.length}
                onDismiss={() => setCommitting(false)}
                onCommitted={reload}
            />
            <PrSheet
                visible={prSheet}
                branch={branch}
                onDismiss={() => setPrSheet(false)}
                onSubmit={(prTitle, body) => {
                    setPrSheet(false)
                    afterSheet(() => openPr(prTitle, body))
                }}
            />
        </Screen>
    )
}

export default ChangesScreen

/** The commit message, prefilled from `git.suggest_message`, and whether to commit everything. */
const CommitSheet: React.FC<{
    visible: boolean
    projectId: number | undefined
    worktree: string | undefined
    stagedCount: number
    changedCount: number
    onDismiss: () => void
    onCommitted: () => void
}> = ({ visible, projectId, worktree, stagedCount, changedCount, onDismiss, onCommitted }) => {
    const styles = useSheetStyles()
    const [message, setMessage] = useState('')
    const [all, setAll] = useState(stagedCount === 0)
    const { busy, run } = useGuardedAction()
    const touched = useRef(false)
    // Each opening starts from "everything" when nothing is staged; the counts may move under an
    // open sheet without resetting the person's choice.
    const [shown, setShown] = useState(false)
    if (visible !== shown) {
        setShown(visible)
        if (visible) setAll(stagedCount === 0)
    }

    useEffect(() => {
        if (!visible || projectId === undefined || touched.current) return
        let live = true
        relay
            .call<{ message: string }>('git.suggest_message', {
                project_id: projectId,
                worktree: worktree,
            })
            .then((result) => {
                if (live && !touched.current) setMessage(result.message)
            })
            .catch(() => {})
        return () => {
            live = false
        }
    }, [visible, projectId, worktree])

    const commit = async () => {
        const text = message.trim()
        if (!text || projectId === undefined) return
        const result = await run<{ sha: string }>(
            'git.commit',
            { project_id: projectId, worktree: worktree, message: text, all: all },
            (out) => `Committed ${out.sha.slice(0, 7)}`
        )
        if (result) {
            touched.current = false
            setMessage('')
            onDismiss()
            onCommitted()
        }
    }

    const nothing = all ? changedCount === 0 : stagedCount === 0
    return (
        <Sheet visible={visible} onDismiss={onDismiss}>
            <View style={styles.body}>
                <Text style={styles.title}>Commit</Text>
                <Field
                    label="Message"
                    value={message}
                    onChangeText={(text) => {
                        touched.current = true
                        setMessage(text)
                    }}
                    multiline
                    lines={4}
                    mono
                />
                <SwitchRow
                    label="Commit all changes"
                    description={
                        all
                            ? `Stages everything first, new files included (${changedCount}).`
                            : `Only what is staged (${stagedCount}).`
                    }
                    value={all}
                    onChange={setAll}
                />
                <View style={styles.actions}>
                    <ThemedButton label="Cancel" variant="secondary" onPress={onDismiss} />
                    <ThemedButton
                        label={busy ? 'Committing…' : 'Commit'}
                        variant={message.trim() && !nothing && !busy ? 'primary' : 'disabled'}
                        onPress={commit}
                    />
                </View>
            </View>
        </Sheet>
    )
}

/** Optional title and body for `git.pr.open`; blank lets `gh --fill` write them. */
const PrSheet: React.FC<{
    visible: boolean
    branch: string
    onDismiss: () => void
    onSubmit: (title: string, body: string) => void
}> = ({ visible, branch, onDismiss, onSubmit }) => {
    const styles = useSheetStyles()
    const [title, setTitle] = useState('')
    const [body, setBody] = useState('')
    return (
        <Sheet visible={visible} onDismiss={onDismiss}>
            <View style={styles.body}>
                <Text style={styles.title}>Pull request</Text>
                <Text style={styles.message}>
                    For {branch}. Leave both blank to fill them from the commits.
                </Text>
                <Field label="Title" value={title} onChangeText={setTitle} />
                <Field label="Body" value={body} onChangeText={setBody} multiline lines={4} />
                <View style={styles.actions}>
                    <ThemedButton label="Cancel" variant="secondary" onPress={onDismiss} />
                    <ThemedButton
                        label="Continue"
                        onPress={() => onSubmit(title.trim(), body.trim())}
                    />
                </View>
            </View>
        </Sheet>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        strip: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.m,
            minHeight: 34,
            borderRadius: 8,
            backgroundColor: color.neutral._200,
        },
        branch: {
            flexShrink: 1,
            color: color.text._100,
            fontFamily: 'monospace',
            fontWeight: '600',
        },
        meta: {
            flex: 1,
            color: color.text._400,
            fontSize: fontSize.s,
        },
        list: {
            rowGap: 2,
        },
        stageKey: {
            width: 30,
            height: 26,
            alignItems: 'center',
            justifyContent: 'center',
            borderRadius: 6,
            borderWidth: 1,
            borderColor: color.neutral._500,
        },
        stageText: {
            color: color.text._200,
            fontSize: fontSize.l,
            lineHeight: fontSize.l + 2,
        },
    })
}
