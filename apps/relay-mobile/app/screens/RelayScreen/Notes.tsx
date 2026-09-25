import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Caption,
    Chip,
    EmptyState,
    Field,
    Mono,
    QueryView,
    relay,
    Row,
    Screen,
    Section,
    Sheet,
    SwitchRow,
    useBusQuery,
    useProjectParam,
} from '@components/relay'
import {
    attempt,
    isStandingNote,
    noteHref,
    noteSnippet,
    noteTitle,
    RelayNote,
    useDeletedNote,
} from '@components/relay/notes/types'
import { Theme } from '@lib/theme/ThemeManager'

import { ago } from './console'

type AuditRow = {
    id: number
    ts: string
    op: string
    payload: any
    result_summary: any
    undo_op: { op: string; payload: any } | null
    undone_by: number | null
    undo_of: number | null
}

const VERBS: Record<string, string> = {
    'notes.create': 'Created',
    'notes.update': 'Edited',
    'notes.pin': 'Pinned or unpinned',
    'notes.delete': 'Deleted',
    'notes.restore': 'Restored',
}

/**
 * The project's notes, as the desktop's Notes page: the standing brief agents get at dispatch
 * (read-only here, with a quick append), pinned notes, the rest, search, a new note, and Undo
 * for the person's own recent note changes (audit.undo).
 */
const NotesScreen = () => {
    const styles = useStyles()
    const router = useRouter()
    const { projectId, project } = useProjectParam()
    const [search, setSearch] = useState('')
    const [creating, setCreating] = useState(false)
    const [append, setAppend] = useState('')
    const [appending, setAppending] = useState(false)
    const deleted = useDeletedNote((state) => state.deleted)
    const enabled = projectId !== undefined
    const scope = { events: ['notes.*'], projectId: projectId, enabled: enabled }

    const notes = useBusQuery<RelayNote[]>(
        'notes.list',
        { project_id: projectId },
        { ...scope, select: (r) => r.notes }
    )
    const standing = useBusQuery<string>(
        'notes.standing',
        { project_id: projectId },
        { ...scope, select: (r) => r.text }
    )
    const recent = useBusQuery<AuditRow[]>(
        'audit.list',
        { project_id: projectId, actor: 'user', op_prefix: 'notes.', limit: 20 },
        { ...scope, select: (r) => r.rows }
    )

    const reload = () => {
        notes.reload()
        standing.reload()
        recent.reload()
    }

    const appendStanding = async () => {
        const text = append.trim()
        if (!text || projectId === undefined) return
        setAppending(true)
        const done = await attempt(
            () => relay.guarded('notes.append', { project_id: projectId, text: text }),
            'Added to the standing brief'
        )
        setAppending(false)
        if (done) {
            setAppend('')
            reload()
        }
    }

    const restore = async () => {
        if (!deleted) return
        const done = await attempt(
            () => relay.guarded('notes.restore', { note_id: deleted.id }),
            'Note restored'
        )
        if (done) {
            useDeletedNote.setState({ deleted: undefined })
            reload()
        }
    }

    const undo = async (row: AuditRow) => {
        const done = await attempt(
            () => relay.guarded('audit.undo', { audit_id: row.id }),
            'Undone'
        )
        if (done) reload()
    }

    const query = search.trim().toLowerCase()
    const matches = (note: RelayNote) =>
        !query ||
        (note.title ?? '').toLowerCase().includes(query) ||
        note.body.toLowerCase().includes(query)

    // Only a note's latest change can be undone: the engine refuses an older one once the
    // note has changed since (audit.stale). Rows come newest first.
    const latest = new Set<number>()
    const undoable = (recent.data ?? [])
        .filter((row) => {
            const id = noteOf(row)
            if (id === undefined || latest.has(id)) return false
            latest.add(id)
            return !!row.undo_op && !row.undone_by && !row.undo_of && !!VERBS[row.op]
        })
        .slice(0, 5)

    return (
        <Screen
            title={project ? `Notes · ${project.name}` : 'Notes'}
            actions={[
                {
                    icon: 'plus',
                    label: 'New note',
                    onPress: () => setCreating(true),
                    disabled: !enabled,
                },
            ]}
            onRefresh={reload}
            refreshing={notes.loading && notes.data !== undefined}>
            {deleted && deleted.project_id === projectId && (
                <View style={styles.banner}>
                    <Text style={styles.bannerText} numberOfLines={2}>
                        Deleted “{deleted.title}”
                    </Text>
                    <ThemedButton label="Undo" variant="secondary" onPress={restore} />
                    <ThemedButton
                        iconName="close"
                        variant="tertiary"
                        onPress={() => useDeletedNote.setState({ deleted: undefined })}
                    />
                </View>
            )}

            <Section title="Standing brief">
                <View style={styles.brief}>
                    <Caption>What every agent gets at dispatch</Caption>
                    {standing.data === undefined ? (
                        <Text style={styles.muted}>{standing.error?.message ?? 'Loading…'}</Text>
                    ) : standing.data.trim() ? (
                        <Mono lines={12}>{standing.data}</Mono>
                    ) : (
                        <Text style={styles.muted}>
                            Empty. Agents get no standing notes until something is added.
                        </Text>
                    )}
                    <Field
                        value={append}
                        onChangeText={setAppend}
                        placeholder="Quick append a line to the brief"
                        multiline
                        lines={2}
                    />
                    <View style={styles.actions}>
                        <ThemedButton
                            label={appending ? 'Adding…' : 'Append'}
                            variant={appending || !append.trim() ? 'disabled' : 'primary'}
                            onPress={appendStanding}
                        />
                    </View>
                </View>
            </Section>

            <Field value={search} onChangeText={setSearch} placeholder="Search notes" />

            <QueryView
                query={notes}
                isEmpty={(list) => list.length === 0}
                empty={
                    <EmptyState
                        icon="file-text"
                        title="No notes yet"
                        text="Notes are shared with the PC and, when pinned, with the agents."
                        action={{ label: 'New note', onPress: () => setCreating(true) }}
                    />
                }>
                {(list) => {
                    const shown = list.filter(matches)
                    const pinned = shown.filter((note) => note.pinned)
                    const rest = shown.filter((note) => !note.pinned)
                    const row = (note: RelayNote) => {
                        const isStanding = isStandingNote(list, note, standing.data)
                        return (
                            <Row
                                key={note.id}
                                icon={
                                    isStanding ? 'profile' : note.pinned ? 'pushpin' : 'file-text'
                                }
                                label={noteTitle(note)}
                                detail={
                                    [ago(note.updated_at), noteSnippet(note)]
                                        .filter(Boolean)
                                        .join(' · ') || undefined
                                }
                                right={isStanding ? <Chip label="Standing" tone="primary" /> : null}
                                onPress={() => router.push(noteHref(note, isStanding))}
                            />
                        )
                    }
                    if (shown.length === 0)
                        return <EmptyState icon="search" title="No note matches" text={search} />
                    return (
                        <>
                            {pinned.length > 0 && (
                                <Section title={`Pinned · ${pinned.length}`}>
                                    {pinned.map(row)}
                                </Section>
                            )}
                            {rest.length > 0 && (
                                <Section title={`Notes · ${rest.length}`}>{rest.map(row)}</Section>
                            )}
                        </>
                    )
                }}
            </QueryView>

            {undoable.length > 0 && (
                <Section title="Your recent note changes">
                    {undoable.map((row) => (
                        <Row
                            key={row.id}
                            icon="rollback"
                            label={`${VERBS[row.op]} ${auditSubject(row)}`}
                            detail={ago(row.ts)}
                            right={
                                <ThemedButton
                                    label="Undo"
                                    variant="tertiary"
                                    onPress={() => undo(row)}
                                />
                            }
                        />
                    ))}
                </Section>
            )}

            {projectId !== undefined && (
                <NewNoteSheet
                    projectId={projectId}
                    visible={creating}
                    onDismiss={() => setCreating(false)}
                    onCreated={(note) => {
                        setCreating(false)
                        notes.reload()
                        router.push(noteHref(note))
                    }}
                />
            )}
        </Screen>
    )
}

export default NotesScreen

const noteOf = (row: AuditRow): number | undefined =>
    row.payload?.note_id ?? row.result_summary?.id ?? undefined

const auditSubject = (row: AuditRow) => {
    const summary = row.result_summary
    if (summary && typeof summary === 'object' && (summary.title || summary.body))
        return `“${noteTitle({ title: summary.title ?? null, body: summary.body ?? '' })}”`
    const id = row.payload?.note_id ?? summary?.id
    return id ? `note #${id}` : 'a note'
}

const NewNoteSheet: React.FC<{
    projectId: number
    visible: boolean
    onDismiss: () => void
    onCreated: (note: RelayNote) => void
}> = ({ projectId, visible, onDismiss, onCreated }) => {
    const styles = useStyles()
    const [title, setTitle] = useState('')
    const [body, setBody] = useState('')
    const [pinned, setPinned] = useState(false)
    const [busy, setBusy] = useState(false)

    const create = async () => {
        setBusy(true)
        const note = await attempt(() =>
            relay.guarded<RelayNote>('notes.create', {
                project_id: projectId,
                title: title.trim() || null,
                body: body,
                pinned: pinned,
            })
        )
        setBusy(false)
        if (!note) return
        setTitle('')
        setBody('')
        setPinned(false)
        onCreated(note)
    }

    const empty = !title.trim() && !body.trim()

    return (
        <Sheet visible={visible} onDismiss={onDismiss}>
            <View style={styles.sheet}>
                <Text style={styles.sheetTitle}>New note</Text>
                <Field label="Title" value={title} onChangeText={setTitle} placeholder="Optional" />
                <Field
                    label="Body"
                    value={body}
                    onChangeText={setBody}
                    multiline
                    lines={8}
                    placeholder="Markdown"
                />
                <SwitchRow
                    label="Pinned"
                    description="Pinned notes are listed first."
                    value={pinned}
                    onChange={setPinned}
                />
                <View style={styles.actions}>
                    <ThemedButton label="Cancel" variant="tertiary" onPress={onDismiss} />
                    <ThemedButton
                        label={busy ? 'Saving…' : 'Create'}
                        variant={busy || empty ? 'disabled' : 'primary'}
                        onPress={create}
                    />
                </View>
            </View>
        </Sheet>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        banner: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            padding: spacing.l,
            borderRadius: 16,
            backgroundColor: color.neutral._300,
        },
        bannerText: {
            flex: 1,
            color: color.text._200,
        },
        brief: {
            rowGap: spacing.m,
            paddingVertical: spacing.l,
        },
        muted: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
        sheet: {
            rowGap: spacing.l,
        },
        sheetTitle: {
            color: color.text._100,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
    })
}
