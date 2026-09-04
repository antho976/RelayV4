# Contracts the native client must honour

A digest of Relay 2's `docs/BUS.md` (§1, §5, §6.2, §7, §8.2, §9) and the brief's §6, written for
whoever writes `apps/relay-native`. BUS.md is the source of truth. When this page and BUS.md
differ, BUS.md wins and this page is fixed (D207). Shapes below were checked against
`relay schema <op>` on 2026-09-03 against Relay 2 `1745dd3`.

## 1. The envelope

```ts
interface Request  { v: 1; id: string; actor: Actor; op: string; payload: object; token?: string }
type Response =
  | { v: 1; id: string;        ok: true;  result: object; replayed?: true; mail?: { priority: number } }
  | { v: 1; id: string | null; ok: false; error: BusError; replayed?: true; mail?: { priority: number } };
interface BusError { kind: Kind; code: string; message: string; details?: object; hint?: string;
                     confirm?: { op: string; payload: object } }   // confirm present iff kind === "held"
interface Event    { v: 1; ev: string; ts: string; actor: Actor; cause?: string; project_id?: number; payload: object }
type Actor = "user" | `agent:${string}` | "test" | "system";
```

- `id` is a UUID v4 minted by the client. It is also the idempotency key: a resend with the same
  id returns the recorded result with `replayed: true` instead of executing again.
- The native client is actor `user`. Agents reach the bus with `RELAY_SESSION` + `RELAY_TOKEN`
  through the CLI and MCP; the GUI never impersonates an agent and never sends `system`.
- `project_id` is a payload field, never an envelope field.
- `mail.priority` is live metadata for agent sessions. The GUI may show it; it must not act on it.

## 2. Error kinds and what the client does with them

| kind | meaning | client behaviour |
|---|---|---|
| `invalid` | bad envelope, unknown op, payload fails schema | a client bug: log with the code, surface as a diagnostic, never retry unchanged |
| `not_found` | referenced entity does not exist | drop the stale reference, re-query the list it came from |
| `conflict` | well-formed, but current state forbids it | show `message` and `hint`; the user decides |
| `refused` | policy said no, no confirm path | show `message` and `hint`; no retry |
| `held` | policy paused it; `confirm` names the op that lifts the hold | present the hold and the `confirm` op to the user; the GUI is the user actor and may call it after the user chooses to |
| `unavailable` | dependency absent: provider CLI, device, git, UI executor | show `hint`; degrade the surface, do not spin |
| `internal` | engine bug, `details.trace` carries a trace id | surface the trace id; never parse `message` |

Dispatch on `kind` and `code`, never on `message`.

## 3. Events

- Emitted after commit. Transitions only, never activity: `session.changed` does not fire because
  output arrived; the busy indicator is derived from the `pty` stream.
- Best-effort delivery. A client that was not subscribed re-queries: every event has a query op
  that yields the same truth. Consequence: on reconnect, re-query, do not attempt replay.
- `file.changed` is coalesced (≥ 100 ms per worktree) and carries a list of paths.
- `resource.sample` only while `app.resources.watch {on: true}` or from the 60 s reconcile pass.
- Eleven streaming panes produce eleven `pty` streams and zero bus events per second.

## 4. The socket door

- Path `$XDG_RUNTIME_DIR/relay/<instance>.sock`, mode 0600, `instance ∈ {stable, dev, test}`.
- Newline-delimited JSON. Each line is one `Request`, one `Response`, one `Event` or one frame.
  Pipelining is allowed; responses are correlated by `id`, not by order.
- `bus.subscribe {events?: string[]}` (socket door only) turns the connection into a subscriber;
  `Event` lines are then interleaved with responses.
- Line classification, in this order: has `ev` → Event; has `stream` → data-plane frame;
  otherwise → Response by `id`. Anything else is a malformed line: the client raises a typed
  local error (its own `kind`/`code`, e.g. `client.protocol`) and keeps reading.
- One engine per instance: `flock` on `$XDG_RUNTIME_DIR/relay/<instance>.lock`, store opened
  `locking_mode=EXCLUSIVE`. A second `relay serve` exits 5 with "engine already running (pid N)".
- The socket is served by whichever process owns the engine: the Tauri app, or `relay serve`.
  Relay 3 connects to `relay serve` (D202).
- Relay 2's own `socket::Client` is a small sequential helper. Do not base the GUI on it
  (brief §6.2): write one reader that routes concurrently.

## 5. Data-plane frames

| stream | attached by | frame line | notes |
|---|---|---|---|
| `pty` | `session.attach {session, epoch?, from_seq?}` | `{v:1, stream:"pty", session, epoch, seq, data: base64}` | ≤ 60 fps, coalesced; backpressure coalesces, never drops. First frame after attach is the catch-up |
| `logcat` | `device.run`, `device.build` | `{v:1, stream:"logcat", run_id, seq, line}` | a build streams Gradle output only, then ends |
| `log` | `app.log.tail` | `{v:1, stream:"log", seq, record}` | Relay's structured log |
| `mirror` | `device.mirror.start` | H.264 NAL units | Tauri `Channel` only; the socket door returns `unavailable`. A native mirror is a Stage 7 contract question, not a spike one |

`epoch` increments on every spawn and wake; `seq` restarts at 0 within an epoch.
`session.scrollback {session, lines?}` returns `{text, epoch, seq}` so a client can resume
exactly. If a subscriber lags, the engine logs "pty subscriber lagged; frames dropped — client
should re-attach from its last seq"; the client detects the gap by `seq` and re-attaches from its
last `(epoch, seq)`. Input is not a stream: it is the op `session.input`.

## 6. PTY ownership (brief §6.3)

The engine owns the process, the ring buffer, sequencing, attach and detach, input and resize.
A VTE widget is a renderer and an input surface; it never spawns a child.

| fact | value (Relay 2 `1745dd3`) |
|---|---|
| retained scrollback | 8 MiB per session (`pty.rs` `SCROLLBACK_BYTES`) |
| catch-up on attach | at most 256 KiB (`pty.rs` `ATTACH_CATCHUP_BYTES`), starting on a frame boundary; the last 8192 frame boundaries are indexed for `from_seq` replay; older history via `session.scrollback` |
| teardown order | kill child → drop master → abort reader (Relay 2 D18); zero orphans is tested |
| pane disappears | the session lives on; only the attachment ends (`session.detach`) |
| resize | `session.resize {session, cols, rows}`, user actor; coalesced by the client, never polled |
| input | `session.input {session, data}`, user actor, audited |

## 7. The ops a terminal pane and an editor use

| op | kind | actors | payload | result |
|---|---|---|---|---|
| `session.create` | mutation | user | `project_id, provider, role?, model?, effort?, branch?, worktree?, task_id?, module_id?, pair_with?, bus_writes?, allow_ui?` | `Session` |
| `session.spawn` | mutation | user | `session, prompt?` | `Session` |
| `session.attach` | query, stream `pty` | all | `session, epoch?, from_seq?` | frames follow |
| `session.detach` | query | all | `session` | – |
| `session.input` | mutation | user | `session, data` | – |
| `session.resize` | mutation | user | `session, cols, rows` | – |
| `session.scrollback` | query | all | `session, lines?` | `text, epoch, seq` |
| `session.get` / `session.list` | query | all | `session` / filters | `Session` / `{sessions}` |
| `bus.subscribe` | query, socket only | all | `events?` | `{subscribed}` |
| `file.read` | query | all | `project_id, worktree?, path, max_bytes?` | `text?, bytes_b64?, mime, size, truncated` |
| `file.write` | mutation | all | `project_id, worktree?, path, text` | `bytes, removed_lines, added_lines`; may return `held` |
| `app.status` | query | all | – | `pid, uptime_s, store_path, socket_path, ui_connected, sessions_live, providers` |

`user` in the actors column means `user_only`: agents cannot type into or resize a terminal. Exact
shapes: `relay schema <op>` or `bus.schema`.

## 8. Worktrees and removal safety (brief §6.4, §18)

Every agent works in an isolated worktree under `.relay/worktrees/<session>`. Removing a
project, workspace or navigation entry is not permission to delete a repository, worktree, branch,
terminal or unsaved buffer. Destructive scope is explicit in the op and its confirmation; there
is no automatic cleanup that discards state.

## 9. No polling (brief §6.5)

No UI timers, no idle subprocesses, no repeated filesystem scans. The engine's watchers and burst
coalescing exist so the client never has to. A reconnect is triggered by a socket error or a user
action, not by a clock. The one clock in the system is the engine's 60 s reconcile pass.

## 10. Budgets (brief §6.6, Relay 2 SPEC §15)

| target | value |
|---|---|
| startup to interactive, six restored panes | < 2 s |
| eleven streaming panes | visually smooth, no drops |
| idle CPU, app open, unfocused | < 1 %, zero subprocess spawns per minute |
| terminal creation and removal | near-instant, perceived |
| memory | Relay itself < 1 GiB with six panes, agents' own RAM excluded |
| output loss when panes hide, move, detach | none |

Measured on a real display (`docs/VERIFICATION.md` §3–§4), never claimed from code.

## 11. Contract gaps to diagnose during P2, not paper over

- `ui.*` ops. BUS.md §6.5 describes a Tauri-only executor door (`bus:ui-op` / `bus:ui-result`).
  Diagnosed during P2 (2026-09-03): at Relay 2 `1745dd3` no such door exists in relay-core either.
  `handlers/ui.rs` executes `ui.state`, `ui.page.switch` and `ui.pane.*` in core against
  engine-held shell state and emits `ui.changed`; nothing checks the registry's `Executor::Ui`
  attribute. Consequence for Relay 3: the client subscribes to `ui.changed` and mirrors
  engine-held shell state (D210); whether the `.ui()` executor attribute is dropped from the
  registry or a real socket executor door is added is a bus decision for Stage 1.
- The `mirror` stream is Tauri-only. Relay 3 needs a socket or shared-memory path in Stage 7.
- `session.attach` on the socket door base64-encodes bytes (Relay 2 PERFORMANCE.md F2). Measure
  the cost at eleven panes before deciding it matters.

## 12. Changing the contract

1. Declare the op in `crates/relay-bus/src/ops/<ns>.rs` with `op!`, `payload!`, `result!`; add it
   to the module's `entries!`. Schema names are namespaced (`TaskMoveIn`), never bare.
2. `cargo run -p relay-bus --example dump_schema > schema/bus.v1.json`; the drift test fails otherwise.
3. Implement the handler in `crates/relay-core/src/handlers/<ns>.rs`, inside the pipeline.
4. Add a bus-driven test through the socket door (`crates/relay-core/tests/bus.rs` style).
5. Only then touch the client. Never fork the schema; never add a native-only side door.
6. Make the same change in Relay 2 first when it is reusable truth (brief §17, D203).
