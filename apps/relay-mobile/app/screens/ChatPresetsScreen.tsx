import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useLiveQuery } from 'drizzle-orm/expo-sqlite'
import { Redirect } from 'expo-router'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { KeyboardAwareScrollView } from 'react-native-keyboard-controller'
import { SafeAreaView } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import DropdownSheet from '@components/input/DropdownSheet'
import ThemedTextInput from '@components/input/ThemedTextInput'
import Alert from '@components/views/Alert'
import HeaderTitle from '@components/views/HeaderTitle'
import { ChatPresetType } from '@db/schema'
import { useLiveQueryJoined } from '@lib/hooks/LiveQueryJoined'
import { Chats } from '@lib/state/Chat'
import { ChatPresets } from '@lib/state/ChatPresets'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type GiveTarget = Awaited<ReturnType<typeof ChatPresets.db.query.giveTargets>>[0]

const ChatPresetsScreen = () => {
    const styles = useStyles()
    const { t } = useTranslation()
    const { color, spacing } = Theme.useTheme()
    const { chatId } = Chats.useChat()
    const { data: chatData } = useLiveQueryJoined(Chats.db.live.chat(chatId ?? -1), [chatId], {
        deepCheck: true,
    })
    const activePresetId = chatData?.active_preset_id
    const setActivePreset = Chats.db.mutate.updateActivePreset
    const { data: presets } = useLiveQuery(ChatPresets.db.query.presetsForChatQuery(chatId ?? -1), [
        chatId,
    ])

    const [editing, setEditing] = useState<
        { id?: number; data: ChatPresets.ChatPresetEdit } | undefined
    >()
    const [giving, setGiving] = useState<
        { preset: ChatPresetType; targets: GiveTarget[] } | undefined
    >()

    if (!chatId) return <Redirect href=".." />

    const isOwner = (preset: ChatPresetType) => preset.owner_chat_id === chatId

    const handleActivate = async (preset: ChatPresetType) => {
        const next = activePresetId === preset.id ? null : preset.id
        await setActivePreset(chatId, next)
    }

    const handleSave = async () => {
        if (!editing) return
        const data = { ...editing.data, name: editing.data.name.trim() }
        if (!data.name) {
            Logger.errorToast(t('chatPresets.toast.nameEmpty'))
            return
        }
        if (editing.id) {
            await ChatPresets.db.mutate.updatePreset(editing.id, data)
            Logger.infoToast(t('chatPresets.toast.saved'))
        } else {
            const id = await ChatPresets.db.mutate.createPreset(chatId, data)
            await setActivePreset(chatId, id)
            Logger.infoToast(t('chatPresets.toast.created'))
        }
        setEditing(undefined)
    }

    const handleDelete = (preset: ChatPresetType) => {
        Alert.alert({
            title: t('chatPresets.delete.title'),
            description: t('chatPresets.delete.description', { name: preset.name }),
            buttons: [
                { label: t('common.actions.cancel') },
                {
                    label: t('common.actions.delete'),
                    type: 'warning',
                    onPress: async () => {
                        await ChatPresets.db.mutate.deletePreset(preset.id)
                        if (activePresetId === preset.id) await setActivePreset(chatId, null)
                    },
                },
            ],
        })
    }

    const handleRemoveFromChat = (preset: ChatPresetType) => {
        Alert.alert({
            title: t('chatPresets.remove.title'),
            description: t('chatPresets.remove.description', { name: preset.name }),
            buttons: [
                { label: t('common.actions.cancel') },
                {
                    label: t('chatPresets.remove.confirm'),
                    type: 'warning',
                    onPress: async () => {
                        await ChatPresets.db.mutate.removeFromChat(preset.id, chatId)
                        if (activePresetId === preset.id) await setActivePreset(chatId, null)
                    },
                },
            ],
        })
    }

    const handleOpenGive = async (preset: ChatPresetType) => {
        const targets = await ChatPresets.db.query.giveTargets(preset.id, chatId)
        if (targets.length === 0) {
            Logger.infoToast(t('chatPresets.noGiveTargets'))
            return
        }
        setGiving({ preset, targets })
    }

    const handleGive = async (target: GiveTarget) => {
        if (!giving) return
        await ChatPresets.db.mutate.giveToChat(giving.preset.id, target.id)
        Logger.infoToast(t('chatPresets.given', { name: target.name }))
        setGiving(undefined)
    }

    const renderEditor = () => {
        if (!editing) return null
        const { data } = editing
        const update = (patch: Partial<ChatPresets.ChatPresetEdit>) =>
            setEditing({ ...editing, data: { ...data, ...patch } })
        return (
            <View style={styles.card}>
                <Text style={styles.cardTitle}>
                    {editing.id ? t('chatPresets.edit') : t('chatPresets.new')}
                </Text>
                <ThemedTextInput
                    label={t('chatPresets.name')}
                    value={data.name}
                    onChangeText={(name) => update({ name })}
                />
                <ThemedTextInput
                    label={t('chatPresets.systemPrompt')}
                    description={t('chatPresets.systemPromptDescription')}
                    multiline
                    numberOfLines={6}
                    value={data.system_prompt}
                    onChangeText={(system_prompt) => update({ system_prompt })}
                />
                <ThemedTextInput
                    label={t('chatPresets.persona')}
                    description={t('chatPresets.personaDescription')}
                    multiline
                    numberOfLines={6}
                    value={data.persona}
                    onChangeText={(persona) => update({ persona })}
                />
                <ThemedTextInput
                    label={t('chatPresets.rules')}
                    description={t('chatPresets.rulesDescription')}
                    multiline
                    numberOfLines={6}
                    value={data.rules}
                    onChangeText={(rules) => update({ rules })}
                />
                <View style={styles.row}>
                    <ThemedButton
                        label={t('common.actions.cancel')}
                        variant="secondary"
                        onPress={() => setEditing(undefined)}
                    />
                    <ThemedButton
                        label={t('common.actions.save')}
                        iconName="save"
                        onPress={handleSave}
                    />
                </View>
            </View>
        )
    }

    const renderPreset = (preset: ChatPresetType) => {
        const active = preset.id === activePresetId
        const owner = isOwner(preset)
        return (
            <View key={preset.id} style={[styles.card, active && styles.cardActive]}>
                <TouchableOpacity
                    style={styles.presetHeader}
                    onPress={() => handleActivate(preset)}>
                    <AntDesign
                        name={active ? 'check-circle' : 'minus-circle'}
                        size={22}
                        color={active ? color.primary._500 : color.text._600}
                    />
                    <View style={{ flex: 1 }}>
                        <Text style={styles.presetName}>{preset.name}</Text>
                        <Text style={styles.presetMeta}>
                            {owner ? t('chatPresets.createdHere') : t('chatPresets.givenByOther')}
                            {active ? `  ·  ${t('chatPresets.active')}` : ''}
                        </Text>
                    </View>
                </TouchableOpacity>
                <Text style={styles.presetPreview} numberOfLines={2}>
                    {[
                        preset.system_prompt && t('chatPresets.systemPrompt'),
                        preset.persona && t('chatPresets.persona'),
                        preset.rules && t('chatPresets.rules'),
                    ]
                        .filter((item) => item)
                        .join(', ') || t('chatPresets.emptyPreset')}
                </Text>
                <View style={styles.row}>
                    {owner ? (
                        <>
                            <ThemedButton
                                label={t('common.actions.edit')}
                                iconName="edit"
                                variant="secondary"
                                onPress={() =>
                                    setEditing({
                                        id: preset.id,
                                        data: {
                                            name: preset.name,
                                            system_prompt: preset.system_prompt,
                                            persona: preset.persona,
                                            rules: preset.rules,
                                        },
                                    })
                                }
                            />
                            <ThemedButton
                                label={t('chatPresets.give')}
                                iconName="export"
                                variant="secondary"
                                onPress={() => handleOpenGive(preset)}
                            />
                            <ThemedButton
                                label={t('common.actions.delete')}
                                iconName="delete"
                                variant="critical"
                                onPress={() => handleDelete(preset)}
                            />
                        </>
                    ) : (
                        <ThemedButton
                            label={t('chatPresets.removeFromChat')}
                            iconName="close"
                            variant="critical"
                            onPress={() => handleRemoveFromChat(preset)}
                        />
                    )}
                </View>
            </View>
        )
    }

    return (
        <SafeAreaView edges={['bottom']} style={{ flex: 1 }}>
            <HeaderTitle title={t('chatPresets.title')} />
            <KeyboardAwareScrollView
                bottomOffset={16}
                showsVerticalScrollIndicator={false}
                keyboardShouldPersistTaps="always"
                contentContainerStyle={styles.container}>
                <Text style={styles.hint}>{t('chatPresets.hint')}</Text>

                {giving && (
                    <View style={styles.card}>
                        <Text style={styles.cardTitle}>
                            {t('chatPresets.giveTitle', { name: giving.preset.name })}
                        </Text>
                        <DropdownSheet
                            data={giving.targets}
                            labelExtractor={(item) => item.name}
                            onChangeValue={handleGive}
                            placeholder={t('chatPresets.givePlaceholder')}
                            modalTitle={t('chatPresets.giveModalTitle')}
                            search
                        />
                        <ThemedButton
                            label={t('common.actions.cancel')}
                            variant="secondary"
                            onPress={() => setGiving(undefined)}
                        />
                    </View>
                )}

                {renderEditor()}

                {!editing && (
                    <ThemedButton
                        label={t('chatPresets.new')}
                        iconName="plus"
                        onPress={() => setEditing({ data: ChatPresets.blankPreset() })}
                    />
                )}

                {presets.length === 0 && !editing && (
                    <Text style={styles.empty}>{t('chatPresets.empty')}</Text>
                )}
                {presets.map(renderPreset)}
                <View style={{ height: spacing.xl3 }} />
            </KeyboardAwareScrollView>
        </SafeAreaView>
    )
}

export default ChatPresetsScreen

const useStyles = () => {
    const { color, spacing, borderRadius, fontSize, borderWidth } = Theme.useTheme()
    return StyleSheet.create({
        container: {
            paddingHorizontal: spacing.xl,
            paddingTop: spacing.m,
            rowGap: spacing.l,
        },
        hint: {
            color: color.text._500,
            fontSize: fontSize.s,
        },
        empty: {
            color: color.text._500,
            fontStyle: 'italic',
            textAlign: 'center',
            paddingVertical: spacing.xl,
        },
        card: {
            backgroundColor: color.neutral._100,
            borderRadius: borderRadius.m,
            borderWidth: borderWidth.m,
            borderColor: color.neutral._200,
            padding: spacing.l,
            rowGap: spacing.m,
        },
        cardActive: {
            borderColor: color.primary._500,
        },
        cardTitle: {
            color: color.text._100,
            fontSize: fontSize.l,
        },
        presetHeader: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
        },
        presetName: {
            color: color.text._100,
            fontSize: fontSize.l,
        },
        presetMeta: {
            color: color.text._500,
            fontSize: fontSize.s,
        },
        presetPreview: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        row: {
            flexDirection: 'row',
            columnGap: spacing.m,
            justifyContent: 'flex-end',
            flexWrap: 'wrap',
            rowGap: spacing.m,
        },
    })
}
