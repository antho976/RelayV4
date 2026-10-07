# Audit findings left open: native client Lows, first half

Scope: RA-425..480 (`apps/relay-native`), fixed on branch `relay/sly-koala` (2026-10-07).
RA-425 and RA-471 were already fixed before this pass. Everything else in the range is fixed
except the parts below. Each one says why it is open and what closing it would take.

None of these fixes has been run in the GTK app. relay-native compiles and its unit tests pass on
this machine, but no GUI smoke or manual check was done.

## Needs the engine

- **RA-480: the module editor opens as unsaved when the stored icon is `""` or padded.** The fix
  belongs in `module.create` / `module.update` (`crates/relay-core/src/handlers/module.rs`): trim
  the icon and store NULL when it is empty. A client-only normalization is unsafe. The Draft's
  save sends its base as `expected`, and the engine compares that against the raw stored value,
  so editing an icon stored as `""` would be refused as an edit conflict.
- **RA-461, engine half: the large-rewrite card cannot say whether git can restore the file.**
  The client now adds "git cannot restore what is there now" only when the hold carries
  `"recoverable": false`, and otherwise says nothing about git. The engine has to add that field
  in `destructive_decision` (`crates/relay-core/src/guardrail.rs`): `false` when git was asked
  and said no, `null` when the check is off or git failed. Until it does, the clause never shows.
- **RA-459, engine half: symlinked directories.** The tree now draws a `symlink` entry as a
  folder when its target, checked on the local disk, is a directory inside the checkout. That
  assumes the client sees the engine's filesystem, as `Editor::concerns` already does. A
  `target_kind` on `file.tree` entries (relay-bus + relay-core) would remove that assumption.
- **RA-451, engine half: a detached HEAD is reported as the branch `"HEAD"`.** The client now
  treats `"HEAD"` as no branch (header "Detached HEAD", Push and PR rows disabled or hidden). An
  empty branch or a `detached` flag from `git.status` would be cleaner.

## Needs files outside this range's claims

- **RA-427: NON_UNIQUE application.** Dropping `NON_UNIQUE` changes the Wayland app_id per
  instance, but `resources/com.quietsoftware.Relay4.desktop` (StartupWMClass) and
  `scripts/install-native-desktop.py` write one fixed desktop entry, so `.stable`/`.test`
  instances would lose their dock icon. A unique app would also make `./run.sh dev` from a
  worktree raise the already-open dev window and exit instead of running the new build. Proposed:
  the base id for dev, `com.quietsoftware.Relay4.<instance>` otherwise, keep `NON_UNIQUE` when
  `RELAY_NATIVE_SOCKET` is set (smoke and perf runs), and have the install script write one
  `.desktop` per instance.
- **RA-440: Quick-create's "More fields…" drops the typed fields.** The editor can only open
  pre-filled if `task_pages::compose` takes a prefill (title, type, priority, column). The board
  side is one line once it does.
- **RA-431, styling half: `ui.toast` level.** Toasts are now attributed (`agent:<name>: `) and
  honour `ttl_ms`. Styling by `level` needs a class in `css/sessions.css`.
- **RA-432, remainder: coarse event routing.** `settings.changed` appearance reloads are now
  batched behind a 150 ms timer, so a Save reloads once. Still open: decoding the wallpaper only
  when `appearance.wallpaper` changes (`shell.rs::load_appearance`), and routing each event only
  to the pages it affects (`pages.rs`). The latter is a redesign of the refresh model.
- **RA-433, remainder.** `refresh()` now restores `registry_dirty` when `project.list` or
  `workspace.list` fails. `restored_project` is still set before its `settings.get` in
  `shell.rs::restore_layout`.
- **RA-463, remainder: a prompt already up when a panel opens.** The tray is raised above later
  overlay children whenever a prompt is shown or closed. A prompt that is already showing when a
  panel opens stays under it until the next prompt event. Raising it at that moment needs a hook
  in `panel.rs`.

## Left deliberately

- **RA-452, failed-status half.** The Git panel now clears and bumps its revision when its
  project or checkout changes, so a stale panel's keys cannot act on the new checkout. When
  `git.status` fails for the same checkout, the panel keeps its last rows. Blanking it on a
  transient error seemed worse, and the verifier notes Git actions there mostly fail anyway.
- **RA-467, remainder.** Thumbnails now skip the fetch when the tree's size is over 16 MiB, and
  skip the full-size decode when the card closed before the bytes came. The Git panel's rows
  carry no size, so it still fetches. There is no single in-flight limit and no engine-side
  downscale. The verifier judged those larger than the payoff, since results are cached per file
  stamp and only start after a 350 ms dwell.

## Side effects worth knowing

- RA-444: board letter shortcuts now ignore Caps Lock, so Shift plus a letter (Shift+C, Shift+V)
  acts like the plain letter, where before it did nothing. J/K reorder is now taken from Shift
  alone. Ctrl+Shift+Z still does not undo.
- RA-457: a save rewrites every line ending to the file's majority style (a lone CR is left
  alone), so in a mixed CRLF/LF file the minority-style lines are converted too.
- RA-458: a held editor save now asks "Allow this held action once?". The dialog shows the held
  request, cut at 2000 characters because a save carries the whole file.

## Verification note

`cargo clippy -p relay-native --all-targets -- -D warnings` fails on this branch. The failures are
not from this range: about 113 GTK 4.10 deprecation lints (`ComboBoxText`, `StyleContext`, …) in
files this pass did not change (`onboarding.rs`, `task_pages.rs`, `tools_devices.rs`, `pages.rs`,
`smoke.rs`, …, plus the untouched `commit_graph` in `code_git.rs`), and
`clippy::chunks_exact_to_as_chunks` at `sounds.rs:150`. With `-A deprecated`, the only error left
is the `sounds.rs` one. The RA-466 icon rewrite removed some of the deprecated calls.
