import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useRouter } from 'expo-router'
import React from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { RelayPeer, sessionHref, terminalHref } from '@components/relay/sessions'
import { RelaySession } from '@lib/engine/Relay/RelayClient'
import { Theme } from '@lib/theme/ThemeManager'

import { ago, stateColor } from './console'
import Lamp from './Lamp'

type SessionItemProps = {
    session: RelaySession
    /** Its row of `session.peers`: the agent's own line on what it is doing. */
    peer?: RelayPeer
}

/**
 * One card of the wall: lamp, name, state, which agent, where it works, when it last spoke,
 * and what it says it is doing. Tap for the terminal, long-press for the session's details.
 */
const SessionItem: React.FC<SessionItemProps> = ({ session, peer }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const held = session.state === 'blocked'
    const tint = stateColor(session.state, color)
    const where = session.branch
    const when = ago(session.last_output_at)
    // The peer table is the fresher source; a session event carries the intent too.
    const intent = peer?.intent || (session as RelaySession & { intent?: string | null }).intent
    const doing = intent || (peer?.task_title ? `Task: ${peer.task_title}` : '')
    return (
        <TouchableOpacity
            style={[styles.card, held && { borderColor: color.error._400 }]}
            onPress={() => router.push(terminalHref(session.name))}
            onLongPress={() => router.push(sessionHref(session.name))}>
            <View style={styles.head}>
                <Lamp state={session.state} />
                <Text numberOfLines={1} style={[styles.name, held && styles.nameHeld]}>
                    {session.name}
                </Text>
                <View style={[styles.pill, { borderColor: tint }]}>
                    <Text style={[styles.pillText, { color: tint }]}>{session.state}</Text>
                </View>
            </View>
            <View style={styles.meta}>
                <Text style={styles.agent}>
                    {session.provider} · {session.role}
                </Text>
                {!!where && (
                    <View style={styles.where}>
                        <AntDesign name="branches" size={12} color={color.text._500} />
                        <Text numberOfLines={1} style={styles.detail}>
                            {where}
                        </Text>
                    </View>
                )}
                {!!when && <Text style={styles.when}>{when} ago</Text>}
            </View>
            {!!doing && (
                <Text numberOfLines={2} style={[styles.intent, !intent && styles.task]}>
                    {doing}
                </Text>
            )}
        </TouchableOpacity>
    )
}

export default SessionItem

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        card: {
            rowGap: spacing.s,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.l,
            backgroundColor: color.neutral._200,
            borderRadius: 14,
            borderWidth: 1,
            borderColor: 'transparent',
        },
        head: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        name: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.m,
            fontWeight: '600',
        },
        nameHeld: {
            color: color.error._300,
        },
        pill: {
            borderWidth: 1,
            borderRadius: 10,
            paddingHorizontal: 8,
            paddingVertical: 1,
        },
        pillText: {
            fontSize: fontSize.s - 2,
            letterSpacing: 0.6,
            textTransform: 'uppercase',
        },
        meta: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingLeft: 8 + spacing.m,
        },
        agent: {
            color: color.text._300,
            fontSize: fontSize.s,
        },
        where: {
            flex: 1,
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: 4,
        },
        detail: {
            flexShrink: 1,
            color: color.text._400,
            fontSize: fontSize.s,
        },
        intent: {
            paddingLeft: 8 + spacing.m,
            color: color.text._200,
            fontSize: fontSize.s,
            fontStyle: 'italic',
        },
        task: {
            color: color.text._400,
            fontStyle: 'normal',
        },
        when: {
            color: color.text._500,
            fontSize: fontSize.s,
            fontVariant: ['tabular-nums'],
        },
    })
}
