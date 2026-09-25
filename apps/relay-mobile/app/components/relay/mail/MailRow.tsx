import { useRouter } from 'expo-router'
import React from 'react'
import { StyleSheet, Text, View } from 'react-native'

import { Chip, Tone } from '@components/relay/Kit'
import { Theme } from '@lib/theme/ThemeManager'

import { MailRecipient, RelayMessage, taskHref } from './types'

const when = (ts: string) => {
    const date = new Date(ts)
    if (Number.isNaN(date.getTime())) return ts
    const today = date.toDateString() === new Date().toDateString()
    return today
        ? date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
        : date.toLocaleDateString([], { month: 'short', day: 'numeric' }) +
              ' ' +
              date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

const recipientTone = (r: MailRecipient): Tone =>
    r.acked_at ? 'live' : r.state === 'running' || r.state === 'idle' ? 'primary' : 'neutral'

/**
 * One message: from → to, the time, priority and the task it is about, the text, and — for
 * the person's own sent mail — where each addressee stands (read, or its session state).
 */
const MailRow: React.FC<{
    message: RelayMessage
    recipients?: MailRecipient[]
    /** Draw a divider above; for rows after the first in a card. */
    divider?: boolean
}> = ({ message, recipients, divider }) => {
    const styles = useStyles()
    const router = useRouter()
    return (
        <View style={[styles.row, divider && styles.divider]}>
            <View style={styles.head}>
                <Text numberOfLines={1} style={styles.who}>
                    {message.from} → {message.to === '*' ? 'everyone' : message.to}
                </Text>
                <Text style={styles.when}>{when(message.sent_at)}</Text>
            </View>
            {(message.priority ||
                message.re_task !== null ||
                (!recipients && message.acked_at)) && (
                <View style={styles.chips}>
                    {message.priority && <Chip label="Priority" tone="warn" icon="thunderbolt" />}
                    {message.re_task !== null && (
                        <Chip
                            label={`Task #${message.re_task}`}
                            icon="link"
                            onPress={() => router.push(taskHref(message.re_task as number))}
                        />
                    )}
                    {!recipients && message.acked_at && <Chip label="Read" tone="live" />}
                </View>
            )}
            <Text selectable style={styles.text}>
                {message.text}
            </Text>
            {recipients && recipients.length > 0 && (
                <View style={styles.chips}>
                    {recipients.map((r) => (
                        <Chip
                            key={r.session}
                            label={`${r.session} · ${r.acked_at ? 'read' : r.state}`}
                            tone={recipientTone(r)}
                            icon={r.acked_at ? 'check' : undefined}
                        />
                    ))}
                </View>
            )}
        </View>
    )
}

export default MailRow

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        row: {
            rowGap: spacing.s,
            paddingVertical: spacing.l,
        },
        divider: {
            borderTopWidth: 1,
            borderTopColor: color.neutral._300,
        },
        head: {
            flexDirection: 'row',
            alignItems: 'baseline',
            columnGap: spacing.m,
        },
        who: {
            flex: 1,
            color: color.text._200,
            fontSize: fontSize.s,
            fontWeight: '600',
        },
        when: {
            color: color.text._500,
            fontSize: fontSize.s,
            fontVariant: ['tabular-nums'],
        },
        chips: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            columnGap: spacing.s,
            rowGap: spacing.s,
        },
        text: {
            color: color.text._100,
            fontSize: fontSize.m,
            lineHeight: 20,
        },
    })
}
