import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import { relay, RelayHold } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

/**
 * A guardrail hold: an agent asked to do something the policy paused. The same two decisions
 * the desktop offers, through the same two ops — the engine still applies every other policy.
 */
const HoldItem: React.FC<{ hold: RelayHold }> = ({ hold }) => {
    const styles = useStyles()
    const [busy, setBusy] = useState(false)

    const decide = async (op: 'guardrail.confirm' | 'guardrail.reject') => {
        setBusy(true)
        try {
            await relay.request(op, { hold_id: hold.id })
            Logger.infoToast(op === 'guardrail.confirm' ? 'Allowed once' : 'Denied')
            await relay.refreshAttention()
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
        } finally {
            setBusy(false)
        }
    }

    const summary = summarize(hold)

    return (
        <View style={styles.record}>
            <View style={styles.head}>
                <Text style={styles.session}>{hold.session ?? 'system'}</Text>
                <Text style={styles.policy}>{hold.policy}</Text>
            </View>
            <Text style={styles.op}>{hold.op}</Text>
            {!!summary && (
                <Text numberOfLines={6} style={styles.detail}>
                    {summary}
                </Text>
            )}
            <View style={styles.actions}>
                <ThemedButton
                    label="Deny"
                    variant={busy ? 'disabled' : 'critical'}
                    onPress={() => decide('guardrail.reject')}
                />
                <ThemedButton
                    label="Allow once"
                    variant={busy ? 'disabled' : 'primary'}
                    onPress={() => decide('guardrail.confirm')}
                />
            </View>
        </View>
    )
}

export default HoldItem

const summarize = (hold: RelayHold): string => {
    const details = hold.details
    if (!details || typeof details !== 'object') return ''
    const parts: string[] = []
    for (const key of ['command', 'path', 'paths', 'reason', 'kind', 'message']) {
        const value = details[key]
        if (value === undefined || value === null) continue
        parts.push(`${key}: ${typeof value === 'string' ? value : JSON.stringify(value)}`)
    }
    if (parts.length === 0) {
        const text = JSON.stringify(details)
        return text.length > 400 ? text.slice(0, 400) + '…' : text
    }
    return parts.join('\n')
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        record: {
            backgroundColor: color.neutral._200,
            borderRadius: 12,
            padding: spacing.l,
            rowGap: spacing.s,
            borderLeftColor: color.error._300,
            borderLeftWidth: 3,
        },
        head: {
            flexDirection: 'row',
            justifyContent: 'space-between',
        },
        session: {
            color: color.error._300,
            fontWeight: '600',
        },
        policy: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        op: {
            color: color.text._100,
            fontFamily: 'monospace',
        },
        detail: {
            color: color.text._300,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
            marginTop: spacing.s,
        },
    })
}
