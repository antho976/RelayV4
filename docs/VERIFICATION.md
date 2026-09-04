# Native rebuild verification

Verified on 2026-09-04 on the local Linux desktop, using GTK 4.22.4,
VTE 0.84.1 and GtkSourceView 5.20.0.

## UI parity follow-up

The shell now follows the pinned Relay-2 TopBar, Sidebar, TerminalPane,
StatusBar and Board card geometry more closely. Page navigation moved into
the sidebar, projects are grouped by workspace, used icons preserve Relay-2's
SVG paths, terminal labels sit above the screens, and state uses small tally
lights. The Board has full-height bordered lanes, task IDs and agent rows.
The launch sheet overlays the wall instead of expanding a compact window.

Follow-up checks: native build, both native unit tests, native Clippy with
warnings denied, formatting and diff whitespace checks. The display harness
covers eight captures at 1440×900 and 1024×768, including three columns with the
sidebar hidden and the compact launch sheet. Assertions cover actual capture
size, long project names, sidebar toggling, all three layout controls, retained
terminal widgets, a visible launch form and disabled background controls.
The existing six-provider paste/echo, window-close survival and editor-save
checks remain in the harness. Run one affected view with
`python3 scripts/native-smoke.py launch`, or omit arguments for the whole run.

The generic design detector flags the selected project's 2px white edge;
this intentionally matches Relay-2 Sidebar.svelte. There are no stylesheet
parsing errors. The host AT-SPI registry failure remains, so screen-reader
operation is still unverified.

This is a UI parity increment. Missing native pages and richer Relay-2 flows
listed below still prevent a full 1:1 claim. GTK window controls and editor
highlighting still use native styling.

## Automated checks

| Check | Result |
| --- | --- |
| `cargo build -p relay-native -p relay-cli --offline` | Passed |
| `cargo test --workspace --offline` | 162 passed, 1 ignored |
| `cargo fmt -p relay-native --check` | Passed |
| `cargo clippy -p relay-native --all-targets --offline -- -D warnings` | Passed |
| Generated bus schema drift test | Passed with `guardrail.hold.get` included |
| `python3 scripts/native-smoke.py` | Passed |

Tests cover the inherited agent surface, mailbox, priority hints, claims,
role gates, review groups, guardrails, sessions and task lifecycle. The new
hold-inspection assertions verify the frozen payload, open state, token
removal and agent refusal. Native tests cover response correlation with
interleaved events, cancellation on disconnect, and terminal sequence handling.

This machine has `/tmp/.git`, which contaminates an inherited workspace
discovery test. The successful full run used `TMPDIR=/dev/shm`; no production
behavior or assertions were changed to conceal that environment condition.
The tests need permission to create local sockets and launch fixture processes.

## Display and interaction evidence

The smoke harness starts an isolated engine with a temporary repository and
six fake provider processes. No paid provider or user store is used.

- Captured the terminal wall at 1440×900 and 1024×768.
- Captured Board, Mailbox, Guardrails and Code. Code includes a loaded,
  highlighted README and an edit saved through `file.write`.
- Sent paste text through each VTE widget and verified the provider's echo in
  engine scrollback for all six sessions.
- Asserted the editor disables editing, Save and Discard during load/save.
- Confirmed all six sessions remain running after native windows close, then
  parked the disposable fixture processes.
- The final CSS produced no parsing errors and the application did not panic.

Screenshots and logs are local artifacts in `.impeccable/review/`. The host's
AT-SPI registry service failed to activate and GTK logged accessibility-bus
warnings. Screen-reader operation is therefore **not verified**. No desktop
accessibility settings were changed.

## Performance evidence and limits

The final smoke run measured the native client, with six quiet fixture
terminals, for one three-second interval after startup: **0.0% CPU at the
kernel tick resolution, 213.8 MiB RSS**. An earlier run measured 276.1 MiB RSS.
These are short debug-build observations, not stable benchmark distributions.
They exclude the engine and agent processes. The fixture's output is quiet,
so this does not establish throughput, input-latency percentiles, startup
budgets, or eleven-pane sustained streaming performance.

The code enforces bounded queues, separate terminal/control connections,
output-driven drains and event-driven refresh. Those properties support the
performance direction but are not substituted for measurements.

## Review corrections

Independent review identified four asynchronous data-loss paths. The fixes
freeze the editor during file operations, check revision/project identity
before applying results, retain dirty state when a discard reload fails,
recheck dirty state after repository registration, and preserve form input
entered while a previous submission is awaiting a response.
The reviewer scored all four findings resolved after the fix batch and
recapture. That verdict covers those findings, not full application readiness.

## Scope still ahead

This is the rebuilt native foundation, not a finished parity or release claim.
The archived V3 roadmap remains intact for full docking/undocking, saved window
layouts, richer task editing, worktree-aware Code/Git tools, Notes satellite,
skills/settings, devices and packaging. Engine behavior and agent contracts
carry over; native widgets for every existing operation do not yet exist.

Editor saves detect existing external changes but the read/write pair is not
an atomic compare-and-swap against arbitrary external filesystem writers.
The current editor targets the project checkout and limits editable reads to
1 MiB. Real-provider TUI interactions, prolonged streaming, full accessibility
and daily-driver acceptance remain unverified.
