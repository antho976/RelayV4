import React from 'react'
import { View } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'

import { stateColor } from './console'

/** One dot, coloured by what the session is doing. See `stateColor`. */
const Lamp: React.FC<{ state: string; size?: number }> = ({ state, size = 8 }) => {
    const { color } = Theme.useTheme()
    return (
        <View
            style={{
                width: size,
                height: size,
                borderRadius: size / 2,
                backgroundColor: stateColor(state, color),
            }}
        />
    )
}

export default Lamp
