import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Modules screen lands: The project's modules and their tasks. */
const ModulesScreen = () => (
    <Screen title="Modules">
        <EmptyState
            icon="appstore"
            title="Coming soon"
            text="The project's modules and their tasks."
        />
    </Screen>
)

export default ModulesScreen
