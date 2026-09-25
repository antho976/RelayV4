import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the PC settings screen lands: Settings of the PC itself: notifications, guardrails, providers, skills, backups. */
const PcSettingsScreen = () => (
    <Screen title="PC settings">
        <EmptyState
            icon="setting"
            title="Coming soon"
            text="Settings of the PC itself: notifications, guardrails, providers, skills, backups."
        />
    </Screen>
)

export default PcSettingsScreen
