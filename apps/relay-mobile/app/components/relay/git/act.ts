import { useCallback, useRef, useState } from 'react'

import { isCancelled, relay, RelayRequestError } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'

/** The message a person should read for a failed request: the engine's own words, and its hint. */
export const problemText = (e: unknown) => {
    if (e instanceof RelayRequestError)
        return e.error.hint ? `${e.error.message} (${e.error.hint})` : e.error.message
    return `${(e as Error)?.message ?? e}`
}

/**
 * Run mutations one at a time from a screen: `busy` names the one in flight, a Deny on a held
 * action stays quiet, any other failure is a toast. `run` resolves with the op's result, or
 * undefined when it failed or was denied.
 *
 *     const { busy, run } = useGuardedAction()
 *     await run('git.stage', { project_id, worktree, paths }, 'Staged')
 */
export const useGuardedAction = () => {
    const [busy, setBusy] = useState<string | undefined>(undefined)
    const inFlight = useRef(false)
    const run = useCallback(
        async <T = any>(
            op: string,
            payload: object,
            success?: string | ((result: T) => string | undefined)
        ): Promise<T | undefined> => {
            if (inFlight.current) return undefined
            inFlight.current = true
            setBusy(op)
            try {
                const result = await relay.guarded<T>(op, payload)
                const note = typeof success === 'function' ? success(result) : success
                if (note) Logger.infoToast(note)
                return result
            } catch (e) {
                if (!isCancelled(e)) Logger.errorToast(problemText(e))
                return undefined
            } finally {
                inFlight.current = false
                setBusy(undefined)
            }
        },
        []
    )
    return { busy, run }
}
