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
import { activeHost, useRelayHostsStore } from '@lib/state/RelayHosts'
import { Theme } from '@lib/theme/ThemeManager'
import { linkColor } from '@screens/RelayScreen/console'

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

type NavItem = {
    label: string
    icon: AntDesignIconName
    onPress: () => void
    active?: boolean
    count?: number
    disabled?: boolean
}

/**
 * The app's drawer, Relay first. At the top the PC, which is the home screen, with how it is
 * reached and what waits there: its workspaces, the inbox, the paired PCs. Below it what runs
 * on the phone itself: characters, models, and the recent conversations with them. At the
 * bottom, the profile bubble that opens Settings and a button to start a new chat.
 */
const SettingsDrawer = () => {
    const styles = useStyles()
    const { t } = useTranslation()
    const { color } = Theme.useTheme()
    const router = useRouter()
    const path = usePathname()
    const { appMode } = useAppMode()
    const relayStatus = useRelayStore((state) => state.status)
    const hostName = useRelayStore((state) => state.hostName)
    const waiting = useRelayStore(
        (state) => state.holds.length + state.notifications.length + state.inReview
    )
    const paired = useRelayHostsStore((state) => state.hosts.length > 0)
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
        if (path !== target) router.push(target)
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
            go('/screens/CharacterListScreen')
            return
        }
        const id = await Chats.db.mutate.createChat(charId)
        if (id) await openChat(charId, id)
    }

    const online = relayStatus === 'online'
    const pcName = (paired && (hostName ?? activeHost()?.name)) || 'PC'
    const pcMeta = !paired
        ? 'Not paired yet'
        : online
          ? 'Connected'
          : relayStatus === 'connecting'
            ? 'Connecting…'
            : 'Not connected'
    const link = (label: string, icon: AntDesignIconName, target: string): NavItem => ({
        label: label,
        icon: icon,
        onPress: () => go(target),
        active: path.startsWith(target),
    })

    // What reaches the PC: the workspaces sidebar sits on the home screen, so it opens there.
    const relayNav: NavItem[] = paired
        ? [
              {
                  label: 'Workspaces',
                  icon: 'folder',
                  onPress: () => {
                      go('/')
                      setShow(Drawer.ID.RELAY, true)
                  },
                  disabled: !online,
              },
              {
                  ...link('Inbox', 'bell', '/screens/RelayScreen/Inbox'),
                  count: online ? waiting : 0,
                  disabled: !online,
              },
              link('Paired PCs', 'link', '/screens/RelayScreen/Hosts'),
          ]
        : [link('Pair a PC', 'qrcode', '/screens/RelayScreen/Hosts')]
    const localNav: NavItem[] = [
        link('Characters', 'team', '/screens/CharacterListScreen'),
        appMode === 'remote'
            ? link(t('navigation.api'), 'api', '/screens/ConnectionsManagerScreen')
            : link(t('navigation.models'), 'branches', '/screens/ModelManagerScreen'),
    ]

    const renderNav = (items: NavItem[]) =>
        items.map((entry) => (
            <TouchableOpacity
                key={entry.label}
                style={[styles.navRow, entry.active && styles.navRowActive]}
                disabled={entry.disabled}
                onPress={entry.onPress}>
                <AntDesign
                    name={entry.icon}
                    size={20}
                    color={entry.disabled ? color.text._600 : color.text._200}
                />
                <Text style={[styles.navLabel, entry.disabled && styles.navLabelOff]}>
                    {entry.label}
                </Text>
                {!!entry.count && (
                    <Text style={styles.count}>{entry.count > 99 ? '99+' : entry.count}</Text>
                )}
            </TouchableOpacity>
        ))

    const initial = userName.trim().charAt(0).toUpperCase() || 'Y'

    return (
        <Drawer.Body drawerID={Drawer.ID.SETTINGS} drawerStyle={styles.drawer}>
            <View style={styles.head}>
                <Text style={styles.wordmark}>Relay</Text>
            </View>

            <FlatList
                style={{ flex: 1 }}
                data={recents}
                keyExtractor={(item) => String(item.id)}
                ListHeaderComponent={
                    <View>
                        <View style={styles.nav}>
                            <TouchableOpacity
                                style={[styles.pcRow, path === '/' && styles.navRowActive]}
                                onPress={() => go('/')}>
                                <View style={styles.pcIcon}>
                                    <AntDesign name="desktop" size={20} color={color.text._200} />
                                    <View
                                        style={[
                                            styles.pcLamp,
                                            {
                                                backgroundColor: linkColor(relayStatus, color),
                                                borderColor: color.neutral._300,
                                            },
                                        ]}
                                    />
                                </View>
                                <View style={{ flex: 1 }}>
                                    <Text numberOfLines={1} style={styles.pcName}>
                                        {pcName}
                                    </Text>
                                    <Text numberOfLines={1} style={styles.pcMeta}>
                                        {pcMeta}
                                    </Text>
                                </View>
                            </TouchableOpacity>
                            {renderNav(relayNav)}
                        </View>
                        <View style={styles.divider} />
                        <Text style={styles.sectionTitle}>Local</Text>
                        <View style={styles.nav}>{renderNav(localNav)}</View>
                        <Text style={[styles.sectionTitle, styles.recentsTitle]}>Recent chats</Text>
                    </View>
                }
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
            fontSize: fontSize.l,
        },
        navLabelOff: {
            color: color.text._600,
        },
        count: {
            minWidth: 22,
            textAlign: 'center',
            color: color.text._100,
            fontSize: fontSize.s,
            fontVariant: ['tabular-nums'],
            paddingHorizontal: 6,
            paddingVertical: 1,
            borderRadius: 10,
            overflow: 'hidden',
            backgroundColor: color.primary._500,
        },
        pcRow: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.m,
            borderRadius: 20,
        },
        pcIcon: {
            width: 40,
            height: 40,
            borderRadius: 12,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.neutral._300,
        },
        pcLamp: {
            position: 'absolute',
            right: -2,
            bottom: -2,
            width: 12,
            height: 12,
            borderRadius: 6,
            borderWidth: 2,
        },
        pcName: {
            color: color.text._100,
            fontSize: fontSize.xl,
            fontWeight: '600',
        },
        pcMeta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        divider: {
            height: 1,
            backgroundColor: color.neutral._300,
            marginHorizontal: spacing.xl2,
            marginVertical: spacing.l,
        },
        sectionTitle: {
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
            paddingHorizontal: spacing.xl2,
            paddingTop: spacing.s,
            paddingBottom: spacing.s,
        },
        recentsTitle: {
            paddingTop: spacing.xl,
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
