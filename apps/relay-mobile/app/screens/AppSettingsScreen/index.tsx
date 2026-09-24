import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { useMMKVBoolean } from 'react-native-mmkv'

import appConfig from '@appconfig'
import HeaderTitle from '@components/views/HeaderTitle'
import AppModeToggle from '@components/views/SettingsDrawer/AppModeToggle'
import { AppSettings } from '@lib/constants/GlobalValues'
import { useAppMode } from '@lib/state/AppMode'
import { Characters } from '@lib/state/Characters'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { SECTIONS, SectionId } from './Section'

type Row = { label: string; icon: AntDesignIconName; detail?: string; onPress: () => void }

const Card: React.FC<{ rows: Row[] }> = ({ rows }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <View style={styles.card}>
            {rows.map((row, index) => (
                <TouchableOpacity
                    key={row.label}
                    style={[styles.row, index > 0 && styles.rowDivider]}
                    onPress={row.onPress}>
                    <AntDesign name={row.icon} size={22} color={color.text._200} />
                    <View style={{ flex: 1 }}>
                        <Text style={styles.rowLabel}>{row.label}</Text>
                        {!!row.detail && <Text style={styles.rowDetail}>{row.detail}</Text>}
                    </View>
                </TouchableOpacity>
            ))}
        </View>
    )
}

/**
 * Settings, reached from the profile bubble: who you are on top, then one card per kind of
 * setting — the models, the PC, and the app itself — each row one tap from its page.
 */
const SettingsScreen = () => {
    const styles = useStyles()
    const { t } = useTranslation()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const { appMode } = useAppMode()
    const userName = Characters.useUserStore((state) => state.card?.name ?? 'You')
    const [devMode, setDevMode] = useMMKVBoolean(AppSettings.DevMode)
    const [taps, setTaps] = useState(0)

    const section = (id: SectionId) => () =>
        router.push({ pathname: '/screens/AppSettingsScreen/Section', params: { id } })
    const page = (path: string) => () => router.push(path as never)

    const models: Row[] = [
        appMode === 'remote'
            ? {
                  label: t('navigation.api'),
                  icon: 'api',
                  onPress: page('/screens/ConnectionsManagerScreen'),
              }
            : {
                  label: t('navigation.models'),
                  icon: 'branches',
                  onPress: page('/screens/ModelManagerScreen'),
              },
        {
            label: t('navigation.sampler'),
            icon: 'control',
            onPress: page('/screens/SamplerManagerScreen'),
        },
        {
            label: t('navigation.formatting'),
            icon: 'profile',
            onPress: page('/screens/FormattingManagerScreen'),
        },
        {
            label: t('navigation.lorebooks'),
            icon: 'book',
            onPress: page('/screens/LorebookManagerScreen'),
        },
        { label: t('navigation.tts'), icon: 'sound', onPress: page('/screens/TTSManagerScreen') },
    ]
    const connection: Row[] = [
        {
            label: 'Paired PCs',
            icon: 'desktop',
            detail: 'Routes, Tailscale, pairing',
            onPress: page('/screens/RelayScreen/Hosts'),
        },
    ]
    const app: Row[] = (Object.keys(SECTIONS) as SectionId[]).map((id) => ({
        label: SECTIONS[id].title,
        icon: SECTIONS[id].icon,
        detail: SECTIONS[id].detail,
        onPress: section(id),
    }))
    const support: Row[] = [
        { label: t('navigation.logs'), icon: 'file-text', onPress: page('/screens/LogsScreen') },
        ...(__DEV__ || devMode
            ? [
                  {
                      label: t('navigation.dev_components'),
                      icon: 'tool' as const,
                      onPress: page('/screens/ComponentTestScreen'),
                  },
                  {
                      label: t('navigation.dev_colortest'),
                      icon: 'bg-colors' as const,
                      onPress: page('/screens/ColorTestScreen'),
                  },
                  {
                      label: t('navigation.dev_markdown'),
                      icon: 'file-markdown' as const,
                      onPress: page('/screens/MarkdownTestScreen'),
                  },
              ]
            : []),
    ]

    // Seven taps on the version toggle developer tools, as they always have.
    const tapVersion = () => {
        const next = taps + 1
        if (next < 7) {
            setTaps(next)
            return
        }
        setTaps(0)
        setDevMode(!devMode)
        Logger.infoToast(
            devMode ? t('common.labels.devModeDisabled') : t('common.labels.devModeEnabled')
        )
    }

    return (
        <ScrollView contentContainerStyle={styles.page}>
            <HeaderTitle title={t('settings.title')} />

            <TouchableOpacity
                style={[styles.card, styles.account]}
                onPress={page('/screens/UserManagerScreen')}>
                <View style={styles.bubble}>
                    <Text style={styles.bubbleText}>
                        {userName.trim().charAt(0).toUpperCase() || 'Y'}
                    </Text>
                </View>
                <View style={{ flex: 1 }}>
                    <Text style={styles.accountName}>{userName}</Text>
                    <Text style={styles.rowDetail}>Your persona in chats</Text>
                </View>
                <AntDesign name="right" size={16} color={color.text._500} />
            </TouchableOpacity>

            <Text style={styles.groupTitle}>Models</Text>
            <View style={styles.card}>
                <View style={styles.modeRow}>
                    <AppModeToggle />
                </View>
            </View>
            <Card rows={models} />

            <Text style={styles.groupTitle}>Connection</Text>
            <Card rows={connection} />

            <Text style={styles.groupTitle}>App</Text>
            <Card rows={app} />
            <Card rows={support} />

            <TouchableOpacity activeOpacity={0.8} onPress={tapVersion} style={styles.about}>
                <Text style={styles.aboutTitle}>Relay</Text>
                <Text style={styles.aboutText}>
                    {(__DEV__ || devMode) && `${t('common.labels.devMode')} · `}
                    {t('about.versionPrefix') + appConfig.expo.version}
                </Text>
                <Text style={styles.aboutText}>Open source under the AGPL-3.0 license.</Text>
            </TouchableOpacity>
        </ScrollView>
    )
}

export default SettingsScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        page: {
            padding: spacing.xl,
            rowGap: spacing.l,
            paddingBottom: spacing.xl3 * 2,
        },
        card: {
            backgroundColor: color.neutral._200,
            borderRadius: 22,
            overflow: 'hidden',
        },
        account: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            padding: spacing.xl,
        },
        bubble: {
            width: 44,
            height: 44,
            borderRadius: 22,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: '#8b7fd6',
        },
        bubbleText: {
            color: '#ffffff',
            fontSize: fontSize.xl,
            fontWeight: '600',
        },
        accountName: {
            color: color.text._100,
            fontSize: fontSize.xl,
        },
        groupTitle: {
            color: color.text._400,
            fontSize: fontSize.m,
            marginTop: spacing.l,
            marginLeft: spacing.m,
        },
        modeRow: {
            paddingHorizontal: spacing.m,
            paddingTop: spacing.l,
        },
        row: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.xl,
            paddingHorizontal: spacing.xl,
            paddingVertical: spacing.xl,
        },
        rowDivider: {
            borderTopWidth: 1,
            borderTopColor: color.neutral._100,
        },
        rowLabel: {
            color: color.text._100,
            fontSize: fontSize.xl,
        },
        rowDetail: {
            color: color.text._500,
            fontSize: fontSize.m,
            marginTop: 2,
        },
        about: {
            alignItems: 'center',
            rowGap: 2,
            marginTop: spacing.xl2,
        },
        aboutTitle: {
            color: color.text._300,
            fontFamily: 'serif',
            fontSize: fontSize.xl2,
        },
        aboutText: {
            color: color.text._500,
            fontSize: fontSize.s,
        },
    })
}
