import AntDesign from '@react-native-vector-icons/ant-design/static'
import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, TextInput, TouchableOpacity, View } from 'react-native'

import { Theme } from '@lib/theme/ThemeManager'

/** A key a phone keyboard does not have and an agent CLI keeps asking for. */
export type TermKey = {
    label: string
    /** Bytes for the PTY. */
    data: string
    /** Answer keys (`y`, `n`) are followed by their own Enter. */
    answer?: boolean
    accessibility?: string
}

export const TERM_KEYS: TermKey[] = [
    { label: 'Esc', data: '\x1b' },
    { label: 'Tab', data: '\t' },
    { label: '⇧Tab', data: '\x1b[Z', accessibility: 'Shift Tab' },
    { label: '↑', data: '\x1b[A', accessibility: 'Up' },
    { label: '↓', data: '\x1b[B', accessibility: 'Down' },
    { label: '←', data: '\x1b[D', accessibility: 'Left' },
    { label: '→', data: '\x1b[C', accessibility: 'Right' },
    { label: '⏎', data: '\r', accessibility: 'Enter' },
    { label: '^C', data: '\x03', accessibility: 'Control C' },
    { label: 'y', data: 'y', answer: true },
    { label: 'n', data: 'n', answer: true },
    { label: '/', data: '/' },
    { label: '^D', data: '\x04', accessibility: 'Control D' },
]

type Props = {
    enabled: boolean
    placeholder: string
    onKey: (key: TermKey) => void
    /** Text to type and submit; empty text is a bare Enter. */
    onSubmit: (text: string) => void
}

/**
 * The bottom of the terminal: a row of small keys that scrolls sideways, and a composer that
 * grows to four lines with one round button. With text it sends and submits; empty, it is
 * Enter.
 */
const Composer: React.FC<Props> = ({ enabled, placeholder, onKey, onSubmit }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const [text, setText] = useState('')

    const submit = () => {
        if (!enabled) return
        onSubmit(text)
        setText('')
    }

    return (
        <View style={styles.dock}>
            <ScrollView
                horizontal
                keyboardShouldPersistTaps="always"
                showsHorizontalScrollIndicator={false}
                contentContainerStyle={styles.keys}>
                {TERM_KEYS.map((key) => (
                    <TouchableOpacity
                        key={key.label}
                        style={[styles.key, !enabled && styles.off]}
                        disabled={!enabled}
                        accessibilityLabel={key.accessibility ?? key.label}
                        onPress={() => onKey(key)}>
                        <Text style={styles.keyText}>{key.label}</Text>
                    </TouchableOpacity>
                ))}
            </ScrollView>
            <View style={styles.row}>
                <TextInput
                    style={styles.input}
                    value={text}
                    onChangeText={setText}
                    placeholder={placeholder}
                    placeholderTextColor={color.text._500}
                    autoCapitalize="none"
                    autoCorrect={false}
                    multiline
                    editable={enabled}
                    submitBehavior="submit"
                    returnKeyType="send"
                    onSubmitEditing={submit}
                />
                <TouchableOpacity
                    style={[styles.send, !enabled && styles.off]}
                    disabled={!enabled}
                    accessibilityLabel={text ? 'Send' : 'Enter'}
                    onPress={submit}>
                    <AntDesign
                        name={text ? 'arrow-up' : 'enter'}
                        size={18}
                        color={color.primary._100}
                    />
                </TouchableOpacity>
            </View>
        </View>
    )
}

export default Composer

const LINE = 20

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        dock: {
            rowGap: spacing.s,
            paddingTop: spacing.s,
            paddingBottom: spacing.m,
        },
        keys: {
            columnGap: 6,
            paddingHorizontal: spacing.m,
        },
        key: {
            minWidth: 38,
            height: 30,
            paddingHorizontal: spacing.m,
            alignItems: 'center',
            justifyContent: 'center',
            borderRadius: 9,
            backgroundColor: color.neutral._300,
        },
        keyText: {
            color: color.text._200,
            fontSize: fontSize.s,
            fontFamily: 'monospace',
        },
        off: {
            opacity: 0.4,
        },
        row: {
            flexDirection: 'row',
            alignItems: 'flex-end',
            columnGap: spacing.s,
            paddingHorizontal: spacing.m,
        },
        input: {
            flex: 1,
            minHeight: 40,
            maxHeight: LINE * 4 + 20,
            color: color.text._100,
            backgroundColor: color.neutral._200,
            borderRadius: 20,
            paddingHorizontal: spacing.l,
            paddingTop: 10,
            paddingBottom: 10,
            fontSize: fontSize.m,
            lineHeight: LINE,
            textAlignVertical: 'center',
        },
        send: {
            width: 40,
            height: 40,
            borderRadius: 20,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.primary._500,
        },
    })
}
