import React, { ReactNode, useMemo } from 'react'
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'
import { palette } from '@screens/RelayScreen/console'

import { diffLines } from './diff'
import { Hunk } from './types'

/** Past this many lines a diff is cut, with a note; a phone is not the place for a 50k-line diff. */
const MAX_LINES = 2000

/**
 * A diff's hunks on the console's black: numbered lines, additions green, removals red, and a
 * horizontal scroll instead of wrapping so columns stay where the code put them.
 */
export const DiffBlock: React.FC<{
    hunks?: Hunk[]
    loading?: boolean
    binary?: boolean
    error?: string
    /** Shown when there are no hunks. Default "No textual changes." */
    empty?: string
}> = ({ hunks, loading, binary, error, empty = 'No textual changes.' }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const lines = useMemo(() => diffLines(hunks ?? []), [hunks])
    const shown = lines.slice(0, MAX_LINES)
    const width = String(shown.reduce((most, line) => Math.max(most, line.number ?? 0), 0)).length
    const note = error
        ? error
        : binary
          ? 'Binary file; no text diff.'
          : loading || !hunks
            ? 'Loading…'
            : lines.length === 0
              ? empty
              : undefined
    return (
        <View style={styles.diff}>
            {!!note && (
                <Text style={[styles.meta, !!error && { color: color.error._200 }]}>{note}</Text>
            )}
            {!note && (
                <ScrollView horizontal showsHorizontalScrollIndicator={false}>
                    <View>
                        {shown.map((line, i) => (
                            <Text
                                key={i}
                                selectable
                                style={[
                                    styles.line,
                                    line.kind === 'head' && styles.hunkHead,
                                    line.kind === 'add' && styles.added,
                                    line.kind === 'del' && styles.removed,
                                    line.kind === 'note' && styles.meta,
                                ]}>
                                <Text style={styles.gutter}>
                                    {(line.number === undefined
                                        ? ''
                                        : String(line.number)
                                    ).padStart(width, ' ')}{' '}
                                </Text>
                                {line.text || ' '}
                            </Text>
                        ))}
                        {lines.length > shown.length && (
                            <Text style={styles.meta}>
                                {lines.length - shown.length} more lines not shown.
                            </Text>
                        )}
                    </View>
                </ScrollView>
            )}
        </View>
    )
}

/**
 * One changed file: its status letter, path (with the old name of a rename), and counts. Tap
 * toggles `children` (the diff) under it; `right` holds per-file actions such as Stage.
 */
export const DiffFileRow: React.FC<{
    path: string
    status: string
    oldPath?: string | null
    added?: number
    removed?: number
    binary?: boolean
    /** Replaces the counts, e.g. "new". */
    meta?: string
    open?: boolean
    onPress?: () => void
    right?: ReactNode
    children?: ReactNode
}> = ({ path, status, oldPath, added, removed, binary, meta, open, onPress, right, children }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <View>
            <TouchableOpacity
                style={[styles.file, open && styles.fileOpen]}
                disabled={!onPress}
                onPress={onPress}>
                <Text style={styles.status}>{status || ' '}</Text>
                <Text numberOfLines={2} style={styles.path}>
                    {oldPath ? `${oldPath} → ` : ''}
                    {path}
                </Text>
                {meta !== undefined || binary ? (
                    <Text style={styles.meta}>{meta ?? 'binary'}</Text>
                ) : added !== undefined || removed !== undefined ? (
                    <Text style={styles.counts}>
                        <Text style={{ color: palette.live }}>+{added ?? 0}</Text>{' '}
                        <Text style={{ color: color.error._300 }}>-{removed ?? 0}</Text>
                    </Text>
                ) : null}
                {right}
            </TouchableOpacity>
            {open && children}
        </View>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        file: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            backgroundColor: color.neutral._300,
            paddingHorizontal: spacing.m,
            paddingVertical: spacing.sm,
            minHeight: 40,
        },
        fileOpen: {
            backgroundColor: color.neutral._400,
        },
        status: {
            width: 14,
            color: color.text._400,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        path: {
            flex: 1,
            color: color.text._100,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        counts: {
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        meta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        diff: {
            backgroundColor: palette.ink,
            paddingHorizontal: spacing.m,
            paddingVertical: spacing.s,
            marginBottom: spacing.s,
        },
        hunkHead: {
            color: color.text._500,
            marginTop: spacing.s,
        },
        gutter: {
            color: color.text._600,
        },
        line: {
            color: palette.paper,
            fontFamily: 'monospace',
            fontSize: 11,
            lineHeight: 15,
        },
        added: {
            color: palette.live,
        },
        removed: {
            color: color.error._200,
        },
    })
}
