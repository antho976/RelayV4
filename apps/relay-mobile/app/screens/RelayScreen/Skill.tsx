import { useLocalSearchParams, useRouter } from 'expo-router'
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
    Screen,
    Section,
    SwitchRow,
    useBusQuery,
    useRelayStore,
} from '@components/relay'
import { attempt, formatTime } from '@components/relay/settings/common'
import { Skill } from '@components/relay/settings/types'
import { Theme } from '@lib/theme/ThemeManager'

const TEMPLATE = `---
name:
description:
---

`

/**
 * One skill: its name and Markdown body to read and edit, the projects whose agents get it,
 * and Delete. Without `skill_id` it is a new skill (enabled in every project, as on the PC).
 */
const SkillScreen = () => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const router = useRouter()
    const params = useLocalSearchParams<{ skill_id?: string }>()
    const parsed = params.skill_id ? Number(params.skill_id) : NaN
    const [skillId, setSkillId] = useState<number | undefined>(
        Number.isFinite(parsed) ? parsed : undefined
    )
    const creating = skillId === undefined
    const projects = useRelayStore((state) => state.projects)
    // There is no skill.get: the list carries bodies, and a skill.changed reloads it.
    const query = useBusQuery<Skill | null>(
        'skill.list',
        {},
        {
            enabled: !creating,
            events: ['skill.*'],
            select: (raw) => raw.skills.find((item: Skill) => item.id === skillId) ?? null,
        }
    )
    const skill = query.data ?? undefined
    // Unsaved edits; without any, the form shows the skill as the PC has it now.
    const [edit, setEdit] = useState<{ name: string; body: string } | undefined>(
        creating ? { name: '', body: TEMPLATE } : undefined
    )
    const [saving, setSaving] = useState(false)
    const name = edit?.name ?? skill?.name ?? ''
    const body = edit?.body ?? skill?.body ?? ''
    const setName = (text: string) => setEdit({ name: text, body: body })
    const setBody = (text: string) => setEdit({ name: name, body: text })

    if (!creating && !skill) {
        return (
            <Screen title="Skill">
                {query.error ? (
                    <ErrorState error={query.error} onRetry={query.reload} />
                ) : query.data === null ? (
                    <EmptyState
                        icon="book"
                        title="Skill not found"
                        text="It may have been deleted."
                    />
                ) : (
                    <LoadingState />
                )}
            </Screen>
        )
    }

    const dirty = creating
        ? name.trim().length > 0
        : !!skill && (name.trim() !== skill.name || body !== skill.body)

    const save = async () => {
        if (!dirty || !name.trim()) return
        setSaving(true)
        const next = await attempt(
            () =>
                creating
                    ? relay.guarded<Skill>('skill.create', { name: name.trim(), body: body })
                    : relay.guarded<Skill>('skill.update', {
                          skill_id: skillId,
                          ...(name.trim() !== skill!.name ? { name: name.trim() } : {}),
                          ...(body !== skill!.body ? { body: body } : {}),
                      }),
            creating ? 'Skill created' : 'Skill saved'
        )
        setSaving(false)
        if (!next) return
        setEdit(undefined)
        query.setData(next)
        if (creating) setSkillId(next.id)
    }

    const remove = async () => {
        if (!skill) return
        const yes = await confirm({
            title: `Delete ${skill.name}?`,
            message: 'Agents stop getting it in every project. Undo is on the PC.',
            confirmLabel: 'Delete',
            destructive: true,
        })
        if (!yes) return
        const done = await attempt(
            () => relay.guarded('skill.delete', { skill_id: skill.id }),
            `${skill.name} deleted`
        )
        if (done !== undefined) router.back()
    }

    const toggle = async (projectId: number, enabled: boolean) => {
        if (!skill) return
        const next = await attempt(() =>
            relay.guarded<Skill>('skill.enable', {
                skill_id: skill.id,
                project_id: projectId,
                enabled: enabled,
            })
        )
        if (next) query.setData(next)
    }

    return (
        <Screen
            title={creating ? 'New skill' : (skill?.name ?? 'Skill')}
            actions={creating ? undefined : [{ icon: 'delete', label: 'Delete', onPress: remove }]}>
            <View style={{ rowGap: spacing.l }}>
                <Field
                    label="Name"
                    value={name}
                    onChangeText={setName}
                    autoCapitalize="none"
                    autoCorrect={false}
                />
                <Field
                    label="Instructions (Markdown)"
                    value={body}
                    onChangeText={setBody}
                    multiline
                    lines={16}
                    mono
                    autoCapitalize="none"
                    autoCorrect={false}
                    textAlignVertical="top"
                />
                {!!skill?.source_url && (
                    <Text style={styles.note} selectable>
                        Installed from {skill.source_url}
                        {skill.source_path ? ` · ${skill.source_path}` : ''}
                        {skill.revision ? ` @ ${skill.revision.slice(0, 10)}` : ''}. Installing
                        again replaces edits made here.
                    </Text>
                )}
                {!!skill && <Text style={styles.note}>Updated {formatTime(skill.updated_at)}</Text>}
                <View style={styles.actions}>
                    {!creating && dirty && (
                        <ThemedButton
                            label="Revert"
                            variant="secondary"
                            onPress={() => setEdit(undefined)}
                        />
                    )}
                    <ThemedButton
                        label={saving ? 'Saving…' : creating ? 'Create' : 'Save'}
                        variant={dirty && name.trim() && !saving ? 'primary' : 'disabled'}
                        onPress={save}
                    />
                </View>
            </View>
            {creating ? (
                <Text style={styles.note}>
                    A new skill is on in every project; switch it off per project once it exists.
                </Text>
            ) : (
                <Section title="Projects">
                    {projects.length === 0 ? (
                        <Text style={[styles.note, { paddingVertical: spacing.l }]}>
                            No projects on this PC.
                        </Text>
                    ) : (
                        projects.map((project) => (
                            <SwitchRow
                                key={project.id}
                                label={project.name}
                                value={!!skill?.enabled_in.includes(project.id)}
                                onChange={(value) => toggle(project.id, value)}
                            />
                        ))
                    )}
                </Section>
            )}
        </Screen>
    )
}

export default SkillScreen

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        note: {
            color: color.text._400,
            fontSize: fontSize.s,
            lineHeight: 18,
        },
        actions: {
            flexDirection: 'row',
            justifyContent: 'flex-end',
            columnGap: spacing.m,
        },
    })
}
