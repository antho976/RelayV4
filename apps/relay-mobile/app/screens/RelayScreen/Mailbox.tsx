import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, View } from 'react-native'

import {
    Chip,
    EmptyState,
    ErrorState,
    LoadingState,
    Screen,
    Section,
    Segmented,
    Sheet,
    useProjectParam,
    useRelayStore,
} from '@components/relay'
import { MailComposer, MailRow, useMail, useOutbox } from '@components/relay/mail'
import { Theme } from '@lib/theme/ThemeManager'

type Tab = 'all' | 'sent'

/**
 * A project's mailbox, as the desktop's: every message between the person and the agents
 * (newest first, filtered to one session's history on demand), the person's own sent mail
 * with where each addressee stands, and a composer for one session or everyone.
 */
const MailboxScreen = () => {
    const styles = useStyles()
    const { projectId, project } = useProjectParam()
    const [tab, setTab] = useState<Tab>('all')
    const [session, setSession] = useState<string | undefined>(undefined)
    const [composing, setComposing] = useState(false)
    const sessions = useRelayStore((state) => state.sessions)
    const names = sessions
        .filter((s) => s.project_id === projectId && s.state !== 'closed')
        .map((s) => s.name)

    const mail = useMail(projectId, session)
    const outbox = useOutbox(projectId, tab === 'sent')

    const reload = () => (tab === 'sent' ? outbox.reload() : mail.reload())

    // Sessions to filter by: live ones, plus any that appear in the mail already.
    const filters = Array.from(
        new Set([...names, ...(mail.data ?? []).flatMap((m) => [m.from, m.to])])
    ).filter((name) => name !== '*' && name !== 'user' && name !== 'system')

    const messages = mail.data ? [...mail.data].reverse() : undefined

    return (
        <Screen
            title={project ? `Mailbox · ${project.name}` : 'Mailbox'}
            actions={[
                {
                    icon: 'edit',
                    label: 'New message',
                    onPress: () => setComposing(true),
                    disabled: projectId === undefined,
                },
            ]}
            onRefresh={reload}
            refreshing={tab === 'sent' ? outbox.loading : mail.loading}>
            <Segmented
                options={[
                    { value: 'all', label: 'Messages' },
                    { value: 'sent', label: 'Sent by you' },
                ]}
                value={tab}
                onChange={setTab}
            />

            {tab === 'all' && (
                <>
                    {filters.length > 0 && (
                        <ScrollView
                            horizontal
                            showsHorizontalScrollIndicator={false}
                            contentContainerStyle={styles.filters}>
                            <Chip
                                label="All sessions"
                                selected={session === undefined}
                                onPress={() => setSession(undefined)}
                            />
                            {filters.map((name) => (
                                <Chip
                                    key={name}
                                    label={name}
                                    tone={names.includes(name) ? 'primary' : 'neutral'}
                                    selected={session === name}
                                    onPress={() => setSession(session === name ? undefined : name)}
                                />
                            ))}
                        </ScrollView>
                    )}
                    {messages === undefined ? (
                        mail.error ? (
                            <ErrorState error={mail.error} onRetry={mail.reload} />
                        ) : (
                            <LoadingState />
                        )
                    ) : messages.length === 0 ? (
                        <EmptyState
                            icon="mail"
                            title="No mail"
                            text={
                                session
                                    ? `Nothing to or from ${session} yet.`
                                    : 'Agents and you write here to coordinate.'
                            }
                            action={{ label: 'Write', onPress: () => setComposing(true) }}
                        />
                    ) : (
                        <Section title={session ? `${session} · ${messages.length}` : undefined}>
                            {messages.map((message, index) => (
                                <MailRow key={message.id} message={message} divider={index > 0} />
                            ))}
                        </Section>
                    )}
                </>
            )}

            {tab === 'sent' &&
                (outbox.data === undefined ? (
                    outbox.error ? (
                        <ErrorState error={outbox.error} onRetry={outbox.reload} />
                    ) : (
                        <LoadingState />
                    )
                ) : outbox.data.length === 0 ? (
                    <EmptyState
                        icon="export"
                        title="Nothing sent"
                        text="Messages you send show here with each agent's read state."
                    />
                ) : (
                    <Section>
                        {outbox.data.map((entry, index) => (
                            <MailRow
                                key={entry.message.id}
                                message={entry.message}
                                recipients={entry.recipients}
                                divider={index > 0}
                            />
                        ))}
                    </Section>
                ))}

            {projectId !== undefined && (
                <Sheet visible={composing} onDismiss={() => setComposing(false)}>
                    <View style={styles.sheet}>
                        <Text style={styles.sheetTitle}>New message</Text>
                        <MailComposer
                            projectId={projectId}
                            sessions={names}
                            onSent={() => {
                                setComposing(false)
                                mail.reload()
                                outbox.reload()
                            }}
                        />
                    </View>
                </Sheet>
            )}
        </Screen>
    )
}

export default MailboxScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        filters: {
            columnGap: spacing.s,
        },
        sheet: {
            rowGap: spacing.l,
        },
        sheetTitle: {
            color: color.text._100,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
    })
}
