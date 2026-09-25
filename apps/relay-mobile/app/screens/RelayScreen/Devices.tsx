import React from 'react'

import { EmptyState, Screen } from '@components/relay'

/** Placeholder until the Android devices screen lands: Run and build on Android devices. */
const DevicesScreen = () => (
    <Screen title="Android devices">
        <EmptyState icon="mobile" title="Coming soon" text="Run and build on Android devices." />
    </Screen>
)

export default DevicesScreen
