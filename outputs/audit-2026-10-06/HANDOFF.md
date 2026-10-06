# Audit handoff: finishing round 2

This directory holds round 1 of the first full audit of RelayV4. It covers bugs, security, bad code, code that could be better, and overengineering, across everything except the phone app. Round 1 is complete and verified. Round 2 (deduplication, independent re-checks, a completeness pass and the final write-up) is left to do, ideally at high effort; it does not need max.

## What was done

- **Audited commit:** `361c6f9`. PR #30 (device/APK support, by session brisk-vole) merged into `main` after that, as `408daf7` with merge `f2ed1cf`. It touched `crates/relay-core/src/handlers/device.rs`, `crates/relay-bus/src/ops/device.rs`, `schema/bus.v1.json`, `docs/engine/BUS.md`, `apps/relay-native/src/mirror.rs`, `mirror/glyphs.rs`, `tools_devices.rs`, `css/mirror.css`, `css/tools.css`, `smoke.rs` and `crates/relay-core/tests/phase12.rs`. The user chose not to audit PR #30 separately. They will rebase before fixing anything; recheck findings in those files then.
- **Round 1:** a Workflow of 475 agents ([round1-workflow.js](round1-workflow.js)), which took about 19 hours at max effort.
  - 41 chunk auditors each read one area in full.
  - 18 lens agents each followed one concern across the whole codebase.
  - Every finding went to an adversarial verifier, in batches of up to 5 grouped by file.
  - Every finding the verifier kept at high or critical went to one more skeptic agent.
- **Results:** 1,126 candidates, 23 refuted, 1,103 kept (1,072 confirmed, 31 plausible).
  - Confirmed or plausible, by final severity: 1 critical, 137 high, 374 medium, 591 low.
  - These counts include many duplicates, because lens agents and chunk auditors often found the same issue. For example, roughly 17 findings describe `project.clone` running `git clone` under the store lock.
- **Tooling run** (results are in the header of [report-preliminary.md](report-preliminary.md)):
  - `cargo test`: one failure, Blender 5 `Action.fcurves`.
  - Clippy: engine crates clean. The native client fails one new Rust 1.98 lint in a test.
  - `clippy::pedantic`: run to back the auditors' cast checks.
  - `cargo audit`: no vulnerabilities. `faster-hex` is flagged unsound and `bisync` is yanked, both via `gix`.

### A known weakness to fix in round 2

The round-1 skeptic for each high/critical finding was shown the first verifier's verdict. It confirmed all 164 it saw, downgrading 26 of them, which suggests anchoring. Verifiers did change severities a lot: 10 of 11 claimed criticals became high, and 56 of 204 highs became medium or low. So the high/critical set is calibrated but not independently confirmed. It needs a blind re-check (step 2 below) before the report calls it final.

## Files here

| File | What it is |
| --- | --- |
| `round1.json` | The workflow's full return value: `{summary, results:[{unit, kind, coverage, findings:[...]}]}`. Each finding has `id` (`<unit>#n`), `title`, `category`, `severity` and `confidence` (as the auditor gave them), `file`, `line`, `symbol`, `related` (other `file:line`s), `evidence`, `impact`, `fix`, `effort`, `v1` (batch verifier verdict), `v2` (skeptic verdict, high/critical only), `status` (`confirmed`, `plausible` or `refuted`), `final_severity` and `final_category`. |
| `index.tsv` | One line per kept finding (1,103), sorted by file and line: id, severity, category, location, title. Grep it to check whether something is already covered. |
| `coverage.md` | Every auditor's coverage note: what it read, how, what it ruled out, and out-of-chunk observations it did **not** report. |
| `report-preliminary.md` | A report generated from round 1. Duplicates are merged only where the file and line are identical: 211 merged, leaving 892. |
| `build_report.py` | Generates the report. `python3 -I build_report.py cfg.json`, where `cfg.json` sets `rounds`, `dedup` (`{groups:[{keep, dupes, note}]}`), `header`, `pr30_files`, `out_md` and `out_json` (see the docstring). It merges each dedup group into its `keep` finding and splits the output into Part A (defects and security) and Part B (quality and design). |
| `round1-workflow.js` | The round-1 script. Its `RULES` (evidence standard, severity rubric, categories), `FINDING`/`VERDICT` schemas and verify prompts can be reused unchanged in round 2. |

## Round 2, in order

1. **Semantic dedup.** Two findings are duplicates when they share a root cause, so one fix resolves both, even if they cite different call sites (e.g. `task.dispatch` nested prepares, cited at both `engine.rs:328` and `task.rs:683`). A shared theme is not enough. Same `file:line` is not proof either: verify it.
   - Shard by area. Round 1 used these sizes, by primary file: engine core 239; handlers, session/task/notes/module/overlap/guardrail/audit/bus 106; the other handlers 202; bus, CLI, remote and misc 239; native `app`/`client`/`shell`/`board`/`code_git`/`editor`/`project_files`/`image_preview` 109; other native 170; tests and perf 38.
   - Give each agent its shard and let it grep `index.tsv` for cross-shard duplicates.
   - Keep the finding a skeptic verified (`v2` present), then the most accurate one. Carry the other locations into `related`.
   - Union-find over `related` links chains unrelated issues into one huge cluster (one hit 174). Don't use it.
2. **Blind re-check of every unique high and critical finding** (about 60 to 70 after dedup). Use one fresh verifier per finding, **not shown any earlier verdict**, told to refute by default and to recalibrate severity with the same rubric. Use its verdict and severity as final. If it refutes a finding that two earlier verifiers confirmed, list the finding as disputed with both arguments; don't silently drop it. Reconcile severities across merged duplicates. For example, "the socket accepts a client-claimed `actor: user`" was rated medium by one lens and high by others.
3. **Completeness pass over `coverage.md`.** Pull out every concrete issue an auditor saw outside its chunk and did not report, grep `index.tsv` for it, and turn each uncovered one into a finding that goes through the normal verification. Candidates already seen:
   - `socket.rs` reads request lines with unbounded `BufReader::lines()`.
   - `MirrorRuntime::send_control` does a blocking `write_all` (`device.rs:279-283`).
   - `workspace.discover` walks a tree to depth 4 inside a locked handler (`workspace.rs:115-133`, `393-413`).
   - `attach_bytes` writes files before the transaction commits (`task.rs:505-530`).
4. **Final report.**
   - Write the executive summary: top issues to fix first, cross-cutting themes, what was ruled out, and the method.
   - Regenerate with `build_report.py` using the dedup groups and the blind verdicts.
   - Put it at `outputs/audit-2026-10-06/report.md`.
   - The user's earlier report convention is Markdown under `outputs/`. An HTML page with severity, area and category filters would help: about 600 to 700 unique findings are expected.
5. **After the rebase onto PR #30:** recheck the findings in the files listed above.

## Themes seen in the high/critical set (for the executive summary)

These come from reading the round-1 high/critical titles, before dedup and the blind re-check:

- **Guardrails can be walked around.**
  - `denied_commands` only match one contiguous literal spelling: `rm -fr`, `git push -f`, `git push origin main --force`, `/bin/rm`, `sh -c`, `$( )` all pass.
  - Shell writes through the Bash tool skip every write-path check.
  - `git commit --no-verify` skips the commit gate.
  - Codex writes are gated as `exec`.
  - The Claude hook adapter has no deadline: a slow engine makes Claude Code time it out and run the tool unchecked.
  - Grants ignore quoted words and over-match a trailing `*`.
  - The `**` glob loses its backtrack point.
  - An agent can claim `actor: user` on the socket with no credential, so it can answer its own guardrail request.
  - An agent can edit `.claude/settings.local.json` to drop its hook.
- **Confirm/replay.** `guardrail.confirm` replays an agent's held `file.*` (and `git.commit`) op without the agent's session, so the write lands in the user's primary checkout; this is the one critical. Separately, confirming a hold raised by a hook grants nothing: the retry is held again.
- **Store-lock invariant broken in several places:**
  - `project.clone` runs `git clone` under the lock with no timeout.
  - `worktree::git`/`git_mutate` (behind every git mutation) has no timeout.
  - `git.pr.open` runs `gh pr create` under the lock.
  - `avd.create` and `avd.boot` do the same with avdmanager, emulator and adb.
  - `task.dispatch` runs nested staged prepares (`git fetch`, `worktree add`) inside its transaction, undoing D149.
  - `file.write` runs an unbounded Myers diff under the lock.
- **Data loss:**
  - `worktree.remove` deletes any directory it is given (`remove_dir_all` fallback, relative paths).
  - Crash recovery kills every process carrying `RELAY_*` env, including an Unreal Editor an agent launched.
  - The Unreal `quit_editor` SIGTERMs after a 3 s probe without saving.
  - `editors_for` matches editors of other checkouts.
- **Native client:**
  - Any reply over 2 MiB tears down the whole control connection: large or binary diffs, the Skills page, mailbox history, big notes.
  - One serialized control socket gives head-of-line blocking.
  - The Git panel is rebuilt on every file event, with a paginated `gh api` call each time.
  - Widgets leak through strong closure cycles.
  - The guardrail prompt queue panics on a RefCell double borrow.
  - Replace All with zero matches panics in notes.
  - Refreshes reset state: board menus close, half-typed denial reasons are lost.
- **Phone door:**
  - No channel security: the token crosses the wire in clear, frames after the handshake are unauthenticated, and the transport is plain `ws://`.
  - No handshake timeout and no connection cap, and the door runs inside the engine process: file-descriptor exhaustion.
  - `run.sh` opens it on `0.0.0.0` at every launch.
  - Pairing is first-come and silent.
  - Revoking a device doesn't disconnect it.
  - `remote.json` read-modify-write races.
  - `wss://` rendezvous panics because rustls has no CryptoProvider.
- **Session and task logic:**
  - Sessions sharing a worktree are treated as one review group, which deadlocks independent agents on the primary checkout.
  - `assign_session` pairs unrelated agents.
  - Exited sessions cannot be relaunched.
  - Closing a session deletes the hook directory `core.hooksPath` still points at, silently disabling the commit gate.
  - `task.approve` fails after branch cleanup.
  - `project.remove` fails with a foreign-key error once a project has labels.
- **Unbounded growth:**
  - Integration worktrees and branches are never removed.
  - Trash and soft-deleted rows are never purged (`app.reconcile` is a stub).
  - Mailbox history grows forever.
  - Each wallpaper rotation copies an image into the audit table.
- **Tooling and platform:**
  - The Blender 5 API break.
  - `device.run.stop` kills only the `fish -lc` wrapper, so Gradle keeps installing.
  - `run.sh` leaves the engine in the terminal's process group: Ctrl+C kills every agent.
  - The `/tmp` runtime-dir fallback is predictable.
  - `store.db` is world-readable.
  - An agent-reported `provider_ref` reaches the provider's argv on resume.

## Practical notes

- Agents must not run cargo while another build owns the target directory, and must not call `mcp__relay__*` tools: they act on the live engine. Round 1 ran its agents read-only; no repository file was changed.
- The machine has GTK 4.22.5, VTE 0.84.1, GtkSourceView 5.20 and pango 1.58.2, so `relay-native` builds and lints here, even though several auditors assumed it could not.
- `git fetch` / fast-forward from inside an agent worktree is blocked by the auto-mode classifier, because it changes refs shared with other worktrees.
