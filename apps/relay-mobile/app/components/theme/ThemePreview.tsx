import AntDesign from '@react-native-vector-icons/ant-design/static'
import React from 'react'
import { useTranslation } from 'react-i18next'
import { StyleSheet, Text, View } from 'react-native'

import { ThemeColor } from '@lib/theme/ThemeColor'

type ThemePreviewProps = {
    theme: ThemeColor
}

/**
 * The app in miniature, painted entirely in `theme`: a chat header, the person's bubble, a
 * reply with a quoted line, an accent button and the input pill. It ignores the page's own
 * colours, so it can show a theme before it is the one in use.
 */
const ThemePreview: React.FC<ThemePreviewProps> = ({ theme }) => {
    const { t } = useTranslation()
    const character = t('settings.colors.preview.character')
    return (
        <View
            style={[
                styles.frame,
                { backgroundColor: theme.neutral._100, borderColor: theme.neutral._300 },
            ]}
            accessibilityLabel={theme.name}>
            <View style={styles.header}>
                <AntDesign name="menu" size={18} color={theme.text._300} />
                <View style={[styles.avatar, { backgroundColor: theme.neutral._400 }]}>
                    <Text style={[styles.avatarText, { color: theme.text._100 }]}>
                        {character.charAt(0)}
                    </Text>
                </View>
                <Text numberOfLines={1} style={[styles.title, { color: theme.text._100 }]}>
                    {character}
                </Text>
                <AntDesign name="ellipsis" size={18} color={theme.text._400} />
            </View>

            <View style={styles.body}>
                <View style={[styles.userBubble, { backgroundColor: theme.neutral._300 }]}>
                    <Text style={[styles.message, { color: theme.text._100 }]}>
                        {t('settings.colors.preview.user')}
                    </Text>
                </View>

                <View style={styles.reply}>
                    <Text style={[styles.message, { color: theme.text._200 }]}>
                        {t('settings.colors.preview.reply')}{' '}
                        <Text style={{ color: theme.quote }}>
                            {t('settings.colors.preview.quote')}
                        </Text>
                    </Text>
                    <View style={styles.replyMeta}>
                        <View style={[styles.line, { backgroundColor: theme.neutral._300 }]} />
                        <View style={[styles.accent, { backgroundColor: theme.primary._500 }]}>
                            <AntDesign name="check" size={12} color={theme.primary._100} />
                            <Text style={[styles.accentText, { color: theme.primary._100 }]}>
                                {t('settings.colors.preview.action')}
                            </Text>
                        </View>
                    </View>
                </View>
            </View>

            <View
                style={[
                    styles.input,
                    { backgroundColor: theme.neutral._200, borderColor: theme.neutral._300 },
                ]}>
                <AntDesign name="plus" size={16} color={theme.text._400} />
                <Text numberOfLines={1} style={[styles.placeholder, { color: theme.text._500 }]}>
                    {t('settings.colors.preview.input')}
                </Text>
                <View style={[styles.send, { backgroundColor: theme.primary._500 }]}>
                    <AntDesign name="arrow-up" size={16} color={theme.neutral._100} />
                </View>
            </View>
        </View>
    )
}

export default ThemePreview

const styles = StyleSheet.create({
    frame: {
        borderRadius: 26,
        borderWidth: 1,
        padding: 16,
        rowGap: 16,
        overflow: 'hidden',
    },
    header: {
        flexDirection: 'row',
        alignItems: 'center',
        columnGap: 10,
    },
    avatar: {
        width: 28,
        height: 28,
        borderRadius: 14,
        alignItems: 'center',
        justifyContent: 'center',
    },
    avatarText: {
        fontSize: 13,
        fontWeight: '600',
    },
    title: {
        flex: 1,
        // eslint-disable-next-line i18next/no-literal-string
        fontFamily: 'serif',
        fontSize: 18,
    },
    body: {
        rowGap: 14,
    },
    userBubble: {
        alignSelf: 'flex-end',
        maxWidth: '82%',
        paddingHorizontal: 14,
        paddingVertical: 9,
        borderRadius: 18,
        borderBottomRightRadius: 6,
    },
    message: {
        fontSize: 14,
        lineHeight: 20,
    },
    reply: {
        rowGap: 10,
        paddingRight: 24,
    },
    replyMeta: {
        flexDirection: 'row',
        alignItems: 'center',
        columnGap: 10,
    },
    line: {
        height: 6,
        width: 56,
        borderRadius: 3,
    },
    accent: {
        flexDirection: 'row',
        alignItems: 'center',
        columnGap: 4,
        paddingHorizontal: 12,
        paddingVertical: 5,
        borderRadius: 999,
    },
    accentText: {
        fontSize: 12,
        fontWeight: '600',
    },
    input: {
        flexDirection: 'row',
        alignItems: 'center',
        columnGap: 10,
        borderWidth: 1,
        borderRadius: 999,
        paddingLeft: 14,
        paddingRight: 5,
        paddingVertical: 5,
    },
    placeholder: {
        flex: 1,
        fontSize: 14,
    },
    send: {
        width: 30,
        height: 30,
        borderRadius: 15,
        alignItems: 'center',
        justifyContent: 'center',
    },
})
