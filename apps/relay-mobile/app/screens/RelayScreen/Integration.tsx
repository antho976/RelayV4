import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Integration screen lands: Integration runs across sessions. */
const IntegrationScreen = () => (
    <Screen title="Integration">
        <EmptyState
            icon="experiment"
            title="Coming soon"
            text="Integration runs across sessions."
        />
    </Screen>
)

export default IntegrationScreen
