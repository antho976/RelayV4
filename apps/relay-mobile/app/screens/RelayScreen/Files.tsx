import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useEffect, useState } from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    ErrorState,
    Field,
    LoadingState,
    relay,
    Screen,
    Section,
    useBusQuery,
    useProjectParam,
    useRelayEvent,
} from '@components/relay'
import {
    ActionSheet,
    Entry,
    formatBytes,
    PromptSheet,
    problemText,
    SheetAction,
    useGuardedAction,
    WorktreePicker,
} from '@components/relay/git'
import { Theme } from '@lib/theme/ThemeManager'

type Hit = { path: string; line: number; col: number; text: string }
type Folder = { entries?: Entry[]; error?: string }
type Prompt =
    | { kind: 'create'; into: string; entry: 'file' | 'dir' }
    | { kind: 'rename'; entry: Entry }

/** Git badges the engine sets on tree entries, as words for the long-press sheet. */
const BADGE: Record<string, string> = { M: 'modified', A: 'added', D: 'deleted', '?': 'untracked' }

/**
 * A worktree's files: a tree that loads one folder at a time with git badges, content search,
 * and the file operations the desktop has — new file or folder, rename, trash with Undo,
 * revert to HEAD. A file opens in FileView. Params: `project_id`, optional `worktree`.
 * Desktop: relay-native editor.rs and project_files.rs.
 */
const FilesScreen = () => {
    const router = useRouter()
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const { projectId, project } = useProjectParam()
    const params = useLocalSearchParams<{ worktree?: string }>()
    const [worktree, setWorktree] = useState<string | undefined>(
        typeof params.worktree === 'string' && params.worktree ? params.worktree : undefined
    )
    const [open, setOpen] = useState<Record<string, Folder>>({})
    const [menu, setMenu] = useState<{ title: string; actions: SheetAction[] } | undefined>(
        undefined
    )
    const [prompt, setPrompt] = useState<Prompt | undefined>(undefined)
    const [undo, setUndo] = useState<{ trash_id: number; path: string } | undefined>(undefined)
    const [query, setQuery] = useState('')
    const [glob, setGlob] = useState('')
    const [regex, setRegex] = useState(false)
    const [search, setSearch] = useState<{ hits?: Hit[]; error?: string; loading?: boolean }>()
    const { busy, run } = useGuardedAction()
    const ready = projectId !== undefined && !!worktree
    const scope = { project_id: projectId ?? 0, worktree: worktree ?? '' }

    const root = useBusQuery<Entry[]>(
        'file.tree',
        { ...scope, path: '', depth: 1, git_badges: true },
        {
            enabled: ready,
            events: ['file.changed', 'git.changed'],
            projectId: projectId,
            select: (raw) => raw.entries,
        }
    )

    // Another worktree is another tree.
    useEffect(() => {
        setOpen({})
        setSearch(undefined)
    }, [worktree])

    // The Undo offer lasts ten seconds.
    useEffect(() => {
        if (!undo) return
        const timer = setTimeout(() => setUndo(undefined), 10_000)
        return () => clearTimeout(timer)
    }, [undo])

    const loadFolder = async (path: string) => {
        try {
            const result = await relay.call<{ entries: Entry[] }>('file.tree', {
                ...scope,
                path: path,
                depth: 1,
                git_badges: true,
            })
            setOpen((now) => (path in now ? { ...now, [path]: { entries: result.entries } } : now))
        } catch (e) {
            setOpen((now) => (path in now ? { ...now, [path]: { error: problemText(e) } } : now))
        }
    }

    // Open folders follow the disk like the root does.
    useRelayEvent(
        ['file.changed', 'git.changed'],
        () => Object.keys(open).forEach((path) => loadFolder(path)),
        { projectId: projectId, debounceMs: 400, enabled: ready }
    )

    if (projectId === undefined) {
        return (
            <Screen title="Files">
                <EmptyState icon="folder" title="No project" text="Open Files from a project." />
            </Screen>
        )
    }

    const toggle = (entry: Entry) => {
        if (open[entry.path]) {
            setOpen((now) => {
                const next = { ...now }
                // Closing a folder closes what is open inside it.
                for (const path of Object.keys(next))
                    if (path === entry.path || path.startsWith(`${entry.path}/`)) delete next[path]
                return next
            })
            return
        }
        setOpen((now) => ({ ...now, [entry.path]: {} }))
        loadFolder(entry.path)
    }

    const openFile = (path: string) =>
        router.push({
            pathname: '/screens/RelayScreen/FileView',
            params: { project_id: String(projectId), worktree: worktree ?? '', path: path },
        })

    const refresh = () => {
        root.reload()
        Object.keys(open).forEach((path) => loadFolder(path))
    }

    const remove = async (entry: Entry) => {
        const yes = await confirm({
            title: `Move ${entry.name} to the trash?`,
            message:
                entry.kind === 'dir'
                    ? 'The folder and everything in it go to the project trash (.relay/trash); Undo brings them back.'
                    : 'It goes to the project trash (.relay/trash); Undo brings it back.',
            confirmLabel: 'Move to trash',
            destructive: true,
        })
        if (!yes) return
        const result = await run<{ trash_id: number }>('file.delete', {
            ...scope,
            path: entry.path,
        })
        if (result) {
            setUndo({ trash_id: result.trash_id, path: entry.path })
            refresh()
        }
    }

    const restoreHead = async (entry: Entry) => {
        const yes = await confirm({
            title: `Revert ${entry.name} to HEAD?`,
            message: 'Throws away every uncommitted change to this file. This cannot be undone.',
            confirmLabel: 'Revert',
            destructive: true,
        })
        if (!yes) return
        const done = await run(
            'file.restore_head',
            { ...scope, path: entry.path },
            `Reverted ${entry.name}`
        )
        if (done) refresh()
    }

    const restore = async () => {
        if (!undo) return
        const back = undo
        setUndo(undefined)
        const done = await run(
            'file.restore',
            { project_id: projectId, trash_id: back.trash_id },
            `Restored ${back.path}`
        )
        if (done) refresh()
    }

    const newIn = (into: string) =>
        setMenu({
            title: into ? `New in ${into}` : 'New in the worktree root',
            actions: [
                {
                    label: 'New file',
                    icon: 'file-add',
                    onPress: () => setPrompt({ kind: 'create', into, entry: 'file' }),
                },
                {
                    label: 'New folder',
                    icon: 'folder-add',
                    onPress: () => setPrompt({ kind: 'create', into, entry: 'dir' }),
                },
            ],
        })

    const entryMenu = (entry: Entry) => {
        const actions: SheetAction[] = []
        if (entry.kind === 'dir') {
            actions.push(
                {
                    label: 'New file here',
                    icon: 'file-add',
                    onPress: () => setPrompt({ kind: 'create', into: entry.path, entry: 'file' }),
                },
                {
                    label: 'New folder here',
                    icon: 'folder-add',
                    onPress: () => setPrompt({ kind: 'create', into: entry.path, entry: 'dir' }),
                }
            )
        } else {
            actions.push({ label: 'Open', icon: 'file-text', onPress: () => openFile(entry.path) })
        }
        actions.push({
            label: 'Rename',
            icon: 'edit',
            onPress: () => setPrompt({ kind: 'rename', entry }),
        })
        if (entry.kind !== 'dir' && (entry.badge === 'M' || entry.badge === 'D'))
            actions.push({
                label: 'Revert to HEAD',
                icon: 'rollback',
                destructive: true,
                onPress: () => restoreHead(entry),
            })
        actions.push({
            label: 'Move to trash',
            icon: 'delete',
            destructive: true,
            onPress: () => remove(entry),
        })
        setMenu({ title: entry.path, actions })
    }

    const submitPrompt = async (value: string) => {
        if (!prompt) return false
        if (prompt.kind === 'create') {
            const path = prompt.into ? `${prompt.into}/${value}` : value
            const made = await run<Entry>(
                'file.create',
                { ...scope, path: path, kind: prompt.entry },
                `Created ${path}`
            )
            if (!made) return false
            if (prompt.into && !open[prompt.into]) setOpen((now) => ({ ...now, [prompt.into]: {} }))
            if (prompt.into) loadFolder(prompt.into)
            root.reload()
            if (prompt.entry === 'file') openFile(made.path)
            return true
        }
        const renamed = await run<Entry>(
            'file.rename',
            { ...scope, path: prompt.entry.path, new_name: value },
            (out) => `Renamed to ${out.name}`
        )
        if (renamed) refresh()
        return !!renamed
    }

    const runSearch = async () => {
        const text = query.trim()
        if (!text || !ready) return
        setSearch({ loading: true })
        const payload: Record<string, unknown> = { ...scope, query: text, limit: 200 }
        if (glob.trim()) payload.glob = glob.trim()
        if (regex) payload.regex = true
        try {
            const result = await relay.call<{ hits: Hit[] }>('file.search', payload)
            setSearch({ hits: result.hits })
        } catch (e) {
            setSearch({ error: problemText(e) })
        }
    }

    const rows = (entries: Entry[], depth: number): React.ReactNode[] =>
        entries.flatMap((entry) => {
            const folder = open[entry.path]
            const isDir = entry.kind === 'dir'
            const row = (
                <TouchableOpacity
                    key={entry.path}
                    style={[styles.entry, { paddingLeft: 8 + depth * 16 }]}
                    onPress={() => (isDir ? toggle(entry) : openFile(entry.path))}
                    onLongPress={() => entryMenu(entry)}>
                    <AntDesign
                        name={isDir ? (folder ? 'folder-open' : 'folder') : 'file'}
                        size={15}
                        color={isDir ? color.primary._700 : color.text._400}
                    />
                    <Text
                        numberOfLines={1}
                        style={[styles.name, entry.badge === 'D' && styles.deleted]}>
                        {entry.name}
                    </Text>
                    {!isDir && <Text style={styles.size}>{formatBytes(entry.size)}</Text>}
                    {!!entry.badge && (
                        <Text
                            style={[
                                styles.badge,
                                entry.badge === '?' || entry.badge === 'A'
                                    ? styles.badgeNew
                                    : entry.badge === 'D'
                                      ? styles.deleted
                                      : styles.badgeModified,
                            ]}>
                            {entry.badge}
                        </Text>
                    )}
                </TouchableOpacity>
            )
            if (!isDir || !folder) return [row]
            const inside = folder.error ? (
                <Text
                    key={`${entry.path}/!`}
                    style={[styles.problem, { paddingLeft: 24 + depth * 16 }]}>
                    {folder.error}
                </Text>
            ) : !folder.entries ? (
                <Text
                    key={`${entry.path}/…`}
                    style={[styles.note, { paddingLeft: 24 + depth * 16 }]}>
                    Loading…
                </Text>
            ) : folder.entries.length === 0 ? (
                <Text
                    key={`${entry.path}/∅`}
                    style={[styles.note, { paddingLeft: 24 + depth * 16 }]}>
                    Empty folder
                </Text>
            ) : null
            return [row, ...(inside ? [inside] : rows(folder.entries ?? [], depth + 1))]
        })

    return (
        <Screen
            title={project ? `${project.name} · files` : 'Files'}
            onRefresh={refresh}
            refreshing={root.loading && !!root.data}
            actions={[
                {
                    icon: 'plus',
                    label: 'New file or folder',
                    disabled: !ready,
                    onPress: () => newIn(''),
                },
            ]}
            footer={
                undo ? (
                    <View style={styles.undo}>
                        <Text numberOfLines={1} style={styles.undoText}>
                            Moved {undo.path} to the trash
                        </Text>
                        <ThemedButton label="Undo" variant="secondary" onPress={restore} />
                    </View>
                ) : undefined
            }>
            <WorktreePicker projectId={projectId} value={worktree} onChange={setWorktree} />
            <Section>
                <Field
                    value={query}
                    onChangeText={(text) => {
                        setQuery(text)
                        if (!text) setSearch(undefined)
                    }}
                    placeholder="Search file contents"
                    autoCapitalize="none"
                    autoCorrect={false}
                    returnKeyType="search"
                    onSubmitEditing={runSearch}
                />
                <View style={styles.searchOptions}>
                    <View style={styles.glob}>
                        <Field
                            value={glob}
                            onChangeText={setGlob}
                            placeholder="Glob, e.g. *.rs"
                            autoCapitalize="none"
                            autoCorrect={false}
                            mono
                            onSubmitEditing={runSearch}
                        />
                    </View>
                    <Chip
                        label="Regex"
                        icon="code"
                        selected={regex}
                        tone={regex ? 'primary' : 'neutral'}
                        onPress={() => setRegex((on) => !on)}
                    />
                    <Chip label="Search" icon="search" tone="primary" onPress={runSearch} />
                </View>
            </Section>
            {search ? (
                <Section
                    title={
                        search.hits
                            ? `${search.hits.length}${search.hits.length >= 200 ? '+' : ''} matches`
                            : 'Search'
                    }
                    action={{ label: 'Close', onPress: () => setSearch(undefined) }}>
                    {search.loading && <LoadingState />}
                    {!!search.error && <Text style={styles.problem}>{search.error}</Text>}
                    {search.hits?.length === 0 && <Text style={styles.note}>No matches.</Text>}
                    {search.hits?.map((hit, index) => (
                        <TouchableOpacity
                            key={`${hit.path}:${hit.line}:${hit.col}:${index}`}
                            style={styles.hit}
                            onPress={() => openFile(hit.path)}>
                            <Text numberOfLines={1} style={styles.hitPath}>
                                {hit.path}:{hit.line}
                            </Text>
                            <Text numberOfLines={2} style={styles.hitText}>
                                {hit.text.trim()}
                            </Text>
                        </TouchableOpacity>
                    ))}
                </Section>
            ) : !ready ? null : root.data === undefined ? (
                root.error ? (
                    <ErrorState error={root.error} onRetry={root.reload} />
                ) : (
                    <LoadingState />
                )
            ) : root.data.length === 0 ? (
                <EmptyState icon="folder" title="Empty worktree" />
            ) : (
                <View>
                    {rows(root.data, 0)}
                    <Text style={styles.hint}>Long-press a file or folder for more.</Text>
                </View>
            )}
            <ActionSheet
                visible={!!menu}
                title={menu?.title ?? ''}
                detail={(() => {
                    const entry = menu && findEntry(root.data, open, menu.title)
                    return entry?.badge ? `Git: ${BADGE[entry.badge] ?? entry.badge}` : undefined
                })()}
                actions={menu?.actions ?? []}
                onDismiss={() => setMenu(undefined)}
            />
            <PromptSheet
                visible={!!prompt}
                title={
                    prompt?.kind === 'rename'
                        ? `Rename ${prompt.entry.name}`
                        : prompt?.entry === 'dir'
                          ? 'New folder'
                          : 'New file'
                }
                message={
                    prompt?.kind === 'create'
                        ? `In ${prompt.into || 'the worktree root'}.`
                        : undefined
                }
                label="Name"
                initial={prompt?.kind === 'rename' ? prompt.entry.name : ''}
                confirmLabel={prompt?.kind === 'rename' ? 'Rename' : 'Create'}
                busy={!!busy}
                onSubmit={submitPrompt}
                onDismiss={() => setPrompt(undefined)}
            />
        </Screen>
    )
}

export default FilesScreen

const findEntry = (
    rootEntries: Entry[] | undefined,
    open: Record<string, Folder>,
    path: string
): Entry | undefined => {
    const all = [
        ...(rootEntries ?? []),
        ...Object.values(open).flatMap((folder) => folder.entries ?? []),
    ]
    return all.find((entry) => entry.path === path)
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        entry: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            minHeight: 38,
            paddingRight: spacing.m,
            borderBottomWidth: StyleSheet.hairlineWidth,
            borderBottomColor: color.neutral._300,
        },
        name: {
            flex: 1,
            color: color.text._100,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        size: {
            color: color.text._500,
            fontSize: fontSize.s,
        },
        badge: {
            width: 16,
            textAlign: 'center',
            fontFamily: 'monospace',
            fontWeight: '700',
            fontSize: fontSize.s,
        },
        badgeModified: {
            color: color.quote,
        },
        badgeNew: {
            color: '#2ec469',
        },
        deleted: {
            color: color.error._300,
        },
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
            paddingVertical: spacing.s,
        },
        hint: {
            color: color.text._500,
            fontSize: fontSize.s,
            textAlign: 'center',
            marginTop: spacing.l,
        },
        problem: {
            color: color.error._300,
            fontSize: fontSize.s,
            paddingVertical: spacing.s,
        },
        searchOptions: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.s,
            paddingBottom: spacing.m,
        },
        glob: {
            flex: 1,
        },
        hit: {
            paddingVertical: spacing.m,
            rowGap: 2,
            borderBottomWidth: StyleSheet.hairlineWidth,
            borderBottomColor: color.neutral._300,
        },
        hitPath: {
            color: color.primary._700,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        hitText: {
            color: color.text._300,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        undo: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.xl,
            paddingVertical: spacing.m,
            backgroundColor: color.neutral._200,
        },
        undoText: {
            flex: 1,
            color: color.text._200,
        },
    })
}
