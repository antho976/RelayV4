import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    ErrorState,
    Field,
    LoadingState,
    relay,
    SwitchRow,
    useBusQuery,
} from '@components/relay'
import { Theme } from '@lib/theme/ThemeManager'

import { attempt, splitLines } from './common'

export type GuardrailConfig = {
    caps: { files: number; lines: number }
    destructive_write: {
        min_removed_lines: number
        min_removed_pct: number
        min_file_lines: number
        allow_if_recoverable: boolean
    }
    protected_paths: string[]
    denied_commands: string[]
    [key: string]: unknown
}

type Draft = {
    files: string
    lines: string
    min_removed_lines: string
    min_removed_pct: string
    min_file_lines: string
    allow_if_recoverable: boolean
    protected_paths: string
    denied_commands: string
}

const toDraft = (config: GuardrailConfig): Draft => ({
    files: String(config.caps.files),
    lines: String(config.caps.lines),
    min_removed_lines: String(config.destructive_write.min_removed_lines),
    min_removed_pct: String(config.destructive_write.min_removed_pct),
    min_file_lines: String(config.destructive_write.min_file_lines),
    allow_if_recoverable: config.destructive_write.allow_if_recoverable,
    protected_paths: config.protected_paths.join('\n'),
    denied_commands: config.denied_commands.join('\n'),
})

const sameList = (a: string[], b: string[]) =>
    a.length === b.length && a.every((item, index) => item === b[index])

/**
 * The changed fields as a `guardrail.config.set` patch, or an error to show. Only what the
 * person changed goes in, so a project override stays as small as what they meant.
 */
const toPatch = (config: GuardrailConfig, draft: Draft): { patch?: any; problem?: string } => {
    const whole = (text: string, label: string) => {
        const value = Number(text.trim())
        if (!text.trim() || !Number.isInteger(value) || value < 0)
            throw new Error(`${label} must be a whole number of 0 or more.`)
        return value
    }
    try {
        const patch: any = {}
        const caps: any = {}
        const files = whole(draft.files, 'Maximum changed files')
        const lines = whole(draft.lines, 'Maximum changed lines')
        if (files !== config.caps.files) caps.files = files
        if (lines !== config.caps.lines) caps.lines = lines
        if (Object.keys(caps).length > 0) patch.caps = caps
        const destructive: any = {}
        const dw = config.destructive_write
        const removed = whole(draft.min_removed_lines, 'Minimum removed lines')
        const shortest = whole(draft.min_file_lines, 'Minimum file length')
        const pct = Number(draft.min_removed_pct.trim())
        if (!draft.min_removed_pct.trim() || !Number.isFinite(pct) || pct < 0 || pct > 100)
            throw new Error('Minimum removed percent must be between 0 and 100.')
        if (removed !== dw.min_removed_lines) destructive.min_removed_lines = removed
        if (pct !== dw.min_removed_pct) destructive.min_removed_pct = pct
        if (shortest !== dw.min_file_lines) destructive.min_file_lines = shortest
        if (draft.allow_if_recoverable !== dw.allow_if_recoverable)
            destructive.allow_if_recoverable = draft.allow_if_recoverable
        if (Object.keys(destructive).length > 0) patch.destructive_write = destructive
        const paths = splitLines(draft.protected_paths)
        const commands = splitLines(draft.denied_commands)
        if (!sameList(paths, config.protected_paths)) patch.protected_paths = paths
        if (!sameList(commands, config.denied_commands)) patch.denied_commands = commands
        return { patch }
    } catch (e) {
        return { problem: (e as Error).message }
    }
}

/**
 * The guardrail thresholds and lists, PC-wide or as one project's override (the desktop's
 * Settings → Guardrails). A project shows the effective values: global merged with its own.
 */
const GuardrailEditor: React.FC<{ projectId?: number }> = ({ projectId }) => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const scoped = projectId !== undefined
    const overridePath = scoped ? `guardrails.projects.${projectId}` : ''
    const query = useBusQuery<GuardrailConfig>(
        'guardrail.config.get',
        scoped ? { project_id: projectId } : {},
        { events: ['guardrail.config_changed', 'settings.changed'] }
    )
    const override = useBusQuery<Record<string, unknown>>(
        'settings.get',
        { path: overridePath },
        {
            enabled: scoped,
            events: ['guardrail.config_changed', 'settings.changed'],
            select: (raw) => (raw?.value && typeof raw.value === 'object' ? raw.value : {}),
        }
    )
    // Unsaved edits; without any, the form shows the values as the PC has them now.
    const [edit, setEdit] = useState<Draft | undefined>(undefined)
    const [saving, setSaving] = useState(false)
    const config = query.data

    if (!config) {
        if (query.error) return <ErrorState error={query.error} onRetry={query.reload} />
        return <LoadingState />
    }

    const draft = edit ?? toDraft(config)
    const { patch, problem } = toPatch(config, draft)
    const dirty = !!patch && Object.keys(patch).length > 0
    const set = (key: keyof Draft) => (value: any) => setEdit({ ...draft, [key]: value })
    const overridden = Object.keys(override.data ?? {})
    const isOverridden = (key: string) => scoped && overridden.includes(key)

    const save = async () => {
        if (!dirty) return
        setSaving(true)
        const next = await attempt(
            () =>
                relay.guarded<GuardrailConfig>('guardrail.config.set', {
                    ...(scoped ? { project_id: projectId } : {}),
                    patch,
                }),
            'Guardrails saved'
        )
        setSaving(false)
        if (next) {
            setEdit(undefined)
            query.setData(next)
            override.reload()
        }
    }

    const clearOverride = async () => {
        const yes = await confirm({
            title: 'Use the PC-wide guardrails?',
            message: 'This project forgets its own guardrail values and follows the PC settings.',
            confirmLabel: 'Clear override',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(
            () => relay.guarded('settings.reset', { path: overridePath }),
            'Override cleared'
        )
        if (done !== undefined) {
            query.reload()
            override.reload()
        }
    }

    const mark = (key: string) =>
        isOverridden(key) ? <Chip label="Set for this project" tone="primary" /> : null

    return (
        <View style={{ rowGap: spacing.l, paddingVertical: spacing.l }}>
            {scoped && (
                <Text style={styles.note}>
                    {overridden.length > 0
                        ? 'This project overrides some PC-wide values; the rest follow the PC settings.'
                        : 'This project follows the PC-wide guardrails. A value saved here applies to this project only.'}
                </Text>
            )}
            <View style={styles.heading}>
                <Text style={styles.label}>Change caps per task</Text>
                {mark('caps')}
            </View>
            <View style={styles.pair}>
                <View style={styles.half}>
                    <Field
                        label="Files"
                        value={draft.files}
                        onChangeText={set('files')}
                        keyboardType="number-pad"
                    />
                </View>
                <View style={styles.half}>
                    <Field
                        label="Lines"
                        value={draft.lines}
                        onChangeText={set('lines')}
                        keyboardType="number-pad"
                    />
                </View>
            </View>
            <View style={styles.heading}>
                <Text style={styles.label}>Destructive write</Text>
                {mark('destructive_write')}
            </View>
            <Text style={styles.note}>
                A write that removes at least this many lines, or this share of a file, is held for
                you to confirm.
            </Text>
            <View style={styles.pair}>
                <View style={styles.half}>
                    <Field
                        label="Removed lines"
                        value={draft.min_removed_lines}
                        onChangeText={set('min_removed_lines')}
                        keyboardType="number-pad"
                    />
                </View>
                <View style={styles.half}>
                    <Field
                        label="Removed percent"
                        value={draft.min_removed_pct}
                        onChangeText={set('min_removed_pct')}
                        keyboardType="decimal-pad"
                    />
                </View>
            </View>
            <Field
                label="Percent rule skips files shorter than (lines)"
                value={draft.min_file_lines}
                onChangeText={set('min_file_lines')}
                keyboardType="number-pad"
            />
            <SwitchRow
                label="Allow if recoverable"
                description="Let a large rewrite through when git can put the file back."
                value={draft.allow_if_recoverable}
                onChange={set('allow_if_recoverable')}
            />
            <View style={styles.heading}>
                <Text style={styles.label}>Protected paths</Text>
                {mark('protected_paths')}
            </View>
            <Field
                value={draft.protected_paths}
                onChangeText={set('protected_paths')}
                multiline
                lines={4}
                mono
                autoCapitalize="none"
                autoCorrect={false}
                placeholder="One glob per line, e.g. .github/**"
            />
            <View style={styles.heading}>
                <Text style={styles.label}>Denied commands</Text>
                {mark('denied_commands')}
            </View>
            <Field
                value={draft.denied_commands}
                onChangeText={set('denied_commands')}
                multiline
                lines={4}
                mono
                autoCapitalize="none"
                autoCorrect={false}
                placeholder="One command per line, e.g. git push --force"
            />
            {!!problem && <Text style={styles.problem}>{problem}</Text>}
            <View style={styles.actions}>
                {scoped && overridden.length > 0 && (
                    <ThemedButton
                        label="Clear override"
                        variant="secondary"
                        onPress={clearOverride}
                    />
                )}
                {dirty && (
                    <ThemedButton
                        label="Revert"
                        variant="secondary"
                        onPress={() => setEdit(undefined)}
                    />
                )}
                <ThemedButton
                    label={saving ? 'Saving…' : 'Save'}
                    variant={dirty && !problem && !saving ? 'primary' : 'disabled'}
                    onPress={save}
                />
            </View>
        </View>
    )
}

export default GuardrailEditor

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        heading: {
            flexDirection: 'row',
            alignItems: 'center',
            justifyContent: 'space-between',
            columnGap: spacing.m,
        },
        label: {
            color: color.text._200,
            fontSize: fontSize.m,
            fontWeight: '600',
        },
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        pair: {
            flexDirection: 'row',
            columnGap: spacing.m,
        },
        half: {
            flex: 1,
        },
        problem: {
            color: color.error._300,
            fontSize: fontSize.s,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            flexWrap: 'wrap',
            gap: spacing.m,
        },
    })
}
