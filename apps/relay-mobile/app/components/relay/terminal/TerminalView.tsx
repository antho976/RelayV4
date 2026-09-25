import AntDesign from '@react-native-vector-icons/ant-design/static'
import React, { memo, useCallback, useEffect, useRef, useState } from 'react'
import {
    FlatList,
    LayoutChangeEvent,
    NativeScrollEvent,
    NativeSyntheticEvent,
    ScrollView,
    StyleSheet,
    Text,
    TouchableOpacity,
    View,
} from 'react-native'
import { Gesture, GestureDetector } from 'react-native-gesture-handler'

import { TermRow, TermSpan } from '@lib/engine/Relay/TerminalEmulator'
import { Theme } from '@lib/theme/ThemeManager'

import { TERM_INK, TERM_PAPER } from './colors'

/** Smallest text the terminal is drawn at, fitted or not. */
export const MIN_FONT = 7
export const MAX_FONT = 18
const PAD_X = 6
/** Android's monospace advance is 0.6 em; measured on the device once it has laid out. */
const DEFAULT_ADVANCE = 0.6
const SAMPLE = 'M'.repeat(40)

type Props = {
    rows: TermRow[]
    cols: number
    /** Text size in points; undefined fits the PTY's width to the screen. */
    fontSize?: number
    onFontSize: (size: number | undefined) => void
    /** The size the text is drawn at right now, fitted or chosen. */
    onSize?: (size: number) => void
    /** Drawn over the screen when there is nothing to show yet, or a problem. */
    overlay?: React.ReactNode
}

/**
 * The emulator's rows as monospace text: history and screen in one list that follows the
 * output unless the person scrolled up (then a "Jump to latest" pill). The PTY is wider than
 * a phone: by default the text is sized so a whole row fits, down to 7 pt; pinch to zoom, and
 * a zoomed screen scrolls sideways too.
 */
const TerminalView: React.FC<Props> = ({ rows, cols, fontSize, onFontSize, onSize, overlay }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const [width, setWidth] = useState(0)
    const [advance, setAdvance] = useState(DEFAULT_ADVANCE)
    const [pinch, setPinch] = useState<number | undefined>(undefined)
    const [following, setFollowing] = useState(true)
    const follow = useRef(true)
    const list = useRef<FlatList<TermRow>>(null)

    const fitted = width > 0 ? (width - PAD_X * 2) / (cols * advance) : 10
    const size = Math.max(MIN_FONT, Math.min(MAX_FONT, fontSize ?? fitted))
    const lineHeight = Math.ceil(size * 1.22)
    const contentWidth = Math.ceil(cols * advance * size + PAD_X * 2)
    const wide = contentWidth > width + 1

    useEffect(() => {
        onSize?.(size)
    }, [size, onSize])

    // Output arrived: stay on the newest row, unless the person is reading further up.
    useEffect(() => {
        if (follow.current && rows.length > 0) list.current?.scrollToEnd({ animated: false })
    }, [rows, lineHeight])

    const onScroll = useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
        const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent
        const atEnd = contentOffset.y + layoutMeasurement.height >= contentSize.height - 24
        if (follow.current !== atEnd) {
            follow.current = atEnd
            setFollowing(atEnd)
        }
    }, [])

    const jump = () => {
        follow.current = true
        setFollowing(true)
        list.current?.scrollToEnd({ animated: true })
    }

    const pinchGesture = Gesture.Pinch()
        .runOnJS(true)
        .onUpdate((event) => setPinch(event.scale))
        .onEnd((event) => {
            setPinch(undefined)
            const next = size * event.scale
            // Back near the fitted size (or below it) means "fit" again.
            if (Math.abs(next - fitted) < 0.6 || next < fitted) onFontSize(undefined)
            else onFontSize(Math.min(MAX_FONT, Math.round(next * 2) / 2))
        })
        .onFinalize(() => setPinch(undefined))

    const renderItem = useCallback(
        ({ item }: { item: TermRow }) => (
            <Line row={item} fontSize={size} lineHeight={lineHeight} />
        ),
        [size, lineHeight]
    )

    const body = (
        <FlatList
            ref={list}
            data={rows}
            renderItem={renderItem}
            keyExtractor={keyOf}
            getItemLayout={(_, index) => ({
                length: lineHeight,
                offset: lineHeight * index,
                index: index,
            })}
            style={{ width: wide ? contentWidth : '100%' }}
            contentContainerStyle={styles.content}
            onScroll={onScroll}
            scrollEventThrottle={64}
            initialNumToRender={80}
            maxToRenderPerBatch={60}
            windowSize={5}
            removeClippedSubviews
            onContentSizeChange={() => {
                if (follow.current) list.current?.scrollToEnd({ animated: false })
            }}
            // The keyboard opening shrinks the screen: keep the newest row in view.
            onLayout={() => {
                if (follow.current) list.current?.scrollToEnd({ animated: false })
            }}
        />
    )

    return (
        <View
            style={styles.screen}
            onLayout={(event: LayoutChangeEvent) => setWidth(event.nativeEvent.layout.width)}>
            {/* Measures the font's advance so the fitted size is exact. */}
            <Text
                style={[styles.measure, { fontSize: 20 }]}
                onLayout={(event) => {
                    const measured = event.nativeEvent.layout.width / (SAMPLE.length * 20)
                    if (measured > 0.3 && measured < 1) setAdvance(measured)
                }}>
                {SAMPLE}
            </Text>
            <GestureDetector gesture={pinchGesture}>
                <View style={styles.fill} collapsable={false}>
                    {wide ? (
                        <ScrollView horizontal bounces={false} style={styles.fill}>
                            {body}
                        </ScrollView>
                    ) : (
                        body
                    )}
                </View>
            </GestureDetector>
            {overlay && <View style={styles.overlay}>{overlay}</View>}
            {pinch !== undefined && (
                <View style={styles.zoomBadge} pointerEvents="none">
                    <Text style={styles.zoomText}>
                        {Math.max(MIN_FONT, Math.min(MAX_FONT, size * pinch)).toFixed(1)} pt
                    </Text>
                </View>
            )}
            {!following && rows.length > 0 && (
                <TouchableOpacity style={styles.jump} onPress={jump}>
                    <AntDesign name="arrow-down" size={13} color={color.text._100} />
                    <Text style={styles.jumpText}>Jump to latest</Text>
                </TouchableOpacity>
            )}
        </View>
    )
}

export default TerminalView

const keyOf = (_: TermRow, index: number) => String(index)

/** One row. Rows that did not change keep their object, so they are not drawn again. */
const Line = memo(function Line({
    row,
    fontSize,
    lineHeight,
}: {
    row: TermRow
    fontSize: number
    lineHeight: number
}) {
    return (
        <Text
            numberOfLines={1}
            style={[
                lineStyle.text,
                { fontSize: fontSize, lineHeight: lineHeight, height: lineHeight },
            ]}>
            {row.spans.length === 0 ? ' ' : row.spans.map(spanNode)}
        </Text>
    )
})

const spanNode = (span: TermSpan, index: number) => {
    if (
        !span.fg &&
        !span.bg &&
        !span.bold &&
        !span.dim &&
        !span.italic &&
        !span.underline &&
        !span.strike
    )
        return span.text
    return (
        <Text
            key={index}
            style={{
                color: span.fg,
                backgroundColor: span.bg,
                fontWeight: span.bold ? '700' : undefined,
                fontStyle: span.italic ? 'italic' : undefined,
                opacity: span.dim ? 0.6 : undefined,
                textDecorationLine:
                    span.underline && span.strike
                        ? 'underline line-through'
                        : span.underline
                          ? 'underline'
                          : span.strike
                            ? 'line-through'
                            : undefined,
            }}>
            {span.text}
        </Text>
    )
}

const lineStyle = StyleSheet.create({
    text: {
        color: TERM_PAPER,
        fontFamily: 'monospace',
        includeFontPadding: false,
    },
})

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        screen: {
            flex: 1,
            backgroundColor: TERM_INK,
            borderRadius: 14,
            overflow: 'hidden',
        },
        fill: {
            flex: 1,
        },
        content: {
            paddingHorizontal: PAD_X,
            paddingTop: spacing.s,
            paddingBottom: spacing.m,
        },
        measure: {
            position: 'absolute',
            opacity: 0,
            fontFamily: 'monospace',
            includeFontPadding: false,
        },
        overlay: {
            ...StyleSheet.absoluteFill,
            alignItems: 'center',
            justifyContent: 'center',
            padding: spacing.xl,
        },
        jump: {
            position: 'absolute',
            bottom: spacing.l,
            alignSelf: 'center',
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.s,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.s,
            borderRadius: 999,
            backgroundColor: color.neutral._300,
            borderWidth: StyleSheet.hairlineWidth,
            borderColor: color.neutral._500,
            elevation: 3,
        },
        jumpText: {
            color: color.text._100,
            fontSize: fontSize.s,
        },
        zoomBadge: {
            position: 'absolute',
            top: spacing.m,
            alignSelf: 'center',
            paddingHorizontal: spacing.m,
            paddingVertical: 3,
            borderRadius: 999,
            backgroundColor: color.neutral._300,
        },
        zoomText: {
            color: color.text._200,
            fontSize: fontSize.s,
            fontVariant: ['tabular-nums'],
        },
    })
}
