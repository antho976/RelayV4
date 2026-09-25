import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Notes screen lands: The project's notes. */
const NotesScreen = () => (
    <Screen title="Notes">
        <EmptyState icon="file-text" title="Coming soon" text="The project's notes." />
    </Screen>
)

export default NotesScreen
