import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import React from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'

export type Segment<T extends string> = { value: T; label: string; icon?: AntDesignIconName }

type SegmentedControlProps<T extends string> = {
    segments: readonly Segment<T>[]
    selected: T
    onChange: (value: T) => void
}

/** A row of pills in a sunken track; exactly one is lit. */
const SegmentedControl = <T extends string>({
    segments,
    selected,
    onChange,
}: SegmentedControlProps<T>) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <View style={styles.track} accessibilityRole="tablist">
            {segments.map((segment) => {
                const active = segment.value === selected
                return (
                    <TouchableOpacity
                        key={segment.value}
                        accessibilityRole="tab"
                        accessibilityState={{ selected: active }}
                        style={[styles.segment, active && styles.active]}
                        onPress={() => !active && onChange(segment.value)}>
                        {segment.icon && (
                            <AntDesign
                                name={segment.icon}
                                size={15}
                                color={active ? color.text._100 : color.text._500}
                            />
                        )}
                        <Text
                            numberOfLines={1}
                            style={[styles.label, active && styles.labelActive]}>
                            {segment.label}
                        </Text>
                    </TouchableOpacity>
                )
            })}
        </View>
    )
}

export default SegmentedControl

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        track: {
            flexDirection: 'row',
            backgroundColor: color.neutral._100,
            borderRadius: 999,
            padding: spacing.s,
            columnGap: spacing.s,
        },
        segment: {
            flex: 1,
            flexDirection: 'row',
            alignItems: 'center',
            justifyContent: 'center',
            columnGap: spacing.sm,
            paddingVertical: spacing.m + 2,
            paddingHorizontal: spacing.s,
            borderRadius: 999,
        },
        active: {
            backgroundColor: color.neutral._400,
        },
        label: {
            color: color.text._500,
            fontSize: fontSize.m,
        },
        labelActive: {
            color: color.text._100,
            fontWeight: '600',
        },
    })
}
