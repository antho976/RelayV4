import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Mailbox screen lands: Messages between you and the agents. */
const MailboxScreen = () => (
    <Screen title="Mailbox">
        <EmptyState icon="mail" title="Coming soon" text="Messages between you and the agents." />
    </Screen>
)

export default MailboxScreen
