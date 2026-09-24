import { useCallback, useEffect, useRef, useState } from 'react'
import { AppState } from 'react-native'

import { RelayDevice, requireRelayDevice } from './RelayDeviceModule'

/** Whether "Ask in Relay" shows in the text-selection menu. False where unsupported. */
export const isProcessTextEnabled = async (): Promise<boolean> =>
    RelayDevice ? RelayDevice.isProcessTextEnabled() : false

/** Shows or hides the menu entry. Resolves to the state actually in effect afterwards. */
export const setProcessTextEnabled = async (enabled: boolean): Promise<boolean> =>
    requireRelayDevice('setProcessTextEnabled').setProcessTextEnabled(enabled)

/** Returns text sent through the menu entry, once; null when there is none. */
export const consumeProcessText = (): string | null => RelayDevice?.consumeProcessText() ?? null

/**
 * Calls `onText` with text sent through the menu entry: text that launched the app, and text
 * that arrives while it runs. Each text is delivered exactly once, however often the app
 * resumes. The latest `onText` is always used, so it need not be memoized.
 */
export const useProcessTextOnForeground = (onText: (text: string) => void) => {
    const handler = useRef(onText)
    useEffect(() => {
        handler.current = onText
    })

    useEffect(() => {
        const native = RelayDevice
        if (!native) return
        const deliver = () => {
            const text = native.consumeProcessText()
            if (text) handler.current(text)
        }
        deliver()
        const onEvent = native.addListener('onProcessText', deliver)
        // a fallback in case the event fires before this listener exists
        const onActive = AppState.addEventListener('change', (state) => {
            if (state === 'active') deliver()
        })
        return () => {
            onEvent.remove()
            onActive.remove()
        }
    }, [])
}

/** State and setter for a settings switch bound to the menu entry. */
export const useProcessTextSetting = () => {
    const [enabled, setEnabledState] = useState(false)
    const [ready, setReady] = useState(false)

    useEffect(() => {
        let live = true
        isProcessTextEnabled()
            .then((value) => live && setEnabledState(value))
            .catch(() => {})
            .finally(() => live && setReady(true))
        return () => {
            live = false
        }
    }, [])

    /** Resolves to false when the change failed; the state then shows what is really in effect. */
    const setEnabled = useCallback(async (value: boolean): Promise<boolean> => {
        try {
            setEnabledState(await setProcessTextEnabled(value))
            return true
        } catch {
            setEnabledState(await isProcessTextEnabled().catch(() => false))
            return false
        }
    }, [])

    const supported = RelayDevice !== null
    return { enabled, setEnabled, ready, supported }
}
