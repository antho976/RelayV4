import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Add a project screen lands: Clone a repository or add a folder on the PC. */
const AddProjectScreen = () => (
    <Screen title="Add a project">
        <EmptyState
            icon="plus"
            title="Coming soon"
            text="Clone a repository or add a folder on the PC."
        />
    </Screen>
)

export default AddProjectScreen
