import React, { useState } from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { EmptyState, ErrorState, LoadingState } from '@components/relay/Kit'
import { Theme } from '@lib/theme/ThemeManager'

import MailComposer from './MailComposer'
import MailRow from './MailRow'
import { useMail } from './types'

/**
 * One session's mail, for embedding (the Terminal screen): what it received — its own read
 * state, broadcasts included — and what it sent, oldest first, then a composer addressed to
 * it. Shows the latest `limit` messages, with a link to the earlier ones.
 */
const MailThread: React.FC<{
    project_id: number
    session: string
    /** Messages shown before "Show earlier". Default 20. */
    limit?: number
    /** Hide the composer (read-only history). */
    readOnly?: boolean
}> = ({ project_id, session, limit = 20, readOnly }) => {
    const styles = useStyles()
    const mail = useMail(project_id, session)
    const [all, setAll] = useState(false)

    const list = mail.data ?? []
    const shown = all ? list : list.slice(-limit)
    const hidden = list.length - shown.length

    return (
        <View style={styles.thread}>
            {mail.data === undefined ? (
                mail.error ? (
                    <ErrorState error={mail.error} onRetry={mail.reload} />
                ) : (
                    <LoadingState />
                )
            ) : list.length === 0 ? (
                <EmptyState icon="mail" text={`No mail to or from ${session} yet.`} />
            ) : (
                <View style={styles.card}>
                    {hidden > 0 && (
                        <TouchableOpacity hitSlop={8} onPress={() => setAll(true)}>
                            <Text style={styles.more}>Show {hidden} earlier</Text>
                        </TouchableOpacity>
                    )}
                    {shown.map((message, index) => (
                        <MailRow
                            key={message.id}
                            message={message}
                            divider={index > 0 || hidden > 0}
                        />
                    ))}
                </View>
            )}
            {!readOnly && (
                <MailComposer projectId={project_id} to={session} onSent={() => mail.reload()} />
            )}
        </View>
    )
}

export default MailThread

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        thread: {
            rowGap: spacing.l,
        },
        card: {
            backgroundColor: color.neutral._200,
            borderRadius: 16,
            paddingHorizontal: spacing.l,
        },
        more: {
            color: color.primary._700,
            fontSize: fontSize.s,
            paddingVertical: spacing.m,
            textAlign: 'center',
        },
    })
}
