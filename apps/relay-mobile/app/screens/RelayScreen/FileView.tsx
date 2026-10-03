import * as Crypto from 'expo-crypto'
import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useState } from 'react'
import { Image, ScrollView, StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    confirm,
    EmptyState,
    Field,
    isCancelled,
    QueryView,
    relay,
    RelayRequestError,
    Screen,
    useBusQuery,
} from '@components/relay'
import {
    ActionSheet,
    afterSheet,
    baseName,
    dirName,
    Entry,
    formatBytes,
    PromptSheet,
    problemText,
    useGuardedAction,
} from '@components/relay/git'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { palette } from './console'

type ReadOut = {
    text: string | null
    bytes_b64: string | null
    mime: string
    size: number
    truncated: boolean
}

/** What the phone asks for; the engine's own ceiling is 32 MiB, a phone's patience far less. */
const MAX_BYTES = 1024 * 1024
/** Lines drawn in the viewer; past this the rest is noted, not rendered. */
const MAX_LINES = 5000

/**
 * One file of a worktree: text with line numbers (monospace, scrolls sideways instead of
 * wrapping), an image drawn from its bytes, or a note for binary and oversized files. Text can
 * be edited in place — the write carries the hash of what was read, so a file that changed on
 * the PC meanwhile is refused rather than overwritten. Also rename, revert to HEAD and trash
 * with Undo. Params: `project_id`, `worktree`, `path`. Desktop: relay-native editor.rs,
 * image_preview.rs.
 */
const FileViewScreen = () => {
    const router = useRouter()
    const styles = useStyles()
    const params = useLocalSearchParams<{ project_id?: string; worktree?: string; path?: string }>()
    const parsed = params.project_id ? Number(params.project_id) : NaN
    const projectId = Number.isFinite(parsed) ? parsed : undefined
    const worktree =
        typeof params.worktree === 'string' && params.worktree ? params.worktree : undefined
    const path = typeof params.path === 'string' ? params.path : ''
    const scope: Record<string, unknown> = { project_id: projectId ?? 0, path: path }
    if (worktree) scope.worktree = worktree

    const [editing, setEditing] = useState<{ base: string; draft: string } | undefined>(undefined)
    const [saving, setSaving] = useState(false)
    const [menu, setMenu] = useState(false)
    const [renaming, setRenaming] = useState(false)
    const [trashed, setTrashed] = useState<number | undefined>(undefined)
    const [aspect, setAspect] = useState(1)
    const { busy, run } = useGuardedAction()
    const file = useBusQuery<ReadOut>(
        'file.read',
        { ...scope, max_bytes: MAX_BYTES },
        {
            enabled: projectId !== undefined && !!path && trashed === undefined,
            events: ['file.changed'],
            projectId: projectId,
            debounceMs: 500,
        }
    )

    if (projectId === undefined || !path) {
        return (
            <Screen title="File">
                <EmptyState icon="file" title="No file" text="Open one from Files." />
            </Screen>
        )
    }

    const data = file.data
    const editable = !!data && data.text !== null && !data.truncated

    const save = async () => {
        if (!editing) return
        setSaving(true)
        try {
            const expected = await Crypto.digestStringAsync(
                Crypto.CryptoDigestAlgorithm.SHA256,
                editing.base
            )
            const result = await relay.guarded<{ added_lines: number; removed_lines: number }>(
                'file.write',
                { ...scope, text: editing.draft, expected_sha256: expected }
            )
            Logger.infoToast(`Saved · +${result.added_lines} -${result.removed_lines}`)
            setEditing(undefined)
            file.reload()
        } catch (e) {
            if (isCancelled(e)) return
            if (e instanceof RelayRequestError && e.error.code === 'file.edit_conflict') {
                const reload = await confirm({
                    title: 'Changed on the PC',
                    message:
                        'The file changed or was removed since you opened it, so your edit was not written. Reload it? Your draft is discarded.',
                    confirmLabel: 'Reload',
                    cancelLabel: 'Keep editing',
                    destructive: true,
                })
                if (reload) {
                    setEditing(undefined)
                    file.reload()
                }
                return
            }
            Logger.errorToast(problemText(e))
        } finally {
            setSaving(false)
        }
    }

    const cancelEdit = async () => {
        if (editing && editing.draft !== editing.base) {
            const yes = await confirm({
                title: 'Discard your changes?',
                confirmLabel: 'Discard',
                destructive: true,
            })
            if (!yes) return
        }
        setEditing(undefined)
    }

    const remove = async () => {
        const yes = await confirm({
            title: `Move ${baseName(path)} to the trash?`,
            message: 'It goes to the project trash (.relay/trash); Undo brings it back.',
            confirmLabel: 'Move to trash',
            destructive: true,
        })
        if (!yes) return
        const result = await run<{ trash_id: number }>('file.delete', scope)
        if (result) setTrashed(result.trash_id)
    }

    const restore = async () => {
        if (trashed === undefined) return
        const back = await run<Entry>(
            'file.restore',
            { project_id: projectId, trash_id: trashed },
            `Restored ${path}`
        )
        if (back) {
            setTrashed(undefined)
            if (back.path !== path) router.setParams({ path: back.path })
        }
    }

    const restoreHead = async () => {
        const yes = await confirm({
            title: `Revert ${baseName(path)} to HEAD?`,
            message: 'Throws away every uncommitted change to this file. This cannot be undone.',
            confirmLabel: 'Revert',
            destructive: true,
        })
        if (!yes) return
        const done = await run('file.restore_head', scope, 'Reverted to HEAD')
        if (done) file.reload()
    }

    const title = baseName(path)
    if (trashed !== undefined) {
        return (
            <Screen title={title}>
                <EmptyState
                    icon="delete"
                    title="Moved to the trash"
                    text={path}
                    action={{ label: busy ? 'Restoring…' : 'Undo', onPress: restore }}
                />
                <ThemedButton label="Back" variant="tertiary" onPress={() => router.back()} />
            </Screen>
        )
    }

    const lines = data?.text?.split('\n') ?? []
    if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop()
    const shown = lines.slice(0, MAX_LINES)
    const width = String(shown.length).length

    return (
        <Screen
            title={editing ? `Editing ${title}` : title}
            onRefresh={editing ? undefined : file.reload}
            refreshing={file.loading && !!data}
            actions={
                editing
                    ? []
                    : [
                          {
                              icon: 'edit',
                              label: 'Edit',
                              disabled: !editable,
                              onPress: () =>
                                  data?.text !== null &&
                                  data?.text !== undefined &&
                                  setEditing({ base: data.text, draft: data.text }),
                          },
                          { icon: 'ellipsis', label: 'More', onPress: () => setMenu(true) },
                      ]
            }
            footer={
                editing ? (
                    <View style={styles.footer}>
                        <ThemedButton label="Cancel" variant="secondary" onPress={cancelEdit} />
                        <ThemedButton
                            label={saving ? 'Saving…' : 'Save'}
                            variant={
                                !saving && editing.draft !== editing.base ? 'primary' : 'disabled'
                            }
                            onPress={() => {
                                if (!saving && editing.draft !== editing.base) save()
                            }}
                        />
                    </View>
                ) : undefined
            }>
            <Text numberOfLines={2} selectable style={styles.path}>
                {dirName(path) ? `${dirName(path)}/` : ''}
                <Text style={styles.pathName}>{title}</Text>
                {data ? `  ·  ${formatBytes(data.size)} · ${data.mime}` : ''}
            </Text>
            {editing ? (
                <Field
                    value={editing.draft}
                    onChangeText={(text) =>
                        setEditing((now) => (now ? { ...now, draft: text } : now))
                    }
                    multiline
                    lines={24}
                    mono
                    autoCapitalize="none"
                    autoCorrect={false}
                    spellCheck={false}
                    textAlignVertical="top"
                />
            ) : (
                <QueryView query={file}>
                    {(read) =>
                        read.text !== null ? (
                            <>
                                {read.truncated && (
                                    <Text style={styles.notice}>
                                        Showing the first {formatBytes(MAX_BYTES)} of{' '}
                                        {formatBytes(read.size)}. Editing needs the whole file.
                                    </Text>
                                )}
                                <View style={styles.code}>
                                    <Text style={styles.gutter}>
                                        {shown
                                            .map((_, i) => String(i + 1).padStart(width, ' '))
                                            .join('\n')}
                                    </Text>
                                    <ScrollView horizontal style={styles.codeScroll}>
                                        <Text selectable style={styles.text}>
                                            {shown.join('\n') || ' '}
                                        </Text>
                                    </ScrollView>
                                </View>
                                {lines.length > shown.length && (
                                    <Text style={styles.notice}>
                                        {lines.length - shown.length} more lines not shown.
                                    </Text>
                                )}
                            </>
                        ) : read.bytes_b64 && read.mime.startsWith('image/') && !read.truncated ? (
                            <Image
                                source={{ uri: `data:${read.mime};base64,${read.bytes_b64}` }}
                                style={[styles.image, { aspectRatio: aspect }]}
                                resizeMode="contain"
                                onLoad={(event) => {
                                    const { width: w, height: h } = event.nativeEvent.source
                                    if (w > 0 && h > 0) setAspect(w / h)
                                }}
                            />
                        ) : (
                            <EmptyState
                                icon="file-unknown"
                                title={read.truncated ? 'Too large to show' : 'Binary file'}
                                text={`${read.mime} · ${formatBytes(read.size)}${
                                    read.truncated
                                        ? ` · the phone reads at most ${formatBytes(MAX_BYTES)}`
                                        : ''
                                }`}
                            />
                        )
                    }
                </QueryView>
            )}
            <ActionSheet
                visible={menu}
                title={path}
                onDismiss={() => setMenu(false)}
                actions={[
                    { label: 'Rename', icon: 'edit', onPress: () => setRenaming(true) },
                    {
                        label: 'Revert to HEAD',
                        icon: 'rollback',
                        detail: 'Discard uncommitted changes to this file',
                        destructive: true,
                        onPress: restoreHead,
                    },
                    { label: 'Move to trash', icon: 'delete', destructive: true, onPress: remove },
                ]}
            />
            <PromptSheet
                visible={renaming}
                title={`Rename ${title}`}
                label="Name"
                initial={title}
                confirmLabel="Rename"
                busy={!!busy}
                onDismiss={() => setRenaming(false)}
                onSubmit={async (name) => {
                    const entry = await run<Entry>(
                        'file.rename',
                        { ...scope, new_name: name },
                        (out) => `Renamed to ${out.name}`
                    )
                    if (entry) afterSheet(() => router.setParams({ path: entry.path }))
                    return !!entry
                }}
            />
        </Screen>
    )
}

export default FileViewScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        path: {
            color: color.text._400,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        pathName: {
            color: color.text._100,
        },
        notice: {
            color: color.quote,
            fontSize: fontSize.s,
        },
        code: {
            flexDirection: 'row',
            backgroundColor: palette.ink,
            borderRadius: 8,
            paddingVertical: spacing.m,
            overflow: 'hidden',
        },
        gutter: {
            color: color.text._600,
            fontFamily: 'monospace',
            fontSize: 12,
            lineHeight: 17,
            paddingHorizontal: spacing.s,
            textAlign: 'right',
            borderRightWidth: StyleSheet.hairlineWidth,
            borderRightColor: color.neutral._500,
        },
        codeScroll: {
            flex: 1,
        },
        text: {
            color: palette.paper,
            fontFamily: 'monospace',
            fontSize: 12,
            lineHeight: 17,
            paddingHorizontal: spacing.m,
        },
        image: {
            width: '100%',
            borderRadius: 8,
            backgroundColor: color.neutral._200,
        },
        footer: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
            paddingHorizontal: spacing.xl,
            paddingVertical: spacing.m,
        },
    })
}
