import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Git screen lands: Commit history and branches. */
const GitScreen = () => (
    <Screen title="Git">
        <EmptyState icon="branches" title="Coming soon" text="Commit history and branches." />
    </Screen>
)

export default GitScreen
