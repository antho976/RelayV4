# Native parity verification

Verified on 2026-09-04 on the local Linux desktop, using GTK 4.22.4,
VTE 0.84.1 and GtkSourceView 5.20.0.

## Integration of the UI parity pass

The UI pass is integrated with the expanded native workflows. The merge retains
Relay-V4's isolated engine/store, launch recovery, task/note editing, device
ownership, saved layouts and worktree tools. Compact shell styling, CSS-colored
Relay-2 icons, top terminal metadata, Board card styling and the launch overlay
are adapted to those implementations. The smoke harness includes long repository
names, retained panes across Grid/Focus/Review, exact screenshot dimensions, and
a compact launch preview alongside the upstream edit/launch/burst checks.

## Automated checks

| Check | Result |
| --- | --- |
| `cargo build -p relay-native -p relay-cli --offline` | Passed |
| `cargo test --workspace --offline` | 173 passed, 1 ignored |
| `cargo fmt -p relay-native --check` | Passed |
| `cargo clippy -p relay-native --all-targets --offline -- -D warnings -A deprecated` | Passed |
| Generated bus schema drift test | Passed |
| `python3 scripts/native-smoke.py` | Passed |
| `python3 scripts/test-launcher.py` | Passed |
| `python3 scripts/test-launcher-real.py` | Passed |

The launcher regression test opens the actual GTK window twice with an isolated
engine and a legacy socket present. Both windows verify their engine connection,
the second launch reuses the same process, and all data stays under `relay-v4`.
A stale `RELAY_NATIVE_SOCKET` is cleared. The workspace suite was rerun after
separating V4's engine socket and store from Relay-2/V3.

The inherited engine tests cover agent roles, mailbox priority, review groups,
claims, guardrails, task approvals, provider lifecycle and native bus contracts.
New tests cover atomic expected-field updates (including concurrent writers),
expected-content file saves, assignment recovery after partial launch, mirror
socket cleanup, watcher ownership and input-overflow reset. Native tests also
cover transport correlation, terminal sequence handling, draft conflicts,
shortcuts, decoder dimensions and generated audio samples.

GTK's ComboBoxText and Dialog APIs remain supported in the pinned GTK4 version
but are deprecated. The native lint command permits those deprecation warnings;
this is not a clean default `-D warnings` result.

Tests requiring Unix sockets and fixture subprocesses ran outside the restricted
sandbox. Its synthetic `/tmp/.git` also changes one inherited workspace-discovery
fixture's ancestry. No production behavior or test expectation was changed to
hide those environment restrictions.

## Display and interaction evidence

The harness creates a disposable engine, store, repositories and fake providers.
It captures the wall at 1440×900 and 1024×768, plus Board, Mailbox, Guardrails,
Code, Notes, Plan, Modules, Settings, Skills, Dashboard, Notifications and Devices.

It verifies:

- Native task and note edits are saved through their real controls.
- GtkSourceView saves an edited README into the selected checkout.
- Six initial VTE widgets send paste input and receive provider echoes.
- The native launch sheet creates two builders and a reviewer with one shared
  worktree and a staged task queue.
- All eleven fixture sessions remain alive after native windows close.
- Eleven simultaneous bursts of 2,048 lines each reach native VTE tail markers
  within a five-second completion budget. This checks delivery to the renderer,
  not only bytes received by the engine.

Screenshots/logs are local artifacts in `.impeccable/review/`. The host AT-SPI
registry failed to activate, so screen-reader operation remains unverified.
No desktop accessibility settings were changed.

## Performance evidence and limits

A three-second sample with six quiet fixture terminals measured **0.0% CPU at
kernel tick resolution and 213.0 MiB RSS** for the debug native client. This is a
short observation, not a stable benchmark distribution; it excludes engine and
provider processes. The separate eleven-pane burst establishes completion within
its budget, not exact latency, sustained throughput, frame timing or losslessness
of every intermediate line.

Queues and rendered logs are bounded. Terminal/video streams use dedicated
connections; refreshes are event-driven, and device monitoring is released when
its view or connection closes. These code properties are not substituted for
measurements.

## Review and boundaries

Independent review found and corrected partial-launch recovery, stale-write
races, hidden note-window shutdown, cross-project navigation, device form
initialization, compact pane sizing and dropped mirror touch-release events.
The final bounded review cleared those fixes and the corrected icon geometry
in the desktop and compact captures, with no introduced regression found.
The full current surface is not certified as a pixel-identical Relay-2 clone.

Physical Android operation, release signing/upload, real-provider TUIs,
wallpaper file-picker interaction, actual audio output and prolonged runtime
remain unverified. File editing remains bounded to 1 MiB, and the external-file
check cannot prevent a separate non-Relay process writing between verification
and rename. [PARITY.md](PARITY.md) separates native coverage from later roadmap work.
