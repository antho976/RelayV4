import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Project settings screen lands: Name, build and run commands, guardrails. */
const ProjectSettingsScreen = () => (
    <Screen title="Project settings">
        <EmptyState
            icon="setting"
            title="Coming soon"
            text="Name, build and run commands, guardrails."
        />
    </Screen>
)

export default ProjectSettingsScreen
