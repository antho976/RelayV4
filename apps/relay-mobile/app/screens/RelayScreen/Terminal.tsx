import { activateKeepAwakeAsync, deactivateKeepAwake } from 'expo-keep-awake'
import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import {
    AppState,
    LayoutChangeEvent,
    ScrollView,
    StyleSheet,
    Text,
    TextInput,
    TouchableOpacity,
    View,
} from 'react-native'
import { useReanimatedKeyboardAnimation } from 'react-native-keyboard-controller'
import { useMMKVBoolean } from 'react-native-mmkv'
import Animated, { useAnimatedStyle } from 'react-native-reanimated'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import { useBottomSheetRef } from '@components/views/BottomSheet'
import HeaderTitle from '@components/views/HeaderTitle'
import InputSheet from '@components/views/InputSheet'
import { AppSettings } from '@lib/constants/GlobalValues'
import { relay, RelaySession, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { VtScreen } from '@lib/engine/Relay/Terminal'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { palette } from './console'
import Lamp from './Lamp'
import SummarySheet, { SummarySheetRef } from './SummarySheet'

/** Keys a phone keyboard does not have and an agent CLI keeps asking for. */
const KEYS: { label: string; data: string }[] = [
    { label: 'Esc', data: '\x1b' },
    { label: 'Tab', data: '\t' },
    { label: '↑', data: '\x1b[A' },
    { label: '↓', data: '\x1b[B' },
    { label: '←', data: '\x1b[D' },
    { label: '→', data: '\x1b[C' },
    { label: '^C', data: '\x03' },
    { label: 'y', data: 'y\r' },
    { label: 'n', data: 'n\r' },
]

/** How often the screen text is rebuilt while output streams. */
const RENDER_INTERVAL_MS = 60

/** A new size settles this long before the PTY hears of it: one rotation is several layouts. */
const FIT_SETTLE_MS = 150

/** The screen's padding, which the column and row counts leave out. */
const PAD = { left: 10, right: 4, top: 8, bottom: 12 }

/** Measured in the terminal's font for the width of one column and the height of one row. */
const RULER = 'M'.repeat(20)

type Size = { cols: number; rows: number }

/**
 * One session's terminal. Scrollback first, then live frames, through a screen that follows
 * the cursor the way the PC's terminal does; keystrokes go straight to the PTY through
 * `session.input`, which the engine answers without touching its store.
 *
 * While it is open the screen borrows the PTY at the phone's own width, so an agent's CLI
 * lays itself out for the phone instead of being cut off at the PC's. The engine hands the
 * PC's size back when the screen detaches or the link drops; "Phone width" off keeps the PC's
 * size and scrolls sideways instead.
 */
const TerminalScreen = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const insets = useSafeAreaInsets()
    const params = useLocalSearchParams<{ session: string }>()
    const name = typeof params.session === 'string' ? params.session : ''
    const session = useRelayStore((state) => state.sessions.find((item) => item.name === name))
    const status = useRelayStore((state) => state.status)
    const [keepOn] = useMMKVBoolean(AppSettings.RelayKeepScreenOn)
    const [fitSetting, setFitSetting] = useMMKVBoolean(AppSettings.RelayFitTerminal)
    const fit = fitSetting !== false
    const [text, setText] = useState('')
    const [input, setInput] = useState('')
    const [attached, setAttached] = useState(false)
    const [problem, setProblem] = useState('')
    const mailSheet = useBottomSheetRef()
    const summarySheet = useRef<SummarySheetRef>(null)
    const router = useRouter()
    const screen = useRef(new VtScreen())
    const scroll = useRef<ScrollView>(null)
    const dirty = useRef(false)
    const stick = useRef(true)
    /** The PTY's size on the PC, as the engine reported it when the screen attached. */
    const [pcSize, setPcSize] = useState<Size>()
    const [cell, setCell] = useState<{ width: number; height: number }>()
    const [area, setArea] = useState<{ width: number; height: number }>()
    const [foreground, setForeground] = useState(AppState.currentState === 'active')
    /** This screen holds the PTY at the phone's size. */
    const borrowed = useRef(false)

    const { height } = useReanimatedKeyboardAnimation()
    const animatedStyle = useAnimatedStyle(() => ({
        paddingBottom: -height.value - insets.bottom,
        flex: 1,
    }))

    // The phone stays awake while a terminal is open; a person reading output is not idle.
    useEffect(() => {
        if (keepOn === false) return
        activateKeepAwakeAsync('relay-terminal').catch(() => {})
        return () => {
            deactivateKeepAwake('relay-terminal').catch(() => {})
        }
    }, [keepOn])

    // A session that was just launched has no PTY until it is running; the state change
    // re-runs this and attaches then. Park then Wake or Resume starts a new process under
    // the same name, so the pid is a dependency too: the view re-attaches to the new PTY.
    // Not the state itself: idle/running flips must not rebuild the screen.
    const launched = !!session && session.state !== 'created'
    const pid = session?.pid ?? null
    useEffect(() => {
        if (!name || status !== 'online') return
        if (!launched) return
        let detach: (() => void) | undefined
        let cancelled = false
        const timer = setInterval(() => {
            if (!dirty.current) return
            dirty.current = false
            setText(screen.current.text())
        }, RENDER_INTERVAL_MS)
        ;(async () => {
            try {
                const back = await relay.request<{
                    text: string
                    epoch: number
                    seq: number
                    cols?: number
                    rows?: number
                }>('session.scrollback', { session: name, lines: 400 })
                if (cancelled) return
                // An engine too old to report its size cannot lend it either: that PC's
                // terminals stay at its width, read sideways.
                const size =
                    back.cols && back.rows ? { cols: back.cols, rows: back.rows } : undefined
                screen.current.reset(back.text, size?.cols ?? 120, size?.rows ?? 40)
                setPcSize(size)
                dirty.current = true
                detach = await relay.attach(
                    name,
                    (_frame, chunk) => {
                        screen.current.feed(chunk)
                        dirty.current = true
                    },
                    { epoch: back.epoch, seq: back.seq }
                )
                if (cancelled) {
                    detach()
                    return
                }
                setAttached(true)
                setProblem('')
            } catch (e) {
                if (!cancelled) setProblem(`${(e as Error).message}`)
            }
        })()
        return () => {
            cancelled = true
            clearInterval(timer)
            // Detaching is what hands a borrowed size back; a dropped link does it as well.
            detach?.()
            borrowed.current = false
            setAttached(false)
        }
    }, [name, status, launched, pid])

    // The phone's columns and rows: the screen's size over one character's.
    const phone: Size | undefined =
        cell && area
            ? {
                  cols: Math.max(
                      20,
                      Math.floor((area.width - PAD.left - PAD.right - 1) / cell.width)
                  ),
                  rows: Math.max(4, Math.floor((area.height - PAD.top - PAD.bottom) / cell.height)),
              }
            : undefined
    const lend = fit && !!phone && !!pcSize
    const target = !attached || !pcSize ? undefined : lend && phone ? phone : pcSize
    const targetCols = target?.cols
    const targetRows = target?.rows

    // Fit the PTY to what the screen shows. Borrowed, the size goes back to the PC by itself
    // when this screen detaches; set for good, it ends the loan, which is "Phone width" off.
    useEffect(() => {
        if (!targetCols || !targetRows || !foreground) return
        const timer = setTimeout(() => {
            screen.current.resize(targetCols, targetRows)
            dirty.current = true
            if (!lend && !borrowed.current) return
            borrowed.current = lend
            relay
                .request('session.resize', {
                    session: name,
                    cols: targetCols,
                    rows: targetRows,
                    ...(lend ? { until_detach: true } : {}),
                })
                .catch((e) => Logger.warn(`Could not fit the terminal: ${(e as Error).message}`))
        }, FIT_SETTLE_MS)
        return () => clearTimeout(timer)
    }, [name, targetCols, targetRows, lend, foreground])

    // Put away, the phone hands the terminal back at once, so whoever sits down at the PC
    // finds it as they left it. Back in front, the effect above borrows it again.
    useEffect(() => {
        const subscription = AppState.addEventListener('change', (next) => {
            setForeground(next === 'active')
            if (next !== 'background' || !borrowed.current || !pcSize) return
            borrowed.current = false
            relay
                .request('session.resize', { session: name, cols: pcSize.cols, rows: pcSize.rows })
                .catch(() => {})
        })
        return () => subscription.remove()
    }, [name, pcSize])

    const onScreenLayout = (event: LayoutChangeEvent) => {
        const { width, height } = event.nativeEvent.layout
        // Rows follow the screen as it is with the keyboard down. The keyboard coming up only
        // covers the top of the agent's screen; resizing for it would have the agent redraw
        // everything each time it does.
        setArea((prev) =>
            !prev || Math.abs(prev.width - width) >= 1 || height > prev.height + 1
                ? { width, height }
                : prev
        )
    }

    // Wide lines scroll sideways instead of wrapping into each other.
    const wide = !lend

    const send = useCallback(
        (data: string) => {
            if (!attached) return
            relay.input(name, data)
        },
        [attached, name]
    )

    const handleSend = () => {
        if (!input) {
            send('\r')
            return
        }
        // A pasted multi-line text still means one Enter per line to the agent's CLI.
        send(input.replace(/\r?\n/g, '\r') + '\r')
        setInput('')
    }

    /**
     * Mail reaches an agent that is busy: the engine hands it over at the agent's next bus
     * call, where a keystroke would only sit in the PTY until it reads its prompt.
     */
    const mail = async (body: string) => {
        if (!session) return
        try {
            await relay.request('mailbox.send', {
                project_id: session.project_id,
                to: session.name,
                text: body,
                priority: true,
            })
            Logger.infoToast(`Mailed ${session.name}`)
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
        }
    }

    const lifecycle = async (op: string) => {
        try {
            await relay.request(op, { session: name })
            await relay.refresh()
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
        }
    }

    return (
        <View style={{ flex: 1, paddingBottom: insets.bottom }}>
            <HeaderTitle title={name || 'Terminal'} />
            <InputSheet
                ref={mailSheet}
                title="Mail the agent"
                description="Delivered as priority mail: the agent sees it at its next step even while it is busy. For an agent waiting at its prompt, typing below is quicker."
                placeholder="Also update the changelog when you are done."
                multiline
                onConfirm={mail}
            />
            <Animated.View style={animatedStyle}>
                <SummarySheet ref={summarySheet} session={name} />
                <Strip
                    session={session}
                    onLifecycle={lifecycle}
                    onMail={() => mailSheet.current?.open()}
                    onChanges={() =>
                        router.push({
                            pathname: '/screens/RelayScreen/Changes',
                            params: { session: name },
                        })
                    }
                    onSummarize={() => summarySheet.current?.open(screen.current.text())}
                    fit={pcSize ? fit : undefined}
                    onFit={() => setFitSetting(!fit)}
                />
                <View style={styles.screen} onLayout={onScreenLayout}>
                    <Text
                        style={[styles.mono, styles.ruler]}
                        onLayout={(event) => {
                            const { width, height } = event.nativeEvent.layout
                            if (width > 0 && height > 0)
                                setCell({ width: width / RULER.length, height: height })
                        }}>
                        {RULER}
                    </Text>
                    <ScrollView
                        ref={scroll}
                        contentContainerStyle={styles.screenContent}
                        onContentSizeChange={() => {
                            if (stick.current) scroll.current?.scrollToEnd({ animated: false })
                        }}
                        onScroll={(event) => {
                            const { contentOffset, contentSize, layoutMeasurement } =
                                event.nativeEvent
                            stick.current =
                                contentOffset.y + layoutMeasurement.height >=
                                contentSize.height - 40
                        }}
                        scrollEventThrottle={100}>
                        {!!problem && <Text style={styles.problem}>{problem}</Text>}
                        {!problem && status !== 'online' && (
                            <Text style={styles.problem}>Not connected to the PC.</Text>
                        )}
                        {wide ? (
                            <ScrollView horizontal showsHorizontalScrollIndicator={false}>
                                <Text selectable style={styles.mono}>
                                    {text}
                                </Text>
                            </ScrollView>
                        ) : (
                            <Text selectable style={styles.mono}>
                                {text}
                            </Text>
                        )}
                    </ScrollView>
                </View>
                <View style={styles.keys}>
                    {KEYS.map((key) => (
                        <TouchableOpacity
                            key={key.label}
                            style={styles.key}
                            disabled={!attached}
                            onPress={() => send(key.data)}>
                            <Text style={[styles.keyText, !attached && { opacity: 0.45 }]}>
                                {key.label}
                            </Text>
                        </TouchableOpacity>
                    ))}
                </View>
                <View style={styles.inputRow}>
                    <TextInput
                        style={styles.input}
                        value={input}
                        onChangeText={setInput}
                        placeholder={attached ? 'Type to the session…' : 'Waiting for the session…'}
                        placeholderTextColor={color.text._500}
                        autoCapitalize="none"
                        autoCorrect={false}
                        multiline
                        editable={attached}
                        submitBehavior="submit"
                        returnKeyType="send"
                        onSubmitEditing={handleSend}
                    />
                    <ThemedButton
                        label={input ? 'Send' : 'Enter'}
                        variant={attached ? 'primary' : 'disabled'}
                        onPress={handleSend}
                    />
                </View>
            </Animated.View>
        </View>
    )
}

export default TerminalScreen

const Strip: React.FC<{
    session?: RelaySession
    onLifecycle: (op: string) => void
    onMail: () => void
    onChanges: () => void
    onSummarize: () => void
    /** Whether the PTY is fitted to the phone; absent when the PC cannot lend it. */
    fit?: boolean
    onFit: () => void
}> = ({ session, onLifecycle, onMail, onChanges, onSummarize, fit, onFit }) => {
    const styles = useStyles()
    if (!session) return null
    const awake =
        session.state === 'running' || session.state === 'idle' || session.state === 'blocked'
    // The actions that apply right now, in the order a person reaches for them.
    const actions: { label: string; onPress: () => void; on?: boolean }[] = [
        { label: 'Summary', onPress: onSummarize },
        { label: 'Changes', onPress: onChanges },
        ...(awake ? [{ label: 'Mail', onPress: onMail }] : []),
        ...(awake ? [{ label: 'Park', onPress: () => onLifecycle('session.park') }] : []),
        ...(session.state === 'parked'
            ? [{ label: 'Wake', onPress: () => onLifecycle('session.wake') }]
            : []),
        ...(session.state === 'restorable'
            ? [{ label: 'Resume', onPress: () => onLifecycle('session.resume') }]
            : []),
        ...(fit !== undefined ? [{ label: 'Phone width', onPress: onFit, on: fit }] : []),
    ]
    return (
        <View style={styles.strip}>
            <View style={styles.identity}>
                <Lamp state={session.state} />
                <Text
                    numberOfLines={1}
                    style={[styles.stripName, session.state === 'blocked' && styles.held]}>
                    {session.name}
                </Text>
                <Text numberOfLines={1} style={styles.stripMeta}>
                    {session.provider} · {session.role} · {session.state}
                </Text>
            </View>
            <ScrollView
                horizontal
                showsHorizontalScrollIndicator={false}
                contentContainerStyle={styles.actions}>
                {actions.map((action) => (
                    <TouchableOpacity
                        key={action.label}
                        style={[styles.action, action.on && styles.actionOn]}
                        hitSlop={6}
                        onPress={action.onPress}>
                        <Text style={[styles.actionText, action.on && styles.actionTextOn]}>
                            {action.label}
                        </Text>
                    </TouchableOpacity>
                ))}
            </ScrollView>
        </View>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        strip: {
            rowGap: spacing.s,
            paddingHorizontal: spacing.m,
            paddingVertical: spacing.s,
            backgroundColor: color.neutral._200,
            borderBottomColor: color.neutral._400,
            borderBottomWidth: 1,
        },
        identity: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        stripName: {
            flexShrink: 1,
            color: color.text._100,
            fontWeight: '600',
        },
        held: {
            color: color.error._300,
        },
        stripMeta: {
            flex: 1,
            color: color.text._400,
            fontSize: fontSize.s,
        },
        actions: {
            columnGap: spacing.s,
        },
        action: {
            paddingHorizontal: spacing.m,
            paddingVertical: 3,
            borderRadius: 12,
            backgroundColor: color.neutral._300,
        },
        actionText: {
            color: color.text._200,
            fontSize: fontSize.s,
        },
        actionOn: {
            backgroundColor: color.primary._500,
        },
        actionTextOn: {
            color: color.primary._100,
        },
        screen: {
            flex: 1,
            backgroundColor: palette.ink,
        },
        screenContent: {
            paddingLeft: PAD.left,
            paddingRight: PAD.right,
            paddingTop: PAD.top,
            paddingBottom: PAD.bottom,
        },
        ruler: {
            position: 'absolute',
            opacity: 0,
        },
        mono: {
            color: palette.paper,
            fontFamily: 'monospace',
            fontSize: 12,
            lineHeight: 16,
        },
        problem: {
            color: color.error._300,
            marginBottom: spacing.m,
        },
        keys: {
            flexDirection: 'row',
            backgroundColor: color.neutral._200,
            borderTopColor: color.neutral._400,
            borderTopWidth: 1,
            paddingHorizontal: 2,
            paddingVertical: 2,
            columnGap: 2,
        },
        key: {
            flex: 1,
            minHeight: 28,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.neutral._300,
            borderRadius: 2,
        },
        keyText: {
            color: color.text._100,
            fontSize: fontSize.s,
            fontFamily: 'monospace',
        },
        inputRow: {
            flexDirection: 'row',
            alignItems: 'flex-end',
            columnGap: spacing.m,
            padding: spacing.m,
            backgroundColor: color.neutral._200,
        },
        input: {
            flex: 1,
            minHeight: 36,
            maxHeight: 120,
            color: color.text._100,
            backgroundColor: palette.ink,
            borderColor: color.neutral._500,
            borderWidth: 1,
            borderRadius: 2,
            paddingHorizontal: spacing.m,
            paddingVertical: spacing.sm,
            fontFamily: 'monospace',
            fontSize: 13,
        },
    })
}
