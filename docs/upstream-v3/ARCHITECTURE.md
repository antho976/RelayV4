# Architecture of the native client

Design for the spike, derived from the brief (§3–§4, §6, §11–§13) and Relay 2's `BUS.md`.
Nothing on this page is proven yet; `docs/RELAY-3-NATIVE-SPIKE-RESULTS.md` will confirm, amend or
reject each section, and Claude's UI specification will reshape the widget tree after the spike.
Keep this page honest: when the spike disproves a line, edit the line and add a D-entry.

## 1. Process model

```
┌──────────────────────────────┐      NDJSON over Unix socket      ┌──────────────────────────────┐
│ apps/relay-native            │ ◄──────────────────────────────► │ relay serve (relay-core)      │
│  GTK main thread: widgets    │  $XDG_RUNTIME_DIR/relay/<i>.sock │  store.db (SQLite, exclusive) │
│  tokio thread: socket reader │                                  │  PTYs, ring buffers, watchers │
│  VTE panes, GtkSourceView    │                                  │  gix reads, git mutations     │
└──────────────────────────────┘                                  │  guardrails, audit, mailbox   │
                                                                  └──────────────┬───────────────┘
 agents (claude / codex CLIs in .relay/worktrees/<session>)  ── relay CLI / MCP ─┘  same socket
```

- The client is one more socket-door client, exactly like the CLI and MCP (D202). It has no
  private door and no engine code linked in.
- Initial model: the client connects to a `relay serve` that is started separately. Whether the
  client starts the engine as a child, or a `relayd` outlives every client, is a Stage 1 decision
  taken on spike evidence (brief §4, §15). The spike must show that closing the client leaves the
  engine and its sessions alive, and that relaunching reattaches (brief §12 matrix).
- Instances: development uses `dev`; automated tests use `test`; `stable` is never touched.

## 2. Crates

```
Cargo.toml                 workspace; default-members = relay-bus, relay-core, relay-cli
crates/relay-bus           reused as a library by the client: envelope, BusError, op payload and
                           result types. The client never hand-writes JSON shapes it can import.
crates/relay-core          engine; never a dependency of the client
crates/relay-cli           `relay serve` and the agent door
apps/relay-native          NOT in default-members during the spike (GTK must not break `cargo test`)
  Cargo.toml               gtk4, vte4, sourceview5, glib, tokio, async-channel, serde_json,
                           uuid, base64, relay-bus. Versions pinned; GTK feature level = installed 4.22
  src/main.rs              gtk::Application, instance selection, runtime start
  src/app.rs               windows, layout areas, focus, geometry restore, shutdown order
  src/bus_client.rs        the socket client (§4)
  src/terminal.rs          one VTE pane bound to one session (§5)
  src/editor.rs            one GtkSourceView bound to one file (§6)
```

Placeholders only in the spike. Proof code and product styling live in different files so proof
code can be deleted (D205).

## 3. Threads and the async bridge

- The GTK main thread owns every widget. A tokio runtime on one dedicated background thread owns
  the socket, the reader task and the writer path.
- tokio → GTK: an `async_channel` receiver awaited from `glib::spawn_future_local` on the default
  main context (or `glib::idle_add_local_once` for one-shot deliveries). Every widget mutation
  happens inside that closure, on the main thread.
- GTK → tokio: `runtime.spawn` for requests; an `mpsc` sender for the writer.
- Rules: no widget touched off the main thread; no `block_on` on the main thread; no timers.

## 4. `bus_client.rs`

```rust
struct Client {
    writer:  mpsc::Sender<String>,                              // one line per send
    pending: Mutex<HashMap<Uuid, oneshot::Sender<Response>>>,   // request correlation
    events:  async_channel::Sender<Event>,                      // one consumer on the main thread
    streams: Mutex<HashMap<String /*session*/, async_channel::Sender<PtyFrame>>>,
}
```

- `request(op, payload)`: mint a UUID, register the oneshot, write the line, await the response.
  Returns `Result<T, BusError>` with `T` deserialised through relay-bus result types.
- Reader task: `read_line` → classify (`ev` → event, `stream` → frame, else response by `id`)
  → route. Unknown shapes become a typed local error (`client.protocol`) and are logged, not fatal.
- Frames route by `session` to the pane that attached. `logcat` and `log` route by `run_id` or
  a single sink, when those panes exist.
- Disconnect: every pending oneshot resolves with `client.disconnected`; stream senders close so
  panes render a detached state; the event consumer gets one `Disconnected` marker. Reconnect is
  an explicit action (socket error handler or user), followed by re-query, never event replay.
- Tests: a Unix socket fixture that scripts interleaved response, event and frame lines; a
  malformed-line case; a disconnect-with-pending case; an out-of-order response case. The real
  in-process test engine is used where it is smaller and more faithful.
- Not built: an actor framework, a reactive store, a generic RPC layer. The bus already is the
  architecture (brief §11).

## 5. `terminal.rs`: a VTE pane over a core-owned PTY

Output path
1. `session.attach {session, epoch?, from_seq?}` with the last known `(epoch, seq)` when
   reattaching, none on first attach.
2. The first frame is the bounded catch-up; feed it, then live frames, in arrival order, with
   `vte::Terminal::feed`. Base64 is decoded on the tokio side; bytes cross to the main thread.
3. Track `(epoch, seq)`. Within an epoch, `seq` must be `last + 1`. A gap is a detected fault:
   log it, `session.detach`, re-attach from the last good `(epoch, seq)`. A new epoch (spawn or
   wake) resets the terminal before feeding.
4. On widget destruction: `session.detach`. The session lives on (Contracts §6).
5. Reattach after client restart: `session.scrollback` for history the ring no longer holds is a
   Stage 2 question; the spike proves that engine-owned history is not lost, not that every byte
   is redrawn.

Input path
- VTE's `commit` signal delivers what the user typed, already translated (arrows, modifiers,
  bracketed paste, control sequences). Forward it unchanged as `session.input {session, data}`.
  No local echo: the PTY echoes.

Resize path
- On `char-size-changed` and size allocation, read `column_count()` and `row_count()`; coalesce
  through one pending main-context callback (idle or frame clock); send `session.resize` only
  when the pair changed. Never poll size.

Scroll ownership
- The VTE widget owns wheel, touchpad and Shift+PageUp/PageDown. No outer scrolled window around
  a terminal. This is the "one obvious scroll owner under the pointer" requirement (brief §18).

Hidden and moved panes
- Keep the attachment while a pane is hidden or moved between containers; VTE keeps its state
  off-screen. P4a measures whether that is affordable at eleven panes; if not, detach on hide and
  reattach from `(epoch, seq)` on show.

## 6. `editor.rs`: GtkSourceView over `file.read` / `file.write`

- `file.read {project_id, worktree?, path}` → `text` into a `sourceview5::Buffer`; language
  guessed from the path by `LanguageManager`; line numbers, current-line highlight, search via
  `SearchContext`; read-only by `set_editable(false)`; dirty state from the buffer's modified flag.
- Save is `file.write {project_id, worktree?, path, text}`. A `held` response (guardrail) shows
  the hold and its `confirm` op; a `refused` shows the reason. The widget owns no filesystem logic.
- Large-file and UTF-8 behaviour are measured in P4b, not assumed.

## 7. `app.rs` / `main.rs`

- One `gtk::Application`. Application id for the spike is a placeholder in the `.dev` namespace
  (`com.quietsoftware.relay.native.dev`); the product id is a Stage 1 decision next to the
  Relay 2 ids `com.quietsoftware.relay` / `.relay.dev`.
- Main window with four placeholder areas (header or command area, sidebar, centre, status).
  One secondary window that opens and closes cleanly. Basic geometry restore.
- Geometry and shell state go through the existing `ui.*` state ops if their shape fits; if not,
  that is a contract gap to record (Contracts §11), not a reason for a local settings file.
- Shutdown order: detach every stream, cancel pending requests, close the socket, stop the
  runtime, quit the application. The spike proves no client process or socket connection outlives
  the window (brief §10).

## 8. Styling

Spike: GTK's default theme, no CSS beyond what proves a capability. After Claude's UI direction:
one GTK CSS provider fed by application-owned tokens; Pango for typography; GSK custom rendering
only where ordinary widgets cannot do the job (wallpaper, overlays). libadwaita patterns adopted
individually when they materially help, never as the foundation (D201).

## 9. State ownership

| owner | state |
|---|---|
| engine | everything durable: sessions, tasks, notes, files, git, guardrails, audit, layouts persisted through `ui.*` |
| client | window geometry until persisted, VTE screen state, unsaved editor buffers, the last `(epoch, seq)` per attachment |

The client never caches engine truth beyond the last event; on reconnect it re-queries.

## 10. What the spike must answer before this page is trusted

- VTE feed throughput with eleven live panes; CPU idle and streaming; memory; input latency.
- Catch-up size versus terminal reset cost on reattach.
- Whether hidden panes keep or drop their attachment.
- Whether `relay serve` is a child of the client or independent (engine lifetime).
- The `ui.*` executor gap and the base64 cost on the socket door (Contracts §11).
- Whether plain gtk4-rs is enough for the shell, or one libadwaita pattern earns its place.
