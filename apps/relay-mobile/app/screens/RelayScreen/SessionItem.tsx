import { useRouter } from 'expo-router'
import React from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { RelayProject, RelaySession } from '@lib/engine/Relay/RelayClient'
import { Theme } from '@lib/theme/ThemeManager'

import { ago } from './console'
import Lamp from './Lamp'

type SessionItemProps = {
    session: RelaySession
    project?: RelayProject
}

/** One row of the wall: lamp, name, where it works, and when it last said anything. */
const SessionItem: React.FC<SessionItemProps> = ({ session, project }) => {
    const styles = useStyles()
    const router = useRouter()
    const held = session.state === 'blocked'
    const where = `${project?.name ?? `project #${session.project_id}`} · ${session.branch}`
    const when = session.last_output_at ? ago(session.last_output_at) : ''
    return (
        <TouchableOpacity
            style={styles.row}
            onPress={() =>
                router.push({
                    pathname: '/screens/RelayScreen/Terminal',
                    params: { session: session.name },
                })
            }>
            <Lamp state={session.state} />
            <View style={styles.body}>
                <View style={styles.head}>
                    <Text numberOfLines={1} style={[styles.name, held && styles.nameHeld]}>
                        {session.name}
                    </Text>
                    <Text style={styles.state}>{session.state}</Text>
                </View>
                <Text numberOfLines={1} style={styles.detail}>
                    {where}
                </Text>
            </View>
            {!!when && <Text style={styles.when}>{when}</Text>}
        </TouchableOpacity>
    )
}

export default SessionItem

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        row: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            backgroundColor: color.neutral._200,
            borderRadius: 12,
        },
        body: {
            flex: 1,
            rowGap: 2,
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
        state: {
            color: color.text._400,
            fontSize: fontSize.s - 1,
            letterSpacing: 0.8,
            textTransform: 'uppercase',
        },
        detail: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        when: {
            color: color.text._500,
            fontSize: fontSize.s,
            fontVariant: ['tabular-nums'],
        },
    })
}
