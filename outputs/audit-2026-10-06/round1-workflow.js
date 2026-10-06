export const meta = {
  name: 'relay-audit-round1',
  description: 'Full bug, security and code-quality audit of RelayV4 except the phone app: 41 file-chunk auditors + 18 cross-cutting sweeps, every finding adversarially verified, high/critical re-checked by a skeptic',
  phases: [
    { title: 'Audit', detail: 'chunk auditors read every file; lens agents follow one concern across modules' },
    { title: 'Verify', detail: 'batch adversarial verification, then a skeptic per high/critical finding' },
  ],
}

const ROOT = '/home/anthony/dev3/RelayV4/.relay/worktrees/spry-newt'
const SCRATCH = '/tmp/claude-1000/-home-anthony-dev3-RelayV4--relay-worktrees-spry-newt/38c95c7e-730e-48ae-b011-06ecb624f458/scratchpad'
const PEDANTIC = SCRATCH + '/tool/clippy-pedantic.txt'

const CATS = ['bug', 'security', 'concurrency', 'invariant', 'performance', 'resource-leak', 'error-handling', 'data-loss', 'dead-code', 'duplication', 'overengineering', 'code-quality', 'test-quality', 'docs-drift']
const SEV = ['critical', 'high', 'medium', 'low']

const FINDING = {
  type: 'object',
  properties: {
    title: { type: 'string', description: 'One specific line: what is wrong and where' },
    category: { type: 'string', enum: CATS },
    severity: { type: 'string', enum: SEV },
    confidence: { type: 'string', enum: ['high', 'medium', 'low'] },
    file: { type: 'string', description: 'repo-relative path' },
    line: { type: 'integer', description: '1-indexed line of the decisive code' },
    symbol: { type: 'string', description: 'function / type / selector involved' },
    related: { type: 'array', items: { type: 'string' }, description: 'other file:line locations (callers, repeats, duplicates)' },
    evidence: { type: 'string', description: 'what the code does, quoting the decisive lines' },
    impact: { type: 'string', description: 'concrete trigger -> consequence (defects) or concrete ongoing cost (quality)' },
    fix: { type: 'string', description: 'specific recommended change; for overengineering, the simpler design and what it removes' },
    effort: { type: 'string', enum: ['S', 'M', 'L'] },
  },
  required: ['title', 'category', 'severity', 'confidence', 'file', 'line', 'evidence', 'impact', 'fix', 'effort'],
}
const FINDINGS = {
  type: 'object',
  properties: {
    findings: { type: 'array', items: FINDING },
    coverage_note: { type: 'string', description: 'files/regions not fully read, search patterns used, anything left unchecked' },
  },
  required: ['findings', 'coverage_note'],
}
const VERDICT = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    verdict: { type: 'string', enum: ['confirmed', 'plausible', 'refuted'] },
    severity: { type: 'string', enum: SEV },
    category: { type: 'string', enum: CATS },
    reason: { type: 'string', description: 'the evidence for the verdict; for refuted, exactly what the finding got wrong' },
    correction: { type: 'string', description: 'corrections to lines, mechanism, impact or fix; empty string if none' },
  },
  required: ['id', 'verdict', 'severity', 'category', 'reason', 'correction'],
}
const VERDICTS = { type: 'object', properties: { verdicts: { type: 'array', items: VERDICT } }, required: ['verdicts'] }

const RULES = `You are auditing RelayV4 at commit 361c6f9. It is a Rust workspace: a local engine (crates/relay-core: one SQLite store behind one mutex, a typed command bus over a Unix socket, PTY-backed AI-agent sessions in git worktrees, guardrails that gate what agents may run/write, Android device and emulator tools, scrcpy mirroring), the bus types (crates/relay-bus), the CLI and MCP servers (crates/relay-cli, including Unreal/Blender tooling that runs embedded Python), the phone door (crates/relay-remote: LAN listener, rendezvous relay, tunnel, pairing), and a GTK4/VTE/GtkSourceView desktop client (apps/relay-native). The phone app (apps/relay-mobile) is OUT OF SCOPE: never report on it.
This is the project's FIRST code audit: nothing has been reviewed before except by the author using the app. Assume nothing is known-good.
Repository root (your cwd): ${ROOT}. Architecture references if you need them: docs/ARCHITECTURE.md, docs/engine/BUS.md, docs/engine/SPEC.md, docs/engine/DECISIONS.md (code comments cite these as §n and Dnnn). CLAUDE.md lists invariants that are easy to break.

HARD RULES
- Strictly read-only. Do not modify, create, delete, stage or commit anything in the repository. Do not run cargo, rustc, npm or the app (a build owns the target dir). Do not call any mcp__relay__* tool: they act on the user's live Relay engine.
- Use Read, Grep, Glob and read-only shell (rg, git log/show/blame). Throwaway experiments (e.g. checking a regex, a path rule, a parser on a sample string) go under ${SCRATCH}/exp/ only; run Python with -I.
- The code is deliberately not rustfmt-clean. Never report formatting, naming style, missing doc sections, must_use or other pedantic-lint trivia.

EVIDENCE STANDARD
- Every finding cites a repo-relative file and 1-indexed line and quotes or precisely paraphrases the decisive code.
- Trace it: read the callers/callees needed to show the problem is reachable and not handled elsewhere (an upstream guard, a retry, a transaction, a lock, a documented decision). If a comment or DECISIONS entry shows the behavior is intended AND harmless, it is not a finding.
- Give a concrete trigger -> consequence. "Might be a problem" with no trigger is not a finding. If unsure, report with confidence low and say exactly what you could not establish.
- One finding per root cause; when the same mistake repeats, report it once and list the other locations in "related".

SEVERITY (real-world impact: a single-user desktop app with a local engine, many concurrent agent sessions, and an optional phone door reachable over LAN or a rendezvous server)
- critical: data loss/corruption; a security hole reachable by another local user, a network peer, or an agent escaping its guardrails; an engine crash/hang that takes down every session; a frequent hard failure of a core flow.
- high: likely user-visible wrong behavior or failure in a normal flow; a freeze of the bus or the UI; unbounded growth in normal use; a safety check that can be silently bypassed.
- medium: a real bug in a less common path or edge case; a design/maintainability problem with a concrete ongoing cost (e.g. duplicated logic that has already diverged).
- low: a minor bug in a rare edge case; a clear simplification or cleanup with modest payoff.

CATEGORIES: bug, security, concurrency, invariant (breaks a CLAUDE.md rule: slow work in a handler registered with Engine::register while holding the store mutex, a subprocess not run through proc::output_with_timeout, session.input touching SQLite, migration rules), performance, resource-leak, error-handling, data-loss, dead-code, duplication, overengineering, code-quality, test-quality, docs-drift.`

const NATIVE = 'GTK4 client. Look especially for: blocking work on the GTK main thread (synchronous bus requests through client.rs, file IO, subprocesses, image decoding) that freezes the UI; signal handlers connected again on every refresh (duplicate callbacks, leaks); glib timeout/idle sources never removed; Rc/RefCell cycles through closures capturing strong clones (widgets never freed; should use downgrade); RefCell double-borrow panics on re-entrant callbacks; async results applied after the view moved on (stale generation); unsaved edits lost; state not reset when switching project/session.'

const CHUNKS = [
  { key: 'core-engine', title: 'engine dispatch, transactions, events, proc helper, audit log', files: ['crates/relay-core/src/engine.rs', 'crates/relay-core/src/serve.rs', 'crates/relay-core/src/lib.rs', 'crates/relay-core/src/time.rs', 'crates/relay-core/src/paths.rs', 'crates/relay-core/src/proc.rs', 'crates/relay-core/src/handlers/mod.rs', 'crates/relay-core/src/handlers/bus.rs', 'crates/relay-core/src/handlers/audit.rs', 'crates/relay-core/src/audit.rs', 'crates/relay-core/build.rs', 'crates/relay-core/Cargo.toml'], focus: 'Request dispatch and the transaction/mutex discipline (register / register_unlocked / register_staged); event fan-out behind bus.wait and bus.subscribe (lost wakeups, unbounded queues, slow subscribers); role/op gating; proc::output_with_timeout itself (does it really kill and reap on timeout, drain stdout and stderr concurrently, bound output, kill the process group?).' },
  { key: 'core-socket-store', title: 'Unix socket server, SQLite store and migrations, crash recovery', files: ['crates/relay-core/src/socket.rs', 'crates/relay-core/src/store.rs', 'crates/relay-core/src/recovery.rs'], focus: 'Socket: framing, partial reads/writes, oversized or malformed frames, client disconnect mid-request, peer authentication and how a connection is bound to a session/role, subscription cleanup. Store: migrations append-only with SCHEMA_VERSION bumps and every earlier version openable, pragmas (WAL, busy_timeout, foreign keys), backup. Recovery after a crash.' },
  { key: 'core-session', title: 'session handler: lifecycle, done/report, review groups, mailbox, bootstrap', files: ['crates/relay-core/src/handlers/session.rs', 'crates/relay-core/src/sessions.rs'], focus: 'Session spawn/attach/detach/close/restore; the session.done state machine and FIFO review-group advancement (builders finish before review, reviewers before advance); mailbox priority and ack; bootstrap contents; session.input must not touch SQLite (D148).' },
  { key: 'core-guardrail', title: 'guardrail policy, grants, request/resolve flow', files: ['crates/relay-core/src/guardrail.rs', 'crates/relay-core/src/guardrail/grants.rs', 'crates/relay-core/src/handlers/guardrail.rs'], focus: 'Command matching (prefix vs token matching, repeated spaces, flag reordering like rm -fr / rm -r -f, absolute paths like /bin/rm, env/sudo/command wrappers, sh -c strings, ; && | $( ) chains, quoting), path normalization (.., symlinks, ~, relative vs absolute, case), caps accounting, grants (once vs session scope, expiry, replay, who may approve), the wait flow. Any bypass of a deny rule is high or critical.' },
  { key: 'core-git', title: 'git handlers, GitHub client, worktree management', files: ['crates/relay-core/src/handlers/git.rs', 'crates/relay-core/src/github.rs', 'crates/relay-core/src/worktree.rs'], focus: 'git via gix and subprocesses (timeouts, store mutex), worktree create/remove safety (must never delete user data or a path outside .relay/worktrees), branch switching guards, commit/stage correctness, PR listing (pagination, rate limits, token handling, timeouts).' },
  { key: 'core-pty-providers', title: 'PTY, provider launch, provider CLI updates, usage', files: ['crates/relay-core/src/pty.rs', 'crates/relay-core/src/providers.rs', 'crates/relay-core/src/provider_updates.rs', 'crates/relay-core/src/handlers/provider.rs', 'crates/relay-core/src/usage.rs'], focus: 'PTY spawn/read/write/resize, scrollback ring buffers (bounds, UTF-8 split across reads), child exit detection and reaping, provider command lines and environment (secrets passed via env/argv), CLI self-update runner (timeouts, bounded output, what it downloads and whether it verifies it), usage accounting.' },
  { key: 'core-device-handler', title: 'device handler: adb, emulator, AVD, scrcpy orchestration', files: ['crates/relay-core/src/handlers/device.rs'], focus: 'Every adb/emulator/avdmanager/scrcpy subprocess: timeout via proc::output_with_timeout, store-mutex discipline for slow calls, device lease checks, argument/shell injection into `adb shell` strings, parsing of adb output, install/run flows. NOTE: a peer has just changed this file on main (avd.boot -no-window, new avd.stop); audit the version in your checkout.' },
  { key: 'core-device-mirror', title: 'device model, device leases, mirroring backend', files: ['crates/relay-core/src/device.rs', 'crates/relay-core/src/device_lease.rs', 'crates/relay-core/src/handlers/device_lease.rs', 'crates/relay-core/src/mirror.rs'], focus: 'Lease acquire/release/expiry and races between sessions; mirroring (scrcpy server push and verification, port forwarding, stream handling, cleanup on disconnect or engine exit).' },
  { key: 'core-tasks-notes', title: 'task board, modules, notes handlers', files: ['crates/relay-core/src/handlers/task.rs', 'crates/relay-core/src/handlers/module.rs', 'crates/relay-core/src/handlers/notes.rs'], focus: 'Task CRUD, column moves and ordering/positions, parent/child cycles, labels, linked commits, changelog, notes and standing notes, soft delete, pagination, concurrent edits from UI and agents (lost updates).' },
  { key: 'core-workspace-files', title: 'workspace registry, file ops, overlap detection, awareness', files: ['crates/relay-core/src/handlers/workspace.rs', 'crates/relay-core/src/handlers/file.rs', 'crates/relay-core/src/handlers/overlap.rs', 'crates/relay-core/src/awareness.rs'], focus: 'file.* ops: path traversal and symlink escape out of the project/worktree root, trash/restore, rename/move collisions and overwrites, non-atomic writes, large files. Workspace/project registry consistency. Overlap detection between sessions claims. Awareness/peer state.' },
  { key: 'core-hooks-skills', title: 'provider hooks, skills, plugins loader, fs watcher, branch cleanup', files: ['crates/relay-core/src/hooks.rs', 'crates/relay-core/src/skills.rs', 'crates/relay-core/src/plugins.rs', 'crates/relay-core/src/watch.rs', 'crates/relay-core/src/branch_cleanup.rs'], focus: 'Hook/config installation into provider config files (clobbering user settings, non-atomic writes, JSON merge errors), skills discovery and enablement, compiled-in plugin loading, filesystem watcher debouncing/leaks, branch cleanup safety (never deleting unmerged, protected, checked-out or session-owned branches).' },
  { key: 'core-misc-handlers', title: 'app, V3 import, integration, notify, settings, UI-state handlers', files: ['crates/relay-core/src/handlers/app.rs', 'crates/relay-core/src/handlers/import_v3.rs', 'crates/relay-core/src/handlers/integration.rs', 'crates/relay-core/src/handlers/notify.rs', 'crates/relay-core/src/handlers/settings.rs', 'crates/relay-core/src/handlers/ui.rs'], focus: 'App status/backup/recovery, V3 import (idempotency, partial failure, data mapping), integration requests, notifications, settings validation and persistence, UI layout/state persistence.' },
  { key: 'bus', title: 'relay-bus: envelope, errors, op registry, schema, types', files: ['crates/relay-bus/src/envelope.rs', 'crates/relay-bus/src/error.rs', 'crates/relay-bus/src/lib.rs', 'crates/relay-bus/src/registry.rs', 'crates/relay-bus/src/schema.rs', 'crates/relay-bus/src/types.rs', 'crates/relay-bus/src/ops/mod.rs', 'crates/relay-bus/src/ops/app.rs', 'crates/relay-bus/src/ops/audit.rs', 'crates/relay-bus/src/ops/bus.rs', 'crates/relay-bus/src/ops/device.rs', 'crates/relay-bus/src/ops/file.rs', 'crates/relay-bus/src/ops/git.rs', 'crates/relay-bus/src/ops/guardrail.rs', 'crates/relay-bus/src/ops/module.rs', 'crates/relay-bus/src/ops/notes.rs', 'crates/relay-bus/src/ops/notify.rs', 'crates/relay-bus/src/ops/overlap.rs', 'crates/relay-bus/src/ops/provider.rs', 'crates/relay-bus/src/ops/session.rs', 'crates/relay-bus/src/ops/task.rs', 'crates/relay-bus/src/ops/ui.rs', 'crates/relay-bus/src/ops/workspace.rs', 'crates/relay-bus/examples/dump_schema.rs', 'crates/relay-bus/tests/registry.rs', 'crates/relay-bus/Cargo.toml'], focus: 'Envelope (de)serialization and versioning, error model, op registry gating (roles, user-only ops, bus_writes, allow_ui), schema generation, serde defaults/optional fields/enum tagging that could silently accept bad input or break compatibility.' },
  { key: 'remote', title: 'phone door: direct listener, rendezvous, tunnel, pairing, bridge; deploy', files: ['crates/relay-remote/src/bridge.rs', 'crates/relay-remote/src/direct.rs', 'crates/relay-remote/src/lib.rs', 'crates/relay-remote/src/pairlink.rs', 'crates/relay-remote/src/registry.rs', 'crates/relay-remote/src/rendezvous.rs', 'crates/relay-remote/src/tunnel.rs', 'crates/relay-remote/src/wire.rs', 'crates/relay-remote/tests/remote.rs', 'crates/relay-remote/Cargo.toml', 'crates/relay-cli/src/remote.rs', 'deploy/relay-remote.service', 'deploy/relay-rendezvous.service', 'deploy/rendezvous.Dockerfile'], focus: 'This is network-facing: bind addresses, authentication of every request, pairing secret generation (entropy, RNG) and storage, encryption of traffic, replay protection, constant-time comparisons, what a rendezvous operator can see or inject, which bus ops a phone may call, DoS (frame size limits, connection limits, slow clients), deploy unit/Dockerfile hardening.' },
  { key: 'cli-main', title: 'CLI entry, MCP server, Unreal process mgmt, Blender MCP', files: ['crates/relay-cli/src/main.rs', 'crates/relay-cli/src/mcp.rs', 'crates/relay-cli/src/unreal_process.rs', 'crates/relay-cli/src/blender.rs', 'crates/relay-cli/Cargo.toml'], focus: 'CLI argument handling and `relay q` bus client, MCP JSON-RPC framing over stdio and tool schemas, Unreal editor process management, Blender job runner (subprocess timeouts, temp files, how user values are embedded into Python job source = injection risk). Blender 5.2.1 is installed locally and `cargo test` fails: blender::tests::the_blender_tools_work_on_a_real_rig panics with AttributeError: Action object has no attribute fcurves.' },
  { key: 'cli-unreal', title: 'Unreal MCP server', files: ['crates/relay-cli/src/unreal.rs'], focus: 'Build/log tools, Remote Control HTTP calls (timeouts, error handling, response size), Python executed inside the editor (interpolation of user/agent values into Python source = injection), path handling, large outputs, tool argument validation.' },
  { key: 'cli-python', title: 'embedded Blender and Unreal Python', files: ['crates/relay-cli/src/blender_py/anim_inspect.py', 'crates/relay-cli/src/blender_py/common.py', 'crates/relay-cli/src/blender_py/export.py', 'crates/relay-cli/src/blender_py/info.py', 'crates/relay-cli/src/blender_py/mesh_check.py', 'crates/relay-cli/src/blender_py/render.py', 'crates/relay-cli/src/blender_py/rig_check.py', 'crates/relay-cli/src/blender_py/run.py', 'crates/relay-cli/src/blender_py/tests/make_fixture.py', 'crates/relay-cli/src/unreal_py/anim_inspect.py', 'crates/relay-cli/src/unreal_py/anim_preview.py', 'crates/relay-cli/src/unreal_py/asset_audit.py', 'crates/relay-cli/src/unreal_py/asset_refs.py', 'crates/relay-cli/src/unreal_py/blueprint_info.py', 'crates/relay-cli/src/unreal_py/capture.py', 'crates/relay-cli/src/unreal_py/common.py', 'crates/relay-cli/src/unreal_py/data_table.py', 'crates/relay-cli/src/unreal_py/import_fbx.py', 'crates/relay-cli/src/unreal_py/play.py', 'crates/relay-cli/src/unreal_py/preview_asset.py', 'crates/relay-cli/src/unreal_py/project_check.py', 'crates/relay-cli/src/unreal_py/tests/run_inspect.py', 'crates/relay-cli/src/unreal_py/tests/unreal.py'], focus: 'API misuse and version compatibility. Blender 5.2.1 is installed at ~/.local/bin/blender and the real-rig test fails with AttributeError: Action has no attribute fcurves (info.py:45; also export.py:48) because Blender 4.4+/5.x moved F-curves into slotted/layered actions: find every API use broken or deprecated on Blender 5.x. You MAY run `blender --background --factory-startup --python-expr "..."` (or a script under the scratch dir) to check an API fact. Also: exceptions swallowed, wrong units/axes, the result protocol back to Rust (stdout parsing robustness), and how these scripts get their inputs (injection).' },
  { key: 'native-app', title: 'native app startup, global state, bus client', files: ['apps/relay-native/src/app.rs', 'apps/relay-native/src/main.rs', 'apps/relay-native/src/client.rs', 'apps/relay-native/Cargo.toml'], focus: NATIVE + ' Also: the bus client (blocking requests vs async, reconnect after engine restart, event subscription thread to main-loop handoff, timeouts).' },
  { key: 'native-shell', title: 'native shell and project/workspace registry sidebar', files: ['apps/relay-native/src/shell.rs', 'apps/relay-native/src/shell/registry.rs'], focus: NATIVE },
  { key: 'native-board', title: 'native task board view', files: ['apps/relay-native/src/board_view.rs'], focus: NATIVE + ' Also: drag-and-drop ordering math, filters, optimistic updates vs server truth.' },
  { key: 'native-codegit', title: 'native Code/Git page and image preview', files: ['apps/relay-native/src/code_git.rs', 'apps/relay-native/src/image_preview.rs'], focus: NATIVE + ' Also: staging/commit flows, diff rendering of huge files, image decoding of large/untrusted files on the main thread.' },
  { key: 'native-editor-files', title: 'native editor, project file tree, icons', files: ['apps/relay-native/src/editor.rs', 'apps/relay-native/src/project_files.rs', 'apps/relay-native/src/icons.rs'], focus: NATIVE + ' Also: save conflicts (file changed on disk / by an agent while open), encoding of non-UTF-8 files, huge files, trash/rename/move from the tree.' },
  { key: 'native-mirror', title: 'native device mirror and device tools', files: ['apps/relay-native/src/mirror.rs', 'apps/relay-native/src/mirror/decode.rs', 'apps/relay-native/src/mirror/input.rs', 'apps/relay-native/src/mirror/glyphs.rs', 'apps/relay-native/src/tools_devices.rs'], focus: NATIVE + ' Also: the video decode pipeline (FFmpeg subprocess or decoder: process cleanup, frame buffer bounds, back-pressure), input event coordinate mapping and rotation. NOTE: a peer just changed mirror.rs and tools_devices.rs on main; audit the version in your checkout.' },
  { key: 'native-notes-a', title: 'native notes pages, notes window, notes menu', files: ['apps/relay-native/src/note_pages.rs', 'apps/relay-native/src/notes_window.rs', 'apps/relay-native/src/note_pages/menu.rs', 'apps/relay-native/src/note_pages/glyphs.rs'], focus: NATIVE + ' Also: draft retention across window hide/close, dirty tracking, save races with agent edits.' },
  { key: 'native-notes-b', title: 'native note document model and text, session context, notification center', files: ['apps/relay-native/src/note_pages/doc.rs', 'apps/relay-native/src/note_pages/text.rs', 'apps/relay-native/src/session_context.rs', 'apps/relay-native/src/notification_center.rs'], focus: NATIVE + ' Also: text offset math (byte vs char vs grapheme indices, GTK TextIter offsets are in chars), markdown parsing edge cases, notification dedup.' },
  { key: 'native-tasks', title: 'native task pages and agent menu', files: ['apps/relay-native/src/task_pages.rs', 'apps/relay-native/src/agent_menu.rs'], focus: NATIVE },
  { key: 'native-launch', title: 'native launch dialog, onboarding, terminal widget', files: ['apps/relay-native/src/launch.rs', 'apps/relay-native/src/onboarding.rs', 'apps/relay-native/src/terminal.rs'], focus: NATIVE + ' Also: the VTE terminal feed path (output delivery, scrollback, paste/input forwarding latency, resize), stale launch submissions.' },
  { key: 'native-tools', title: 'native tools pages: settings, skills, plugins', files: ['apps/relay-native/src/tools.rs', 'apps/relay-native/src/tools_settings.rs', 'apps/relay-native/src/tools_skills.rs', 'apps/relay-native/src/tools_plugins.rs'], focus: NATIVE + ' Also: settings serialization round-trips (fractional/boolean values), toggles that resubmit themselves.' },
  { key: 'native-market-status', title: 'native market, status bar, usage, provider updates', files: ['apps/relay-native/src/tools_market.rs', 'apps/relay-native/src/status_usage.rs', 'apps/relay-native/src/status.rs', 'apps/relay-native/src/provider_updates.rs'], focus: NATIVE + ' Also: polling timers doing heavy work every tick.' },
  { key: 'native-misc', title: 'native guardrail pages/settings, pages, panel, sounds, wallpaper, fonts, shortcuts', files: ['apps/relay-native/src/guardrail_pages.rs', 'apps/relay-native/src/guardrail_settings.rs', 'apps/relay-native/src/pages.rs', 'apps/relay-native/src/panel.rs', 'apps/relay-native/src/sounds.rs', 'apps/relay-native/src/wallpaper_rotation.rs', 'apps/relay-native/src/fonts.rs', 'apps/relay-native/src/shortcuts.rs'], focus: NATIVE + ' Also: the guardrail exception prompt (can the user approve the wrong request? stale request ids), sound playback via ffplay subprocesses (reaping, overlap), keyboard shortcut conflicts. clippy on Rust 1.98 fails on sounds.rs:150 (chunks_exact_to_as_chunks) while CI pins 1.94: note toolchain drift once, briefly.' },
  { key: 'native-css', title: 'native GTK CSS', files: ['apps/relay-native/src/theme.css', 'apps/relay-native/src/css/board.css', 'apps/relay-native/src/css/git_files.css', 'apps/relay-native/src/css/guardrails.css', 'apps/relay-native/src/css/mirror.css', 'apps/relay-native/src/css/notes.css', 'apps/relay-native/src/css/sessions.css', 'apps/relay-native/src/css/tools.css', 'apps/relay-native/src/css/usage.css', 'apps/relay-native/src/css/workspace.css', 'apps/relay-native/resources/relay-editor.xml'], focus: 'GTK4 CSS loaded in this order from app.rs: theme.css then css/mirror, notes, git_files, board, sessions, workspace, guardrails, usage, tools. Find: selectors/classes never applied by the Rust code (grep add_css_class / set_css_classes / css_classes / widget names / set_widget_name), rules duplicated or silently overridden by a later file, properties GTK4 CSS does not support (they are ignored with a warning), hardcoded colors that bypass the @define-color tokens, contradictory sizes, !important pile-ups. Also the GtkSourceView style scheme XML. Report dead CSS grouped per file with counts, not one finding per selector.' },
  { key: 'native-smoke', title: 'native smoke-test harness', files: ['apps/relay-native/src/smoke.rs', 'apps/relay-native/src/smoke_notes.rs', 'apps/relay-native/src/smoke_project_files.rs', 'apps/relay-native/src/smoke_registry.rs', 'apps/relay-native/src/roadmap_smoke.rs'], tests: true, focus: 'In-app smoke harness compiled into the shipping binary: is it gated so it cannot run or be triggered in normal use (env vars), does it touch real user data, is it flaky, duplicated, or dead?' },
  { key: 'tests-sessions', title: 'engine tests: sessions, store', files: ['crates/relay-core/tests/sessions.rs', 'crates/relay-core/tests/store.rs'], tests: true },
  { key: 'tests-bus-guardrails', title: 'engine tests: bus, guardrails', files: ['crates/relay-core/tests/bus.rs', 'crates/relay-core/tests/guardrails.rs'], tests: true },
  { key: 'tests-mirror-board', title: 'engine tests: mirror, board, agent surface', files: ['crates/relay-core/tests/mirror.rs', 'crates/relay-core/tests/board.rs', 'crates/relay-core/tests/agent_surface.rs'], tests: true },
  { key: 'tests-features', title: 'engine tests: agent features, awareness, phase7, phase8', files: ['crates/relay-core/tests/agent_features.rs', 'crates/relay-core/tests/awareness.rs', 'crates/relay-core/tests/phase7.rs', 'crates/relay-core/tests/phase8.rs'], tests: true },
  { key: 'tests-misc', title: 'engine tests: phase9/11/12, registry removal, branch cleanup, device lease, branch switch', files: ['crates/relay-core/tests/phase9.rs', 'crates/relay-core/tests/phase11.rs', 'crates/relay-core/tests/phase12.rs', 'crates/relay-core/tests/registry_remove.rs', 'crates/relay-core/tests/branch_cleanup.rs', 'crates/relay-core/tests/device_lease.rs', 'crates/relay-core/tests/git_branch_switch.rs'], tests: true },
  { key: 'perf-harness', title: 'performance harness and perf scripts', files: ['crates/relay-core/examples/perf.rs', 'scripts/perf/baseline.py', 'scripts/perf/native-baseline.py', 'scripts/perf/compare.py', 'scripts/perf/report.py'], tests: true, focus: 'Does the harness measure what it claims (warm-up, timer placement, percentiles, comparing like with like)? Does it touch real user data or a live engine?' },
  { key: 'scripts-infra', title: 'scripts, launcher, CI, manifests, build script', files: ['scripts/install-native-desktop.py', 'scripts/native-pointer-click.py', 'scripts/native-setup-smoke.py', 'scripts/native-smoke.py', 'scripts/test-launcher.py', 'scripts/test-launcher-real.py', 'run.sh', '.github/workflows/ci.yml', 'Cargo.toml', 'crates/relay-core/Cargo.toml', 'crates/relay-cli/Cargo.toml', 'crates/relay-remote/Cargo.toml', 'crates/relay-bus/Cargo.toml', 'apps/relay-native/Cargo.toml', 'crates/relay-core/build.rs', '.gitignore', '.gitattributes'], focus: 'Launcher correctness (run.sh: quoting, PATH, stale engine reuse, error handling), CI (action pinning by tag vs SHA, permissions, cache poisoning, what is not tested), dependency declarations (unused deps, duplicate versions, features enabled but unused, default features), scripts that touch real user data or leave processes behind. Ignore .github/workflows/mobile-apk.yml (phone app).' },
  { key: 'plugins-unreal', title: 'Unreal Engine plugin content vs implementation', files: ['plugins/unreal-engine/plugin.json', 'plugins/unreal-engine/instructions.md', 'plugins/unreal-engine/README.md', 'plugins/unreal-engine/docs/mcp-tools.md', 'plugins/unreal-engine/docs/setup.md', 'plugins/unreal-engine/docs/agent-workflow.md'], tests: false, focus: 'Read the listed files fully, then check every documented MCP tool, parameter, default and behavior against crates/relay-cli/src/unreal.rs and the loader in crates/relay-core/src/plugins.rs; check every skills/*/SKILL.md frontmatter (name/description validity) under plugins/unreal-engine/skills; then skim the skill reference recipes for code that is wrong for current Unreal (non-existent APIs, wrong signatures, deprecated calls). Report only what you are confident about; group recipe errors per skill.' },
  { key: 'plugins-blender', title: 'Blender plugin content vs implementation', files: ['plugins/blender/plugin.json', 'plugins/blender/instructions.md', 'plugins/blender/README.md', 'plugins/blender/docs/mcp-tools.md', 'plugins/blender/docs/setup.md'], tests: false, focus: 'Read the listed files fully, then check every documented MCP tool against crates/relay-cli/src/blender.rs and crates/relay-core/src/plugins.rs; check skills/*/SKILL.md frontmatter; then check the skill recipes under plugins/blender/skills for bpy code that is wrong on Blender 5.x (installed locally: Blender 5.2.1 at ~/.local/bin/blender; you MAY verify an API fact with `blender --background --factory-startup --python-expr "..."`), e.g. Action.fcurves removed by slotted actions. Group recipe errors per skill.' },
]

const LENSES = [
  { key: 'lens-store-mutex', concern: 'Store-mutex discipline (CLAUDE.md, BUS.md 5.1). Enumerate EVERY handler registration (Engine::register / register_unlocked / register_staged and any other registration path). For each plain `register` handler, decide whether its body (including helpers it calls) does slow work while the mutex is held: subprocesses, network, gix operations on big repos, directory/tree walks, large file reads, sleeps, waits, device calls. For staged handlers, check the commit phase does not do slow work and the read phase does not rely on state that can change before commit (TOCTOU). For unlocked handlers, check they do not mutate the store or read inconsistent multi-table state.' },
  { key: 'lens-subprocess', concern: 'Every subprocess spawn in relay-core, relay-cli, relay-remote and relay-native (std::process::Command, tokio::process::Command, portable_pty, glib/gio subprocess). Is it bounded (proc::output_with_timeout or equivalent)? Are stdout and stderr drained concurrently (pipe-buffer deadlock)? Is the child killed AND reaped on timeout/cancel (zombies, orphaned grandchildren, process groups)? Are inherited file descriptors and environment (tokens, RELAY_* vars) leaked to children? Is PATH resolution safe? Is any value interpolated into a shell string (sh -c) or into a script source? Do values that can start with "-" get passed where they would be parsed as options (git, adb)?' },
  { key: 'lens-sqlite', concern: 'All SQLite usage in relay-core: query_row/prepare vs prepare_cached on hot paths, N+1 query loops, missing indexes for frequent WHERE/ORDER BY (read the migrations in store.rs), multi-statement mutations outside a transaction, migration correctness (append-only, SCHEMA_VERSION bump, every earlier version openable, idempotency on partial failure), session.input touching SQLite (D148), JSON stored in TEXT columns parsed without error handling, LIKE patterns built from user input without escaping % and _, timestamps stored as TEXT and compared lexicographically with mixed formats, integer width mismatches, tables that grow forever without pruning.' },
  { key: 'lens-concurrency', concern: 'Threads, channels, mutexes, condvars, atomics and tokio tasks across the engine (engine.rs, socket.rs, pty.rs, watch.rs, mirror.rs, device_lease.rs), relay-remote and the native client: lock-ordering deadlocks and re-entrant locking, locks held across blocking IO or .await, std::sync::Mutex in async code, mutex poisoning (lock().unwrap() cascading panics), races and TOCTOU on files and state, lost wakeups in bus.wait/subscribe, unbounded channels, tasks/threads never joined or cancelled, shutdown ordering, Relaxed atomics used for synchronization.' },
  { key: 'sec-local-ipc', concern: 'SECURITY: local IPC and privilege boundaries. The engine Unix socket (where it lives, file and directory permissions, umask, whether peer credentials are checked), how a connection is bound to a session and role, whether an agent session can act as the user or as another session (e.g. by supplying a different `session`, project_id or role in a payload), user-only ops (approving guardrail requests, settings, deleting) reachable by agents, MCP servers exposed by relay-cli (what they let an agent do), environment variables or files that hand an agent more authority than its role (tokens, socket paths), and the provider hooks that enforce guardrails (can an agent edit its own hook config, settings or the files the engine trusts to weaken enforcement?).' },
  { key: 'sec-remote-door', concern: 'SECURITY: the phone door, the network-facing surface (crates/relay-remote, crates/relay-cli/src/remote.rs, the engine side that serves it, deploy/). Bind addresses and default exposure, authentication of every frame/request, pairing secret generation (entropy, RNG source) and storage (file permissions), whether traffic is encrypted end to end or readable/injectable by the rendezvous operator or a LAN attacker, replay protection, constant-time secret comparison, which bus ops a paired phone may invoke and whether that is least-privilege, frame-size and connection limits (memory DoS, slowloris), revocation of a lost phone, logging of secrets, systemd/Docker hardening.' },
  { key: 'sec-guardrail-bypass', concern: 'SECURITY: try to defeat the guardrails as an adversarial agent would. Understand first how enforcement works end to end (which component checks commands and writes: provider hooks, the engine, the CLI) and what the default denied_commands and write_roots are. Then construct concrete bypasses: command-string variants (rm -fr, rm -r -f, rm  -rf with two spaces, /bin/rm, \\rm, env rm, command rm, xargs rm, find -delete, git clean -xdf, git push -f / --force-with-lease, git reset --hard via alias), wrappers (bash -c, sh -c, python -c, eval, $( ), backticks, ; && || | chains, newlines), path tricks (.., symlinks created first then written through, ~, $HOME, relative paths resolved against a different cwd, case, trailing slashes, NUL), cap evasion (many small writes), grant abuse (an agent approving its own request, grant scope wider than asked, grants surviving session end), and TOCTOU between check and action. Report each working bypass with the exact input.' },
  { key: 'sec-fs-injection', concern: 'SECURITY: filesystem and injection across all crates. file.* ops and any engine code that writes, moves or deletes: path traversal, symlink following out of the allowed root, deleting the wrong directory (worktree removal, trash, cleanup), predictable temp paths in /tmp (symlink races), permissions on created files and directories (store DB, backups, briefs, logs, pairing secrets: world-readable?). Injection: values interpolated into shell commands, into Python sources run by Blender/Unreal, into git/adb argument lists (option injection via leading "-"), into SQL, into GTK markup (set_markup with unescaped text = markup injection/crash). Secrets: GitHub tokens, provider API keys, pairing keys written to logs, the SQLite store, audit entries, session briefs, scrollback or crash reports.' },
  { key: 'sec-supply-chain', concern: 'SECURITY: supply chain and configuration. Cargo.toml/Cargo.lock (unmaintained or yanked crates, git dependencies, wildcard versions, duplicate major versions, heavy optional features), the vendored scrcpy-server binary in apps/relay-native/resources (provenance, is its hash verified before being pushed to a device?), build.rs, the provider CLI auto-update feature (what it downloads/executes, from where, any signature or checksum verification), plugins/MCP servers enabled per project (can a malicious repository configure code execution simply by being opened? e.g. project-level config, hooks, skills read from the repo), CI workflow (actions pinned by tag not SHA, token permissions), scripts/install-native-desktop.py and run.sh (what they execute).' },
  { key: 'lens-data-loss', concern: 'Data loss and integrity across the engine and native client: worktree/branch removal deleting unmerged work or user files, file writes that are not atomic (crash mid-write truncates the file; should write temp + rename), saves that overwrite a newer on-disk version (agent edited the file while open in the editor), lost updates between the UI and agents editing the same task/note (last writer wins silently), backups/restore and V3 import correctness, migrations that can fail half-way, deleting a project/workspace cascading to data the user expects to keep, trash that cannot be restored, scrollback/transcripts truncated without notice.' },
  { key: 'lens-gtk-mainthread', concern: 'relay-native main-thread health, across all its files: synchronous bus requests or other blocking calls (file IO, subprocess, DNS/network, image decode, git) executed from GTK signal handlers, timeouts or idle callbacks (each one freezes the whole UI; quantify how long it can block); glib timeout/idle sources added without keeping the SourceId and never removed; signal handlers connected on every rebuild/refresh (duplicate callbacks, leaks); Rc<RefCell<..>> cycles through closures capturing strong clones of widgets or state (memory never freed; should use glib::clone!(@weak) or Rc::downgrade); RefCell borrow_mut while a borrow is live across a callback that re-enters (runtime panic); background threads touching GTK objects (must not).' },
  { key: 'lens-errors-panics', concern: 'Panics and error handling in non-test code of all crates: unwrap/expect on values that can really be Err/None at runtime, indexing and string slicing that can panic (non-char-boundary slicing of user text, empty vectors), arithmetic overflow (overflow checks are ON in the dev profile the app actually runs in, so u32/usize subtraction underflow panics), division by zero; a panic in a handler or socket thread: does it poison the store mutex or kill the engine? Errors silently discarded (let _ =, .ok(), unwrap_or_default) where the user needs to know; error messages that mislead; inconsistent error codes over the bus.' },
  { key: 'lens-dead-dup', concern: 'Dead code and duplication across the whole workspace: functions, types, consts, settings and CSS never referenced (prove with grep), bus ops defined in relay-bus with no handler or never called by any client (native, CLI, MCP, remote), handlers for ops nobody calls, duplicate implementations of the same helper in several crates/modules (time formatting, path normalization, shell quoting, git invocation, JSON helpers, size formatting, provider detection), and copy-pasted blocks that have diverged (one copy fixed, the other not: that divergence is a bug, report it as such).' },
  { key: 'lens-overengineering', concern: 'Overengineering and design debt across the workspace: abstractions with a single implementation or caller, traits/generics with one concrete type, layers that only forward, configuration knobs nobody sets, state machines more complex than the behavior needs, home-grown reimplementations of std or of a dependency already in Cargo.lock, functions over ~300 lines that interleave UI construction with business logic, modules with tangled responsibilities (god files), stringly-typed protocols where a type exists. Be concrete: name the simpler design, what it deletes, and what risk the current complexity creates. Do not report mere size; report complexity that costs something.' },
  { key: 'lens-contract', concern: 'Bus contract consistency between: op definitions (crates/relay-bus/src/ops, types.rs), handler implementations (crates/relay-core/src/handlers), schema/bus.v1.json, and callers (apps/relay-native client calls, relay-cli, the MCP exposure, relay-remote bridge). Find payload fields declared but ignored by the handler, fields a handler reads that are not declared, result shapes a caller parses differently from what the handler returns (silently defaulting), enum values that differ between sides, error codes/kinds callers match on that handlers never produce (or vice versa), schema/bus.v1.json out of date relative to the code, ops gated to roles that look wrong.' },
  { key: 'lens-perf', concern: 'Performance on the hot paths: PTY output fan-out (pty.rs -> socket.rs -> native terminal.rs), session.input latency, request dispatch overhead, store queries per request, JSON (de)serialization of large payloads (scrollback, file trees, diffs), unbounded file tree walks, git status on large repos, native client timers that do heavy work every tick, O(n^2) loops over sessions/tasks/lines, repeated allocation/cloning in loops, regexes compiled per call. Quantify where possible (n, frequency). docs/perf has an earlier baseline and fixes; check whether claimed fixes hold.' },
  { key: 'lens-resources', concern: 'Resource lifecycle and unbounded growth across all crates: file descriptors, child processes, threads, PTYs, sockets, temp files/dirs, worktrees and branches left behind, scrollback buffers, in-memory maps keyed by session/task/project/connection that never evict, log files that grow forever, SQLite tables that grow without pruning (audit, events, mailbox, usage, activity), subscriptions or waiters never removed when a client disconnects, engine shutdown/restart leaving orphaned agent processes or stale sockets/lock files.' },
  { key: 'lens-docs', concern: 'Docs that mislead: CLAUDE.md, README.md, docs/ARCHITECTURE.md, docs/engine/BUS.md, docs/engine/SPEC.md, docs/engine/DECISIONS.md, docs/PARITY.md, docs/VERIFICATION.md, docs/MOBILE.md (only its PC-side claims), plugin READMEs, and code comments that state invariants. Report claims that are false about the current code (commands, op names, payloads, invariants, file paths, behaviors, guarantees), and invariants the docs promise that the code does not enforce. Skip cosmetic issues; only report drift that would lead a developer or agent to a wrong action.' },
]

function chunkPrompt(c) {
  return `${RULES}

YOUR ASSIGNMENT: audit chunk "${c.key}" (${c.title}).
Files (read every line of each):
${c.files.map(f => '- ' + f).join('\n')}
${c.focus ? 'Area notes: ' + c.focus : ''}

Find BOTH kinds of problem:
1. Defects: logic errors, wrong conditions, off-by-one, unhandled edge cases (empty/huge/non-UTF-8/unicode input, missing files, concurrent modification), panics on real input (unwrap/expect/indexing/slicing at non-char boundaries; arithmetic overflow, since overflow checks are ON in the dev profile the app actually runs in), swallowed errors, races/TOCTOU, deadlocks, invariant violations, resource leaks and unbounded growth, stale state, security problems, data loss, wrong SQL, broken cancellation/timeouts.
2. Code that is bad, could be better, or is overengineered: dead code, duplicated logic (name both locations), abstractions with a single use, needless indirection or generality, reimplementing std or an existing dependency, functions so long or tangled they hide bugs, comments that contradict the code, needless clones/allocations on hot paths, O(n^2) where n can be large. For each, name the simpler design and what it removes.

Pedantic clippy output for the workspace is at ${PEDANTIC} (short format, one warning per line). Grep it for your files: cast truncation/wrap/sign-loss warnings can hide real bugs where they touch lengths, sizes, offsets, timestamps or ids; check those. Do not report the lints themselves.

Be exhaustive within your files. A typical 100 KB chunk of this codebase yields 8 to 25 genuine findings; do not pad with trivia and do not stop early. Return findings via StructuredOutput; put any file or region you could not fully read in coverage_note.`
}

function testPrompt(c) {
  return `${RULES}

YOUR ASSIGNMENT: audit chunk "${c.key}" (${c.title}). This is test or harness code.
Files (read every line of each):
${c.files.map(f => '- ' + f).join('\n')}
${c.focus ? 'Area notes: ' + c.focus : ''}

Find:
1. Test-quality problems: tests that cannot fail or assert nothing meaningful; assertions that check a mock instead of the code; timing-dependent sleeps/polls that will flake on a loaded machine or CI; shared global state between tests that run in parallel (env vars like HOME/XDG_*, current dir, fixed ports/socket paths, fixed temp names); tests that touch the real user's data (~/.local/share/relay-v4, real engine sockets, real provider binaries, the network); leaked processes, threads or temp dirs; misleading names; copy-pasted fixtures that should be one helper; dead helpers.
2. Real bugs in the production code that these tests reveal or encode (an assertion pinning wrong behavior, a test that works around a bug): follow into the code under test and report the bug at its production location.
3. Significant coverage gaps only where a critical path visibly has no test (name the path), not a generic "add more tests".
4. For harness code compiled into or shipped with the app: whether it can be triggered in normal use.

Return findings via StructuredOutput; put anything you could not fully read in coverage_note.`
}

function lensPrompt(l) {
  return `${RULES}

YOUR ASSIGNMENT: cross-cutting sweep "${l.key}" over the whole workspace except apps/relay-mobile. Other auditors are reading every file chunk by chunk; your value is following ONE concern across module and crate boundaries, which a chunk reader cannot.
Concern: ${l.concern}
Method: enumerate the relevant sites systematically with Grep and keep a list; inspect each; follow data and control flow across files. Report only real problems with the evidence standard above. When the same mistake repeats, report one finding per distinct root cause and list every location in "related".
Return findings via StructuredOutput; in coverage_note list the search patterns you used and any class of site you could not finish.`
}

function verifyPrompt(batch, origin) {
  return `${RULES}

YOUR ROLE: adversarial verifier. Below are ${batch.length} candidate findings from another auditor (${origin}). Auditors over-report; kill what is wrong and calibrate what is right. For EACH finding:
1. Open the cited code yourself and read enough surrounding code and callers to judge independently. Do not trust the finding's quotes or line numbers.
2. Try to refute it: does the code really do what is claimed? Is the trigger reachable (callers, guards, validation, transactions, locks upstream)? Is it handled elsewhere? Is it an intended, documented, harmless decision? For quality findings: is the duplication/overengineering real, and would the proposed change genuinely be simpler without losing something the code needs?
3. Verdict: "confirmed" (you independently established it, possibly with corrections), "plausible" (probably real, but the trigger or impact could not be fully established), or "refuted" (wrong, unreachable, already handled, or not worth changing). Between plausible and refuted, choose refuted.
4. Re-rate severity with the rubric, calibrated to real impact in this app rather than the auditor's enthusiasm, and fix the category if wrong. Put corrections (right line, real mechanism, better fix) in "correction".

FINDINGS
${batch.map(f => JSON.stringify(f)).join('\n\n')}

Return exactly one verdict per finding id via StructuredOutput.`
}

function skepticPrompt(f, v1) {
  return `${RULES}

YOUR ROLE: final skeptic for a finding a first verifier rated ${v1.severity} / ${v1.verdict}. High-severity claims carry the report, and a wrong one destroys its credibility. Default to refuting unless you establish it yourself from the code.
Answer for yourself: (a) Does the code do what is claimed? (b) Is the trigger reachable in real use of this app, by whom, how often? (c) What exactly happens to the user or their data when it fires? (d) Is the severity right per the rubric? (e) Is the proposed fix correct and complete, or would it break something? Put the answers to (b)-(e) in "reason" and any corrections in "correction".

FINDING
${JSON.stringify(f)}

FIRST VERIFIER
${JSON.stringify(v1)}

Return your verdict for id ${f.id} via StructuredOutput.`
}

function byFileLine(a, b) {
  if (a.file < b.file) return -1
  if (a.file > b.file) return 1
  return (a.line || 0) - (b.line || 0)
}

function finalize(m) {
  let status = 'unverified'
  let severity = m.severity
  let category = m.category
  if (m.v1) {
    severity = m.v1.severity
    category = m.v1.category
    if (m.v1.verdict === 'refuted') status = 'refuted'
    else if (m.v2) {
      if (m.v2.verdict === 'refuted') status = 'disputed'
      else { status = m.v2.verdict; severity = m.v2.severity; category = m.v2.category }
    } else status = m.v1.verdict
  }
  return { ...m, status, final_severity: severity, final_category: category }
}

async function verifyUnit(u, fs) {
  const sorted = [...fs].sort(byFileLine)
  const batches = []
  for (let i = 0; i < sorted.length; i += 5) batches.push(sorted.slice(i, i + 5))
  const vmap = {}
  await parallel(batches.map((b, bi) => async () => {
    let pending = b
    for (let attempt = 0; attempt < 2 && pending.length; attempt++) {
      const r = await agent(verifyPrompt(pending, u.key), { label: `verify:${u.key}:${bi + 1}${attempt ? 'r' : ''}`, phase: 'Verify', schema: VERDICTS, effort: 'high' })
      if (r && r.verdicts) r.verdicts.forEach(v => { if (pending.some(p => p.id === v.id)) vmap[v.id] = v })
      pending = pending.filter(p => !vmap[p.id])
    }
  }))
  const merged = fs.map(f => ({ ...f, v1: vmap[f.id] || null, v2: null }))
  const hi = merged.filter(m => m.v1 && m.v1.verdict !== 'refuted' && (m.v1.severity === 'critical' || m.v1.severity === 'high'))
  await parallel(hi.map(m => async () => {
    const { v1, v2, ...f } = m
    m.v2 = await agent(skepticPrompt(f, v1), { label: `skeptic:${m.id}`, phase: 'Verify', schema: VERDICT })
  }))
  return merged.map(finalize)
}

const UNITS = [
  ...CHUNKS.map(c => ({ ...c, kind: 'chunk' })),
  ...LENSES.map(l => ({ ...l, kind: 'lens' })),
]

phase('Audit')
log(`${CHUNKS.length} chunk auditors + ${LENSES.length} lens sweeps`)

const results = await pipeline(
  UNITS,
  u => agent(u.kind === 'lens' ? lensPrompt(u) : (u.tests ? testPrompt(u) : chunkPrompt(u)), { label: `audit:${u.key}`, phase: 'Audit', schema: FINDINGS }),
  async (res, u) => {
    if (!res) {
      log(`audit:${u.key} FAILED (no result)`)
      return { unit: u.key, kind: u.kind, failed: true, coverage: 'auditor failed', findings: [] }
    }
    const raw = (res.findings || []).filter(f => !(f.file || '').startsWith('apps/relay-mobile'))
    const fs = raw.map((f, i) => ({ ...f, id: `${u.key}#${i + 1}`, source: u.key }))
    log(`audit:${u.key}: ${fs.length} candidate findings`)
    const fin = fs.length ? await verifyUnit(u, fs) : []
    const kept = fin.filter(f => f.status !== 'refuted').length
    log(`verify:${u.key}: ${kept}/${fs.length} survived`)
    return { unit: u.key, kind: u.kind, coverage: res.coverage_note, findings: fin }
  },
)

const all = results.filter(Boolean).flatMap(r => r.findings)
const count = (pred) => all.filter(pred).length
const summary = {
  units: results.filter(Boolean).length,
  failed_units: results.filter(r => !r || r.failed).map((r, i) => r ? r.unit : UNITS[i].key),
  candidates: all.length,
  confirmed: count(f => f.status === 'confirmed'),
  plausible: count(f => f.status === 'plausible'),
  disputed: count(f => f.status === 'disputed'),
  refuted: count(f => f.status === 'refuted'),
  unverified: count(f => f.status === 'unverified'),
  by_severity: SEV.map(s => [s, count(f => (f.status === 'confirmed' || f.status === 'plausible') && f.final_severity === s)]),
}
log(`done: ${summary.confirmed} confirmed, ${summary.plausible} plausible, ${summary.disputed} disputed, ${summary.refuted} refuted`)
return { summary, results }
