import { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { useLocalSearchParams } from 'expo-router'
import React from 'react'
import { View } from 'react-native'
import { KeyboardAwareScrollView } from 'react-native-keyboard-controller'

import HeaderTitle from '@components/views/HeaderTitle'
import { Theme } from '@lib/theme/ThemeManager'

import CharacterSettings from './CharacterSettings'
import ChatSettings from './ChatSettings'
import ChatWindowSettings from './ChatWindowSettings'
import DatabaseSettings from './DatabaseSettings'
import GeneratingSettings from './GeneratingSettings'
import NotificationSettings from './NotificationSettings'
import PrivacySettings from './PrivacySettings'
import ScreenSettings from './ScreenSettings'
import SecuritySettings from './SecuritySettings'
import StyleSettings from './StyleSettings'

/** The app's own settings, grouped the way a person looks for them. */
export const SECTIONS = {
    appearance: {
        title: 'Appearance',
        detail: 'Theme, background, screen',
        icon: 'skin',
        parts: [StyleSettings, ScreenSettings],
    },
    chat: {
        title: 'Chat',
        detail: 'Messages, generation, characters',
        icon: 'message',
        parts: [ChatSettings, ChatWindowSettings, GeneratingSettings, CharacterSettings],
    },
    privacy: {
        title: 'Privacy & security',
        detail: 'Lock, hidden chats, what leaves the phone',
        icon: 'lock',
        parts: [PrivacySettings, SecuritySettings],
    },
    notifications: {
        title: 'Notifications',
        detail: 'Sounds and alerts',
        icon: 'bell',
        parts: [NotificationSettings],
    },
    data: {
        title: 'Data',
        detail: 'Export, import, reset',
        icon: 'database',
        parts: [DatabaseSettings],
    },
} satisfies Record<
    string,
    { title: string; detail?: string; icon: AntDesignIconName; parts: React.FC[] }
>

export type SectionId = keyof typeof SECTIONS

/** One settings category: the existing settings blocks for it, on a page of their own. */
const SettingsSection = () => {
    const { spacing } = Theme.useTheme()
    const { id } = useLocalSearchParams<{ id: string }>()
    const section = SECTIONS[(id as SectionId) in SECTIONS ? (id as SectionId) : 'appearance']
    return (
        <KeyboardAwareScrollView
            style={{ paddingHorizontal: spacing.xl2 }}
            contentContainerStyle={{ rowGap: spacing.xl, paddingTop: spacing.l }}>
            <HeaderTitle title={section.title} />
            {section.parts.map((Part, index) => (
                <Part key={index} />
            ))}
            <View style={{ paddingVertical: spacing.xl3 }} />
        </KeyboardAwareScrollView>
    )
}

export default SettingsSection
