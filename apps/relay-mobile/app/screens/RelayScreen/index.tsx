import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useFocusEffect, useRouter } from 'expo-router'
import React, { useCallback, useState } from 'react'
import { RefreshControl, ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import HeaderButton from '@components/views/HeaderButton'
import HeaderTitle from '@components/views/HeaderTitle'
import { relay, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { activeHost, useRelayHostsStore } from '@lib/state/RelayHosts'
import { Theme } from '@lib/theme/ThemeManager'

import { linkColor } from './console'
import HoldItem from './HoldItem'
import RequestSheet from './RequestSheet'
import SessionItem from './SessionItem'

/**
 * The PC tab: the agent wall on the desktop, from a phone. One line says whether the PC is
 * there; what needs a decision comes first; then the live sessions; and at the bottom, the
 * one thing a person came here to do — hand an agent some work.
 *
 * Pairing, routes and the PC's own settings live one tap away, behind the gear.
 */
const RelayScreen = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const paired = useRelayHostsStore((state) => state.hosts.length > 0)
    const {
        status,
        transport,
        hostName,
        error,
        sessions,
        projects,
        holds,
        notifications,
        inReview,
    } = useRelayStore()
    const [showRequest, setShowRequest] = useState(false)
    const [refreshing, setRefreshing] = useState(false)

    useFocusEffect(
        useCallback(() => {
            // Read the stores directly: this runs on focus, not on every status change.
            const current = useRelayStore.getState()
            const known = useRelayHostsStore.getState().hosts.length > 0
            if (current.status === 'online') relay.refresh().catch(() => {})
            else if (current.status === 'offline' && known && !current.stopped) {
                // Reach the paired PC on arrival, and again after a drop. A Disconnect the
                // person asked for is respected until they tap Connect.
                relay.connect(activeHost()).catch(() => {})
            }
        }, [])
    )

    const connect = async () => {
        try {
            if (status === 'online') await relay.refresh()
            else if (paired) await relay.connect(activeHost())
        } catch (e) {
            Logger.warnToast(`${(e as Error).message}`)
        }
    }

    const handleRefresh = async () => {
        setRefreshing(true)
        await connect()
        setRefreshing(false)
    }

    const openHosts = () => router.push('/screens/RelayScreen/Hosts')
    const projectOf = (id: number) => projects.find((item) => item.id === id)
    const live = sessions.filter((item) => item.state !== 'closed')
    const attention = holds.length > 0 || notifications.length > 0 || inReview > 0

    if (!paired) {
        return (
            <SafeAreaView edges={['bottom']} style={styles.fill}>
                <HeaderTitle title="PC" />
                <View style={styles.empty}>
                    <AntDesign name="desktop" size={48} color={color.text._600} />
                    <Text style={styles.emptyTitle}>Your desktop, from here</Text>
                    <Text style={styles.emptyText}>
                        Pair this phone with the PC that runs Relay. On the same WiFi the link is
                        direct and never leaves your network; away from home it goes through a
                        server you host.
                    </Text>
                    <ThemedButton label="Pair a PC" iconName="qrcode" onPress={openHosts} />
                </View>
            </SafeAreaView>
        )
    }

    return (
        <SafeAreaView edges={['bottom']} style={styles.fill}>
            <HeaderTitle title="PC" />
            <HeaderButton
                headerRight={() => (
                    <TouchableOpacity onPress={openHosts} hitSlop={12}>
                        <AntDesign name="setting" size={22} color={color.text._300} />
                    </TouchableOpacity>
                )}
            />
            <RequestSheet visible={showRequest} setVisible={setShowRequest} />
            <ScrollView
                contentContainerStyle={styles.page}
                refreshControl={
                    <RefreshControl
                        refreshing={refreshing}
                        onRefresh={handleRefresh}
                        tintColor={color.text._300}
                        colors={[color.text._300]}
                    />
                }>
                <TouchableOpacity style={styles.status} onPress={openHosts}>
                    <View style={[styles.lamp, { backgroundColor: linkColor(status, color) }]} />
                    <View style={{ flex: 1 }}>
                        <Text style={styles.statusText}>
                            {status === 'online'
                                ? hostName
                                : status === 'connecting'
                                  ? `Connecting to ${hostName ?? 'PC'}…`
                                  : 'Not connected'}
                        </Text>
                        {status === 'online' ? (
                            <Text style={styles.statusMeta}>
                                {transport === 'direct' ? 'WiFi · private' : 'Via your server'}
                            </Text>
                        ) : (
                            !!error && <Text style={styles.error}>{error}</Text>
                        )}
                    </View>
                    {status === 'offline' && (
                        <ThemedButton label="Connect" variant="secondary" onPress={connect} />
                    )}
                </TouchableOpacity>

                {status === 'online' && attention && (
                    <View style={styles.section}>
                        <Text style={styles.heading}>Needs you</Text>
                        {holds.map((hold) => (
                            <HoldItem key={hold.id} hold={hold} />
                        ))}
                        {inReview > 0 && (
                            <TouchableOpacity
                                style={styles.notice}
                                onPress={() => router.push('/screens/RelayScreen/Board')}>
                                <Text style={styles.noticeTitle}>
                                    {inReview} task{inReview === 1 ? '' : 's'} in review
                                </Text>
                                <Text style={styles.noticeBody}>Open the board to approve.</Text>
                            </TouchableOpacity>
                        )}
                        {notifications.slice(0, 8).map((item) => (
                            <View key={item.id} style={styles.notice}>
                                <Text style={styles.noticeTitle}>{item.title}</Text>
                                {!!item.body && (
                                    <Text numberOfLines={3} style={styles.noticeBody}>
                                        {item.body}
                                    </Text>
                                )}
                            </View>
                        ))}
                        {notifications.length > 0 && (
                            <ThemedButton
                                label="Mark all read"
                                variant="tertiary"
                                buttonStyle={{ alignSelf: 'flex-end' }}
                                onPress={() =>
                                    relay
                                        .request('notify.ack_all', {})
                                        .then(() => relay.refreshAttention())
                                        .catch((e) => Logger.errorToast(`${e.message}`))
                                }
                            />
                        )}
                    </View>
                )}

                {status === 'online' && (
                    <View style={styles.section}>
                        <View style={styles.headingRow}>
                            <Text style={styles.heading}>Sessions</Text>
                            <TouchableOpacity
                                hitSlop={8}
                                onPress={() => router.push('/screens/RelayScreen/Board')}>
                                <Text style={styles.link}>Board</Text>
                            </TouchableOpacity>
                        </View>
                        {live.length === 0 ? (
                            <Text style={styles.note}>
                                Nothing running. Ask for something below, or launch agents from the
                                desktop; they appear here as they start.
                            </Text>
                        ) : (
                            <View style={{ rowGap: 4 }}>
                                {live.map((session) => (
                                    <SessionItem
                                        key={session.name}
                                        session={session}
                                        project={projectOf(session.project_id)}
                                    />
                                ))}
                            </View>
                        )}
                    </View>
                )}
            </ScrollView>

            {status === 'online' && (
                <TouchableOpacity style={styles.composer} onPress={() => setShowRequest(true)}>
                    <Text style={styles.composerText}>Ask an agent on the PC…</Text>
                    <View style={styles.composerSend}>
                        <AntDesign name="arrow-up" size={16} color={color.text._900} />
                    </View>
                </TouchableOpacity>
            )}
        </SafeAreaView>
    )
}

export default RelayScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        fill: {
            flex: 1,
        },
        page: {
            padding: spacing.xl,
            rowGap: spacing.xl2,
            paddingBottom: spacing.xl3,
        },
        empty: {
            flex: 1,
            alignItems: 'center',
            justifyContent: 'center',
            rowGap: spacing.l,
            paddingHorizontal: spacing.xl2,
            paddingBottom: spacing.xl3,
        },
        emptyTitle: {
            color: color.text._100,
            fontSize: fontSize.xl,
        },
        emptyText: {
            color: color.text._400,
            textAlign: 'center',
            lineHeight: 20,
            marginBottom: spacing.m,
        },
        status: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
        },
        lamp: {
            width: 8,
            height: 8,
            borderRadius: 4,
        },
        statusText: {
            color: color.text._100,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
        statusMeta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        error: {
            color: color.error._300,
            fontSize: fontSize.s,
        },
        section: {
            rowGap: spacing.m,
        },
        headingRow: {
            flexDirection: 'row',
            alignItems: 'baseline',
            justifyContent: 'space-between',
        },
        heading: {
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
        },
        link: {
            color: color.primary._700,
            fontSize: fontSize.s,
        },
        note: {
            color: color.text._400,
            lineHeight: 20,
        },
        notice: {
            backgroundColor: color.neutral._200,
            borderRadius: 12,
            padding: spacing.l,
            rowGap: 2,
        },
        noticeTitle: {
            color: color.text._100,
        },
        noticeBody: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        composer: {
            flexDirection: 'row',
            alignItems: 'center',
            marginHorizontal: spacing.xl,
            marginBottom: spacing.l,
            paddingLeft: spacing.xl,
            paddingRight: spacing.s,
            paddingVertical: spacing.s,
            borderRadius: 24,
            backgroundColor: color.neutral._200,
            borderColor: color.neutral._400,
            borderWidth: 1,
        },
        composerText: {
            flex: 1,
            color: color.text._500,
            fontSize: fontSize.m,
        },
        composerSend: {
            width: 32,
            height: 32,
            borderRadius: 16,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.primary._500,
        },
    })
}
