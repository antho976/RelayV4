import React, { useState } from 'react'
import { ScrollView, StyleSheet, Text, View } from 'react-native'

import ThemedButton from '@components/buttons/ThemedButton'
import {
    Caption,
    Chip,
    EmptyState,
    ErrorState,
    LoadingState,
    Mono,
    QueryView,
    relay,
    Row,
    Screen,
    Section,
    Sheet,
    SwitchRow,
    useBusQuery,
    useProjectParam,
    useRelayStore,
} from '@components/relay'
import { attempt } from '@components/relay/settings/common'
import { Plugin, PluginDoc } from '@components/relay/settings/types'
import { Theme } from '@lib/theme/ThemeManager'

type PluginDetail = {
    plugin: Plugin
    instructions: string
    docs: PluginDoc[]
    skill: PluginDoc | null
}

/** Something long to read in a sheet: a plugin's instructions, a doc or one skill's SKILL.md. */
type Reading = { title: string; body: string }

/**
 * The plugins bundled with Relay on the PC (the desktop's Tools → Plugins): what each brings —
 * skills, always-on agent instructions, docs, MCP servers — and the projects it is on for.
 * With `project_id`, the list's switches act on that project.
 */
const PluginsScreen = () => {
    const { projectId, project } = useProjectParam()
    const query = useBusQuery<Plugin[]>('plugin.list', projectId ? { project_id: projectId } : {}, {
        events: ['plugin.*'],
        select: (raw) => raw.plugins,
    })
    const [openId, setOpenId] = useState<string | undefined>(undefined)

    const toggle = async (plugin: Plugin, target: number, enabled: boolean) => {
        const next = await attempt(() =>
            relay.guarded<Plugin>('plugin.enable', {
                plugin_id: plugin.id,
                project_id: target,
                enabled: enabled,
            })
        )
        if (next) query.setData((list) => list?.map((item) => (item.id === next.id ? next : item)))
    }

    return (
        <Screen
            title={project ? `Plugins · ${project.name}` : 'Plugins'}
            onRefresh={query.reload}
            refreshing={query.loading}>
            <QueryView
                query={query}
                isEmpty={(plugins) => plugins.length === 0}
                empty={
                    <EmptyState
                        icon="appstore"
                        title="No plugins"
                        text="The Relay on this PC bundles none."
                    />
                }>
                {(plugins) => (
                    <Section title={`Bundled · ${plugins.length}`}>
                        {plugins.map((plugin) =>
                            projectId !== undefined ? (
                                <View key={plugin.id}>
                                    <SwitchRow
                                        label={`${plugin.name} ${plugin.version}`}
                                        description={plugin.summary}
                                        value={plugin.enabled_in.includes(projectId)}
                                        onChange={(value) => toggle(plugin, projectId, value)}
                                    />
                                    <Row
                                        label="Details"
                                        detail={`${plugin.skills.length} skills · ${plugin.mcp_servers.length} MCP servers`}
                                        onPress={() => setOpenId(plugin.id)}
                                    />
                                </View>
                            ) : (
                                <Row
                                    key={plugin.id}
                                    label={`${plugin.name} ${plugin.version}`}
                                    detail={plugin.summary}
                                    icon="appstore"
                                    right={
                                        plugin.enabled_in.length > 0 ? (
                                            <Chip
                                                label={`${plugin.enabled_in.length} on`}
                                                tone="live"
                                            />
                                        ) : undefined
                                    }
                                    onPress={() => setOpenId(plugin.id)}
                                />
                            )
                        )}
                    </Section>
                )}
            </QueryView>
            <PluginSheet
                pluginId={openId}
                onDismiss={() => setOpenId(undefined)}
                onToggle={toggle}
                plugin={query.data?.find((item) => item.id === openId)}
            />
        </Screen>
    )
}

export default PluginsScreen

/** One plugin in full, with a switch per project and each skill's SKILL.md on a tap. */
const PluginSheet: React.FC<{
    pluginId?: string
    plugin?: Plugin
    onDismiss: () => void
    onToggle: (plugin: Plugin, projectId: number, enabled: boolean) => void
}> = ({ pluginId, plugin, onDismiss, onToggle }) => {
    const styles = useStyles()
    const { spacing } = Theme.useTheme()
    const projects = useRelayStore((state) => state.projects)
    const detail = useBusQuery<PluginDetail>(
        'plugin.get',
        { plugin_id: pluginId },
        { enabled: pluginId !== undefined }
    )
    const [reading, setReading] = useState<Reading | undefined>(undefined)
    const [loadingSkill, setLoadingSkill] = useState<string | undefined>(undefined)
    const shown = plugin ?? detail.data?.plugin

    const readSkill = async (name: string) => {
        if (!pluginId) return
        setLoadingSkill(name)
        const result = await attempt(() =>
            relay.call<PluginDetail>('plugin.get', { plugin_id: pluginId, skill: name })
        )
        setLoadingSkill(undefined)
        if (result?.skill) setReading({ title: name, body: result.skill.body })
    }

    return (
        <Sheet
            visible={pluginId !== undefined}
            onDismiss={() => (reading ? setReading(undefined) : onDismiss())}>
            <ScrollView style={styles.scroll} contentContainerStyle={{ rowGap: spacing.l }}>
                {reading ? (
                    <>
                        <Text style={styles.title}>{reading.title}</Text>
                        <Mono>{reading.body}</Mono>
                        <ThemedButton
                            label="Back"
                            iconName="left"
                            variant="secondary"
                            onPress={() => setReading(undefined)}
                        />
                    </>
                ) : !shown ? (
                    detail.error ? (
                        <ErrorState error={detail.error} onRetry={detail.reload} />
                    ) : (
                        <LoadingState />
                    )
                ) : (
                    <>
                        <Text style={styles.title}>
                            {shown.name} <Text style={styles.version}>{shown.version}</Text>
                        </Text>
                        <View style={styles.chips}>
                            <Chip label={shown.category} />
                        </View>
                        <Text style={styles.text}>{shown.description || shown.summary}</Text>

                        {shown.skills.length > 0 && <Caption>Skills</Caption>}
                        {shown.skills.map((skill) => (
                            <Row
                                key={skill.name}
                                label={skill.name}
                                detail={skill.description}
                                icon="book"
                                right={
                                    loadingSkill === skill.name ? (
                                        <Text style={styles.version}>Loading…</Text>
                                    ) : undefined
                                }
                                onPress={() => readSkill(skill.name)}
                            />
                        ))}

                        {shown.mcp_servers.length > 0 && <Caption>MCP servers</Caption>}
                        {shown.mcp_servers.map((server) => (
                            <Row
                                key={server.name}
                                label={server.name}
                                detail={`${server.description}${
                                    server.tools.length ? `\nTools: ${server.tools.join(', ')}` : ''
                                }`}
                                icon="api"
                                detailLines={4}
                            />
                        ))}

                        {!!detail.data?.instructions && (
                            <Row
                                label="Agent instructions"
                                detail="What every agent of an enabled project reads in its brief"
                                icon="profile"
                                onPress={() =>
                                    setReading({
                                        title: 'Agent instructions',
                                        body: detail.data!.instructions,
                                    })
                                }
                            />
                        )}
                        {detail.data?.docs.map((doc) => (
                            <Row
                                key={doc.path}
                                label={doc.path}
                                icon="file-text"
                                onPress={() => setReading({ title: doc.path, body: doc.body })}
                            />
                        ))}

                        <Caption>Projects</Caption>
                        {projects.length === 0 && (
                            <Text style={styles.text}>No projects on this PC.</Text>
                        )}
                        {projects.map((item) => (
                            <SwitchRow
                                key={item.id}
                                label={item.name}
                                description={
                                    shown.suggested_for.includes(item.id)
                                        ? 'Looks like this plugin’s kind of project'
                                        : undefined
                                }
                                value={shown.enabled_in.includes(item.id)}
                                onChange={(value) => onToggle(shown, item.id, value)}
                            />
                        ))}
                    </>
                )}
            </ScrollView>
        </Sheet>
    )
}

const useStyles = () => {
    const { color, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        scroll: {
            maxHeight: 560,
        },
        title: {
            color: color.text._100,
            fontSize: fontSize.xl,
            fontWeight: '600',
        },
        version: {
            color: color.text._400,
            fontSize: fontSize.s,
            fontWeight: 'normal',
        },
        chips: {
            flexDirection: 'row',
        },
        text: {
            color: color.text._300,
            lineHeight: 20,
        },
    })
}
