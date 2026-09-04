# Relay native

Fresh Rust desktop client using **GTK4, VTE and GtkSourceView**, with Relay-2's
compact console and agent wall. No Tauri or webview.

The Rust engine, bus, CLI and their tests come from Relay-2. The native client
is new. [Source revisions](docs/SOURCES.md) identify both references. The full
[V3 roadmap](docs/upstream-v3/ROADMAP.md) and its supporting documents are
preserved for subsequent work; [the current scope](docs/ROADMAP.md) separates
this rebuild from that backlog.

## Run

Requires GTK 4.22+, VTE 0.84+ and GtkSourceView 5.18+. Build both binaries:

```fish
cargo build -p relay-cli -p relay-native
set -x RELAY_INSTANCE dev
./target/debug/relay --instance dev serve
```

In a second terminal, with the same instance:

```fish
set -x RELAY_INSTANCE dev
./target/debug/relay-native
```

An existing engine for that instance can be used instead of starting another.
The matching rebuilt engine provides `guardrail.hold.get`; an older engine
can render the shell but cannot provide exact hold review. The client never
silently replaces a running engine. Closing it keeps sessions and worktrees.

Open a repository from the sidebar, then add a solo builder or a builder with
a reviewer. The Code tab currently edits the project checkout. Agents work in
their own engine-managed worktrees. Tasks, mailbox, holds and project notes
use the same bus as the CLI.

## Verify

```fish
cargo test --workspace
cargo fmt -p relay-native --check
cargo clippy -p relay-native --all-targets -- -D warnings
python3 scripts/native-smoke.py
```

The display smoke test creates its own temporary store, repository and fake
provider executables. It opens the GTK application at two desktop sizes and
checks that six sessions survive window closure. It never launches a paid
model or touches the user's project store. Screenshots go under
`.impeccable/review/`.

See [verification and limits](docs/VERIFICATION.md). The application has no
refresh timers. Each terminal has a bounded stream connection separate from
control requests; project/page changes detach unneeded streams. Engine state
changes refresh the visible surface, and VTE owns terminal scrolling.
