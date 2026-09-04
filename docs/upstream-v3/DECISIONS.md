# Decisions — Relay 3

Numbering starts at D200 so an entry can be cited unambiguously next to Relay 2's D1–D158
(`~/dev/Relay-2/docs/DECISIONS.md`), which continue to bind the engine and the bus. Append; never
rewrite. Each entry says whether it is **accepted** or **proposed**, and by what evidence.

- **D200 Relay 3 lives in its own repository.** Accepted 2026-09-03 by Antho creating
  `antho976/Relay-V3` and registering it as a Relay project. The brief (§7) originally placed the
  spike at `apps/relay-native/` inside Relay 2; that path now means this repository. Relay 2 stays
  at `~/dev/Relay-2`, unmodified by Relay 3 work, as the executable specification, the daily driver
  and the rollback target until Stage 9 (`docs/ROADMAP.md`) is passed. Consequence: the engine
  crates must exist here too (D203), and Relay 2 fixes that produce reusable truth (brief §17) are
  made in Relay 2 and carried over, not forked.
- **D201 Stack is GTK 4, VTE 4 and GtkSourceView 5 through gtk4-rs, vte4 and sourceview5, in
  Rust.** Proposed by the brief (§4–§5); becomes accepted when the spike's proceed gate (§14)
  passes. Plain gtk4-rs first; libadwaita is not a foundation dependency; Qt and the Rust-first
  renderers are fallbacks only against a measured blocker, never taste. Crate versions are pinned
  in `apps/relay-native/Cargo.toml` when the crate is created, with the GTK feature level matching
  the installed 4.22 (`docs/READINESS.md`).
- **D202 The bus stays the product boundary; the native client is a socket-door client of
  `relay serve`.** Accepted, from brief §6.1–§6.2 and Relay 2 BUS.md §0. No widget calls a
  relay-core handler; no native-only side door; a missing capability becomes an op first. Initial
  process model is client + `relay serve`; a long-lived `relayd` is decided after the spike
  (brief §4, §15).
- **D203 Engine crates are imported from Relay 2 at a pinned commit by plain copy, not by
  history rewrite or a path dependency.** Import run 2026-09-03 (`scripts/import-engine.sh --run`)
  at `1745dd3f68b7bc786f48142ad7da591537aeb057`, the full commit in `ENGINE-ORIGIN`; accepted and
  committed as `403fa4a` on 2026-09-03 after Antho raised the guardrail caps.
  Options weighed:
  (a) plain copy of `crates/relay-bus`, `crates/relay-core`, `crates/relay-cli`, `schema/`,
  `Cargo.lock` and the engine docs at a named commit, recorded in `ENGINE-ORIGIN`;
  (b) `git subtree` / filter-repo import preserving history; (c) a `path = "../Relay-2/crates/…"`
  dependency. (c) couples two checkouts and breaks the moment either moves. (b) drags the Tauri
  app and 148 skill files through history for no gain, since Relay 2's history stays alive next
  door. (a) is one command (`scripts/import-engine.sh --run`), reviewable as one commit, and the
  origin commit is written down. Cost of (a): engine fixes made in Relay 2 after the pin are
  ported by cherry-pick or re-copy; the brief already says those fixes belong in Relay 2 (§17).
  Pin proposed: `1745dd3` (Relay 2 `main` on 2026-09-03). The import is larger than the builder
  guardrail file cap (60), so Antho runs it as the user actor.
- **D204 Relay 3 decisions are numbered from D200.** Accepted 2026-09-03. Two logs with
  overlapping numbers would make "see D18" ambiguous in a handoff.
- **D205 The spike ships plain widgets; Claude owns the UI after the spike.** Accepted, from
  brief §6.7 and §7. No theme tokens, icon system, design system or product styling before the
  spike results exist. Proof code and product styling stay in separate files so the former can be
  deleted.
- **D206 Scripts report; only Antho installs.** Accepted, from brief §4 and §9.1.
  `scripts/preflight.sh` exits non-zero and prints the exact `pacman` line; it never runs it.
- **D207 The brief is the intent document; the other docs digest it.** Accepted 2026-09-03.
  `docs/RELAY-3-NATIVE-BUILD-BRIEF.md` was moved from the repository root (`Relay Native rebuild`)
  to the path its own §22 names, body unchanged, with a five-line location note. When a digest
  (`CONTRACTS.md`, `ARCHITECTURE.md`, `ROADMAP.md`) and the brief disagree, the brief wins until
  Antho edits the brief; a digest is fixed to match, never the reverse.
- **D208 Product-version naming stays as Antho uses it.** Accepted, from the brief's naming note.
  Relay 2 is the Tauri application, Relay 3 is this rebuild; Relay 2's internal "v4" labels are
  history, not a rename licence.
- **D209 rustfmt gates only `apps/relay-native`; the engine crates keep Relay 2's formatting.**
  Accepted 2026-09-03. `cargo fmt --all --check` reports 929 hunks on the imported crates, and
  exactly the same 929 in Relay 2, which has no `rustfmt.toml` and no fmt step in CI. Reformatting
  the copy would make every engine patch unportable in both directions (D203 relies on
  cherry-picks and `docs/patches/`). So CI and the cadence rules run `cargo fmt -p relay-native
  --check` only, and an engine change is formatted by hand in the file's existing style.
- **D210 The native client mirrors engine-held shell state; it does not register as a `ui.*`
  executor.** Accepted 2026-09-03 on P2 evidence (CONTRACTS §11): at Relay 2 `1745dd3` the
  `ui.*` handlers execute in core against engine state and emit `ui.changed`; no executor door
  exists on any door, and nothing reads the registry's `Executor::Ui` attribute. Relay 3 therefore
  subscribes to `ui.changed`, applies it to its widgets, and drives changes through the same
  `ui.*` ops as any client. Whether the `.ui()` attribute is removed from the registry, or a real
  socket executor door is added for agent-driven UI actions, is a Stage 1 bus decision made in
  Relay 2 first (brief §17).
- **D211 The socket reader never blocks on a pane.** Accepted 2026-09-03. Each attached session
  has a bounded frame channel; when a pane stops draining it, the reader drops the frame with a
  warning instead of stalling every response behind one slow widget. The pane detects the `seq`
  gap and re-attaches from its last `(epoch, seq)`, the same recovery the engine's own lag drop
  requires (CONTRACTS §5). Consequence for P3: the VTE consumer must drain on the main thread at
  frame rate and treat a gap as a re-attach, never as corruption.
- **D212 Binding feature levels match the installed libraries.** Accepted 2026-09-03.
  `gtk4` builds with `v4_22`, `vte4` with `v0_84`, `sourceview5` with the highest level at or below
  the installed 5.20 (`docs/READINESS.md`). Found the hard way: `vte4 0.10` needs
  `gtk::Accessible`, which gtk4 0.11 hides behind `v4_10`, so the default feature set does not
  even compile the terminal binding. A machine with older libraries lowers these flags; nothing
  else changes.
- **D213 D201 is accepted; PTY output is coalesced before sequence assignment.** Accepted
  2026-09-04 on the repaired P4a proceed-gate evidence. Raw PTY reads are gathered for one 17 ms
  display interval and capped at 64 KiB before the engine assigns a sequence, retains the frame,
  and broadcasts it. This preserves exact `(epoch, seq)` resume semantics and D211's bounded,
  non-blocking client route while applying backpressure at the producer. Eleven simultaneous
  50 MiB streams completed with zero gaps, duplicates, re-attaches, client drops, or engine
  subscriber lag. Consequence: GTK 4, VTE 4, and GtkSourceView 5 are the accepted Relay 3 stack;
  later engine imports must preserve this repair until it is ported back to Relay 2.
- **D214 The spike proceed gate is accepted; Stage 1 keeps `relay serve` as a separate process.**
  Accepted 2026-09-04 by Antho authorizing the build to continue through the first viewable native
  checkpoint after reviewing the repaired P4a result. The GTK client connects through the socket
  door and may start, reconnect to, or close independently from the engine; closing the UI never
  owns session lifetime. A new `relayd` executable adds no proven value yet, so Stage 1
  productionizes the existing `relay serve` boundary and revisits supervision only if lifecycle
  evidence requires it. D200 keeps Relay 2 available, D202 fixes the socket boundary, D210 fixes
  shared UI-state ownership, and D213 accepts GTK 4, VTE 4, and GtkSourceView 5.
- **D215 Native accessibility is part of the widget contract, not a later polish pass.** Accepted
  2026-09-04 with the Stage 1 continuation. Product controls use GTK semantics, expose specific
  accessible names, remain keyboard reachable with visible focus, and honor system text scaling
  and high-contrast preferences. Custom color, spacing, and typography may style those controls;
  they may not replace semantics with pointer-only drawing. Display and assistive-technology
  checks remain required evidence before a surface is called complete.
- **D216 Relay 2's icon set is ported as GTK 4.22 SVG, not approximated by stock symbolics.**
  Accepted 2026-09-04 while giving the native shell its missing pages. `lib/Icon.svelte` is the
  source of truth for Relay's 16px single-weight stroke icons; the native shell had been drawing
  Adwaita stock names, which made every key and nav row read as a generic GTK application. All 54
  icons now live in `apps/relay-native/icons/` as one file each, generated from the Svelte source
  so they cannot drift by hand. GTK 4.22 renders SVG itself and recolours elements carrying
  `foreground-stroke` and `foreground-fill` with the widget's CSS colour, which is exactly what
  `currentColor` does in Relay 2: one file serves rest, hover, disabled and selected. Consequence:
  a new icon is added to `Icon.svelte` first and ported, never invented natively; `icons.rs` has
  a test that every name in Relay 2's `IconName` union resolves.
- **D217 The editor's colours are Relay's, installed as a GtkSourceView style scheme.** Accepted
  2026-09-04. `styles/relay.xml` is the port of `lib/cmTheme.ts` and the `SYN_MATTE` palette, so
  the Code page's editor matches Relay 2 instead of GtkSourceView's light `classic` default.
  GtkSourceView loads schemes from directories only, so `syntax.rs` writes Relay's own file into
  the user's data directory once per run and adds that directory to the manager's search path;
  nothing in the user's tree is read or replaced. A failure to install falls back to
  `Adwaita-dark`, which is a worse-looking editor rather than a broken one.
- **D218 Pages read the engine on show and on the events they name, never on a timer.** Accepted
  2026-09-04. Each page declares which event prefixes concern it (`Page::wants`); the shell
  re-reads only the page that is on screen, plus the sidebar hierarchy, the bell's unread tally
  and the status strip's meters. This keeps the no-polling rule while making every surface live,
  and it means a page that is not visible costs nothing. Consequence: a new page must declare its
  events or it will look stale, and a burst of events must not stack requests — the Dashboard
  guards its own in-flight refresh.
- **D219 Relay draws its own glyph ink; GTK's symbolic path does not reach a `GtkSvg` paintable.**
  Accepted 2026-09-04 after every icon in the shell rendered black. A `GtkImage` holding a
  `GtkSvg` does not take its ink from the widget's CSS colour the way an icon-theme symbolic
  does, and no CSS rule — on the image node or any ancestor — changed it. `icons.rs` therefore
  reads the image's own colour with `gtk_widget_get_color` and rebuilds the paintable at that
  ink on map, on every state change, and on a style-class change the caller announces with
  `icons::repaint_glyphs`. The stylesheet stays the source of truth, which is what `currentColor`
  gives Relay 2. Consequence: an icon's ink is a CSS fact, stated once per context in `theme.rs`,
  and a new interactive surface that toggles a style class in code must ask for the repaint.
- **D220 A rejected stylesheet is reported, not absorbed.** Accepted 2026-09-04. `GtkCssProvider`
  drops a stylesheet from the offending rule onward without saying anything, which during D219
  was indistinguishable from a rule that parsed but did nothing. `theme::install` now connects
  `parsing-error` and logs the section and message at `error`. A theme error is a defect.
- **D221 The status strip carries work, not engine internals.** Accepted 2026-09-04 on Antho's
  review. The pid, the socket path and the `connected: local engine` line are logs, not chrome:
  they were readable at a glance and told the user nothing they could act on. The strip is
  Relay 2's — project, branch, machine, then provider usage and the resource readout. Connection
  state is shown where it can be acted on: the red banner and the controls it disables. A
  provider that has reported no usage window says "usage unavailable" once, rather than showing a
  row of meters with no numbers behind them, because an empty meter reads as a broken meter.
- **D222 The app bar is the window titlebar; Relay draws no second bar.** Accepted 2026-09-04 on
  Antho's report that the shell showed a compositor title bar above its own. Relay 2 runs with
  `decorations: false` (`tauri.conf.json`) and draws its own bar, including the window controls.
  GTK reaches the same result through client-side decorations: the bar is installed with
  `gtk_window_set_titlebar` inside a `GtkWindowHandle`, which stops the compositor drawing its
  own while keeping the drag region, double-click-to-maximise and the resize edges that
  `set_decorated(false)` would throw away. `theme.rs` clears GTK's default `.titlebar` chrome so
  the bar keeps its own. Consequence: the bar is no longer a child of the content box, so
  anything that assumed a single root column must reach it through the window.
- **D223 Board rules live in `board_model.rs`, apart from the widgets.** Accepted 2026-09-04.
  Relay 2 keeps `lib/board/model.ts` free of components for the same reason: the lens, the family
  ordering and the grouping are rules with edge cases — a subtree filter that must keep a
  grandchild, a `~` bucket that always sorts last, a badge that must not count whitespace as an
  opinion — and they are worth testing without a display. The port carries them one-for-one with
  their tests. Consequence: a board rule is changed in the model and asserted there; the page
  only renders what the model decided.
- **D224 Two Relay 2 device ops cannot be ported: the engine refuses them on this door.** Accepted
  2026-09-04. `device.mirror.start` and `device.signing.create` are declared `Doors::TauriOnly`,
  and `engine.rs` returns `bus.door` for them on the socket. The native client is a socket client
  (D202), so screen mirroring and creating a Relay-owned Android signing key are unavailable to
  it — a contract fact, not a missing feature. The device panel therefore does not offer a mirror
  window or a create-key form, and says why where a reader would look for them. Changing this is
  an engine decision about the door, not a client change.
