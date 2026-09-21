/**
 * Local notifications for the moments an agent on the PC needs a person: a guardrail hold, a
 * blocked session, a finished one. They come from the PC link itself — no push service is
 * involved — so they only fire while the app is connected, and only when it is not in front.
 */
import * as Notifications from 'expo-notifications'
import { AppState } from 'react-native'

import { AppSettings } from '@lib/constants/GlobalValues'
import { registerForPushNotificationsAsync } from '@lib/notifications/Notifications'
import { Logger } from '@lib/state/Logger'
import { mmkv } from '@lib/storage/MMKV'

import type { BusEvent } from './RelayClient'

let permissionAsked = false

/** Ask once, and only when nobody has answered yet: a denial is never nagged about. */
export const ensureNotifyPermission = async () => {
    if (permissionAsked || !mmkv.getBoolean(AppSettings.RelayNotify)) return
    permissionAsked = true
    try {
        // The installed typings do not resolve the base permission shape; the fields are stable.
        const current = (await Notifications.getPermissionsAsync()) as {
            granted?: boolean
            canAskAgain?: boolean
        }
        if (!current.granted && current.canAskAgain !== false) {
            await registerForPushNotificationsAsync()
        }
    } catch (e) {
        Logger.debug(`Relay: notification permission check failed: ${e}`)
    }
}

export const notifyIfAway = async (title: string, body: string) => {
    if (!mmkv.getBoolean(AppSettings.RelayNotify)) return
    if (AppState.currentState === 'active') return
    try {
        await Notifications.scheduleNotificationAsync({
            content: {
                title: title,
                body: body,
                sound: mmkv.getBoolean(AppSettings.PlayNotificationSound),
                vibrate: mmkv.getBoolean(AppSettings.VibrateNotification)
                    ? [250, 125, 250]
                    : undefined,
                data: { relay: true },
            },
            trigger: null,
        })
    } catch (e) {
        Logger.debug(`Relay: could not show a notification: ${e}`)
    }
}

/** Turn the bus events that mean "needs you" into a notification, when the app is away. */
export const attentionFromEvent = (event: BusEvent) => {
    const payload = event.payload ?? {}
    if (event.ev === 'guardrail.held') {
        const who = payload.session ?? 'An agent'
        notifyIfAway(
            'Agent needs permission',
            `${who} is held by ${payload.policy ?? 'a guardrail'}.`
        )
        return
    }
    if (event.ev === 'notify.new') {
        const who = payload.session ?? 'An agent'
        switch (payload.category) {
            case 'agent_done':
                notifyIfAway('Agent finished', `${who} reports done and is waiting for review.`)
                return
            case 'agent_blocked':
                notifyIfAway('Agent blocked', `${who} is blocked and needs you.`)
                return
            default:
                return
        }
    }
}
