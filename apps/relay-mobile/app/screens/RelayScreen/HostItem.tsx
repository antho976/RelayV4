import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import React from 'react'
import { StyleSheet, Text, TouchableOpacity, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import HorizontalSelector from '@components/input/HorizontalSelector'
import Alert from '@components/views/Alert'
import { useBottomSheetRef } from '@components/views/BottomSheet'
import InputSheet from '@components/views/InputSheet'
import { parseManualAddress } from '@lib/engine/Relay/PairLink'
import { relay, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { isTailnetUrl, RelayHost, RelayRoute, useRelayHostsStore } from '@lib/state/RelayHosts'
import { Theme } from '@lib/theme/ThemeManager'

import { linkColor } from './console'

type HostItemProps = {
    host: RelayHost
}

type Route = { url: string; kind: 'wifi' | 'tailscale' | 'added' | 'server'; removable: boolean }

const KIND: Record<Route['kind'], { label: string; icon: AntDesignIconName }> = {
    wifi: { label: 'WiFi', icon: 'wifi' },
    tailscale: { label: 'Tailscale', icon: 'global' },
    added: { label: 'Added', icon: 'link' },
    server: { label: 'Server', icon: 'cloud-server' },
}

const routesOf = (host: RelayHost): Route[] => {
    const kindOf = (url: string, fallback: Route['kind']): Route['kind'] =>
        isTailnetUrl(url) ? 'tailscale' : fallback
    const listed = host.direct.map((url) => ({
        url: url,
        kind: kindOf(url, 'wifi'),
        removable: false,
    }))
    const added = (host.extra ?? [])
        .filter((url) => !host.direct.includes(url))
        .map((url) => ({ url: url, kind: kindOf(url, 'added'), removable: true }))
    const via = host.via ? [{ url: host.via, kind: 'server' as const, removable: false }] : []
    return [...listed, ...added, ...via]
}

/** A paired PC: where it can be reached, which route to prefer, and the connect key. */
const HostItem: React.FC<HostItemProps> = ({ host }) => {
    const styles = useStyles()
    const { color, spacing } = Theme.useTheme()
    const status = useRelayStore((state) => state.status)
    const hostId = useRelayStore((state) => state.hostId)
    const routeUrl = useRelayStore((state) => state.routeUrl)
    const wanted = useRelayStore((state) => state.wanted)
    const { updateHost, removeHost, setActiveHost } = useRelayHostsStore()
    const addSheet = useBottomSheetRef()
    const connected = status === 'online' && hostId === host.id
    const connecting = status === 'connecting' && hostId === host.id
    // Offline between attempts: the link is still wanted and will try again on its own.
    const retrying = status === 'offline' && wanted && hostId === host.id
    const routes = routesOf(host)
    const hasTailnet = routes.some((route) => route.kind === 'tailscale')

    const handleConnect = async () => {
        setActiveHost(host.id)
        try {
            await relay.connect(host)
        } catch (e) {
            Logger.errorToast(`${(e as Error).message}`)
        }
    }

    const handleForget = () => {
        Alert.alert({
            title: 'Forget PC',
            description: `Forget "${host.name}"? You will have to pair again to reach it. To stop this phone from the PC side, run \`relay remote revoke ${host.deviceId}\` there.`,
            buttons: [
                { label: 'Cancel' },
                {
                    label: 'Forget',
                    type: 'warning',
                    onPress: () => {
                        if (hostId === host.id) relay.disconnect()
                        removeHost(host.id)
                    },
                },
            ],
        })
    }

    const addAddress = (text: string) => {
        const url = parseManualAddress(text)
        if (!url) return
        const extra = [...(host.extra ?? []).filter((item) => item !== url), url]
        updateHost(host.id, { extra })
        Logger.infoToast(isTailnetUrl(url) ? 'Tailscale address added' : 'Address added')
        // With nothing connected, this PC is worth trying again on the new address right away;
        // a live link to this or another PC is left alone.
        if (useRelayStore.getState().status === 'offline') {
            const stored = useRelayHostsStore.getState().hosts.find((item) => item.id === host.id)
            if (stored) relay.connect(stored).catch(() => {})
        }
    }

    const removeAddress = (url: string) => {
        Alert.alert({
            title: 'Remove address',
            description: `Stop trying ${url} for "${host.name}"?`,
            buttons: [
                { label: 'Cancel' },
                {
                    label: 'Remove',
                    type: 'warning',
                    onPress: () =>
                        updateHost(host.id, {
                            extra: (host.extra ?? []).filter((item) => item !== url),
                        }),
                },
            ],
        })
    }

    // A pin is offered only for a route this PC has: "Server only" with no server would
    // leave nothing to try. A pin whose route has since gone reads, and acts, as Auto.
    const hasDirect = routes.some((route) => route.kind !== 'server')
    const choices: { label: string; value: RelayRoute }[] = [
        { label: 'Auto', value: 'auto' },
        ...(hasDirect && host.via ? [{ label: 'Direct only', value: 'direct' as const }] : []),
        ...(hasDirect && host.via ? [{ label: 'Server only', value: 'via' as const }] : []),
    ]
    const route = choices.some((choice) => choice.value === host.route) ? host.route : 'auto'

    return (
        <View style={styles.record}>
            <InputSheet
                ref={addSheet}
                title="Add an address"
                description="Reach this PC from anywhere without a server: install Tailscale on the PC and on this phone, sign both in to the same account, and enter the PC's Tailscale address (100.x.y.z, or its name.tailnet.ts.net). Any other address the PC answers on works too."
                placeholder="100.101.102.103"
                confirmLabel="Add"
                verifyText={(text) =>
                    parseManualAddress(text)
                        ? ''
                        : 'Enter an address like 100.101.102.103, my-pc.tail1234.ts.net or ws://host:7420.'
                }
                onConfirm={addAddress}
            />
            <View style={styles.head}>
                <View style={styles.icon}>
                    <AntDesign name="desktop" size={20} color={color.text._200} />
                    <View
                        style={[
                            styles.iconLamp,
                            {
                                backgroundColor: linkColor(
                                    connected ? 'online' : connecting ? 'connecting' : 'offline',
                                    color
                                ),
                                borderColor: color.neutral._200,
                            },
                        ]}
                    />
                </View>
                <View style={{ flex: 1 }}>
                    <Text style={styles.name} numberOfLines={1}>
                        {host.name}
                    </Text>
                    <Text style={styles.meta} numberOfLines={1}>
                        {connected
                            ? 'Connected'
                            : connecting
                              ? 'Connecting…'
                              : retrying
                                ? 'Retrying'
                                : 'Not connected'}{' '}
                        · {host.instance}
                    </Text>
                </View>
                {connected || retrying ? (
                    <ThemedButton
                        label="Disconnect"
                        variant="secondary"
                        onPress={() => relay.disconnect()}
                    />
                ) : (
                    <ThemedButton
                        label={connecting ? 'Connecting…' : 'Connect'}
                        variant={connecting ? 'disabled' : 'primary'}
                        onPress={handleConnect}
                    />
                )}
            </View>

            <View style={styles.routes}>
                {routes.map((route) => {
                    const live = connected && routeUrl === route.url
                    return (
                        <View key={`${route.kind}:${route.url}`} style={styles.route}>
                            <AntDesign
                                name={KIND[route.kind].icon}
                                size={14}
                                color={live ? color.primary._700 : color.text._500}
                            />
                            <Text style={styles.routeKind}>{KIND[route.kind].label}</Text>
                            <Text
                                numberOfLines={1}
                                style={[styles.routeUrl, live && { color: color.text._100 }]}>
                                {route.url.replace(/^wss?:\/\//, '')}
                            </Text>
                            {live && <Text style={styles.inUse}>in use</Text>}
                            {route.removable && (
                                <TouchableOpacity
                                    hitSlop={10}
                                    onPress={() => removeAddress(route.url)}>
                                    <AntDesign name="close" size={14} color={color.text._500} />
                                </TouchableOpacity>
                            )}
                        </View>
                    )
                })}
                {!host.via && !hasTailnet && (
                    <Text style={styles.routeQuiet}>
                        Same network only. Add a Tailscale address to reach it from anywhere.
                    </Text>
                )}
                <TouchableOpacity style={styles.addRoute} onPress={() => addSheet.current?.open()}>
                    <AntDesign name="plus" size={14} color={color.primary._700} />
                    <Text style={styles.addRouteText}>Add Tailscale or other address</Text>
                </TouchableOpacity>
            </View>

            {choices.length > 1 && (
                <HorizontalSelector
                    style={{ flex: 0, marginTop: spacing.s }}
                    values={choices}
                    selected={route}
                    onPress={(next) => updateHost(host.id, { route: next })}
                />
            )}
            <View style={styles.footer}>
                <Text style={styles.device} numberOfLines={1}>
                    device {host.deviceId}
                </Text>
                <ThemedButton label="Forget" variant="tertiary" onPress={handleForget} />
            </View>
        </View>
    )
}

export default HostItem

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        record: {
            backgroundColor: color.neutral._200,
            borderRadius: 16,
            padding: spacing.l,
            rowGap: spacing.l,
        },
        head: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
        },
        icon: {
            width: 40,
            height: 40,
            borderRadius: 12,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: color.neutral._300,
        },
        iconLamp: {
            position: 'absolute',
            right: -2,
            bottom: -2,
            width: 12,
            height: 12,
            borderRadius: 6,
            borderWidth: 2,
        },
        name: {
            color: color.text._100,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
        meta: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        routes: {
            rowGap: spacing.s,
            paddingTop: spacing.m,
            borderTopWidth: 1,
            borderTopColor: color.neutral._300,
        },
        route: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        routeKind: {
            width: 64,
            color: color.text._400,
            fontSize: fontSize.s,
        },
        routeUrl: {
            flex: 1,
            color: color.text._300,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        inUse: {
            color: color.primary._700,
            fontSize: fontSize.s - 1,
            letterSpacing: 0.6,
            textTransform: 'uppercase',
        },
        routeQuiet: {
            color: color.text._500,
            fontSize: fontSize.s,
        },
        addRoute: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
            paddingVertical: spacing.s,
        },
        addRouteText: {
            color: color.primary._700,
            fontSize: fontSize.s,
        },
        footer: {
            flexDirection: 'row',
            alignItems: 'center',
            justifyContent: 'space-between',
        },
        device: {
            flex: 1,
            color: color.text._500,
            fontSize: fontSize.s - 1,
            fontFamily: 'monospace',
        },
    })
}
