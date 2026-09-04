# Readiness — observed 2026-09-03

Facts observed on the build machine. Nothing here is inferred from code. Re-run
`bash scripts/preflight.sh` and update the affected row whenever a fact changes.

## Machine

| item | value |
|---|---|
| OS | CachyOS (Arch), kernel 7.1.8-1-cachyos |
| session | KDE on Wayland (`XDG_SESSION_TYPE=wayland`, `WAYLAND_DISPLAY=wayland-0`; `DISPLAY=:0` also set) |
| CPU threads | 16 |
| RAM | 30 GiB |
| shell | fish |

## Toolchain

| tool | version | note |
|---|---|---|
| rustc / cargo | 1.97.1 (Arch `rust 1:1.97.1-1.1`) | system toolchain |
| pkgconf | 3.0.5 | provides `pkg-config` |
| node / npm | 26.7.0 | Relay 2 UI only; Relay 3 does not need it |
| gh, git | present | `gh` authenticated as antho976 |
| mold, sccache | not installed | optional; Relay 2's `.cargo/config.toml` carries the commented block |

## Native packages (brief §9.1)

| pkg-config name | pacman package | needed for | status |
|---|---|---|---|
| `gtk4` | `gtk4 1:4.22.4-1.1` | shell | installed, **4.22.4** |
| `vte-2.91-gtk4` | `vte4` | terminal | installed 2026-09-04, **0.84.1** |
| `gtksourceview-5` | `gtksourceview5` | editor | installed 2026-09-04, **5.20.0** |
| `pango` | `pango 1.58.2` | text | installed (dependency of gtk4) |
| `cairo` | 1.18.4 | | installed |
| `libadwaita-1` | `libadwaita 1:1.9.3-1.1` | not a spike dependency (D201) | installed, unused |
| `webkit2gtk-4.1` | 2.52.5 | Relay 2 only | installed |

Install line, for Antho to run after approving it (brief §4, D206):

```sh
sudo pacman -S --needed vte4 gtksourceview5
```

Then `bash scripts/preflight.sh` and record the exact versions in the two MISSING rows.

## Relay 2 reference (`~/dev/Relay-2`)

| item | value |
|---|---|
| commit | `1745dd3` "Merge pull request #26 from antho976/relay/proud-lynx" |
| `main` vs `origin/main` | 0 ahead, 0 behind (fetched 2026-09-03) |
| working tree | clean, except untracked `docs/RELAY-3-NATIVE-BUILD-BRIEF.md` (byte-identical to this repo's copy) |
| version / store schema | 4.0.0-dev / v12 |
| `target/debug/relay` | built; this Relay session uses it (`$RELAY_BIN`, instance `dev`) |

Headless test suite, `cargo test --workspace --exclude relay-app --no-fail-fast`:

| suite | result |
|---|---|
| relay-bus registry | 4 passed |
| relay-cli unit | 3 passed |
| relay-core unit (`src/lib.rs`) | **33 passed, 1 failed** |
| relay-core integration: agent_features 9, agent_surface 11, awareness 5, board 7, bus 15, guardrails 5, mirror 33, phase7 5, phase8 7, phase9 2, phase11 2, phase12 2, sessions 13, store 3 | all passed |

The one failure is `handlers::workspace::tests::suggested_workspace_keeps_a_non_repository_directory`
(`crates/relay-core/src/handlers/workspace.rs:381`): `suggested_workspace_from(<tempdir>/projects)`
returned `/`. Cause found on this machine: an empty directory `/tmp/.git` exists (created
2026-09-03 15:42, not a repository). The function walks up from the temp directory, sees
`/tmp/.git`, treats `/tmp` as a repository and suggests its parent. So the test is environment-
sensitive and the function trusts a `.git` name without checking it is a repository. Both are
Relay 2 items (BACKLOG A.12); neither touches the socket door or the PTY tests. Nothing was changed
in Relay 2 and `/tmp/.git` was left alone.

Fixed 2026-09-03 in this repository's copy (`crates/relay-core/src/handlers/workspace.rs`):
`suggested_workspace_from` now counts an ancestor as a repository only when `.git` is a directory
holding `HEAD` or a `gitdir:` pointer file; the test also creates a bare `.git` in its own temp
root so the case is exercised with or without `/tmp/.git`, and refuses with a clear message if
the temp dir itself sits inside a checkout. Same-source patch for Relay 2:
`docs/patches/0001-workspace-bare-git-dir.patch` (`git apply --check` passes at `1745dd3`).
With `/tmp/.git` still present: `cargo test -p relay-core --lib` → `35 passed; 0 failed`.

This repository, `cargo test --workspace --exclude relay-native` (2026-09-03, after the fix): relay-bus
registry 4, relay-cli 3, relay-core unit 35, agent_features 9, agent_surface 11, awareness 5,
board 7, bus 15, guardrails 5, mirror 33, phase7 5, phase8 7, phase9 2, phase11 2, phase12 2,
sessions 13, store 3 — all passed, exit 0. One earlier run of the same command failed
`sessions::skills_are_app_wide_folders_in_every_checkout` with "materialized skills dirty the
worktree: ?? .claude/settings.local.json ?? .relay/"; it passed 5/5 on immediate reruns (3× alone,
2× the whole binary). Likely cause, unverified: `worktree::exclude_paths` is a read-modify-write of
`.git/info/exclude` with no lock, called both from the `skill-materialize` thread spawned after
`project.add` commits and from `worktree::create` on `session.spawn`; under load the later writer
drops the other's entries. A Relay 2 item (reusable truth, brief §17), not fixed here.

## Relay board for this project

| item | value |
|---|---|
| project | Relay-V3, `project_id` 3, base branch `main` |
| modules / tasks | `Relay 3 spike`, tasks 55–66; live state and intended ordering are in `docs/BOARD-SEED.md` |
| builder role | `session.bootstrap` `can_call` does not include `task.create`, `session.claim`, `session.intent` or `session.release`; seeding runs as the user actor, and claims are unavailable to builders on this engine build |
| MCP | each worktree's `.relay/relay.mcp.json` points at `~/dev/Relay-2/target/debug/relay --instance dev mcp` |

## This repository

| item | value |
|---|---|
| GitHub | `antho976/Relay-V3`, private, default branch `main` |
| `main` | `d3f498a` after PR #3 merged P3, P4b, and shell wiring |
| this branch | `relay/amber-zebra`: proud-osprey's committed handoff integrated; P4a wall and results added |
| Cargo workspace | `Cargo.toml` with members `crates/relay-bus`, `crates/relay-core`, `crates/relay-cli`, `apps/relay-native`; engine crates copied from Relay 2 `1745dd3` on 2026-09-03 (`ENGINE-ORIGIN`, D203); `cargo test --workspace --exclude relay-native` green here (see below); imported files committed as `403fa4a` |
| CI | `.github/workflows/ci.yml`: `rust` job (`cargo test --workspace --exclude relay-native`, clippy on the three engine crates); `native` job (`apt` `libgtk-4-dev`, `cargo fmt -p relay-native --check`, `cargo check -p relay-native`) marked `continue-on-error` until one runner build is observed green. No runner run observed yet |

## Blockers before the spike can start

1. **Native packages.** Done 2026-09-04: vte 0.84.1 and gtksourceview 5.20.0 installed by Antho.
2. **Engine import.** Run 2026-09-03 at `1745dd3` (D203, `ENGINE-ORIGIN`); `cargo test` green
   here with the unit failure fixed in this copy and the patch for Relay 2 in `docs/patches/`.
   Committed as `403fa4a`. Remaining: apply `docs/patches/0001-workspace-bare-git-dir.patch`
   in Relay 2.
3. **Board seed.** Run `scripts/seed-board.sh` so the spike proceeds as assigned tasks with
   `session.bootstrap` context, not ad hoc prompts.

## Go / no-go for "the spike may start"

- [x] `scripts/preflight.sh` exits 0 (three required pkg-config rows present), versions recorded above (2026-09-04)
- [x] engine crates present here, `ENGINE-ORIGIN` written, `cargo test` green, schema drift test green (`403fa4a`)
- [x] D203 marked accepted in `docs/DECISIONS.md` with the pinned commit (`403fa4a`)
- [x] headless engine proof done from this repository's build (`docs/VERIFICATION.md` §2), results recorded here (2026-09-03, below)
- [x] Relay 2 baseline recorded in `docs/BASELINE-RELAY2.md` (`docs/VERIFICATION.md` §3), with
  the camera/root/input-automation gaps explicitly left unclaimed
- [x] board seeded 2026-09-04 (tasks 55–66); P1–P3 and P4b were run by Fable subagents from this session rather than dispatched tasks, and their columns were moved afterwards

## Headless engine proof (observed 2026-09-03)

VERIFICATION §2 run from this repository's build (`cargo build -p relay-cli` → `target/debug/relay`,
`relay 4.0.0-dev`) against instance `test` only; `dev` (the engine running this session) and
`stable` untouched. Shell was zsh (`export RELAY_INSTANCE=test`). Two deviations from the script,
both deliberate: `serve --store <scratch>/test-store.db` so the pre-existing
`~/.local/share/relay/test/store.db` (2026-08-18) was neither opened nor migrated; and every client
call ran with `RELAY_SESSION`, `RELAY_TOKEN`, `RELAY_PROJECT`, `RELAY_WORKTREE`, `RELAY_BRIEF`
unset, see step 2. Output trimmed, nothing invented.

| step | command | observed |
|---|---|---|
| 1 | `relay serve --store …/test-store.db &` (pid 701606) | log: `socket door open socket=/run/user/1000/relay/test.sock instance=test`, `engine up instance=test store=…/test-store.db`. `ls -l /run/user/1000/relay/`: `srw------- test.sock`, `-rw------- test.lock` (6 bytes, the pid); `dev.sock`/`dev.lock` from 16:09 untouched |
| 2 | `relay q app.status` | first attempt, with this agent session's environment inherited: `{"kind":"invalid","code":"bus.actor","message":"unknown session \"proud-osprey\""}` — the CLI binds `agent:<RELAY_SESSION>` from the environment (BUS §8.2) and the test engine does not know the dev instance's session; correct refusal, not a gap. With the five `RELAY_*` session vars unset, `bus.whoami` → `actor: "user"`, and `app.status` → `{"pid":701606,"uptime_s":32,"store_path":"…/test-store.db","socket_path":"/run/user/1000/relay/test.sock","ui_connected":false,"sessions_live":0,"providers":[{"provider":"claude","installed":true,"path":"/home/anthony/.local/bin/claude",…},{"provider":"codex","installed":true,…}]}` |
| 3 | `relay events --filter 'session.*' &` | subscriber stayed connected for the whole run and received 4 `session.changed` lines, interleaved with nothing else. First event: `{"v":1,"ev":"session.changed","ts":"2026-09-04T00:19:20.471529859Z","actor":"user","cause":"2d4a6e45-4bd4-496c-896a-e06e5292e2ea","project_id":1,"payload":{"id":1,"name":"vivid-tapir",…,"state":"created",…}}`; the `cause` is the id of the `session.create` request. The third carried `actor: "agent:vivid-tapir"` and `provider_ref` set, i.e. the spawned Claude Code reported in through its lifecycle hook within 0.43 s of spawn |
| — | `relay q workspace.create '{"path":"/home/anthony/dev-2"}'` | `{"id":1,"path":"/home/anthony/dev-2","name":"dev-2","order":0,…}` |
| — | `relay q project.add '{"workspace_id":1,"path":"/home/anthony/dev-2/Relay-V3"}'` | `{"id":1,"workspace_id":1,"path":"/home/anthony/dev-2/Relay-V3","name":"Relay-V3","base_branch":"main",…}` |
| — | `relay q provider.list` | `claude` installed at `/home/anthony/.local/bin/claude`, `codex` at `/home/anthony/.local/bin/codex`; `session.create` `provider` enum is `["claude","codex"]`, no shell-only provider on this build |
| — | `relay q session.create '{"project_id":1,"provider":"claude"}'` | name `vivid-tapir`, role `builder`, branch `relay/vivid-tapir`, worktree `/home/anthony/dev-2/Relay-V3/.relay/worktrees/vivid-tapir`, `state: "created"`, `pid: null` |
| 4 | `relay q session.spawn '{"session":"vivid-tapir"}'` | `state: "running"`, `pid: 704441`, `spawned_at` set; `ps` shows the child as `claude --mcp-config .relay/relay.mcp.json --settings .relay/relay.settings.json --append-system-prompt-file .relay/sessions/vivid-tapir/role-instructions.md --name vivid-tapir`, parent pid 701606 (the headless engine, no Tauri) |
| — | `timeout 5 relay attach vivid-tapir` | streamed 2699 bytes of PTY output in 5 s: the Claude Code banner (`Claude Code v2.1.259`, `Fable 5.1 with medium effort · Claude Max`, `~/dev-2/Relay-V3/.relay/worktrees/vivid-tapir`), with alternate-screen and mouse-mode escapes intact; `timeout` ended the client with SIGTERM (not Ctrl-C), exit 124 |
| 5 | `relay q session.get '{"session":"vivid-tapir"}'` after the client left | `state: "running"`, `pid: 704441` unchanged; `ps -p 704441` alive 23 s later |
| — | `relay q session.close '{"session":"vivid-tapir"}'` | `{"freed_mb":0.035}`; event `session.changed … state=closed` arrived at the subscriber; `ps -p 704441` gone; the worktree directory removed and no longer in `git worktree list`; branch `relay/vivid-tapir` **left behind** in `/home/anthony/dev-2/Relay-V3` (Relay 2 behaviour, `git.branch.clean_merged` exists for it; not deleted by this proof). Afterwards `session.get` → stderr `{"kind":"not_found","code":"session.not_found","message":"no live session named \"vivid-tapir\""}`, exit 1; `session.list {"project_id":1}` → `[]` |
| — | `kill 701606` | log `SIGTERM`; process gone within 2 s; `test.sock` unlinked; **`test.lock` remains** (6 bytes, dead pid 701606) with the `flock` released (`flock -n test.lock true` succeeds). A second `relay serve` on the same instance took the lock, rewrote its pid into the file, bound `test.sock` again (BUS §6.2 stale takeover) and was stopped the same way |

Verdict: every §2 step behaves as the contract says. Not a gap but two wording facts for the
docs: VERIFICATION §2 says "socket and lock are gone" — the socket is; the lock *file* stays and
only the `flock` is released, which BUS §6.2 already describes as the stale-takeover path. And a
client inheriting another instance's `RELAY_SESSION`/`RELAY_TOKEN` is refused with `bus.actor`,
so scripts driving `test` from inside a Relay session must clear those variables. Engine log
warnings seen: `store lock over budget op="session.create" held_ms=22.9` and
`op="session.close" held_ms=407.5` (worktree add/remove inside the store lock; Relay 2 item).
