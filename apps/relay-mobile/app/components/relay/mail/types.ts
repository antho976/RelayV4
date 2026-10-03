import { useBusQuery } from '@components/relay/hooks'

/** `types::Message`, as mailbox.list returns it. `to` is a session name or `*`. */
export type RelayMessage = {
    id: number
    project_id: number
    from: string
    to: string
    text: string
    re_task: number | null
    priority: boolean
    sent_at: string
    /** For one session's list: when that session read it; otherwise when the last addressee did. */
    acked_at: string | null
}

/** One addressee of a sent message (`MailboxRecipient`). */
export type MailRecipient = {
    session: string
    state: string
    acked_at: string | null
}

export type OutboxEntry = { message: RelayMessage; recipients: MailRecipient[] }

export type SendResult = {
    message: RelayMessage
    recipients: MailRecipient[]
    delivery: 'queued' | 'session_parked' | 'not_running' | 'no_recipients' | string
}

/** mailbox.send's `delivery`, as a person reads it. */
export const deliveryText = (result: SendResult) => {
    const count = result.recipients.length
    const who = count === 1 ? result.recipients[0].session : `${count} sessions`
    switch (result.delivery) {
        case 'queued':
            return `Sent to ${who}`
        case 'session_parked':
            return `Sent; ${who} ${count === 1 ? 'is' : 'are'} parked and will read it on waking`
        case 'not_running':
            return `Stored; ${who} ${count === 1 ? 'is' : 'are'} not running`
        case 'no_recipients':
            return 'Nobody to deliver to'
        default:
            return `Sent (${result.delivery})`
    }
}

const bySent = (a: RelayMessage, b: RelayMessage) =>
    a.sent_at === b.sent_at ? a.id - b.id : a.sent_at < b.sent_at ? -1 : 1

/**
 * The project's mail, oldest first, reloaded on mailbox events. With `session`, that
 * session's history: what it received (its own read state, broadcasts included) and what
 * it sent.
 */
export const useMail = (projectId: number | undefined, session?: string) => {
    const enabled = projectId !== undefined
    const options = { events: ['mailbox.*'], projectId: projectId, enabled: enabled }
    const all = useBusQuery<RelayMessage[]>(
        'mailbox.list',
        { project_id: projectId },
        { ...options, select: (r) => r.messages }
    )
    const received = useBusQuery<RelayMessage[]>(
        'mailbox.list',
        { project_id: projectId, session: session },
        { ...options, enabled: enabled && !!session, select: (r) => r.messages }
    )
    let data: RelayMessage[] | undefined = all.data
    if (session) {
        if (all.data === undefined || received.data === undefined) data = undefined
        else {
            const seen = new Set(received.data.map((m) => m.id))
            data = [
                ...received.data,
                ...all.data.filter((m) => m.from === session && !seen.has(m.id)),
            ].sort(bySent)
        }
    }
    const reload = async () => {
        await Promise.all([all.reload(), session ? received.reload() : Promise.resolve()])
    }
    return {
        data: data,
        error: all.error ?? (session ? received.error : undefined),
        loading: all.loading || (!!session && received.loading),
        reload: reload,
    }
}

export const useOutbox = (projectId: number | undefined, enabled = true) =>
    useBusQuery<OutboxEntry[]>(
        'mailbox.outbox',
        { project_id: projectId, limit: 100 },
        {
            events: ['mailbox.*'],
            projectId: projectId,
            enabled: enabled && projectId !== undefined,
            select: (r) => r.sent,
        }
    )

export const taskHref = (taskId: number) => ({
    pathname: '/screens/RelayScreen/Task' as const,
    params: { task_id: String(taskId) },
})
