/** Result shapes of the git, worktree, file and integration ops (crates/relay-bus/src/types.rs). */

export type Worktree = {
    path: string
    branch: string
    head: string
    session: string | null
    dirty: boolean
    disk_mb: number | null
}

export type FileStatus = {
    path: string
    /** Porcelain-style: `M`, `A`, `D`, `R`, `?`, `U`, or empty. */
    index: string
    worktree: string
    renamed_from: string | null
}

export type GitStatus = {
    branch: string
    upstream: string | null
    ahead: number | null
    behind: number | null
    files: FileStatus[]
}

export type DiffFile = {
    path: string
    old_path: string | null
    status: string
    added: number
    removed: number
    binary: boolean
}

export type Hunk = {
    old_start: number
    new_start: number
    old_lines?: number
    new_lines?: number
    text: string
}

export type Commit = {
    sha: string
    parents: string[]
    author: string
    email: string
    at: string
    subject: string
    body: string
    refs: string[]
}

export type Branch = {
    name: string
    head: string
    upstream: string | null
    ahead: number | null
    behind: number | null
    merged: boolean
    session: string | null
    current: boolean
}

export type PullRequest = {
    number: number
    branch: string
    draft: boolean
    url: string
    title: string
    state: string
    same_repository: boolean
}

export type Entry = {
    path: string
    name: string
    kind: 'file' | 'dir' | 'symlink'
    size: number | null
    modified_at: string | null
    badge: string | null
    children: Entry[] | null
}

export type IntegrationState =
    | 'queued'
    | 'merging'
    | 'building'
    | 'deploying'
    | 'passed'
    | 'failed'
    | 'conflict'
    | 'discarded'

export type Integration = {
    id: number
    project_id: number
    branches: string[]
    worktree: string | null
    state: IntegrationState
    /** The two branches that conflicted. */
    conflict: [string, string] | null
    log_tail: string
    started_at: string | null
    finished_at: string | null
}

/** The first seven characters of a sha, as git prints it. */
export const shortSha = (sha: string) => sha.slice(0, 7)

/** The last path component: a worktree's folder name, a file's name. */
export const baseName = (path: string) => {
    const parts = path.split('/').filter(Boolean)
    return parts[parts.length - 1] ?? path
}

/** The folder a worktree-relative path sits in ('' for the root). */
export const dirName = (path: string) => {
    const at = path.lastIndexOf('/')
    return at < 0 ? '' : path.slice(0, at)
}

/** "12 KB" for a byte count. */
export const formatBytes = (bytes: number | null | undefined) => {
    if (bytes === null || bytes === undefined) return ''
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes < 10240 ? 1 : 0)} KB`
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}
