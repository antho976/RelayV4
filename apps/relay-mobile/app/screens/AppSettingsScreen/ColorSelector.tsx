import AntDesign from '@react-native-vector-icons/ant-design/static'
import { setStringAsync } from 'expo-clipboard'
import { setBackgroundColorAsync } from 'expo-system-ui'
import React, { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
    ScrollView,
    Share,
    StyleSheet,
    Text,
    TouchableOpacity,
    useColorScheme,
    View,
} from 'react-native'
import { useShallow } from 'zustand/react/shallow'

import ThemedButton from '@components/buttons/ThemedButton'
import SegmentedControl, { Segment } from '@components/theme/SegmentedControl'
import SettingsGroup from '@components/theme/SettingsGroup'
import ThemeCard from '@components/theme/ThemeCard'
import ThemePreview from '@components/theme/ThemePreview'
import { isDarkTheme } from '@components/theme/util'
import Alert from '@components/views/Alert'
import { useBottomSheetRef } from '@components/views/BottomSheet'
import ContextMenu, { ContextMenuButtonProps } from '@components/views/ContextMenu'
import HeaderTitle from '@components/views/HeaderTitle'
import InputSheet from '@components/views/InputSheet'
import { Logger } from '@lib/state/Logger'
import { DefaultColorSchemes, ThemeColor } from '@lib/theme/ThemeColor'
import { Theme } from '@lib/theme/ThemeManager'
import { pickJSONDocument, saveStringToDownload } from '@lib/utils/File'

type Mode = 'light' | 'dark' | 'system'
type Slot = 'light' | 'dark'

/** Paints the system bars behind the app to match the theme that is now on screen. */
const paint = (theme: ThemeColor) => {
    setBackgroundColorAsync(theme.neutral._100)
}

const toJSON = (theme: ThemeColor) => JSON.stringify(theme, null, 4)

/** Splits the grid into rows of two, so both columns line up whatever the names' lengths. */
const pairs = <T,>(items: T[]) =>
    items.reduce<T[][]>((rows, item, index) => {
        if (index % 2 === 0) rows.push([item])
        else rows[rows.length - 1].push(item)
        return rows
    }, [])

/**
 * Everything about the app's colours: a live preview, the Light / Dark / System mode and the
 * grid of themes, with importing and sharing themes as JSON (docs/CustomThemes.md).
 *
 * Light and Dark pin one theme (`color`) and turn off following the phone; System follows it
 * with `lightColor` and `darkColor`. Picking a theme while pinned also files it as the light or
 * dark theme by its own brightness, so flipping between Light, Dark and System keeps the last
 * theme chosen for each.
 */
export const ThemeSection = () => {
    const { t } = useTranslation()
    const styles = useStyles()
    const { color: pageColor } = Theme.useTheme()
    const systemTheme = useColorScheme()
    const phoneSlot: Slot = systemTheme === 'dark' ? 'dark' : 'light'
    const {
        systemDark,
        setSystemDark,
        color,
        lightColor,
        darkColor,
        setColor,
        setLightColor,
        setDarkColor,
        customColors,
        addCustomColor,
        removeColorScheme,
    } = Theme.useColorState(
        useShallow((state) => ({
            systemDark: state.useSystemDarkMode,
            setSystemDark: state.setUseSystemDarkMode,
            color: state.color,
            lightColor: state.lightColor,
            darkColor: state.darkColor,
            setColor: state.setColor,
            setLightColor: state.setLightColor,
            setDarkColor: state.setDarkColor,
            customColors: state.customColors,
            addCustomColor: state.addCustomColor,
            removeColorScheme: state.removeColorScheme,
        }))
    )
    const [editing, setEditing] = useState<Slot>(phoneSlot)
    const inputRef = useBottomSheetRef()

    const pinnedDark =
        color.name === darkColor.name && color.name !== lightColor.name
            ? true
            : color.name === lightColor.name && color.name !== darkColor.name
              ? false
              : isDarkTheme(color)
    const mode: Mode = systemDark ? 'system' : pinnedDark ? 'dark' : 'light'
    const slotTheme = (slot: Slot) => (slot === 'dark' ? darkColor : lightColor)
    const selected = systemDark ? slotTheme(editing) : color

    const setMode = (next: Mode) => {
        if (next === 'system') {
            setSystemDark(true)
            setEditing(phoneSlot)
            paint(slotTheme(phoneSlot))
            return
        }
        const theme = slotTheme(next)
        setSystemDark(false)
        setColor(theme)
        paint(theme)
    }

    const pick = (theme: ThemeColor) => {
        if (systemDark) {
            if (editing === 'dark') setDarkColor(theme)
            else setLightColor(theme)
            if (editing === phoneSlot) paint(theme)
            return
        }
        setColor(theme)
        if (isDarkTheme(theme)) setDarkColor(theme)
        else setLightColor(theme)
        paint(theme)
    }

    const confirmDelete = (theme: ThemeColor, index: number) =>
        Alert.alert({
            title: t('settings.colors.alert.deleteTheme.title'),
            description: t('settings.colors.alert.deleteTheme.description', { name: theme.name }),
            buttons: [
                { label: t('common.actions.cancel') },
                {
                    label: t('settings.colors.alert.deleteTheme.confirm'),
                    type: 'warning',
                    onPress: () => removeColorScheme(index),
                },
            ],
        })

    const deleteButton = (theme: ThemeColor, index: number): ContextMenuButtonProps => ({
        label: t('settings.colors.contextMenu.delete'),
        icon: 'delete',
        variant: 'warning',
        onPress: (close) => {
            close()
            confirmDelete(theme, index)
        },
    })

    const importFile = () =>
        pickJSONDocument().then((result) => {
            if (!result.success) return
            addCustomColor(result.data)
        })

    const segments: Segment<Mode>[] = [
        { value: 'light', label: t('settings.colors.modeLight'), icon: 'sun' },
        { value: 'dark', label: t('settings.colors.modeDark'), icon: 'moon' },
        { value: 'system', label: t('settings.colors.modeSystem'), icon: 'mobile' },
    ]

    const builtIn = DefaultColorSchemes.schemes.map((theme) => ({ theme: theme, custom: -1 }))
    const custom = customColors.map((theme, index) => ({ theme: theme, custom: index }))
    const inUse = (theme: ThemeColor) =>
        systemDark ? theme.name === slotTheme(phoneSlot).name && editing !== phoneSlot : false

    const slotRow = (slot: Slot) => {
        const theme = slotTheme(slot)
        const active = editing === slot
        return (
            <TouchableOpacity
                key={slot}
                accessibilityState={{ selected: active }}
                style={[styles.slot, active && styles.slotActive]}
                onPress={() => setEditing(slot)}>
                <AntDesign
                    name={slot === 'dark' ? 'moon' : 'sun'}
                    size={20}
                    color={active ? pageColor.text._100 : pageColor.text._400}
                />
                <View style={{ flex: 1 }}>
                    <Text style={styles.slotLabel}>
                        {slot === 'dark'
                            ? t('settings.colors.darkTheme')
                            : t('settings.colors.lightTheme')}
                    </Text>
                    <Text numberOfLines={1} style={styles.slotDetail}>
                        {theme.name}
                    </Text>
                </View>
                <View style={styles.swatches}>
                    {[theme.neutral._100, theme.neutral._300, theme.primary._500].map(
                        (swatch, index) => (
                            <View
                                key={index}
                                style={[
                                    styles.swatch,
                                    {
                                        backgroundColor: swatch,
                                        borderColor: pageColor.neutral._400,
                                    },
                                ]}
                            />
                        )
                    )}
                </View>
            </TouchableOpacity>
        )
    }

    const previewCaption = systemDark
        ? editing === 'dark'
            ? t('settings.colors.darkTheme')
            : t('settings.colors.lightTheme')
        : customColors.some((item) => item.name === selected.name)
          ? t('settings.colors.custom')
          : t('settings.colors.builtIn')

    return (
        <View style={styles.section}>
            <InputSheet
                ref={inputRef}
                onConfirm={(text) => {
                    try {
                        addCustomColor(JSON.parse(text))
                    } catch (e) {
                        Logger.errorToast(
                            t('settings.colors.error.failedToImport', { error: `${e}` })
                        )
                    }
                }}
                multiline
                title={t('settings.colors.pasteThemeTitle')}
            />

            <View style={styles.previewBlock}>
                <ThemePreview theme={selected} />
                <View style={styles.previewCaption}>
                    <Text numberOfLines={1} style={styles.previewName}>
                        {selected.name}
                    </Text>
                    <Text style={styles.previewNote}>{previewCaption}</Text>
                </View>
            </View>

            <SettingsGroup
                title={t('settings.colors.mode')}
                detail={
                    systemDark
                        ? phoneSlot === 'dark'
                            ? t('settings.colors.systemFollowingDark')
                            : t('settings.colors.systemFollowingLight')
                        : t('settings.colors.fixedDescription')
                }>
                <SegmentedControl segments={segments} selected={mode} onChange={setMode} />
                {systemDark && (
                    <View style={styles.slots}>
                        {slotRow('light')}
                        {slotRow('dark')}
                    </View>
                )}
            </SettingsGroup>

            <SettingsGroup
                title={
                    systemDark
                        ? editing === 'dark'
                            ? t('settings.colors.darkTheme')
                            : t('settings.colors.lightTheme')
                        : t('settings.colors.title')
                }>
                {pairs([...builtIn, ...custom]).map((row) => (
                    <View key={row[0].theme.name} style={styles.gridRow}>
                        {row.map(({ theme, custom: index }) => (
                            <View key={theme.name} style={styles.gridCell}>
                                <ContextMenu
                                    longPress
                                    placement="center"
                                    onPress={() => pick(theme)}
                                    buttons={[
                                        {
                                            label: t('settings.colors.contextMenu.share'),
                                            icon: 'share-alt',
                                            onPress: (close) => {
                                                close()
                                                Share.share({
                                                    title: theme.name,
                                                    message: toJSON(theme),
                                                }).catch(() => {})
                                            },
                                        },
                                        {
                                            label: t('settings.colors.contextMenu.copy'),
                                            icon: 'copy',
                                            onPress: (close) => {
                                                close()
                                                setStringAsync(toJSON(theme)).then(() =>
                                                    Logger.infoToast(
                                                        t('settings.colors.messages.copied')
                                                    )
                                                )
                                            },
                                        },
                                        {
                                            label: t('settings.colors.contextMenu.save'),
                                            icon: 'download',
                                            onPress: (close) => {
                                                close()
                                                saveStringToDownload(
                                                    toJSON(theme),
                                                    `${theme.name}.json`,
                                                    'utf8'
                                                ).then(() =>
                                                    Logger.infoToast(
                                                        t('settings.colors.messages.saved', {
                                                            name: theme.name,
                                                        })
                                                    )
                                                )
                                            },
                                        },
                                        ...(index >= 0 ? [deleteButton(theme, index)] : []),
                                    ]}>
                                    <ThemeCard
                                        theme={theme}
                                        selected={theme.name === selected.name}
                                        note={
                                            inUse(theme)
                                                ? t('settings.colors.inUse')
                                                : index >= 0
                                                  ? t('settings.colors.custom')
                                                  : undefined
                                        }
                                    />
                                </ContextMenu>
                            </View>
                        ))}
                        {row.length === 1 && <View style={styles.gridCell} />}
                    </View>
                ))}

                <View style={styles.actions}>
                    <ThemedButton
                        label={t('settings.colors.contextMenu.importTheme')}
                        iconName="download"
                        iconSize={16}
                        variant="secondary"
                        buttonStyle={styles.action}
                        onPress={importFile}
                    />
                    <ThemedButton
                        label={t('settings.colors.contextMenu.pasteTheme')}
                        iconName="file-text"
                        iconSize={16}
                        variant="secondary"
                        buttonStyle={styles.action}
                        onPress={() => inputRef.current?.open()}
                    />
                </View>
                <Text style={styles.hint}>{t('settings.colors.longPressHint')}</Text>
            </SettingsGroup>
        </View>
    )
}

/** The theme settings on a page of their own; Settings → App → Appearance shows them inline. */
const ColorSelector = () => {
    const { t } = useTranslation()
    const { spacing } = Theme.useTheme()
    return (
        <ScrollView
            contentContainerStyle={{
                padding: spacing.xl,
                paddingBottom: spacing.xl3 * 2,
            }}>
            <HeaderTitle title={t('settings.colors.title')} />
            <ThemeSection />
        </ScrollView>
    )
}

export default ColorSelector

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        section: {
            rowGap: spacing.xl2,
        },
        previewBlock: {
            rowGap: spacing.l,
        },
        previewCaption: {
            flexDirection: 'row',
            alignItems: 'baseline',
            justifyContent: 'space-between',
            columnGap: spacing.l,
            paddingHorizontal: spacing.m,
        },
        previewName: {
            flex: 1,
            color: color.text._100,
            // eslint-disable-next-line i18next/no-literal-string
            fontFamily: 'serif',
            fontSize: fontSize.xl3,
        },
        previewNote: {
            color: color.text._500,
            fontSize: fontSize.m,
        },
        slots: {
            rowGap: spacing.s,
        },
        slot: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.l,
            borderRadius: 18,
        },
        slotActive: {
            backgroundColor: color.neutral._300,
        },
        slotLabel: {
            color: color.text._100,
            fontSize: fontSize.l,
        },
        slotDetail: {
            color: color.text._500,
            fontSize: fontSize.m,
            marginTop: 1,
        },
        swatches: {
            flexDirection: 'row',
        },
        swatch: {
            width: 18,
            height: 18,
            borderRadius: 9,
            borderWidth: 1,
            marginLeft: -5,
        },
        gridRow: {
            flexDirection: 'row',
            columnGap: spacing.l,
        },
        gridCell: {
            flex: 1,
        },
        actions: {
            flexDirection: 'row',
            columnGap: spacing.m,
            marginTop: spacing.s,
        },
        action: {
            flex: 1,
            paddingHorizontal: spacing.l,
        },
        hint: {
            color: color.text._500,
            fontSize: fontSize.s,
            textAlign: 'center',
            paddingHorizontal: spacing.m,
        },
    })
}
