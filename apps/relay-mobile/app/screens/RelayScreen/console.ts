import { Theme } from '@lib/theme/ThemeManager'

type ThemeColor = ReturnType<typeof Theme.useTheme>['color']

/**
 * The console's own colours, the ones the desktop wall uses and the theme does not carry:
 * the screen-black a terminal sits on, its text, and the one green that means "alive".
 */
export const palette = {
    ink: '#0a0a0b',
    paper: '#ececea',
    live: '#2ec469',
} as const

/**
 * Relay's session lamp: colour only where it means a state. Live green for running, held
 * red for blocked, the waiting amber for a session that is spawning, quiet ink for the rest.
 */
export const stateColor = (state: string, color: ThemeColor): string => {
    switch (state) {
        case 'running':
            return palette.live
        case 'blocked':
            return color.error._300
        case 'spawning':
            return color.quote
        case 'idle':
            return color.text._400
        default:
            return color.neutral._700
    }
}

/** The link's lamp: the same three lights, for a connection instead of a session. */
export const linkColor = (
    status: 'offline' | 'connecting' | 'online',
    color: ThemeColor
): string => {
    switch (status) {
        case 'online':
            return palette.live
        case 'connecting':
            return color.quote
        default:
            return color.neutral._700
    }
}

/** "3m" for three minutes ago; empty when there was never any output. */
export const ago = (iso: string | null | undefined, now = Date.now()): string => {
    if (!iso) return ''
    const then = Date.parse(iso)
    if (Number.isNaN(then)) return ''
    const seconds = Math.max(0, Math.floor((now - then) / 1000))
    if (seconds < 60) return `${seconds}s`
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m`
    if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`
    return `${Math.floor(seconds / 86400)}d`
}
