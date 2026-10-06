import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    ErrorState,
    Field,
    LoadingState,
    relay,
    relayHref,
    RelayProject,
    Row,
    Screen,
    Section,
    Segmented,
    useBusQuery,
    useRelayStore,
} from '@components/relay'
import { attempt, formatTime } from '@components/relay/settings/common'
import { RelayWorkspace } from '@lib/engine/Relay/RelayClient'
import { Theme } from '@lib/theme/ThemeManager'

type GitHubStatus = { installed: boolean; connected: boolean; login: string | null }
type GitHubRepo = {
    name: string
    full_name: string
    description: string | null
    clone_url: string
    ssh_url: string
    private: boolean
    archived: boolean
    updated_at: string | null
}
type Discovered = { path: string; repositories: { path: string; name: string }[] }
type Source = 'github' | 'local'

/** How many repositories the list shows before asking for a narrower search. */
const SHOWN = 60

/**
 * Add a project to the PC (the desktop's onboarding): pick or create the workspace it goes
 * in, then clone one of the account's GitHub repositories (or any URL) into it, or register a
 * Git repository that is already in the workspace's folder.
 */
const AddProjectScreen = () => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const router = useRouter()
    const workspaces = useRelayStore((state) => state.workspaces)
    const [workspaceId, setWorkspaceId] = useState<number | undefined>(undefined)
    const [creating, setCreating] = useState(false)
    const [source, setSource] = useState<Source>('github')
    // The first workspace is the usual answer; a new one becomes the pick once it exists.
    const chosen = workspaces.some((item) => item.id === workspaceId)
        ? workspaceId
        : workspaces[0]?.id
    const workspace = workspaces.find((item) => item.id === chosen)

    const opened = (project: RelayProject) => {
        relay.refreshProjects().catch(() => {})
        router.replace(relayHref('Project', { project_id: String(project.id) }))
    }

    return (
        <Screen title="Add a project">
            <Section title="Workspace">
                {workspaces.map((item) => (
                    <Row
                        key={item.id}
                        label={item.name}
                        detail={item.path}
                        mono
                        icon={item.id === chosen ? 'check-circle' : 'folder'}
                        chevron={false}
                        onPress={() => {
                            setWorkspaceId(item.id)
                            setCreating(false)
                        }}
                    />
                ))}
                <Row
                    label="New workspace"
                    detail="A folder on the PC that holds projects"
                    icon="folder-add"
                    chevron={false}
                    onPress={() => setCreating(!creating)}
                />
                {(creating || workspaces.length === 0) && (
                    <NewWorkspace
                        onCreated={(created) => {
                            setWorkspaceId(created.id)
                            setCreating(false)
                        }}
                    />
                )}
            </Section>

            {workspace && (
                <>
                    <Segmented
                        options={[
                            { value: 'github', label: 'From GitHub' },
                            { value: 'local', label: 'On this PC' },
                        ]}
                        value={source}
                        onChange={setSource}
                    />
                    <View style={{ rowGap: spacing.xl }}>
                        {source === 'github' ? (
                            <FromGitHub workspace={workspace} onAdded={opened} />
                        ) : (
                            <OnThisPc workspace={workspace} onAdded={opened} />
                        )}
                    </View>
                </>
            )}
            {!workspace && workspaces.length > 0 && (
                <Text style={styles.note}>Pick the workspace the project goes in.</Text>
            )}
        </Screen>
    )
}

export default AddProjectScreen

/** Register a folder on the PC as a workspace; the path is typed, as the phone cannot browse it. */
const NewWorkspace: React.FC<{ onCreated: (workspace: RelayWorkspace) => void }> = ({
    onCreated,
}) => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const [typed, setPath] = useState<string | undefined>(undefined)
    const [name, setName] = useState('')
    const [busy, setBusy] = useState(false)
    // The PC's suggestion (its usual projects folder), to start from.
    const suggested = useBusQuery<Discovered>('workspace.discover', {})
    const path = typed ?? suggested.data?.path ?? ''

    const create = async () => {
        setBusy(true)
        const created = await attempt(
            () =>
                relay.guarded<RelayWorkspace>('workspace.create', {
                    path: path.trim(),
                    ...(name.trim() ? { name: name.trim() } : {}),
                }),
            'Workspace added'
        )
        setBusy(false)
        if (!created) return
        await relay.refreshProjects().catch(() => {})
        onCreated(created)
    }

    const ready = path.trim().startsWith('/') && !busy
    return (
        <View style={{ rowGap: spacing.l, paddingVertical: spacing.l }}>
            <Text style={styles.note}>
                A workspace is a folder on the PC, not on this phone. Type its full path there (for
                example /home/you/code); the PC creates it if it does not exist yet.
            </Text>
            <Field
                label="Folder on the PC"
                value={path}
                onChangeText={setPath}
                autoCapitalize="none"
                autoCorrect={false}
                mono
                placeholder="/home/you/code"
            />
            <Field
                label="Name (optional)"
                value={name}
                onChangeText={setName}
                placeholder="The folder's name"
            />
            <View style={styles.actions}>
                <ThemedButton
                    label={busy ? 'Adding…' : 'Add workspace'}
                    variant={ready ? 'primary' : 'disabled'}
                    onPress={create}
                />
            </View>
        </View>
    )
}

type SourceProps = { workspace: RelayWorkspace; onAdded: (project: RelayProject) => void }

/** Clone a GitHub repository (or any Git URL) into the workspace with the PC's own git. */
const FromGitHub: React.FC<SourceProps> = ({ workspace, onAdded }) => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const status = useBusQuery<GitHubStatus>('github.status', {}, { events: ['github.*'] })
    const connected = !!status.data?.connected
    const repos = useBusQuery<GitHubRepo[]>(
        'github.repo.list',
        {},
        { enabled: connected, events: ['github.*'], select: (raw) => raw.repositories }
    )
    const [search, setSearch] = useState('')
    const [url, setUrl] = useState('')
    const [dest, setDest] = useState('')
    const [cloning, setCloning] = useState<string | undefined>(undefined)
    const [signingIn, setSigningIn] = useState(false)

    const clone = async (target: string, label: string) => {
        if (cloning) return
        const yes = await confirm({
            title: `Clone ${label}?`,
            message: `Into ${workspace.path}${dest.trim() ? `/${dest.trim()}` : ''}. A large repository can take several minutes; keep the app open until it finishes.`,
            confirmLabel: 'Clone',
        })
        if (!yes) return
        setCloning(target)
        const result = await attempt(
            () =>
                relay.guarded<{ project: RelayProject }>('project.clone', {
                    workspace_id: workspace.id,
                    url: target,
                    ...(dest.trim() ? { dest: dest.trim() } : {}),
                }),
            `Cloned ${label}`
        )
        setCloning(undefined)
        if (result) onAdded(result.project)
    }

    const signIn = async () => {
        setSigningIn(true)
        const started = await attempt(() => relay.guarded<{ started: boolean }>('github.connect'))
        if (!started) setSigningIn(false)
    }

    const needle = search.trim().toLowerCase()
    const matches = (repos.data ?? []).filter(
        (repo) =>
            !needle ||
            repo.full_name.toLowerCase().includes(needle) ||
            (repo.description ?? '').toLowerCase().includes(needle)
    )

    let account: React.ReactNode
    if (status.data === undefined) {
        account = status.error ? (
            <ErrorState error={status.error} onRetry={status.reload} />
        ) : (
            <LoadingState />
        )
    } else if (!status.data.installed) {
        account = (
            <EmptyState
                icon="github"
                title="GitHub CLI not installed"
                text="Install gh on the PC and sign in there to list your repositories. You can still clone any URL below."
            />
        )
    } else if (!connected) {
        account = (
            <EmptyState
                icon="github"
                title="Sign in on the PC"
                text={
                    signingIn
                        ? 'A browser opened on the PC. Finish signing in there; this updates by itself.'
                        : 'The PC is not signed in to GitHub. Signing in opens a browser on the PC, so do it at the PC. You can still clone any URL below.'
                }
                action={
                    signingIn ? undefined : { label: 'Start sign-in on the PC', onPress: signIn }
                }
            />
        )
    } else {
        account = (
            <Section title={`GitHub · ${status.data.login ?? 'signed in'}`}>
                <View style={{ paddingVertical: spacing.m }}>
                    <Field
                        value={search}
                        onChangeText={setSearch}
                        placeholder="Search repositories"
                        autoCapitalize="none"
                        autoCorrect={false}
                    />
                </View>
                {repos.data === undefined ? (
                    repos.error ? (
                        <ErrorState error={repos.error} onRetry={repos.reload} />
                    ) : (
                        <LoadingState label="Asking GitHub…" />
                    )
                ) : matches.length === 0 ? (
                    <Text style={[styles.note, { paddingVertical: spacing.l }]}>
                        No repository matches.
                    </Text>
                ) : (
                    matches
                        .slice(0, SHOWN)
                        .map((repo) => (
                            <Row
                                key={repo.full_name}
                                label={repo.full_name}
                                detail={
                                    [
                                        repo.description,
                                        repo.updated_at && `Updated ${formatTime(repo.updated_at)}`,
                                    ]
                                        .filter(Boolean)
                                        .join('\n') || undefined
                                }
                                icon={repo.private ? 'lock' : 'github'}
                                right={
                                    cloning === repo.clone_url ? (
                                        <Chip label="Cloning…" tone="live" />
                                    ) : repo.archived ? (
                                        <Chip label="Archived" />
                                    ) : undefined
                                }
                                disabled={!!cloning}
                                onPress={() => clone(repo.clone_url, repo.full_name)}
                            />
                        ))
                )}
                {matches.length > SHOWN && (
                    <Text style={[styles.note, { paddingVertical: spacing.l }]}>
                        {matches.length - SHOWN} more; search to narrow the list.
                    </Text>
                )}
            </Section>
        )
    }

    return (
        <>
            {account}
            <Section title="Clone options">
                <View style={{ rowGap: spacing.l, paddingVertical: spacing.l }}>
                    <Field
                        label="Folder name (optional)"
                        value={dest}
                        onChangeText={setDest}
                        autoCapitalize="none"
                        autoCorrect={false}
                        mono
                        placeholder="The repository's name"
                        description={`Created inside ${workspace.path}.`}
                    />
                    <Field
                        label="Or clone a URL"
                        value={url}
                        onChangeText={setUrl}
                        autoCapitalize="none"
                        autoCorrect={false}
                        keyboardType="url"
                        mono
                        placeholder="https://github.com/owner/repo.git"
                    />
                    {!!cloning && (
                        <LoadingState label="Cloning on the PC… this can take a few minutes." />
                    )}
                    <View style={styles.actions}>
                        <ThemedButton
                            label="Clone URL"
                            iconName="download"
                            variant={url.trim() && !cloning ? 'primary' : 'disabled'}
                            onPress={() => clone(url.trim(), url.trim())}
                        />
                    </View>
                </View>
            </Section>
        </>
    )
}

/** Register a Git repository already inside the workspace's folder. */
const OnThisPc: React.FC<SourceProps> = ({ workspace, onAdded }) => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const projects = useRelayStore((state) => state.projects)
    const found = useBusQuery<Discovered>('workspace.discover', { path: workspace.path })
    const [adding, setAdding] = useState<string | undefined>(undefined)
    const known = new Set(projects.map((item) => item.path))

    const add = async (repo: { path: string; name: string }) => {
        setAdding(repo.path)
        const project = await attempt(
            () =>
                relay.guarded<RelayProject>('project.add', {
                    workspace_id: workspace.id,
                    path: repo.path,
                }),
            `${repo.name} added`
        )
        setAdding(undefined)
        if (project) onAdded(project)
    }

    return (
        <Section
            title={`Repositories in ${workspace.name}`}
            action={{ label: 'Scan again', onPress: found.reload }}>
            {found.data === undefined ? (
                found.error ? (
                    <ErrorState error={found.error} onRetry={found.reload} />
                ) : (
                    <LoadingState label="Looking for Git repositories…" />
                )
            ) : found.data.repositories.length === 0 ? (
                <Text style={[styles.note, { paddingVertical: spacing.l }]}>
                    No Git repositories in {found.data.path}. Clone one from GitHub instead.
                </Text>
            ) : (
                found.data.repositories.map((repo) => {
                    const added = known.has(repo.path)
                    return (
                        <Row
                            key={repo.path}
                            label={repo.name}
                            detail={repo.path}
                            mono
                            icon="folder"
                            chevron={false}
                            disabled={added || !!adding}
                            right={
                                added ? (
                                    <Chip label="Added" />
                                ) : adding === repo.path ? (
                                    <Chip label="Adding…" tone="live" />
                                ) : (
                                    <Chip label="Add" tone="primary" icon="plus" />
                                )
                            }
                            onPress={() => add(repo)}
                        />
                    )
                })
            )}
        </Section>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
    })
}
