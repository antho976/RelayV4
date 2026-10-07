# Audit findings left open: engine core, Low

Scope: RA-307..347, RA-603..649 and RA-740, fixed on branch `relay/bold-egret` (2026-10-07), on
top of `relay/amber-heron` and `origin/main`. Everything in that range is fixed or was already
fixed, except the parts below. Each says why it is open and what closing it would take.

Already fixed before this pass, with nothing to do: RA-320 (write targets resolve symlinks,
RA-101), RA-343 (`read_tail` decodes lossily), RA-347 (`worktree.remove` was already staged),
RA-607 (the Tauri door scaffolding was gone), and RA-740's file.rs half (`file.import` and
`file.restore_head` were already staged).

## Refactors left on purpose

- **RA-313, partial: an audited agent request still takes the store lock four times.**
  Actor resolution, identity filling and the idempotency lookup now share one acquisition and one
  read of the session row (it was up to six). Authorize keeps its own acquisition because it runs
  after payload validation, and validating a large payload under the lock would break the
  invariant. `mail_hint` also locks separately. The authorize→handler window the verifier calls
  microseconds is still there. Closing all of it means running authorization inside the
  handler's transaction, which restructures the dispatch pipeline.
- **RA-617, partial: the `Driver` trait still encapsulates little.** `spawn_profile` is now
  built from `args()` instead of a hand-copied `profile()`, which is removed. The
  provider-specific hook installation and extra arguments still live in `handlers/session.rs`,
  and the usage, updater and effort `match`es are still outside the trait.
- **RA-621, open: closed vocabularies are strings at every boundary.** Converting the ~25
  hand-written enum/string tables to typed enums touches the store, every handler and the bus
  schema. It is a project of its own, not a Low fix.
- **RA-625, partial: `handle_conn` is still one long function.** The six copies of the
  spawn_blocking dispatch sequence are now one `dispatch_blocking` helper. Stream frames share
  one `stream_frame` helper and use `ENVELOPE_V` instead of a literal `1`. Splitting the
  per-connection state into an `Attachments` struct and the op chain into a `match` is the large
  refactor and was skipped.
- **RA-632, partial: literal SQL through `Connection::query_row`.** Every such call in
  `handlers/device.rs` now uses `prepare_cached`. The same pattern elsewhere in the codebase is
  outside this range's files.

## Needs a design decision

- **RA-316, partial: `skill.install` still takes a repository's HEAD wholesale.** The clone is
  bounded by a timeout, runs with `core.symlinks=false`, and can no longer be walked out of
  through a symlink (RA-317). Still open, as the verifier suggested: showing the files and
  executables before install, pinning a commit, and enabling a newly installed skill only for the
  current project instead of everywhere (`enable_everywhere` in `handlers/provider.rs`).
- **RA-622, partial: an update can start while a launch is being prepared.** The `spawning`
  state was never written, so the dead check for it is gone and `sessions::set_state` is
  deleted. Closing the gap needs either an in-memory per-provider launch count on `Engine`,
  raised in `stage_launch` and dropped when `finish_launch` ends, which `provider_updates` checks;
  or a real `spawning` row state with a revert on failure.

## Small remainders

- **RA-341: the inline-keystroke fall-through.** Keystrokes now run inline only when the PTY's
  writer is free, and the resource tick reads the store on a blocking worker. If the PTY leaves
  the map between `answers_from_memory` and `pty_fast_path`, the request still falls through to
  the staged `session.input` handler on the runtime thread. That handler's prepare phase takes a
  short store read. The window is tiny, and the refusal it produces is the correct one.
- **RA-333, optional: in-process doors while quitting.** The socket door refuses requests with
  `app.quitting` once quitting starts and closes before the engine kills its children. `dispatch`
  itself does not refuse lifecycle mutations while `is_quitting()`, so an in-process caller is not
  covered.
- **RA-312, optional: `relay cmd <op> <payload>` mints a fresh id per call.** Replay works for
  any caller that keeps its id, as the full-envelope form does. A `--id` / `RELAY_REQUEST_ID`
  option for the short form would be a relay-cli change.
- **RA-634, partial: device bus tests still find a real `jarsigner`.** Run commands in the test
  instance now use `sh -c`, which reads no login profile. The bundle signing check still runs
  whatever `jarsigner` it finds through `JAVA_HOME`/`PATH`.
- **RA-649, partial: a blank `workspace.discover`/`create` path resolves against the engine's
  working directory.** This is now documented and the walk skips unreadable directories.
  Refusing a blank path would break `tests/phase12.rs`. The useful fix is for the CLI to send its
  own cwd when the path is blank (`crates/relay-cli/src/main.rs`).

## Client follow-ups handed to the native and mobile owners

These are client changes that follow from engine fixes here. The engine side is done.

- **RA-605:** `device.mirror.start` can refuse with `device.mirror_server_mismatch`, when the
  scrcpy jar fails its pinned SHA-256. `apps/relay-native/src/mirror.rs` should map it to a
  message next to `device.mirror_server_missing` (sent to sly-koala). Deploy note: an installed
  engine looks for the jar at `$RELAY_SCRCPY_SERVER`, then
  `~/.local/share/relay-v4/scrcpy-server-v4.1`, then the build checkout.
- **RA-326:** `setclipboard` text over `SET_CLIPBOARD_MAX_LENGTH` is refused with
  `device.input`. The native paste path only logs that refusal at debug level (sent to
  sly-koala).
- **RA-331:** a PTY killed by a signal now reports exit code `None`. `session_context.rs` still
  says "The process exited" for both `None` and `0` (sent to sly-seal).
- **RA-648, partial:** `task.copy_text`'s doc no longer claims to be the board's single source.
  The board still builds the same string twice (`board_view.rs`, two identical `format!`s), and
  no client calls the op.
- **RA-615:** the native `app.rs` keeps a third copy of the `<instance>.sock` / `.lock` names.
  The engine now has `Instance::socket_file()` and `lock_file()`.
- **RA-338:** the native and mobile clients ignore the new `bus.lagged` event. That is harmless,
  but they could refetch their state when it arrives.

## Behaviour changes worth knowing

- **`git.branch.clean_merged` is now an alias of `git.branch.cleanup` (RA-740).** It deletes
  only closed sessions' merged `relay/*` branches, plus their unused checkouts, and no longer any
  merged local branch. The phone's Git screen calls it, and its dialog text now says so
  (typechecked and linted; not run, since there is no Android SDK here).
- **A tracked `.codex/hooks.json` now refuses a Codex launch** with
  `session.codex_hooks_tracked` (RA-325). Writing Relay's machine-specific handlers into it would
  share them on the next commit, and skipping them would run Codex without guardrails.
- **Agents see less on the socket (RA-336, RA-337).** `bus.subscribe`/`bus.wait` now go through
  the full envelope, token and role checks, so a tokenless agent is refused. An agent's stream
  carries only its own project's events and project-less ones (`device.*` from anywhere). Of
  `mailbox.new` it gets only mail to it, from it, or broadcast. `guardrail.holds.list` and
  `guardrail.requests.list` are scoped the same way.
- **Decision number:** amber-heron's socket peer check was written as D164, which main had
  already given to the ui shell model. On this branch it is D165.
