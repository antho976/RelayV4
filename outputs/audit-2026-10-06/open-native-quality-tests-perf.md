# Audit findings left open: native code quality, engine tests, perf harness

Scope: the Low findings RA-650..722 and RA-741/742, fixed on branch `relay/spry-zebra` and then
on `relay/brisk-heron` (both 2026-10-07). "Still open" is the current list; the history section
keeps the list as spry-zebra left it, with what closing each item would take. Findings named
nowhere here are fixed, or were already fixed when checked (RA-669's CI timeout, RA-680,
RA-688's editor side, RA-699's and RA-703's app.rs parts).

## Closed on `relay/brisk-heron` (2026-10-07)

Everything the list below used to hold as open was taken up again on `relay/brisk-heron`.
What is still open, in full or in part, is under "Still open"; the rest is fixed:

- perf.rs halves of RA-650..657, plus RA-652 and RA-653: the response leaves the timed
  region before it is sized, `ok` comes from the call, "during" rows report their overlap
  (`device.build`/`task.list` variants dropped), startup/recovery scans are drained between
  iterations, strace attaches under Yama scope 1 and counts only between markers, and filters
  match exactly unless they end in `*` (`baseline.py` follows). strace was checked live (7.2,
  Yama scope 1): every case recorded exactly one marked region per iteration. RA-651 needed no change:
  `isolate_host` already points git at the fixture's own config.
- RA-675..678, RA-742 (sessions.rs, store.rs), RA-671 (every listed file uses `tests/common`),
  RA-665 (the signing test opens a file store in a TempDir).
- RA-672: every relay-core test binary sets `GIT_CONFIG_GLOBAL=/dev/null` and
  `GIT_CONFIG_NOSYSTEM=1` before `main` (a `ctor` in `lib.rs` tests and `tests/common`); the
  whole suite passes with a global `core.hooksPath` pointing at a failing hook.
- RA-673 (`watch::DEBOUNCE`), RA-674 (`watch_tick`, tested), RA-689 (`time::span`;
  `relative::span` in the client), RA-709 (`Skill.description` from one parser; the native and
  mobile parsers are gone).
- RA-663: the `usage.report` doc drift is corrected and `tests/usage_source.rs` tests which
  source `usage.get` prefers.
- RA-686 and RA-692: the pure git-view and mirror logic, and the client's mirror payload
  builders, live in the headless `relay-client` crate, which CI tests; `tests/native_input.rs`
  builds its payloads with them, so it can no longer drift from the client.
- RA-681, RA-690 (`Choice`), RA-707, RA-710 (editor scheme from the palette table; the
  extensions summary uses `@secondary`; the Notes close red `#c42b1c` stays, by decision),
  RA-717, RA-719, RA-720.
- The roadmap tools part: the skills widget names were still current; it failed earlier, at
  the branch picker's switch key, and now opens the picker first.

Found and fixed on the way: `guardrail::write_roots` keyed a linked worktree's primary memory
root on `.git/worktrees`; stored stamps had variable-width fractions, so their text did not
sort in time order (now nine digits always, and `list_backups` sorts by instant); the fake
`gh` unit test could fail with ETXTBSY; and since 54a6cdd the native client never restored the
saved layout at launch, and dropped layout changes until a later refresh (`refresh` passed
`restore_layout` project 0 on the first pass). With that fixed, `scripts/native-smoke.py` was run
on a live display and passed end to end: every page, burst, and the roadmap parts `notes`,
`files`, `lifecycle`, `tools` and `registry`. That also needed four harness fixes, none of them
app bugs: Notes reopens its note after the library restores tabs, lifecycle follows the new
session's own pane, the tools check skips parked panes, and registry returns to Agents first.

## Still open

- **RA-663, production half.** `usage.get`'s test-instance branch stays: the test instance
  must not read the developer's real provider files, and the real path is now tested on a dev
  instance. After-close cleanup and `after_merged_prs` stay off in tests: turning them on
  starts a thread that runs the real `gh` and races `branch_cleanup.rs`'s assertions.
  `sh` vs `fish` would need a setting nobody asked for.
- **RA-686.** The `image_preview.rs` tests decode Pixbufs and stay in relay-native.
- **New figures.** BASELINE and FIXES keep their historical numbers, with the notes about
  the old harness reworded; no new run was taken with the fixed harness.

## History: as left by `relay/spry-zebra`

### Held by another session the whole time (amber-heron)

`crates/relay-core/examples/perf.rs`, `tests/sessions.rs`, `tests/store.rs` and
`tests/agent_surface.rs` stayed claimed by amber-heron, which was still fixing Medium engine
findings in them, so they were not edited here.

- **RA-650, perf.rs half.** The `bytes` count serializes the result inside the timed closure.
  Fix: return the `Response` from the work closure and compute `bytes` after the counters are
  read. The BASELINE doc now says which rows this inflates.
- **RA-651, perf.rs half.** `git()` and `small_repo` inherit global git config. Fix: run them
  with `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`, as the Python fixtures and
  `tests/common` now do.
- **RA-652.** `path.engine.startup` and `path.recovery.run` leave a recovery dirty-scan thread
  running per iteration. Fix: run recovery with an immediate `DirtyScan` in the harness, or
  join the scan before the next iteration.
- **RA-653.** Several `path.*` cases report `ok: true` when the measured call fails. Fix:
  derive `ok` from the call's result.
- **RA-654, perf.rs half.** The "during" cases wait a fixed 5 ms and never check that the
  other op is still running. Fix: have the other op signal from inside its handler, or record
  both ops' start and end times and report the overlap. Drop or relabel the `device.build` and
  `task.list` variants. The FIXES doc now says the 470x row measured no overlap.
- **RA-655, perf.rs half.** `strace_start` should call `prctl(PR_SET_PTRACER, …)` or check
  `child.try_wait()` after its sleep. `strace_stop` should emit null when strace exited early.
  `baseline.py` and `report.py` already skip the pass and say why.
- **RA-656, perf.rs half.** The strace counts include each iteration's setup, cleanup and
  forked git children. Fix: count only between markers, or subtract a no-op case that runs the
  same setup and cleanup. The doc says so in the meantime.
- **RA-657, perf.rs half.** `run` treats every filter as a prefix (~line 1709). Fix: match
  exactly unless the filter ends in `*`, and make `baseline.py`'s `matches()` follow the same
  rule. `baseline.py`'s shards no longer run a case twice.
- **RA-661, store.rs half.** Already covered: `every_prior_version_migrates_forward` puts rows
  in every table at every version. The board half is fixed.
- **RA-675, RA-676, RA-677, RA-742.** All four are in `tests/sessions.rs`: the write-roots
  expectation computed by the code under test, frame loops that spin 10 s on EOF, three
  assertions about behaviour the test cannot observe, and 60/100 ms bounds against a 150 ms
  grace. Not started.
- **RA-678.** `tests/store.rs`: the backup-retention order check is guaranteed by the sort it
  tests. Not started.
- **RA-671, the rest.** `tests/common/mod.rs` is used by 13 test files. `sessions.rs`,
  `agent_surface.rs` and `store.rs` (held) still carry their own copies. So do `retention.rs`,
  `git_handlers.rs`, `session_lifecycle.rs`, `file_task_audit.rs`, `bounded_payloads.rs`,
  `integration_registry.rs`, `guardrail_bypasses.rs`, `device_run_worker.rs`,
  `device_signing.rs` and `phase11.rs`, which amber-heron was editing in the same hours though
  it had not claimed them. Each needs `mod common;` and the same swap of helpers.
- **RA-665, the rest.** `tests/device_signing.rs` also calls `device.signing.create` on an
  in-memory store, so it still leaves `/tmp/relay-signing-test-*` directories behind. Fix: open
  a file store in a `TempDir`, as `bus.rs` now does.

### Needs engine changes (production code other sessions hold)

- **RA-663, production half.** `handlers/session.rs` (after-close cleanup), `git.rs`
  (`after_merged_prs`), `provider.rs` (usage file reads) and `device.rs` (`sh` vs `fish`)
  still branch on `Instance::Test`. The verifier's cheaper fix: correct the doc drift that says
  hooks call `usage.report` (`src/audit.rs:16`, BUS.md), and add one test of which source
  `usage.get` prefers, with `CLAUDE_CONFIG_DIR` pointed at a temp dir.
- **RA-672, engine half.** The engine's own git calls read global config. With a global
  `core.hooksPath`, three phase8 tests fail with `git.pre_commit_failed`, because `git.commit`
  and `hooks::inherited_hooks_dir` read the global hooks path. Fix: give the test process
  `GIT_CONFIG_GLOBAL=/dev/null` (workspace-wide `[env]`), or pin `core.hooksPath` in phase8's
  fixture, which would change what the chaining test covers.
- **RA-673, watch.rs half.** The watcher's 125 ms debounce is a literal (~line 178). Make it a
  `pub const` so `tests/phase8.rs` can derive its quiet window from it.
- **RA-674, the rest.** The every-15th-tick disk refresh is untested. It needs the tick body in
  `handlers/app.rs` pulled out into a sync function.
- **RA-709, engine half.** The client now reads SKILL.md front matter the engine's way. The
  real fix: add `description` to the bus `Skill` type, compute it in relay-core with one
  front-matter parser (`github.rs::frontmatter_name` should use it too), and delete the client
  parser.
- **RA-689, engine copies.** `device_lease.rs` `ago` and `usage.rs` `reset_label` (duplicated
  by `status_usage.rs` `span()`) still format time themselves. The native client now has one
  helper, `relative.rs`.

### CI cannot build the native client

- **RA-686.** The pure diff, graph and status logic in `code_git.rs` is tested only inside
  relay-native, which CI never builds (GTK 4.14 on the runners, 4.22 needed). Plan: move
  `hunk_header`, `diff_lines`, `diff_marks`, `inline_diff`/`Mark`, `change_summary`,
  `commit_graph`/`GraphRow`, `is_conflict`/`is_staged`/`is_unstaged`, `has_conflict_markers`,
  `split_path` and their `graph_tests` into a headless module (e.g. `relay-bus::git_view`).
  The `image_preview.rs` tests decode Pixbufs and cannot move.
- **RA-692, CI half.** `tests/native_input.rs` now sends every `device.mirror.input` payload
  the client builds (hand-copied, so keep it in step with `mirror.rs` and `mirror/input.rs`)
  and checks the exact bytes. The GTK-free client pieces (`Gate`, `decoder_bytes` and `ppm` in
  `mirror/decode.rs`; `coalesce`, `map_point` and `scroll_amount` in `mirror/input.rs`) still
  build only with relay-native. Moving them needs a headless module in the default members.

### Deliberately not done (cost out of proportion for a Low)

- **RA-681, the split.** The dead state is gone and the close policy is its own method.
  Splitting `Ui::build` into per-area builders is a long reshuffle of a GTK constructor for
  readability only.
- **RA-690, the `Choice` type.** One `bind_keys` now binds the key rows to their hidden
  controls. Replacing the hidden DropDowns needs the harness rewritten first:
  `roadmap_smoke.rs` and `smoke.rs` drive them by widget name (`launch-provider-N`,
  `launch-effort-N`, `launch-mode`, `launch-builders`, `launch-count-control`).
- **RA-707, the split.** Save now reads typed field readers instead of parsing widget names.
  Splitting the ~590-line `refresh` per category, and fetching initial values in one join, is
  not done. The verifier rated the sequential local-socket reads as cheap.
- **RA-710, the rest.** The CSS and Rust colour literals now come from tokens, and one
  `fonts::PALETTES` table holds the per-mode palette. Still open:
  - `relay-editor.xml`'s literals need a palette-derived source.
  - `notes.css` keeps `@notes_paper` beside `note_pages.rs` `NOTES_PAPER`, which is
    deliberately the same in every mode.
  - The Notes close key `#c42b1c` and `tools.css` `#c4c4c2` match no token, so changing them
    is a design call.
- **RA-717, one wait.** The setup-github tab switch keeps a commented 500 ms delay. No
  assertion depends on it.
- **RA-719, exception requests.** The harness now drives Allow once and Reject on held
  actions. Approve once, Approve for this session, Deny and Revoke on `guardrail.request` cards
  need a request from a real session. Plan: the same pattern on the `guardrail-request-{id}`
  cards.
- **RA-720, the rest.** Trimming `verify_tools` in `note_pages.rs`.

### Found while verifying, outside this range

- The roadmap `tools` smoke part looks for `skills-split`, `skills-project` and
  `skill-enabled-*`. No widget has had those names since the Skills page rebuild (280ae4b), so
  that part cannot pass beyond the skills section.
- The settings smoke used to set font size 9.75, which is the default, so Save sent nothing
  and the check failed even at the base commit. It now sets 10.25. The settings check was run
  live and passes.
- On this machine the live display stopped producing frames partway through the smoke runs
  ("no rendered frame after two seconds"), with the original harness too. Most smoke pages were
  therefore compiled, not run, after the changes.
