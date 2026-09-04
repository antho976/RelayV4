# Verification

Commands, the measurement protocol, the manual terminal matrix, and the handoff format. Every
number that reaches a doc comes from one of these procedures, run on this machine, with the
command that produced it written next to it. Compilation and unit tests are not visual validation
(brief §20).

## 1. Commands

Engine (after D203):

```sh
cargo check
cargo test                                                   # bus + core, headless
cargo run -p relay-bus --example dump_schema > schema/bus.v1.json   # after any op change
cargo test -p relay-bus                                      # includes the schema drift test
git diff --check
```

Native crate (once it exists):

```sh
cargo fmt    -p relay-native --check                         # the engine crates are not rustfmt-clean by design (D209)
cargo check  -p relay-native
cargo test   -p relay-native
cargo clippy -p relay-native --all-targets -- -D warnings
```

Preflight: `bash scripts/preflight.sh` (exit 0 required before any native build claim).

## 2. Headless engine proof (brief §9.2)

Run against the `test` instance so `dev` and `stable` stay untouched. Fish syntax. From inside a
Relay agent session, unset `RELAY_SESSION`, `RELAY_TOKEN`, `RELAY_PROJECT`, `RELAY_WORKTREE` and
`RELAY_BRIEF` first, or the `test` engine refuses the inherited actor as an unknown session.
Pass `--store <scratch>/test-store.db` to `serve` so an old `test` store is not migrated by accident.

```sh
set -x RELAY_INSTANCE test
set -x RELAY_BIN ./target/debug/relay            # this repository's build after D203
$RELAY_BIN serve &                                # 1. socket appears under $XDG_RUNTIME_DIR/relay/test.sock
ls -l $XDG_RUNTIME_DIR/relay/                     #    record mode 0600 and the .lock
$RELAY_BIN q app.status                           # 2. a basic request succeeds
$RELAY_BIN events --filter 'session.*' &          # 3. events arrive independently of responses
$RELAY_BIN q workspace.create '{"path":"/home/anthony/dev-2"}'
$RELAY_BIN q project.add '{"workspace_id":1,"path":"/home/anthony/dev-2/Relay-V3"}'   # check `relay schema project.add`
$RELAY_BIN q session.create '{"project_id":1,"provider":"claude"}'   # → name, worktree, branch
$RELAY_BIN q session.spawn  '{"session":"<name>"}'                   # 4. PTY launched without Tauri
$RELAY_BIN attach <name>                          #    output streams; Ctrl-C detaches, the session lives
$RELAY_BIN q session.get    '{"session":"<name>"}' # 5. still running after the client left
$RELAY_BIN q session.close  '{"session":"<name>"}'
kill %1                                            # serve exits; the socket is unlinked, the lock file stays (flock released)
```

Record: socket path and mode, the `app.status` result, one event line with its `cause`, the
session name, and whether `state` stayed `running` after detach. A step that fails is a contract
gap to diagnose before any GUI code (brief §9.2).

## 3. Relay 2 baseline (brief §9.3) → `docs/BASELINE-RELAY2.md`

Same machine, same instance (`dev`), Relay 2 at the pinned commit, built in release
(`cargo build --release -p relay-app` per Relay 2's README; note the profile used). Repeat each
measurement three times; record all three and the command.

| measurement | procedure |
|---|---|
| cold and warm startup, six restored panes | prepare six restorable sessions; `date +%s%N` before launch; the time to interactive is the first frame with all six panes attached (screen capture timestamp or the `engine up` → first `session.attach` span in `~/.local/share/relay/logs`). Cold: after `sync; echo 3 > /proc/sys/vm/drop_caches` (root). Warm: immediate relaunch |
| idle CPU | app open and unfocused, six panes, agents parked: `top -b -d 5 -n 13 -p <pids>` over 60 s; report the mean %CPU of the Relay processes (app and webkit helpers), plus `/proc/<pid>/status` `voluntary_ctxt_switches` delta |
| memory after six panes | `ps -o pid,rss,cmd -p <pids>` for the app and every WebKit process; sum RSS; agents' own processes excluded |
| terminal creation latency | screen recording at 60 fps of "New session → pane visible with prompt"; count frames. Also the `session.create` → `session.spawn` → first `pty` frame span from the log |
| terminal removal latency | same recording method for "remove pane → gone" |
| eleven-pane streaming | eleven sessions each running `yes | head -c 50M` or `cat /dev/urandom | base64`; screen recording; note dropped or stuttering frames; `top` as above during the run |
| scroll and resize | wheel over a terminal inside a scrollable region: which surface scrolled; window resize during output: redraw correctness and lag |
| session survives pane removal | remove a pane while its agent prints; `relay q session.get` shows `running`; re-add or `relay attach` shows continuous output |

## 4. Spike measurements (brief §13)

Same protocol as §3 against `apps/relay-native`, plus:

| measurement | procedure |
|---|---|
| frame smoothness at eleven panes | GTK inspector (`GTK_DEBUG=interactive`) frame-rate overlay, plus a 60 fps screen recording; report the observed rate and any stalls |
| CPU streaming and idle | `top -b -d 5 -n 13 -p <pid>` (one process now: no helpers) |
| memory | `ps -o rss -p <pid>` after six and after eleven panes |
| input latency, focused pane | 240 fps phone recording of keypress to glyph, or `evtest` timestamp to screen-capture frame; report frames |
| hide / reveal / remove cost | time from the action to the next steady frame, from the recording; note whether the attachment was kept |
| reconnection | kill and restart `relay serve`; kill and restart the client; count lost or duplicated bytes against `session.scrollback` |

Profile before optimising (`perf record -g`, or `GTK_DEBUG=` frame timings); the profile goes
in the results doc next to any optimisation it justified.

## 5. Handoff format at the end of every task (brief §21)

Report, in this order, and nothing before the outcome:

1. Outcome: done, partial, blocked, or stop-gate hit.
2. Exact commit (short SHA) and branch.
3. Files changed.
4. Bus operations added or changed; schema regeneration status.
5. Tests and checks run, with the commands.
6. Native dependency versions (`scripts/preflight.sh` output).
7. Measured performance where relevant, with the procedure number from §3 or §4.
8. Visual or device checks actually performed, on which display.
9. Remaining risks.
10. Behaviour that differs from Relay 2, and why.
11. The next smallest vertical slice.

Never report a native visual result that was not observed on a real display.

## 6. Terminal interaction matrix (brief §12), a checklist

Run on a real display, tick each with the session name and date. Success is no lost or duplicated
output, no stuck input, one scroll owner, correct resize, stable reconnection.

- [ ] normal shell prompt
- [ ] Claude Code
- [ ] Codex CLI
- [ ] Kimi or Qwen, if installed
- [ ] mouse selection and copy
- [ ] paste (bracketed; multi-line)
- [ ] wheel and touchpad scrolling
- [ ] Shift+PageUp / Shift+PageDown
- [ ] search, if enabled
- [ ] a full-screen TUI (htop, vim) redraws correctly, including after resize
- [ ] rapid output (`yes`, `cat` of a large file)
- [ ] Unicode, emoji, combining marks, wide characters
- [ ] window resize during output
- [ ] hide, show, move, detach, reattach the pane
- [ ] close the native client while the engine owns a live session
- [ ] relaunch the client and restore the session with history intact
