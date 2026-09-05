# Native roadmap implementation

The supplied chat message is the roadmap. This report records the implementation and remaining limits, rather than replacing that message with another backlog.

The earlier Relay-2 visual pass was approved. These changes implement the subsequent native-app requests, so separate Notes, project Files/Git, and the new controls intentionally change that earlier layout.

## Implemented

| Request | Native result |
| --- | --- |
| Repeated agent notifications | Repeated completion/attention reports update the existing unread card. Duplicate alerts and peer broadcasts are suppressed. A late Stop after Done cannot falsely complete the next queued task. |
| Skills library | Resizable library/detail layout, project selector, directory/subskill grouping, and project-bound enablement. Long names are ellipsized. |
| Full task page | Click a task to see/edit details, messages, subtasks, linked tasks, commits, and paginated task activity. Unsent message text blocks dismissal. Activity reads are restricted to the user. |
| Local branches and missing PRs | Searchable branch selection and local removal controls; PR lookup includes closed/merged PRs across pages. Failed/incomplete lookup is unknown. Protected, checked-out, session-owned, and unmerged branches remain protected. |
| Branch freshness | Ahead/behind counts preserve unknown states. Native agent creation performs a bounded fetch; a newer upstream base is used only when it preserves local base commits. Briefs describe divergence and fetch age. |
| Noisy completion prompt | Removed the injected “recorded for review” prompt. New assignments use durable mail and wait for a completed provider turn before PTY handoff. |
| CLI updates | Opt-in startup checks and manual update controls for known user-owned Claude native/Codex standalone installations. Live providers are skipped; unknown/package-managed installs are left to their package manager. Background updates have a timeout and bounded output. |
| Resume controls | Clear context and discard sit beside Resume. Clear keeps the worktree; discard requires confirmation and retains the branch/worktree. |
| Workspace/project dragging | Persistent ordering. Projects reorder within the same workspace and pinned group; cross-workspace movement is not implemented. |
| Project branch option | Editable base branch is explicitly labeled as the base for new agents. Existing agent branches are preserved. |
| Add agents wording | The project action becomes Add agents when sessions already exist. |
| Stale launch tasks | Initial task selection belongs only to the first profile. Project/form/generation checks reject stale submission and stale async task results. |
| Device branch overflow | Searchable, bounded checkout selector with ellipsized labels and separate full paths. |
| Before-commit summary | Visible added/modified/deleted/renamed file summary. This is deterministic, not a model-generated explanation of intent. |
| Project file tools | Shared GtkSourceView editor, search, create, rename, trash/undo, scoped drag/move, and retained unsaved edits across Agents/Editor/Git. |
| Branch dropdown | Custom searchable project/workspace/checkout/branch picker. Switching branches refuses dirty or session-owned checkouts and refuses overwriting ignored files. |
| Terminal scrolling | VTE scrollback has its own scrollbar; the outer wall scrollbar reserves space instead of overlapping terminals. |
| Notes workspace | Notes opens immediately in a separate retained window. The main Notes tab opens that window; there is no duplicate in-app Notes page. |
| Notes chrome and library | Custom borderless top bar, full-width workspace, collapsible/resizable persisted document library, close/hide/reopen preserving drafts. Main-window close still checks dirty drafts. |
| Registry menus/removal | Menus have named sections and icon actions. Removal explains the existing safeguards: no forced closing of terminals or cascading removal of projects. |
| Remove Plan | Removed navigation and special Plan behavior. Existing Plan documents remain ordinary editable notes. |
| Project Git and workspace selection | Files, Git history, staging, commits, branch tools, and workspace/project selection are available inside the project. Removed the separate Code navigation tab. |
| Better editing/file icons | Native GtkSourceView editing and diff tools are shared by projects; type-specific SVG file/folder icons replace generic rows. This does not add VS Code extensions or an LSP server. |
| Window presets | Visual preset previews, focus-plus-grid layout, existing grid/focus/review options, and saved Files/Git visibility and widths. |
| App icon | Bundled custom SVG, GTK registration, desktop entry, and launcher installation helper. Installer checked in a temporary XDG data directory. |
| Files folding | Files control toggles the real project tree; obsolete wall file tree and parent-folder arrow removed. |
| Wallpapers | Three built-in dark gradients, random rotation, staged library changes, full uncropped preview, and active selection synchronized after rotation. Empty/custom libraries are preserved. |
| Settings | Search across categories, clearer grouping, dim/content-protection controls below wallpapers, and correct fractional/boolean settings serialization. |
| Review groups | FIFO task queues and per-participant completion. All builders finish before review; the shared worktree stays on the task until all reviewers finish. Next-task mail is durable. |
| Model/default controls | Visible model/effort controls with provider-specific choices; all added profiles default to Claude. |
| Event-driven file refresh | At most one automatic tree refresh per second during bursts, no idle polling, and filesystem read events do not cause a refresh loop. Explicit user actions remain immediate. |

## Remaining decisions and limits

- Dashboard redesign/removal remains a discussion item, as requested.
- Voice/text assistant, structured agent-chat mode, provider chat-history synchronization, and the Render/mobile companion remain future product/architecture work.
- Haiku-generated commit explanations and terminal naming are not added. The commit summary uses file changes; terminal metadata uses agent intent while keeping its stable mailbox identity visible.
- The reported terminal delay was not reproduced in the small native fixture. Five warm iterations measured medians of 45.8 ms from create to visible VTE output and 39.5 ms from close to widget removal. Real repository size and real provider startup still need measurement.
- Branch deletion remains conservative. A merged/squashed PR alone does not authorize deleting a branch containing commits Git cannot prove merged.
- No real CLI update, physical Android workflow, publish operation, or user-store migration was exercised. Hookless providers receive queued assignments through mailbox without automatic PTY input. A new taskless Done request after advancement cannot be distinguished from an intended completion of the new task; this change does not introduce an assignment-token protocol.

## Verification

- Workspace suite: 195 tests passed; one documentation example intentionally ignored. Clean fixtures used `/var/tmp` because an unrelated `/tmp/.git` marker interferes with a repository-discovery test.
- Native build and strict Clippy passed, allowing the existing GTK deprecations.
- Full native smoke: 19 captures, editor/task/note saves, six paste echoes, eleven sessions surviving window close, and eleven 2,048-line VTE bursts completing inside the five-second budget.
- All ten setup/device/utility scenarios passed.
- Dedicated Notes/task, launch/Settings/Skills/wallpaper, registry drag/drop, and Files/Git interaction regressions passed. Files/Git also passed at 1024×768, including no idle refresh loop.
- Desktop launcher/icon installation passed in a temporary XDG data directory; the desktop entry validates.

Logs/screenshots are under `.impeccable/review/`.

The native fixtures use disposable engines and fake providers. Coverage includes launch scoping/defaults, Settings and wallpaper state, Notes draft/window lifecycle, task draft guards, Files/Git edits and filesystem actions, registry ordering, terminal input, and eleven simultaneous 2,048-line output bursts.

Changes are local to this worktree. The existing running app/engine was not restarted, and its sessions were not closed. The launcher builds the updated binaries; backend changes require an engine running the updated code.
