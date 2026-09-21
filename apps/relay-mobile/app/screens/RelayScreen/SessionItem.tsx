import { useRouter } from 'expo-router'
import React from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { RelayProject, RelaySession } from '@lib/engine/Relay/RelayClient'
import { Theme } from '@lib/theme/ThemeManager'

import Lamp from './Lamp'

/** "3m" for three minutes ago; empty when there was never any output. */
export const ago = (iso: string | null | undefined, now = Date.now()): string => {
    if (!iso) return ''
    const then = Date.parse(iso)
    if (Number.isNaN(then)) return ''
    const seconds = Math.max(0, Math.floor((now - then) / 1000))
    if (seconds < 60) return `${seconds}s`
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m`
    if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`
    return `${Math.floor(seconds / 86400)}d`
}

type SessionItemProps = {
    session: RelaySession
    project?: RelayProject
}

/** One plate of the wall: the identity strip a desktop terminal carries, without the terminal. */
const SessionItem: React.FC<SessionItemProps> = ({ session, project }) => {
    const styles = useStyles()
    const router = useRouter()
    const held = session.state === 'blocked'
    return (
        <TouchableOpacity
            style={styles.plate}
            onPress={() =>
                router.push({
                    pathname: '/screens/RelayScreen/Terminal',
                    params: { session: session.name },
                })
            }>
            <View style={styles.strip}>
                <Lamp state={session.state} />
                <Text numberOfLines={1} style={[styles.name, held && styles.nameHeld]}>
                    {session.name}
                </Text>
                <Text style={styles.meta}>
                    {session.provider} · {session.role}
                </Text>
                <Text style={styles.state}>{session.state}</Text>
            </View>
            <View style={styles.body}>
                <Text numberOfLines={1} style={styles.detail}>
                    {project?.name ?? `project #${session.project_id}`} · {session.branch}
                </Text>
                <Text style={styles.detail}>
                    {session.task_id != null ? `task #${session.task_id} · ` : ''}
                    {session.last_output_at
                        ? `output ${ago(session.last_output_at)} ago`
                        : 'no output yet'}
                </Text>
            </View>
        </TouchableOpacity>
    )
}

export default SessionItem

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        plate: {
            backgroundColor: color.neutral._100,
            borderColor: color.neutral._400,
            borderWidth: 1,
        },
        strip: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.m,
            minHeight: 30,
            backgroundColor: color.neutral._200,
            borderBottomColor: color.neutral._400,
            borderBottomWidth: 1,
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
        meta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        state: {
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 0.8,
            textTransform: 'uppercase',
        },
        body: {
            paddingHorizontal: spacing.m,
            paddingVertical: spacing.sm,
            rowGap: 2,
        },
        detail: {
            color: color.text._400,
            fontSize: fontSize.s,
            fontFamily: 'monospace',
        },
    })
}
