# Audit findings left open: Low, bus / CLI / phone door / plugins / scripts / docs

Scope: the Low findings RA-277..306, RA-533..553, RA-561, RA-578..602 and RA-723..739, fixed
on branch `relay/golden-crane` (2026-10-07). Everything in that range is fixed except the
parts below. Each entry says why it is open and what closing it would take.

## Needs a change in relay-core (held by other sessions at the time)

- **RA-277, door half:** `exception_waited` in `crates/relay-core/src/socket.rs` reads
  `request_id` with `as_i64()`, so `{"request_id":"17"}` misses the stored-answer shortcut.
  Accept a numeric string there too. (The bus half — `matching` must be an object — is fixed.)
- **RA-279, lock half:** `bus.schema` is still registered with `Engine::register` in
  `handlers/bus.rs`. The render is now cached once per process, so the cost under the lock is
  one clone; `register_unlocked` would remove even that.
- **RA-578, error kind:** an invalid actor still comes back as `bus.envelope` from
  `Engine::parse`. Reading `actor` first would let it answer `bus.actor`.
- **RA-580, core leftovers:** `_ctx_used` and `_CALLABLE_USED` in `handlers/bus.rs`.
- **RA-583, optional:** `GateOut` still has `verdict`/`error`/`hold_id`, though the verdict is
  always `allow` (now documented). Shrinking it means changing `handlers/guardrail.rs` too.
- **RA-584, guard:** a `debug_assert` in `Ctx::emit` that the event is one the op declares.
- **RA-585:** the remaining closed-set strings (session state, roles, …) as enums. That needs
  matching changes in core's handlers; only the `group_by` drift was fixed.
- **RA-587, core half:** registry tests for handler coverage and declared emits belong in
  relay-core's tests.
- **RA-602, code half:** on `Lagged`, `socket.rs` should send a gap/resync line, and
  `relay attach` should notice a seq gap and re-attach with `from_seq`. BUS.md §7 already
  states that a lagging subscriber loses frames.
- **RA-551, partial output:** `ue_run_tests` cannot return the stdout it had when it timed
  out until `relay_core::proc::output_with_timeout` returns partial output on timeout.
- **RA-723, shutdown copy:** the phone door's own `sigterm()` and quit wait duplicate
  relay-core's, which is private. Making that public would let the door reuse it. A shared
  `accept_loop`/`open_door`/HTTP-probe helper was left, as the verifier advised.

## Docs outside this range

- **RA-582:** D157 in `docs/engine/DECISIONS.md` still says "TauriOnly; the socket/CLI door
  never receives the secret". BUS.md §10.14 already describes the socket and stdin behaviour.
- **RA-601:** D149 should list the nested `invoke_registered`/`prepare_registered` paths.
  BUS.md §5.1 is corrected.
- **RA-594:** D161 still promises a hand-side check; `hand_sides` is now documented as a
  bone-name hint in the tools and SKILL.md.

DECISIONS.md was being edited by `relay/amber-heron` at the time, so these were left rather
than written as conflicting edits.

## Native client (another session's range)

- **RA-598, code half:** the dead `.files-rail` and `.wall-files-heading` rules in
  `apps/relay-native/resources/theme.css` and the vacuous check at `smoke.rs:75`. DESIGN.md
  now describes the CSS as it is.

## Needs a real Unreal editor

- **RA-550:** the Remote Control keys are still written to the shared
  `Config/DefaultRemoteControl.ini`. Moving them to the per-user `Saved/Config` layer needs a
  real editor to confirm Remote Control reads that layer. `bAllowAnyRemoteFunctionCall` is
  kept because `ue_call` and the Python bridge may need it. The console key is no longer
  written, and the setup advice warns that the ini is shared.
- **Fixed but not run against an editor:** every Unreal change in this range ran only against
  the stand-in `unreal_py/tests/unreal.py`:
  - per-sample pose evaluation (RA-303);
  - cancellable slow tasks (RA-593);
  - registry-checked import cleanup (RA-553);
  - the mip enum check (RA-302);
  - data-table and socket saves (RA-305);
  - crash-reporter matching on `Saved/Crashes` (RA-293);
  - `import_preflight` (RA-588).

## Launcher: run.sh edits refused, need the user

The guardrail and the auto-mode permission classifier refused the edits to `run.sh`, so they
were not made. They need the user to make or allow them.
- **RA-546:** drop `RELAY_BIN`, `RELAY_STORE`, `RELAY_WORKTREE`, `RELAY_PROJECT` and
  `RELAY_BRIEF` from the `serve` environment. The guardrail treats removing
  `RELAY_SESSION`/`RELAY_TOKEN`/`RELAY_ACTOR` as shedding identity.
- **RA-547:** a `notify-send` fallback for launcher failures when stderr is not a terminal.
- **RA-561:** start a new engine only when `relay ping` exits 5 (unreachable). On 124 (busy),
  keep waiting and report "running but not answering".
- **RA-731:** the engine log has no rotation. Doing it properly needs a rolling log writer in
  the engine (crate code); a cap-on-start rotation in run.sh was part of the refused edit.

## Needs member manifests or other sources

- **RA-545, rand:** moving `rand` 0.9 to 0.10 needs source edits in
  `crates/relay-remote/src/registry.rs` and `crates/relay-core/src/sessions.rs`. The gix
  feature trim is done.
- **RA-730:** `rust-version = "1.94"` is set in `[workspace.package]`, but has no effect until
  each member gets `rust-version.workspace = true` (the four `crates/*/Cargo.toml` and
  `apps/relay-native/Cargo.toml`).
- **RA-544, images:** the Dockerfile base images under `deploy/` are not pinned by digest.
- **RA-734 / RA-737:** the fixture and perf harnesses were fixed in place. They were not
  merged into one shared module, which the verifiers rated optional.

## Deliberate or limited

- **RA-286:** UTF-16 ini files are left untouched rather than decoded and merged.
- **RA-287:** `editor_status` still scans the whole log for bind failures, because they are
  logged early in the session.
- **RA-538:** over `ws://` the Bearer header is as visible on the wire as the query string
  was. It no longer lands in URLs or proxy logs. The fallback to `?secret=` for an old
  rendezvous server has no test.
- **RA-541:** a phone may call only the ops the app names (`wire::PHONE_OPS`, 150 of 223).
  That still includes `guardrail.config.set`, `plugin.enable`, `skill.install` and file
  writes, because the app has screens for them. There is no separate `remote:<device>` actor
  in the audit log. A new op the phone calls must be added to `PHONE_OPS`.
- **RA-724:** no `wss://` dial test, because it needs a TLS server.
