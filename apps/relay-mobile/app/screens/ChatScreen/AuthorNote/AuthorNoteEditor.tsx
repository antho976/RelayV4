import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ScrollView, Text, View } from 'react-native'
import { useShallow } from 'zustand/react/shallow'

import ThemedButton from '@components/buttons/ThemedButton'
import HorizontalSelector from '@components/input/HorizontalSelector'
import ThemedSlider from '@components/input/ThemedSlider'
import ThemedTextInput from '@components/input/ThemedTextInput'
import Alert from '@components/views/Alert'
import BottomSheet from '@components/views/BottomSheet'
import { Tokenizer } from '@lib/engine/Tokenizer'
import { useLiveQueryJoined } from '@lib/hooks/LiveQueryJoined'
import { useDebounceTokenizer } from '@lib/hooks/Tokenizer'
import { AuthorNote, AuthorNotes, NoteType } from '@lib/state/AuthorNotes'
import { Characters } from '@lib/state/Characters'
import { Chats } from '@lib/state/Chat'
import { authorNoteBodyState, authorNoteEditorState } from '@lib/state/components/AuthorNotes'
import { Theme } from '@lib/theme/ThemeManager'

const getNoteTypeFromIds = (note: AuthorNote) => {
    if (note.chat_id) return NoteType.CHAT
    if (note.character_id) return NoteType.CHARACTER
    return NoteType.GLOBAL
}

const getUpdateValue = (
    noteType: NoteType,
    charId: number | undefined,
    chatId: number | undefined
) => {
    let updates: Partial<AuthorNote> | null = null
    switch (noteType) {
        case NoteType.CHARACTER:
            if (charId) {
                updates = {
                    character_id: charId,
                    chat_id: null,
                }
            }
            break

        case NoteType.CHAT:
            if (chatId) {
                updates = {
                    character_id: null,
                    chat_id: chatId,
                }
            }
            break

        case NoteType.GLOBAL:
            updates = {
                character_id: null,
                chat_id: null,
            }
            break
    }
    return updates
}

const AuthorNoteEditor = () => {
    const { t } = useTranslation()
    const tokenizer = Tokenizer.useTokenizer()
    const { color, spacing, fontSize } = Theme.useTheme()
    const setCurrentNoteType = authorNoteBodyState(useShallow((state) => state.setCurrentNoteType))
    const charId = Characters.useCharacterStore(useShallow((state) => state.id))
    const chatId = Chats.useChatState(useShallow((state) => state.id))
    const { close, noteId, ref } = authorNoteEditorState(useShallow((state) => state))
    // the unsaved copy of the note; live updates only replace it while it is unedited
    const [draft, setDraft] = useState<{ note: AuthorNote; edited: boolean } | undefined>()
    const placeHolderNote = draft?.note
    const edited = draft?.edited ?? false

    const handleSetPlaceholder = (newNote: AuthorNote) => {
        setDraft({ note: newNote, edited: true })
    }
    const {
        data: [note],
    } = useLiveQueryJoined(AuthorNotes.db.live.note(noteId ?? -1), [noteId ?? -1], {
        targets: [
            {
                tableName: 'author_notes',
                rowId: noteId ?? -1,
            },
        ],
        onUpdated: (result) => {
            const [item] = result
            if (!item) return
            setDraft((draft) =>
                draft?.edited && draft.note.id === item.id ? draft : { note: item, edited: false }
            )
        },
    })
    const contentTokens = useDebounceTokenizer(placeHolderNote?.content ?? '', 300)

    const handleSave = async () => {
        if (!note || !placeHolderNote) return
        await AuthorNotes.db.mutate.updateNote(note.id, {
            name: placeHolderNote.name,
            content: placeHolderNote.content,
            note: placeHolderNote.note,
            priority: placeHolderNote.priority,
            depth: placeHolderNote.depth,
            token_length: await tokenizer(placeHolderNote.content),
        })
        close()
    }

    const backAction = (close: () => void) => {
        if (!note || !placeHolderNote || !edited) return close()
        Alert.alert({
            title: t('authorNotes.unsavedChanges.title'),
            description: t('authorNotes.unsavedChanges.description'),
            buttons: [
                { label: t('common.actions.cancel') },
                {
                    label: t('authorNotes.unsavedChanges.discard'),
                    onPress: close,
                    type: 'warning',
                },
                { label: t('common.actions.save'), onPress: handleSave },
            ],
        })
        return true
    }

    const handleUpdateNoteType = async (
        noteType: NoteType,
        charId: number | undefined,
        chatId: number | undefined
    ) => {
        if (!note) return
        const updates = getUpdateValue(noteType, charId, chatId)
        if (!updates) return
        const updated = await AuthorNotes.db.mutate.updateNote(note.id, updates).then(() => true)
        if (updated) setCurrentNoteType(noteType)
    }

    // the sheet stays mounted so the first open() reaches it before the note has loaded
    return (
        <BottomSheet
            onRequestClose={backAction}
            onClose={() => setDraft(note ? { note: note, edited: false } : undefined)}
            sheetStyle={{ flex: 1 }}
            ref={ref}>
            {noteId !== undefined && note?.id === noteId && placeHolderNote?.id === noteId && (
                <>
                    <ScrollView
                        showsVerticalScrollIndicator={false}
                        contentContainerStyle={{ rowGap: spacing.xl, paddingBottom: spacing.xl2 }}>
                        <ThemedTextInput
                            label={t('common.labels.name')}
                            containerStyle={{ flex: 0 }}
                            value={placeHolderNote.name}
                            onChangeText={(t) => {
                                handleSetPlaceholder({ ...placeHolderNote, name: t })
                            }}
                        />
                        <ThemedTextInput
                            label={t('authorNotes.item.content', { tokenLength: contentTokens })}
                            containerStyle={{ flex: 0 }}
                            numberOfLines={10}
                            value={placeHolderNote.content}
                            onChangeText={(t) => {
                                handleSetPlaceholder({ ...placeHolderNote, content: t })
                            }}
                        />
                        <Text style={{ color: color.text._700, fontSize: fontSize.s }}>
                            {t('common.labels.tokens')}: {contentTokens}
                        </Text>

                        <ThemedTextInput
                            label={t('authorNotes.item.comments')}
                            containerStyle={{ flex: 0 }}
                            numberOfLines={2}
                            value={placeHolderNote.note}
                            onChangeText={(t) => {
                                handleSetPlaceholder({ ...placeHolderNote, note: t })
                            }}
                        />
                        <ThemedSlider
                            label={t('authorNotes.item.priority')}
                            min={0}
                            max={1000}
                            step={1}
                            value={placeHolderNote.priority ?? 0}
                            onValueChange={(value) =>
                                handleSetPlaceholder({ ...placeHolderNote, priority: value })
                            }
                        />
                        <ThemedSlider
                            label={t('authorNotes.item.depth')}
                            min={0}
                            max={10}
                            step={1}
                            value={placeHolderNote.depth ?? 0}
                            onValueChange={(value) =>
                                handleSetPlaceholder({ ...placeHolderNote, depth: value })
                            }
                        />
                        <HorizontalSelector
                            label={t('authorNotes.type')}
                            style={{ flex: 1 }}
                            values={[
                                { label: t('authorNotes.selector.chat'), value: NoteType.CHAT },
                                {
                                    label: t('authorNotes.selector.character'),
                                    value: NoteType.CHARACTER,
                                },
                                { label: t('authorNotes.selector.global'), value: NoteType.GLOBAL },
                            ]}
                            selected={getNoteTypeFromIds(note)}
                            onPress={(type) => {
                                if (type === getNoteTypeFromIds(note)) return
                                handleUpdateNoteType(type, charId, chatId)
                            }}
                        />
                    </ScrollView>

                    <View
                        style={{
                            flexDirection: 'row',
                            columnGap: spacing.l,
                            justifyContent: 'space-between',
                            marginTop: 8,
                        }}>
                        <ThemedButton
                            label={t('common.actions.delete')}
                            variant="critical"
                            iconName="delete"
                            onPress={() => {
                                Alert.alert({
                                    title: t('authorNotes.alert.delete.title'),
                                    description: t('authorNotes.alert.delete.description', {
                                        name: note.name,
                                    }),
                                    buttons: [
                                        { label: t('common.actions.cancel') },
                                        {
                                            label: t('authorNotes.alert.delete.button'),
                                            onPress: () => {
                                                AuthorNotes.db.mutate.deleteNote(note.id)
                                                close()
                                            },
                                        },
                                    ],
                                })
                            }}
                        />
                        <ThemedButton
                            label={t('common.actions.reset')}
                            variant="tertiary"
                            iconName="reload"
                            onPress={() => setDraft({ note: note, edited: false })}
                        />
                        <ThemedButton
                            label={t('common.actions.save')}
                            variant="secondary"
                            iconName="save"
                            onPress={handleSave}
                        />
                    </View>
                </>
            )}
        </BottomSheet>
    )
}

export default AuthorNoteEditor
