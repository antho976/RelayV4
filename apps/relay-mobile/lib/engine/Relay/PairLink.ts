/**
 * The `relay://pair` link a PC prints (as text and as a QR code) when you run
 * `relay remote pair`. It carries the one-time code and every route to the engine: direct
 * LAN addresses first, then the rendezvous the PC dials out to, if one is configured.
 */

export const RELAY_DEFAULT_PORT = 7420

export type PairLink = {
    code: string
    host: string
    hostId: string
    instance: string
    /** `ws://192.168.1.20:7420`, … — the same network only. */
    direct: string[]
    /** `wss://server/join/<room>` — anywhere, through the person's own server. */
    via?: string
}

const decode = (value: string) => {
    try {
        return decodeURIComponent(value.replace(/\+/g, ' '))
    } catch {
        return value
    }
}

/**
 * Parse a `relay://pair?…` link. Returns `undefined` for anything else, so a scanned QR that is
 * not ours is ignored rather than misread.
 */
export const parsePairLink = (text: string): PairLink | undefined => {
    const trimmed = text.trim()
    const match = /^relay:\/\/pair\?(.*)$/i.exec(trimmed)
    if (!match) return undefined
    const params: Record<string, string> = {}
    for (const pair of match[1].split('&')) {
        if (!pair) continue
        const eq = pair.indexOf('=')
        const key = decode(eq === -1 ? pair : pair.slice(0, eq))
        const value = decode(eq === -1 ? '' : pair.slice(eq + 1))
        params[key] = value
    }
    if (params.v !== '1' || !params.code) return undefined
    const direct = (params.direct ?? '')
        .split(',')
        .map((item) => item.trim())
        .filter((item) => /^wss?:\/\//i.test(item))
    const via = params.via && /^wss?:\/\//i.test(params.via) ? params.via : undefined
    if (direct.length === 0 && !via) return undefined
    return {
        code: params.code,
        host: params.host || 'Relay PC',
        hostId: params.id || '',
        instance: params.instance || 'stable',
        direct: direct,
        via: via,
    }
}

/**
 * What a person types instead of scanning: `192.168.1.20`, `192.168.1.20:7420`,
 * `ws://…`, `wss://…`, or a rendezvous join URL. Anything else is refused.
 */
export const parseManualAddress = (text: string): string | undefined => {
    const trimmed = text.trim()
    if (!trimmed) return undefined
    if (/^wss?:\/\//i.test(trimmed)) return trimmed.replace(/\/+$/, '')
    if (/^https?:\/\//i.test(trimmed)) {
        return trimmed.replace(/^http/i, 'ws').replace(/\/+$/, '')
    }
    if (/^[a-z0-9.-]+(:\d{1,5})?$/i.test(trimmed)) {
        const withPort = trimmed.includes(':') ? trimmed : `${trimmed}:${RELAY_DEFAULT_PORT}`
        return `ws://${withPort}`
    }
    return undefined
}
