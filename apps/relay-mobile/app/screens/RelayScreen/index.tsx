import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useFocusEffect, useRouter } from 'expo-router'
import React, { useCallback, useState } from 'react'
import { RefreshControl, ScrollView, StyleSheet, Text, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import SectionTitle from '@components/text/SectionTitle'
import HeaderTitle from '@components/views/HeaderTitle'
import { relay, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { activeHost, useRelayHostsStore } from '@lib/state/RelayHosts'
import { Theme } from '@lib/theme/ThemeManager'

import HoldItem from './HoldItem'
import HostItem from './HostItem'
import PairSheet from './PairSheet'
import RequestSheet from './RequestSheet'
import SessionItem from './SessionItem'

/**
 * The PC tab: the agent wall on the desktop, from a phone. Paired machines, the live sessions
 * on the one that is connected, and whatever is waiting on a decision.
 */
const RelayScreen = () => {
    const styles = useStyles()
    const { color, spacing } = Theme.useTheme()
    const router = useRouter()
    const hosts = useRelayHostsStore((state) => state.hosts)
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
    const [showPair, setShowPair] = useState(false)
    const [showRequest, setShowRequest] = useState(false)
    const [refreshing, setRefreshing] = useState(false)

    useFocusEffect(
        useCallback(() => {
            // Read the stores directly: this runs on focus, not on every status change.
            const current = useRelayStore.getState()
            const paired = useRelayHostsStore.getState().hosts.length > 0
            if (current.status === 'online') relay.refresh().catch(() => {})
            else if (current.status === 'offline' && paired && !current.stopped) {
                // Reach the paired PC on arrival, and again after a drop. A Disconnect the
                // person asked for is respected until they tap Connect.
                relay.connect(activeHost()).catch(() => {})
            }
        }, [])
    )

    const handleRefresh = async () => {
        setRefreshing(true)
        try {
            if (status === 'online') await relay.refresh()
            else if (hosts.length > 0) await relay.connect(activeHost())
        } catch (e) {
            Logger.warnToast(`${(e as Error).message}`)
        } finally {
            setRefreshing(false)
        }
    }

    const projectOf = (id: number) => projects.find((item) => item.id === id)
    const live = sessions.filter((item) => item.state !== 'closed')

    return (
        <SafeAreaView edges={['bottom']} style={{ flex: 1 }}>
            <HeaderTitle title="PC" />
            <PairSheet visible={showPair} setVisible={setShowPair} />
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
                <View style={styles.status}>
                    <View style={styles.statusRow}>
                        <View
                            style={[
                                styles.lamp,
                                {
                                    backgroundColor:
                                        status === 'online'
                                            ? '#2ec469'
                                            : status === 'connecting'
                                              ? color.quote
                                              : color.neutral._700,
                                },
                            ]}
                        />
                        <Text style={styles.statusText}>
                            {status === 'online'
                                ? `Connected to ${hostName}`
                                : status === 'connecting'
                                  ? `Connecting to ${hostName ?? 'PC'}…`
                                  : hosts.length === 0
                                    ? 'No PC paired'
                                    : 'Not connected'}
                        </Text>
                        {status === 'online' && (
                            <Text style={styles.transport}>
                                {transport === 'direct' ? 'WIFI · PRIVATE' : 'VIA YOUR SERVER'}
                            </Text>
                        )}
                    </View>
                    {!!error && status !== 'online' && <Text style={styles.error}>{error}</Text>}
                </View>

                {hosts.length === 0 && (
                    <View style={styles.empty}>
                        <AntDesign name="desktop" size={56} color={color.text._700} />
                        <Text style={styles.emptyTitle}>Work from your phone</Text>
                        <Text style={styles.emptyText}>
                            Pair this phone with the PC that runs Relay. On the same WiFi the link
                            is direct and never leaves your network; away from home it goes through
                            a server you host yourself.
                        </Text>
                        <Text style={styles.emptyText}>
                            On the PC: `relay remote serve --pair`, then scan the code.
                        </Text>
                    </View>
                )}

                {status === 'online' &&
                    (holds.length > 0 || notifications.length > 0 || inReview > 0) && (
                        <View style={styles.section}>
                            <SectionTitle>Needs you</SectionTitle>
                            {holds.map((hold) => (
                                <HoldItem key={hold.id} hold={hold} />
                            ))}
                            {inReview > 0 && (
                                <Text style={styles.note}>
                                    {inReview} task{inReview === 1 ? '' : 's'} waiting for review on
                                    the desktop.
                                </Text>
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
                    <View style={{ flexDirection: 'row', columnGap: spacing.m }}>
                        <ThemedButton
                            label="New request"
                            iconName="plus"
                            buttonStyle={{ flex: 1 }}
                            onPress={() => setShowRequest(true)}
                        />
                        <ThemedButton
                            label="Board"
                            iconName="profile"
                            variant="secondary"
                            buttonStyle={{ flex: 1 }}
                            onPress={() => router.push('/screens/RelayScreen/Board')}
                        />
                    </View>
                )}

                {status === 'online' && (
                    <View style={styles.section}>
                        <SectionTitle>Sessions</SectionTitle>
                        {live.length === 0 ? (
                            <Text style={styles.note}>
                                No live sessions. Launch agents from the desktop; they appear here
                                as they start.
                            </Text>
                        ) : (
                            <View style={{ rowGap: 2 }}>
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

                {hosts.length > 0 && (
                    <View style={styles.section}>
                        <SectionTitle>Paired PCs</SectionTitle>
                        <View style={{ rowGap: spacing.m }}>
                            {hosts.map((host) => (
                                <HostItem key={host.id} host={host} />
                            ))}
                        </View>
                    </View>
                )}

                <ThemedButton
                    label={hosts.length === 0 ? 'Pair a PC' : 'Pair another PC'}
                    iconName="qrcode"
                    variant={hosts.length === 0 ? 'primary' : 'secondary'}
                    onPress={() => setShowPair(true)}
                />
                <Text style={styles.footnote}>
                    Chats and on-device models stay on this phone. The PC link carries only what you
                    do on the PC tab.
                </Text>
            </ScrollView>
        </SafeAreaView>
    )
}

export default RelayScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        page: {
            padding: spacing.xl,
            rowGap: spacing.xl,
            paddingBottom: spacing.xl3,
        },
        status: {
            backgroundColor: color.neutral._200,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            rowGap: spacing.s,
        },
        statusRow: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        lamp: {
            width: 8,
            height: 8,
            borderRadius: 4,
        },
        statusText: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.m,
        },
        transport: {
            color: color.text._400,
            fontSize: fontSize.s - 1,
            letterSpacing: 1,
        },
        error: {
            color: color.error._300,
            fontSize: fontSize.s,
        },
        empty: {
            alignItems: 'center',
            rowGap: spacing.m,
            paddingVertical: spacing.xl2,
        },
        emptyTitle: {
            color: color.text._100,
            fontSize: fontSize.xl,
        },
        emptyText: {
            color: color.text._400,
            textAlign: 'center',
        },
        section: {
            rowGap: spacing.m,
        },
        note: {
            color: color.text._400,
        },
        notice: {
            backgroundColor: color.neutral._300,
            padding: spacing.l,
            rowGap: 2,
            borderBottomColor: color.quote,
            borderBottomWidth: 2,
        },
        noticeTitle: {
            color: color.text._100,
        },
        noticeBody: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        footnote: {
            color: color.text._500,
            fontSize: fontSize.s,
            textAlign: 'center',
        },
    })
}
