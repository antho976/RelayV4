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

The follow-up removes separate task, module, skill, registry, Git and build-log
windows. Notes and the emulator remain detachable. Task details use the content
page, with editing and relationships side by side at desktop width and stacked
at compact width. The command palette and layout menu remain inside the shell.
The launch sheet matches the reference's 780px width, provider SVGs, choice cards
and fixed action footer. Layout saves are serialized and coalesced per project
so rapid Grid/Focus/Review changes persist the latest selection.

## Automated checks

| Check | Result |
| --- | --- |
| `cargo build -p relay-native -p relay-cli --offline` | Passed |
| `cargo test --workspace --offline` | 173 passed, 1 ignored |
| `cargo fmt -p relay-native --check` | Not a check to rely on: the workspace is deliberately not rustfmt-clean (CLAUDE.md), so this is neither run in CI nor kept passing |
| `cargo clippy -p relay-native --all-targets --offline -- -D warnings -A deprecated` | Passed |
| Generated bus schema drift test | Passed |
| `python3 scripts/native-smoke.py` | Passed |
| `python3 scripts/test-launcher.py` | Passed. It broke when run.sh began registering the desktop entry and passed again once that step stopped blocking launch; it now stubs the installer, and CI runs it in the engine job |
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

GTK's ComboBoxText APIs remain supported in the pinned GTK4 version
but are deprecated. The native lint command permits those deprecation warnings;
this is not a clean default `-D warnings` result.

Tests requiring Unix sockets and fixture subprocesses ran outside the restricted
sandbox. Its synthetic `/tmp/.git` also changes one inherited workspace-discovery
fixture's ancestry. No production behavior or test expectation was changed to
hide those environment restrictions.

## Display and interaction evidence

The harness creates a disposable engine, store, repositories and fake providers.
It captures the wall at 1440×900 and 1024×768, plus Board, Mailbox, Guardrails,
Code, Notes, Plan, Modules, Settings, Skills, Dashboard, Notifications and Devices,
plus the command palette, layout menu and compact launch preview.

It verifies:

- Native task and note edits are saved through their real controls.
- Unsaved task dismissal is refused; app-owned flows keep one visible GTK window.
- In-app confirmations return the correct result for both accept and cancel.
- Rapid layout changes retain the final saved mode.
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

The earlier implementation's three-second sample with six quiet fixture terminals measured **0.0% CPU at
kernel tick resolution and 213.0 MiB RSS** for the debug native client. This is a
short observation, not a stable benchmark distribution; it excludes engine and
provider processes. The separate eleven-pane burst establishes completion within
its budget, not exact latency, sustained throughput, frame timing or losslessness
of every intermediate line.

Queues and rendered logs are bounded. Terminal/video streams use dedicated
connections; refreshes are event-driven, and device monitoring is released when
its view or connection closes. These code properties are not substituted for
measurements.

### Optimization pass

Two things were changed together: the build profiles, and the hot paths.

`run.sh` launches `target/debug`, so the dev profile is the profile the
application runs in. Dependencies now build at `opt-level = 3` and workspace
crates at `opt-level = 1`; debug assertions, overflow checks and debug info are
unchanged. `[profile.dev.package."*"]` also sets `OPT_LEVEL=3` for dependency
build scripts, so the bundled `sqlite3.c` is compiled optimized rather than at
`-O0`. Release builds add fat LTO and a single codegen unit.

Measured, on this workstation, with the same source compiled at the same
optimization level on both sides so that only the code differs:

Scrollback ring, at the optimization level the application used to run at:

| Path | Before | After |
| --- | --- | --- |
| Ring append, 4000 × 8 KiB through a full 8 MiB ring | 200.7 ms | 10.9 ms |
| Whole-ring read (`park`, `session.resume`) | 39.4 ms | 4.7 ms |
| Terminal attach catch-up, ×200 | 257.1 ms | 1.1 ms |

At `opt-level = 3` the same three changes are 1.9×, ~1× and 1.5×; the gap is the
profile, which is the other half of this pass. These are microbenchmarks of
`Ring` alone, not end-to-end terminal latency.

Engine request pipeline, in process, median of three interleaved rounds of the
same harness run against both trees, both at the new profile:

| Op | Before | After | |
| --- | --- | --- | --- |
| `bus.ping` | 1.85 µs | 1.63 µs | 1.1× |
| `session.list` | 18.20 µs | 2.03 µs | 9.0× |
| `settings.set` (audited mutation) | 109.63 µs | 84.00 µs | 1.3× |
| `Engine::parse`, 8 KiB payload | 2.25 µs | 1.40 µs | 1.6× |

`session.list` is dominated by re-compiling its built SQL on every call; it is
the op the shell re-runs on every `session.changed`.

Individual changes the two harnesses above do not isolate, old implementation
against new in one binary, at both profiles:

| Change | Old profile | New profile |
| --- | --- | --- |
| `pty` frame encode, 4 KiB | ~6–12× | ~2–3× |
| `pty` frame encode, 64 KiB | ~10× | ~2–2.5× |
| Socket line classify + parse, 4 KiB | 1.2× | 2.0× |
| Socket line classify + parse, 64 KiB | 1.1× | 1.0× |
| Hex of a 32-byte digest | 4.0× | 8.9× |
| Worktree disk walk, 3600 files, warm cache, 4 cores | 2.3× | 2.2× |
| Subprocess wait for a child that exits at once | 3.4× (5.45 → 1.62 ms) | 3.6× (5.43 → 1.49 ms) |

This machine is noisy enough that the frame-encode ratios moved between runs, so
they are given as ranges. The disk-walk figure is warm-cache on four cores and
says nothing about a cold one. The subprocess figure is the poll interval, not
the child: it is the latency every short `git status` or `adb devices` used to
be billed for.

What changed, and why each one is on a hot path:

- **Scrollback ring.** Trimming used one `pop_front` per byte, so a 64 KiB read
  cost 65 536 ring operations once the ring was full; reads copied element by
  element, and the frame index was scanned linearly. Trimming is now one bulk
  `drain`, reads copy through the deque's two contiguous halves, and the index
  is binary-searched. `session.scrollback {lines}` reads only the tail it asks
  for instead of materializing all 8 MiB.
- **Frame encoding.** `pty` frames are formatted straight into their wire line.
  Base64's alphabet contains nothing JSON must escape, so the payload is encoded
  once, in place, instead of being allocated, wrapped in a `Value` and re-scanned
  by `serde_json`. A test asserts the result is byte-identical to what `serde_json`
  produces for the same `Frame`, for empty, escaping, binary and 64 KiB payloads.
- **Socket writes** coalesce whatever is already queued into one `write_all`
  rather than one syscall per frame.
- **Request pipeline.** Envelopes parse once into `Request` instead of into a
  `Value` and then out of it again; payload validation and handler dispatch read
  *through* the request's payload instead of each cloning it; the post-commit
  `mail` sideband takes the store lock once rather than twice.
- **SQLite.** The statements every request runs — session by id and by name,
  project by id, the audit idempotency probe and insert, session state and
  scrollback — go through the connection's prepared-statement cache instead of
  re-compiling their SQL per call. The audit log hashes the payload copy it is
  about to store rather than serializing the payload a second time.
- **Guardrails** read a memoized default settings tree, cloning only the
  `guardrails` branch they overlay, on every audited agent mutation.
- **Subprocess waits** back off from 150 µs rather than sleeping a flat 5 ms, so
  a `git status` that finishes in 3 ms is not billed for 5.
- **Worktree disk sizing** walks with a shared work stack across threads.
- **Icons** cache their rendered paintable; they repaint on every state change,
  and each repaint re-parsed the same SVG.
- **The native client** recognizes a socket line's shape from its leading bytes
  and parses it once, where it used to stage every line through a
  `serde_json::Value` and then walk that into the envelope. Each envelope
  declares its distinguishing key second, so the prefix identifies the shape and
  proves the version; a line that does not match falls back to the old path, and
  a test in `relay-bus` asserts the three prefixes, so a reordered field turns
  the fast path off rather than breaking a reader.

### Cost while the machine is doing something else

Relay is meant to sit beside a game, a build or a call, so a second pass went
after what it spends in the background rather than what it spends per call.

It already has no idle cost worth naming: the only recurring timers in the
application are the opt-in wallpaper rotation and one-shot debounces, and the
resource sampler runs only while its panel is open. What remained was work that
runs whether or not anyone is looking at it.

- **Terminals detach when the window is not on screen.** Panes tracked the
  visible page and the pane layout, but not the window, so an agent printing
  output behind a fullscreen game still paid for every frame: socket read, JSON
  parse, VTE feed and a redraw. The compositor already reports this —
  `GDK_TOPLEVEL_STATE_SUSPENDED` for minimized, covered or another workspace,
  with `MINIMIZED` for backends that do not send it — and the engine keeps the
  scrollback ring regardless, so a hidden window detaches and re-attaches from
  its last sequence with bounded catch-up, the same path a reconnect takes.
- **Disk walks are background work and now run like it.** The walk takes at most
  four threads rather than up to eight, and its helpers run niced, so the
  scheduler hands the cores back the moment anything else wants them. The
  thirty-second worktree pass behind the resources panel moved off the blocking
  pool onto its own niced thread, so a pool thread is not left demoted for
  whatever runs there next. On Linux `setpriority(PRIO_PROCESS, 0, …)` is
  per-thread, so the rest of the process is unaffected.
- **Keystrokes no longer visit the blocking pool.** `session.input` against a
  live PTY touches no SQLite (D148) but still crossed to a pool thread and back
  to find that out. Measured at **24.7 µs per keystroke, 27.4 µs with the pool
  under load, against 0.024 µs for the work itself.** A door now answers such a
  request inline, gated on checks that are all lock-free, and only for writes
  small enough that they cannot park on the child's input queue. The gate
  decides where a request runs, never whether it is answered.

No claim is made about end-to-end frame latency, sustained throughput under
load, GTK rendering cost, or the effect on any particular game, none of which
this pass measured.

## Review and boundaries

The earlier implementation's independent review found and corrected partial-launch recovery, stale-write
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
