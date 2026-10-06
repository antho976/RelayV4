import AntDesign from '@react-native-vector-icons/ant-design/static'
import React from 'react'
import { StyleSheet, Text, View } from 'react-native'

import { ThemeColor } from '@lib/theme/ThemeColor'
import { Theme } from '@lib/theme/ThemeManager'

type ThemeCardProps = {
    theme: ThemeColor
    selected: boolean
    /** A small note under the name, such as "Custom" or "In use". */
    note?: string
}

/**
 * One theme in the picker grid: a thumbnail of a chat in that theme's own colours — page, a
 * bubble, reply lines, the input with its accent — and the name underneath in the page's.
 */
const ThemeCard: React.FC<ThemeCardProps> = ({ theme, selected, note }) => {
    const { color } = Theme.useTheme()
    return (
        <View style={styles.cell}>
            <View
                style={[
                    styles.thumb,
                    {
                        backgroundColor: theme.neutral._100,
                        borderColor: selected ? color.primary._500 : color.neutral._300,
                    },
                ]}>
                <View style={styles.row}>
                    <View style={[styles.dot, { backgroundColor: theme.neutral._400 }]} />
                    <View
                        style={[styles.bar, { width: '38%', backgroundColor: theme.text._300 }]}
                    />
                </View>
                <View style={[styles.bubble, { backgroundColor: theme.neutral._300 }]}>
                    <View
                        style={[styles.bar, { width: '80%', backgroundColor: theme.text._200 }]}
                    />
                </View>
                <View style={{ rowGap: 5 }}>
                    <View
                        style={[styles.bar, { width: '86%', backgroundColor: theme.text._400 }]}
                    />
                    <View style={styles.row}>
                        <View
                            style={[styles.bar, { width: '44%', backgroundColor: theme.text._400 }]}
                        />
                        <View
                            style={[styles.bar, { width: '22%', backgroundColor: theme.quote }]}
                        />
                    </View>
                </View>
                <View style={[styles.input, { backgroundColor: theme.neutral._200 }]}>
                    <View style={[styles.accent, { backgroundColor: theme.primary._500 }]} />
                </View>
                {selected && (
                    <View style={[styles.check, { backgroundColor: color.primary._500 }]}>
                        <AntDesign name="check" size={12} color={color.primary._100} />
                    </View>
                )}
            </View>
            <View style={styles.caption}>
                <Text
                    numberOfLines={1}
                    style={[
                        styles.name,
                        { color: selected ? color.text._100 : color.text._200 },
                        selected && styles.nameSelected,
                    ]}>
                    {theme.name}
                </Text>
                {!!note && (
                    <Text numberOfLines={1} style={[styles.note, { color: color.text._500 }]}>
                        {note}
                    </Text>
                )}
            </View>
        </View>
    )
}

export default ThemeCard

const styles = StyleSheet.create({
    cell: {
        rowGap: 8,
    },
    thumb: {
        height: 118,
        borderRadius: 18,
        borderWidth: 2,
        padding: 10,
        rowGap: 8,
        justifyContent: 'space-between',
    },
    row: {
        flexDirection: 'row',
        alignItems: 'center',
        columnGap: 6,
    },
    dot: {
        width: 10,
        height: 10,
        borderRadius: 5,
    },
    bar: {
        height: 5,
        borderRadius: 3,
    },
    bubble: {
        alignSelf: 'flex-end',
        width: '58%',
        paddingHorizontal: 7,
        paddingVertical: 6,
        borderRadius: 8,
        alignItems: 'flex-end',
    },
    input: {
        height: 16,
        borderRadius: 8,
        paddingHorizontal: 3,
        alignItems: 'flex-end',
        justifyContent: 'center',
    },
    accent: {
        width: 11,
        height: 11,
        borderRadius: 6,
    },
    check: {
        position: 'absolute',
        top: 8,
        right: 8,
        width: 22,
        height: 22,
        borderRadius: 11,
        alignItems: 'center',
        justifyContent: 'center',
    },
    caption: {
        paddingHorizontal: 4,
    },
    name: {
        fontSize: 14,
    },
    nameSelected: {
        fontWeight: '600',
    },
    note: {
        fontSize: 12,
        marginTop: 1,
    },
})
