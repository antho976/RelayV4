import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    Field,
    QueryView,
    Row,
    Screen,
    Section,
    Segmented,
    Sheet,
    SwitchRow,
    useBusQuery,
    useProjectParam,
} from '@components/relay'
import {
    ActionSheet,
    Branch,
    Commit,
    shortSha,
    useGuardedAction,
    WorktreePicker,
} from '@components/relay/git'
import { useSheetStyles } from '@components/relay/Sheet'
import { Theme } from '@lib/theme/ThemeManager'

import { ago } from './console'

type Tab = 'history' | 'branches'

const PAGE = 50
/** git.log's own ceiling; it has no cursor, so "more" asks for a longer list. */
const MAX_LOG = 500

/**
 * A project's git beyond one diff: the working changes of a worktree (opens Changes), its
 * history with each commit's files, and branches — switch, create, delete, clean merged.
 * Params: `project_id`, optional `worktree`. Desktop: relay-native code_git.rs.
 */
const GitScreen = () => {
    const router = useRouter()
    const styles = useStyles()
    const { projectId, project } = useProjectParam()
    const params = useLocalSearchParams<{ worktree?: string }>()
    const [worktree, setWorktree] = useState<string | undefined>(
        typeof params.worktree === 'string' && params.worktree ? params.worktree : undefined
    )
    const [tab, setTab] = useState<Tab>('history')
    const [limit, setLimit] = useState(PAGE)
    // History of another branch than the worktree's own (git.log {branch}).
    const [logBranch, setLogBranch] = useState<string | undefined>(undefined)
    const [branchSheet, setBranchSheet] = useState<Branch | undefined>(undefined)
    const [creating, setCreating] = useState<{ start: string } | undefined>(undefined)
    const { busy, run } = useGuardedAction()
    const ready = projectId !== undefined && !!worktree
    const scope = { project_id: projectId ?? 0, worktree: worktree ?? '' }

    const log = useBusQuery<Commit[]>(
        'git.log',
        logBranch ? { ...scope, branch: logBranch, limit: limit } : { ...scope, limit: limit },
        {
            enabled: ready && tab === 'history',
            events: ['git.changed'],
            projectId: projectId,
            select: (raw) => raw.commits,
        }
    )
    const branches = useBusQuery<{ current: string; branches: Branch[] }>('git.branches', scope, {
        enabled: ready,
        events: ['git.changed', 'worktree.changed', 'session.*'],
        projectId: projectId,
    })

    if (projectId === undefined) {
        return (
            <Screen title="Git">
                <EmptyState icon="folder" title="No project" text="Open Git from a project." />
            </Screen>
        )
    }

    const openChanges = () =>
        router.push({
            pathname: '/screens/RelayScreen/Changes',
            params: { project_id: String(projectId), worktree: worktree ?? '' },
        })
    const openCommit = (sha: string) =>
        router.push({
            pathname: '/screens/RelayScreen/Commit',
            params: { project_id: String(projectId), sha: sha, worktree: worktree ?? '' },
        })

    const switchTo = async (branch: Branch) => {
        const yes = await confirm({
            title: `Switch to ${branch.name}?`,
            message:
                'The checkout must be clean and no live session may own it; the engine refuses otherwise.',
            confirmLabel: 'Switch',
        })
        if (!yes) return
        await run(
            'git.branch.switch',
            { project_id: projectId, worktree: worktree, name: branch.name },
            `On ${branch.name}`
        )
        branches.reload()
    }

    const remove = async (branch: Branch) => {
        const yes = await confirm({
            title: `Delete ${branch.name}?`,
            message:
                'Only a merged local branch that is not checked out and not owned by a session can be deleted.',
            confirmLabel: 'Delete',
            destructive: true,
        })
        if (!yes) return
        await run(
            'git.branch.delete',
            { project_id: projectId, name: branch.name },
            `Deleted ${branch.name}`
        )
        branches.reload()
    }

    const cleanMerged = async () => {
        const preview = await run<{ deleted: string[] }>('git.branch.clean_merged', {
            project_id: projectId,
            dry_run: true,
        })
        if (!preview) return
        if (preview.deleted.length === 0) {
            await confirm({
                title: 'Nothing to clean',
                message:
                    'Nothing to clean up: only merged relay/* branches of closed sessions are deleted, with their unused checkouts. Your own branches are kept.',
                confirmLabel: 'OK',
            })
            return
        }
        const yes = await confirm({
            title: `Delete ${preview.deleted.length} merged branch${preview.deleted.length === 1 ? '' : 'es'}?`,
            message: 'Merged relay/* branches of closed sessions, with their unused checkouts. Your own branches are kept.',
            confirmLabel: 'Delete',
            destructive: true,
            body: <Text style={styles.list}>{preview.deleted.join('\n')}</Text>,
        })
        if (!yes) return
        await run<{ deleted: string[] }>(
            'git.branch.clean_merged',
            { project_id: projectId, dry_run: false },
            (out) => `Deleted ${out.deleted.length}`
        )
        branches.reload()
    }

    const current = branches.data?.branches.find((item) => item.current)
    return (
        <Screen
            title={project ? `${project.name} · git` : 'Git'}
            onRefresh={() => {
                log.reload()
                branches.reload()
            }}
            refreshing={(log.loading && !!log.data) || (branches.loading && !!branches.data)}>
            <WorktreePicker projectId={projectId} value={worktree} onChange={setWorktree} />
            <Section>
                <Row
                    label="Changes"
                    detail={
                        current
                            ? `${current.name}${current.upstream ? ` · ↑${current.ahead ?? '?'} ↓${current.behind ?? '?'}` : ' · no upstream'} · stage, commit, push, PR`
                            : 'Stage, commit, push, pull request'
                    }
                    icon="diff"
                    disabled={!worktree}
                    onPress={openChanges}
                />
            </Section>
            <Segmented
                options={[
                    { value: 'history', label: 'History' },
                    { value: 'branches', label: 'Branches' },
                ]}
                value={tab}
                onChange={setTab}
            />
            {tab === 'history' && logBranch && (
                <View style={styles.filter}>
                    <Chip
                        label={`History of ${logBranch}  ✕`}
                        icon="branches"
                        tone="primary"
                        selected
                        onPress={() => setLogBranch(undefined)}
                    />
                </View>
            )}
            {tab === 'history' && (
                <QueryView
                    query={log}
                    isEmpty={(commits) => commits.length === 0}
                    empty={<EmptyState icon="history" title="No commits yet" />}>
                    {(commits) => (
                        <>
                            <Section>
                                {commits.map((commit) => (
                                    <Row
                                        key={commit.sha}
                                        label={commit.subject || '(no message)'}
                                        detail={`${shortSha(commit.sha)} · ${commit.author} · ${ago(commit.at)}${
                                            commit.refs.length ? ` · ${commit.refs.join(', ')}` : ''
                                        }`}
                                        detailLines={1}
                                        onPress={() => openCommit(commit.sha)}
                                    />
                                ))}
                            </Section>
                            {commits.length >= limit && limit < MAX_LOG && (
                                <ThemedButton
                                    label={log.loading ? 'Loading…' : 'Load more'}
                                    variant="secondary"
                                    onPress={() => setLimit((n) => Math.min(MAX_LOG, n + PAGE))}
                                />
                            )}
                        </>
                    )}
                </QueryView>
            )}
            {tab === 'branches' && (
                <QueryView query={branches}>
                    {(data) => (
                        <Section
                            title={`Branches · ${data.branches.length}`}
                            action={{
                                label: 'New branch',
                                onPress: () => setCreating({ start: '' }),
                            }}>
                            {data.branches.map((branch) => (
                                <Row
                                    key={branch.name}
                                    label={branch.name}
                                    icon={branch.current ? 'check-circle' : 'branches'}
                                    detail={[
                                        shortSha(branch.head),
                                        branch.upstream
                                            ? `${branch.upstream} ↑${branch.ahead ?? '?'} ↓${branch.behind ?? '?'}`
                                            : 'local',
                                        branch.session ? `session ${branch.session}` : '',
                                    ]
                                        .filter(Boolean)
                                        .join(' · ')}
                                    mono
                                    right={
                                        branch.current ? (
                                            <Chip label="current" tone="primary" />
                                        ) : branch.merged ? (
                                            <Chip label="merged" />
                                        ) : undefined
                                    }
                                    chevron={false}
                                    disabled={!!busy}
                                    onPress={() => setBranchSheet(branch)}
                                />
                            ))}
                        </Section>
                    )}
                </QueryView>
            )}
            {tab === 'branches' && (
                <Section title="Housekeeping">
                    <Row
                        label={
                            busy === 'git.branch.clean_merged'
                                ? 'Checking…'
                                : 'Clean merged branches'
                        }
                        detail="Preview first; keeps base, checked-out and session branches"
                        icon="delete"
                        disabled={!!busy}
                        onPress={cleanMerged}
                    />
                </Section>
            )}
            <ActionSheet
                visible={!!branchSheet}
                title={branchSheet?.name ?? ''}
                detail={
                    branchSheet
                        ? `${shortSha(branchSheet.head)}${branchSheet.merged ? ' · merged' : ''}${branchSheet.session ? ` · owned by ${branchSheet.session}` : ''}`
                        : undefined
                }
                onDismiss={() => setBranchSheet(undefined)}
                actions={
                    branchSheet
                        ? [
                              {
                                  label: 'Switch this worktree to it',
                                  icon: 'swap',
                                  disabled: branchSheet.current,
                                  onPress: () => switchTo(branchSheet),
                              },
                              {
                                  label: 'New branch from here',
                                  icon: 'plus',
                                  onPress: () => setCreating({ start: branchSheet.name }),
                              },
                              {
                                  label: 'History of this branch',
                                  icon: 'history',
                                  onPress: () => {
                                      setLogBranch(
                                          branchSheet.current ? undefined : branchSheet.name
                                      )
                                      setLimit(PAGE)
                                      setTab('history')
                                  },
                              },
                              {
                                  label: 'Delete',
                                  icon: 'delete',
                                  destructive: true,
                                  disabled: branchSheet.current || !!branchSheet.session,
                                  onPress: () => remove(branchSheet),
                              },
                          ]
                        : []
                }
            />
            <CreateBranchSheet
                visible={!!creating}
                start={creating?.start ?? ''}
                busy={busy === 'git.branch.create'}
                onDismiss={() => setCreating(undefined)}
                onSubmit={async (name, start, checkout) => {
                    const payload: Record<string, unknown> = {
                        project_id: projectId,
                        worktree: worktree,
                        name: name,
                        checkout: checkout,
                    }
                    if (start) payload.start_point = start
                    const done = await run<{ name: string; head: string }>(
                        'git.branch.create',
                        payload,
                        (out) => `Created ${out.name} at ${shortSha(out.head)}`
                    )
                    if (done) {
                        branches.reload()
                        log.reload()
                    }
                    return !!done
                }}
            />
        </Screen>
    )
}

export default GitScreen

const CreateBranchSheet: React.FC<{
    visible: boolean
    start: string
    busy: boolean
    onDismiss: () => void
    onSubmit: (name: string, start: string, checkout: boolean) => Promise<boolean>
}> = ({ visible, start, busy, onDismiss, onSubmit }) => {
    const styles = useSheetStyles()
    const [name, setName] = useState('')
    const [from, setFrom] = useState(start)
    const [checkout, setCheckout] = useState(true)
    const [shownFor, setShownFor] = useState<string | undefined>(undefined)
    // Reset when the sheet opens for a (possibly different) start point.
    const key = visible ? start : undefined
    if (key !== shownFor) {
        setShownFor(key)
        if (visible) {
            setName('')
            setFrom(start)
            setCheckout(true)
        }
    }
    const submit = async () => {
        if (!name.trim() || busy) return
        if (await onSubmit(name.trim(), from.trim(), checkout)) onDismiss()
    }
    return (
        <Sheet visible={visible} onDismiss={onDismiss}>
            <View style={styles.body}>
                <Text style={styles.title}>New branch</Text>
                <Field
                    label="Name"
                    value={name}
                    onChangeText={setName}
                    autoCapitalize="none"
                    autoCorrect={false}
                    mono
                />
                <Field
                    label="Start point"
                    value={from}
                    onChangeText={setFrom}
                    placeholder="HEAD"
                    autoCapitalize="none"
                    autoCorrect={false}
                    mono
                    description="A branch, tag or sha. Blank starts from the worktree's HEAD."
                />
                <SwitchRow
                    label="Check it out"
                    description="Switch this worktree to the new branch."
                    value={checkout}
                    onChange={setCheckout}
                />
                <View style={styles.actions}>
                    <ThemedButton label="Cancel" variant="secondary" onPress={onDismiss} />
                    <ThemedButton
                        label={busy ? 'Creating…' : 'Create'}
                        variant={name.trim() && !busy ? 'primary' : 'disabled'}
                        onPress={submit}
                    />
                </View>
            </View>
        </Sheet>
    )
}

const useStyles = () => {
    const { color, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        filter: {
            flexDirection: 'row',
        },
        list: {
            color: color.text._200,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
            lineHeight: 20,
        },
    })
}
