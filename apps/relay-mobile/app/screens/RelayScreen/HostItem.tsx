import React from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import HorizontalSelector from '@components/input/HorizontalSelector'
import Alert from '@components/views/Alert'
import { relay, useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Logger } from '@lib/state/Logger'
import { RelayHost, RelayRoute, useRelayHostsStore } from '@lib/state/RelayHosts'
import { Theme } from '@lib/theme/ThemeManager'

type HostItemProps = {
    host: RelayHost
}

/** A paired PC: where it can be reached, which route to prefer, and the connect key. */
const HostItem: React.FC<HostItemProps> = ({ host }) => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const { status, hostId, transport } = useRelayStore()
    const { updateHost, removeHost, setActiveHost } = useRelayHostsStore()
    const connected = status === 'online' && hostId === host.id
    const connecting = status === 'connecting' && hostId === host.id

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

    const routes: { label: string; value: RelayRoute }[] = [
        { label: 'Auto', value: 'auto' },
        { label: 'WiFi only', value: 'direct' },
        { label: 'Server only', value: 'via' },
    ]

    return (
        <View style={styles.record}>
            <View style={styles.head}>
                <View style={{ flex: 1 }}>
                    <Text style={styles.name}>{host.name}</Text>
                    <Text style={styles.meta}>
                        {host.instance} · device {host.deviceId}
                    </Text>
                </View>
                {connected ? (
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
            <View style={{ rowGap: 2 }}>
                {host.direct.map((url) => (
                    <Text key={url} style={styles.route}>
                        wifi {url}
                        {connected && transport === 'direct' ? '  ●' : ''}
                    </Text>
                ))}
                {host.via ? (
                    <Text style={styles.route}>
                        server {host.via}
                        {connected && transport === 'via' ? '  ●' : ''}
                    </Text>
                ) : (
                    <Text style={styles.routeQuiet}>
                        no server route: reachable on the same network only
                    </Text>
                )}
            </View>
            <HorizontalSelector
                style={{ flex: 0, marginTop: spacing.s }}
                values={routes}
                selected={host.route}
                onPress={(route) => updateHost(host.id, { route })}
            />
            <ThemedButton
                label="Forget this PC"
                variant="tertiary"
                buttonStyle={{ alignSelf: 'flex-end' }}
                onPress={handleForget}
            />
        </View>
    )
}

export default HostItem

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        record: {
            backgroundColor: color.neutral._300,
            padding: spacing.l,
            rowGap: spacing.m,
        },
        head: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
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
        route: {
            color: color.text._300,
            fontFamily: 'monospace',
            fontSize: fontSize.s,
        },
        routeQuiet: {
            color: color.text._500,
            fontSize: fontSize.s,
        },
    })
}
