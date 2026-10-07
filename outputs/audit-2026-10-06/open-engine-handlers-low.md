# Audit findings left open: engine handler Lows

Scope: RA-348..424 (Low defects in `crates/relay-core/src/handlers/*`), fixed on branch
`relay/jade-yak` (2026-10-07). Everything in that range is fixed or was already fixed, except the
parts below. Each says why it is open and what closing it would take. Most need a file another
session held while this range was worked (`worktree.rs`, `skills.rs`, `guardrail.rs`,
`awareness.rs`, `audit.rs`, `hooks.rs`, `github.rs`, `module.rs`, `relay-bus` op files, BUS.md).

Already fixed before this pass (no change made): RA-363, RA-365, RA-366, RA-383; the device half
of RA-372 (every request-path call in `device.rs` was already bounded).

## Needs a payload or schema change

- **RA-405: worktree removal has no dirty check.** `session.close`, `project.remove` and
  `discard_restorable` still force-delete uncommitted work. Closing it needs a
  `discard_changes: Option<bool>` on `CloseIn` and on the discard payload in
  `relay-bus/src/ops/session.rs`, a `worktree.dirty` refusal without it, and a decision on whether
  `session.close` should keep the worktree by default (BUS.md, every client).
- **RA-413: agent writes have no optimistic check.** Adding `expected` to
  `task.changelog.write` and the note writes changes their payloads in `relay-bus` and the MCP tool
  descriptions; refusing a write when the row changed since the agent last read it needs read
  tracking, a redesign.
- **RA-416: undoing task.approve is a plain task.move.** A real inverse needs its own op (e.g.
  `task.unapprove {task_id, column, position, state, sha}`) that also removes the commit link;
  `task.move`'s payload rejects unknown fields.
- **RA-414, rest: an undone detach gets a new id and a second file copy.** Name and mime now
  survive. Keeping the id needs a soft-delete migration on `attachments` and a restore op.
- **RA-373, rest: clean_merged does not report failed deletes to the caller.** It now deletes
  every branch it can and logs the rest; returning them needs a `failed` field on
  `CleanMergedOut` (`relay-bus/src/ops/git.rs`).

## Needs a file another session holds

- **RA-370: worktree.disk walks build directories twice.** Needs one single-pass walker in
  `worktree.rs` returning `(total, build)`, called by `worktree.disk` and `app.rs`'s `dir_usage`.
- **RA-372, rest: unbounded subprocesses outside handlers.** `github.rs` (`gh api user`,
  `repo.list`, `git clone`, `connect`) and `hooks.rs` (`run_user_pre_commit`) still need
  `proc::output_with_timeout` with `GIT_TERMINAL_PROMPT=0` / `GH_PROMPT_DISABLED=1`. `git.push`
  is fixed.
- **RA-378: guardrail.explain reads files it discards.** In `guardrail.rs` (around
  `request.old_file(path)`), skip the read when `removed == 0`; the verdict cannot change.
- **RA-394: skill.enable recopies the folder in every checkout.** The materialization stamp in
  `skills.rs` should be built from content (id, revision, hash of name and body, asset version)
  instead of `updated_at`, which undo and `skill.changed` also use.
- **RA-386 / RA-411, rest: D106 project scoping.** Now scoped for agents: `dashboard.get`,
  `notify.list`, `task.list`, `task.label.list`, `guardrail.holds.list`,
  `guardrail.requests.list`, `guardrail.request.get`, `guardrail.hold.get`,
  `session.restorable`. Still open: `module.get/list/stats/changelog.draft` (`module.rs`, add
  `notes::assert_actor_project`). `audit.list/get` are user-only already (RA-057).
- **RA-397, rest: session.bootstrap still returns the launch prompt as `assignment`.** The nudge
  and brief now treat it as first-launch only; `awareness.rs` needs the same check (keyed on
  `spawned_at`) or a rename to an "original instructions" field.
- **RA-401, rest: the CLI forwards the whole hook payload.** `relay-cli/src/main.rs` should drop
  `tool_response` before `session.report`, which reads only `tool_input`, `session_id`, `message`
  and `notification_type`; `audit.rs` hashes all of it.
- **RA-403, rest:** every `session.report` bumps `updated_at`, so undoing a `session.update` on a
  running session needs `force` (`audit.rs`), and the `task_sessions` row the update inserted
  survives the undo.
- **RA-382, rest: v3 import copies attachments under the store lock.** Copies are now removed on
  failure; moving them off the lock means registering `ImportV3` with `register_staged` in
  `handlers/app.rs`.
- **RA-385, rest: the brief has no size budget.** Notes are capped; `awareness.rs` /
  `providers.rs` should cap each brief section and point at `notes.get`, or hand Codex its
  instructions by file instead of argv.
- **RA-409, rest: orphaned attachment files are never reclaimed** after detach, task delete or
  project delete. Needs a recovery sweep that keeps files an undoable audit row still references.
- **RA-412: task.list hydrates every task.** The cheap fix is in the native client (skip or
  lazily load done tasks, `apps/relay-native/src/pages.rs`); the full fix batches `row_task`'s
  per-task lookups.
- **RA-358, rest:** an ACQUIRED lease event is still lost if a later step of the same request
  fails, and on a `device.run` commit failure; needs an engine-level after-rollback emit.

## Docs to update (BUS.md and the bus ops are held by golden-crane)

- `avd.boot` refuses `avd.already_running`; `avd.changed` carries an optional `message` with
  `failed` / `stopped` (RA-349).
- `task.attach` by path accepts optional `name` and `mime` (RA-414).
- `ui.pane.move` now emits the whole model plus `move` (RA-420; BUS.md §6.5 and D40).
- `ui.layout.save` falls back to `native.layout.current.<id>` (RA-419; D150, `tests/bus.rs:767`).
- `session.update` refuses `session.branch_primary` / `session.branch_shared` (RA-402).
- `apps/relay-native/src/agent_menu.rs:87`, the RA-352 "also at" site, was not looked at.
