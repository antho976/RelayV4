# Desktop app audit: game-engine plugins, MCP servers, performance — 2026-09-22

This is a read-only audit of `apps/relay-native` and of the engine paths it depends on when
the work is mostly the Unreal and Blender plugins and MCP servers. **Nothing was built or run.**
`relay-native` cannot be compiled on this image (see `CLAUDE.md`). Every entry below comes from
reading the code. Entries marked ✔ were traced line by line against the source by a second
reader. Entries marked ~ are likely but depend on behaviour outside this repository.

Severity: **blocker** stops the work, **major** costs real time or data, **minor** is an irritant.

---

## A. Plugin and MCP bugs

### A1. Large Unreal tool results fail with "the editor script printed no result" — major ✔
- **Where:** `crates/relay-cli/src/unreal.rs:1351` (`python()` → `tail(.., 2000)`), `tail` at `:1263`, `python_json` at `:1361`.
- **Mechanism:** every editor script returns its result as one line, `RELAY_JSON:<json>`. `tail` keeps
  the last `MAX_OUTPUT` (60 000) bytes by cutting from the *front*. When the JSON line is longer than
  about 60 KB, the cut removes the `RELAY_JSON:` prefix and `python_json` finds no result.
- **What triggers it:**
  - `ue_data_table action=export` on any table over about 60 KB (dialogue and loot tables);
  - `ue_asset_audit` at its default of 400 assets per class;
  - `ue_blueprint_info` on a folder;
  - `ue_asset_refs` at depth 4;
  - `ue_level_actors` or `ue_search_assets` with a limit above roughly 350.
- **Fix:** find the `RELAY_JSON:` line in the untrimmed output first, then trim only what is shown to the
  agent. Or cap the result line on the Python side and say so in the result.

### A2. Relay's hook path switches off Git LFS and every other hook except pre-commit — major ✔
- **Where:** `crates/relay-core/src/hooks.rs:59-146`.
- **Mechanism:** `install_git` sets `core.hooksPath` for the worktree to `.relay/hooks/<session>/` and writes
  only `pre-commit` there. The previous `pre-commit` is chained. `pre-push`, `post-checkout`,
  `post-commit`, `post-merge` and `commit-msg` are not.
- **Effect:** Git LFS installs exactly those hooks, and almost every Unreal repository uses LFS. A push
  from an agent's checkout skips `git lfs pre-push`. So does a push from the primary checkout, where
  the Unreal plugin puts agents by default. The remote receives pointers with no objects, and
  teammates' clones and CI fail.
- **Fix:** for every hook name git knows, write a small forwarder in the Relay hook directory that `exec`s
  the previous hook when it exists. `pre-commit` keeps the guardrail step.

### A3. Closing one of several sessions in the primary checkout switches the pre-commit gate off for the rest — major ✔
- **Where:** `crates/relay-core/src/handlers/session.rs:1374-1398`.
- **Mechanism:** when other sessions still share the worktree, `uninstall_git_chain` is skipped.
  `remove_hook_dir(repo, &s.name)` still runs unconditionally. If the closing session was the last one
  launched or resumed, which is the usual case, it owns the current `core.hooksPath`. That path now
  points at a deleted directory.
- **Effect:** git treats a missing hooks directory as having no hooks. The guardrail gate and the user's
  own pre-commit stop running until some session relaunches. The comment on `remove_hook_dir`
  (`hooks.rs:256`) assumes the opposite.
- **Fix:** skip `remove_hook_dir` while other sessions are live, or point `core.hooksPath` at a surviving
  session's directory first.

### A4. The primary checkout is never excluded from git — major ✔
- **Where:** `worktree::ensure_excluded` (`worktree.rs:39`) is called only from `worktree::create` (`:199`).
  The primary-checkout path in `session.create` never calls it.
- **Effect:** in a project that has only used primary-checkout agents (the Unreal plugin's default), `git
  status` shows `.relay/`, `.claude/settings.local.json` and `.codex/hooks.json` as untracked. `.relay/`
  holds hooks, `relay.mcp.json` and `sessions/<name>/role-instructions.md`, which contains the brief
  and the launch prompt. A `git add -A` by the user or an agent commits them, along with absolute paths
  to the relay binary.
- **Fix:** call `ensure_excluded(repo)` in `launch` (or when a project is added), not only when a worktree
  is created.

### A5. Codex sessions: plugin tools time out at 60 s and miss the engine's environment — major ~
- **Where:** `crates/relay-core/src/providers.rs:97-119` (`codex_mcp_config`).
- **Mechanism:** only `command`, `args` and `env` are set.
- **Timeout:** without `tool_timeout_sec`, Codex gives up on a tool call after 60 s. The calls that take
  longer are `ue_build` (up to 3600 s), `ue_run_tests`, `ue_play`, `ue_asset_audit`,
  `blender_render` and `blender_export`. The server is single-threaded and keeps working on the
  abandoned call, so every later call queues behind it.
- **Environment:** Codex passes stdio MCP servers only a short allowlist of environment variables unless
  `env_vars` is set. So `UE_ROOT`, `UE_PROJECT`, `UE_REMOTE_CONTROL_URL`/`_PASSPHRASE`, `BLENDER_BIN`,
  `RELAY_WORKTREE` and `RELAY_SESSION` do not arrive. The result is "engine not found", or 401 from a
  passphrase-protected Remote Control.
- **Claude sessions are not affected.** Both Codex defaults come from Codex's documentation, not from
  its source. Check them against the Codex version you run.
- **Fix:** emit `mcp_servers.<name>.tool_timeout_sec` (per plugin, e.g. from `plugin.json`) and
  `mcp_servers.<name>.env_vars=[…]`.

### A6. No way to add your own MCP server — gap, not a bug ✔
- Plugins are compiled in from `plugins/*/plugin.json` (`plugins.rs:28`). `plugin.enable` only switches
  bundled ids.
- Relay's `--mcp-config` is added to Claude's own configuration, not substituted for it. No
  `--strict-mcp-config` is passed. So servers from `claude mcp add` and from a project's committed
  `.mcp.json` still load.
- **Catch:**
  - Claude's default "local" scope is keyed by directory path.
  - An *untracked* `.mcp.json` is not copied into new worktrees.
  - So a custom server you added in the primary checkout is missing in every pooled `.relay/worktrees/<name>` agent.
- **Workarounds today:** `claude mcp add --scope user`, or commit `.mcp.json`.
- **Fix:** a per-project "extra MCP servers" setting, written into `relay.mcp.json` next to the plugin
  servers, and passed as `--config` to Codex.

### A7. Smaller plugin issues ✔
- **Editor lock race** (`unreal.rs:483-500`): `acquire_lock` reads and then writes, with no atomic create.
  Two agents calling at the same moment can both hold the lock.
- **The lock can't be released while the editor is down:** `ue_editor_lock` is in `LIVE` (`unreal.rs:33`),
  so it needs a reachable editor. After a crash the agent can't check or release the lock and has to
  wait out the 15-minute idle expiry.
- **Misleading close refusal** (`session.rs:1352`): with `remove_worktree` omitted (default `true`), closing
  any session that shares the primary checkout is refused with "close the PAIR partner first", although
  nothing is paired and nothing would be removed. The desktop app passes `false`; CLI and MCP callers
  hit the refusal.
- **Every editor Python call opens an undo transaction** (`generateTransaction: true`), read-only
  queries included. This fills the editor's undo history.
- **`blender_to_unreal` finds the `.uproject` from the Blender checkout.** If the art repo and the game repo
  are separate Relay projects, it fails unless `UE_PROJECT` is set.
- **Checked and fine:**
  - MCP framing in `mcp.rs`, `unreal.rs` and `blender.rs`: notifications get no reply, parse errors return
    -32700, and nothing else writes to stdout.
  - Subprocess timeouts: builds, tests and Blender all go through `output_with_timeout`.
  - Resume gets plugin servers.
  - Codex TOML quoting is correct.
  - Two sessions in one directory do not race on `relay.mcp.json`.

## B. Desktop client bugs

### B1. One large reply disconnects the whole app — blocker ✔
- **Where:** `apps/relay-native/src/client.rs:48,183-188`.
- **Mechanism:** the main connection treats any frame over 2 MiB as a protocol error, and the app then
  shows "Engine disconnected". Reconnecting is manual, and the view that made the request asks for
  the same thing again.
- **Replies that can exceed 2 MiB:**
  - `git.diff.file` (`handlers/git.rs:235`) returns the whole HEAD blob, the whole working file
    (`from_utf8_lossy`, `:765`) and a unified diff, with no size cap. The client's 1 MiB check
    (`code_git.rs:842`) runs only *after* the reply arrives. Clicking a modified `.uasset` or `.umap` in
    Changes drops the connection. A text file of about 700 KB is already enough.
  - `git.status` with `--untracked-files=all` on a first commit of an Unreal project, or a repo with no
    `.gitignore` (about 15 000 paths). `prepare_project` calls it again after every reconnect, so this
    one loops.
  - `file.search`, where one hit inside a single-line `.gltf` with embedded buffers, a minified JSON
    file or a source map carries the whole line.
- **Fix, both sides:**
  - Engine: refuse or truncate binary and oversized files in `git.diff.file` (NUL check and a size cap,
    as `file.read` already does), cap `git.status` entries, and clip search-hit lines.
  - Client: fail the one request instead of the connection when a frame is too large.

### B2. Every agent state change rebuilds the Notes window and takes focus from the note you're typing in — major ✔
- **Where:** `app.rs:1191` calls `refresh_notes` from `refresh()`. `refresh()` runs for every `session.*` event
  from every project (`app.rs:988-990`).
- **Mechanism:** the engine emits `session.changed` every time an agent goes idle ↔ running ↔ blocked.
  Each time, `notes.list` (full bodies) runs and `note_pages::workspace` (`note_pages.rs:471`) clears the
  page and detaches `note_tabs` from its parent.
- **Effect:** with six agents working, focus leaves the note editor, search text clears and the rail
  scrolls to the top, several times a minute. This also happens while the window is hidden.
- **Fix:** don't call `refresh_notes` from `refresh()`, since the `notes.changed` events already cover it.
  Rebuild only the rail, and never reparent `note_tabs`.

### B3. The session menu shows stale settings and can undo your previous change — major ✔
- **Where:** `shell.rs:1061-1065` captures a snapshot of the session in the menu's click handler. It is
  refreshed only when the render signature changes (`app.rs:1241`).
- **Mechanism:** the signature has no `bus_writes`, `allow_ui`, `model`, `effort` or `spawned_at`. Save sends
  both check boxes (`shell.rs:1151`).
- **Scenario:**
  1. Turn on *Allow UI control* and save.
  2. Reopen the menu. It shows the box off.
  3. Turn on *Allow agent bus writes* and save.
  4. `allow_ui` is written back as `false`, with no warning.
- This matters directly for MCP work, because those two switches decide what an agent's Relay MCP
  tools may do.
- **Fix:** add those fields to the signature, or have the menu read the current row from `ui.sessions`
  when it opens.

### B4. A terminal pane goes dead after one attach error and doesn't recover on its own — minor ✔
- **Where:** `terminal.rs:409-420`.
- **Mechanism:** on `Err`, the stream loop shows the error and stops (`break`), but `active` stays `true`.
  The next `set_active(true)` then does nothing (`:393`).
- **Effect:** the pane stays frozen until the page or layout changes, the window is minimised, or you
  click Reconnect.
- **Triggers:** an attach that times out behind a slow locked write (see C1), or `session.not_spawned`
  while the PTY is being replaced.
- **Fix:** reset `active` to `false` before the `break`, or retry with backoff.

### B5. File-watcher events aren't tagged with a project, so they refresh the current project — major ✔
- **Where:** `engine.rs:564` (`emit_system` sets no `project_id`), `watch.rs:113-120`, client filter at
  `app.rs:978`.
- **Effect:** a write in *any* watched worktree, including another project's or an agent's, makes the Code
  view reload:
  - the tree, `git.status`, branches, log, `integration.list` and `pr.list`;
  - the search, which it runs again.
- With unsaved edits it also shows "Files changed on disk…" for files that have nothing to do with the
  open one (`editor.rs:492`). With agents compiling or cooking, this happens continuously.
- **Fix:** set `project_id` on watcher events and compare the event's path with the open files.

### B6. The file tree hides folders that search can still reach — minor ✔
- **Where:** `file.rs:517-520` via `watch.rs:170-187`.
- **Mechanism:** `list_dir` hides any path component named `Saved`, `Binaries`, `Intermediate`,
  `DerivedDataCache`, `build` or `dist`.
- **Effect:** `Saved/Logs` (crash logs) and `Saved/Config` can't be browsed from the tree, yet search
  returns hits inside them. For Unreal work, `Saved/Logs` is the folder you most often want.

### B7. Page state problems — minor ✔
- **Skills page:** the project picker re-renders from a snapshot (`tools_skills.rs:160-176`). A
  `skill.changed` event for a project that isn't current is dropped by the client's project filter, so
  switching back shows the old switch state. `plugin.enable` or `skill.enable` for another project has
  the same problem.
- **Plugins page:** it is rebuilt by any event that triggers `refresh_page` (task, mailbox, run,
  `usage.changed`; `tools_plugins.rs:24-41`). Open expanders collapse, and the scroll position resets
  mid-read.
- **Settings "Save":** it sends `settings.set` for every control, changed or not, from a page built once
  per project. So it overwrites values changed by the CLI or the phone in the meantime, and stops
  partway if one call fails (`tools_settings.rs:118-160, 596-656`).
- **Reconnect button:** it appears next to plain information messages, because toasts and
  provider-update progress use `show_error` (`app.rs:765`).
- **Engine restart:** after a restart nothing reconnects automatically. When you reconnect, every pane is
  rebuilt and attaches with no sequence number, so it gets back only its last 256 KiB instead of its
  full scrollback (`app.rs:909`).

## C. Performance not covered by the 2026-09-19 baseline

The baseline measured the engine on a 1 500-file repository and has no client numbers (§5, §6). The
items below are what an Unreal-sized tree and a busy client add.

### C1. Slow work done while holding the store lock (breaks the `CLAUDE.md` invariant)

These freeze the whole bus, keystrokes included, for as long as they run.

| op | registration | what runs while the lock is held | file |
|---|---|---|---|
| `session.close` of a pooled worktree | `register` | `dir_size` over the whole worktree, `purge_build`, `git worktree remove --force` (an rm -rf of the checkout) | `session.rs:1391`, `worktree.rs:352` |
| `session.spawn` / `session.resume` | `register` | `install_git`: 3–5 `git` subprocesses through `Command::output`, not `proc::output_with_timeout`; both skill materializations | `session.rs:354-383`, `hooks.rs:553` |
| `file.rename` / `file.move` / `file.delete` | `register` | `guard_path_mutation` reads the whole file with `read_to_string` (a 3 GB `.umap` is read in full, then fails UTF-8) | `file.rs:578` |
| `file.import` | `register` | copies whole directory trees | `file.rs:262, 321-338` |
| `workspace.discover` | `register` | 4-level directory walk; onboarding runs it automatically. One unreadable directory fails the whole walk | `workspace.rs:115` |

Details:

- **`session.close`:** an Unreal agent worktree with `Intermediate/`, `Binaries/` and `DerivedDataCache/` is
  tens of GB. The close can hold the lock for seconds to minutes. `BUILD_DIRS` (`worktree.rs:12`)
  doesn't list any Unreal folder either, so the "freed" figure lumps everything into the worktree size.
  Move the size walk and the removal into an after-commit or staged phase.
- **`session.spawn` / `resume`:** a git that blocks on a credential helper or `index.lock` freezes the bus
  with no deadline, and the close path has the same problem. Route `hooks::git` through
  `proc::output_with_timeout`. Move `launch`'s file and git preparation into `register_staged`.
- **`file.rename` / `move` / `delete`:** skip the content read for files over a small cap, or read only a
  prefix.

### C2. `file.search` reads the whole project again on every file change
- **Where:** `file.rs:362-417, 629`.
- **Mechanism:**
  - `skip_dir` skips only `.git`, `.relay`, `node_modules` and `target`. It skips no Unreal folder and
    doesn't read `.gitignore`.
  - Every file is read whole into a shared buffer, which grows to the size of the largest file (a
    `.pak`, or a multi-GB `.umap`) and stays that size.
  - `editor.rs:483` re-runs the search 1 s after every `file.changed` while the search box has text. With
    B5, that means any change anywhere.
  - One unreadable subdirectory aborts the whole search.
- **Fix:** respect `.gitignore` (the `ignore` crate), skip files over a size cap or containing NUL bytes, and
  don't re-run the search on unrelated events.

### C3. Every `file.tree` call runs a full `git status`
- **Where:** `file.rs:30-33`, `editor.rs:741-758`.
- **Mechanism:** each folder expand runs `git status --untracked-files=all` (10 s timeout). After an
  invalidate, the rebuilt tree re-expands K folders, which is K+1 concurrent status runs plus the one from
  `refresh_git`.
- **Effect:** on an LFS repository these contend for `index.lock`. A timeout falls back to
  `unwrap_or_default()`, so badges disappear with no error shown.
- The baseline lists memoizing this as "the next item". On an Unreal tree it matters much more than the
  10 ms it measured.

### C4. The recursive inotify watch has no limits and restarts from scratch after a failure ~
- **Where:** `watch.rs:128-146`.
- **Mechanism:** `RecursiveMode::Recursive` places a watch on every directory, `Intermediate/`,
  `DerivedDataCache/`, `.git/objects/` and `Content/` included, for every worktree the tree touches.
  `is_generated_path` filters only events, not watches. The `watchers` map is never pruned.
- **Effect:** on `ENOSPC` the watcher is dropped and the next `file.tree` walks the whole tree again. In the
  meantime it can use up `max_user_watches`, and Unreal Editor or your IDE then report "file watcher
  limit reached".
- **Fix:** watch non-recursively on the directories the tree has expanded, or filter directories before
  adding watches.

### C5. Committing reads and line-diffs every staged file in full
- **Where:** `git.rs:513, 781-793` (`staged_numstat`).
- **Mechanism:** for every staged file it reopens the repository and index, loads both blobs, and runs a line
  diff.
- **Effect:** committing a batch of binary (non-LFS) assets is slow and uses a lot of memory. It runs
  unlocked, so the bus stays responsive.
- **Fix:** skip binary blobs (a NUL check), as `git diff --numstat` does.

### C6. Unreal plugin log reads
- **Where:** `unreal.rs:766-776`.
- **Effect:** `ue_run_tests` in-editor re-reads the entire editor log once a second, and `ue_log` reads the
  whole file. Editor logs reach hundreds of MB.
- **Fix:** remember the byte offset and seek past it.

### C7. Client-side costs
- **Status bar:** `refresh_status` (`status.rs:71`) fires on every `usage.changed`, `device.changed` and
  `run.changed`, with no coalescing. Each run sends `usage.get` (a provider JSONL tail read), `device.list`
  (a new `adb` process) and `provider.list`, then rebuilds the meters. Replies can arrive out of order.
  It should use the same pending/dirty pattern as `refresh`.
- **Project sidebar:** it is rebuilt (clear, then recreate every row, menu and drag source) on every agent
  state change, twice per refresh (`app.rs:1162, 1184`). The signature includes session state, and it
  serializes every project to JSON to build that signature. An open project menu disappears.
- **Terminal input:** keystrokes share a socket with output (`client.rs:219-242`). A chatty pane that fills
  the 64-notice queue delays the replies to its own keystrokes. Typing into a busy agent lags.
- **Appearance reload:** every appearance change or wallpaper rotation sends six `settings.get` calls,
  fetches the whole wallpaper library again (up to 1.5 MB), and decodes the JPEG on the GTK thread
  (`shell.rs:1610-1730`). Then every pane is restyled and resized.

## D. Suggested order

1. **A2** (LFS hooks), **A3** (lost pre-commit gate) and **A4** (unexcluded `.relay/`): each silently damages
   the repository.
2. **B1** (disconnect on large replies): stops the app the first time you click a changed asset.
3. **C1** (locked slow work, especially `session.close` and `file.rename` on large assets) and **B5** with
   **C2** (project-less watcher events driving whole-tree searches).
4. **A1** (Unreal result truncation) and **A5** (Codex timeout and environment).
5. **B2** and **B3** (notes focus theft, stale session menu), then the rest.

Any change under `apps/relay-native/` has to be compiled and smoke-tested on a machine with GTK 4.22;
none of it can be verified here.
