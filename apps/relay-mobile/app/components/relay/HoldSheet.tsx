import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import { useHoldPromptStore } from '@lib/engine/Relay/HoldPrompt'
import { HeldRequest, RelayHold } from '@lib/engine/Relay/RelayClient'
import { Theme } from '@lib/theme/ThemeManager'

import { Sheet, useSheetStyles } from './Sheet'

type Fact = { label: string; value: string }

const lineCount = (text: string) => (text ? text.split('\n').length : 0)

const show = (value: unknown): string =>
    typeof value === 'string'
        ? value
        : Array.isArray(value) && value.every((item) => typeof item === 'string')
          ? value.join('\n')
          : JSON.stringify(value)

/**
 * The facts of a hold a person decides on, in reading order: why (the policy's numbers, from
 * `hold.details` — a user bypass nests the original policy's under `original`), then what
 * would run (from the frozen request's payload). Large texts are summarised by line count.
 */
export const holdFacts = (hold: RelayHold, request?: HeldRequest): Fact[] => {
    const facts: Fact[] = []
    const raw = hold.details && typeof hold.details === 'object' ? hold.details : {}
    const details = {
        ...raw,
        ...(raw.original && typeof raw.original === 'object' ? raw.original : {}),
    }
    const add = (label: string, value: unknown) => {
        if (value === undefined || value === null || value === '') return
        facts.push({ label: label, value: show(value) })
    }
    add('Policy', details.policy ?? hold.policy)
    add('Reason', details.reason)
    add('Validator', details.validator)
    add('Pattern', details.pattern)
    if (details.removed_lines !== undefined) {
        const pct =
            typeof details.removed_pct === 'number' ? ` (${details.removed_pct.toFixed(1)}%)` : ''
        add(
            'Lines',
            `-${details.removed_lines}${pct} +${details.added_lines ?? 0} of ${details.old_lines ?? '?'}`
        )
        add('Limit', `${details.limit_lines} lines or ${details.limit_pct}%`)
    }
    if (details.files !== undefined && details.cap_files !== undefined) {
        add('Size', `${details.files} files, ${details.lines} lines`)
        add('Caps', `${details.cap_files} files, ${details.cap_lines} lines`)
    }
    const payload = request?.payload ?? {}
    add('Path', payload.path ?? details.path)
    add('From', payload.from)
    add('To', payload.to)
    add('Paths', payload.paths ?? details.paths)
    add('Command', payload.command ?? details.command)
    add('Worktree', payload.worktree)
    add('Branch', payload.branch ?? payload.name)
    add('Message', payload.message)
    if (typeof payload.text === 'string') add('New text', `${lineCount(payload.text)} lines`)
    if (typeof payload.new_text === 'string')
        add('New text', `${lineCount(payload.new_text)} lines`)
    if (typeof payload.diff === 'string') add('Diff', `${lineCount(payload.diff)} lines`)
    if (payload.all === true) add('Stage', 'all changes')
    return facts
}

/**
 * What a hold would do, for a person about to decide it: the op, who asked, the engine's
 * message, the facts above, and the exact frozen request behind a toggle. Used by the global
 * HoldSheet and by the Inbox's HoldItem.
 */
export const HoldDetails: React.FC<{
    hold: RelayHold
    request?: HeldRequest
    message?: string
}> = ({ hold, request, message }) => {
    const styles = useStyles()
    const [exact, setExact] = useState(false)
    const facts = holdFacts(hold, request)
    return (
        <View style={styles.details}>
            <View style={styles.head}>
                <Text style={styles.op}>{request?.op ?? hold.op}</Text>
                <Text style={styles.who}>{hold.session ?? request?.actor ?? 'user'}</Text>
            </View>
            {!!message && <Text style={styles.message}>{message}</Text>}
            {facts.map((fact, index) => (
                <View key={`${fact.label}${index}`} style={styles.fact}>
                    <Text style={styles.label}>{fact.label}</Text>
                    <Text numberOfLines={8} selectable style={styles.value}>
                        {fact.value}
                    </Text>
                </View>
            ))}
            {request && (
                <TouchableOpacity hitSlop={8} onPress={() => setExact(!exact)}>
                    <Text style={styles.link}>
                        {exact ? 'Hide the exact request' : 'Show the exact request'}
                    </Text>
                </TouchableOpacity>
            )}
            {request && exact && (
                <View style={styles.exactBox}>
                    <Text selectable style={styles.exact}>
                        {JSON.stringify({ op: request.op, payload: request.payload }, null, 2)}
                    </Text>
                </View>
            )}
        </View>
    )
}

/**
 * The global question for a held mutation of the phone's own (`relay.guarded`). Mounted once
 * in app/_layout.tsx; shows the head of the HoldPrompt queue. Back or a tap outside denies.
 */
const HoldSheet = () => {
    const sheet = useSheetStyles()
    const head = useHoldPromptStore((state) => state.queue[0])
    const answer = useHoldPromptStore((state) => state.answer)
    return (
        <Sheet visible={!!head} onDismiss={() => answer(false)}>
            {head && (
                <View style={sheet.body}>
                    <Text style={sheet.title}>Allow this held action once?</Text>
                    <ScrollView style={sheet.scroll}>
                        <HoldDetails
                            hold={head.hold}
                            request={head.request}
                            message={head.error.message}
                        />
                    </ScrollView>
                    <View style={sheet.actions}>
                        <ThemedButton
                            label="Deny"
                            variant="critical"
                            onPress={() => answer(false)}
                        />
                        <ThemedButton
                            label="Allow once"
                            variant="primary"
                            onPress={() => answer(true)}
                        />
                    </View>
                </View>
            )}
        </Sheet>
    )
}

export default HoldSheet

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        details: {
            rowGap: spacing.s,
        },
        head: {
            flexDirection: 'row',
            justifyContent: 'space-between',
            columnGap: spacing.m,
        },
        op: {
            flex: 1,
            color: color.text._100,
            fontFamily: 'monospace',
        },
        who: {
            color: color.error._300,
            fontWeight: '600',
        },
        message: {
            color: color.text._300,
            lineHeight: 20,
        },
        fact: {
            flexDirection: 'row',
            columnGap: spacing.m,
        },
        label: {
            width: 76,
            color: color.text._500,
            fontSize: fontSize.s,
        },
        value: {
            flex: 1,
            color: color.text._200,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        link: {
            color: color.primary._700,
            fontSize: fontSize.s,
            marginTop: spacing.xs,
        },
        exactBox: {
            borderRadius: 8,
            padding: spacing.m,
            backgroundColor: color.neutral._100,
        },
        exact: {
            color: color.text._300,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
    })
}
