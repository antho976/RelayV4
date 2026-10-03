import { isCancelled, RelayRequestError } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'

/** The board's five columns (BUS.md §6), in the order work flows through them. */
export type Column = 'backlog' | 'ready' | 'active' | 'in_review' | 'done'
export type TaskState = 'none' | 'dispatched' | 'running' | 'blocked' | 'failed' | 'awaiting_review'
export type Priority = 'low' | 'medium' | 'high' | 'urgent'
export type Size = 'S' | 'M' | 'L'
export type TaskType = 'task' | 'feature' | 'bug' | 'chore' | 'spike'
export type Relation = 'blocked_by' | 'duplicate_of'

export type Attachment = {
    id: number
    task_id: number
    name: string
    mime: string
    bytes: number
    path: string
    created_at: string
}

/** `types::Task`, as task.get / task.list return it. */
export type Task = {
    id: number
    project_id: number
    module_id: number | null
    title: string
    body: string
    changelog: string
    column: Column
    position: number
    state: TaskState
    priority: Priority
    size: Size | null
    type: TaskType
    parent_id: number | null
    depth: number
    children: number[]
    rollup: { total: number; done: number }
    labels: string[]
    blocked_by: number[]
    blocks: number[]
    duplicate_of: number | null
    sessions: string[]
    commits: { sha: string; branch: string | null; linked_at: string }[]
    attachments: Attachment[]
    created_at: string
    updated_at: string
    deleted_at: string | null
}

export type Module = {
    id: number
    project_id: number
    name: string
    icon: string | null
    priority: Priority
    order: number
    completed_at: string | null
    created_at: string
    updated_at: string
    deleted_at: string | null
}

export type ModuleSummary = Module & {
    counts: Partial<Record<Column, number>>
    progress_pct: number
}

export type ModuleHeader = {
    count: number
    in_flight: number
    issues: number
    completed: number
    completion_pct: number
}

export const COLUMNS: { value: Column; label: string }[] = [
    { value: 'backlog', label: 'Backlog' },
    { value: 'ready', label: 'Ready' },
    { value: 'active', label: 'Active' },
    { value: 'in_review', label: 'Review' },
    { value: 'done', label: 'Done' },
]

export const columnLabel = (column: Column) =>
    column === 'in_review'
        ? 'In review'
        : (COLUMNS.find((c) => c.value === column)?.label ?? column)

export const TYPES: { value: TaskType; label: string }[] = [
    { value: 'task', label: 'Task' },
    { value: 'feature', label: 'Feature' },
    { value: 'bug', label: 'Bug' },
    { value: 'chore', label: 'Chore' },
    { value: 'spike', label: 'Spike' },
]

export const PRIORITIES: { value: Priority; label: string }[] = [
    { value: 'low', label: 'Low' },
    { value: 'medium', label: 'Medium' },
    { value: 'high', label: 'High' },
    { value: 'urgent', label: 'Urgent' },
]

export const SIZES: { value: Size | 'none'; label: string }[] = [
    { value: 'none', label: 'None' },
    { value: 'S', label: 'S' },
    { value: 'M', label: 'M' },
    { value: 'L', label: 'L' },
]

export const priorityTone = (priority: Priority) =>
    priority === 'urgent' ? 'danger' : priority === 'high' ? 'warn' : 'neutral'

export const stateLabel = (state: TaskState) => state.replace('_', ' ')

export const stateTone = (state: TaskState) =>
    state === 'failed' || state === 'blocked'
        ? 'danger'
        : state === 'running' || state === 'dispatched'
          ? 'live'
          : state === 'awaiting_review'
            ? 'primary'
            : 'neutral'

/** A session a task can be dispatched to: the handler refuses exited and closed ones. */
export const DISPATCHABLE = ['created', 'parked', 'restorable', 'running', 'idle', 'blocked']

export const errorText = (e: unknown) =>
    e instanceof RelayRequestError ? e.error.message : e instanceof Error ? e.message : String(e)

export const isConflict = (e: unknown) =>
    e instanceof RelayRequestError &&
    (e.error.code === 'task.edit_conflict' || e.error.code === 'module.edit_conflict')

/**
 * Run a mutation and toast its failure; a denied hold stays quiet. Resolves to the result, or
 * undefined when it did not happen.
 */
export const attempt = async <T>(run: () => Promise<T>, done?: string): Promise<T | undefined> => {
    try {
        const result = await run()
        if (done) Logger.infoToast(done)
        return result
    } catch (e) {
        if (!isCancelled(e)) Logger.errorToast(errorText(e))
        return undefined
    }
}

export const taskHref = (taskId: number) => ({
    pathname: '/screens/RelayScreen/Task' as const,
    params: { task_id: String(taskId) },
})

export const moduleHref = (moduleId: number) => ({
    pathname: '/screens/RelayScreen/Module' as const,
    params: { module_id: String(moduleId) },
})

export const shortTime = (ts: string | null | undefined) => {
    if (!ts) return ''
    const date = new Date(ts)
    if (Number.isNaN(date.getTime())) return ts
    const now = new Date()
    return date.toDateString() === now.toDateString()
        ? date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
        : date.toLocaleDateString([], { month: 'short', day: 'numeric' })
}
