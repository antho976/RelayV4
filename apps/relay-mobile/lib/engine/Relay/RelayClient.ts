/**
 * The phone's end of Relay's remote door (RelayV4 `crates/relay-remote`): one WebSocket that
 * carries the same newline-delimited bus lines the desktop client and CLI use, after a short
 * handshake that proves the phone still holds the credential it received when it paired.
 *
 * Routes are tried in privacy order — the LAN first, the person's own rendezvous server
 * second — unless the host is pinned to one. Nothing here talks to any third party.
 */
import * as Crypto from 'expo-crypto'
import * as Device from 'expo-device'
import { AppState } from 'react-native'
import { create } from 'zustand'

import { Logger } from '@lib/state/Logger'
import { activeHost, directRoutes, RelayHost, useRelayHostsStore } from '@lib/state/RelayHosts'

import { attentionFromEvent, ensureNotifyPermission } from './Attention'
import { PairLink } from './PairLink'
import { decodeFrame } from './Terminal'

export type RelayStatus = 'offline' | 'connecting' | 'online'
export type RelayTransport = 'direct' | 'via'

export type BusError = {
    kind: string
    code: string
    message: string
    hint?: string
    details?: unknown
    confirm?: { op: string; payload: Record<string, unknown> }
}

export type BusResponse = {
    v: number
    id: string | null
    ok: boolean
    result?: any
    error?: BusError
    replayed?: boolean
}

export type BusEvent = {
    v: number
    ev: string
    ts: string
    actor: string
    cause?: string
    project_id?: number
    payload: any
}

export type PtyFrame = {
    v: number
    stream: string
    session?: string
    epoch?: number
    seq: number
    data: string
}

export type RelaySession = {
    id: number
    name: string
    project_id: number
    provider: string
    role: string
    branch: string
    worktree: string
    state: string
    task_id: number | null
    last_output_at: string | null
    pid: number | null
}

export type RelayProject = {
    id: number
    workspace_id: number
    name: string
    path: string
}

export type RelayWorkspace = {
    id: number
    name: string
    path: string
    order: number
}

export type RelayHold = {
    id: number
    session: string | null
    op: string
    policy: string
    state: string
    created_at: string
    details: any
}

export type RelayNotification = {
    id: number
    project_id: number | null
    category: string
    title: string
    body: string
    read: boolean
    created_at: string
}

export class RelayRequestError extends Error {
    constructor(public readonly error: BusError) {
        super(`${error.code}: ${error.message}`)
    }
}

type Pending = {
    resolve: (response: BusResponse) => void
    reject: (error: Error) => void
    timer: ReturnType<typeof setTimeout>
}

type FrameListener = (frame: PtyFrame, text: string) => void
type EventListener = (event: BusEvent) => void

type RelayState = {
    status: RelayStatus
    /** True from connect/pair until disconnect: the UI may reconnect on its own. */
    wanted: boolean
    /** True after the person tapped Disconnect, until they connect again. */
    stopped: boolean
    transport?: RelayTransport
    /** The address the live connection went through. */
    routeUrl?: string
    hostId?: string
    hostName?: string
    version?: string
    error?: string
    sessions: RelaySession[]
    projects: RelayProject[]
    workspaces: RelayWorkspace[]
    holds: RelayHold[]
    notifications: RelayNotification[]
    inReview: number
    lastRefresh?: number
}

const REQUEST_TIMEOUT_MS = 20_000
const HANDSHAKE_TIMEOUT_MS = 8_000
const KEEPALIVE_MS = 25_000
const RECONNECT_MIN_MS = 1_000
const RECONNECT_MAX_MS = 30_000

export const useRelayStore = create<RelayState>()(() => ({
    status: 'offline',
    wanted: false,
    stopped: false,
    sessions: [],
    projects: [],
    workspaces: [],
    holds: [],
    notifications: [],
    inReview: 0,
}))

const uuid = () => Crypto.randomUUID()

const sha256 = (text: string) => Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.SHA256, text)

/** The device name the PC shows in `relay remote devices`. */
const deviceLabel = () => Device.deviceName || Device.modelName || 'Phone'

/** Try a single WebSocket route: connect, take the greeting, answer it, wait for the verdict. */
type Opened = {
    socket: WebSocket
    welcome: { ok: boolean; device?: string; token?: string; error?: string }
    greeting: { host: string; host_id: string; instance: string; version: string }
}

const openRoute = (
    url: string,
    hello: (challenge: string) => Promise<Record<string, unknown>>,
    onLine: (line: string) => void
): Promise<Opened> =>
    new Promise((resolve, reject) => {
        let settled = false
        let stage: 'greeting' | 'welcome' | 'open' = 'greeting'
        let greeting: Opened['greeting'] | undefined
        const socket = new WebSocket(url)
        const fail = (reason: string) => {
            if (settled) return
            settled = true
            clearTimeout(timer)
            try {
                socket.close()
            } catch {}
            reject(new Error(reason))
        }
        const timer = setTimeout(() => fail(`no answer from ${url}`), HANDSHAKE_TIMEOUT_MS)
        socket.onerror = () => fail(`cannot reach ${url}`)
        socket.onclose = () => {
            if (stage !== 'open') fail(`closed by ${url}`)
        }
        socket.onmessage = async (message) => {
            const line = typeof message.data === 'string' ? message.data : ''
            if (stage === 'open') {
                onLine(line)
                return
            }
            let parsed: any
            try {
                parsed = JSON.parse(line)
            } catch {
                fail('the PC did not speak Relay')
                return
            }
            if (stage === 'greeting') {
                if (parsed?.relay !== 'remote' || typeof parsed.challenge !== 'string') {
                    // A rendezvous with no host present answers with a verdict straight away.
                    if (parsed?.ok === false) fail(String(parsed.error ?? 'refused'))
                    else fail('the PC did not speak Relay')
                    return
                }
                greeting = parsed
                stage = 'welcome'
                try {
                    socket.send(JSON.stringify(await hello(parsed.challenge)))
                } catch (e) {
                    fail(`${e}`)
                }
                return
            }
            // welcome
            if (settled) return
            settled = true
            clearTimeout(timer)
            stage = 'open'
            if (!parsed?.ok) {
                try {
                    socket.close()
                } catch {}
                reject(new Error(String(parsed?.error ?? 'refused')))
                return
            }
            resolve({ socket: socket, welcome: parsed, greeting: greeting! })
        }
    })

/**
 * Open the first route of a group that answers, closing the others. A PC lists every LAN
 * address it has, and a phone should not wait out a timeout on each in turn.
 */
const openFirst = (
    candidates: { url: string; transport: RelayTransport }[],
    hello: (challenge: string) => Promise<Record<string, unknown>>,
    onLine: (line: string) => void
): Promise<{ opened: Opened; transport: RelayTransport; url: string }> =>
    new Promise((resolve, reject) => {
        if (candidates.length === 0) {
            reject(new Error('no route'))
            return
        }
        let settled = false
        let failures = 0
        let lastError = new Error('no route')
        for (const candidate of candidates) {
            openRoute(candidate.url, hello, onLine)
                .then((opened) => {
                    if (settled) {
                        try {
                            opened.socket.close()
                        } catch {}
                        return
                    }
                    settled = true
                    resolve({ opened: opened, transport: candidate.transport, url: candidate.url })
                })
                .catch((e: Error) => {
                    lastError = e
                    failures++
                    // A refusal from the PC is final for the whole group: every address is
                    // the same door, and the answer will not change on the next one.
                    if (
                        !settled &&
                        (failures === candidates.length || /^(pair\.|auth\.)/.test(e.message))
                    ) {
                        settled = true
                        reject(lastError)
                    }
                })
        }
    })

const denialText = (code: string) => {
    switch (code) {
        case 'pair.invalid':
            return 'The pairing code was wrong or has expired. Run `relay remote pair` again.'
        case 'auth.unknown_device':
            return 'This phone is no longer paired with that PC. Pair it again.'
        case 'auth.bad_proof':
            return 'The stored credential no longer matches. Pair again.'
        case 'host.offline':
            return 'The PC is not connected to its rendezvous server right now.'
        default:
            return code
    }
}

class RelayClient {
    private socket?: WebSocket
    private pending = new Map<string, Pending>()
    private frameListeners = new Map<string, Set<FrameListener>>()
    private eventListeners = new Set<EventListener>()
    private keepalive?: ReturnType<typeof setInterval>
    private generation = 0
    /** The connection the person asked for; a drop is retried until `disconnect()`. */
    private wanted?: RelayHost
    private reconnectTimer?: ReturnType<typeof setTimeout>
    private reconnectDelay = RECONNECT_MIN_MS

    constructor() {
        // A phone that comes back to the foreground reconnects at once instead of waiting
        // out the backoff; one that goes to the background keeps whatever the OS allows.
        AppState.addEventListener('change', (next) => {
            if (next !== 'active' || !this.wanted) return
            if (useRelayStore.getState().status === 'offline') this.reconnectNow()
        })
    }

    get online() {
        return useRelayStore.getState().status === 'online'
    }

    private reconnectNow() {
        if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
        this.reconnectTimer = undefined
        if (!this.wanted) return
        // The stored record, not the one captured at connect: an address added or a route
        // pinned since then applies to the next attempt.
        const host =
            useRelayHostsStore.getState().hosts.find((item) => item.id === this.wanted?.id) ??
            this.wanted
        this.connect(host, true).catch(() => this.scheduleReconnect())
    }

    private scheduleReconnect() {
        if (!this.wanted || this.reconnectTimer) return
        const delay = this.reconnectDelay
        this.reconnectDelay = Math.min(this.reconnectDelay * 2, RECONNECT_MAX_MS)
        this.reconnectTimer = setTimeout(() => {
            this.reconnectTimer = undefined
            this.reconnectNow()
        }, delay)
    }

    /**
     * Route groups for a host, in the order they are tried: every LAN address at once, then
     * the server. A pinned route is its group alone.
     */
    routeGroups(host: RelayHost): { url: string; transport: RelayTransport }[][] {
        const direct = directRoutes(host).map((url) => ({
            url: url,
            transport: 'direct' as const,
        }))
        const via = host.via ? [{ url: host.via, transport: 'via' as const }] : []
        const groups =
            host.route === 'direct' ? [direct] : host.route === 'via' ? [via] : [direct, via]
        return groups.filter((group) => group.length > 0)
    }

    /**
     * Pair with a PC from a scanned or typed link. On success the host is stored and the
     * connection stays open.
     */
    async pair(link: PairLink): Promise<RelayHost> {
        this.disconnect()
        const generation = ++this.generation
        useRelayStore.setState({
            status: 'connecting',
            wanted: true,
            stopped: false,
            error: undefined,
            hostName: link.host,
        })
        const groups = [
            link.direct.map((url) => ({ url: url, transport: 'direct' as const })),
            link.via ? [{ url: link.via, transport: 'via' as const }] : [],
        ].filter((group) => group.length > 0)
        let lastError = 'no route'
        for (const group of groups) {
            try {
                const { opened, transport, url } = await openFirst(
                    group,
                    async () => ({ v: 1, pair: link.code, device_name: deviceLabel() }),
                    (line) => this.onLine(line)
                )
                if (generation !== this.generation) {
                    opened.socket.close()
                    throw new Error('cancelled')
                }
                const host: RelayHost = {
                    id: opened.greeting.host_id || link.hostId || uuid(),
                    name: opened.greeting.host || link.host,
                    instance: opened.greeting.instance || link.instance,
                    direct: link.direct,
                    via: link.via,
                    deviceId: opened.welcome.device!,
                    token: opened.welcome.token!,
                    route: 'auto',
                    pairedAt: Date.now(),
                    lastConnectedAt: Date.now(),
                    lastTransport: transport,
                }
                useRelayHostsStore.getState().addHost(host)
                // A freshly paired PC is one the person wants to stay connected to.
                this.wanted = host
                this.reconnectDelay = RECONNECT_MIN_MS
                this.adopt(opened, host, transport, url)
                return host
            } catch (e) {
                lastError = denialText(`${(e as Error).message}`)
                // A wrong code is final; a route that cannot be reached is not. A newer
                // connect owns the store now, so a superseded pairing says nothing.
                if (generation !== this.generation) throw new Error('cancelled')
                if (/pair\.|expired/.test(lastError)) break
            }
        }
        useRelayStore.setState({ status: 'offline', wanted: false, error: lastError })
        throw new Error(lastError)
    }

    /** Connect to a stored host, trying its routes in order. Drops are retried until `disconnect()`. */
    async connect(host: RelayHost = activeHost()!, retrying = false): Promise<void> {
        if (!host) throw new Error('No PC paired yet')
        // A reconnect timer from an earlier drop must not fire into this connection.
        if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
        this.reconnectTimer = undefined
        this.teardown()
        this.wanted = host
        if (!retrying) this.reconnectDelay = RECONNECT_MIN_MS
        const generation = ++this.generation
        useRelayStore.setState({
            status: 'connecting',
            wanted: true,
            stopped: false,
            error: undefined,
            hostId: host.id,
            hostName: host.name,
        })
        const groups = this.routeGroups(host)
        let lastError =
            host.route === 'direct'
                ? 'This PC has no WiFi address; set its route to Auto or Server'
                : host.route === 'via'
                  ? 'This PC has no server route; set its route to Auto or WiFi'
                  : 'This PC has no address to connect to'
        if (groups.length === 0) this.wanted = undefined
        for (const group of groups) {
            try {
                const { opened, transport, url } = await openFirst(
                    group,
                    async (challenge) => ({
                        v: 1,
                        device: host.deviceId,
                        proof: await sha256(`${challenge}:${host.token}`),
                    }),
                    (line) => this.onLine(line)
                )
                if (generation !== this.generation) {
                    opened.socket.close()
                    return
                }
                useRelayHostsStore.getState().updateHost(host.id, {
                    name: opened.greeting.host || host.name,
                    instance: opened.greeting.instance || host.instance,
                    lastConnectedAt: Date.now(),
                    lastTransport: transport,
                })
                this.adopt(opened, host, transport, url)
                this.reconnectDelay = RECONNECT_MIN_MS
                await this.refresh()
                return
            } catch (e) {
                lastError = denialText(`${(e as Error).message}`)
                // A revoked or broken credential is final; keep retrying anything else.
                if (/paired|credential/.test(lastError)) {
                    this.wanted = undefined
                    break
                }
            }
        }
        if (generation === this.generation) {
            useRelayStore.setState({
                status: 'offline',
                wanted: !!this.wanted,
                error: this.wanted ? `${lastError} — retrying` : lastError,
            })
            if (this.wanted) this.scheduleReconnect()
        }
        throw new Error(lastError)
    }

    /** Stop and stay stopped: no reconnect until the person asks again. */
    disconnect() {
        this.wanted = undefined
        if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
        this.reconnectTimer = undefined
        this.teardown()
        useRelayStore.setState({ wanted: false, stopped: true })
    }

    private teardown() {
        this.generation++
        if (this.keepalive) clearInterval(this.keepalive)
        this.keepalive = undefined
        const socket = this.socket
        this.socket = undefined
        for (const [, pending] of this.pending) {
            clearTimeout(pending.timer)
            pending.reject(new Error('disconnected'))
        }
        this.pending.clear()
        this.frameListeners.clear()
        if (socket) {
            socket.onclose = null
            socket.onerror = null
            socket.onmessage = null
            try {
                socket.close()
            } catch {}
        }
        useRelayStore.setState({
            status: 'offline',
            transport: undefined,
            routeUrl: undefined,
            sessions: [],
            holds: [],
            notifications: [],
            inReview: 0,
        })
    }

    private adopt(opened: Opened, host: RelayHost, transport: RelayTransport, url: string) {
        const generation = this.generation
        this.socket = opened.socket
        opened.socket.onclose = () => {
            if (generation !== this.generation) return
            this.socket = undefined
            if (this.keepalive) clearInterval(this.keepalive)
            for (const [, pending] of this.pending) {
                clearTimeout(pending.timer)
                pending.reject(new Error('connection closed'))
            }
            this.pending.clear()
            useRelayStore.setState({
                status: 'offline',
                error: this.wanted
                    ? 'Connection lost — reconnecting'
                    : 'Connection closed by the PC',
            })
            if (this.wanted) this.scheduleReconnect()
        }
        opened.socket.onerror = () => {}
        useRelayStore.setState({
            status: 'online',
            transport: transport,
            routeUrl: url,
            hostId: host.id,
            hostName: opened.greeting.host || host.name,
            version: opened.greeting.version,
            error: undefined,
        })
        this.keepalive = setInterval(() => {
            // A ping that times out means the socket is dead without saying so (a NAT that
            // dropped the flow); closing it hands the drop to the reconnect path.
            this.request('bus.ping', {}).catch((e: Error) => {
                if (/timed out/.test(e.message)) opened.socket.close()
            })
        }, KEEPALIVE_MS)
        ensureNotifyPermission()
        // Transitions only: the engine never emits an event per byte of output.
        this.request('bus.subscribe', {
            events: ['session.*', 'guardrail.*', 'notify.*', 'task.*', 'project.*', 'workspace.*'],
        }).catch(() => {})
        Logger.info(`Relay: connected to ${host.name} (${transport})`)
    }

    private onLine(line: string) {
        let parsed: any
        try {
            parsed = JSON.parse(line)
        } catch {
            return
        }
        if (typeof parsed?.stream === 'string') {
            const frame = parsed as PtyFrame
            if (frame.stream === 'pty' && frame.session) {
                const listeners = this.frameListeners.get(frame.session)
                if (listeners && listeners.size > 0) {
                    const text = decodeFrame(String(frame.data ?? ''))
                    for (const listener of listeners) listener(frame, text)
                }
            }
            return
        }
        if (typeof parsed?.ev === 'string') {
            const event = parsed as BusEvent
            this.onEvent(event)
            for (const listener of this.eventListeners) listener(event)
            return
        }
        const response = parsed as BusResponse
        if (response.id) {
            const pending = this.pending.get(response.id)
            if (pending) {
                this.pending.delete(response.id)
                clearTimeout(pending.timer)
                pending.resolve(response)
            }
        }
    }

    private onEvent(event: BusEvent) {
        const state = useRelayStore.getState()
        if (event.ev === 'session.changed' && event.payload?.name) {
            const session = event.payload as RelaySession
            const others = state.sessions.filter((item) => item.name !== session.name)
            const sessions =
                session.state === 'closed' ? others : [...others, session].sort(bySessionOrder)
            useRelayStore.setState({ sessions })
        } else if (
            event.ev.startsWith('guardrail.') ||
            event.ev.startsWith('notify.') ||
            event.ev === 'task.changed'
        ) {
            this.refreshAttention().catch(() => {})
        } else if (event.ev.startsWith('project.') || event.ev.startsWith('workspace.')) {
            // A project added or renamed on the desktop shows up in the sidebar and the
            // request sheet without a pull-to-refresh.
            this.refreshProjects().catch(() => {})
        }
        attentionFromEvent(event)
    }

    /** One bus request. Rejects with `RelayRequestError` on a typed refusal. */
    async request<T = any>(op: string, payload: Record<string, unknown> = {}): Promise<T> {
        const socket = this.socket
        if (!socket || socket.readyState !== WebSocket.OPEN) {
            throw new Error('Not connected to a PC')
        }
        const id = uuid()
        const line = JSON.stringify({ v: 1, id: id, actor: 'user', op: op, payload: payload })
        const response = await new Promise<BusResponse>((resolve, reject) => {
            const timer = setTimeout(() => {
                this.pending.delete(id)
                reject(new Error(`${op} timed out`))
            }, REQUEST_TIMEOUT_MS)
            this.pending.set(id, { resolve, reject, timer })
            socket.send(line)
        })
        if (!response.ok) throw new RelayRequestError(response.error!)
        return response.result as T
    }

    /** Keystrokes: fire and forget, so typing never waits on a round trip. */
    input(session: string, data: string) {
        const socket = this.socket
        if (!socket || socket.readyState !== WebSocket.OPEN) return
        socket.send(
            JSON.stringify({
                v: 1,
                id: uuid(),
                actor: 'user',
                op: 'session.input',
                payload: { session, data },
            })
        )
    }

    /** Attach a session's PTY stream; returns the detach function. */
    async attach(
        session: string,
        listener: FrameListener,
        from?: { epoch: number; seq: number }
    ): Promise<() => void> {
        let listeners = this.frameListeners.get(session)
        if (!listeners) {
            listeners = new Set()
            this.frameListeners.set(session, listeners)
        }
        listeners.add(listener)
        const payload: Record<string, unknown> = { session }
        if (from) {
            payload.epoch = from.epoch
            payload.from_seq = from.seq
        }
        try {
            await this.request('session.attach', payload)
        } catch (e) {
            listeners.delete(listener)
            if (listeners.size === 0) this.frameListeners.delete(session)
            throw e
        }
        return () => {
            const set = this.frameListeners.get(session)
            set?.delete(listener)
            if (set && set.size === 0) {
                this.frameListeners.delete(session)
                this.request('session.detach', { session }).catch(() => {})
            }
        }
    }

    onEvents(listener: EventListener): () => void {
        this.eventListeners.add(listener)
        return () => this.eventListeners.delete(listener)
    }

    /** Sessions, projects and attention items in as few round trips as the bus allows. */
    async refresh() {
        const [sessions] = await Promise.all([
            this.request<{ sessions: RelaySession[] }>('session.list', {}),
            this.refreshProjects(),
        ])
        useRelayStore.setState({
            sessions: [...sessions.sessions].sort(bySessionOrder),
            lastRefresh: Date.now(),
        })
        await this.refreshAttention()
    }

    /** Projects and the workspaces that hold them, for the sidebar and the request sheet. */
    async refreshProjects() {
        const [projects, workspaces] = await Promise.all([
            this.request<{ projects: RelayProject[] }>('project.list', {}),
            this.request<{ workspaces: RelayWorkspace[] }>('workspace.list', {}).catch(() => ({
                workspaces: [] as RelayWorkspace[],
            })),
        ])
        useRelayStore.setState({
            projects: projects.projects,
            workspaces: [...workspaces.workspaces].sort(
                (a, b) => a.order - b.order || a.name.localeCompare(b.name)
            ),
        })
    }

    async refreshAttention() {
        try {
            const dashboard = await this.request<{
                holds_open: RelayHold[]
                notifications: RelayNotification[]
                in_review: unknown[]
            }>('dashboard.get', {})
            useRelayStore.setState({
                holds: dashboard.holds_open ?? [],
                notifications: (dashboard.notifications ?? []).filter((item) => !item.read),
                inReview: dashboard.in_review?.length ?? 0,
            })
        } catch (e) {
            // An older engine without dashboard.get still lists holds.
            try {
                const holds = await this.request<{ holds: RelayHold[] }>('guardrail.holds.list', {
                    open_only: true,
                })
                useRelayStore.setState({ holds: holds.holds ?? [] })
            } catch {
                Logger.debug(`Relay: attention refresh failed: ${e}`)
            }
        }
    }
}

const stateOrder: Record<string, number> = {
    blocked: 0,
    running: 1,
    spawning: 2,
    idle: 3,
    created: 4,
    restorable: 5,
    parked: 6,
    exited: 7,
    closed: 8,
}

const bySessionOrder = (a: RelaySession, b: RelaySession) =>
    (stateOrder[a.state] ?? 9) - (stateOrder[b.state] ?? 9) || a.name.localeCompare(b.name)

export const relay = new RelayClient()
