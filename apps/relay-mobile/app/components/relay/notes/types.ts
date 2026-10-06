import { create } from 'zustand'

import { isCancelled, RelayRequestError } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'

/** `types::Note`, as notes.list / notes.get return it. */
export type RelayNote = {
    id: number
    project_id: number
    title: string | null
    body: string
    pinned: boolean
    created_at: string
    updated_at: string
    deleted_at: string | null
}

/** The title the engine gives the standing note when `notes.append {project_id}` creates it. */
export const STANDING_TITLE = 'Standing notes'

/**
 * Whether a note of `notes.list` is the project's standing note. The row does not say so;
 * the list sorts it first, it is always pinned, and its body is exactly what notes.standing
 * returns (the engine refuses to unpin or delete it either way).
 */
export const isStandingNote = (notes: RelayNote[], note: RelayNote, standingText?: string) => {
    if (notes[0]?.id !== note.id || !note.pinned) return false
    if (standingText) return note.body === standingText
    return note.title === STANDING_TITLE
}

/** The title, or the first line of the body for an untitled note. */
export const noteTitle = (note: Pick<RelayNote, 'title' | 'body'>) => {
    const title = note.title?.trim()
    if (title) return title
    const line = note.body.split('\n').find((text) => text.trim())
    return line
        ? line
              .replace(/^#+\s*/, '')
              .trim()
              .slice(0, 80)
        : 'Untitled note'
}

/** The body on one line, for a list row. */
export const noteSnippet = (note: Pick<RelayNote, 'title' | 'body'>) =>
    note.body.replace(/\s+/g, ' ').trim().slice(0, 200)

export const noteHref = (note: Pick<RelayNote, 'id' | 'project_id'>, standing = false) => ({
    pathname: '/screens/RelayScreen/Note' as const,
    params: {
        note_id: String(note.id),
        project_id: String(note.project_id),
        ...(standing ? { standing: '1' } : {}),
    },
})

export const errorText = (e: unknown) =>
    e instanceof RelayRequestError ? e.error.message : e instanceof Error ? e.message : String(e)

export const isEditConflict = (e: unknown) =>
    e instanceof RelayRequestError && e.error.code === 'notes.edit_conflict'

/**
 * Run a mutation and toast its failure; a denied hold stays quiet. Resolves to the result, or
 * undefined when it did not happen.
 */
export const attempt = async <T>(run: () => Promise<T>, done?: string): Promise<T | undefined> => {
    try {
        const result = await run()
        if (done) Logger.infoToast(done)
        return result
    } catch (e) {
        if (!isCancelled(e)) Logger.errorToast(errorText(e))
        return undefined
    }
}

/**
 * The note deleted last, so the notes list can offer Undo (notes.restore) after the note's
 * own screen has closed.
 */
export const useDeletedNote = create<{
    deleted?: { id: number; project_id: number; title: string }
}>()(() => ({}))
