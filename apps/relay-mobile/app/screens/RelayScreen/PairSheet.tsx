import { CameraView, useCameraPermissions } from 'expo-camera'
import { getStringAsync } from 'expo-clipboard'
import React, { useEffect, useRef, useState } from 'react'
import { Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import ThemedTextInput from '@components/input/ThemedTextInput'
import BottomSheet, { useBottomSheetRef } from '@components/views/BottomSheet'
import { PairLink, parseManualAddress, parsePairLink } from '@lib/engine/Relay/PairLink'
import { relay } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type PairSheetProps = {
    visible: boolean
    setVisible: (visible: boolean) => void
}

type Mode = 'scan' | 'type'

/**
 * Pair with a PC: scan the QR that `relay remote pair` prints, paste the link, or type the
 * address and the code by hand.
 */
const PairSheet: React.FC<PairSheetProps> = ({ visible, setVisible }) => {
    const { color, spacing, fontSize } = Theme.useTheme()
    const [permission, requestPermission] = useCameraPermissions()
    const [mode, setMode] = useState<Mode>('scan')
    const [busy, setBusy] = useState(false)
    const [address, setAddress] = useState('')
    const [code, setCode] = useState('')
    const [error, setError] = useState('')
    const [scanned, setScanned] = useState(false)
    // State updates land on the next render; the camera can report twice before then.
    const scanLock = useRef(false)
    const pairLock = useRef(false)
    const sheet = useBottomSheetRef()

    // The sheet is opened and closed through its ref; `visible` is what the parent asked for.
    useEffect(() => {
        if (visible) sheet.current?.open()
        else sheet.current?.close()
    }, [visible, sheet])

    const finish = async (link: PairLink) => {
        if (pairLock.current) return
        pairLock.current = true
        setBusy(true)
        setError('')
        try {
            const host = await relay.pair(link)
            Logger.infoToast(`Paired with ${host.name}`)
            await relay.refresh().catch(() => {})
            setVisible(false)
        } catch (e) {
            setError(`${(e as Error).message}`)
        } finally {
            pairLock.current = false
            setBusy(false)
        }
    }

    // The camera reports the same code several times a second; one scan is one attempt,
    // and "Scan again" is the only way to re-arm it after a failure.
    const handleScanned = (data: string) => {
        if (scanLock.current || pairLock.current) return
        scanLock.current = true
        setScanned(true)
        const link = parsePairLink(data)
        if (!link) {
            setError('That code is not a Relay pairing link.')
            return
        }
        finish(link)
    }

    const handlePaste = async () => {
        const text = await getStringAsync()
        const link = parsePairLink(text)
        if (!link) {
            setError('The clipboard does not hold a relay://pair link.')
            return
        }
        finish(link)
    }

    const handleTyped = () => {
        const url = parseManualAddress(address)
        if (!url) {
            setError('Enter the PC address, like 192.168.1.20 or wss://your-server/join/<room>.')
            return
        }
        if (code.replace(/[^a-z0-9]/gi, '').length < 8) {
            setError('Enter the eight-character pairing code shown on the PC.')
            return
        }
        const isVia = /\/join\//.test(url)
        finish({
            code: code,
            host: 'Relay PC',
            hostId: '',
            instance: 'stable',
            direct: isVia ? [] : [url],
            via: isVia ? url : undefined,
        })
    }

    const canScan = permission?.granted

    return (
        <BottomSheet
            ref={sheet}
            onClose={() => {
                setVisible(false)
                setError('')
                scanLock.current = false
                setScanned(false)
            }}
            sheetStyle={{ maxHeight: '85%' }}>
            <View style={{ rowGap: spacing.l }}>
                <Text style={{ color: color.text._100, fontSize: fontSize.l }}>Pair a PC</Text>
                <Text style={{ color: color.text._400 }}>
                    On the PC, run `relay remote pair`. Scan the code it prints, or type what it
                    shows, then approve this phone in that terminal when it asks.
                </Text>
                <View style={{ flexDirection: 'row', columnGap: spacing.m }}>
                    <ThemedButton
                        label="Scan"
                        iconName="scan"
                        variant={mode === 'scan' ? 'primary' : 'secondary'}
                        buttonStyle={{ flex: 1 }}
                        onPress={() => setMode('scan')}
                    />
                    <ThemedButton
                        label="Type"
                        iconName="edit"
                        variant={mode === 'type' ? 'primary' : 'secondary'}
                        buttonStyle={{ flex: 1 }}
                        onPress={() => setMode('type')}
                    />
                </View>

                {mode === 'scan' && (
                    <View style={{ rowGap: spacing.m }}>
                        {canScan ? (
                            <CameraView
                                style={{
                                    height: 260,
                                    borderRadius: 2,
                                    overflow: 'hidden',
                                    backgroundColor: color.neutral._200,
                                }}
                                facing="back"
                                barcodeScannerSettings={{ barcodeTypes: ['qr'] }}
                                onBarcodeScanned={
                                    scanned || busy
                                        ? undefined
                                        : (result) => handleScanned(result.data)
                                }
                            />
                        ) : (
                            <View
                                style={{
                                    height: 120,
                                    alignItems: 'center',
                                    justifyContent: 'center',
                                    rowGap: spacing.m,
                                    backgroundColor: color.neutral._200,
                                }}>
                                <Text style={{ color: color.text._400, textAlign: 'center' }}>
                                    Camera access is needed to scan the pairing code.
                                </Text>
                                <ThemedButton
                                    label="Allow camera"
                                    variant="secondary"
                                    onPress={() => requestPermission()}
                                />
                            </View>
                        )}
                        {scanned && !busy && (
                            <ThemedButton
                                label="Scan again"
                                iconName="reload"
                                variant="secondary"
                                onPress={() => {
                                    scanLock.current = false
                                    setScanned(false)
                                    setError('')
                                }}
                            />
                        )}
                        <ThemedButton
                            label="Paste link from clipboard"
                            iconName="copy"
                            variant="secondary"
                            onPress={handlePaste}
                        />
                    </View>
                )}

                {mode === 'type' && (
                    <View style={{ rowGap: spacing.m }}>
                        <ThemedTextInput
                            label="PC address"
                            description="The direct line the PC printed, its Tailscale address (100.x.y.z) or its rendezvous join link."
                            value={address}
                            onChangeText={setAddress}
                            placeholder="192.168.1.20  or  wss://server/join/…"
                            autoCapitalize="none"
                            autoCorrect={false}
                            keyboardType="url"
                        />
                        <ThemedTextInput
                            label="Pairing code"
                            value={code}
                            onChangeText={setCode}
                            placeholder="ABCD-EFGH"
                            autoCapitalize="characters"
                            autoCorrect={false}
                        />
                        <ThemedButton
                            label={busy ? 'Pairing…' : 'Pair'}
                            variant={busy ? 'disabled' : 'primary'}
                            onPress={handleTyped}
                        />
                    </View>
                )}

                {busy && mode === 'scan' && (
                    <Text style={{ color: color.text._300 }}>
                        Pairing… approve it in the terminal on the PC when asked.
                    </Text>
                )}
                {!!error && <Text style={{ color: color.error._300 }}>{error}</Text>}
            </View>
        </BottomSheet>
    )
}

export default PairSheet
