import { useLiveQuery } from 'drizzle-orm/expo-sqlite'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { View } from 'react-native'
import { FlatList } from 'react-native-gesture-handler'
import { SafeAreaView } from 'react-native-safe-area-context'
import { create } from 'zustand'

import LongButton from '@components/buttons/LongButton'
import ThemedButton from '@components/buttons/ThemedButton'
import ThemedSlider from '@components/input/ThemedSlider'
import ThemedSwitch from '@components/input/ThemedSwitch'
import ThemedTextInput from '@components/input/ThemedTextInput'
import SectionTitle from '@components/text/SectionTitle'
import TText from '@components/text/TText'
import Accordion from '@components/views/Accordion'
import HeaderTitle from '@components/views/HeaderTitle'
import { LorebookType } from '@db/schema'
import { useLiveQueryJoined } from '@lib/hooks/LiveQueryJoined'
import { Lorebooks } from '@lib/state/lorebooks'

import LorebookEntryEditor, { useLorebookEntryEditorState } from './LorebookEntryEditor'

type LorebookInfoState = {
    id?: number
    setId: (id: number) => void
}

export const useLorebookInfoState = create<LorebookInfoState>()((set) => ({
    setId: (id) => set({ id }),
}))

const LorebookInfoScreen = () => {
    const { t } = useTranslation()
    const [search, setSearch] = useState('')
    const { id } = useLorebookInfoState()
    const { data: entries } = useLiveQuery(
        Lorebooks.db.live.lorebookEntryNameList(id ?? -1, search),
        [search]
    )
    const [placeholderInfo, setPlaceholderInfo] = useState<LorebookType | undefined>(undefined)
    const openEditor = useLorebookEntryEditorState((state) => state.open)
    // edits not yet written, merged so quick edits to two fields both land
    const pending = useRef<Partial<LorebookType>>({})
    const timer = useRef<ReturnType<typeof setTimeout>>(undefined)

    useLiveQueryJoined(Lorebooks.db.live.lorebookInfo(id ?? -1), [id], {
        onUpdated: (result) => {
            if (result) setPlaceholderInfo({ ...result, ...pending.current })
        },
    })

    const flush = useCallback(() => {
        clearTimeout(timer.current)
        const patch = pending.current
        pending.current = {}
        if (id && Object.keys(patch).length > 0) Lorebooks.db.mutate.updateLorebookInfo(id, patch)
    }, [id])

    useEffect(() => flush, [flush])

    const handleUpdate = (lorebookInfo: Partial<LorebookType>) => {
        if (placeholderInfo) {
            setPlaceholderInfo({ ...placeholderInfo, ...lorebookInfo })
        }
        pending.current = { ...pending.current, ...lorebookInfo }
        clearTimeout(timer.current)
        timer.current = setTimeout(flush, 300)
    }

    return (
        <SafeAreaView style={{ flex: 1, rowGap: 16 }}>
            <HeaderTitle title={t('lorebook.labels.info')} />

            <View
                style={{
                    paddingHorizontal: 12,
                    rowGap: 8,
                }}>
                <Accordion
                    label={t('lorebook.labels.generationSettings')}
                    bodyStyle={{ rowGap: 8, paddingBottom: 24 }}>
                    <ThemedTextInput
                        containerStyle={{ flex: 0 }}
                        label={t('common.labels.name')}
                        value={placeholderInfo?.name ?? ''}
                        onChangeText={(name) => handleUpdate({ name })}
                    />
                    <ThemedTextInput
                        multiline
                        numberOfLines={4}
                        containerStyle={{ flex: 0 }}
                        label={t('common.labels.description')}
                        value={placeholderInfo?.description ?? ''}
                        onChangeText={(description) => handleUpdate({ description })}
                    />
                    <ThemedSlider
                        label={t('lorebook.fields.tokenBudget')}
                        value={placeholderInfo?.token_budget ?? 0}
                        min={1}
                        max={128000}
                        onValueChange={(token_budget) => handleUpdate({ token_budget })}
                    />
                    <ThemedSwitch
                        label={t('lorebook.fields.recursiveScanning')}
                        value={placeholderInfo?.recursive_scanning ?? false}
                        onChangeValue={(recursive_scanning) => handleUpdate({ recursive_scanning })}
                    />
                    <ThemedSlider
                        label={t('lorebook.fields.scanDepth')}
                        value={placeholderInfo?.scan_depth ?? 0}
                        min={0}
                        max={100}
                        onValueChange={(scan_depth) => handleUpdate({ scan_depth })}
                    />
                </Accordion>
            </View>
            <SectionTitle style={{ marginHorizontal: 12, paddingTop: 16, paddingBottom: 8 }}>
                {t('lorebook.fields.entries')}
            </SectionTitle>
            <View
                style={{
                    flexDirection: 'row',
                    alignItems: 'center',
                    paddingHorizontal: 12,
                    columnGap: 8,
                }}>
                <ThemedTextInput
                    containerStyle={{}}
                    placeholder={t('common.actions.search')}
                    value={search}
                    onChangeText={setSearch}
                />

                <ThemedButton
                    variant="secondary"
                    buttonStyle={{ flex: 0, paddingHorizontal: 8 }}
                    iconName="plus"
                    onPress={async () => {
                        if (!id) return
                        const entryId = await Lorebooks.db.mutate.createLorebookEntry(
                            t('lorebook.new.entry'),
                            id
                        )
                        openEditor(entryId)
                    }}
                />
            </View>

            <FlatList
                style={{ paddingHorizontal: 12 }}
                data={entries}
                contentContainerStyle={{ rowGap: 4, paddingBottom: 64 }}
                keyExtractor={(item) => item.id.toString()}
                renderItem={({ item }) => (
                    <LongButton active={false} onPress={() => openEditor(item.id)}>
                        <TText>{item.name}</TText>
                    </LongButton>
                )}
            />
            <LorebookEntryEditor />
        </SafeAreaView>
    )
}

export default LorebookInfoScreen
