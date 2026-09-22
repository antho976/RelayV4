import { useRouter } from 'expo-router'
import React from 'react'
import { Text, TouchableOpacity, View } from 'react-native'
import { useMMKVBoolean } from 'react-native-mmkv'

import ThemedSwitch from '@components/input/ThemedSwitch'
import SectionTitle from '@components/text/SectionTitle'
import { AppSettings } from '@lib/constants/GlobalValues'
import { useAppModeStore } from '@lib/state/AppMode'
import { Theme } from '@lib/theme/ThemeManager'

/**
 * Where the app draws its line: on-device first, an API only when the phone cannot do the
 * work itself, and the PC link never involved in chats. What the PC link may do to the phone
 * is decided next to the PCs themselves, under PC → Paired PCs.
 */
const PrivacySettings = () => {
    const { color, spacing } = Theme.useTheme()
    const router = useRouter()
    const [localFirst, setLocalFirst] = useMMKVBoolean(AppSettings.LocalFirstFallback)
    const appMode = useAppModeStore((state) => state.appMode)

    return (
        <View style={{ rowGap: 8 }}>
            <SectionTitle>Privacy</SectionTitle>
            <Text style={{ color: color.text._400, marginBottom: spacing.s }}>
                {appMode === 'local'
                    ? 'Chats run on this phone. Nothing is sent anywhere unless you allow the fallback below.'
                    : 'API mode is on: chats go to the API you selected. Switch to On device in the drawer to keep them on the phone.'}
            </Text>
            <ThemedSwitch
                label="Fall back to an API when no local model can run"
                value={localFirst}
                onChangeValue={setLocalFirst}
                description="On device, if no model is loaded and none is set to auto-load, use the active API connection for that message instead of failing. You are told every time this happens. Off means the phone never sends a chat anywhere while on device."
            />
            <TouchableOpacity onPress={() => router.push('/screens/RelayScreen/Hosts')}>
                <Text style={{ color: color.text._400, paddingVertical: spacing.m }}>
                    The PC link never carries a chat. Its notifications and screen setting are under{' '}
                    <Text style={{ color: color.primary._700 }}>PC → Paired PCs</Text>.
                </Text>
            </TouchableOpacity>
        </View>
    )
}

export default PrivacySettings
