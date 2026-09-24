import { getStringAsync } from 'expo-clipboard'
import React, { useImperativeHandle, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import ThemedTextInput from '@components/input/ThemedTextInput'
import { Theme } from '@lib/theme/ThemeManager'

import BottomSheet, { BottomSheetRef, useBottomSheetRef } from './BottomSheet'

export type InputSheetProps = {
    ref: BottomSheetRef
    onConfirm: (text: string) => void
    onClose?: () => void
    title?: string
    description?: string
    placeholder?: string
    verifyText?: (text: string) => string
    autoFocus?: boolean
    defaultValue?: string
    multiline?: boolean
    confirmLabel?: string
}

export type InputSheetRef = BottomSheetRef

const InputSheet: React.FC<InputSheetProps> = ({
    ref,
    onConfirm = (text) => {},
    onClose = () => {},
    title = '',
    description = '',
    placeholder = '',
    verifyText = (text: string) => '',
    autoFocus = false,
    defaultValue = '',
    multiline = false,
    confirmLabel,
}) => {
    const { t } = useTranslation()
    const [text, setText] = useState(defaultValue)
    const [errorMessage, setErrorMessage] = useState('')
    const { color, fontSize, spacing } = Theme.useTheme()
    const sheetRef = useBottomSheetRef()

    // every open starts from the current defaultValue, not from what was typed last time
    useImperativeHandle(ref, () => ({
        open: () => {
            setText(defaultValue)
            setErrorMessage('')
            sheetRef.current?.open()
        },
        close: () => sheetRef.current?.close(),
    }))

    const handleClose = () => {
        sheetRef.current?.close()
        onClose()
        setErrorMessage('')
    }

    return (
        <BottomSheet ref={sheetRef} onRequestClose={handleClose}>
            <View style={{ rowGap: spacing.xl }}>
                {title && (
                    <Text
                        style={{
                            color: color.text._100,
                            fontSize: fontSize.l,
                            paddingLeft: spacing.s,
                        }}>
                        {title}
                    </Text>
                )}
                {description && (
                    <Text
                        style={{
                            color: color.text._400,
                        }}>
                        {description}
                    </Text>
                )}

                <View style={{ flexDirection: 'row', columnGap: spacing.m }}>
                    <ThemedTextInput
                        multiline={multiline}
                        autoFocus={autoFocus}
                        placeholder={placeholder}
                        value={text}
                        onChangeText={setText}
                        containerStyle={{ flex: 1 }}
                        numberOfLines={10}
                    />
                </View>
                {errorMessage && <Text style={{ color: color.error._300 }}>{errorMessage}</Text>}

                <View
                    style={{
                        flexDirection: 'row',
                        justifyContent: 'space-between',
                    }}>
                    <ThemedButton
                        label={t('common.actions.close')}
                        variant="secondary"
                        onPress={handleClose}
                    />
                    <View
                        style={{
                            flexDirection: 'row',
                            columnGap: spacing.xl,
                            justifyContent: 'flex-end',
                        }}>
                        <ThemedButton
                            iconStyle={{ color: color.text._400 }}
                            iconName="close"
                            variant="tertiary"
                            accessibilityLabel={t('common.actions.clear')}
                            onPress={() => setText('')}
                        />
                        <ThemedButton
                            iconStyle={{ color: color.text._400 }}
                            iconName="copy"
                            variant="tertiary"
                            accessibilityLabel={t('common.actions.paste')}
                            onPress={async () => {
                                const paste = await getStringAsync()
                                if (paste) setText((text) => text + paste)
                            }}
                        />
                    </View>
                    <ThemedButton
                        label={confirmLabel ?? t('common.actions.save')}
                        onPress={() => {
                            const result = verifyText(text)
                            if (result) setErrorMessage(result)
                            else {
                                onConfirm(text)
                                handleClose()
                            }
                        }}
                    />
                </View>
            </View>
        </BottomSheet>
    )
}

export default InputSheet
