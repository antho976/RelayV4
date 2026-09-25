import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    ErrorState,
    LoadingState,
    relay,
    Row,
    Section,
    useBusQuery,
    useRelayEvent,
} from '@components/relay'
import { Theme } from '@lib/theme/ThemeManager'

import { attempt } from './common'
import { ProviderInfo } from './types'

type UpdateState = { state: string; message: string }

const TITLES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex' }

/**
 * The agent CLIs on the PC: installed or not, version, who is signed in; a re-check, and a
 * background update of a user-installed one with its progress (provider.update.changed).
 */
const Providers = () => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const query = useBusQuery<ProviderInfo[]>(
        'provider.list',
        {},
        { events: ['provider.version', 'provider.changed'], select: (raw) => raw.providers }
    )
    const [updates, setUpdates] = useState<Record<string, UpdateState>>({})
    const [checking, setChecking] = useState(false)

    useRelayEvent(['provider.update.changed'], (event) => {
        const { provider, state, message } = event.payload ?? {}
        if (!provider) return
        setUpdates((current) => ({ ...current, [provider]: { state, message } }))
        if (state === 'complete' || state === 'failed') query.reload()
    })

    const refresh = async () => {
        setChecking(true)
        const result = await attempt(() =>
            relay.guarded<{ providers: ProviderInfo[] }>('provider.refresh')
        )
        setChecking(false)
        if (result) query.setData(result.providers)
    }

    const update = async (info: ProviderInfo) => {
        const title = TITLES[info.provider] ?? info.provider
        const yes = await confirm({
            title: `Update ${title}?`,
            message:
                'The PC updates it in the background. Running sessions keep their version; new ones use the update. Relay skips it while a session of this provider is live.',
            confirmLabel: 'Update',
        })
        if (!yes) return
        const result = await attempt(() =>
            relay.guarded<{ started: boolean; method: string; message: string }>(
                'provider.update',
                { provider: info.provider }
            )
        )
        if (!result) return
        setUpdates((current) => ({
            ...current,
            [info.provider]: {
                state: result.started ? 'updating' : 'not_started',
                message: result.message,
            },
        }))
    }

    return (
        <Section
            title="Agent providers"
            action={{ label: checking ? 'Checking…' : 'Check again', onPress: refresh }}>
            {query.data === undefined ? (
                query.error ? (
                    <ErrorState error={query.error} onRetry={query.reload} />
                ) : (
                    <LoadingState />
                )
            ) : (
                query.data.map((info) => {
                    const progress = updates[info.provider]
                    const busy = progress?.state === 'updating'
                    const detail = info.installed
                        ? [
                              info.version ? `Version ${info.version}` : 'Version unknown',
                              info.signed_in_as
                                  ? `signed in as ${info.signed_in_as}`
                                  : 'not signed in',
                          ].join(' · ')
                        : 'Not installed on the PC'
                    return (
                        <View key={info.provider} style={{ paddingBottom: spacing.m }}>
                            <Row
                                label={TITLES[info.provider] ?? info.provider}
                                detail={detail}
                                icon={info.installed ? 'check-circle' : 'close-circle'}
                                right={
                                    info.installed && !info.guarded ? (
                                        <Chip label="No guardrail hooks" tone="warn" />
                                    ) : undefined
                                }
                            />
                            {!!info.path && (
                                <Text style={styles.path} numberOfLines={1}>
                                    {info.path}
                                </Text>
                            )}
                            {progress && (
                                <Text
                                    style={[
                                        styles.progress,
                                        progress.state === 'failed' && styles.failed,
                                    ]}>
                                    {progress.state === 'updating' ? 'Updating… ' : ''}
                                    {progress.message}
                                </Text>
                            )}
                            {info.installed && (
                                <View style={styles.actions}>
                                    <ThemedButton
                                        label={busy ? 'Updating…' : 'Update'}
                                        iconName="cloud-download"
                                        variant={busy ? 'disabled' : 'secondary'}
                                        onPress={() => update(info)}
                                    />
                                </View>
                            )}
                        </View>
                    )
                })
            )}
        </Section>
    )
}

export default Providers

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        path: {
            color: color.text._500,
            fontSize: fontSize.s,
            fontFamily: 'monospace',
            marginTop: -spacing.m,
            marginLeft: 18 + spacing.l,
        },
        progress: {
            color: color.text._300,
            fontSize: fontSize.s,
            marginTop: spacing.s,
            marginLeft: 18 + spacing.l,
        },
        failed: {
            color: color.error._300,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            marginTop: spacing.s,
        },
    })
}
