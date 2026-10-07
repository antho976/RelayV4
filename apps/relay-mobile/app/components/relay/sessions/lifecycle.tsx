import React, { useState } from 'react'
import { Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import { Field, SwitchRow } from '@components/relay/Kit'
import { confirm, Sheet, useSheetStyles } from '@components/relay/Sheet'
import { isCancelled, relay, RelayRequestError } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'

import { megabytes } from './links'

/** What a lifecycle action needs to know of a session. */
export type LifecycleSession = {
    name: string
    state: string
    worktree?: string
}

export type LifecycleKey = 'start' | 'park' | 'wake' | 'resume' | 'clear' | 'close'

export type LifecycleAction = { key: LifecycleKey; label: string; destructive?: boolean }

/**
 * The lifecycle actions a session in `state` offers, in the order the desktop wall shows them
 * (relay-native shell.rs `session_actions`): Start a session that was allocated but never
 * spawned, Wake a parked one, Resume a restorable or exited one (Clear context too for a
 * restorable one), Park anything live; Close is always last.
 */
export const lifecycleActions = (state: string): LifecycleAction[] => {
    const actions: LifecycleAction[] = []
    if (state === 'created') actions.push({ key: 'start', label: 'Start' })
    else if (state === 'parked') actions.push({ key: 'wake', label: 'Wake' })
    else if (state === 'restorable' || state === 'exited')
        actions.push({ key: 'resume', label: 'Resume' })
    else if (state !== 'closed' && state !== 'spawning')
        actions.push({ key: 'park', label: 'Park' })
    if (state === 'restorable') actions.push({ key: 'clear', label: 'Clear context' })
    if (state !== 'closed') actions.push({ key: 'close', label: 'Close…', destructive: true })
    return actions
}

const OPS: Record<Exclude<LifecycleKey, 'start' | 'close' | 'clear'>, string> = {
    park: 'session.park',
    wake: 'session.wake',
    resume: 'session.resume',
}

const report = (e: unknown) => {
    if (isCancelled(e)) return
    Logger.errorToast(`${(e as Error).message}`)
}

/**
 * Send a request that may delete a session's worktree. The PC refuses `worktree.dirty` when the
 * worktree holds uncommitted work; the person is then asked, and only a yes sends it again with
 * `discard_changes`. Resolves `undefined` when they keep the work.
 */
export const confirmingDiscard = async <T,>(
    name: string,
    send: (discardChanges: boolean) => Promise<T>
): Promise<T | undefined> => {
    try {
        return await send(false)
    } catch (e) {
        if (!(e instanceof RelayRequestError) || e.error.code !== 'worktree.dirty') throw e
        const yes = await confirm({
            title: `Discard ${name}'s changes?`,
            message: `${e.error.message}. Removing the worktree deletes them for good; commit or stash them first to keep them.`,
            confirmLabel: 'Discard changes',
            destructive: true,
        })
        return yes ? await send(true) : undefined
    }
}

/**
 * A session's lifecycle, as the desktop runs it: `run(key)` performs the action, asking first
 * where the desktop asks (Clear context), or opening its sheet (Start with an optional prompt,
 * Close with the cleanup switches). Render `sheets` once in the screen that uses it.
 * `onClosed` runs after a successful close, e.g. to leave the session's screen.
 */
export const useSessionLifecycle = (
    session: LifecycleSession | undefined,
    options: { onClosed?: () => void; onStarted?: () => void } = {}
) => {
    const [busy, setBusy] = useState<LifecycleKey | undefined>(undefined)
    const [starting, setStarting] = useState(false)
    const [closing, setClosing] = useState(false)
    const [prompt, setPrompt] = useState('')
    const [removeWorktree, setRemoveWorktree] = useState(false)
    const [purgeBuild, setPurgeBuild] = useState(false)
    const styles = useSheetStyles()
    const name = session?.name ?? ''

    const perform = async (key: LifecycleKey, op: string, payload: object) => {
        setBusy(key)
        try {
            const result = await relay.guarded(op, payload)
            relay.refresh().catch(() => {})
            return result
        } finally {
            setBusy(undefined)
        }
    }

    const run = async (key: LifecycleKey) => {
        if (!session || busy) return
        try {
            switch (key) {
                case 'start':
                    setPrompt('')
                    setStarting(true)
                    return
                case 'close':
                    setRemoveWorktree(false)
                    setPurgeBuild(false)
                    setClosing(true)
                    return
                case 'clear':
                    if (
                        !(await confirm({
                            title: 'Clear context?',
                            message:
                                'Start fresh in this same session and worktree. The saved provider conversation context is cleared.',
                            confirmLabel: 'Clear and start',
                            destructive: true,
                        }))
                    )
                        return
                    await perform(key, 'session.clear_restorable', { session: name })
                    return
                default:
                    await perform(key, OPS[key], { session: name })
            }
        } catch (e) {
            report(e)
        }
    }

    const start = async () => {
        setStarting(false)
        const text = prompt.trim()
        try {
            // An omitted prompt keeps whatever assignment was allocated with the session.
            await perform(
                'start',
                'session.spawn',
                text ? { session: name, prompt: text } : { session: name }
            )
            options.onStarted?.()
        } catch (e) {
            report(e)
        }
    }

    const close = async () => {
        setClosing(false)
        try {
            const result = await confirmingDiscard(name, (discard) =>
                perform('close', 'session.close', {
                    session: name,
                    remove_worktree: removeWorktree,
                    purge_build: purgeBuild,
                    ...(discard ? { discard_changes: true } : {}),
                })
            )
            if (result === undefined) return
            const freed = Number(result?.freed_mb ?? 0)
            Logger.infoToast(
                freed > 0 ? `Closed ${name} · freed ${megabytes(freed)}` : `Closed ${name}`
            )
            options.onClosed?.()
        } catch (e) {
            report(e)
        }
    }

    const sheets = (
        <>
            <Sheet visible={starting} onDismiss={() => setStarting(false)}>
                <View style={styles.body}>
                    <Text style={styles.title}>Start {name}</Text>
                    <Text style={styles.message}>
                        Launch the agent&apos;s CLI in its worktree. Leave the prompt empty to keep
                        the assignment it was created with.
                    </Text>
                    <Field
                        label="Opening prompt"
                        value={prompt}
                        onChangeText={setPrompt}
                        placeholder="Optional"
                        multiline
                        lines={4}
                    />
                    <View style={styles.actions}>
                        <ThemedButton
                            label="Cancel"
                            variant="secondary"
                            onPress={() => setStarting(false)}
                        />
                        <ThemedButton label="Start" onPress={start} />
                    </View>
                </View>
            </Sheet>
            <Sheet visible={closing} onDismiss={() => setClosing(false)}>
                <View style={styles.body}>
                    <Text style={styles.title}>Close {name}?</Text>
                    <Text style={styles.message}>
                        Stops its process and takes it off the wall. The branch is always kept.
                        {session?.worktree ? `\n${session.worktree}` : ''}
                    </Text>
                    <View>
                        <SwitchRow
                            label="Remove worktree"
                            description="Delete the checkout after closing. Refused while a paired session still uses it."
                            value={removeWorktree}
                            onChange={setRemoveWorktree}
                        />
                        <SwitchRow
                            label="Purge build output"
                            description="Free the disk its build directories use."
                            value={purgeBuild}
                            onChange={setPurgeBuild}
                        />
                    </View>
                    <View style={styles.actions}>
                        <ThemedButton
                            label="Cancel"
                            variant="secondary"
                            onPress={() => setClosing(false)}
                        />
                        <ThemedButton label="Close session" variant="critical" onPress={close} />
                    </View>
                </View>
            </Sheet>
        </>
    )

    return {
        actions: session ? lifecycleActions(session.state) : [],
        run: run,
        busy: busy,
        sheets: sheets,
    }
}
