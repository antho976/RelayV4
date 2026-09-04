# Relay 3 native spike: results so far

Measured through the repaired P4a, 2026-09-04. The spike passes its proceed gate: the eleven-pane
wall carried all eleven simultaneous 50 MiB streams without lost or duplicated frames. Everything
below was observed on this machine; the per-phase documents carry the commands and numbers.

| phase | status | evidence |
|---|---|---|
| P1 shell | done, accepted by Antho on the display | `apps/relay-native/src/app.rs`, `--self-test` logs |
| P2 bus client | done | `src/bus_client.rs`, 11 fixture tests |
| shell wiring | done against the live `dev` engine | connect, `app.status`, `bus.subscribe`, events, explicit reconnect, ARCHITECTURE §7 shutdown, SIGTERM and disconnected paths, from logs |
| P3 VTE terminal | done, **GO** on every §14 stop condition | `docs/spike/P3-terminal.md` |
| P4b GtkSourceView | done | `docs/spike/P4b-editor.md` |
| P4a eleven-pane wall | done, **GO** | `docs/spike/P4a-terminal-wall.md`: one 50 MiB burst per pane completed with zero gaps, duplicates, re-attaches, or subscriber lag |

## Native dependency versions

GTK 4.22.4 (gtk4 0.11.4, `v4_22`) · VTE 0.84.1 (vte4 0.10.0, `v0_84`) · GtkSourceView 5.20.0
(sourceview5 0.11.2, `v5_18`) · glib 2.88 (glib-rs 0.22.9) · rustc 1.97.1. D212.

## Brief §14 proceed gate, item by item

| condition | state |
|---|---|
| VTE works with the core-owned PTY model | yes: no spawn call; byte-exact output within the catch-up bound; seq gap → re-attach verified; session survived client close, park/wake and relaunch |
| socket client handles responses, events and frames concurrently | yes: P2 tests and the live shell |
| eleven panes feasible | yes under the required stream load: 9,009 frames and 588,988,257 post-PTY bytes reached VTE, with zero gaps or duplicates |
| GtkSourceView covers the editor behaviour | yes for every §13.2 bullet; colours and gutter need a human look |
| window and focus behaviour stable | yes for the P1 cycles; mouse-driven resize and transient stacking need a human look |
| no second business-logic implementation | yes: the client holds no relay-core logic; every action is a bus op |
| Relay 2 intact | yes: untouched except the patch offered in `docs/patches/` |

The original raw-read framing overflowed both the engine subscriber and standalone-client queues.
Coalescing at the PTY producer boundary now keeps the same bounded queues and sequence contract
without drops. No dependency or bus-schema change was needed.

## Decision: proceed, accepted

Antho accepted the proceed recommendation on 2026-09-04 by authorizing work through the first
viewable Stage 1 checkpoint. Relay 3 continues with GTK 4, VTE, GtkSourceView, the socket door,
and a separately lived `relay serve` process (D214). The repaired exact-load result clears the
technical stop gate; real-display visual, input-latency, and assistive-technology checks remain
validation work, not dependency blockers.

## Findings that change later stages

- **No `ui.*` executor door exists** in the engine (D210). The client mirrors `ui.changed`.
- **Attach catch-up is bounded at 256 KiB.** Older history is not redrawn on relaunch; the engine
  still holds it and `session.scrollback` returns it. Stage 2 wires scrollback into reattach.
- **No shell-only provider.** Sessions are Claude or Codex; the "normal shell prompt" and
  standalone-TUI matrix rows cannot be driven through the bus. A verification gap, and a product
  question for Antho: does Relay 3 want a plain terminal session type?
- **Backpressure is repaired at wall scale.** PTY raw reads are coalesced for 17 ms and capped at
  64 KiB before sequence assignment and broadcast. The repaired exact load delivered 819 frames
  and 53,544,387 bytes to every pane with zero gaps, duplicates, re-attaches, client drops, or
  engine subscriber lag. The bus schema and D211's non-blocking client rule are unchanged.
- **P4a process measurements:** native RSS was 195,480 KiB before the repaired burst and 201,164
  KiB peak/settled. The native process used 28.0–29.9% CPU for fourteen one-second samples and
  21.0% on the fifteenth; the engine used 17.0–26.0%. Both were idle after about 15 seconds.
  Engine RSS grew from 20,864 KiB to 203,988 KiB settled, including eleven retained 8 MiB
  scrollback rings. Idle native CPU remains 0.0% across the earlier 13 five-second samples.
- **Hide/reveal lifecycle:** the first automated click exposed a re-entrant `RefCell` panic when
  GTK synchronously emitted `map`. The pane now drops its borrow before changing visibility; the
  repeated 60 fps capture passed. Finish review then found retained ownership after Remove and
  ambiguous repeated controls; Remove now releases shell ownership, pane signal callbacks are
  weak, controls have session-qualified accessible labels, and focus moves to an adjacent pane.
  The ownership/accessibility follow-up has compile/test coverage but no repeated display or AT pass.
- **`file.read` stalled twice** during P4b (90 s and 40 s) while the CLI answered in between; not
  reproduced afterwards. Recorded as a Relay 2 engine item to diagnose (socket door
  `spawn_blocking` starvation or `git worktree list` in root verification are the candidates).
- **Guardrail thresholds on `dev` are inert** (`min_removed_lines: 4000000`), so a destructive
  write through the editor was allowed; the `held` path was proven with the 64 MiB comparison cap
  instead, and hold 15 was left open on `dev` for Antho to see the flow.
- **glib-rs 0.22 has no Unix signal binding**; the shell waits for SIGTERM on tokio and crosses to
  the main thread.
- **`file.not_found` is kind `unavailable`**, `max_bytes` truncation can split a UTF-8 character,
  and `held` carries no `hint`: small bus contract items for Relay 2.

## What a human must still confirm on the display

Glyph rendering and colours in VTE and GtkSourceView; mouse selection and copy; touchpad and
wheel scrolling of VTE-local scrollback; Paned drag and transient stacking of the secondary
window; physical input latency; and perceived frame smoothness on the real compositor. P4a was
visually inspected only on a nested KWin/Xwayland display. Session-qualified accessible labels
and focus recovery also need a keyboard and assistive-technology pass.

## Next

1. Record the missing Relay 2 baseline (task 58) before making migration comparisons.
2. Begin the first Stage 1 foundation slice with the accepted native stack and socket boundary.
3. Port the PTY producer repair to Relay 2 or preserve it as an explicit engine-copy patch before
   the next engine import.
