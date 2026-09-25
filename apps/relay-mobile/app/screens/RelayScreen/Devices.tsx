import React, { useEffect, useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    ErrorState,
    Field,
    LoadingState,
    relay,
    Row,
    Screen,
    Section,
    Segmented,
    SwitchRow,
    useBusQuery,
    useProjectParam,
    useRelayEvent,
} from '@components/relay'
import { isTracked, listenToLogcat, trackRun } from '@components/relay/devices/logcat'
import LogView from '@components/relay/devices/LogView'
import { attempt, errorText, formatTime } from '@components/relay/settings/common'
import { DeviceRun } from '@components/relay/settings/types'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

type Device = { serial: string; model: string; kind: 'usb' | 'avd'; state: string }
type Avd = {
    name: string
    device: string | null
    package: string | null
    path: string | null
    running_serial: string | null
}
type Worktree = { path: string; branch: string; session: string | null; dirty: boolean }
type Tab = 'run' | 'runs' | 'log'

const ACTIVE = ['building', 'running']
const isActive = (run: DeviceRun) => ACTIVE.includes(run.state)

const stateTone = (state: string) =>
    state === 'running' || state === 'building' ? 'live' : state === 'failed' ? 'danger' : 'neutral'

/**
 * A project's Android side (the desktop's Tools → Devices): the devices and emulators the PC
 * sees, booting an emulator, Run on device and Build with a live log, and past runs to stop.
 */
const DevicesScreen = () => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const { projectId, project } = useProjectParam()
    const [tab, setTab] = useState<Tab>('run')
    const [runId, setRunId] = useState<number | undefined>(undefined)

    useEffect(() => listenToLogcat(), [])

    const devices = useBusQuery<Device[]>(
        'device.list',
        {},
        {
            events: ['device.*', 'avd.*'],
            select: (raw) => raw.devices,
        }
    )
    const avds = useBusQuery<Avd[]>(
        'avd.list',
        {},
        { events: ['avd.*'], select: (raw) => raw.avds }
    )
    const worktrees = useBusQuery<Worktree[]>(
        'worktree.list',
        { project_id: projectId },
        {
            enabled: projectId !== undefined,
            events: ['worktree.*'],
            projectId: projectId,
            select: (raw) => raw.worktrees,
        }
    )
    const runs = useBusQuery<DeviceRun[]>(
        'device.run.list',
        { project_id: projectId },
        {
            enabled: projectId !== undefined,
            events: ['run.*'],
            projectId: projectId,
            select: (raw) => [...raw.runs].sort((a: DeviceRun, b: DeviceRun) => b.id - a.id),
        }
    )

    useRelayEvent(['run.crash'], (event) => {
        const crashed = event.payload?.run_id
        if (crashed !== undefined && isTracked(crashed))
            Logger.errorToast('The app crashed on the device')
    })

    const [worktree, setWorktree] = useState('')
    const [picked, setDevice] = useState<string | undefined>(undefined)
    const [variant, setVariant] = useState('debug')
    const [format, setFormat] = useState<'apk' | 'aab'>('apk')
    const [publish, setPublish] = useState(false)
    const [starting, setStarting] = useState(false)

    const ready = (devices.data ?? []).filter((item) => item.state === 'device')
    // The first ready device is the usual target; a pick holds while it stays connected.
    const device = ready.some((item) => item.serial === picked) ? picked : ready[0]?.serial

    if (projectId === undefined) {
        return (
            <Screen title="Android devices">
                <EmptyState icon="mobile" title="No project" text="Open a project first." />
            </Screen>
        )
    }

    const start = async (op: 'device.run' | 'device.build') => {
        if (!variant.trim()) {
            Logger.errorToast('Enter the Gradle variant to build.')
            return
        }
        const payload: Record<string, unknown> = {
            project_id: projectId,
            variant: variant.trim(),
            ...(worktree ? { worktree } : {}),
        }
        if (op === 'device.run') payload.device = device
        else {
            payload.format = format
            payload.publish = publish
        }
        if (op === 'device.build' && publish) {
            const yes = await confirm({
                title: 'Build and publish?',
                message:
                    'The PC uploads the artifact with Gradle Play Publisher when the build succeeds.',
                confirmLabel: 'Build and publish',
            })
            if (!yes) return
        }
        setStarting(true)
        const run = await attempt(() => relay.guarded<DeviceRun>(op, payload))
        setStarting(false)
        if (!run) return
        trackRun(run.id)
        setRunId(run.id)
        setTab('log')
        runs.reload()
    }

    const stop = async (run: DeviceRun) => {
        const done = await attempt(
            () => relay.guarded('device.run.stop', { run_id: run.id }),
            'Stopped'
        )
        if (done !== undefined) runs.reload()
    }

    const boot = async (avd: Avd, cold: boolean) => {
        const done = await attempt(
            () => relay.guarded('avd.boot', { name: avd.name, ...(cold ? { cold: true } : {}) }),
            `Booting ${avd.name} on the PC`
        )
        if (done !== undefined) avds.reload()
    }

    const refresh = () => {
        devices.reload()
        avds.reload()
        worktrees.reload()
        runs.reload()
    }

    const shownRun = runs.data?.find((item) => item.id === runId)

    const tabs = (
        <Segmented
            options={[
                { value: 'run', label: 'Run' },
                { value: 'runs', label: `Runs${runs.data?.some(isActive) ? ' ·' : ''}` },
                { value: 'log', label: 'Log' },
            ]}
            value={tab}
            onChange={setTab}
        />
    )

    if (tab === 'log') {
        return (
            <Screen
                title={project ? `${project.name} · Android` : 'Android devices'}
                scroll={false}>
                <View style={[styles.logPage, { rowGap: spacing.l }]}>
                    {tabs}
                    {runId === undefined ? (
                        <EmptyState
                            icon="profile"
                            title="No log yet"
                            text="Start a run or a build from this phone, or pick one of its runs, to follow its log here."
                        />
                    ) : (
                        <>
                            <View style={styles.runHead}>
                                <Text style={styles.runTitle} numberOfLines={1}>
                                    {shownRun
                                        ? `#${shownRun.id} · ${shownRun.kind}${shownRun.device ? ` on ${shownRun.device}` : ''}`
                                        : `#${runId}`}
                                </Text>
                                {shownRun && (
                                    <Chip label={shownRun.state} tone={stateTone(shownRun.state)} />
                                )}
                                {shownRun && isActive(shownRun) && (
                                    <ThemedButton
                                        label="Stop"
                                        variant="critical"
                                        onPress={() => stop(shownRun)}
                                    />
                                )}
                            </View>
                            {shownRun?.artifact && (
                                <Text style={styles.note} selectable numberOfLines={2}>
                                    {shownRun.artifact}
                                    {shownRun.signing ? ` · ${shownRun.signing}` : ''}
                                </Text>
                            )}
                            {isTracked(runId) ? (
                                <LogView runId={runId} active={!!shownRun && isActive(shownRun)} />
                            ) : (
                                <Text style={styles.note}>
                                    Logs only stream to the phone for runs started from this phone
                                    (since the app last started). This run’s log is on the PC.
                                </Text>
                            )}
                        </>
                    )}
                </View>
            </Screen>
        )
    }

    return (
        <Screen
            title={project ? `${project.name} · Android` : 'Android devices'}
            onRefresh={refresh}
            refreshing={devices.loading || runs.loading}>
            {tabs}
            {tab === 'run' ? (
                <>
                    <Section title="Devices" action={{ label: 'Refresh', onPress: devices.reload }}>
                        {devices.data === undefined ? (
                            devices.error ? (
                                <ErrorState error={devices.error} onRetry={devices.reload} />
                            ) : (
                                <LoadingState />
                            )
                        ) : devices.data.length === 0 ? (
                            <Text style={[styles.note, { paddingVertical: spacing.l }]}>
                                No device is connected to the PC. Plug one in with USB debugging on,
                                or boot an emulator below.
                            </Text>
                        ) : (
                            devices.data.map((item) => (
                                <Row
                                    key={item.serial}
                                    label={item.model || item.serial}
                                    detail={`${item.serial} · ${item.kind === 'avd' ? 'emulator' : 'USB'}`}
                                    icon={item.serial === device ? 'check-circle' : 'mobile'}
                                    chevron={false}
                                    disabled={item.state !== 'device'}
                                    right={
                                        item.state !== 'device' ? (
                                            <Chip label={item.state} tone="warn" />
                                        ) : undefined
                                    }
                                    onPress={() => setDevice(item.serial)}
                                />
                            ))
                        )}
                    </Section>

                    <Section title="Emulators">
                        {avds.data === undefined ? (
                            avds.error ? (
                                <Text style={[styles.note, { paddingVertical: spacing.l }]}>
                                    {errorText(avds.error)}
                                </Text>
                            ) : (
                                <LoadingState />
                            )
                        ) : avds.data.length === 0 ? (
                            <Text style={[styles.note, { paddingVertical: spacing.l }]}>
                                No Android Virtual Device on the PC. Create one on the desktop.
                            </Text>
                        ) : (
                            avds.data.map((avd) => (
                                <Row
                                    key={avd.name}
                                    label={avd.name}
                                    detail={[
                                        avd.device,
                                        avd.running_serial
                                            ? `running as ${avd.running_serial}`
                                            : 'Tap to boot, long-press for a cold boot',
                                    ]
                                        .filter(Boolean)
                                        .join(' · ')}
                                    icon="android"
                                    chevron={false}
                                    right={
                                        avd.running_serial ? (
                                            <Chip label="Running" tone="live" />
                                        ) : undefined
                                    }
                                    disabled={!!avd.running_serial}
                                    onPress={() => boot(avd, false)}
                                    onLongPress={() => boot(avd, true)}
                                />
                            ))
                        )}
                    </Section>

                    <Section title="Run or build">
                        <View style={{ rowGap: spacing.l, paddingVertical: spacing.l }}>
                            <Text style={styles.label}>Build from</Text>
                            <View style={styles.chips}>
                                <Chip
                                    label="Primary checkout"
                                    selected={worktree === ''}
                                    onPress={() => setWorktree('')}
                                />
                                {(worktrees.data ?? [])
                                    // The primary checkout is the first chip already.
                                    .filter((tree) => tree.path !== project?.path)
                                    .map((tree) => (
                                        <Chip
                                            key={tree.path}
                                            label={
                                                tree.session
                                                    ? `${tree.branch} (${tree.session})`
                                                    : tree.branch
                                            }
                                            selected={worktree === tree.path}
                                            onPress={() => setWorktree(tree.path)}
                                        />
                                    ))}
                            </View>
                            <Field
                                label="Gradle variant"
                                value={variant}
                                onChangeText={setVariant}
                                autoCapitalize="none"
                                autoCorrect={false}
                                mono
                            />
                            <ThemedButton
                                label={
                                    starting
                                        ? 'Starting…'
                                        : device
                                          ? `Run on ${ready.find((item) => item.serial === device)?.model ?? device}`
                                          : 'Run on device'
                                }
                                iconName="play-circle"
                                variant={device && !starting ? 'primary' : 'disabled'}
                                onPress={() => start('device.run')}
                            />
                            <Text style={styles.label}>Build an artifact</Text>
                            <Segmented
                                options={[
                                    { value: 'apk', label: 'APK' },
                                    { value: 'aab', label: 'App Bundle' },
                                ]}
                                value={format}
                                onChange={setFormat}
                            />
                        </View>
                        <SwitchRow
                            label="Publish"
                            description="Upload the artifact with Gradle Play Publisher after a successful build."
                            value={publish}
                            onChange={setPublish}
                        />
                        <View style={{ paddingVertical: spacing.l }}>
                            <ThemedButton
                                label={
                                    starting ? 'Starting…' : publish ? 'Build and publish' : 'Build'
                                }
                                iconName="build"
                                variant={starting ? 'disabled' : 'secondary'}
                                onPress={() => start('device.build')}
                            />
                        </View>
                    </Section>
                    <Text style={styles.note}>
                        Logs stream to the phone only for runs started here. Screen mirroring and
                        release signing keys stay on the desktop.
                    </Text>
                </>
            ) : (
                <Section title="Runs">
                    {runs.data === undefined ? (
                        runs.error ? (
                            <ErrorState error={runs.error} onRetry={runs.reload} />
                        ) : (
                            <LoadingState />
                        )
                    ) : runs.data.length === 0 ? (
                        <Text style={[styles.note, { paddingVertical: spacing.l }]}>
                            No runs or builds yet.
                        </Text>
                    ) : (
                        runs.data.map((run) => (
                            <Row
                                key={run.id}
                                label={`#${run.id} · ${run.kind === 'build' ? `build ${run.format ?? ''}` : `run on ${run.device}`}`}
                                detail={[
                                    run.variant,
                                    run.worktree,
                                    formatTime(run.started_at),
                                    isTracked(run.id) ? 'log on this phone' : undefined,
                                ]
                                    .filter(Boolean)
                                    .join(' · ')}
                                icon={run.kind === 'build' ? 'build' : 'play-circle'}
                                right={
                                    <View style={styles.runRight}>
                                        <Chip label={run.state} tone={stateTone(run.state)} />
                                        {isActive(run) && (
                                            <Chip
                                                label="Stop"
                                                tone="danger"
                                                icon="close"
                                                onPress={() => stop(run)}
                                            />
                                        )}
                                    </View>
                                }
                                onPress={() => {
                                    setRunId(run.id)
                                    setTab('log')
                                }}
                            />
                        ))
                    )}
                </Section>
            )}
        </Screen>
    )
}

export default DevicesScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        logPage: {
            flex: 1,
            padding: spacing.xl,
        },
        runHead: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.m,
        },
        runTitle: {
            flex: 1,
            color: color.text._100,
            fontSize: fontSize.m,
            fontWeight: '600',
        },
        runRight: {
            alignItems: 'flex-end',
            rowGap: spacing.s,
        },
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        label: {
            color: color.text._300,
            fontSize: fontSize.s,
        },
        chips: {
            flexDirection: 'row',
            flexWrap: 'wrap',
            gap: spacing.s,
        },
    })
}
