/**
 * Routes the session screens share. `Session` is not in the kit's `RelayPage` list (the kit
 * belongs to another change), so its href is built here the same way `relayHref` builds one.
 */
export const sessionHref = (session: string) => ({
    pathname: '/screens/RelayScreen/Session' as const,
    params: { session: session },
})

export const terminalHref = (session: string) => ({
    pathname: '/screens/RelayScreen/Terminal' as const,
    params: { session: session },
})

/** The launcher, for a project, optionally with a task staged or a prompt filled in. */
export const launchHref = (
    projectId: number | undefined,
    extra: { task?: number; prompt?: string } = {}
) => {
    const params: Record<string, string> = {}
    if (projectId !== undefined) params.project_id = String(projectId)
    if (extra.task !== undefined) params.task = String(extra.task)
    if (extra.prompt) params.prompt = extra.prompt
    return { pathname: '/screens/RelayScreen/Launch' as const, params: params }
}

/** "812 MB", "1.4 GB", "<1 MB". */
export const megabytes = (mb: number) =>
    mb >= 1024
        ? `${(mb / 1024).toFixed(1)} GB`
        : mb > 0 && mb < 1
          ? '<1 MB'
          : `${Math.round(mb)} MB`
