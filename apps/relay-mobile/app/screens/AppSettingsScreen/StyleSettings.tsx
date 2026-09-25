import AntDesign from '@react-native-vector-icons/ant-design/static'
import { Image } from 'expo-image'
import React from 'react'
import { useTranslation } from 'react-i18next'
import { StyleSheet, Text, View } from 'react-native'
import { useShallow } from 'zustand/react/shallow'

import ThemedButton from '@components/buttons/ThemedButton'
import SettingsGroup from '@components/theme/SettingsGroup'
import Alert from '@components/views/Alert'
import { useBackgroundStore } from '@lib/state/BackgroundImage'
import { Theme } from '@lib/theme/ThemeManager'
import { AppDirectory } from '@lib/utils/File'

import { ChatTextSettings } from './ChatStyleSettings'
import { ThemeSection } from './ColorSelector'

/** The image behind every chat: a thumbnail of it, and buttons to choose, replace or remove it. */
const BackgroundSettings = () => {
    const { t } = useTranslation()
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const { chatBackground, importBackground, deleteBackground } = useBackgroundStore(
        useShallow((state) => ({
            chatBackground: state.image,
            importBackground: state.importImage,
            deleteBackground: state.removeImage,
        }))
    )

    const confirmDelete = () =>
        Alert.alert({
            title: t('settings.style.alert.deleteBackground.title'),
            description: t('settings.style.alert.deleteBackground.description'),
            buttons: [
                { label: t('common.actions.cancel') },
                {
                    label: t('settings.style.alert.deleteBackground.confirm'),
                    type: 'warning',
                    onPress: deleteBackground,
                },
            ],
        })

    return (
        <SettingsGroup title={t('settings.style.background')}>
            <View style={styles.row}>
                <View style={styles.thumb}>
                    {chatBackground ? (
                        <Image
                            cachePolicy="none"
                            contentFit="cover"
                            style={StyleSheet.absoluteFill}
                            source={{ uri: AppDirectory.Assets + chatBackground }}
                        />
                    ) : (
                        <AntDesign name="picture" size={24} color={color.text._500} />
                    )}
                </View>
                <View style={styles.info}>
                    <Text style={styles.detail}>
                        {chatBackground
                            ? t('settings.style.backgroundSet')
                            : t('settings.style.backgroundNone')}
                    </Text>
                    <ThemedButton
                        label={
                            chatBackground
                                ? t('settings.style.replaceImage')
                                : t('settings.style.chooseImage')
                        }
                        iconName="picture"
                        iconSize={16}
                        variant="secondary"
                        onPress={importBackground}
                    />
                    {chatBackground && (
                        <ThemedButton
                            label={t('settings.style.removeImage')}
                            iconName="delete"
                            iconSize={16}
                            variant="critical"
                            onPress={confirmDelete}
                        />
                    )}
                </View>
            </View>
        </SettingsGroup>
    )
}

/** Settings → App → Appearance: the theme, then how chats read and what sits behind them. */
const StyleSettings = () => {
    const { spacing } = Theme.useTheme()
    return (
        <View style={{ rowGap: spacing.xl2 }}>
            <ThemeSection />
            <ChatTextSettings showFullPreview />
            <BackgroundSettings />
        </View>
    )
}

export default StyleSettings

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        row: {
            flexDirection: 'row',
            columnGap: spacing.xl,
            alignItems: 'center',
        },
        thumb: {
            width: 76,
            height: 120,
            borderRadius: 16,
            overflow: 'hidden',
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.neutral._100,
            borderWidth: 1,
            borderColor: color.neutral._300,
        },
        info: {
            flex: 1,
            rowGap: spacing.m,
        },
        detail: {
            color: color.text._400,
            fontSize: fontSize.m,
        },
    })
}
