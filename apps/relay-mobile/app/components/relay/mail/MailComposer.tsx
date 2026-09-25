import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import { Chip, Field, SwitchRow } from '@components/relay/Kit'
import { isCancelled, relay, RelayRequestError } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { deliveryText, SendResult } from './types'

/**
 * Write to an agent (mailbox.send as the person): one session or everyone (`*`), normal or
 * priority (the agent reads it at its next safe point; one session only, as on the desktop),
 * optionally about a task. With `to`, the addressee is fixed.
 */
const MailComposer: React.FC<{
    projectId: number
    /** A fixed addressee; hides the picker. */
    to?: string
    /** Sessions to offer in the picker. */
    sessions?: string[]
    onSent?: (result: SendResult) => void
}> = ({ projectId, to, sessions = [], onSent }) => {
    const styles = useStyles()
    const [recipient, setRecipient] = useState(to ?? '')
    const [text, setText] = useState('')
    const [priority, setPriority] = useState(false)
    const [reTask, setReTask] = useState('')
    const [busy, setBusy] = useState(false)

    const target = to ?? recipient
    const broadcast = target === '*'
    const taskId = reTask.trim() ? Number(reTask.trim().replace(/^#/, '')) : undefined
    const taskValid = taskId === undefined || (Number.isInteger(taskId) && taskId > 0)
    const ready = !!target && !!text.trim() && taskValid && !busy

    const send = async () => {
        if (!ready) return
        setBusy(true)
        try {
            const result = await relay.guarded<SendResult>('mailbox.send', {
                project_id: projectId,
                to: target,
                text: text,
                priority: priority && !broadcast,
                ...(taskId !== undefined ? { re_task: taskId } : {}),
            })
            setText('')
            setPriority(false)
            setReTask('')
            Logger.infoToast(deliveryText(result))
            onSent?.(result)
        } catch (e) {
            if (!isCancelled(e))
                Logger.errorToast(
                    e instanceof RelayRequestError ? e.error.message : (e as Error).message
                )
        } finally {
            setBusy(false)
        }
    }

    return (
        <View style={styles.form}>
            {to === undefined && (
                <View style={{ rowGap: 6 }}>
                    <Text style={styles.label}>To</Text>
                    <ScrollView
                        horizontal
                        showsHorizontalScrollIndicator={false}
                        keyboardShouldPersistTaps="handled"
                        contentContainerStyle={styles.chips}>
                        <Chip
                            label="Everyone"
                            icon="team"
                            tone="primary"
                            selected={recipient === '*'}
                            onPress={() => setRecipient('*')}
                        />
                        {sessions.map((name) => (
                            <Chip
                                key={name}
                                label={name}
                                tone="primary"
                                selected={recipient === name}
                                onPress={() => setRecipient(name)}
                            />
                        ))}
                    </ScrollView>
                    {sessions.length === 0 && (
                        <Text style={styles.note}>No live sessions in this project.</Text>
                    )}
                </View>
            )}
            <Field
                value={text}
                onChangeText={setText}
                placeholder={
                    to ? `Message to ${to}` : broadcast ? 'Message to every session' : 'Message'
                }
                multiline
                lines={3}
            />
            <SwitchRow
                label="Priority"
                description={
                    broadcast
                        ? 'Priority mail goes to one session.'
                        : 'Ask the agent to read it at its next safe point.'
                }
                value={priority && !broadcast}
                onChange={setPriority}
                disabled={broadcast}
            />
            <View style={styles.bottom}>
                <View style={{ flex: 1 }}>
                    <Field
                        value={reTask}
                        onChangeText={setReTask}
                        placeholder="About task # (optional)"
                        keyboardType="number-pad"
                    />
                </View>
                <ThemedButton
                    label={busy ? 'Sending…' : 'Send'}
                    iconName="send"
                    variant={ready ? 'primary' : 'disabled'}
                    onPress={send}
                />
            </View>
            {!taskValid && <Text style={styles.error}>A task number, like 12.</Text>}
        </View>
    )
}

export default MailComposer

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        form: {
            rowGap: spacing.m,
        },
        label: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        chips: {
            columnGap: spacing.s,
        },
        bottom: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        error: {
            color: color.error._300,
            fontSize: fontSize.s,
        },
    })
}
