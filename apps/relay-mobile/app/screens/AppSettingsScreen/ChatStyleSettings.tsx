import { useRouter } from 'expo-router'
import React from 'react'
import { useTranslation } from 'react-i18next'
import { ScrollView, Text, View } from 'react-native'
import Markdown from 'react-native-markdown-display'
import { useShallow } from 'zustand/react/shallow'

import ThemedButton from '@components/buttons/ThemedButton'
import SegmentedControl from '@components/theme/SegmentedControl'
import SettingsGroup from '@components/theme/SettingsGroup'
import HeaderTitle from '@components/views/HeaderTitle'
import { MarkdownStyle } from '@lib/markdown/Markdown'
import { ChatStyle } from '@lib/state/ChatStyle'
import { Theme } from '@lib/theme/ThemeManager'

const renderedText = `
This is some test text
| Row One  | Row Two | Row Three   |
|----------|--------:|-------------|
| Item 1   |  row    | row         |
| Item 2   |  row    | row         |

$s = ut + \\frac{1}{2}at^2$ 

Distance from initial velocity, time and acceleration

A **strong** (bold) text example.

A *emphasized* (italic) text example.

A ~~strikethrough~~ text example.

"A quote text example."
`

/** Base size and weight the chat's markdown is scaled from (lib/markdown/Markdown.tsx). */
const BASE_SIZE = 16
const BASE_WEIGHT = 400

/** Size and weight of chat text, with a sample line set in the chosen style. */
export const ChatTextSettings: React.FC<{ showFullPreview?: boolean }> = ({
    showFullPreview = false,
}) => {
    const { t } = useTranslation()
    const router = useRouter()
    const { color, spacing, fontSize } = Theme.useTheme()
    const { weight, size, setWeight, setSize } = ChatStyle.useChatStyle(
        useShallow((state) => ({
            weight: state.textWeight,
            size: state.fontSize,
            setWeight: state.setTextWeight,
            setSize: state.setFontSize,
        }))
    )
    const sampleWeight = Math.max(
        100,
        Math.min(900, BASE_WEIGHT + ChatStyle.weightModifierMap[weight])
    )
    return (
        <SettingsGroup title={t('settings.style.chatText')}>
            <Text
                style={{
                    color: color.text._200,
                    fontSize: Math.max(
                        ChatStyle.MIN_FONT_SIZE,
                        BASE_SIZE + ChatStyle.sizeModifierMap[size]
                    ),
                    fontWeight: `${sampleWeight}` as '400',
                    paddingVertical: spacing.s,
                }}>
                {t('settings.style.chatTextSample')}
            </Text>
            <Text style={{ color: color.text._400, fontSize: fontSize.m }}>
                {t('settings.chatstyle.fontSize')}
            </Text>
            <SegmentedControl
                segments={ChatStyle.SIZES.map((item) => ({
                    value: item,
                    label: item.toUpperCase(),
                }))}
                selected={size}
                onChange={setSize}
            />
            <Text style={{ color: color.text._400, fontSize: fontSize.m }}>
                {t('settings.chatstyle.fontWeight')}
            </Text>
            <SegmentedControl
                segments={ChatStyle.WEIGHTS.map((item) => ({
                    value: item,
                    label: t(`settings.style.weights.${item}`),
                }))}
                selected={weight}
                onChange={setWeight}
            />
            {showFullPreview && (
                <ThemedButton
                    label={t('settings.style.fullPreview')}
                    iconName="file-markdown"
                    iconSize={16}
                    variant="secondary"
                    onPress={() => router.push('/screens/AppSettingsScreen/ChatStyleSettings')}
                />
            )}
        </SettingsGroup>
    )
}

/** Chat text settings with a full markdown sample: tables, maths, emphasis and quotes. */
const ChatStyling = () => {
    const { t } = useTranslation()
    const { markdown, rules, style } = MarkdownStyle.useCustomFormatting()
    const { color, spacing } = Theme.useTheme()
    return (
        <ScrollView
            contentContainerStyle={{
                padding: spacing.xl,
                rowGap: spacing.xl2,
                paddingBottom: spacing.xl3 * 2,
            }}>
            <HeaderTitle title={t('settings.chatstyle.title')} />
            <View
                style={{
                    borderRadius: 22,
                    alignItems: 'center',
                    padding: spacing.xl2,
                    justifyContent: 'center',
                    backgroundColor: color.neutral._100,
                    borderColor: color.neutral._300,
                    borderWidth: 1,
                }}>
                <Markdown mergeStyle={false} markdownit={markdown} rules={rules} style={style}>
                    {renderedText}
                </Markdown>
            </View>
            <ChatTextSettings />
        </ScrollView>
    )
}

export default ChatStyling
