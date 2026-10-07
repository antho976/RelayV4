/** `skill.list` rows (relay-bus types.rs `Skill`). */
export type Skill = {
    id: number
    name: string
    /** Read by the engine from the SKILL.md front matter, else the first prose line. */
    description: string
    body: string
    source_url: string | null
    source_path: string | null
    revision: string | null
    enabled_in: number[]
    created_at: string
    updated_at: string
}

/** `plugin.list` rows (relay-bus types.rs `Plugin`). */
export type Plugin = {
    id: string
    name: string
    version: string
    category: string
    summary: string
    description: string
    skills: { name: string; description: string; files: number }[]
    mcp_servers: { name: string; description: string; tools: string[] }[]
    docs: string[]
    enabled_in: number[]
    suggested_for: number[]
}

export type PluginDoc = { path: string; body: string }

/** `project.get` (relay-bus types.rs `Project`). */
export type ProjectFull = {
    id: number
    workspace_id: number
    path: string
    name: string
    base_branch: string
    build_cmd: string | null
    run_cmd: string | null
    protected_paths: string[]
    critical_files: string[]
    order: number
    pinned: boolean
    created_at: string
    updated_at: string
}

/** `provider.list` rows (relay-bus types.rs `ProviderInfo`). */
export type ProviderInfo = {
    provider: string
    installed: boolean
    path: string | null
    version: string | null
    signed_in_as: string | null
    last_seen_version: string | null
    guarded: boolean
}

/** `device.run.list` rows (relay-bus types.rs `Run`). */
export type DeviceRun = {
    id: number
    project_id: number
    kind: string
    device: string
    worktree: string
    state: string
    artifact: string | null
    variant: string | null
    format: string | null
    publish: boolean
    signing: string | null
    started_at: string
    finished_at: string | null
}
