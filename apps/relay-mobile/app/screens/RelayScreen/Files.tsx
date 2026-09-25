import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Files screen lands: The project's files. */
const FilesScreen = () => (
    <Screen title="Files">
        <EmptyState icon="folder" title="Coming soon" text="The project's files." />
    </Screen>
)

export default FilesScreen
