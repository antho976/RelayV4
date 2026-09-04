# Native rebuild verification

Verified on 2026-09-04 on the local Linux desktop, using GTK 4.22.4,
VTE 0.84.1 and GtkSourceView 5.20.0.

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
