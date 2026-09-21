import React from 'react'
import { Text, View } from 'react-native'
import { useMMKVBoolean } from 'react-native-mmkv'

import ThemedSwitch from '@components/input/ThemedSwitch'
import SectionTitle from '@components/text/SectionTitle'
import { AppSettings } from '@lib/constants/GlobalValues'
import { useAppModeStore } from '@lib/state/AppMode'
import { Theme } from '@lib/theme/ThemeManager'

/**
 * Where the app draws its line: on-device first, a remote API only when the phone cannot do
 * the work itself, and the PC link never involved in chats.
 */
const PrivacySettings = () => {
    const { color, spacing } = Theme.useTheme()
    const [localFirst, setLocalFirst] = useMMKVBoolean(AppSettings.LocalFirstFallback)
    const [keepOn, setKeepOn] = useMMKVBoolean(AppSettings.RelayKeepScreenOn)
    const [notify, setNotify] = useMMKVBoolean(AppSettings.RelayNotify)
    const appMode = useAppModeStore((state) => state.appMode)

    return (
        <View style={{ rowGap: 8 }}>
            <SectionTitle>Privacy</SectionTitle>
            <Text style={{ color: color.text._400, marginBottom: spacing.s }}>
                {appMode === 'local'
                    ? 'Chats run on this phone. Nothing is sent anywhere unless you allow the fallback below.'
                    : 'Remote mode is on: chats go to the API you selected. Switch to Local in the drawer to keep them on the phone.'}
            </Text>
            <ThemedSwitch
                label="Fall back to an API when no local model can run"
                value={localFirst}
                onChangeValue={setLocalFirst}
                description="In Local mode, if no model is loaded and none is set to auto-load, use the active API connection for that message instead of failing. You are told every time this happens. Off means the phone never sends a chat anywhere while in Local mode."
            />
            <ThemedSwitch
                label="Tell me when an agent on the PC needs me"
                value={notify}
                onChangeValue={setNotify}
                description="While connected and the app is in the background, a notification is shown when an agent is held by a guardrail, gets blocked, or finishes. Nothing is sent through any push service; it comes straight from the PC link."
            />
            <ThemedSwitch
                label="Keep the screen on in a PC terminal"
                value={keepOn}
                onChangeValue={setKeepOn}
                description="While a session's terminal is open on the PC tab, the phone does not sleep."
            />
        </View>
    )
}

export default PrivacySettings
