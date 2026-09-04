# Roadmap

Live status: the phase table in `docs/RELAY-3-NATIVE-SPIKE-RESULTS.md` and the Relay board
(`docs/BOARD-SEED.md` has the task numbers).

The brief's phases (§9–§13) and stages (§16), written as gates a task can be checked against.
Every row names what must be true before the work starts, what must be true for it to be done, and
what it hands over. Nothing here changes the brief's scope; it makes it schedulable.

## The shape of the whole thing

```
Phase 0  preflight + baseline           ── now
Spike    P1 shell · P2 bus client · P3 VTE · P4 wall + editor
Gate     proceed / change one dependency / stop      (brief §14)
Stage 1  foundation                     ── Astra, vertical slices
Stage 2  Agents wall                    ── first daily-driver milestone
Stage 3  unified project workspace
Stage 4  tasks, board, coordination
Stage 5  Notes satellite window
Stage 6  skills, settings, shell polish
Stage 7  devices
Stage 8  advanced agent surfaces        ── separate features, not launch blockers
Stage 9  replacement and release        ── Relay 2 retired only after explicit acceptance
```

## Phase 0: preflight and baseline (brief §9)

| | |
|---|---|
| entry | this repository exists; the brief is read |
| exit | `docs/READINESS.md` go/no-go checklist fully ticked |
| deliverables | READINESS with exact versions; engine crates imported (D203); headless-engine proof recorded; `docs/BASELINE-RELAY2.md` with reproducible commands; board seeded |
| blocked on | vte4 + gtksourceview5 install (Antho); D203 go (Antho) |

## Spike (brief §10–§13), one task per phase

| phase | entry | exit (success condition) | deliverable |
|---|---|---|---|
| P1 shell | Phase 0 exit | stable across repeated open, close, resize, maximise and secondary-window cycles; no leftover process or socket client after quit | `apps/relay-native` opens a placeholder window with header, sidebar, centre, status area |
| P2 bus client | P1 exit | responses, events and frames demultiplexed concurrently; pending requests cancelled on disconnect; no GTK main-loop blocking; automated tests for the demux and sequence handling | `bus_client.rs` + tests against a socket fixture or the in-process test engine |
| P3 VTE terminal | P2 exit | no lost or duplicated output, no stuck input, one scroll owner, correct resize, stable reconnection; the interaction matrix (VERIFICATION §6) run on a real display | `terminal.rs` rendering a core-owned session |
| P4a wall | P3 exit | eleven panes streaming, measured: frame smoothness, CPU streaming and idle, memory, input latency, hide/reveal/remove cost | numbers in `docs/RELAY-3-NATIVE-SPIKE-RESULTS.md` |
| P4b editor | P2 exit (parallel to P3 is allowed) | load, highlight, gutters, search, edit/read-only, dirty state, save via `file.write`, large file, UTF-8 | `editor.rs` |
| results | P4a + P4b exit | all §14 deliverables present; one recommendation: proceed, change one dependency, or stop | `docs/RELAY-3-NATIVE-SPIKE-RESULTS.md`, D-entries per §15 |

The spike must stay deletable: no design system, no shared UI abstractions, no second persistence
layer, no relay-core logic duplicated in the client (brief §7). Any of the §14 stop conditions ends
the spike with a report, not a bigger rewrite.

## Stages after a successful spike (brief §16)

Each stage is a set of vertical slices: a slice runs end to end (bus op → client → widget → test)
before the next slice starts. Nobody clones every old component first.

| stage | entry | exit | notes |
|---|---|---|---|
| 1 foundation | proceed gate passed; Claude's UI spec exists for the shell | productionised bus client; defined startup, reconnect, shutdown, crash recovery; window geometry and shell state restored; native error presentation; theme tokens, typography, icons and accessibility foundation from Claude's design | decide `relay serve` vs `relayd` here (brief §4) |
| 2 Agents wall | Stage 1 exit | sessions created with provider and model; native panes; layout, focus, resize, drag, undock, presets; resume, park, wake, clear context, discard; status and activity; creation and removal latency measured; one notification per completion | **first useful daily-driver milestone** |
| 3 Code workspace | Stage 2 exit | one Code page containing file tree, native editor, and output; Git behind one top-right button beside Skills; create/rename/move/drag/trash/restore; workspace and branch switching; ahead/behind truth; merged-PR branch cleanup; watcher-driven refresh; agent file dock; build logs with one scroll owner | Code stays a distinct tab; multiview is scoped to this page |
| 4 tasks and board | Stage 2 exit | condensed board; task detail surface; edit; message task or agent; subtasks and links; movement and state history; review-group orchestration; approval contracts preserved; stale "recorded for review" copy removed | |
| 5 Notes | Stage 1 exit | satellite window opening instantly, custom header, no useless borders, resizable and collapsible sidebar, correct close and reopen, full-height content; durable storage and bus ops preserved | not a parity copy of the in-app page |
| 6 skills, settings, polish | Stages 2–4 exit | skill library redesign; project switching and skill classification; draggable projects and workspaces; settings IA and search; wallpaper gallery, rotation, dim and projection; app icon; file and folder icon system; window presets; the settings tabs below | |
| 7 devices | Stage 3 exit | device controls; branch and build selection that never overflows; mirror and run panes; logcat and crash reporting; requested-only resources; scrcpy discovery and lifecycle rules preserved | |
| 8 advanced agent surfaces | Stage 2 exit, individually | assistant control surface; small notifying model; Agent mode; official-app continuity; optional pre-commit summary and naming; safe CLI updates; mobile companion | separate product features; none blocks release |
| 9 replacement | Stages 1–7 exit | parity ledger complete; state and preference migration; fresh install, upgrade, crash recovery, multi-window and accessibility passes; packaging; performance compared to the Relay 2 baseline; a daily-driver period with Relay 2 kept; Tauri removed only after explicit acceptance | |

### Where the "Relay V3 features and overhaul" note lands

Antho's note (`~/Relay V3 features and overhaul`, 2026-08-06) maps onto the stages above rather
than becoming its own list:

| note item | stage |
|---|---|
| Settings → Agents: default agent, auto-generated tab titles, prompt-cache timer, permissions manual or yolo | 6 (settings IA); the bus contract for provider/model selection is Stage 2 |
| Settings → General: tab order, confirm before closing a pinned tab, workspace directory, nested workspaces, ask before deleting a workspace | 6; "ask before deleting" is a removal-safety contract (brief §18), so its bus behaviour is Stage 1 |
| Settings → Terminal: scroll speed, right-click paste, copy on select, scrollback rows | 2 (VTE exposes all four natively) |
| Notification sounds and notifications | 2 (deduplicated completion is a Stage 2 exit criterion); sound choice is 6 |
| Stats and usage: agents spawned, time worked, PRs created, total tokens, estimated cost, active days | 6; needs a bus query that aggregates audit and usage, defined before the widget |
| Bottom line with resource manager and plan usage | 1 (status area is part of the shell foundation); resource sampling stays requested-only |

## Cadence rules for a steady build

1. **One task, one vertical slice, one branch.** A task is sized to fit the builder guardrail
   (60 files, 10 000 lines) with room to spare; if it does not, split it before dispatch.
2. **Entry criteria are checked at bootstrap, not discovered mid-task.** A task whose entry row is
   false is reported `blocked` immediately with the missing fact named.
3. **Every task ends with the handoff in `docs/VERIFICATION.md` §5**, outcome first, exact commit,
   measurements where relevant, and the next smallest slice.
4. **Commits at proven checkpoints**, small, `git diff --check` clean, `cargo fmt -p relay-native --check`
   clean (D209). Schema regenerated in the same commit as any op change.
5. **Measure on a real display before claiming.** A performance or visual claim without an
   observed run is reported as "not observed", never as done.
6. **Decisions are appended, never rewritten.** A surprising choice without a D-entry is a defect.
7. **Relay 2 stays green.** Fixes that produce reusable truth (brief §17) go to Relay 2 first, are
   verified there, then are carried into this repository's engine copy.
8. **Antho's decisions stay Antho's** (brief §17, last list). An agent that reaches one of them
   reports `blocked` with the question, and does not settle it by building.
9. **Pairing:** builder sessions are paired with a reviewer on the same worktree when the slice
   touches the bus client, PTY handling or teardown; the reviewer is read-only and cross-provider
   where possible.
