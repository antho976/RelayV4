/**
 * The PC tab's foundation: what every Relay feature screen builds on. Import from
 * '@components/relay'. PC screens use literal English strings (no i18n keys).
 *
 * Talking to the PC — `relay` from '@lib/engine/Relay/RelayClient' (re-exported here):
 *   relay.call<T>(op, payload?, { timeoutMs? }?): Promise<T>
 *       Any op, actor `user`. Per-op timeout from `timeoutFor(op)` (20 s default; clone,
 *       spawn, push, build and the like are longer). Rejects with RelayRequestError
 *       (`.error: BusError` — kind/code/message/hint/details) or a plain Error (offline,
 *       timed out: the outcome is then unknown, refresh before retrying).
 *   relay.guarded<T>(op, payload?, opts?): Promise<T>
 *       Use for EVERY mutation (git.commit, git.push, file.write, task.delete, …). If the
 *       guardrail holds it, the global HoldSheet asks the person (what is held and why);
 *       Allow runs guardrail.confirm and resolves with the original op's result, Deny runs
 *       guardrail.reject and throws RelayCancelledError — check `isCancelled(e)` and stay
 *       quiet. A confirmed action that still fails throws RelayRequestError.
 *   relay.onStream(stream, key, (frame: StreamFrame) => void): () => void
 *       Non-PTY data frames: 'logcat' keyed by run_id (only for runs this phone started).
 *   relay.watchResources(on) — refcounted; prefer the hook below.
 *   useRelayStore — status, projects, workspaces, sessions, holds, notifications, inReview.
 *
 * Hooks:
 *   useBusQuery<T>(op, payload, { events?, projectId?, enabled?, select?, refetchOnFocus?,
 *       debounceMs? }): { data, error, loading, reload, setData }
 *       Loads when online, reloads on focus, on reconnect and on matching events (debounced).
 *       `payload` is compared by JSON, so an inline object is fine.
 *   useRelayEvent(patterns, handler, { debounceMs?, projectId?, enabled? })
 *       Patterns use the engine's syntax: 'notes.changed', 'git.*', '*'. Subscribed on
 *       connect: EVENT_PATTERNS (session guardrail notify task project workspace mailbox
 *       notes module git file worktree overlap integration usage run device avd skill plugin
 *       provider settings github — all `.*`). Latest handler always called; no deps needed.
 *   useRelayStream(stream, key | undefined, handler) — onStream while mounted.
 *   useResourceSamples(handler, enabled?) — resource.sample while the screen is focused.
 *   useRelayOnline(): boolean
 *   useProjectParam(): { projectId?: number, project?: RelayProject } — the `project_id` param.
 *   relayHref(page, params?) — `router.push(relayHref('Git', { project_id: String(id) }))`.
 *
 * UI (Kit.tsx; theme via Theme.useTheme(), buttons are ThemedButton):
 *   <Screen title actions?={[{icon, onPress, disabled?, label?}]} onRefresh? refreshing?
 *       scroll?={true} footer? offlineNote?={true}> — header with native back, padded page.
 *   <Section title? action?={{label, onPress}} card?={true}> — heading over a card.
 *   <Row label detail? icon? right? chevron? onPress? onLongPress? disabled? destructive? mono?>
 *   <Chip label tone?='neutral'|'primary'|'danger'|'warn'|'live' selected? icon? onPress?>
 *   <Badge value tone? showZero?>   <Caption>   <Mono lines?>
 *   <EmptyState icon? title? text? action?>  <ErrorState error onRetry?>  <LoadingState label?>
 *   <QueryView query={q} isEmpty? empty?>{(data) => …}</QueryView>
 *   <Field label? value onChangeText multiline? lines? mono? description? …TextInputProps>
 *   <SwitchRow label description? value onChange disabled?>
 *   <Segmented options={[{value, label}]} value onChange>
 *   <Sheet visible onDismiss> — controlled bottom sheet.
 *   await confirm({ title, message?, confirmLabel?, cancelLabel?, destructive?, body? })
 *       → boolean, in a bottom sheet (ConfirmHost is mounted in app/_layout.tsx).
 *   <HoldDetails hold request? message?> — the facts of a guardrail hold.
 *
 * Routes: app/screens/RelayScreen/<Page>.tsx, params as strings ({ project_id }).
 * Project.tsx is the per-project hub that links every feature screen.
 */
export * from './hooks'
export * from './Kit'
export { confirm, ConfirmHost, Sheet } from './Sheet'
export type { ConfirmOptions } from './Sheet'
export { default as HoldSheet, HoldDetails, holdFacts } from './HoldSheet'
export {
    EVENT_PATTERNS,
    isCancelled,
    matchEvent,
    relay,
    RelayCancelledError,
    RelayRequestError,
    timeoutFor,
    useRelayStore,
} from '@lib/engine/Relay/RelayClient'
export type {
    BusError,
    BusEvent,
    HeldRequest,
    RelayHold,
    RelayHoldFull,
    RelayProject,
    RequestOptions,
    StreamFrame,
} from '@lib/engine/Relay/RelayClient'
