import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { useFocusEffect, useRouter } from 'expo-router'
import React, { useCallback, useEffect, useState } from 'react'
import { RefreshControl, ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import HeaderButton from '@components/views/HeaderButton'
import HeaderTitle from '@components/views/HeaderTitle'
import { relay, RelayNotification, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

import { ago } from './console'
import HoldItem from './HoldItem'

const ICONS: Record<string, AntDesignIconName> = {
    agent_done: 'check-circle',
    agent_blocked: 'exclamation-circle',
    guardrail: 'safety',
}

/**
 * Everything the PC wants a person to look at, behind the bell on the PC tab: guardrail holds
 * first (an agent is stopped until someone answers), then tasks waiting for review, then the
 * notification feed, newest first. Tapping a notification marks it read on the PC as well.
 */
const InboxScreen = () => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const status = useRelayStore((state) => state.status)
    const holds = useRelayStore((state) => state.holds)
    const inReview = useRelayStore((state) => state.inReview)
    const [items, setItems] = useState<RelayNotification[]>([])
    const [loading, setLoading] = useState(false)

    const load = useCallback(async () => {
        if (status !== 'online') return
        setLoading(true)
        try {
            const result = await relay.request<{ notifications: RelayNotification[] }>(
                'notify.list',
                { limit: 50 }
            )
            setItems(result.notifications)
            await relay.refreshAttention()
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
        } finally {
            setLoading(false)
        }
    }, [status])

    useFocusEffect(
        useCallback(() => {
            load()
        }, [load])
    )

    useEffect(
        () =>
            relay.onEvents((event) => {
                if (event.ev.startsWith('notify.')) load()
            }),
        [load]
    )

    const unread = items.filter((item) => !item.read).length

    const ack = async (item: RelayNotification) => {
        if (item.read) return
        // Read at once on the phone; the PC catches up.
        setItems((list) => list.map((n) => (n.id === item.id ? { ...n, read: true } : n)))
        try {
            await relay.request('notify.ack', { notification_id: item.id })
            await relay.refreshAttention()
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
            load()
        }
    }

    const ackAll = async () => {
        setItems((list) => list.map((n) => ({ ...n, read: true })))
        try {
            await relay.request('notify.ack_all', {})
            await relay.refreshAttention()
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
            load()
        }
    }

    const empty = holds.length === 0 && inReview === 0 && items.length === 0

    return (
        <SafeAreaView edges={['bottom']} style={{ flex: 1 }}>
            <HeaderTitle title="Inbox" />
            <HeaderButton
                headerRight={() =>
                    unread > 0 ? (
                        <TouchableOpacity hitSlop={12} onPress={ackAll}>
                            <Text style={styles.headerAction}>Mark all read</Text>
                        </TouchableOpacity>
                    ) : null
                }
            />
            <ScrollView
                contentContainerStyle={[styles.page, empty && { flexGrow: 1 }]}
                refreshControl={
                    <RefreshControl
                        refreshing={loading}
                        onRefresh={load}
                        tintColor={color.text._300}
                        colors={[color.text._300]}
                    />
                }>
                {status !== 'online' && <Text style={styles.note}>Not connected to the PC.</Text>}

                {holds.length > 0 && (
                    <View style={styles.section}>
                        <Text style={styles.heading}>Waiting for permission · {holds.length}</Text>
                        {holds.map((hold) => (
                            <HoldItem key={hold.id} hold={hold} />
                        ))}
                    </View>
                )}

                {inReview > 0 && (
                    <TouchableOpacity
                        style={styles.review}
                        onPress={() => router.push('/screens/RelayScreen/Board')}>
                        <AntDesign name="check-square" size={20} color={color.primary._700} />
                        <View style={{ flex: 1 }}>
                            <Text style={styles.title}>
                                {inReview} task{inReview === 1 ? '' : 's'} in review
                            </Text>
                            <Text style={styles.body}>Open the board to look and approve.</Text>
                        </View>
                        <AntDesign name="right" size={14} color={color.text._500} />
                    </TouchableOpacity>
                )}

                {items.length > 0 && (
                    <View style={styles.section}>
                        <Text style={styles.heading}>
                            Notifications{unread > 0 ? ` · ${unread} new` : ''}
                        </Text>
                        <View style={styles.list}>
                            {items.map((item, index) => (
                                <TouchableOpacity
                                    key={item.id}
                                    activeOpacity={item.read ? 1 : 0.6}
                                    style={[
                                        styles.item,
                                        index > 0 && styles.itemDivider,
                                        item.read && { opacity: 0.55 },
                                    ]}
                                    onPress={() => ack(item)}>
                                    <AntDesign
                                        name={ICONS[item.category] ?? 'bell'}
                                        size={18}
                                        color={item.read ? color.text._500 : color.text._200}
                                    />
                                    <View style={{ flex: 1, rowGap: 2 }}>
                                        <View style={styles.itemHead}>
                                            <Text numberOfLines={1} style={styles.title}>
                                                {item.title}
                                            </Text>
                                            <Text style={styles.when}>{ago(item.created_at)}</Text>
                                        </View>
                                        {!!item.body && (
                                            <Text numberOfLines={4} style={styles.body}>
                                                {item.body}
                                            </Text>
                                        )}
                                    </View>
                                    {!item.read && (
                                        <View
                                            style={[
                                                styles.dot,
                                                { backgroundColor: color.primary._500 },
                                            ]}
                                        />
                                    )}
                                </TouchableOpacity>
                            ))}
                        </View>
                    </View>
                )}

                {status === 'online' && empty && !loading && (
                    <View style={styles.empty}>
                        <AntDesign name="inbox" size={40} color={color.text._600} />
                        <Text style={styles.emptyTitle}>All clear</Text>
                        <Text style={styles.note}>
                            Nothing needs you right now. Holds, finished agents and review requests
                            land here.
                        </Text>
                    </View>
                )}
            </ScrollView>
        </SafeAreaView>
    )
}

export default InboxScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        page: {
            padding: spacing.xl,
            rowGap: spacing.xl2,
            paddingBottom: spacing.xl3,
        },
        headerAction: {
            color: color.primary._700,
            fontSize: fontSize.m,
        },
        section: {
            rowGap: spacing.m,
        },
        heading: {
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
        },
        review: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            padding: spacing.l,
            borderRadius: 16,
            backgroundColor: color.neutral._200,
        },
        list: {
            borderRadius: 16,
            backgroundColor: color.neutral._200,
            overflow: 'hidden',
        },
        item: {
            flexDirection: 'row',
            alignItems: 'flex-start',
            columnGap: spacing.l,
            padding: spacing.l,
        },
        itemDivider: {
            borderTopWidth: 1,
            borderTopColor: color.neutral._300,
        },
        itemHead: {
            flexDirection: 'row',
            alignItems: 'baseline',
            columnGap: spacing.m,
        },
        title: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.m,
            fontWeight: '600',
        },
        body: {
            color: color.text._400,
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        when: {
            color: color.text._500,
            fontSize: fontSize.s,
            fontVariant: ['tabular-nums'],
        },
        dot: {
            width: 8,
            height: 8,
            borderRadius: 4,
            marginTop: 6,
        },
        note: {
            color: color.text._400,
            lineHeight: 20,
            textAlign: 'center',
        },
        empty: {
            flex: 1,
            alignItems: 'center',
            justifyContent: 'center',
            rowGap: spacing.m,
            paddingHorizontal: spacing.xl2,
        },
        emptyTitle: {
            color: color.text._100,
            fontSize: fontSize.l,
        },
    })
}
