import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import { and, desc, eq } from 'drizzle-orm'
import { usePathname, useRouter } from 'expo-router'
import { useTranslation } from 'react-i18next'
import { FlatList, StyleSheet, Text, TouchableOpacity, View } from 'react-native'
import { useShallow } from 'zustand/react/shallow'

import Drawer from '@components/views/Drawer'
import { db } from '@db/db'
import { characters, chats } from '@db/schema'
import { useRelayStore } from '@lib/engine/Relay/RelayClient'
import { useLiveQueryJoined } from '@lib/hooks/LiveQueryJoined'
import { useAppMode } from '@lib/state/AppMode'
import { Characters } from '@lib/state/Characters'
import { Chats } from '@lib/state/Chat'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

/** Recent conversations across every character, newest first: the sidebar's history. */
const recentChatsQuery = () =>
    db
        .select({
            id: chats.id,
            name: chats.name,
            characterId: chats.character_id,
            characterName: characters.name,
        })
        .from(chats)
        .innerJoin(characters, eq(chats.character_id, characters.id))
        .where(
            and(eq(chats.hidden, false), eq(chats.ghost, false), eq(characters.type, 'character'))
        )
        .orderBy(desc(chats.last_modified))
        .limit(40)

type NavItem = { label: string; icon: AntDesignIconName; path: string; lamp?: string }

/**
 * The app's sidebar: where you go (the PC, your characters, models) above your recent
 * conversations with local agents, and at the bottom the profile bubble that opens Settings
 * and a button to start a new chat.
 */
const SettingsDrawer = () => {
    const styles = useStyles()
    const { t } = useTranslation()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const path = usePathname()
    const { appMode } = useAppMode()
    const relayStatus = useRelayStore((state) => state.status)
    const setShow = Drawer.useDrawerStore((state) => state.setShow)
    const { charId, setCard } = Characters.useCharacterStore(
        useShallow((state) => ({ charId: state.id, setCard: state.setCard }))
    )
    const userName = Characters.useUserStore((state) => state.card?.name ?? 'You')
    const { setId, chatId } = Chats.useChat()
    const { data: recents } = useLiveQueryJoined(recentChatsQuery(), [])

    const close = () => setShow(Drawer.ID.SETTINGS, false)
    const go = (target: string) => {
        close()
        if (target === '/') {
            if (router.canDismiss()) router.dismissAll()
            return
        }
        router.push(target)
    }

    const openChat = async (characterId: number, id: number) => {
        try {
            if (charId !== characterId) await setCard(characterId)
            await setId(id)
            close()
            if (path !== '/screens/ChatScreen') router.push('/screens/ChatScreen')
        } catch (e) {
            Logger.errorToast(`Could not open that chat: ${e}`)
        }
    }

    const newChat = async () => {
        // A new chat is with the character on screen; with none chosen yet, pick one first.
        if (!charId) {
            go('/')
            return
        }
        const id = await Chats.db.mutate.createChat(charId)
        if (id) await openChat(charId, id)
    }

    const pcLamp =
        relayStatus === 'online'
            ? '#2ec469'
            : relayStatus === 'connecting'
              ? color.quote
              : undefined
    const nav: NavItem[] = [
        { label: 'PC', icon: 'desktop', path: '/screens/RelayScreen', lamp: pcLamp },
        { label: 'Characters', icon: 'team', path: '/' },
        appMode === 'remote'
            ? { label: t('navigation.api'), icon: 'api', path: '/screens/ConnectionsManagerScreen' }
            : {
                  label: t('navigation.models'),
                  icon: 'branches',
                  path: '/screens/ModelManagerScreen',
              },
    ]

    const initial = userName.trim().charAt(0).toUpperCase() || 'Y'

    return (
        <Drawer.Body drawerID={Drawer.ID.SETTINGS} drawerStyle={styles.drawer}>
            <View style={styles.head}>
                <Text style={styles.wordmark}>Relay</Text>
            </View>

            <View style={styles.nav}>
                {nav.map((item) => {
                    const active = item.path === '/' ? path === '/' : path.startsWith(item.path)
                    return (
                        <TouchableOpacity
                            key={item.label}
                            style={[styles.navRow, active && styles.navRowActive]}
                            onPress={() => go(item.path)}>
                            <AntDesign name={item.icon} size={22} color={color.text._200} />
                            <Text style={styles.navLabel}>{item.label}</Text>
                            {!!item.lamp && (
                                <View style={[styles.lamp, { backgroundColor: item.lamp }]} />
                            )}
                        </TouchableOpacity>
                    )
                })}
            </View>

            <View style={styles.divider} />

            <FlatList
                style={{ flex: 1 }}
                data={recents}
                keyExtractor={(item) => String(item.id)}
                ListHeaderComponent={<Text style={styles.sectionTitle}>Recents</Text>}
                ListEmptyComponent={
                    <Text style={styles.empty}>
                        Conversations with your characters show up here.
                    </Text>
                }
                renderItem={({ item }) => {
                    const titled = item.name && item.name !== 'New Chat'
                    return (
                        <TouchableOpacity
                            style={[styles.chatRow, chatId === item.id && styles.navRowActive]}
                            onPress={() => openChat(item.characterId, item.id)}>
                            <AntDesign name="message" size={18} color={color.text._400} />
                            <View style={{ flex: 1 }}>
                                <Text numberOfLines={1} style={styles.chatTitle}>
                                    {titled ? item.name : item.characterName}
                                </Text>
                                {titled && (
                                    <Text numberOfLines={1} style={styles.chatMeta}>
                                        {item.characterName}
                                    </Text>
                                )}
                            </View>
                        </TouchableOpacity>
                    )
                }}
            />

            <View style={styles.footer}>
                <TouchableOpacity
                    style={styles.profile}
                    onPress={() => go('/screens/AppSettingsScreen')}>
                    <Text style={styles.profileInitial}>{initial}</Text>
                </TouchableOpacity>
                <TouchableOpacity style={styles.newChat} onPress={newChat}>
                    <AntDesign name="plus" size={16} color={color.primary._100} />
                    <Text style={styles.newChatText}>New chat</Text>
                </TouchableOpacity>
            </View>
        </Drawer.Body>
    )
}

export default SettingsDrawer

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        drawer: {
            width: '86%',
            backgroundColor: color.neutral._100,
            paddingTop: spacing.xl,
            borderTopWidth: 0,
            borderTopRightRadius: 20,
            borderBottomRightRadius: 20,
        },
        head: {
            paddingHorizontal: spacing.xl2,
            paddingBottom: spacing.xl,
        },
        wordmark: {
            color: color.text._100,
            fontFamily: 'serif',
            fontSize: 34,
        },
        nav: {
            paddingHorizontal: spacing.m,
            rowGap: 2,
        },
        navRow: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.xl,
            paddingHorizontal: spacing.xl,
            paddingVertical: spacing.l + 2,
            borderRadius: 999,
        },
        navRowActive: {
            backgroundColor: color.neutral._300,
        },
        navLabel: {
            flex: 1,
            color: color.text._200,
            fontSize: fontSize.xl,
        },
        lamp: {
            width: 8,
            height: 8,
            borderRadius: 4,
        },
        divider: {
            height: 1,
            backgroundColor: color.neutral._300,
            marginHorizontal: spacing.xl2,
            marginVertical: spacing.l,
        },
        sectionTitle: {
            color: color.text._400,
            fontSize: fontSize.l,
            paddingHorizontal: spacing.xl2,
            paddingTop: spacing.s,
            paddingBottom: spacing.m,
        },
        empty: {
            color: color.text._500,
            fontSize: fontSize.m,
            paddingHorizontal: spacing.xl2,
        },
        chatRow: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.xl,
            marginHorizontal: spacing.m,
            paddingHorizontal: spacing.xl,
            paddingVertical: spacing.l,
            borderRadius: 999,
        },
        chatTitle: {
            color: color.text._100,
            fontSize: fontSize.l,
        },
        chatMeta: {
            color: color.text._500,
            fontSize: fontSize.s,
        },
        footer: {
            flexDirection: 'row',
            alignItems: 'center',
            justifyContent: 'space-between',
            paddingHorizontal: spacing.xl,
            paddingTop: spacing.l,
            paddingBottom: spacing.xl,
        },
        profile: {
            width: 52,
            height: 52,
            borderRadius: 26,
            overflow: 'hidden',
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: '#8b7fd6',
        },
        profileInitial: {
            color: '#ffffff',
            fontSize: fontSize.xl2,
            fontWeight: '600',
        },
        newChat: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingHorizontal: spacing.xl2,
            paddingVertical: spacing.l + 2,
            borderRadius: 999,
            backgroundColor: color.primary._500,
        },
        newChatText: {
            color: color.primary._100,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
    })
}
