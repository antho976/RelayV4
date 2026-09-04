# P4b — GtkSourceView editor proof

Brief §13.2, ARCHITECTURE.md §6, CONTRACTS.md §2 and §7. Code: `apps/relay-native/src/editor.rs`
(`relay-native --editor <worktree-relative path>`). Everything below was observed on 2026-09-04
against the live `dev` instance as actor `user` (no token), from worktree
`/home/anthony/dev-2/Relay-V3/.relay/worktrees/proud-osprey`, project 3.

## Versions

| component | version |
|---|---|
| gtksourceview-5 (system) | 5.20.0 |
| gtk4 (system) | 4.22.4 |
| sourceview5 crate | 0.11.2, feature `v5_18` (D212) |
| gtk4 crate | 0.11.4, feature `v4_22` |
| glib crate | 0.22.9 (has no `unix_signal_add_local`; no signal handler is installed) |
| engine | `4.0.0-dev` answering on `/run/user/1000/relay/dev.sock` |

Note on the socket owner: `ss -xp` showed the listener behind `dev.sock` to be
`/home/anthony/dev/Relay-2/target/debug/relay-app` (pid 594705, up 5 h), not a `relay serve`
from this checkout. The editor does not care which process answers; recorded so nobody assumes
the numbers below come from a `relay serve` of this tree.

## Commands

```sh
# scratch files (gitignored; the only paths the proof writes to)
mkdir -p target/editor-proof
#   sample.rs      27 lines, 607 bytes: emoji, ZWJ sequence, combining marks, CJK, RTL, tabs
#   crlf.txt       CRLF line ends, no trailing newline
#   destructive.rs sample.rs + 40 lines (66 lines) for the destructive-write attempt
#   held.rs        copy of sample.rs for the hold

RUST_LOG=info cargo run -p relay-native -- --editor target/editor-proof/sample.rs            # interactive
RUST_LOG=info cargo run -p relay-native -- --editor target/editor-proof/sample.rs --self-check
RUST_LOG=info RELAY_EDITOR_SEARCH='"type"' cargo run -p relay-native -- --editor schema/bus.v1.json --self-check
RUST_LOG=info RELAY_EDITOR_SEARCH=name     cargo run -p relay-native -- --editor Cargo.lock --self-check
RUST_LOG=info cargo run -p relay-native -- --editor target/editor-proof/destructive.rs --self-check-truncate
# hold: grow the file on disk between load and save
RELAY_EDITOR_PAUSE_MS=6000 RUST_LOG=info cargo run -p relay-native -- --editor target/editor-proof/held.rs --self-check &
sleep 3; truncate -s 70M target/editor-proof/held.rs; wait
```

Environment: `RELAY_PROJECT` (default 3), `RELAY_WORKTREE` (default: current directory),
`RELAY_INSTANCE` (default `dev`). `--self-check` does load → first frame → search → read-only
toggle → edit → save → re-read → byte compare, then quits (exit 1 on any failure);
`--self-check-truncate` replaces the file with one line instead of prepending. Both only write
under `target/editor-proof/`; other paths stop after the search step. `RELAY_EDITOR_SEARCH` is
the search term (default `fn`), `RELAY_EDITOR_PAUSE_MS` a wall-clock pause before the save so
the file can be changed on disk. The deadline (90 s) and the pause are the only glib timers, and
both live on the self-check path.

Verification: `cargo check -p relay-native`, `cargo clippy -p relay-native --all-targets --
-D warnings`, `cargo fmt -p relay-native --check`, `cargo test -p relay-native` (23 tests, 5
of them in `editor.rs`: bar text for `held` / `refused` / other, byte-offset diff, payload
shapes), `git diff --check` — all clean.

## Brief §13.2, bullet by bullet

| bullet | result | evidence |
|---|---|---|
| Loading a repository-relative file | observed | `file.read {project_id: 3, worktree, path}` → `text` into `sourceview5::Buffer`; `Cargo.lock`, `schema/bus.v1.json`, `apps/relay-native/src/bus_client.rs`, files under `target/editor-proof/` all load; `../x` is refused by the engine (`invalid` / `file.path`), a missing file answers `unavailable` / `file.not_found`; both land in the bar with kind and code |
| Syntax highlighting | observed (language id), visual confirmation pending | `LanguageManager::default().guess_language(path)` gives `rust` for `.rs`, `json` for `bus.v1.json`, `toml` for `Cargo.lock`, none for `.txt`; `highlight_syntax(true)` is set. A human must confirm the colours are actually drawn |
| Line numbers | set, visual confirmation pending | `View::set_show_line_numbers(true)` |
| Search | observed | `SearchContext` over `SearchSettings {wrap_around, case-insensitive}` driven by a `gtk::SearchEntry` (`search-changed`, Enter / Ctrl+G next, Ctrl+Shift+G previous, buttons). Counts and positions in the log: `fn` → 2 matches at lines 9 and 23 in `sample.rs`, previous wraps back to 9; `"type"` → 1438 matches in `bus.v1.json`; `name` → 345 in `Cargo.lock`. Highlight of all matches is on |
| Editable and read-only modes | observed | `View::set_editable` follows a Read-only toggle; the self-check asserts `is_editable() == false` while the toggle is active. Truncated or non-UTF-8 answers force read-only, disable the toggle and Save, and explain why in the bar |
| Dirty state | observed | title shows ` *` from the buffer's `modified` flag (`modified-changed`); asserted true after the edit and false only after a successful write; a `held` answer leaves it dirty (asserted). A change typed while a save is in flight keeps the buffer dirty (edit generation counter) |
| Save through the bus | observed | Ctrl+S / Save → `file.write {project_id, worktree, path, text}`; `WriteOut {bytes, removed_lines, added_lines}` in the status line; the widget never opens a file (`/proc/self/status` is read once per self-check for the RSS number, that is the only `std::fs` call in the module) |
| Reasonable behaviour on a large file | observed for 276 KB / 13 k lines | numbers below; no optimisation was attempted |
| Correct UTF-8 handling | observed | numbers and bytes below |

Two behaviours the brief lists under "save" that the editor also proves: a `held` answer shows
the hold message and names `confirm.op` + payload in the bar without calling it; a `refused`
answer would show message and hint (unit-tested; not reachable for the user actor, see gaps).

## Numbers

Debug build, `RUST_LOG=info`, warm engine. "to buffer" = `file.read` round trip + `set_text`;
"first frame" = first tick callback on the view after `set_text`; "idle" = a `Priority::LOW`
idle after that (redraw drained); "scan" = `set_search_text` until `occurrences_count` leaves
-1; "forward()" = first synchronous `SearchContext::forward` from the start.

| file | bytes / lines | language | read RTT | set_text | to buffer | first frame | idle | RSS before → after | term | matches | scan | forward() |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| target/editor-proof/sample.rs | 607 / 27 | rust | 1.9 ms | 75 µs | 2.2 ms | 25.2 ms | 25.5 ms | 166.5 → 205.6 MB | `fn` | 2 | 168 µs | 185 µs |
| schema/bus.v1.json | 276 485 / 13 037 | json | 20.8 ms | 3.7 ms | 24.8 ms | 50.8 ms | 141.8 ms | 167.6 → 191.9 MB | `"type"` | 1438 | 12.4 ms | 218 µs |
| Cargo.lock | 88 122 / 3 672 | toml | 10.7 ms | 1.1 ms | 12.1 ms | 30.2 ms | 53.4 ms | 166.1 → 180.8 MB | `name` | 345 | 4.1 ms | 201 µs |
| target/editor-proof/crlf.txt | 52 / 3 | none | 12.0 ms | 79 µs | 12.3 ms | 18.4 ms | 18.5 ms | 165.1 → 177.1 MB | `line` | 4 | 167 µs | 175 µs |

Read RTT varies between 1.8 ms and 16 ms for the same 607-byte file across runs (the engine
runs `git worktree list` per `file.read` to validate the worktree; that subprocess is the floor,
not the client). RSS deltas include GTK's first-frame allocations (font cache, GL), not just
the buffer; the 276 KB JSON costs less RSS than the 607-byte sample because the sample's run
happened to pay more of that first-frame cost. Treat RSS as "about 170–205 MB for one window".

Save and re-read (sample.rs, prefix of 58 bytes added): `file.write` 2.1 ms, `file.read` back
0.9 ms, 665 bytes sent, 665 on disk, byte-identical. crlf.txt: 110 bytes, byte-identical, the
`\r\n` pairs and the missing trailing newline survived (`od -c` confirmed on disk).

UTF-8 content that round-tripped unchanged through `set_text` → `text()` → `file.write` →
`file.read`: `🦀 ✍️ 🇫🇷 👨‍👩‍👧` (emoji, VS16, regional indicators, ZWJ sequence), `é å ñ` as base +
combining (U+0301 / U+030A / U+0303) and precomposed, `日本語 中文 한국어`, `العربية עברית`, tabs.
The self-check compares the re-read `text` with the exact `String` the buffer produced, so the
comparison is on bytes. GtkTextBuffer reports 27 lines / 522 chars for 607 bytes, i.e. it counts
code points, not grapheme clusters; nothing was normalised.

Destructive write (`destructive.rs`, 66 lines → 1 line, `--self-check-truncate`): the engine
**allowed** it — `WriteOut {bytes: 38, removed_lines: 66, added_lines: 1}`, file on disk is 1
line / 38 bytes afterwards. Why, from `guardrail.config.get {project_id: 3}` on this instance:
`destructive_write = {min_removed_lines: 4000000, min_removed_pct: 100.0, min_file_lines: 30,
allow_if_recoverable: true}`; the percentage rule needs `> 100 %`, which is unreachable, the
line rule needs more than four million lines, and even then `target/` is gitignored so
`recoverable()` returns "ignored by git" and allows it. Not a bug in the editor; the dev
instance's policy is effectively off for this kind of write.

## Exact engine answers

`held` (comparison cap; `held.rs` grown to 73 400 320 bytes on disk between load and save):

```json
{"kind":"held","code":"guardrail.destructive_write","message":"target/editor-proof/held.rs is larger than the 64 MiB comparison cap","details":{"path":"target/editor-proof/held.rs","bytes":73400320,"reason":"comparison_cap"},"confirm":{"op":"guardrail.confirm","payload":{"hold_id":15}}}
```

Bar text: `held [guardrail.destructive_write]: target/editor-proof/held.rs is larger than the
64 MiB comparison cap` / `lift it with: guardrail.confirm {"hold_id":15}`. The buffer stayed
dirty, nothing was written (the file was still 70 MiB afterwards and was reset by hand). **Hold
15 is still open on the dev instance** (`guardrail.holds.list {project_id: 3}`); the editor never
calls `guardrail.confirm` or `guardrail.reject`, so a human must resolve it. There is no `hint`
on this error.

`refused`: not provoked. `refuse_or_user_hold` in `crates/relay-core/src/guardrail.rs` turns
every write-policy refusal into a `held` / `guardrail.user_bypass` for actors `user` and `test`;
only agents get `refused`. The bar's `refused` rendering (message + hint) is covered by a unit
test with a synthetic `BusError::refused(...).with_hint(...)`.

Other kinds seen through the editor:

- missing file → `{"kind":"unavailable","code":"file.not_found","message":"target/editor-proof/nope.rs: No such file or directory (os error 2)"}`
- `../x` → `{"kind":"invalid","code":"file.path","message":"\"../x\" must be worktree-relative and contain no .."}`

## Contract gaps and observations

1. `file.not_found` is kind `unavailable`, not `not_found` (`io_err` in
   `crates/relay-core/src/handlers/file.rs` maps every IO failure to `unavailable`). CONTRACTS.md §2
   says `not_found` means "drop the stale reference, re-query" and `unavailable` means "degrade the
   surface"; a file tree that follows §2 will do the wrong thing for a deleted file.
2. `max_bytes` truncation is byte-based and can cut a multibyte sequence: `file.read {max_bytes:
   60}` on `sample.rs` returned `bytes_b64` (not `text`) because byte 60 fell inside an emoji. A
   client that reads a large UTF-8 file with a cap sees it flip to "binary". Either truncate on a
   char boundary or document that `truncated: true` may come with `bytes_b64`.
3. `truncated` answers cannot be saved safely; the editor forces read-only. The contract does not
   say that a `file.write` after a truncated `file.read` replaces the whole file, but it does
   (the write is the full text). Worth a sentence in CONTRACTS.md §7.
4. `mime` is a small extension table: `.txt` and `Cargo.lock` come back `application/octet-stream`
   while their `text` is populated. Clients must key on `text.is_some()`, not on `mime`.
5. `refused` is unreachable for the user actor on the write path (see above); CONTRACTS.md §2's
   "refused: show message and hint; no retry" has no user-side producer today.
6. `held` carries no `hint`; the bar shows the `confirm` op instead. Fine for the GUI, but §2's
   table implies a hint is normally there.
7. The dev instance's destructive-write thresholds (4 000 000 lines / 100 %) make the policy
   inert; the P4b proof of a "destructive-write hold" therefore relied on the 64 MiB comparison
   cap. Whoever owns the dev store should decide whether those numbers are intentional.

## Risks

- **Two early runs hung in `file.read`** (01:11:46 and 01:13:54): `bus connected` logged, then
  no response for 90 s / 40 s, no protocol notice, no disconnect. A CLI `file.read` at 01:13:3x
  between them answered in 4 ms; every run from 01:27 on answered in 2–16 ms. Not reproduced,
  cause unknown; candidates are the per-connection `spawn_blocking(dispatch)` in the socket door
  being starved while another agent's op ran, or the `git worktree list` subprocess in
  `root_verify` blocking. The client has no request timeout by design (CONTRACTS.md §9); the
  product editor needs a visible "waiting on the engine" state rather than a timer.
- The proof uses `Buffer::set_text` / `text()`, not `FileLoader` / `FileSaver`, so
  `implicit-trailing-newline` never applies and the round trip is exact. If a later slice adopts
  the loader (for encoding detection or async loading of multi-MB files) it must keep this
  byte-exactness, or re-measure it.
- Numbers are from a debug build with the window unfocused on a busy machine; the 141 ms "idle"
  on the 13 k-line JSON is dominated by the syntax highlighter's first pass and would move with
  a release build.
- The socket path helper is duplicated (`app.rs` has one, `editor.rs` has one) because this
  slice may not edit `app.rs`; fold them in Stage 1.

## What a human must confirm visually

Run `RUST_LOG=info cargo run -p relay-native -- --editor target/editor-proof/sample.rs` and check:

1. Rust keywords, strings and comments are coloured (highlighting is set, not seen by the check).
2. A line-number gutter is drawn and the current line is tinted.
3. Typing adds ` *` to the title; Ctrl+S removes it and the status line shows the `WriteOut`.
4. Ctrl+F focuses the search entry; typing `fn` highlights both occurrences after a short delay;
   Enter / Next / Previous move the selection and the counter reads `match 1 of 2`, `2 of 2`,
   then `1 of 2 (wrapped)`.
5. The Read-only toggle blocks typing and adds ` [read-only]` to the title.
6. Open `schema/bus.v1.json`: scrolling and searching feel immediate at 13 k lines.
7. The bar: open `target/editor-proof/nope.rs` to see `unavailable [file.not_found]`; the
   `held` bar text is in the log above (reproducing it needs the 70 MiB trick).

## Next smallest slice

Subscribe to `file.changed` for the open path and re-query `file.read` when it fires (with a
"file changed on disk" bar when the buffer is dirty). That is the one bus behaviour a real
editor needs that this proof skipped, and it exercises `bus.subscribe` through the same client.
