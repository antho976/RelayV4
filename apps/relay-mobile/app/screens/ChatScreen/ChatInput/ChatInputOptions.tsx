import AntDesign from '@react-native-vector-icons/ant-design/static'
import { useRouter } from 'expo-router'
import { useTranslation } from 'react-i18next'
import { StyleSheet } from 'react-native'

import ContextMenu from '@components/views/ContextMenu'
import Drawer from '@components/views/Drawer'
import { useLiveQueryJoined } from '@lib/hooks/LiveQueryJoined'
import { Chats } from '@lib/state/Chat'
import { Theme } from '@lib/theme/ThemeManager'

import { useAuthorNoteState } from '../AuthorNote'
import ChatTokenCount from './ChatTokenCount'

type ChatOptionsProps = {
    disabled: boolean
}

const ChatOptions: React.FC<ChatOptionsProps> = ({ disabled }) => {
    const { t } = useTranslation()
    const router = useRouter()
    const styles = useStyles()
    const { ref } = useAuthorNoteState()
    const setShow = Drawer.useDrawerStore((state) => state.setShow)

    const setShowChat = (b: boolean) => {
        setShow(Drawer.ID.CHATLIST, b)
    }

    const { chatId } = Chats.useChat()
    const { data: chatData } = useLiveQueryJoined(Chats.db.live.chat(chatId ?? -1), [chatId], {
        deepCheck: true,
    })
    const chatBackground = chatData?.background_image

    return (
        <ContextMenu
            disabled={disabled}
            buttons={[
                {
                    component: () => <ChatTokenCount />,
                    border: true,
                },
                {
                    onPress: (close) => {
                        close()
                        router.back()
                    },
                    label: t('chat.input.actions.mainMenu'),
                    icon: 'backward',
                },
                {
                    onPress: (close) => {
                        ref?.current?.open()
                        close()
                    },
                    label: t('chat.input.actions.authorNotes'),
                    icon: 'paper-clip',
                },
                {
                    onPress: (close) => {
                        close()
                        router.push('/screens/CharacterEditorScreen')
                    },
                    label: t('chat.input.actions.editCharacter'),
                    icon: 'edit',
                },
                {
                    onPress: (close) => {
                        setShowChat(true)
                        close()
                    },
                    label: t('chat.input.actions.chatHistory'),
                    icon: 'paper-clip',
                },
                {
                    onPress: (close) => {
                        close()
                        router.push('/screens/ChatPresetsScreen')
                    },
                    label: t('chat.input.actions.chatPresets'),
                    icon: 'profile',
                    disabled: !chatId,
                },
                {
                    label: t('chat.background.title'),
                    icon: 'picture',
                    disabled: !chatId,
                    submenu: [
                        {
                            label: t('chat.background.setAction'),
                            icon: 'picture',
                            onPress: async (close) => {
                                close()
                                if (chatId) await Chats.importBackground(chatId, chatBackground)
                            },
                        },
                        {
                            label: t('chat.background.removeAction'),
                            icon: 'delete',
                            variant: 'warning',
                            disabled: !chatBackground,
                            onPress: async (close) => {
                                close()
                                if (chatId && chatBackground)
                                    await Chats.removeBackground(chatId, chatBackground)
                            },
                        },
                    ],
                },
            ]}
            placement="top">
            <AntDesign
                name="caret-up"
                style={[styles.optionsButton, { opacity: disabled ? 0.5 : 1 }]}
                size={24}
            />
        </ContextMenu>
    )
}

export default ChatOptions

const useStyles = () => {
    const { color } = Theme.useTheme()

    return StyleSheet.create({
        optionsButton: {
            color: color.text._500,
            padding: 4,
            backgroundColor: color.neutral._200,
            borderRadius: 16,
        },
    })
}
