import AntDesign, { AntDesignIconName } from '@react-native-vector-icons/ant-design/static'
import React, { ReactNode } from 'react'
import {
    ActivityIndicator,
    RefreshControl,
    ScrollView,
    StyleProp,
    StyleSheet,
    Switch,
    Text,
    TextInputProps,
    TouchableOpacity,
    View,
    ViewStyle,
} from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import ThemedButton from '@components/buttons/ThemedButton'
import ThemedTextInput from '@components/input/ThemedTextInput'
import HeaderButton from '@components/views/HeaderButton'
import HeaderTitle from '@components/views/HeaderTitle'
import { useRelayStore } from '@lib/engine/Relay/RelayClient'
import { Theme } from '@lib/theme/ThemeManager'

export type HeaderAction = {
    icon: AntDesignIconName
    onPress: () => void
    disabled?: boolean
    /** Accessibility label. */
    label?: string
}

type ScreenProps = {
    title: string
    /** Icon buttons on the right of the header. */
    actions?: HeaderAction[]
    /** Pull to refresh; `refreshing` shows the spinner. */
    onRefresh?: () => void
    refreshing?: boolean
    /** False renders children in a plain flex view (for lists that scroll themselves). */
    scroll?: boolean
    /** Pinned under the content, e.g. a composer or a primary button. */
    footer?: ReactNode
    /** Show a "Not connected" line while offline. Default true. */
    offlineNote?: boolean
    children?: ReactNode
}

/**
 * A PC screen: the stack header (title, native back, optional icon actions), an offline
 * note, and a padded scrolling page. Every Relay feature screen starts from this.
 */
export const Screen: React.FC<ScreenProps> = ({
    title,
    actions,
    onRefresh,
    refreshing = false,
    scroll = true,
    footer,
    offlineNote = true,
    children,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const status = useRelayStore((state) => state.status)
    const error = useRelayStore((state) => state.error)
    const headerRight = () =>
        actions && actions.length > 0 ? (
            <View style={styles.headerActions}>
                {actions.map((action) => (
                    <TouchableOpacity
                        key={action.icon}
                        hitSlop={10}
                        disabled={action.disabled}
                        accessibilityLabel={action.label}
                        onPress={action.onPress}>
                        <AntDesign
                            name={action.icon}
                            size={22}
                            color={action.disabled ? color.text._600 : color.text._200}
                        />
                    </TouchableOpacity>
                ))}
            </View>
        ) : null
    const note =
        offlineNote && status !== 'online' ? (
            <Text style={styles.offline}>
                {status === 'connecting'
                    ? 'Connecting to the PC…'
                    : (error ?? 'Not connected to the PC.')}
            </Text>
        ) : null
    return (
        <SafeAreaView edges={['bottom']} style={styles.fill}>
            <HeaderTitle title={title} />
            <HeaderButton headerRight={headerRight} />
            {scroll ? (
                <ScrollView
                    contentContainerStyle={styles.page}
                    keyboardShouldPersistTaps="handled"
                    refreshControl={
                        onRefresh ? (
                            <RefreshControl
                                refreshing={refreshing}
                                onRefresh={onRefresh}
                                tintColor={color.text._300}
                                colors={[color.text._300]}
                            />
                        ) : undefined
                    }>
                    {note}
                    {children}
                </ScrollView>
            ) : (
                <View style={styles.fill}>
                    {note}
                    {children}
                </View>
            )}
            {footer}
        </SafeAreaView>
    )
}

type SectionProps = {
    title?: string
    /** A small link on the right of the title. */
    action?: { label: string; onPress: () => void }
    /** Draw the card behind the children. Default true. */
    card?: boolean
    style?: StyleProp<ViewStyle>
    children?: ReactNode
}

/** An uppercase heading over a rounded card that holds Rows, Fields or anything else. */
export const Section: React.FC<SectionProps> = ({
    title,
    action,
    card = true,
    style,
    children,
}) => {
    const styles = useStyles()
    return (
        <View style={[styles.section, style]}>
            {(title || action) && (
                <View style={styles.headingRow}>
                    <Text style={styles.heading}>{title}</Text>
                    {action && (
                        <TouchableOpacity hitSlop={8} onPress={action.onPress}>
                            <Text style={styles.link}>{action.label}</Text>
                        </TouchableOpacity>
                    )}
                </View>
            )}
            {card ? <View style={styles.card}>{children}</View> : children}
        </View>
    )
}

type RowProps = {
    label: string
    detail?: string
    icon?: AntDesignIconName
    /** Anything on the right: a Chip, a count, a button. */
    right?: ReactNode
    /** Defaults to shown when there is an `onPress`. */
    chevron?: boolean
    onPress?: () => void
    onLongPress?: () => void
    disabled?: boolean
    /** Red label, for delete and the like. */
    destructive?: boolean
    /** Monospace detail, for paths, shas and commands. */
    mono?: boolean
    detailLines?: number
}

/** One line of a Section: label, optional detail under it, and whatever goes right. */
export const Row: React.FC<RowProps> = ({
    label,
    detail,
    icon,
    right,
    chevron,
    onPress,
    onLongPress,
    disabled,
    destructive,
    mono,
    detailLines = 2,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const tappable = !!(onPress || onLongPress)
    return (
        <TouchableOpacity
            style={[styles.row, disabled && styles.disabled]}
            disabled={!tappable || disabled}
            onPress={onPress}
            onLongPress={onLongPress}>
            {icon && (
                <AntDesign
                    name={icon}
                    size={18}
                    color={destructive ? color.error._300 : color.text._400}
                />
            )}
            <View style={styles.rowText}>
                <Text numberOfLines={1} style={[styles.rowLabel, destructive && styles.danger]}>
                    {label}
                </Text>
                {!!detail && (
                    <Text
                        numberOfLines={detailLines}
                        style={[styles.rowDetail, mono && styles.mono]}>
                        {detail}
                    </Text>
                )}
            </View>
            {right}
            {(chevron ?? !!onPress) && <AntDesign name="right" size={14} color={color.text._500} />}
        </TouchableOpacity>
    )
}

export type Tone = 'neutral' | 'primary' | 'danger' | 'warn' | 'live'

const useTone = (tone: Tone) => {
    const { color } = Theme.useTheme()
    switch (tone) {
        case 'primary':
            return color.primary._700
        case 'danger':
            return color.error._300
        case 'warn':
            return color.quote
        case 'live':
            return '#2ec469'
        default:
            return color.text._300
    }
}

/** A small pill: a state, a label, a filter. Tappable when given `onPress`. */
export const Chip: React.FC<{
    label: string
    tone?: Tone
    selected?: boolean
    icon?: AntDesignIconName
    onPress?: () => void
}> = ({ label, tone = 'neutral', selected, icon, onPress }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const tint = useTone(tone)
    return (
        <TouchableOpacity
            disabled={!onPress}
            onPress={onPress}
            style={[
                styles.chip,
                { borderColor: selected ? tint : color.neutral._400 },
                selected && { backgroundColor: color.neutral._300 },
            ]}>
            {icon && <AntDesign name={icon} size={12} color={tint} />}
            <Text numberOfLines={1} style={[styles.chipText, { color: tint }]}>
                {label}
            </Text>
        </TouchableOpacity>
    )
}

/** A count bubble; renders nothing for 0 unless `showZero`. */
export const Badge: React.FC<{ value: number | string; tone?: Tone; showZero?: boolean }> = ({
    value,
    tone = 'neutral',
    showZero,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    const tint = useTone(tone)
    if (!showZero && (value === 0 || value === '0')) return null
    return (
        <Text
            style={[
                styles.badge,
                tone === 'neutral'
                    ? { color: color.text._300, backgroundColor: color.neutral._300 }
                    : { color: '#fff', backgroundColor: tint },
            ]}>
            {typeof value === 'number' && value > 99 ? '99+' : value}
        </Text>
    )
}

/** Nothing to show: a dashed box with an icon, a line of text and an optional action. */
export const EmptyState: React.FC<{
    icon?: AntDesignIconName
    title?: string
    text?: string
    action?: { label: string; onPress: () => void }
}> = ({ icon = 'inbox', title, text, action }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <View style={styles.empty}>
            <AntDesign name={icon} size={28} color={color.text._600} />
            {!!title && <Text style={styles.emptyTitle}>{title}</Text>}
            {!!text && <Text style={styles.note}>{text}</Text>}
            {action && (
                <ThemedButton label={action.label} variant="secondary" onPress={action.onPress} />
            )}
        </View>
    )
}

/** A failed load, with Retry. */
export const ErrorState: React.FC<{ error: Error | string; onRetry?: () => void }> = ({
    error,
    onRetry,
}) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <View style={[styles.empty, { borderColor: color.error._400 }]}>
            <AntDesign name="warning" size={24} color={color.error._300} />
            <Text style={[styles.note, styles.danger]}>
                {typeof error === 'string' ? error : error.message}
            </Text>
            {onRetry && <ThemedButton label="Retry" variant="secondary" onPress={onRetry} />}
        </View>
    )
}

export const LoadingState: React.FC<{ label?: string }> = ({ label }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <View style={styles.loading}>
            <ActivityIndicator color={color.text._300} />
            {!!label && <Text style={styles.note}>{label}</Text>}
        </View>
    )
}

/**
 * The three states of a `useBusQuery` in one line: spinner on the first load, the error
 * with Retry, `empty` when `isEmpty(data)`, else `children(data)`.
 */
export function QueryView<T>({
    query,
    isEmpty,
    empty,
    children,
}: {
    query: { data: T | undefined; error: Error | undefined; loading: boolean; reload: () => void }
    isEmpty?: (data: T) => boolean
    empty?: ReactNode
    children: (data: T) => ReactNode
}) {
    if (query.data === undefined) {
        if (query.error) return <ErrorState error={query.error} onRetry={query.reload} />
        return <LoadingState />
    }
    if (isEmpty?.(query.data)) return <>{empty ?? <EmptyState title="Nothing here" />}</>
    return (
        <>
            {query.error && <ErrorState error={query.error} onRetry={query.reload} />}
            {children(query.data)}
        </>
    )
}

type FieldProps = Omit<TextInputProps, 'value' | 'onChangeText'> & {
    label?: string
    description?: string
    value: string
    onChangeText: (text: string) => void
    /** Monospace input, for commit messages, paths and code. */
    mono?: boolean
    /** Visible lines for a multiline field. Default 4 when multiline. */
    lines?: number
}

/** A labelled text input; `multiline` grows it to `lines`. */
export const Field: React.FC<FieldProps> = ({
    label,
    description,
    mono,
    multiline,
    lines,
    style,
    ...rest
}) => {
    const styles = useStyles()
    const rows = multiline ? (lines ?? 4) : undefined
    return (
        <View style={styles.field}>
            <ThemedTextInput
                label={label}
                multiline={multiline}
                numberOfLines={rows}
                containerStyle={{ flex: 0 }}
                style={[
                    mono && styles.mono,
                    rows ? { minHeight: rows * 20 + 20 } : undefined,
                    style,
                ]}
                {...rest}
            />
            {!!description && <Text style={styles.fieldNote}>{description}</Text>}
        </View>
    )
}

/** A label (and optional description) with a switch on the right, for settings. */
export const SwitchRow: React.FC<{
    label: string
    description?: string
    value: boolean
    onChange: (value: boolean) => void
    disabled?: boolean
}> = ({ label, description, value, onChange, disabled }) => {
    const styles = useStyles()
    const { color } = Theme.useTheme()
    return (
        <View style={[styles.row, disabled && styles.disabled]}>
            <View style={styles.rowText}>
                <Text style={styles.rowLabel}>{label}</Text>
                {!!description && <Text style={styles.rowDetail}>{description}</Text>}
            </View>
            <Switch
                disabled={disabled}
                value={value}
                onValueChange={onChange}
                accessibilityLabel={label}
                trackColor={{ false: color.neutral._300, true: color.neutral._500 }}
                thumbColor={value ? color.primary._500 : color.neutral._400}
            />
        </View>
    )
}

/** A row of mutually exclusive options, e.g. columns or tabs within a screen. */
export function Segmented<T extends string>({
    options,
    value,
    onChange,
}: {
    options: { value: T; label: string }[]
    value: T
    onChange: (value: T) => void
}) {
    const styles = useStyles()
    return (
        <View style={styles.segmented}>
            {options.map((option) => {
                const active = option.value === value
                return (
                    <TouchableOpacity
                        key={option.value}
                        style={[styles.segment, active && styles.segmentActive]}
                        onPress={() => onChange(option.value)}>
                        <Text
                            numberOfLines={1}
                            style={[styles.segmentText, active && styles.segmentTextActive]}>
                            {option.label}
                        </Text>
                    </TouchableOpacity>
                )
            })}
        </View>
    )
}

/** A small uppercase caption, the same as a Section heading, for use inside cards. */
export const Caption: React.FC<{ children: ReactNode }> = ({ children }) => {
    const styles = useStyles()
    return <Text style={styles.heading}>{children}</Text>
}

/** Monospace text block, for diffs, logs and commands. */
export const Mono: React.FC<{ children: ReactNode; lines?: number }> = ({ children, lines }) => {
    const styles = useStyles()
    return (
        <Text numberOfLines={lines} selectable style={[styles.rowDetail, styles.mono]}>
            {children}
        </Text>
    )
}

export const useKitStyles = () => useStyles()

const useStyles = () => {
    const { color, spacing, fontSize } = Theme.useTheme()
    return StyleSheet.create({
        fill: {
            flex: 1,
        },
        page: {
            padding: spacing.xl,
            rowGap: spacing.xl,
            paddingBottom: spacing.xl3,
        },
        headerActions: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.xl2,
            marginRight: spacing.s,
        },
        offline: {
            color: color.error._300,
            fontSize: fontSize.s,
        },
        section: {
            rowGap: spacing.m,
        },
        headingRow: {
            flexDirection: 'row',
            alignItems: 'baseline',
            justifyContent: 'space-between',
            columnGap: spacing.m,
        },
        heading: {
            flex: 1,
            color: color.text._400,
            fontSize: fontSize.s,
            letterSpacing: 1,
            textTransform: 'uppercase',
        },
        link: {
            color: color.primary._700,
            fontSize: fontSize.s,
        },
        card: {
            backgroundColor: color.neutral._200,
            borderRadius: 16,
            paddingHorizontal: spacing.l,
            paddingVertical: spacing.xs,
        },
        row: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: spacing.l,
            paddingVertical: spacing.l,
        },
        rowText: {
            flex: 1,
            rowGap: 2,
        },
        rowLabel: {
            color: color.text._100,
            fontSize: fontSize.m,
        },
        rowDetail: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        mono: {
            fontFamily: 'monospace',
        },
        danger: {
            color: color.error._300,
        },
        disabled: {
            opacity: 0.5,
        },
        chip: {
            flexDirection: 'row',
            alignItems: 'center',
            columnGap: 4,
            paddingHorizontal: spacing.m,
            paddingVertical: 3,
            borderRadius: 999,
            borderWidth: 1,
        },
        chipText: {
            fontSize: fontSize.s,
        },
        badge: {
            minWidth: 22,
            textAlign: 'center',
            fontSize: fontSize.s,
            fontVariant: ['tabular-nums'],
            paddingHorizontal: 6,
            paddingVertical: 1,
            borderRadius: 10,
            overflow: 'hidden',
        },
        empty: {
            alignItems: 'center',
            rowGap: spacing.m,
            paddingVertical: spacing.xl2,
            paddingHorizontal: spacing.xl,
            borderRadius: 16,
            borderWidth: 1,
            borderStyle: 'dashed',
            borderColor: color.neutral._400,
        },
        emptyTitle: {
            color: color.text._200,
            fontSize: fontSize.l,
            fontWeight: '600',
        },
        note: {
            color: color.text._400,
            lineHeight: 20,
            textAlign: 'center',
        },
        loading: {
            alignItems: 'center',
            rowGap: spacing.m,
            paddingVertical: spacing.xl2,
        },
        field: {
            rowGap: spacing.s,
        },
        fieldNote: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        segmented: {
            flexDirection: 'row',
            padding: 3,
            borderRadius: 12,
            backgroundColor: color.neutral._200,
        },
        segment: {
            flex: 1,
            alignItems: 'center',
            paddingVertical: spacing.m,
            borderRadius: 10,
        },
        segmentActive: {
            backgroundColor: color.neutral._400,
        },
        segmentText: {
            color: color.text._400,
            fontSize: fontSize.s,
        },
        segmentTextActive: {
            color: color.text._100,
            fontWeight: '600',
        },
    })
}
