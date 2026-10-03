import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    confirm,
    EmptyState,
    ErrorState,
    Field,
    LoadingState,
    relay,
    relayHref,
    Row,
    Screen,
    Section,
    SwitchRow,
    useBusQuery,
    useProjectParam,
} from '@components/relay'
import { attempt } from '@components/relay/settings/common'
import GuardrailEditor from '@components/relay/settings/GuardrailEditor'
import ProjectExtensions from '@components/relay/settings/ProjectExtensions'
import { ProjectFull } from '@components/relay/settings/types'
import { Theme } from '@lib/theme/ThemeManager'

type Draft = { name: string; build_cmd: string; run_cmd: string; base_branch: string }

const toDraft = (project: ProjectFull): Draft => ({
    name: project.name,
    build_cmd: project.build_cmd ?? '',
    run_cmd: project.run_cmd ?? '',
    base_branch: project.base_branch,
})

/**
 * One project's settings (the desktop's project settings dialog): its name, the commands an
 * integration build and a device run use, the branch agents start from, pinning, the skills
 * and plugins its agents get, its own guardrail values, and forgetting it.
 */
const ProjectSettingsScreen = () => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const router = useRouter()
    const { projectId } = useProjectParam()
    const query = useBusQuery<ProjectFull>(
        'project.get',
        { project_id: projectId },
        { enabled: projectId !== undefined, events: ['project.changed'], projectId: projectId }
    )
    const project = query.data
    // Unsaved edits; without any, the form shows the project as the PC has it now.
    const [edit, setEdit] = useState<Draft | undefined>(undefined)
    const [saving, setSaving] = useState(false)
    const [guardrails, setGuardrails] = useState(false)

    if (projectId === undefined) {
        return (
            <Screen title="Project settings">
                <EmptyState icon="setting" title="No project" text="Open a project first." />
            </Screen>
        )
    }

    if (!project) {
        return (
            <Screen title="Project settings">
                {query.error ? (
                    <ErrorState error={query.error} onRetry={query.reload} />
                ) : (
                    <LoadingState />
                )}
            </Screen>
        )
    }

    const draft = edit ?? toDraft(project)
    const patch: Record<string, unknown> = {}
    if (draft.name.trim() && draft.name.trim() !== project.name) patch.name = draft.name.trim()
    if (draft.base_branch.trim() && draft.base_branch.trim() !== project.base_branch)
        patch.base_branch = draft.base_branch.trim()
    // An empty command goes back to Relay's default (null), as on the desktop.
    const command = (text: string) => (text.trim() ? text.trim() : null)
    if (command(draft.build_cmd) !== project.build_cmd) patch.build_cmd = command(draft.build_cmd)
    if (command(draft.run_cmd) !== project.run_cmd) patch.run_cmd = command(draft.run_cmd)
    const dirty = Object.keys(patch).length > 0
    const set = (key: keyof Draft) => (text: string) => setEdit({ ...draft, [key]: text })

    const apply = async (change: Record<string, unknown>, success?: string) => {
        const next = await attempt(
            () =>
                relay.guarded<ProjectFull>('project.update', { project_id: projectId, ...change }),
            success
        )
        if (next) {
            query.setData(next)
            relay.refreshProjects().catch(() => {})
        }
        return next
    }

    const save = async () => {
        if (!dirty) return
        setSaving(true)
        if (await apply(patch, 'Project saved')) setEdit(undefined)
        setSaving(false)
    }

    const remove = async () => {
        const yes = await confirm({
            title: `Remove ${project.name}?`,
            message:
                'Relay forgets this project: its board, notes and history on the PC go with it. The folder on disk is not touched. A project with live sessions cannot be removed.',
            confirmLabel: 'Remove',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(
            () => relay.guarded('project.remove', { project_id: projectId }),
            `${project.name} removed`
        )
        if (done === undefined) return
        relay.refreshProjects().catch(() => {})
        if (router.canDismiss()) router.dismissAll()
        else router.back()
    }

    return (
        <Screen title={`${project.name} · settings`} onRefresh={query.reload}>
            <Section title="Project">
                <View style={{ rowGap: spacing.l, paddingVertical: spacing.l }}>
                    <Field label="Name" value={draft.name} onChangeText={set('name')} />
                    <Field
                        label="Base branch"
                        value={draft.base_branch}
                        onChangeText={set('base_branch')}
                        autoCapitalize="none"
                        autoCorrect={false}
                        mono
                        description="The branch new agent worktrees start from and integration merges into."
                    />
                    <Field
                        label="Build command"
                        value={draft.build_cmd}
                        onChangeText={set('build_cmd')}
                        autoCapitalize="none"
                        autoCorrect={false}
                        mono
                        placeholder="Automatic"
                        description="Run by an integration build. Empty uses Relay's default."
                    />
                    <Field
                        label="Run command"
                        value={draft.run_cmd}
                        onChangeText={set('run_cmd')}
                        autoCapitalize="none"
                        autoCorrect={false}
                        mono
                        placeholder="Automatic"
                        description="Run for Run on device. Empty uses Gradle's install and launch."
                    />
                    <Text style={styles.path} selectable>
                        {project.path}
                    </Text>
                    <View style={styles.actions}>
                        {dirty && (
                            <ThemedButton
                                label="Revert"
                                variant="secondary"
                                onPress={() => setEdit(undefined)}
                            />
                        )}
                        <ThemedButton
                            label={saving ? 'Saving…' : 'Save'}
                            variant={dirty && !saving ? 'primary' : 'disabled'}
                            onPress={save}
                        />
                    </View>
                </View>
                <SwitchRow
                    label="Pinned"
                    description="Pinned projects come first in the sidebar, on the phone and on the PC."
                    value={project.pinned}
                    onChange={(value) => apply({ pinned: value })}
                />
            </Section>

            <ProjectExtensions projectId={projectId} />

            <Section title="Guardrails">
                <Row
                    label={
                        guardrails ? 'Hide guardrail values' : 'Guardrail values for this project'
                    }
                    detail="Caps, destructive-write thresholds, protected paths, denied commands"
                    icon="safety"
                    chevron={false}
                    onPress={() => setGuardrails(!guardrails)}
                />
                {guardrails && <GuardrailEditor projectId={projectId} />}
                <Row
                    label="Holds and overlaps"
                    icon="exception"
                    onPress={() =>
                        router.push(relayHref('Guardrails', { project_id: String(projectId) }))
                    }
                />
            </Section>

            <Section title="Danger zone">
                <Row
                    label="Remove project"
                    detail="Forget it in Relay; the folder stays on disk."
                    icon="delete"
                    destructive
                    chevron={false}
                    onPress={remove}
                />
            </Section>
        </Screen>
    )
}

export default ProjectSettingsScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        path: {
            color: color.text._400,
            fontSize: fontSize.s,
            fontFamily: 'monospace',
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
    })
}
