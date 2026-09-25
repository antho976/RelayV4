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
import { useHoldPromptStore } from './HoldPrompt'
import { PairLink } from './PairLink'
import { base64Decode, Utf8Stream } from './Terminal'

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

/**
 * Any data-plane frame. `logcat` frames carry `run_id` and one log line as `data`; `mirror`
 * frames carry `mirror_id` and base64 video. A broken stream sends `data: {error}` once.
 */
export type StreamFrame = {
    v: number
    stream: string
    session?: string
    run_id?: number
    mirror_id?: number
    epoch?: number
    seq: number
    data: string | { error: string }
}

/** The frozen action behind a hold, as `guardrail.hold.get` returns it (auth removed). */
export type HeldRequest = {
    v: number
    id: string
    actor: string
    op: string
    payload: Record<string, any>
}

/** The full hold row (`types::Hold`); the dashboard's `RelayHold` is a subset of it. */
export type RelayHoldFull = RelayHold & {
    project_id: number | null
    session_id: number | null
    actor: string
    payload_hash: string
    resolved_at: string | null
    resolved_by: string | null
}

export type RequestOptions = {
    /** Overrides the per-op default from `timeoutFor`. */
    timeoutMs?: number
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

/** The person answered Deny on a held action; nothing ran. Callers usually ignore it quietly. */
export class RelayCancelledError extends Error {
    constructor(message = 'Cancelled: the held action was denied') {
        super(message)
    }
}

export const isCancelled = (e: unknown): e is RelayCancelledError =>
    e instanceof RelayCancelledError

type Pending = {
    resolve: (response: BusResponse) => void
    reject: (error: Error) => void
    timer: ReturnType<typeof setTimeout>
}

type FrameListener = (frame: PtyFrame, text: string) => void
type StreamListener = (frame: StreamFrame) => void

/** One listener's own decoder: a character split across two frames is joined for it. */
type Attachment = { decoder: Utf8Stream; epoch?: number }
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
}

const REQUEST_TIMEOUT_MS = 20_000

// Ops that legitimately run long on the PC: a fresh worktree may hydrate LFS assets, a clone
// or a Gradle build takes minutes. The desktop's table (relay-native client.rs) plus the ops
// it runs off its own timer.
const LONG_OPS: Record<string, number> = {
    'project.clone': 1_800_000,
    'session.create': 180_000,
    'session.spawn': 180_000,
    'task.dispatch': 180_000,
    'git.worktree.create': 180_000,
    'git.push': 300_000,
    'git.fetch': 300_000,
    'git.pr.open': 300_000,
    'git.pr.list': 120_000,
    'integration.request': 300_000,
    'device.build': 300_000,
    'device.run': 300_000,
    'avd.boot': 180_000,
    'app.backup.now': 300_000,
    'skill.install': 300_000,
    'provider.update': 300_000,
    'provider.refresh': 120_000,
    'guardrail.confirm': 300_000,
    'file.search': 60_000,
}

/** How long the phone waits for `op` before calling it lost (the outcome is then unknown). */
export const timeoutFor = (op: string) => LONG_OPS[op] ?? REQUEST_TIMEOUT_MS

/**
 * What the phone subscribes to on connect: every transition a PC screen may want to follow.
 * `resource.sample` is left out on purpose — it ticks every second; see `watchResources`.
 */
export const EVENT_PATTERNS = [
    'session.*',
    'guardrail.*',
    'notify.*',
    'task.*',
    'project.*',
    'workspace.*',
    'mailbox.*',
    'notes.*',
    'module.*',
    'git.*',
    'file.*',
    'worktree.*',
    'overlap.*',
    'integration.*',
    'usage.*',
    'run.*',
    'device.*',
    'avd.*',
    'skill.*',
    'plugin.*',
    'provider.*',
    'settings.*',
    'github.*',
]

/** The engine's own filter (socket.rs `Filter`): exact, `prefix.*`, or `*`. */
export const matchEvent = (patterns: string[], ev: string) =>
    patterns.some(
        (p) =>
            p === '*' ||
            p === ev ||
            (p.endsWith('.*') && ev.startsWith(p.slice(0, -1)) && ev.length > p.length - 1)
    )

/** Order-insensitive JSON equality, for comparing a frozen payload with the one we sent. */
const sameJson = (a: unknown, b: unknown): boolean => {
    if (a === b) return true
    if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false
    if (Array.isArray(a) !== Array.isArray(b)) return false
    const ak = Object.keys(a as object).filter((k) => (a as any)[k] !== undefined)
    const bk = Object.keys(b as object).filter((k) => (b as any)[k] !== undefined)
    if (ak.length !== bk.length) return false
    return ak.every((k) => sameJson((a as any)[k], (b as any)[k]))
}
// Generous: the first packet to a Tailscale peer can wait several seconds while the tunnel
// wakes or is set up through a relay, and a LAN route that cannot answer fails on its own.
const HANDSHAKE_TIMEOUT_MS = 20_000
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

type Route = { url: string; transport: RelayTransport }

type Greeting = { host: string; host_id: string; instance: string; version: string }

/** A route whose PC has greeted: the socket is open and waits for the phone's hello. */
type Greeted = { socket: WebSocket; greeting: Greeting; challenge: string; route: Route }

type Opened = {
    socket: WebSocket
    welcome: { ok: boolean; device?: string; token?: string; error?: string }
    greeting: Greeting
}

/**
 * Sockets still in their handshake, each with the call that abandons it. Whoever starts a
 * handshake registers it here, so a teardown can close every one of them at once.
 */
type Opening = Map<WebSocket, () => void>

const closeQuietly = (socket: WebSocket) => {
    socket.onopen = null
    socket.onclose = null
    socket.onerror = null
    socket.onmessage = null
    try {
        socket.close()
    } catch {}
}

/** Open one route and wait for the PC's greeting. Nothing of the phone's is sent yet. */
const greetRoute = (route: Route, opening: Opening) => {
    let settled = false
    const socket = new WebSocket(route.url)
    let resolve: (greeted: Greeted) => void = () => {}
    let reject: (error: Error) => void = () => {}
    const promise = new Promise<Greeted>((yes, no) => {
        resolve = yes
        reject = no
    })
    const fail = (reason: string) => {
        if (settled) return
        settled = true
        clearTimeout(timer)
        opening.delete(socket)
        closeQuietly(socket)
        reject(new Error(reason))
    }
    const timer = setTimeout(() => fail(`no answer from ${route.url}`), HANDSHAKE_TIMEOUT_MS)
    opening.set(socket, () => fail('cancelled'))
    socket.onerror = () => fail(`cannot reach ${route.url}`)
    socket.onclose = () => fail(`closed by ${route.url}`)
    socket.onmessage = (message) => {
        if (settled) return
        let parsed: any
        try {
            parsed = JSON.parse(typeof message.data === 'string' ? message.data : '')
        } catch {
            fail('the PC did not speak Relay')
            return
        }
        if (parsed?.relay !== 'remote' || typeof parsed.challenge !== 'string') {
            // A rendezvous with no host present answers with a verdict straight away.
            if (parsed?.ok === false) fail(String(parsed.error ?? 'refused'))
            else fail('the PC did not speak Relay')
            return
        }
        settled = true
        clearTimeout(timer)
        // Still in `opening`: `answer` takes it over, and a teardown before then closes it.
        socket.onmessage = null
        resolve({ socket: socket, greeting: parsed, challenge: parsed.challenge, route: route })
    }
    return { promise: promise, cancel: () => fail('cancelled') }
}

/**
 * Race the routes of a group to their greeting and keep the first; every other socket is
 * closed at once, answered or not. A PC lists every LAN address it has, and a phone should
 * not wait out a timeout on each in turn — but it answers on one of them only.
 */
const greetFirst = (routes: Route[], opening: Opening): Promise<Greeted> =>
    new Promise((resolve, reject) => {
        if (routes.length === 0) {
            reject(new Error('no route'))
            return
        }
        let settled = false
        let failures = 0
        const attempts = routes.map((route) => {
            try {
                return greetRoute(route, opening)
            } catch (e) {
                // A malformed address throws in the WebSocket constructor.
                return { promise: Promise.reject(e as Error), cancel: () => {} }
            }
        })
        attempts.forEach((attempt) => {
            attempt.promise
                .then((greeted) => {
                    if (settled) {
                        opening.delete(greeted.socket)
                        closeQuietly(greeted.socket)
                        return
                    }
                    settled = true
                    for (const other of attempts) if (other !== attempt) other.cancel()
                    resolve(greeted)
                })
                .catch((e: Error) => {
                    failures++
                    if (!settled && failures === attempts.length) {
                        settled = true
                        reject(e)
                    }
                })
        })
    })

/** Send the hello on a greeted socket and wait for the verdict. */
const answer = (
    greeted: Greeted,
    hello: (challenge: string) => Promise<Record<string, unknown>>,
    opening: Opening
): Promise<Opened> =>
    new Promise((resolve, reject) => {
        const { socket, route } = greeted
        let settled = false
        const fail = (reason: string) => {
            if (settled) return
            settled = true
            clearTimeout(timer)
            opening.delete(socket)
            closeQuietly(socket)
            reject(new Error(reason))
        }
        const timer = setTimeout(() => fail(`no answer from ${route.url}`), HANDSHAKE_TIMEOUT_MS)
        opening.set(socket, () => fail('cancelled'))
        socket.onerror = () => fail(`cannot reach ${route.url}`)
        socket.onclose = () => fail(`closed by ${route.url}`)
        socket.onmessage = (message) => {
            if (settled) return
            let parsed: any
            try {
                parsed = JSON.parse(typeof message.data === 'string' ? message.data : '')
            } catch {
                fail('the PC did not speak Relay')
                return
            }
            if (!parsed?.ok) {
                fail(String(parsed?.error ?? 'refused'))
                return
            }
            settled = true
            clearTimeout(timer)
            opening.delete(socket)
            // The caller adopts the socket in the same turn and sets its own handlers; the
            // PC sends nothing more until the phone asks.
            socket.onmessage = null
            socket.onclose = null
            socket.onerror = null
            resolve({ socket: socket, welcome: parsed, greeting: greeted.greeting })
        }
        if (socket.readyState !== WebSocket.OPEN) {
            fail(`closed by ${route.url}`)
            return
        }
        hello(greeted.challenge)
            .then((line) => {
                if (!settled) socket.send(JSON.stringify(line))
            })
            .catch((e) => fail(`${e}`))
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
    /** Sockets still in their handshake; a teardown closes them all. */
    private opening: Opening = new Map()
    private pending = new Map<string, Pending>()
    private frameListeners = new Map<string, Map<FrameListener, Attachment>>()
    private eventListeners = new Set<EventListener>()
    /** Stream listeners by `${stream}:${key}`; `*` as the key hears every frame of a stream. */
    private streamListeners = new Map<string, Set<StreamListener>>()
    /** Screens that want `resource.sample`; the engine samples only while this is above 0. */
    private resourceWatchers = 0
    private keepalive?: ReturnType<typeof setInterval>
    private generation = 0
    /** The connection the person asked for; a drop is retried until `disconnect()`. */
    private wanted?: RelayHost
    private reconnectTimer?: ReturnType<typeof setTimeout>
    private reconnectDelay = RECONNECT_MIN_MS
    /** The pairing in flight, so a second scan of the same code joins it instead of racing it. */
    private pairing?: { code: string; promise: Promise<RelayHost> }

    constructor() {
        // A phone that comes back to the foreground reconnects at once instead of waiting
        // out the backoff; one that goes to the background keeps whatever the OS allows.
        AppState.addEventListener('change', (next) => {
            if (next !== 'active' || !this.wanted) return
            if (useRelayStore.getState().status === 'offline') this.reconnectNow()
        })
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
        // A failed attempt schedules the next one itself, and only while it is still the
        // current one; a superseded attempt must not schedule anything.
        this.connect(host, true).catch(() => {})
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
    routeGroups(host: RelayHost): Route[][] {
        const direct = directRoutes(host).map((url) => ({
            url: url,
            transport: 'direct' as const,
        }))
        const via = host.via ? [{ url: host.via, transport: 'via' as const }] : []
        // A pin to a route this PC does not have (Server only with no server) would leave
        // nothing to try; it falls back to Auto instead of failing every connect.
        const pinned = host.route === 'direct' ? direct : host.route === 'via' ? via : []
        const groups = pinned.length > 0 ? [pinned] : [direct, via]
        return groups.filter((group) => group.length > 0)
    }

    /**
     * Pair with a PC from a scanned or typed link. On success the host is stored and the
     * connection stays open. A second call with the same code while the first is in flight
     * gets the first one's answer; the code is single use and is sent once.
     */
    pair(link: PairLink): Promise<RelayHost> {
        if (this.pairing) {
            if (this.pairing.code === link.code) return this.pairing.promise
            return Promise.reject(new Error('Already pairing with a PC; wait for it to finish.'))
        }
        const promise = this.pairOnce(link).finally(() => {
            if (this.pairing?.promise === promise) this.pairing = undefined
        })
        this.pairing = { code: link.code, promise: promise }
        return promise
    }

    private async pairOnce(link: PairLink): Promise<RelayHost> {
        this.disconnect()
        const generation = ++this.generation
        useRelayStore.setState({
            status: 'connecting',
            wanted: true,
            stopped: false,
            error: undefined,
            hostName: link.host,
            // Whatever PC this turns out to be, the last one's lists are not its lists.
            projects: [],
            workspaces: [],
        })
        const groups = [
            link.direct.map((url) => ({ url: url, transport: 'direct' as const })),
            link.via ? [{ url: link.via, transport: 'via' as const }] : [],
        ].filter((group) => group.length > 0)
        let lastError = 'no route'
        for (const group of groups) {
            // Routes race only to the greeting. The code then goes out on the one socket that
            // won: the PC spends it on the first hello it reads, so a second route could only
            // mint a second device or be told the code is wrong.
            let greeted: Greeted
            try {
                greeted = await greetFirst(group, this.opening)
            } catch (e) {
                if (generation !== this.generation) throw new Error('cancelled')
                lastError = denialText(`${(e as Error).message}`)
                continue
            }
            let opened: Opened
            try {
                opened = await answer(
                    greeted,
                    async () => ({ v: 1, pair: link.code, device_name: deviceLabel() }),
                    this.opening
                )
            } catch (e) {
                if (generation !== this.generation) throw new Error('cancelled')
                // Once the code is sent it may be spent, whatever went wrong after; trying it
                // on another route would only be refused.
                lastError = denialText(`${(e as Error).message}`)
                break
            }
            const transport = greeted.route.transport
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
            // The PC has minted this device; keeping the credential costs nothing, even when a
            // newer connect superseded this pairing.
            useRelayHostsStore.getState().addHost(host)
            if (generation !== this.generation) {
                closeQuietly(opened.socket)
                throw new Error('cancelled')
            }
            // A freshly paired PC is one the person wants to stay connected to.
            this.wanted = host
            this.reconnectDelay = RECONNECT_MIN_MS
            this.adopt(opened, host, transport, greeted.route.url)
            return host
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
        const previous = useRelayStore.getState().hostId
        useRelayStore.setState({
            status: 'connecting',
            wanted: true,
            stopped: false,
            error: undefined,
            hostId: host.id,
            hostName: host.name,
            // Another PC's projects must not stay listed while this one is unreachable.
            ...(previous !== host.id ? { projects: [], workspaces: [] } : {}),
        })
        const groups = this.routeGroups(host)
        let lastError = 'This PC has no address to connect to; pair it again or add one'
        if (groups.length === 0) this.wanted = undefined
        for (const group of groups) {
            let greeted: Greeted
            let opened: Opened
            try {
                // One socket answers: the others are closed as soon as one greets, so the
                // proof is not sent on every route in parallel.
                greeted = await greetFirst(group, this.opening)
                opened = await answer(
                    greeted,
                    async (challenge) => ({
                        v: 1,
                        device: host.deviceId,
                        proof: await sha256(`${challenge}:${host.token}`),
                    }),
                    this.opening
                )
            } catch (e) {
                // A newer connect or a disconnect closed this attempt; it has nothing to say.
                if (generation !== this.generation) return
                lastError = denialText(`${(e as Error).message}`)
                // A revoked or broken credential is final; keep retrying anything else.
                if (/paired|credential/.test(lastError)) {
                    this.wanted = undefined
                    break
                }
                continue
            }
            if (generation !== this.generation) {
                closeQuietly(opened.socket)
                return
            }
            const transport = greeted.route.transport
            useRelayHostsStore.getState().updateHost(host.id, {
                name: opened.greeting.host || host.name,
                instance: opened.greeting.instance || host.instance,
                lastConnectedAt: Date.now(),
                lastTransport: transport,
            })
            this.adopt(opened, host, transport, greeted.route.url)
            // The link is up; a refresh that fails is a failed request, not a failed route.
            // A link that is really gone reports itself through the socket closing.
            await this.refresh().catch((e) => Logger.warn(`Relay: refresh failed: ${e}`))
            return
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
        // Handshakes still in flight are abandoned: their sockets close now, not whenever
        // the PC or a timeout gets to them.
        for (const cancel of [...this.opening.values()]) cancel()
        this.opening.clear()
        const socket = this.socket
        this.socket = undefined
        for (const [, pending] of this.pending) {
            clearTimeout(pending.timer)
            pending.reject(new Error('disconnected'))
        }
        this.pending.clear()
        this.frameListeners.clear()
        // A held action asked about on the old link cannot be answered on it any more; the
        // hold stays open on the PC and shows in the Inbox.
        useHoldPromptStore.getState().clear()
        if (socket) closeQuietly(socket)
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
        const socket = opened.socket
        if (this.keepalive) clearInterval(this.keepalive)
        this.keepalive = undefined
        this.socket = socket
        socket.onmessage = (message) =>
            this.onLine(typeof message.data === 'string' ? message.data : '')
        const lost = () => {
            // Only the live socket speaks for the link: an orphan from an earlier attempt
            // going away changes nothing.
            if (this.socket !== socket) return
            this.socket = undefined
            if (this.keepalive) clearInterval(this.keepalive)
            this.keepalive = undefined
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
        socket.onclose = lost
        socket.onerror = () => {}
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
            // dropped the flow). Its close event may never come, so the drop is handled here.
            this.request('bus.ping', {}).catch((e: Error) => {
                if (!/timed out/.test(e.message) || this.socket !== socket) return
                try {
                    socket.close()
                } catch {}
                lost()
            })
        }, KEEPALIVE_MS)
        ensureNotifyPermission()
        this.subscribe()
        // A watch is a lease on the connection; a new connection starts without one.
        if (this.resourceWatchers > 0) {
            this.request('app.resources.watch', { on: true }).catch(() => {})
        }
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
            if (parsed.stream !== 'pty') {
                this.onStreamFrame(parsed as StreamFrame)
                return
            }
            const frame = parsed as PtyFrame
            if (frame.stream === 'pty' && frame.session) {
                const listeners = this.frameListeners.get(frame.session)
                if (listeners && listeners.size > 0) {
                    let bytes: Uint8Array
                    try {
                        bytes = base64Decode(String(frame.data ?? ''))
                    } catch {
                        return
                    }
                    for (const [listener, attachment] of listeners) {
                        // A new epoch is a new process, and its bytes a new stream.
                        if (attachment.epoch !== frame.epoch) {
                            if (attachment.epoch !== undefined) attachment.decoder.reset()
                            attachment.epoch = frame.epoch
                        }
                        listener(frame, attachment.decoder.decode(bytes))
                    }
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
                // An answer from the engine proves the link, not just the handshake: only
                // now does the backoff start over. A door whose engine is down greets and
                // then closes, and that should back off like any other failure.
                this.reconnectDelay = RECONNECT_MIN_MS
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

    /**
     * Transitions only: the engine never emits an event per byte of output. `bus.subscribe`
     * replaces the connection's filter, so this always sends the whole list.
     */
    private subscribe() {
        const events =
            this.resourceWatchers > 0 ? [...EVENT_PATTERNS, 'resource.sample'] : EVENT_PATTERNS
        this.request('bus.subscribe', { events }).catch(() => {})
    }

    private onStreamFrame(frame: StreamFrame) {
        const key =
            frame.run_id ?? frame.mirror_id ?? frame.session ?? (undefined as string | undefined)
        const exact =
            key !== undefined ? this.streamListeners.get(`${frame.stream}:${key}`) : undefined
        const any = this.streamListeners.get(`${frame.stream}:*`)
        for (const listener of [...(exact ?? []), ...(any ?? [])]) {
            try {
                listener(frame)
            } catch (e) {
                Logger.warn(`Relay: stream listener failed: ${e}`)
            }
        }
    }

    /**
     * Hear a data-plane stream other than a session's PTY, e.g. `onStream('logcat', runId, …)`
     * for the log lines of a `device.run`/`device.build` this phone started (the engine sends
     * them only to the connection that asked). `key` is the frame's `run_id`, `mirror_id` or
     * `session`, or `*` for every frame of that stream. Returns the unsubscribe function.
     */
    onStream(stream: string, key: string | number, listener: StreamListener): () => void {
        const id = `${stream}:${key}`
        let set = this.streamListeners.get(id)
        if (!set) {
            set = new Set()
            this.streamListeners.set(id, set)
        }
        set.add(listener)
        return () => {
            const current = this.streamListeners.get(id)
            current?.delete(listener)
            if (current && current.size === 0) this.streamListeners.delete(id)
        }
    }

    /**
     * Live CPU/memory samples (`resource.sample`, about once a second) while at least one
     * caller wants them. Refcounted: every `watchResources(true)` needs its own
     * `watchResources(false)` (the `useResourceSamples` hook does both). Survives reconnects.
     */
    watchResources(on: boolean) {
        const before = this.resourceWatchers
        this.resourceWatchers = Math.max(0, before + (on ? 1 : -1))
        const after = this.resourceWatchers
        if ((before === 0) === (after === 0)) return
        if (useRelayStore.getState().status !== 'online') return
        this.subscribe()
        this.request('app.resources.watch', { on: after > 0 }).catch(() => {})
    }

    /**
     * A typed bus request with the op's own timeout. Rejects with `RelayRequestError` on a
     * typed refusal (including `held`; use `guarded` for mutations), or a plain `Error` when
     * offline or timed out.
     */
    call<T = any>(op: string, payload: object = {}, opts: RequestOptions = {}): Promise<T> {
        return this.request<T>(op, payload as Record<string, unknown>, opts)
    }

    /**
     * A mutation that the guardrail may hold. On `held`, the frozen request is fetched
     * (`guardrail.hold.get`) and shown in the global hold sheet; Allow confirms it
     * (`guardrail.confirm`) and returns the original op's result, Deny rejects the hold and
     * throws `RelayCancelledError` (test with `isCancelled(e)` and stay quiet). A confirmed
     * action that still fails (a cap, a protected path) throws `RelayRequestError`.
     * Desktop equivalent: relay-native code_git.rs `guarded()`.
     */
    async guarded<T = any>(
        op: string,
        payload: object = {},
        opts: RequestOptions = {}
    ): Promise<T> {
        try {
            return await this.call<T>(op, payload, opts)
        } catch (e) {
            if (!(e instanceof RelayRequestError) || e.error.kind !== 'held') throw e
            const confirm = e.error.confirm
            if (confirm?.op !== 'guardrail.confirm') throw e
            const holdId = confirm.payload.hold_id
            const inspection = await this.call<{ hold: RelayHoldFull; request: HeldRequest }>(
                'guardrail.hold.get',
                { hold_id: holdId }
            )
            // What would run must be what the person asked for, or they are asked about
            // something else. The engine never changes a user's payload, so any drift is a bug.
            if (
                inspection.request.op !== op ||
                !sameJson(JSON.parse(JSON.stringify(payload)), inspection.request.payload)
            ) {
                throw new Error(
                    'Held action does not match the requested operation. Inspect it in the Inbox.'
                )
            }
            const allow = await useHoldPromptStore.getState().ask({
                error: e.error,
                hold: inspection.hold,
                request: inspection.request,
            })
            if (!allow) {
                await this.call('guardrail.reject', {
                    hold_id: holdId,
                    reason: 'Denied from the phone',
                }).catch((err) => Logger.warn(`Relay: reject failed: ${err}`))
                this.refreshAttention().catch(() => {})
                throw new RelayCancelledError()
            }
            const confirmed = await this.call<{ hold: RelayHoldFull; outcome: BusResponse }>(
                'guardrail.confirm',
                { hold_id: holdId },
                { timeoutMs: Math.max(timeoutFor(op), timeoutFor('guardrail.confirm')) }
            )
            this.refreshAttention().catch(() => {})
            if (confirmed.outcome?.ok) return confirmed.outcome.result as T
            const failure = confirmed.outcome?.error
            if (failure) throw new RelayRequestError(failure)
            throw new Error('Held action was not completed')
        }
    }

    /** One bus request. Rejects with `RelayRequestError` on a typed refusal. */
    async request<T = any>(
        op: string,
        payload: Record<string, unknown> = {},
        opts: RequestOptions = {}
    ): Promise<T> {
        const socket = this.socket
        if (!socket || socket.readyState !== WebSocket.OPEN) {
            throw new Error('Not connected to a PC')
        }
        const id = uuid()
        const line = JSON.stringify({ v: 1, id: id, actor: 'user', op: op, payload: payload })
        const response = await new Promise<BusResponse>((resolve, reject) => {
            const timer = setTimeout(
                () => {
                    this.pending.delete(id)
                    reject(new Error(`${op} timed out`))
                },
                opts.timeoutMs ?? timeoutFor(op)
            )
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
            listeners = new Map()
            this.frameListeners.set(session, listeners)
        }
        listeners.set(listener, { decoder: new Utf8Stream(), epoch: from?.epoch })
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
