import React from 'react'
import { View } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'

/**
 * Relay's session lamp: colour only where it means a state. Live green for running, held red
 * for blocked, the waiting amber for a session that is spawning, and quiet ink for the rest.
 */
export const lampColor = (
    state: string,
    color: ReturnType<typeof Theme.useTheme>['color']
): string => {
    switch (state) {
        case 'running':
            return '#2ec469'
        case 'blocked':
            return color.error._300
        case 'spawning':
            return color.quote
        case 'idle':
            return color.text._400
        default:
            return color.neutral._700
    }
}

const Lamp: React.FC<{ state: string; size?: number }> = ({ state, size = 8 }) => {
    const { color } = Theme.useTheme()
    return (
        <View
            style={{
                width: size,
                height: size,
                borderRadius: size / 2,
                backgroundColor: lampColor(state, color),
            }}
        />
    )
}

export default Lamp
