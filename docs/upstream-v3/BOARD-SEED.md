# Board seed

Seeded on the live board 2026-09-04 as tasks 55–66 (`Relay 3 spike` module). Task 67 starts the
post-spike migration. The board is the live status; the snapshot below is the intended state on
2026-09-04 after the proceed gate was accepted.
A builder session without an assigned task cannot move tasks (`actor.scope`). Completed tasks
must go through `task.approve`; other columns use `task.move` as the user from the worktree.

```fish
for task_id in 55 56 57 60 61
  $RELAY_BIN --actor user cmd task.approve "{\"task_id\":$task_id}"
end

for move in 59:in_review 62:in_review 64:in_review 63:in_review 65:in_review
  set -l parts (string split : $move)
  $RELAY_BIN --actor user cmd task.move "{\"task_id\":$parts[1],\"column\":\"$parts[2]\"}"
end
```

| # | task | column | note |
|---|---|---|---|
| 55 | Install native packages | done | vte 0.84.1, gtksourceview 5.20.0 |
| 56 | Accept D203 and import the engine | done | `403fa4a` |
| 57 | Prove the headless engine | done | READINESS, merged in PR #2 |
| 58 | Record the Relay 2 baseline | in_review | measured and documented; camera/root/input-automation gaps remain explicit |
| 59 | Relay 2: fix `suggested_workspace_from` | in_review | fixed in this copy; `docs/patches/0001` not yet applied in Relay 2 |
| 60 | P1 native shell proof | done | accepted by Antho on the display |
| 61 | P2 native bus client | done | merged in PR #2 |
| 62 | P3 VTE terminal proof | in_review | GO, PR #3 |
| 63 | P4a eleven-pane wall | in_review | repaired exact load: 11×50 MiB, zero gaps or re-attaches |
| 64 | P4b GtkSourceView proof | in_review | PR #3 |
| 65 | Spike results and recommendation | in_review | recommendation: proceed; repaired P4a clears the technical stop gate |
| 66 | Record post-spike decisions | in_review | proceed accepted; D214–D215 record process and accessibility boundaries |
| 67 | Stage 1: ship the first viewable native shell | in_review | `68108dd`; native shell, live hierarchy, error/reconnect and engine-backed geometry |
| 68 | Stage 2A: make the native Agents wall useful | in_review | `683be35`; project-scoped live sessions auto-attach through the accepted VTE path |
| 69 | Stage 2B: launch agents from the native wall | in_review | `e1c15d4`; native provider discovery and recoverable create/spawn flow |
| 70 | Stage 2C: stop self-observation feedback under burst output | in_review | `c84ba82`; de-amplified saturated pane logging while preserving gap recovery |
| 71 | Stage 2D: faithfully port the Relay 2 shell to native | in_review | `0c6d8a0` + `e2c42a8`; rendered Relay 2 shell restored, Code-only multiview decision recorded, finish review passed |

The ordered task queue for Phase 0 and the spike, on this project's Relay board (`project_id` 3).
`scripts/seed-board.sh` creates the module and these tasks in this order as the user actor
(`task.create` is not a builder op). Dry-run by default; `--run` creates. Ready tasks are Phase 0;
the spike phases start in `backlog` and move to `ready` as their entry criteria (ROADMAP) are met.

Module: **Relay 3 spike**, priority high.

| # | title | size | priority | column | owner | done when |
|---|---|---|---|---|---|---|
| 1 | Install native packages: vte4, gtksourceview5 | S | high | ready | Antho | `scripts/preflight.sh` exits 0; versions recorded in READINESS |
| 2 | Accept D203 and import the engine at 1745dd3 | M | high | ready | Antho, then a builder verifies | `scripts/import-engine.sh --run` done, `ENGINE-ORIGIN` present, `cargo test` and schema drift green, D203 marked accepted |
| 3 | Prove the headless engine from this repository's build | S | high | ready | builder | VERIFICATION §2 steps recorded in READINESS with real output |
| 4 | Record the Relay 2 baseline | M | high | ready | builder | `docs/BASELINE-RELAY2.md` with VERIFICATION §3 numbers, three runs each, commands included |
| 5 | Relay 2: fix `suggested_workspace_from` trusting a bare `.git` directory | S | medium | ready | builder, in Relay 2 | unit test passes with and without `/tmp/.git`; ported to this repo's engine copy |
| 6 | P1 native shell proof | M | high | backlog | builder + reviewer | ROADMAP P1 exit |
| 7 | P2 native bus client with demux tests | L | high | backlog | builder + reviewer | ROADMAP P2 exit; the `ui.*` executor gap diagnosed (CONTRACTS §11) |
| 8 | P3 VTE terminal proof | L | high | backlog | builder + reviewer | ROADMAP P3 exit; VERIFICATION §6 matrix ticked |
| 9 | P4a eleven-pane terminal wall, measured | M | high | backlog | builder | VERIFICATION §4 numbers in the results doc |
| 10 | P4b GtkSourceView proof | M | medium | backlog | builder | ROADMAP P4b exit |
| 11 | Spike results and recommendation | S | high | backlog | builder | `docs/RELAY-3-NATIVE-SPIKE-RESULTS.md` with every brief §14 deliverable and one recommendation |
| 12 | Record post-spike decisions | S | medium | backlog | builder | brief §15 D-entries appended; D201 resolved |

Each task body in the script carries the brief section, the entry criteria, the exit criteria and
the handoff reminder, so `session.bootstrap` hands an agent everything it needs.

Not seeded here: the Relay 2 "reusable truth" items (BACKLOG A.1–A.11) belong on the Relay-2
project's board, and Stages 1–9 are seeded only after the proceed gate.
