import AntDesign from '@react-native-vector-icons/ant-design/static'
import React, { useRef, useState } from 'react'
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'

import { clearLog, MAX_LINES, useLogcat } from './logcat'

/** Logcat's priority letter in threadtime format: ` E ` after the pid/tid. */
const levelOf = (line: string) => line.match(/^\S+\s+\S+\s+\d+\s+\d+\s+([VDIWEF])\s/)?.[1]

/**
 * A run's log as it streams: the last lines, following the end unless you scroll up or pause,
 * with a filter. Only runs started from this phone have lines here.
 */
const LogView: React.FC<{ runId: number; active: boolean }> = ({ runId, active }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const { lines, error } = useLogcat(runId)
    const [paused, setPaused] = useState<string[] | undefined>(undefined)
    const [follow, setFollow] = useState(true)
    const [filter, setFilter] = useState<'all' | 'warn'>('all')
    const scroller = useRef<ScrollView>(null)

    const shown = paused ?? lines
    // Recomputed on each (batched) redraw; a join of 2000 lines is cheap next to a render.
    const picked =
        filter === 'all'
            ? shown
            : shown.filter((line) => {
                  const level = levelOf(line)
                  return level === 'W' || level === 'E' || level === 'F' || !level
              })
    const text = picked.slice(-MAX_LINES).join('\n')

    return (
        <View style={styles.box}>
            <View style={styles.bar}>
                <Text style={styles.count}>
                    {shown.length} lines{paused ? ' · paused' : active ? ' · live' : ''}
                </Text>
                <TouchableOpacity
                    hitSlop={8}
                    onPress={() => setFilter(filter === 'all' ? 'warn' : 'all')}>
                    <Text style={styles.button}>
                        {filter === 'all' ? 'Warnings only' : 'All levels'}
                    </Text>
                </TouchableOpacity>
                <TouchableOpacity
                    hitSlop={8}
                    accessibilityLabel={paused ? 'Resume' : 'Pause'}
                    onPress={() => setPaused(paused ? undefined : [...lines])}>
                    <AntDesign
                        name={paused ? 'play-circle' : 'pause-circle'}
                        size={20}
                        color={color.text._200}
                    />
                </TouchableOpacity>
                <TouchableOpacity
                    hitSlop={8}
                    accessibilityLabel="Clear"
                    onPress={() => {
                        setPaused(undefined)
                        clearLog(runId)
                    }}>
                    <AntDesign name="delete" size={18} color={color.text._200} />
                </TouchableOpacity>
            </View>
            {!!error && <Text style={styles.error}>{error}</Text>}
            <ScrollView
                ref={scroller}
                style={styles.scroll}
                onContentSizeChange={() => {
                    if (follow && !paused) scroller.current?.scrollToEnd({ animated: false })
                }}
                onScroll={(event) => {
                    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent
                    const atEnd =
                        contentOffset.y + layoutMeasurement.height >= contentSize.height - 40
                    if (atEnd !== follow) setFollow(atEnd)
                }}
                scrollEventThrottle={100}>
                <Text selectable style={styles.text}>
                    {text || (active ? 'Waiting for output…' : 'No output.')}
                </Text>
            </ScrollView>
            {!follow && !paused && (
                <TouchableOpacity
                    style={styles.jump}
                    onPress={() => {
                        setFollow(true)
                        scroller.current?.scrollToEnd({ animated: true })
                    }}>
                    <AntDesign name="arrow-down" size={14} color="#fff" />
                    <Text style={styles.jumpText}>Follow</Text>
                </TouchableOpacity>
            )}
        </View>
    )
}

export default LogView

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        box: {
            flex: 1,
            backgroundColor: color.neutral._200,
            borderRadius: 12,
            overflow: 'hidden',
        },
        bar: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            borderBottomWidth: 1,
            borderBottomColor: color.neutral._300,
        },
        count: {
            flex: 1,
            color: color.text._400,
            fontSize: fontSize.s,
        },
        button: {
            color: color.primary._700,
            fontSize: fontSize.s,
        },
        error: {
            color: color.error._300,
            fontSize: fontSize.s,
            paddingHorizontal: spacing.l,
            paddingTop: spacing.m,
        },
        scroll: {
            flex: 1,
        },
        text: {
            color: color.text._200,
            fontFamily: 'monospace',
            fontSize: 11,
            lineHeight: 15,
            padding: spacing.m,
        },
        jump: {
            position: 'absolute',
            right: spacing.l,
            bottom: spacing.l,
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: 4,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.s,
            borderRadius: 999,
            backgroundColor: color.primary._500,
        },
        jumpText: {
            color: '#fff',
            fontSize: fontSize.s,
        },
    })
}
