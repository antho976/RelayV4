import * as Notifications from 'expo-notifications'
import { router } from 'expo-router'
import { useCallback, useEffect } from 'react'
import { AppState, Linking, Platform } from 'react-native'
import { useMMKVBoolean } from 'react-native-mmkv'

import Alert from '@components/views/Alert'
import { AppSettings } from '@lib/constants/GlobalValues'
import { Characters } from '@lib/state/Characters'
import { Chats } from '@lib/state/Chat'
import { Logger } from '@lib/state/Logger'

import { notificationChannel } from './Channel'

const createChannel = async () => {
    if (Platform.OS !== 'android') return
    await Notifications.setNotificationChannelAsync(notificationChannel, {
        name: 'Relay',
        importance: Notifications.AndroidImportance.DEFAULT,
        vibrationPattern: [250, 0, 250, 250],
        lightColor: '#72bb98',
    })
}

// setupNotifications runs from startupApp, once the database is migrated
let startupDone = false
const afterStartup = new Set<() => void>()

export const setupNotifications = () => {
    Notifications.setNotificationHandler({
        handleNotification: async () => ({
            shouldPlaySound: false,
            shouldSetBadge: false,
            shouldShowAlert: false,
            shouldShowBanner: false,
            shouldShowList: false,
        }),
    })
    createChannel().catch((e) => Logger.error(`Failed to create notification channel: ${e}`))
    startupDone = true
    afterStartup.forEach((run) => run())
    afterStartup.clear()
}

/** Asks for POST_NOTIFICATIONS when it can, and points to Settings when it cannot. */
export async function registerForPushNotificationsAsync() {
    await createChannel()
    let permission = await Notifications.getPermissionsAsync()
    if (permission.granted) return true
    if (permission.canAskAgain) {
        permission = await Notifications.requestPermissionsAsync()
        return permission.granted
    }
    Alert.alert({
        title: 'Permission Required',
        description: 'Relay needs permission to send you notifications.',
        buttons: [
            {
                label: 'Cancel',
            },
            {
                label: 'Open Permissions',
                onPress: () => {
                    Linking.openSettings()
                },
            },
        ],
    })
    return false
}

let coldStartHandled = false
let lastHandled: string | undefined

/** A cold-start tap can reach both the listener and getLastNotificationResponse. */
const firstResponse = (response: Notifications.NotificationResponse) => {
    const id = response.notification.request.identifier
    if (id === lastHandled) return false
    lastHandled = id
    return true
}

export function useAppStateNotificationObserver() {
    const [autoLoad] = useMMKVBoolean(AppSettings.ChatOnStartup)
    const [useAuth] = useMMKVBoolean(AppSettings.LocallyAuthenticateUser)
    const { chatId: chatActive, setId } = Chats.useChat()
    const { setCard } = Characters.useCharacterStore()

    const redirect = useCallback(
        async (response: Notifications.NotificationResponse) => {
            // handled once: a response left in place is replayed on every later foreground
            Notifications.clearLastNotificationResponse()
            if (!firstResponse(response)) return
            const data = response.notification.request.content.data
            if (useAuth) return
            if (data?.relay) {
                // An agent on the PC needs a decision: land on the PC tab.
                router.navigate('/screens/RelayScreen')
                return
            }
            if (chatActive ?? autoLoad) return

            const chatId = data?.chatId as number | undefined
            const characterId = data?.characterId as number | undefined

            if (chatId && characterId) {
                Logger.info('Loading chat from notification')
                try {
                    await setId(chatId)
                    await setCard(characterId)
                    router.navigate('/screens/ChatScreen')
                } catch (e) {
                    Logger.error('Failed to load chat: ' + e)
                }
            }
        },
        [chatActive, autoLoad, useAuth, setId, setCard]
    )

    useEffect(() => {
        // a tap that cold-started the app waits for the database; later taps come to the listener
        const handleColdStart = () => {
            if (coldStartHandled) return
            coldStartHandled = true
            const response = Notifications.getLastNotificationResponse()
            if (response?.notification) redirect(response)
        }
        if (startupDone) handleColdStart()
        else afterStartup.add(handleColdStart)
        const subscription = Notifications.addNotificationResponseReceivedListener((response) => {
            if (startupDone) redirect(response)
        })
        const listener = AppState.addEventListener('change', async (nextState) => {
            if (nextState !== 'active') return
            if ((await Notifications.getPresentedNotificationsAsync()).length > 0) {
                await Notifications.dismissAllNotificationsAsync()
            }
        })

        return () => {
            afterStartup.delete(handleColdStart)
            subscription.remove()
            listener.remove()
        }
    }, [redirect])
}
