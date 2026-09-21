import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Text, TouchableOpacity } from 'react-native'
import { useMMKVBoolean } from 'react-native-mmkv'

import appConfig from '@appconfig'
import Drawer from '@components/views/Drawer'
import { AppSettings } from '@lib/constants/GlobalValues'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import AppModeToggle from './AppModeToggle'
import RouteList from './RouteList'
import UserInfo from './UserInfo'

const SettingsDrawer = () => {
    const { t } = useTranslation()
    const { color, spacing } = Theme.useTheme()
    const [devMode, setDevMode] = useMMKVBoolean(AppSettings.DevMode)
    const [tapCount, setTapCount] = useState(0)

    // tapping the version 7 times toggles dev mode (previously on the About page)
    const handleVersionTap = () => {
        const next = tapCount + 1
        if (next >= 7) {
            setTapCount(0)
            setDevMode(!devMode)
            Logger.infoToast(
                devMode ? t('common.labels.devModeDisabled') : t('common.labels.devModeEnabled')
            )
            return
        }
        setTapCount(next)
    }

    return (
        <Drawer.Body
            drawerID={Drawer.ID.SETTINGS}
            drawerStyle={{
                width: '60%',
                paddingBottom: spacing.xl,
            }}>
            <UserInfo />
            <AppModeToggle />
            <RouteList />
            <TouchableOpacity
                activeOpacity={0.8}
                onPress={handleVersionTap}
                style={{ alignSelf: 'center', marginTop: spacing.l, marginBottom: spacing.xl2 }}>
                <Text style={{ color: color.text._300 }}>
                    {(__DEV__ || devMode) && t('common.labels.devMode') + '\t'}
                    {t('about.versionPrefix') + appConfig.expo.version}
                </Text>
            </TouchableOpacity>
        </Drawer.Body>
    )
}

export default SettingsDrawer
