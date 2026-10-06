import React, { ReactNode } from 'react'
import { StyleSheet, Text, View, ViewStyle } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'

type SettingsGroupProps = {
    title: string
    detail?: string
    children: ReactNode
    /** Drop the card's own padding, for content that draws edge to edge. */
    flush?: boolean
    style?: ViewStyle
}

/** A heading over one rounded, filled card: how the settings pages group what belongs together. */
const SettingsGroup: React.FC<SettingsGroupProps> = ({ title, detail, children, flush, style }) => {
    const styles = useStyles()
    return (
        <View style={styles.group}>
            <Text style={styles.title}>{title}</Text>
            <View style={[styles.card, !flush && styles.padded, style]}>
                {!!detail && <Text style={styles.detail}>{detail}</Text>}
                {children}
            </View>
        </View>
    )
}

export default SettingsGroup

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        group: {
            rowGap: spacing.m,
        },
        title: {
            color: color.text._400,
            fontSize: fontSize.m,
            marginLeft: spacing.m,
        },
        card: {
            backgroundColor: color.neutral._200,
            borderRadius: 22,
            overflow: 'hidden',
        },
        padded: {
            padding: spacing.xl,
            rowGap: spacing.l,
        },
        detail: {
            color: color.text._500,
            fontSize: fontSize.m,
        },
    })
}
