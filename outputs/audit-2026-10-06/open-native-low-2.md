# Audit findings left open: native client Lows, second half

Scope: RA-481..532, RA-554..560, RA-743/744 (apps/relay-native, CSS and resources), on branch
`relay/sly-seal` (2026-10-07). Everything in that range is fixed except the parts below. Each item
says why it is open and what closing it would take.

None of the fixes has been run in the app. `cargo check`, `cargo clippy --all-targets -- -D
warnings -A deprecated` and `cargo test` for `relay-native` pass on this machine (GTK 4.22). The
redraws, scrolling, focus and keyboard behaviour have not been observed on screen.

## Not started: the files belong to `sly-koala`

These findings are in files `sly-koala` claimed for RA-425..480. They were left for that session,
or for after it releases the files, rather than edited in parallel.

- **RA-554** `app.rs`: on connect, `device.watch` is awaited before the notice loop starts
  draining.
- **RA-555, RA-556** `guardrail_pages.rs`: approval prompts show agent-chosen text without
  neutralising it, and the approval keys are live as soon as a prompt slides in under the pointer.
- **RA-557** `guardrail_settings.rs`: Reset and Save stay live while another scope's layers load.
- **RA-558** `launch.rs`: an agent's starting task is the first ticked card in board-column order.
- **RA-743** `board_view.rs`: the board's pure helpers (task_matches, haystack, group_of,
  drop_index, zone) should move to a GTK-free module that a default member compiles, so CI runs
  their tests. Add cases for grouped and filtered reorders.

## Partly fixed: the rest needs a file another session holds

- **RA-488** (`note_pages.rs`, sly-koala): the Notes window now says there is no project. Ctrl+N
  and File > New note still return silently from `new_note` when the project is 0, and need a
  message there.
- **RA-499** (`editor.rs`, sly-koala): the drag-move code moved to `editor.rs`
  (`bind_folder_drop`). It should use `set_locked(true/false)` instead of raw `busy`, and check
  `ed.matches(..) && !ed.buffer.is_modified()` before `follow_change`.
- **RA-511** (`app.rs`, sly-koala): the device count now reads on its own connection, coalesced,
  with one retry after `device.adb_timeout`. Still open: in the `"usage.changed" |
  "device.changed" | "run.changed"` event arm, each event should refresh only what it changed.
- **RA-517** (`note_pages.rs`, sly-koala): `task_pages::action_then` takes a reopen callback.
  `module_detail`'s "Add task" should pass `Some(Rc::new(move |ui| open_module(ui, id)))` so the
  module editor reopens instead of closing.
- **RA-529** (`app.rs`, sly-koala): a rotation tick no longer fetches the library twice, an idle
  rotation skips the library read, and the signature no longer serialises the library. Still
  open: `app.rs` should refresh rotation only for the wallpaper keys. (The `shell.rs` half,
  `load_appearance` calling `wallpaper_rotation::refresh`, was taken by spry-zebra.)
- **RA-530** (`note_pages.rs`, sly-koala): `resources/relay-editor.xml` now overrides every
  language-specific style that Adwaita-dark sets. `NOTES_SCHEME` in `note_pages.rs` has the same
  parent and needs the same rust/c/css/xml/diff/python lines, plus def:constant, def:note and
  def:net-address.

## Partly fixed: the rest needs an engine op

- **RA-490 / RA-507**: the bell coalesces its loads, and the popover's summary and Mark all read
  use the bell's unread count. Both still fetch up to 100 rows, because `notify.list` has no
  count-only form.
- **RA-495**: the board's `task.list` and `module.list` now run together. Folding `module_name`
  into `task.list` would remove the second call, but that is an engine change.
- **RA-502**: a sidebar drag now updates only the items whose order changed. A single
  transactional reorder op (one undo entry, one event) does not exist in the engine.

## Handed to spry-zebra

- **RA-525** (`shell.rs`): plugin errors are matched on bus error codes
  (`tools_plugins::explain_error`). spry-zebra is moving the last string-matching caller in
  `shell.rs` over and deleting `explain`. The outcome goes in spry-zebra's open list.

## Doc drift noticed

- **RA-491**: following a guardrail notification now opens the agents wall (D121). Notifications
  without a link still show an "Open" affordance, which D121's last sentence says they should not.
  Either the code or `docs/engine/DECISIONS.md` should change.
