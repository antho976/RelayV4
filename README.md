# Relay native

Fresh Rust desktop client using **GTK4, VTE and GtkSourceView**, with Relay-2's
compact console and agent wall. No Tauri or webview.

The Rust engine, bus, CLI and their tests come from Relay-2. The native client
is new. [Source revisions](docs/SOURCES.md) identify both references. The full
[V3 roadmap](docs/upstream-v3/ROADMAP.md) and its supporting documents are
preserved for subsequent work; [the current scope](docs/ROADMAP.md) separates
this rebuild from that backlog.

## Run

Requires GTK 4.22+, VTE 0.84+ and GtkSourceView 5.18+. From this checkout:

```fish
./run.sh
```

The launcher builds incrementally, starts the `dev` engine if needed, waits for
it to respond, and opens the window. It also works by absolute path from any
directory. Use `./run.sh stable` or `./run.sh test` to select another instance.
Engine startup logs go to `target/engine-<instance>.log`.
Compiler output is saved to `target/launcher-build.log` and shown on build failure.

V4 uses `$XDG_RUNTIME_DIR/relay-v4/` for engine sockets and
`~/.local/share/relay-v4/<instance>/` for data (or `$XDG_DATA_HOME/relay-v4/`).
This keeps it independent of a running Relay-2/V3 engine. Existing Relay-2/V3
projects and sessions stay in their original store; V4 starts with its own registry.

A running engine is reused without replacement; after engine code changes,
it needs a deliberate restart to pick them up. Closing the window keeps the
engine, sessions and worktrees alive.

The engine `run.sh` starts also carries the phone door, so a phone can reach it
on the LAN directly (`relay remote pair` prints the QR code), or from anywhere
through a rendezvous server you host. The phone app lives in `apps/relay-mobile`
(Expo / React Native; built by the **Mobile APK** workflow). See
[docs/MOBILE.md](docs/MOBILE.md).

Open a repository from the sidebar, then add solo agents or a group of
one or two builders sharing a reviewer. Code can edit the primary checkout or
an agent worktree. Board, Notes, Skills, Settings, usage and Android device tools
use the same bus as the CLI. [Parity coverage](docs/PARITY.md) records the current
implementation and validation boundaries. Mirroring and notification audio use
the installed FFmpeg tools (`ffmpeg` and `ffplay`).

**Plugins** bundle skills, standing agent rules, documentation and MCP servers, and are switched
on per project from the Plugins page or the plugin key on a project's sidebar row. The first is
[Unreal Engine](plugins/unreal-engine/README.md): 19 game-development skills (design, levels,
narrative, UI, animation, AI, C++, Blueprints, GAS, networking and more) and `relay unreal-mcp`,
which builds the project, reads its log and drives a running editor through Remote Control and
Python. New plugins are folders under `plugins/`, compiled into the engine.

## Verify

```fish
cargo test --workspace
cargo fmt -p relay-native --check
cargo clippy -p relay-native --all-targets -- -D warnings -A deprecated
python3 scripts/native-smoke.py
```

CI runs the engine, bus and CLI half of that list — `cargo test --locked` and
`cargo clippy --locked --all-targets -- -D warnings` — on every push and pull
request. The native client is not covered: it pins GTK 4.22, VTE 0.84,
GtkSourceView 5.18 and pango 1.56, which the hosted runners do not carry, so its
checks and the display smoke test still have to be run on a machine that has
them. `.github/workflows/ci.yml` records what a runner would need to close that
gap.

The display smoke test creates its own temporary store, repository and fake
provider executables. It opens the GTK application at two desktop sizes and
checks saves, a review-group launch and eleven-session output delivery. It never launches a paid
model or touches the user's project store. Screenshots go under
`.impeccable/review/`.

See [verification and limits](docs/VERIFICATION.md). The application has no
refresh timers. Each terminal has a bounded stream connection separate from
control requests; project/page changes detach unneeded streams. Engine state
changes refresh the visible surface, and VTE owns terminal scrolling.
