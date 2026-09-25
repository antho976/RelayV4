import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    Field,
    HeaderAction,
    QueryView,
    relay,
    Screen,
    Section,
    useBusQuery,
    useRelayEvent,
} from '@components/relay'
import NoteMarkdown from '@components/relay/notes/NoteMarkdown'
import {
    attempt,
    errorText,
    isEditConflict,
    noteTitle,
    RelayNote,
    useDeletedNote,
} from '@components/relay/notes/types'
import { isCancelled } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { ago } from './console'

type Draft = { title: string; body: string; base: RelayNote }

/**
 * One note: rendered markdown, Edit (notes.update with the values it started from as
 * `expected`, so an edit made elsewhere is never overwritten silently), pin, delete with Undo
 * on the notes list, and a quick append. Params: note_id, project_id, standing ('1' for the
 * project's standing note, which cannot be unpinned or deleted).
 */
const NoteScreen = () => {
    const styles = useStyles()
    const router = useRouter()
    const params = useLocalSearchParams<{ note_id?: string; standing?: string }>()
    const noteId = Number(params.note_id)
    const standing = params.standing === '1'
    const valid = Number.isFinite(noteId)
    const note = useBusQuery<RelayNote>('notes.get', { note_id: noteId }, { enabled: valid })
    const [draft, setDraft] = useState<Draft | undefined>(undefined)
    const [conflict, setConflict] = useState(false)
    const [saving, setSaving] = useState(false)
    const [append, setAppend] = useState('')
    const [appending, setAppending] = useState(false)
    const [gone, setGone] = useState(false)

    // Reload on changes to this note only; while editing, the draft keeps its base, so a
    // save after someone else's edit still meets the conflict check.
    useRelayEvent(['notes.*'], (event) => {
        const id = event.payload?.id ?? event.payload?.note_id
        if (id === undefined || id === noteId) note.reload()
    })

    const current = note.data

    const startEdit = () => {
        if (!current) return
        setConflict(false)
        setDraft({ title: current.title ?? '', body: current.body, base: current })
    }

    const save = async () => {
        if (!draft) return
        setSaving(true)
        try {
            const saved = await relay.guarded<RelayNote>('notes.update', {
                note_id: noteId,
                title: draft.title.trim() || null,
                body: draft.body,
                expected: { title: draft.base.title, body: draft.base.body },
            })
            note.setData(saved)
            setDraft(undefined)
            setConflict(false)
            Logger.infoToast('Saved')
        } catch (e) {
            if (isEditConflict(e)) {
                // Someone else saved first. Show their version and keep the draft; saving
                // again replaces what is there now, knowingly.
                const fresh = await relay
                    .call<RelayNote>('notes.get', { note_id: noteId })
                    .catch((err) => {
                        Logger.errorToast(errorText(err))
                        return undefined
                    })
                if (fresh) {
                    note.setData(fresh)
                    setDraft((d) => (d ? { ...d, base: fresh } : d))
                    setConflict(true)
                }
            } else if (!isCancelled(e)) {
                Logger.errorToast(errorText(e))
            }
        } finally {
            setSaving(false)
        }
    }

    const pin = async () => {
        if (!current) return
        const saved = await attempt(
            () =>
                relay.guarded<RelayNote>('notes.pin', {
                    note_id: noteId,
                    pinned: !current.pinned,
                }),
            current.pinned ? 'Unpinned' : 'Pinned'
        )
        if (saved) note.setData(saved)
    }

    const remove = async () => {
        if (!current) return
        const yes = await confirm({
            title: 'Delete this note?',
            message: `“${noteTitle(current)}” — you can undo it from the notes list.`,
            confirmLabel: 'Delete',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(() => relay.guarded('notes.delete', { note_id: noteId }))
        if (!done) return
        useDeletedNote.setState({
            deleted: { id: noteId, project_id: current.project_id, title: noteTitle(current) },
        })
        setGone(true)
        if (router.canGoBack()) router.back()
    }

    const restore = async () => {
        const saved = await attempt(
            () => relay.guarded<RelayNote>('notes.restore', { note_id: noteId }),
            'Note restored'
        )
        if (!saved) return
        useDeletedNote.setState({ deleted: undefined })
        setGone(false)
        note.setData(saved)
    }

    const appendText = async () => {
        const text = append.trim()
        if (!text) return
        setAppending(true)
        const saved = await attempt(
            () => relay.guarded<RelayNote>('notes.append', { note_id: noteId, text: text }),
            'Appended'
        )
        setAppending(false)
        if (!saved) return
        setAppend('')
        note.setData(saved)
    }

    const actions: HeaderAction[] =
        current && !draft && !gone
            ? [
                  { icon: 'edit', label: 'Edit', onPress: startEdit },
                  ...(standing
                      ? []
                      : ([
                            {
                                icon: 'pushpin',
                                label: current.pinned ? 'Unpin' : 'Pin',
                                onPress: pin,
                            },
                            { icon: 'delete', label: 'Delete', onPress: remove },
                        ] as HeaderAction[])),
              ]
            : []

    if (!valid)
        return (
            <Screen title="Note">
                <EmptyState icon="file-text" title="No note" text="This link names no note." />
            </Screen>
        )

    if (gone)
        return (
            <Screen title="Note">
                <EmptyState
                    icon="delete"
                    title="Note deleted"
                    action={{ label: 'Undo', onPress: restore }}
                />
            </Screen>
        )

    return (
        <Screen
            title={current ? noteTitle(current) : 'Note'}
            actions={actions}
            onRefresh={draft ? undefined : note.reload}
            refreshing={note.loading && !!current}>
            <QueryView query={note}>
                {(data) =>
                    draft ? (
                        <>
                            {conflict && (
                                <View style={styles.conflict}>
                                    <Text style={styles.conflictText}>
                                        This note changed on the PC while you were editing; it has
                                        been reloaded and your draft was not saved. Save again to
                                        replace their version with yours, or discard your draft.
                                    </Text>
                                    <Section title="Their version">
                                        <View style={styles.body}>
                                            <Text style={styles.muted}>
                                                {noteTitle(data)} · {ago(data.updated_at)} ago
                                            </Text>
                                            <NoteMarkdown text={data.body || '_Empty_'} />
                                        </View>
                                    </Section>
                                </View>
                            )}
                            <Field
                                label="Title"
                                value={draft.title}
                                onChangeText={(title) => setDraft({ ...draft, title })}
                                placeholder="Untitled"
                            />
                            <Field
                                label="Body"
                                value={draft.body}
                                onChangeText={(body) => setDraft({ ...draft, body })}
                                multiline
                                lines={14}
                                placeholder="Markdown"
                                textAlignVertical="top"
                            />
                            <View style={styles.actions}>
                                <ThemedButton
                                    label={conflict ? 'Discard my draft' : 'Cancel'}
                                    variant="tertiary"
                                    onPress={() => {
                                        setDraft(undefined)
                                        setConflict(false)
                                    }}
                                />
                                <ThemedButton
                                    label={saving ? 'Saving…' : conflict ? 'Save anyway' : 'Save'}
                                    variant={saving ? 'disabled' : 'primary'}
                                    onPress={save}
                                />
                            </View>
                        </>
                    ) : (
                        <>
                            <View style={styles.meta}>
                                {standing && <Chip label="Standing brief" tone="primary" />}
                                {data.pinned && !standing && <Chip label="Pinned" icon="pushpin" />}
                                <Text style={styles.muted}>Edited {ago(data.updated_at)} ago</Text>
                            </View>
                            {standing && (
                                <Text style={styles.muted}>
                                    Agents get this note&apos;s text at dispatch.
                                </Text>
                            )}
                            <Section>
                                <View style={styles.body}>
                                    {data.body.trim() ? (
                                        <NoteMarkdown text={data.body} />
                                    ) : (
                                        <Text style={styles.muted}>Empty note.</Text>
                                    )}
                                </View>
                            </Section>
                            <Section title="Quick append">
                                <View style={styles.body}>
                                    <Field
                                        value={append}
                                        onChangeText={setAppend}
                                        placeholder="Adds a line at the end"
                                        multiline
                                        lines={2}
                                    />
                                    <View style={styles.actions}>
                                        <ThemedButton
                                            label={appending ? 'Appending…' : 'Append'}
                                            variant={
                                                appending || !append.trim() ? 'disabled' : 'primary'
                                            }
                                            onPress={appendText}
                                        />
                                    </View>
                                </View>
                            </Section>
                        </>
                    )
                }
            </QueryView>
        </Screen>
    )
}

export default NoteScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        meta: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            alignItems: 'center',
            columnGap: spacing.m,
            rowGap: spacing.s,
        },
        muted: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        body: {
            rowGap: spacing.m,
            paddingVertical: spacing.l,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
        conflict: {
            rowGap: spacing.m,
            padding: spacing.l,
            borderRadius: 16,
            borderWidth: 1,
            borderColor: color.quote,
        },
        conflictText: {
            color: color.text._200,
            lineHeight: 20,
        },
    })
}
