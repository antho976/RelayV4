# RelayV4 code audit (2026-10-06)

This is the first full audit of RelayV4. It covers bugs, security, bad code, code that could be better, and overengineering, across everything except the phone app. It is final: round 1 found and verified the issues, and round 2 deduplicated them, re-checked every high-severity finding blind, filled coverage gaps and rechecked findings against the code merged since. A filterable version of the same findings is in [report.html](report.html).

- **Audited commit:** `361c6f9`. Line numbers in this report refer to the current `main` (`d04b526`). The two commits are identical outside the 19 non-phone files that PR #30 and PR #31 changed; findings in those files were remapped or rechecked (see Method).
- **Scope:** the engine (`relay-core`), the bus types (`relay-bus`), the CLI and MCP servers including the embedded Blender and Unreal Python (`relay-cli`), the phone door (`relay-remote`, `deploy/`), the GTK client (`apps/relay-native`), tests, scripts, CI, plugins and docs. The phone app (`apps/relay-mobile`) was out of scope.
- **Result:** **744 unique findings**: 0 critical, **56 high**, 236 medium and 452 low. 561 are defects or security issues (Part A) and 183 are code quality or design (Part B). Every high finding has been confirmed by at least two independent verifiers, the last of them blind.

## Executive summary

RelayV4's core design holds up. The single-store engine, the typed bus and the worktree-per-agent model are sound, and the audit found no memory-safety finding, no known-vulnerable dependency, and little overengineering (5 findings). The serious problems cluster in four places:

- **Guardrails.** They can be walked around in ordinary ways.
- **The store-lock rule.** CLAUDE.md's rule against slow work under the store lock is broken in a dozen handlers, so the whole bus can freeze.
- **The phone door.** It is exposed on the network without channel security.
- **The desktop client.** It rebuilds whole panels on every event, which resets what the user is doing, leaks widgets, and can drop its only engine connection on one large reply.

No single finding reached critical after verification. The one claimed critical, a confirmed hold landing in the wrong checkout, is real but needs uncommitted edits to the same file in the main checkout before it destroys work, so it is rated high.

### Fix first

These are ordered by harm prevented per unit of work. S, M and L are the auditors' effort estimates.

1. **Make confirmed holds replay in the agent's worktree** ([RA-009](#ra-009), S). `guardrail.confirm` replays a held agent `file.write` with no session bound, so the write lands in the user's primary checkout and the agent's worktree never gets it.
2. **Make `worktree.remove` refuse anything that is not a registered worktree of the project** ([RA-017](#ra-017), M). Today it accepts any path, even a relative one, and falls back to `remove_dir_all`.
3. **Fail closed when the guardrail hook is slow** ([RA-002](#ra-002), S). The PreToolUse adapter has no deadline of its own. When the engine is busy past the provider's 30 s hook timeout, the provider kills the hook and runs the tool unchecked.
4. **Gate Codex edits as writes** ([RA-003](#ra-003), M). Gate shell writes too: redirections, `sed -i`, `tee`, `cp` ([RA-001](#ra-001), L). At the moment no write guardrail runs for Codex, and none runs for any Bash-tool write.
5. **Validate `provider_ref`** ([RA-030](#ra-030), S). An agent can store a string that later lands in the provider's argv on resume, for example `--dangerously-bypass-approvals-and-sandbox`.
6. **Stop closing one session from deleting the hook directory** that the shared checkout's `core.hooksPath` still points at ([RA-031](#ra-031), M). Today it silently disables the commit gate for every other session.
7. **Close the phone door's network exposure** ([RA-056](#ra-056), [RA-053](#ra-053)). Stop `run.sh` from opening it on `0.0.0.0` at every launch, and add a handshake timeout and a connection cap. Channel security ([RA-052](#ra-052), L) is the larger follow-up.
8. **Move subprocesses out of the store lock and put them under deadlines.** This covers `worktree::git`/`git_mutate` ([RA-022](#ra-022)), `project.clone` ([RA-034](#ra-034)), `gh pr create` ([RA-023](#ra-023)), `avd.create`/`avd.boot` ([RA-018](#ra-018)) and `task.dispatch`'s nested prepares ([RA-032](#ra-032)). Each can freeze every session, keystrokes included.
9. **Stop one large reply from killing the client's connection** ([RA-037](#ra-037)). Any reply line over 2 MiB tears down the client's only engine connection; [RA-027](#ra-027) (mailbox history) and [RA-050](#ra-050) (Skills page) hit this in normal use.
10. **Fix the two client crashes:** the guardrail prompt queue's RefCell double borrow ([RA-042](#ra-042), S), and Replace All with zero matches in notes ([RA-044](#ra-044), S), which aborts the client and loses unsaved notes.
11. **Make `run.sh` start the engine in its own session** ([RA-055](#ra-055), S). Today Ctrl+C in the launching terminal kills every agent.
12. **Port the Blender scripts to Blender 5's layered actions** ([RA-008](#ra-008), S). This is the one failing `cargo test`.

### Themes

- **Guardrails are advisory in practice.**
  - Denied commands match one contiguous literal spelling. `rm -fr`, `git push -f`, `/bin/rm`, `sh -c` and `$( )` all pass ([RA-010](#ra-010), [RA-011](#ra-011)).
  - The `**` glob loses its backtrack point ([RA-012](#ra-012)).
  - Confirming a hook-raised hold grants nothing, so the retry is held again ([RA-025](#ra-025)).
  - At medium:
    - an agent can claim `actor: user` on the socket and answer its own guardrail ([RA-096](#ra-096));
    - `git commit --no-verify` skips the commit gate ([RA-109](#ra-109));
    - grants ignore quoted words ([RA-104](#ra-104));
    - provider settings files sit in the agent-writable worktree ([RA-273](#ra-273)).

  BUS.md calls guardrails "best effort, not a security boundary". The code is weaker than even that wording suggests, and the UI labels Codex "Guarded".
- **The store-lock invariant is convention, not structure.** Besides the subprocess handlers above:
  - `file.search` reads whole files of any size under the lock ([RA-020](#ra-020));
  - rename, move and delete re-read the file to gate it ([RA-021](#ra-021));
  - the socket serves each connection strictly in order, while the client multiplexes all UI traffic on one connection ([RA-015](#ra-015)).

  Seven separate git runners exist, and only some have deadlines. One shared runner with a mandatory timeout would remove the class.
- **Processes outlive or outrun their owners.**
  - Stopping a device run kills only the `fish -lc` wrapper, so Gradle keeps installing ([RA-019](#ra-019)).
  - Crash recovery kills any process carrying `RELAY_*` env, including an Unreal Editor the user now owns ([RA-006](#ra-006)).
  - The Unreal tools can SIGTERM the wrong checkout's editor, or quit without saving ([RA-004](#ra-004), [RA-005](#ra-005)).
- **Session and task flows dead-end.**
  - Sessions sharing a checkout are treated as one review group, which deadlocks independent agents ([RA-028](#ra-028)).
  - Exited sessions cannot be relaunched ([RA-029](#ra-029)).
  - `task.approve` fails after branch cleanup ([RA-033](#ra-033)).
  - `project.remove` fails on a foreign key once a project has labels, after it has already killed the project's agents ([RA-035](#ra-035)).
  - Codex reviewers can never bootstrap ([RA-014](#ra-014)).
  - Desktop launches ignore a plugin's default checkout ([RA-043](#ra-043)).
- **Unbounded growth.**
  - Integration worktrees and branches are never removed ([RA-026](#ra-026)).
  - Every wallpaper rotation copies an image into the audit table ([RA-049](#ra-049)).
  - Mailbox history is never pruned ([RA-027](#ra-027)).
  - Trash and soft-deleted rows are never purged, because `app.reconcile` is a stub ([RA-132](#ra-132), medium).
- **The desktop client rebuilds instead of updating.**
  - The Git panel, Guardrails page, board and sidebar are torn down on every event ([RA-036](#ra-036), [RA-038](#ra-038), [RA-045](#ra-045), [RA-046](#ra-046)). That closes menus, loses half-typed text and moves buttons under the pointer.
  - Closures hold strong references to their own widgets, so each rebuild leaks the old tree ([RA-039](#ra-039), [RA-047](#ra-047)).
  - Each refresh can page the repository's entire PR history through `gh` ([RA-024](#ra-024), [RA-040](#ra-040)).
- **The phone door was built for a trusted LAN.**
  - The token crosses the wire in clear and later frames are unauthenticated ([RA-052](#ra-052)).
  - One slow phone stalls every lane of the rendezvous ([RA-054](#ra-054)).
  - A `wss://` rendezvous panics because rustls has no CryptoProvider ([RA-051](#ra-051)).
- **Docs drift.** 36 findings, most in BUS.md, where the Codex guard, the reconcile pass and the `ui.*` executor are described as they were planned rather than as built.

### What was ruled out

- **Dependencies:** `cargo audit` found no known vulnerabilities. Two transitive warnings come through `gix`: `faster-hex` is unsound and `bisync` is yanked.
- **Lints:** the engine crates are clippy-clean.
- **Refuted in round 1:** 23 candidates. Examples:
  - stream frames racing their response (they cannot under the scheduler in use);
  - the CI trigger branch (the premise was a stale local ref);
  - nullable patch fields not round-tripping (no caller depends on it);
  - `bus.subscribe` skipping envelope checks (those ops carry no authority);
  - `blender_python` lacking a guardrail gate, as a separate issue (it is covered by the plugin-MCP finding).
- **Dropped in the completeness pass:** 13 candidates on reading the code. Examples:
  - idempotency replays (they execute nothing);
  - non-constant-time session-token comparison (same-uid boundary);
  - `/tmp` as an always-on write root (documented, D102);
  - the scrcpy forward port (adb already exposes more to local users).

## Method

**Round 1** used 475 agents at max effort, over about 19 hours (the script is [round1-workflow.js](round1-workflow.js)).
- 41 auditors each read one area in full.
- 18 more each followed one concern across the whole codebase: the CLAUDE.md invariants, SQLite use, concurrency, five security angles, data loss, GTK main-thread health, panics, dead or duplicated code, overengineering, the bus contract, performance, resource leaks and misleading docs.
- Every finding went to a verifier told to refute it, and every high or critical one to a second skeptic.
- The result was 1,126 candidates: 23 refuted and 1,103 kept.

**Round 2** used 92 agents at high effort ([round2-part1-workflow.js](round2-part1-workflow.js), [round2-part2-workflow.js](round2-part2-workflow.js)).

- **Semantic dedup.**
  - Rule: a finding is a duplicate when it shares a root cause with another, so one fix resolves both. A shared theme is not enough, and neither is a shared line.
  - Shards: 11 agents each took one shard and searched the full index for duplicates in other shards.
  - Reconciling: shard groups that shared members were merged ([reconcile.py](reconcile.py)). Two clusters chained distinct root causes and were split by hand: the Codex guard, shell writes and BUS.md drift; and the `ui.*` model and Tauri leftovers.
  - Result: 197 groups, 366 findings merged away, 737 unique. Merged findings keep their locations under "Also at" and their ids under "Merged duplicates" ([round2-dedup.json](round2-dedup.json)).
- **Blind re-check.** It covered every unique finding that any member of its group had rated high or critical: 63 findings.
  - Each went to one fresh verifier. The verifier saw the auditor's evidence and the titles of the merged reports, but no earlier verdict or severity, and was barred from reading the audit outputs.
  - It was told to refute by default and to rate severity fresh with the same rubric. Its verdict and severity are final.
  - Results: all 63 were confirmed and none refuted, so there are no disputed findings. 7 were lowered to medium and the one critical became high ([round2-blind.json](round2-blind.json)).
  - This answers the round-1 concern that the skeptic had been anchored by seeing the first verdict.
  - Medium and low findings had one adversarial verifier each and no blind re-check.
- **Completeness pass.** Two agents read every auditor's coverage notes for issues seen outside their chunk and not reported.
  - 74 such observations were already covered by a finding.
  - 21 were not. Each was written up from the code or dropped with a reason, and every write-up was verified.
  - 8 became findings (1 medium, 7 low); 13 were dropped ([round2-completeness.json](round2-completeness.json)).
- **Recheck after PR #30 and #31.** 168 findings sat in the 19 changed non-phone files, but only 13 were within 12 lines of an edit.
  - Two of those 13 had been merged into other findings.
  - One was high and went through the blind re-check, which reads the current code.
  - The other 10 were rechecked against the diff: 6 are still present, 3 changed and 1 is fixed (Appendix B).
  - Every other line in those files was remapped through the diff hunks ([linemap.py](linemap.py)).

**Severity rubric** (a single-user desktop app with a local engine, many concurrent agent sessions and an optional phone door):
- **Critical:** data loss, a hole reachable by another local user, a network peer or an escaping agent, or an engine-wide hang.
- **High:** likely wrong behaviour in a normal flow, a bus or UI freeze, unbounded growth, or a safety check that is silently bypassed.
- **Medium:** a real bug in a less common path, or a design problem with a concrete ongoing cost.
- **Low:** a minor edge case or a modest cleanup.

## Tooling results

These were run in round 1, at `361c6f9`.

- **`cargo test`:** one failure. `relay-cli` `blender::tests::the_blender_tools_work_on_a_real_rig` fails on the installed Blender 5.2.1 with `AttributeError: 'Action' object has no attribute 'fcurves'` ([RA-008](#ra-008)). `cargo test -p relay-native` passes.
- **`cargo clippy --all-targets -- -D warnings`** (engine, bus, CLI, remote): clean.
- **`cargo clippy -p relay-native --all-targets -- -D warnings -A deprecated`:** one new Rust 1.98 lint, `chunks_exact_to_as_chunks`, in a test at `apps/relay-native/src/sounds.rs:150`. CI pins Rust 1.94, so this is toolchain drift.
- **`clippy::pedantic`:** about 1,400 warnings, mostly docs, `must_use` and cast lints. Auditors checked the cast warnings that touch lengths, offsets, timestamps and ids, and reported only real bugs.
- **`cargo audit`** (1,290 advisories, 383 crates): no vulnerabilities.
  - `faster-hex 0.10.0` is unsound (RUSTSEC-2026-0306, via `gix-hash`).
  - `bisync 0.3.0` is yanked (via `gix-protocol`).

## How to read this

- **Ids.** Part A is defects and security; Part B is code quality and design. Within each part, findings are ordered by severity, then area, and numbered `RA-nnn`.
- **Contents.** Each finding gives the problem with evidence, the concrete impact, the recommended fix, and every verifier's corrections. Where a correction disagrees with the auditor's text, trust the correction; the latest verifier's note comes last.
- **Status.**
  - "Confirmed" means a verifier established the finding from the code.
  - "Plausible" means it is probably real, but the trigger or impact could not be fully established.
  - "Blind re-check" notes come from round 2.
- **Data and tools.**
  - Raw data: [round1.json](round1.json) and the `round2-*.json` files.
  - `round2-merged.json` (round 2 folded into round 1) and `report-data.json` (this report as data) are generated rather than committed. Regenerate them with `python3 -I merge_r2.py round1.json round2-dedup.json round2-completeness.json round2-blind.json round2-linemap.json round2-merged.json`, then `python3 -I build_report.py report-config.json`.
  - The scripts are `build_report.py`, `build_html.py` and `merge_r2.py`.
