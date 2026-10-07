import { useRouter } from 'expo-router'
import React, { useState } from 'react'
import { StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Chip,
    confirm,
    EmptyState,
    Field,
    isCancelled,
    QueryView,
    relay,
    RelayRequestError,
    Row,
    Screen,
    Section,
    useBusQuery,
    useProjectParam,
} from '@components/relay'
import { attempt, errorCode, errorText, settingsHref } from '@components/relay/settings/common'
import { Skill } from '@components/relay/settings/types'
import { Logger } from '@lib/state/Logger'
import { Theme } from '@lib/theme/ThemeManager'

/**
 * The PC's skills (the desktop's Tools → Skills): open one to read or edit it, add a new one,
 * or install SKILL.md files from a GitHub repository. With `project_id`, each row also says
 * whether that project's agents get it.
 */
const SkillsScreen = () => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const router = useRouter()
    const { projectId, project } = useProjectParam()
    const query = useBusQuery<Skill[]>('skill.list', projectId ? { project_id: projectId } : {}, {
        events: ['skill.*'],
        select: (raw) => raw.skills,
    })
    const [url, setUrl] = useState('')
    const [subdir, setSubdir] = useState('')
    const [installing, setInstalling] = useState(false)

    const open = (skill?: Skill) =>
        router.push(
            settingsHref('Skill', {
                ...(skill ? { skill_id: String(skill.id) } : {}),
                ...(projectId ? { project_id: String(projectId) } : {}),
            })
        )

    const install = async () => {
        const payload: Record<string, unknown> = { url: url.trim() }
        if (subdir.trim()) payload.subdir = subdir.trim()
        setInstalling(true)
        try {
            for (;;) {
                try {
                    const result = await relay.guarded<{ skills: Skill[] }>(
                        'skill.install',
                        payload
                    )
                    const names = result.skills.map((item) => item.name).join(', ')
                    Logger.infoToast(
                        result.skills.length === 1
                            ? `Installed ${names}`
                            : `Installed ${result.skills.length} skills`
                    )
                    setUrl('')
                    setSubdir('')
                    query.reload()
                    return
                } catch (e) {
                    // A visible skill already has that name: replacing it needs an explicit yes.
                    const installed =
                        errorCode(e) === 'skill.name_exists' && e instanceof RelayRequestError
                            ? (e.error.details as any)?.installed_skill
                            : undefined
                    if (!installed?.id || payload.replace_skill_id) throw e
                    const yes = await confirm({
                        title: `Replace ${installed.name}?`,
                        message: `${errorText(e)}\n\nReplacing it keeps its project switches and puts the downloaded SKILL.md in its place.`,
                        confirmLabel: 'Replace',
                        destructive: true,
                    })
                    if (!yes) return
                    payload.replace_skill_id = installed.id
                }
            }
        } catch (e) {
            if (!isCancelled(e)) Logger.errorToast(errorText(e))
        } finally {
            setInstalling(false)
        }
    }

    const remove = async (skill: Skill) => {
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
        if (done !== undefined) query.reload()
    }

    const ready = /^https?:\/\/\S+$/.test(url.trim()) && !installing

    return (
        <Screen
            title={project ? `Skills · ${project.name}` : 'Skills'}
            actions={[{ icon: 'plus', label: 'New skill', onPress: () => open() }]}
            onRefresh={query.reload}
            refreshing={query.loading}>
            <QueryView
                query={query}
                isEmpty={(skills) => skills.length === 0}
                empty={
                    <EmptyState
                        icon="book"
                        title="No skills yet"
                        text="A skill is a Markdown file of instructions agents load when a task calls for it."
                        action={{ label: 'New skill', onPress: () => open() }}
                    />
                }>
                {(skills) => (
                    <Section title={`On the PC · ${skills.length}`}>
                        {skills.map((skill) => (
                            <Row
                                key={skill.id}
                                label={skill.name}
                                detail={
                                    skill.description ||
                                    (skill.source_url ? skill.source_url : undefined)
                                }
                                icon="book"
                                right={
                                    projectId ? (
                                        skill.enabled_in.includes(projectId) ? (
                                            <Chip label="On" tone="live" />
                                        ) : (
                                            <Chip label="Off" />
                                        )
                                    ) : skill.source_url ? (
                                        <Chip label="GitHub" icon="github" />
                                    ) : undefined
                                }
                                onPress={() => open(skill)}
                                onLongPress={() => remove(skill)}
                            />
                        ))}
                    </Section>
                )}
            </QueryView>

            <Section title="Install from GitHub">
                <View style={{ rowGap: spacing.l, paddingVertical: spacing.l }}>
                    <Text style={styles.note}>
                        The PC downloads every SKILL.md in the repository (or in one folder of it)
                        and keeps them up to date when you install again.
                    </Text>
                    <Field
                        label="Repository URL"
                        value={url}
                        onChangeText={setUrl}
                        placeholder="https://github.com/owner/repo"
                        autoCapitalize="none"
                        autoCorrect={false}
                        keyboardType="url"
                        mono
                    />
                    <Field
                        label="Folder (optional)"
                        value={subdir}
                        onChangeText={setSubdir}
                        placeholder="skills/my-skill"
                        autoCapitalize="none"
                        autoCorrect={false}
                        mono
                    />
                    <View style={styles.actions}>
                        <ThemedButton
                            label={installing ? 'Installing…' : 'Install'}
                            iconName="cloud-download"
                            variant={ready ? 'primary' : 'disabled'}
                            onPress={install}
                        />
                    </View>
                </View>
            </Section>
            <Text style={styles.note}>Long-press a skill to delete it.</Text>
        </Screen>
    )
}

export default SkillsScreen

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
