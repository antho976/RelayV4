# Working in this repository

## Verifying a change

```fish
cargo test                  # engine, bus and CLI — no display, no GTK
cargo clippy --all-targets -- -D warnings
```

Both run in CI on every push and pull request, so a change that breaks them is
caught before it is merged rather than in use.

**Do not run `cargo test --workspace` in a headless environment.** The workspace
includes `relay-native`, and building it stops at `pango-sys` long before any
test runs. Plain `cargo test` uses the default members, which are exactly the
nine headless crates (`relay-bus`, `relay-core`, `relay-cli`, `relay-remote`,
`relay-money` for Tally's money rules, `relay-arbiter` for Arbiter's trading rules, `relay-gym`
for Avex's training history, and the
native client's GTK-free logic:
`relay-board` for the board, `relay-client` for the Git panel's diff/graph/status
helpers and the device mirror's decoder gate and input payloads).
The full list in `README.md` is for a developer machine with GTK.

## The mobile app: `apps/relay-mobile`

An Expo / React Native app with its own toolchain, independent of Cargo. Verify a change with

```fish
cd apps/relay-mobile && npm ci && npx tsc --noEmit -p tsconfig.json && npm run lint
```

Both run in CI. The Android build (`mobile-apk.yml`) runs on demand from the Actions tab or
on a `mobile-v*` tag; an agent session has no Android SDK, so a change there is typechecked
and linted, not run, and the summary must say so. Its PC-facing code is `lib/engine/Relay/`;
the door it talks to is `crates/relay-remote`, whose integration tests send a selection of
the bus payloads the phone sends, not all of them. Before changing an op the phone calls
(`docs/MOBILE.md` names them per screen), check whether a remote test covers it. The new
screens use literal English strings; the rest of the app is localized (`i18n/`), so `i18next/no-literal-string` warns on them.

## Tally: `apps/tally` and `crates/relay-money`

Tally is the budget tracker for Android (Kotlin, Compose, Room, Gradle), its own app with its own
CI (`.github/workflows/tally-*.yml`, run only for changes under `apps/tally`). An agent session has
no Android SDK, so a change there is written, not built, and the summary must say so.

`crates/relay-money` is a port of `apps/tally/core` with the same tests. The phone and the PC each
hold the whole ledger (`docs/MONEY.md`), so **a money rule changed on one side is changed on the
other in the same commit**, tests included. Stored enum names are the Kotlin constant names.

## The native client cannot be built everywhere

`apps/relay-native` pins GTK 4.22, VTE 0.84, GtkSourceView 5.18 and pango 1.56.
Ubuntu 24.04 — the hosted CI runners and the usual agent-session image — carries
GTK 4.14 and pango 1.52, and no combination of `apt` packages closes that gap.

So: **if you change anything under `apps/relay-native/`, you cannot compile it,
and you must say so plainly in your summary and in the pull request.** Verifying
an API you cannot build against by reading the `gtk4`/`gdk4` crate sources under
`~/.cargo/registry` is worth doing, and is still not a compile. Do not describe
such a change as verified.

## Invariants that are easy to break

- **Handlers run inside one SQLite transaction with the store mutex held**
  (`BUS.md` §5.1). Anything slow — a subprocess, a network call, a tree walk —
  blocks every other request, keystrokes included. Register it with
  `Engine::register_unlocked` (queries) or `Engine::register_staged`
  (mutations with a slow read phase) instead of `Engine::register`.
- **Every subprocess forked from a handler goes through
  `proc::output_with_timeout`.** A child that never returns otherwise freezes
  the bus.
- **The store is one connection behind one mutex.** Reads serialize with writes.
  Prefer `prepare_cached` over `Connection::query_row`, which recompiles its SQL
  on every call.
- **`session.input` must not touch SQLite** (D148). It is the one request whose
  latency a person feels.
- Adding a column means appending a migration to `store::MIGRATIONS` and bumping
  `SCHEMA_VERSION`; every earlier version must stay openable.

## Style

The workspace is **not** rustfmt-clean, in places deliberately — roughly 900
hunks differ from rustfmt's default. Do not reformat files you are otherwise not
changing, and do not run `cargo fmt` across the workspace; it would bury a real
change in noise. Match the density of the code around you.
