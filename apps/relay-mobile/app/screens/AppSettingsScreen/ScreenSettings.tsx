import * as KeepAwake from 'expo-keep-awake'
import React from 'react'
import { useTranslation } from 'react-i18next'
import { useMMKVBoolean } from 'react-native-mmkv'

import ThemedSwitch from '@components/input/ThemedSwitch'
import SettingsGroup from '@components/theme/SettingsGroup'
import { AppSettings } from '@lib/constants/GlobalValues'

const ScreenSettings = () => {
    const { t } = useTranslation()
    const [unlockOrientation, setUnlockOrientation] = useMMKVBoolean(AppSettings.UnlockOrientation)
    const [keepAwake, setKeepAwake] = useMMKVBoolean(AppSettings.KeepAwake)
    return (
        <SettingsGroup title={t('settings.screen.title')} style={{ rowGap: 0 }}>
            <ThemedSwitch
                label={t('settings.screen.unlockOrientation')}
                description={t('settings.screen.unlockOrientationDescription')}
                value={unlockOrientation}
                onChangeValue={setUnlockOrientation}
            />

            <ThemedSwitch
                label={t('settings.screen.keepAwake')}
                description={t('settings.screen.keepAwakeDescription')}
                value={keepAwake}
                onChangeValue={(value) => {
                    setKeepAwake(value)
                    if (value) KeepAwake.activateKeepAwakeAsync()
                    else KeepAwake.deactivateKeepAwake()
                }}
            />
        </SettingsGroup>
    )
}

export default ScreenSettings
