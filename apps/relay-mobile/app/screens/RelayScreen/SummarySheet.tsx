import React, { useEffect, useImperativeHandle, useRef, useState } from 'react'
import { ScrollView, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import BottomSheet, { useBottomSheetRef } from '@components/views/BottomSheet'
import { stopSummary, summarizeReason, summarizeTerminal } from '@lib/engine/Relay/Summarize'
import { Theme } from '@lib/theme/ThemeManager'

export type SummarySheetRef = {
    /** Open the sheet and summarize this text, as it is right now. */
    open: (input: string) => void
}

type SummarySheetProps = {
    ref: React.Ref<SummarySheetRef>
    session: string
}

/** "What did it do?" answered by the phone's own model, from the terminal text it already has. */
const SummarySheet: React.FC<SummarySheetProps> = ({ ref, session }) => {
    const { color, spacing, fontSize } = Theme.useTheme()
    const sheet = useBottomSheetRef()
    const [text, setText] = useState('')
    const [running, setRunning] = useState(false)
    const [problem, setProblem] = useState('')
    const active = useRef(false)

    const stop = () => {
        if (active.current) stopSummary().catch(() => {})
        active.current = false
    }

    useImperativeHandle(ref, () => ({
        open: (input: string) => {
            sheet.current?.open()
            const reason = summarizeReason()
            setText('')
            if (reason) {
                setProblem(reason)
                return
            }
            setProblem('')
            setRunning(true)
            active.current = true
            summarizeTerminal(session, input, (piece) => {
                if (active.current) setText((current) => current + piece)
            })
                .catch((e) => setProblem(`${(e as Error).message}`))
                .finally(() => {
                    setRunning(false)
                    active.current = false
                })
        },
    }))

    // Leaving the terminal while a summary streams must stop the model too.
    useEffect(() => stop, [])

    return (
        <BottomSheet ref={sheet} onClose={stop} sheetStyle={{ maxHeight: '75%' }}>
            <View style={{ rowGap: spacing.m }}>
                <Text style={{ color: color.text._100, fontSize: fontSize.l }}>
                    {session} — on-device summary
                </Text>
                <Text style={{ color: color.text._500, fontSize: fontSize.s }}>
                    Read by the model on this phone. The terminal text never leaves it.
                </Text>
                <ScrollView style={{ maxHeight: 360 }}>
                    {!!problem && <Text style={{ color: color.error._300 }}>{problem}</Text>}
                    <Text selectable style={{ color: color.text._100, lineHeight: 20 }}>
                        {text || (running ? 'Reading…' : '')}
                    </Text>
                </ScrollView>
                <View
                    style={{
                        flexDirection: 'row',
                        justifyContent: 'flex-end',
                        columnGap: spacing.m,
                    }}>
                    {running && (
                        <ThemedButton
                            label="Stop"
                            variant="secondary"
                            onPress={() => stopSummary()}
                        />
                    )}
                    <ThemedButton
                        label="Close"
                        variant="tertiary"
                        onPress={() => sheet.current?.close()}
                    />
                </View>
            </View>
        </BottomSheet>
    )
}

export default SummarySheet
