# P3: VTE terminal proof

The go/no-go test of the rebuild (brief §12, §14). One `vte4::Terminal` bound to a real
core-owned Relay PTY through the P2 bus client — VTE renders and takes input, the engine owns the
process, sequencing, scrollback, attach/detach, input and resize.

**Reading: GO.** VTE works over the existing core-owned PTY model with no lost or duplicated
output, clean detach/re-attach, correct coalesced resize, and stable reconnection across client
close, park/wake (new epoch) and bare relaunch. None of the brief §14 stop conditions were hit.
The proof never calls any VTE spawn function. Open items below are measurement (eleven-pane wall,
P4a) and two contract notes, not blockers.

Owner file: `apps/relay-native/src/terminal.rs` (`terminal::demo(session)`), dispatched by
`main.rs` for `relay-native --terminal <session>`. It consumes `bus_client::Client`
(`connect`/`request`/`attach`→`async_channel::Receiver<PtyFrame>`/`detach`/`Notice`) unchanged;
the only helper written locally is `socket_path()` (`$XDG_RUNTIME_DIR/relay/<instance>.sock`),
because the shell will own instance selection later (ARCHITECTURE §7) and I must not edit
`bus_client.rs`.

## Versions

| component | version | source |
|---|---|---|
| system libvte (`vte-2.91-gtk4`) | 0.84.1 | `pkg-config --modversion` |
| system GTK 4 | 4.22.4 | `pkg-config` |
| system GtkSourceView 5 | 5.20.0 | `pkg-config` |
| `vte4` / `vte4-sys` crate | 0.10.0 (feature `v0_84`) | `Cargo.lock` |
| `gtk4` / `gdk4` crate | 0.11.4 (feature `v4_22`) | `Cargo.lock` (D212) |
| `glib` / `pango` / `cairo-rs` | 0.22.9 | `Cargo.lock` |
| `async-channel` | 2.5.0 | `Cargo.lock` |
| `relay` engine | 4.0.0-dev (this repo, `403fa4a` = Relay 2 `1745dd3`) | `relay --version` |

## How it was driven

Headless `test` engine exactly as VERIFICATION §2 / READINESS require, with the five `RELAY_*`
session vars unset (and, deliberately, the `CLAUDE_*` vars unset when starting `serve`, so the
spawned provider is not treated as a nested child):

```
relay serve --instance test --store /tmp/claude-1000/relay3-p3/test-store.db &
relay q workspace.create '{"path":"/home/anthony/dev-2"}'
relay q project.add '{"workspace_id":1,"path":"/home/anthony/dev-2/Relay-V3"}'
relay q session.create '{"project_id":1,"provider":"claude"}'   # → coral-koala
relay q session.spawn  '{"session":"coral-koala"}'
RELAY_INSTANCE=test RUST_LOG=relay_native=info cargo run -p relay-native -- --terminal coral-koala
```

The real Wayland session is KDE/Wayland, whose screen is locked and cannot be screenshotted
non-interactively. To observe VTE's rendered output and inject real key/pointer/wheel/resize
events I ran a **nested compositor**: `kwin_wayland --virtual --xwayland --xwayland-display :7`
with a rootful `Xwayland :7` inside it, then ran the demo with `GDK_BACKEND=x11 DISPLAY=:7` and
drove it with XTEST (`python-xlib`) plus `import -window root` for screenshots. Keystrokes were
also injected server-side via `relay cmd session.input` to test the output path independently of
GTK. `dev` and `stable` instances were untouched; the `test` engine and nested display were killed
at the end (`session.close` first), leaving nothing running.

Verification harness added to the owned file (glib timers confined to it): `--self-check <secs>
[--dump <path>] [--drop-one] [--paste <text>]`. It forces one re-attach at half-time, optionally
injects a one-frame drop to exercise the gap path and a bracketed paste, then compares **every byte
fed to VTE** against `session.scrollback` and prints one JSON line of counters. Twenty-three unit
tests cover the sequence contract and option parsing (`cargo test -p relay-native`).

## The five brief §14 stop conditions

**1. Does VTE require owning the child? NO.** `terminal.rs` contains no `spawn_*` call. Output
comes only from `session.attach`→`feed`; input leaves only as `session.input`; size as
`session.resize`. The session's lifetime is independent of the pane, verified four ways:
`session.get` reports `running` after the client closes; the session survived `Ctrl+Shift+D`
detach and `Ctrl+Shift+R` re-attach; it survived `session.park`→`session.wake` (which the pane saw
as a new epoch); and a bare relaunch re-attached to the still-live PTY. VTE is a pure renderer and
input surface.

**2. Can the bus support a standalone client without a large redesign? YES.** The P2 `bus_client`
plus `session.{attach,detach,input,resize,scrollback}` were sufficient with no change. Frames route
per session carrying `(epoch, seq)`, decoded to bytes on the tokio side; the drain loop feeds them
on the GTK main thread. Nothing in the pane duplicates relay-core logic (stop condition 5 — the
only local code is a socket-path string).

**3. Native rendering for the terminal wall: NOT MEASURED here.** Eleven panes and idle CPU are
P4a. Single-pane rendering of Claude Code's live TUI (boxes, 256-colour, spinners, alternate
screen) was smooth on the nested compositor. No claim about the wall.

**4. Do required behaviours depend on inaccessible/unmaintained bindings? NO.** `vte4` 0.10 with
`v0_84` exposes everything used: `feed`, `commit`, `column_count`/`row_count`,
`char-size-changed`, `reset`, `set_scrollback_lines`, `text_format`/`text_range_format`,
`text_selected`, `vadjustment`. The one known sharp edge is D212's feature-level pin (already
recorded); nothing here needed a level above `v0_84` / `v4_22`.

**5. Does the spike duplicate relay-core logic? NO.** See condition 2.

## Output path — no lost or duplicated output

`attach` (no `(epoch, seq)` on first attach); the first frame is the bounded catch-up, fed with
`Terminal::feed`, then live frames in arrival order. The drain loop takes everything already queued
in one wake-up (`recv().await` then `try_recv()` to empty), so the socket reader is never waited on
(D211). A `SeqTracker` enforces the CONTRACTS §5 rule: within an epoch `seq` must be `last + 1`
(the catch-up frame is exempt — its `seq` is wherever the catch-up ends); a lower/equal `seq` or
older epoch is a duplicate and is **not** fed; a jump is a gap; a higher epoch resets the terminal
(`reset(true, true)`) before feeding.

**Byte comparison (every run, `fed_bytes` vs `session.scrollback`):**

| run | what | first frame (catch-up) | frames | bytes fed | fed == engine | fed is engine tail | gaps | dupes |
|---|---|---|---|---|---|---|---|---|
| initial attach | banner + a printf marker | 31 363 B | — | 31 363 | **true (exact)** | true | 0 | 0 |
| full matrix (run7) | unicode, paste, Ctrl-C, `seq 1 300`, resize burst, hide/move/detach/reattach, wake | 1 911 B | 369 | 175 723 | true where ring ≤ bound | true | 1 (injected) | 0 |
| relaunch, small ring (run9) | bare relaunch | 96 417 B | — | 96 453 | **true (exact)** | true | 0 | 0 |
| relaunch, ring > 256 KiB (run9b) | bare relaunch after a 476 KB ring | **260 721 B (bounded)** | — | 263 739 | false | **true** | 0 | 0 |

The catch-up is `ATTACH_CATCHUP_BYTES` (256 KiB) starting on a frame boundary (`pty.rs`). When the
ring is smaller than that, the pane holds a **byte-exact** copy of engine scrollback. When the ring
is larger (run9b: engine held 479 358 B), the catch-up covered the last **260 721 B** and the
remaining **215 619 B of older history were not redrawn** — `fed_is_engine_tail: true` confirms
what VTE shows is an exact suffix of engine truth, and `session.scrollback` still returns the full
479 358 B. This is the intended split (ARCHITECTURE §5: "the spike proves engine-owned history is
not lost, not that every byte is redrawn"); wiring the remainder into VTE's scrollback via
`session.scrollback` is the Stage 2 question.

**Injected gap → re-attach (run7), from the logs:**

```
self-check fault injection: frame discarded before the tracker saw it   epoch=1 seq=247 bytes=528
seq gap: frames lost between engine and pane; re-attaching from the last good pair  epoch=1 seq=248 expected=247
re-attaching why="seq gap" from=Some((1, 246))
attached session=coral-koala from=Some((1, 246)) generation=3
first frame after attach ...  epoch=1 seq=248 bytes=586      # engine replayed 247–248 as the catch-up
```

After the re-attach the stream continued with no duplicate and no further gap; the final compare
was `fed_is_engine_tail: true`. A gap is handled as a re-attach, never as corruption (D211). Each
attach bumps a generation counter so a superseded drain loop cannot feed a stale frame.

**New epoch (park→wake, run7):** `session.wake` produced epoch 2; the pane logged `new epoch:
resetting the terminal before feeding epoch=2`, cleared the byte record, and fed the fresh
catch-up (88 131 B) — screenshot shows the terminal correctly showing the woken session's history,
not the old epoch's tail.

## Input path — no stuck input, no local echo

VTE's `commit` signal (already translated: arrows, modifiers, bracketed paste, control sequences,
and VTE's own answers to terminal queries) is forwarded verbatim through one ordered
`async_channel` to `session.input {session, data}`. No local echo: the PTY echoes, and the
byte-exact `fed == engine` result above is the proof that nothing was echoed twice. Observed
commit payloads (from the debug log):

| input | bytes on the wire |
|---|---|
| arrows ↑ ↓ ← → | `ESC[A` `ESC[B` `ESC[D` `ESC[C` |
| Ctrl-C | `\x03` |
| Backspace | `\x08` |
| Shift+PageUp / PageDown | `ESC[5;2~` / `ESC[6;2~` (app-driven; see scroll note) |
| bracketed multi-line paste | one commit: `ESC[200~pasted line one\rpasted line two ESC[201~` |
| VTE answering DA/DSR queries | `ESC[?61;1;21;22;28c`, `DCS>|VTE(8401)ST`, `ESC[?2026;4$y` |

`input_acks == input_commits`, `input_errors: 0` across runs (195/195 in run7). The one time inputs
were refused was correct: after the session had `exited`, `session.input` returned
`conflict/session.exited` and the pane counted it — no stuck input, a typed refusal surfaced.

The bracketed multi-line paste is worth noting: Claude Code received `pasted line one` +
`pasted line two` as one block and reported the `!` shell prefix did **not** apply to it — i.e. the
paste round-tripped intact (screenshot `s7C`).

## Resize path — coalesced, never polled

Two triggers mark one pending `idle_add_local_once` callback: VTE's `char-size-changed` and the
toplevel `GdkSurface`'s `layout` signal. The callback reads `column_count()`/`row_count()` and
sends `session.resize {cols, rows}` **only when the pair changed** (`sent_size`). There are no
timers outside `--self-check`.

- run7: **388 resize triggers → 17 sends.**
- run8c (110 real window-configure steps during live output): **432 triggers → 118 sends** (one
  per settled size; unchanged pairs sent nothing).

On every (re-)attach the pane resets `sent_size` and re-asserts the widget's size, so the PTY
matches the pane even though a prior attach had sized it. Full-screen redraw during a resize burst
was correct (screenshots `s7D`, `sT-vim-resized`), including the terminal reset on a new epoch.

## Scroll ownership

No outer `ScrolledWindow`; the `vte4::Terminal` is a direct child, owns wheel / touchpad /
Shift+PageUp-Down, with `set_scrollback_lines(10_000)`. `set_scroll_on_output(false)` so live
output does not yank a reader off their scroll position; `set_scroll_on_keystroke(true)`. One
obvious scroll owner under the pointer (brief §18) — satisfied structurally.

## Interaction matrix (VERIFICATION §6)

| row | result | evidence |
|---|---|---|
| normal shell prompt | **gap** | no shell-only provider on this bus (enum is `claude`\|`codex`); see contract note 1 |
| Claude Code | **observed** | renders fully — boxes, 256-colour, spinners, alternate screen; screenshots `s7A`–`s7H` |
| Codex CLI | not observed | only a `claude` session driven; `codex` is installed and uses the identical PTY path, but untested |
| Kimi / Qwen | not observed | not installed; not in the provider enum |
| mouse selection and copy | **human-confirm** | Claude Code enables mouse reporting, so VTE forwards drags to the PTY as `ESC[<…M`; a synthetic Shift+drag returned an empty selection. Needs a human to confirm Shift-override selection + Ctrl+Shift+C on the real display |
| paste (bracketed, multi-line) | **observed** | `ESC[200~…ESC[201~` single commit, two lines verbatim (`s7C`) |
| wheel and touchpad scrolling | **partial / human-confirm** | wheel events delivered to VTE (6 mouse reports in run7); with the app's mouse-mode on they go to the app. Native VTE wheel/touchpad scroll of scrollback = human-confirm |
| Shift+PageUp / PageDown | **partial / human-confirm** | delivered as commits `ESC[5;2~`/`ESC[6;2~` (the app consumed them); VTE-local scrollback paging = human-confirm |
| search | not enabled | no search UI in the spike |
| full-screen TUI (htop, vim) | **partial + gap** | Claude Code's own alt-screen TUI (`ESC[?1049h` seen) renders and redraws across resize; a *standalone* htop/vim could not be driven — see contract note 1 |
| rapid output | **observed** | `! seq 1 300`: 369 frames / 175 KB, `fed == engine`, 0 gaps/dupes |
| Unicode, emoji, combining, wide | **observed** | `UNI-😀-é-中文-end` rendered correctly — emoji, combining acute, wide CJK (`s7B`) |
| window resize during output | **observed** | 110-step resize burst during live output, 0 gaps, correct redraw |
| hide, show, move, detach, reattach | **observed** | `Ctrl+Shift+H` hide/show and `Ctrl+Shift+M` move keep the attachment; `Ctrl+Shift+D`/`R` detach and re-attach from `(epoch, seq)` (`s7E`, `s7F`, `s7G`) |
| close client while engine owns a live session | **observed** | `close-request` detaches then destroys; `session.get` afterwards = `running` |
| relaunch and restore session | **observed** | bare relaunch re-attaches; catch-up bounded at 256 KiB; full history via `session.scrollback` |

Screenshots are in `/tmp/claude-1000/relay3-p3/s7*.png`, `sT-vim*.png` (scratch; not committed).

## What a human must still confirm on the real display

- Glyph rendering fidelity and font (nested runs used the fontconfig default, not the desktop's
  `Hack 10`).
- Mouse selection + copy (Shift-override while an app has mouse-mode on), and paste via
  Ctrl+Shift+V.
- Touchpad kinetic scrolling and wheel scrolling of VTE's own 10 000-line scrollback.
- Perceived smoothness — the nested compositor is not a performance measurement.

## Contract gaps / notes (diagnose, do not paper over)

1. **No shell-only session, and the provider `!` prefix has no controlling tty.** Providers are
   `claude` | `codex` only; there is no plain-shell session on this bus, so the "normal shell
   prompt" and "standalone full-screen TUI" rows cannot be driven directly. Claude Code's `!`
   shell prefix runs commands through pipes — `htop`/`vim` printed *"Input is not from a
   terminal"*. This is a **verification** gap, not a VTE gap: VTE renders Claude Code's own
   full-screen TUI (alternate screen included) correctly. A future shell provider, or a PTY
   pass-through for `!`, would let the standalone-TUI rows be exercised.
2. **Catch-up is bounded at 256 KiB; older history needs `session.scrollback`.** Quantified above
   (run9b: 260 721 B redrawn, 215 619 B not). Engine-owned history is not lost. Feeding the
   remainder into VTE's scrollback on relaunch is a Stage 2 decision.
3. **`session.attach` base64-encodes bytes on the socket** (CONTRACTS §11, PERFORMANCE F2). Not a
   problem at one pane; measure the cost at eleven panes in P4a before deciding it matters.
4. Minor: `session.resize` takes `u16` cols/rows; the pane clamps `column_count`/`row_count`
   accordingly. No issue at realistic sizes.

## Verification run

- `cargo check -p relay-native` — clean.
- `cargo clippy -p relay-native --all-targets -- -D warnings` — clean.
- `cargo fmt -p relay-native --check` — clean.
- `cargo test -p relay-native` — 23 passed (6 of them the `SeqTracker` contract).
- `git diff --check` — clean.

## Next smallest slice

Wire this pane into the shell (`app.rs`) as a real pane surface behind one session, so P4a can put
eleven of them on the wall and measure startup, idle CPU and streaming smoothness (brief §6.6) —
the one proceed-gate item P3 deliberately did not measure. That slice should also settle contract
note 2 (relaunch scrollback fill) and note 3 (base64 cost at eleven panes).
