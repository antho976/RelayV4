import { activateKeepAwakeAsync, deactivateKeepAwake } from 'expo-keep-awake'
import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { ScrollView, StyleSheet, Text, TextInput, TouchableOpacity, View } from 'react-native'
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
import { TerminalText } from '@lib/engine/Relay/Terminal'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

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

/**
 * One session's terminal, as text. Scrollback first, then live frames; keystrokes go straight
 * to the PTY through `session.input`, which the engine answers without touching its store.
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
    const [text, setText] = useState('')
    const [input, setInput] = useState('')
    const [attached, setAttached] = useState(false)
    const [problem, setProblem] = useState('')
    const mailSheet = useBottomSheetRef()
    const summarySheet = useRef<SummarySheetRef>(null)
    const router = useRouter()
    const screen = useRef(new TerminalText())
    const scroll = useRef<ScrollView>(null)
    const dirty = useRef(false)
    const stick = useRef(true)

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
    // re-runs this and attaches then.
    // A boolean, not the state itself: idle/running flips must not rebuild the screen.
    const sessionState = session?.state
    const hasPty = sessionState !== 'created' && sessionState !== 'spawning'
    useEffect(() => {
        if (!name || status !== 'online') return
        if (!hasPty) return
        let detach: (() => void) | undefined
        let cancelled = false
        const timer = setInterval(() => {
            if (!dirty.current) return
            dirty.current = false
            setText(screen.current.text())
        }, RENDER_INTERVAL_MS)
        ;(async () => {
            try {
                const back = await relay.request<{ text: string; epoch: number; seq: number }>(
                    'session.scrollback',
                    { session: name, lines: 400 }
                )
                if (cancelled) return
                screen.current.reset(back.text)
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
            detach?.()
            setAttached(false)
        }
    }, [name, status, hasPty])

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
                />
                <ScrollView
                    ref={scroll}
                    style={styles.screen}
                    contentContainerStyle={styles.screenContent}
                    onContentSizeChange={() => {
                        if (stick.current) scroll.current?.scrollToEnd({ animated: false })
                    }}
                    onScroll={(event) => {
                        const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent
                        stick.current =
                            contentOffset.y + layoutMeasurement.height >= contentSize.height - 40
                    }}
                    scrollEventThrottle={100}>
                    {!!problem && <Text style={styles.problem}>{problem}</Text>}
                    {!problem && status !== 'online' && (
                        <Text style={styles.problem}>Not connected to the PC.</Text>
                    )}
                    <Text selectable style={styles.mono}>
                        {text}
                    </Text>
                </ScrollView>
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
}> = ({ session, onLifecycle, onMail, onChanges, onSummarize }) => {
    const styles = useStyles()
    if (!session) return null
    const canMail =
        session.state === 'running' || session.state === 'idle' || session.state === 'blocked'
    const canPark =
        session.state === 'running' || session.state === 'idle' || session.state === 'blocked'
    const canWake = session.state === 'parked'
    const canResume = session.state === 'restorable'
    return (
        <View style={styles.strip}>
            <Lamp state={session.state} />
            <Text
                numberOfLines={1}
                style={[styles.stripName, session.state === 'blocked' && styles.held]}>
                {session.name}
            </Text>
            <Text style={styles.stripMeta}>
                {session.provider} · {session.role} · {session.state}
            </Text>
            <TouchableOpacity onPress={onSummarize}>
                <Text style={styles.stripAction}>sum</Text>
            </TouchableOpacity>
            <TouchableOpacity onPress={onChanges}>
                <Text style={styles.stripAction}>diff</Text>
            </TouchableOpacity>
            {canMail && (
                <TouchableOpacity onPress={onMail}>
                    <Text style={styles.stripAction}>mail</Text>
                </TouchableOpacity>
            )}
            {canPark && (
                <TouchableOpacity onPress={() => onLifecycle('session.park')}>
                    <Text style={styles.stripAction}>park</Text>
                </TouchableOpacity>
            )}
            {canWake && (
                <TouchableOpacity onPress={() => onLifecycle('session.wake')}>
                    <Text style={styles.stripAction}>wake</Text>
                </TouchableOpacity>
            )}
            {canResume && (
                <TouchableOpacity onPress={() => onLifecycle('session.resume')}>
                    <Text style={styles.stripAction}>resume</Text>
                </TouchableOpacity>
            )}
        </View>
    )
}

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        strip: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.m,
            minHeight: 30,
            backgroundColor: color.neutral._200,
            borderBottomColor: color.neutral._400,
            borderBottomWidth: 1,
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
        stripAction: {
            color: color.text._200,
            fontSize: fontSize.s,
            paddingHorizontal: spacing.s,
            letterSpacing: 0.8,
            textTransform: 'uppercase',
        },
        screen: {
            flex: 1,
            backgroundColor: '#0a0a0b',
        },
        screenContent: {
            paddingLeft: 10,
            paddingRight: 4,
            paddingTop: 8,
            paddingBottom: 12,
        },
        mono: {
            color: '#ececea',
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
            backgroundColor: '#0a0a0b',
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
