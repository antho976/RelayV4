import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    confirm,
    EmptyState,
    Field,
    QueryView,
    relay,
    Row,
    Screen,
    Section,
    Segmented,
    Sheet,
    useBusQuery,
    useProjectParam,
} from '@components/relay'
import { useSheetStyles } from '@components/relay/Sheet'
import {
    attempt,
    Choice,
    ChoiceSheet,
    Module,
    ModuleHeader,
    moduleHref,
    ModuleSummary,
    offerUndo,
    Priority,
    PRIORITIES,
    shortTime,
    UndoBar,
    usePieceStyles,
} from '@components/relay/tasks'
import { Theme } from '@lib/theme/ThemeManager'

const detail = (module: ModuleSummary) => {
    const total = Object.values(module.counts).reduce((sum, n) => sum + (n ?? 0), 0)
    const parts = [
        module.priority,
        `${total} task${total === 1 ? '' : 's'}`,
        `${Math.round(module.progress_pct)}% done`,
    ]
    if (module.completed_at) parts.push(`completed ${shortTime(module.completed_at)}`)
    return parts.join(' · ')
}

/** The project's modules: open ones first, completed ones folded away; create one here. */
const ModulesScreen = () => {
    const router = useRouter()
    const styles = useStyles()
    const pieces = usePieceStyles()
    const sheet = useSheetStyles()
    const { projectId, project } = useProjectParam()
    const query = useBusQuery<{ modules: ModuleSummary[]; header: ModuleHeader }>(
        'module.list',
        { project_id: projectId, include_archived: true },
        { events: ['module.*', 'task.*'], projectId: projectId, enabled: projectId !== undefined }
    )
    const [showArchived, setShowArchived] = useState(false)
    const [creating, setCreating] = useState(false)
    const [name, setName] = useState('')
    const [priority, setPriority] = useState<Priority>('medium')
    const [busy, setBusy] = useState(false)
    const [menu, setMenu] = useState<{ title: string; choices: Choice[] } | undefined>(undefined)

    const create = async () => {
        if (projectId === undefined || !name.trim()) return
        setBusy(true)
        const made = await attempt(() =>
            relay.guarded<Module>('module.create', {
                project_id: projectId,
                name: name.trim(),
                priority: priority,
            })
        )
        setBusy(false)
        if (!made) return
        setCreating(false)
        setName('')
        setPriority('medium')
        query.reload()
        router.push(moduleHref(made.id))
    }

    const remove = async (module: ModuleSummary) => {
        const yes = await confirm({
            title: `Delete ${module.name}?`,
            message: 'Its tasks stay on the board, without a module.',
            confirmLabel: 'Delete',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(() => relay.guarded('module.delete', { module_id: module.id }))
        if (done === undefined) return
        query.reload()
        offerUndo(`${module.name} deleted`, () =>
            relay.guarded('module.restore', { module_id: module.id })
        )
    }

    const openMenu = (module: ModuleSummary) =>
        setMenu({
            title: module.name,
            choices: [
                {
                    label: 'Open',
                    icon: 'appstore',
                    onPress: () => router.push(moduleHref(module.id)),
                },
                module.completed_at
                    ? {
                          label: 'Reopen',
                          icon: 'undo',
                          onPress: () =>
                              attempt(
                                  () => relay.guarded('module.reopen', { module_id: module.id }),
                                  'Reopened'
                              ),
                      }
                    : {
                          label: 'Complete',
                          icon: 'check',
                          onPress: () =>
                              attempt(
                                  () => relay.guarded('module.complete', { module_id: module.id }),
                                  'Completed'
                              ),
                      },
                {
                    label: 'Delete',
                    icon: 'delete',
                    destructive: true,
                    onPress: () => remove(module),
                },
            ],
        })

    if (projectId === undefined) {
        return (
            <Screen title="Modules">
                <EmptyState icon="appstore" title="No project" text="Open a project first." />
            </Screen>
        )
    }

    const row = (module: ModuleSummary) => (
        <Row
            key={module.id}
            label={module.icon ? `${module.icon} ${module.name}` : module.name}
            detail={detail(module)}
            icon={module.completed_at ? 'check-circle' : 'appstore'}
            onPress={() => router.push(moduleHref(module.id))}
            onLongPress={() => openMenu(module)}
        />
    )

    return (
        <Screen
            title={project ? `Modules · ${project.name}` : 'Modules'}
            onRefresh={query.reload}
            refreshing={query.loading && query.data !== undefined}
            footer={<UndoBar />}
            actions={[{ icon: 'plus', label: 'New module', onPress: () => setCreating(true) }]}>
            <QueryView query={query}>
                {({ modules, header }) => {
                    const open = modules.filter((m) => !m.completed_at)
                    const archived = modules.filter((m) => !!m.completed_at)
                    return (
                        <>
                            <View style={styles.stats}>
                                <Stat value={header.count} label="modules" />
                                <Stat value={header.in_flight} label="in flight" />
                                <Stat value={header.completed} label="completed" />
                                <Stat
                                    value={`${Math.round(header.completion_pct)}%`}
                                    label="done"
                                />
                            </View>
                            {open.length === 0 ? (
                                <EmptyState
                                    icon="appstore"
                                    title="No open modules"
                                    text="A module groups the tasks of one release outcome."
                                    action={{
                                        label: 'New module',
                                        onPress: () => setCreating(true),
                                    }}
                                />
                            ) : (
                                <Section title={`Open · ${open.length}`}>{open.map(row)}</Section>
                            )}
                            {archived.length > 0 && (
                                <Section
                                    title={`Completed · ${archived.length}`}
                                    action={{
                                        label: showArchived ? 'Hide' : 'Show',
                                        onPress: () => setShowArchived((v) => !v),
                                    }}
                                    card={showArchived}>
                                    {showArchived ? archived.map(row) : null}
                                </Section>
                            )}
                            <Text style={styles.hint}>Long-press a module for more.</Text>
                        </>
                    )
                }}
            </QueryView>
            <Sheet visible={creating} onDismiss={() => setCreating(false)}>
                <View style={[sheet.body, pieces.shrink]}>
                    <Text style={sheet.title}>New module</Text>
                    <Field
                        label="Name"
                        value={name}
                        onChangeText={setName}
                        placeholder="Release outcome"
                        autoFocus
                        onSubmitEditing={create}
                    />
                    <Text style={pieces.label}>Priority</Text>
                    <Segmented options={PRIORITIES} value={priority} onChange={setPriority} />
                    <View style={sheet.actions}>
                        <ThemedButton
                            label="Cancel"
                            variant="secondary"
                            onPress={() => setCreating(false)}
                        />
                        <ThemedButton
                            label={busy ? 'Creating…' : 'Create'}
                            variant={busy || !name.trim() ? 'disabled' : 'primary'}
                            onPress={busy || !name.trim() ? undefined : create}
                        />
                    </View>
                </View>
            </Sheet>
            <ChoiceSheet
                title={menu?.title}
                choices={menu?.choices}
                onDismiss={() => setMenu(undefined)}
            />
        </Screen>
    )
}

const Stat: React.FC<{ value: number | string; label: string }> = ({ value, label }) => {
    const styles = useStyles()
    return (
        <View style={styles.stat}>
            <Text style={styles.statValue}>{value}</Text>
            <Text style={styles.statLabel}>{label}</Text>
        </View>
    )
}

export default ModulesScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        stats: {
            flexDirection: 'row',
            columnGap: spacing.m,
        },
        stat: {
            flex: 1,
            alignItems: 'center',
            paddingVertical: spacing.l,
            borderRadius: 14,
            backgroundColor: color.neutral._200,
        },
        statValue: {
            color: color.text._100,
            fontSize: fontSize.xl,
            fontWeight: '600',
        },
        statLabel: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        hint: {
            color: color.text._500,
            fontSize: fontSize.s,
            textAlign: 'center',
        },
    })
}
