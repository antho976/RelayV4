import AntDesign from '@react-native-vector-icons/ant-design/static'
import { setStringAsync } from 'expo-clipboard'
import { activateKeepAwakeAsync, deactivateKeepAwake } from 'expo-keep-awake'
import { useLocalSearchParams, useRouter } from 'expo-router'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { ActivityIndicator, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { useReanimatedKeyboardAnimation } from 'react-native-keyboard-controller'
import { useMMKVBoolean } from 'react-native-mmkv'
import Animated, { useAnimatedStyle } from 'react-native-reanimated'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import { sessionHref, useSessionLifecycle } from '@components/relay/sessions'
import { MenuItem, MenuSheet } from '@components/relay/settings/common'
import {
    Composer,
    fitSize,
    MAX_FONT,
    MIN_FONT,
    TerminalView,
    TermKey,
    TermMeasure,
    useTerminal,
} from '@components/relay/terminal'
import { useBottomSheetRef } from '@components/views/BottomSheet'
import HeaderButton from '@components/views/HeaderButton'
import HeaderTitle from '@components/views/HeaderTitle'
import InputSheet from '@components/views/InputSheet'
import { AppSettings } from '@lib/constants/GlobalValues'
import { isCancelled, relay, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { answerKey, submitText, TerminalSender } from '@lib/engine/Relay/TerminalInput'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import Lamp from './Lamp'
import SummarySheet, { SummarySheetRef } from './SummarySheet'

/** States with a live process behind the terminal. */
const isAwake = (state?: string) => state === 'running' || state === 'idle' || state === 'blocked'

/** The text size a terminal fitted to the phone starts at, before any zoom. */
const FIT_FONT = 12

/**
 * One session's terminal. The PTY's bytes go through a terminal emulator and are drawn as
 * the desktop draws them; the composer and keys write to the PTY through `session.input`,
 * which the engine answers without touching its store. Text is sent, and its Enter follows in
 * a write of its own, so an agent CLI submits it instead of taking it for a paste.
 *
 * While it is open the screen borrows the PTY at the phone's own width, so the agent lays
 * itself out for the phone instead of being shrunk or cut off at the PC's; a new text size
 * reflows it. "Use the PC's width" keeps the PC's size, fitted to the screen.
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
    const [measure, setMeasure] = useState<TermMeasure>()
    const [fontSize, setFontSize] = useState<number | undefined>(undefined)
    const [menu, setMenu] = useState(false)
    const mailSheet = useBottomSheetRef()
    const summarySheet = useRef<SummarySheetRef>(null)
    const router = useRouter()
    const queue = useRef<Promise<void>>(Promise.resolve())
    const drawn = useRef(MIN_FONT)

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
    // re-runs the feed and attaches then. Park then Wake or Resume starts a new process under
    // the same name, so the pid re-attaches too. Not the state itself: idle/running flips must
    // not rebuild the screen.
    const launched = !!session && session.state !== 'created'
    const pid = session?.pid ?? null
    const fitFont = fontSize ?? FIT_FONT
    const want = fit && measure ? fitSize(measure, fitFont) : undefined
    const feed = useTerminal(name, status === 'online' && launched, pid, want)
    const live = feed.attached && isAwake(session?.state)

    const sender: TerminalSender = {
        write: (data) => relay.inputWritten(name, data),
        key: (data) => relay.input(name, data),
    }
    /** Keystrokes and submissions leave in the order they were made. */
    const enqueue = (job: () => Promise<void>) => {
        queue.current = queue.current.then(job).catch((e) => {
            Logger.errorToast(`${(e as Error).message}`)
        })
    }
    const onSubmit = (text: string) =>
        enqueue(() => submitText(sender, text, { bracketedPaste: feed.emulator().bracketedPaste }))
    const onKey = (key: TermKey) =>
        enqueue(async () => {
            if (key.answer) await answerKey(sender, key.data)
            else sender.key(key.data)
        })

    /**
     * Mail reaches an agent that is busy: the engine hands it over at the agent's next bus
     * call, where a keystroke would only sit in the PTY until it reads its prompt.
     */
    const mail = async (body: string) => {
        if (!session) return
        try {
            await relay.guarded('mailbox.send', {
                project_id: session.project_id,
                to: session.name,
                text: body,
                priority: true,
            })
            Logger.infoToast(`Mailed ${session.name}`)
        } catch (e) {
            if (!isCancelled(e)) Logger.errorToast(`${(e as Error).message}`)
        }
    }

    // Start, Park, Wake, Resume, Clear context and Close, as the desktop's pane header has
    // them. A closed session has no terminal left to show.
    const lifecycle = useSessionLifecycle(session, { onClosed: () => router.back() })
    const openInfo = () => router.push(sessionHref(name))
    // The action that brings a stopped session back leads, on the screen itself.
    const revive = lifecycle.actions.find(
        (action) => action.key === 'start' || action.key === 'wake' || action.key === 'resume'
    )

    const cols = feed.snapshot.cols
    // The PTY is at the phone's size: draw it at the size it was fitted for.
    const fitted = !!want && !!feed.pcSize && cols === want.cols
    const onSize = useCallback((size: number) => {
        drawn.current = size
    }, [])
    const zoom = (step: number) =>
        setFontSize((current) => {
            const base = current ?? drawn.current
            return Math.max(MIN_FONT, Math.min(MAX_FONT, Math.round(base + step)))
        })

    const items: MenuItem[] = [
        {
            label: 'Summarize on this phone',
            icon: 'robot',
            onPress: () => summarySheet.current?.open(feed.emulator().plainText()),
        },
        {
            label: 'Changes',
            icon: 'diff',
            onPress: () =>
                router.push({
                    pathname: '/screens/RelayScreen/Changes',
                    params: { session: name },
                }),
        },
        ...(isAwake(session?.state)
            ? [
                  {
                      label: 'Mail the agent',
                      icon: 'mail' as const,
                      onPress: () => mailSheet.current?.open(),
                  },
              ]
            : []),
        {
            label: 'Copy terminal text',
            icon: 'copy',
            onPress: () => {
                setStringAsync(feed.emulator().plainText())
                    .then(() => Logger.infoToast('Copied'))
                    .catch(() => {})
            },
        },
        { label: 'Larger text', icon: 'zoom-in', onPress: () => zoom(1) },
        { label: 'Smaller text', icon: 'zoom-out', onPress: () => zoom(-1) },
        ...(fontSize !== undefined
            ? [
                  {
                      label: fit ? 'Default text size' : 'Fit width',
                      icon: 'column-width' as const,
                      onPress: () => setFontSize(undefined),
                  },
              ]
            : []),
        // Only an engine that reports the PTY's size can lend it.
        ...(feed.pcSize
            ? [
                  {
                      label: fit ? "Use the PC's width" : 'Fit to this phone',
                      icon: fit ? ('desktop' as const) : ('mobile' as const),
                      onPress: () => setFitSetting(!fit),
                  },
              ]
            : []),
        { label: 'Session details', icon: 'info-circle', onPress: openInfo },
        ...lifecycle.actions.map((action) => ({
            label: action.label,
            icon:
                action.key === 'park'
                    ? ('pause-circle' as const)
                    : action.key === 'close'
                      ? ('close-circle' as const)
                      : action.key === 'clear'
                        ? ('reload' as const)
                        : ('play-circle' as const),
            destructive: action.destructive,
            onPress: () => lifecycle.run(action.key),
        })),
    ]

    const overlay = (() => {
        if (status !== 'online')
            return (
                <Text style={styles.note}>
                    {status === 'connecting' ? 'Connecting to the PC…' : 'Not connected to the PC.'}
                </Text>
            )
        if (!session) return <Text style={styles.note}>This session is gone.</Text>
        if (feed.problem) return <Text style={styles.problem}>{feed.problem}</Text>
        if (!launched) return <Text style={styles.note}>Not started yet.</Text>
        if (feed.loading && feed.snapshot.rows.length === 0)
            return <ActivityIndicator color={color.text._400} />
        return undefined
    })()

    return (
        <View style={{ flex: 1, paddingBottom: insets.bottom }}>
            <HeaderTitle
                title={name || 'Terminal'}
                headerTitle={() => (
                    <TouchableOpacity style={styles.title} onPress={openInfo} disabled={!name}>
                        <View style={styles.titleLine}>
                            {!!session && <Lamp state={session.state} size={7} />}
                            <Text numberOfLines={1} style={styles.titleName}>
                                {name || 'Terminal'}
                            </Text>
                        </View>
                        {!!session && (
                            <Text numberOfLines={1} style={styles.titleMeta}>
                                {session.provider} · {session.role} · {session.state}
                            </Text>
                        )}
                    </TouchableOpacity>
                )}
            />
            <HeaderButton
                headerRight={() =>
                    name ? (
                        <View style={styles.headerActions}>
                            <TouchableOpacity
                                hitSlop={10}
                                accessibilityLabel="Session details"
                                onPress={openInfo}>
                                <AntDesign name="info-circle" size={20} color={color.text._200} />
                            </TouchableOpacity>
                            <TouchableOpacity
                                hitSlop={10}
                                accessibilityLabel="More"
                                onPress={() => setMenu(true)}>
                                <AntDesign name="ellipsis" size={22} color={color.text._200} />
                            </TouchableOpacity>
                        </View>
                    ) : null
                }
            />
            {lifecycle.sheets}
            <MenuSheet
                visible={menu}
                title={name}
                detail={
                    session
                        ? `${session.provider} · ${session.role} · ${cols} columns${fitted ? ', fitted to this phone' : ''}`
                        : undefined
                }
                items={items}
                onDismiss={() => setMenu(false)}
            />
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
                <View style={styles.stage}>
                    <TerminalView
                        rows={feed.snapshot.rows}
                        cols={cols}
                        fontSize={fitted ? fitFont : fontSize}
                        onFontSize={setFontSize}
                        onSize={onSize}
                        onMeasure={setMeasure}
                        reflow={fitted}
                        overlay={overlay}
                    />
                </View>
                {!!session && (feed.saved || (!!revive && !live)) && (
                    <View style={styles.banner}>
                        <Text numberOfLines={1} style={styles.bannerText}>
                            {feed.saved
                                ? `Saved output · ${session.state}`
                                : `Session is ${session.state}`}
                        </Text>
                        {!!revive && (
                            <TouchableOpacity
                                style={styles.pill}
                                disabled={!!lifecycle.busy}
                                onPress={() => lifecycle.run(revive.key)}>
                                <AntDesign
                                    name="caret-right"
                                    size={12}
                                    color={color.primary._100}
                                />
                                <Text style={styles.pillText}>
                                    {lifecycle.busy === revive.key
                                        ? `${revive.label}…`
                                        : revive.label}
                                </Text>
                            </TouchableOpacity>
                        )}
                    </View>
                )}
                <Composer
                    enabled={live}
                    placeholder={live ? 'Message or command…' : 'Waiting for the session…'}
                    onKey={onKey}
                    onSubmit={onSubmit}
                />
            </Animated.View>
        </View>
    )
}

export default TerminalScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        title: {
            alignItems: 'center',
            maxWidth: 240,
        },
        titleLine: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.s,
        },
        titleName: {
            flexShrink: 1,
            color: color.text._100,
            fontFamily: 'serif',
            fontSize: 19,
        },
        titleMeta: {
            color: color.text._500,
            fontSize: fontSize.s - 1,
        },
        headerActions: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.xl,
            marginRight: spacing.s,
        },
        stage: {
            flex: 1,
            paddingHorizontal: spacing.s,
        },
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
            textAlign: 'center',
        },
        problem: {
            color: color.error._300,
            fontSize: fontSize.s,
            textAlign: 'center',
        },
        banner: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            marginHorizontal: spacing.m,
            marginTop: spacing.s,
            paddingLeft: spacing.l,
            paddingRight: spacing.s,
            paddingVertical: spacing.s,
            borderRadius: 999,
            backgroundColor: color.neutral._200,
        },
        bannerText: {
            flex: 1,
            color: color.text._400,
            fontSize: fontSize.s,
        },
        pill: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: 4,
            paddingHorizontal: spacing.l,
            paddingVertical: 5,
            borderRadius: 999,
            backgroundColor: color.primary._500,
        },
        pillText: {
            color: color.primary._100,
            fontSize: fontSize.s,
            fontWeight: '600',
        },
    })
}
