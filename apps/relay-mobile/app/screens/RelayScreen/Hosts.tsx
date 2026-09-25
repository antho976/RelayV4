import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, View } from 'react-native'
import { useMMKVBoolean } from 'react-native-mmkv'
import { SafeAreaView } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import ThemedSwitch from '@components/input/ThemedSwitch'
import { relayHref } from '@components/relay'
import HeaderTitle from '@components/views/HeaderTitle'
import { AppSettings } from '@lib/constants/GlobalValues'
import { useRelayHostsStore } from '@lib/state/RelayHosts'
import { Theme } from '@lib/theme/ThemeManager'

import HostItem from './HostItem'
import PairSheet from './PairSheet'

/**
 * Everything about the link itself, in one place: the PCs this phone paired with and how it
 * reaches them, pairing another, and the two things the PC tab is allowed to do to the phone.
 */
const HostsScreen = () => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const router = useRouter()
    const hosts = useRelayHostsStore((state) => state.hosts)
    const [keepOn, setKeepOn] = useMMKVBoolean(AppSettings.RelayKeepScreenOn)
    const [notify, setNotify] = useMMKVBoolean(AppSettings.RelayNotify)
    const [showPair, setShowPair] = useState(false)

    return (
        <SafeAreaView edges={['bottom']} style={{ flex: 1 }}>
            <HeaderTitle title="Paired PCs" />
            <PairSheet visible={showPair} setVisible={setShowPair} />
            <ScrollView contentContainerStyle={styles.page}>
                {hosts.length === 0 ? (
                    <Text style={styles.note}>
                        No PC paired yet. On the PC, run `relay remote pair` and scan the code.
                    </Text>
                ) : (
                    <View style={{ rowGap: spacing.m }}>
                        {hosts.map((host) => (
                            <HostItem key={host.id} host={host} />
                        ))}
                    </View>
                )}
                <ThemedButton
                    label={hosts.length === 0 ? 'Pair a PC' : 'Pair another PC'}
                    iconName="qrcode"
                    variant={hosts.length === 0 ? 'primary' : 'secondary'}
                    onPress={() => setShowPair(true)}
                />

                <ThemedButton
                    label="PC settings"
                    iconName="setting"
                    variant="secondary"
                    onPress={() => router.push(relayHref('PcSettings'))}
                />

                <View style={styles.section}>
                    <Text style={styles.heading}>On this phone</Text>
                    <ThemedSwitch
                        label="Tell me when an agent needs me"
                        value={notify}
                        onChangeValue={setNotify}
                        description="While connected and the app is in the background, a notification is shown when an agent is held by a guardrail, gets blocked, or finishes. It comes straight from the PC link; no push service is involved."
                    />
                    <ThemedSwitch
                        label="Keep the screen on in a terminal"
                        value={keepOn}
                        onChangeValue={setKeepOn}
                        description="While a session's terminal is open, the phone does not sleep."
                    />
                </View>

                <Text style={styles.footnote}>
                    Chats and on-device models stay on this phone. The PC link carries only what you
                    do on the PC tab, and every action is logged on the desktop as yours.
                </Text>
            </ScrollView>
        </SafeAreaView>
    )
}

export default HostsScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        page: {
            padding: spacing.xl,
            rowGap: spacing.xl2,
            paddingBottom: spacing.xl3,
        },
        section: {
            rowGap: spacing.s,
        },
        heading: {
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
        },
        note: {
            color: color.text._400,
            lineHeight: 20,
        },
        footnote: {
            color: color.text._500,
            fontSize: fontSize.s,
            textAlign: 'center',
            lineHeight: 18,
        },
    })
}
