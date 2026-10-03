import { useRouter } from 'expo-router'
import React from 'react'
import { View } from 'react-native'

import {
    Chip,
    ErrorState,
    LoadingState,
    relay,
    Row,
    Section,
    SwitchRow,
    useBusQuery,
} from '@components/relay'

import { attempt, settingsHref } from './common'
import { Plugin, Skill, skillSummary } from './types'

/**
 * The skills and plugins one project's agents get, each with its switch (the desktop's
 * per-project Skills and Plugins pages), and links to manage the PC's whole set.
 */
const ProjectExtensions: React.FC<{ projectId: number }> = ({ projectId }) => {
    const router = useRouter()
    const skills = useBusQuery<Skill[]>(
        'skill.list',
        { project_id: projectId },
        { events: ['skill.*'], select: (raw) => raw.skills }
    )
    const plugins = useBusQuery<Plugin[]>(
        'plugin.list',
        { project_id: projectId },
        { events: ['plugin.*'], select: (raw) => raw.plugins }
    )
    const params = { project_id: String(projectId) }

    const toggleSkill = async (skill: Skill, enabled: boolean) => {
        const next = await attempt(() =>
            relay.guarded<Skill>('skill.enable', {
                skill_id: skill.id,
                project_id: projectId,
                enabled: enabled,
            })
        )
        if (next) skills.setData((list) => list?.map((item) => (item.id === next.id ? next : item)))
    }

    const togglePlugin = async (plugin: Plugin, enabled: boolean) => {
        const next = await attempt(() =>
            relay.guarded<Plugin>('plugin.enable', {
                plugin_id: plugin.id,
                project_id: projectId,
                enabled: enabled,
            })
        )
        if (next)
            plugins.setData((list) => list?.map((item) => (item.id === next.id ? next : item)))
    }

    return (
        <>
            <Section
                title="Skills"
                action={{
                    label: 'Manage',
                    onPress: () => router.push(settingsHref('Skills', params)),
                }}>
                {skills.data === undefined ? (
                    skills.error ? (
                        <ErrorState error={skills.error} onRetry={skills.reload} />
                    ) : (
                        <LoadingState />
                    )
                ) : skills.data.length === 0 ? (
                    <Row
                        label="No skills on this PC"
                        detail="Add or install one to give agents instructions for a kind of work."
                        onPress={() => router.push(settingsHref('Skills', params))}
                    />
                ) : (
                    skills.data.map((skill) => (
                        <SwitchRow
                            key={skill.id}
                            label={skill.name}
                            description={skillSummary(skill.body) || undefined}
                            value={skill.enabled_in.includes(projectId)}
                            onChange={(value) => toggleSkill(skill, value)}
                        />
                    ))
                )}
            </Section>
            <Section
                title="Plugins"
                action={{
                    label: 'Details',
                    onPress: () => router.push(settingsHref('Plugins', params)),
                }}>
                {plugins.data === undefined ? (
                    plugins.error ? (
                        <ErrorState error={plugins.error} onRetry={plugins.reload} />
                    ) : (
                        <LoadingState />
                    )
                ) : plugins.data.length === 0 ? (
                    <Row label="No plugins" detail="This PC's Relay bundles none." />
                ) : (
                    plugins.data.map((plugin) => (
                        <React.Fragment key={plugin.id}>
                            <SwitchRow
                                label={plugin.name}
                                description={plugin.summary}
                                value={plugin.enabled_in.includes(projectId)}
                                onChange={(value) => togglePlugin(plugin, value)}
                            />
                            {plugin.suggested_for.includes(projectId) &&
                                !plugin.enabled_in.includes(projectId) && (
                                    <View style={{ flexDirection: 'row', paddingBottom: 8 }}>
                                        <Chip label="Suggested for this project" tone="primary" />
                                    </View>
                                )}
                        </React.Fragment>
                    ))
                )}
            </Section>
        </>
    )
}

export default ProjectExtensions
