# Relay-2 native parity coverage

Reference: Relay-2 `1745dd3f68b7bc786f48142ad7da591537aeb057`.
Native stack: Rust, GTK4, VTE and GtkSourceView, matching Relay-V3.
The archived V3 roadmap remains unchanged; its completion marks describe V3.

## Implemented native workflows

| Relay-2 surface | Native implementation |
| --- | --- |
| Shell and project registry | Compact title bar, sidebar navigation, grouped workspaces, project selection, registration, rename, pinning, base/build/run settings and confirmed registry removal |
| Terminal wall | Retained VTE widgets; Grid, Focus and Review modes; resizable columns; ordering; terminal satellites; file rail; named layout save/apply/delete |
| Launch and review groups | 1–11 solo profiles or one/two builders sharing a reviewer; provider/model/effort, worktree, assignment, write/UI rights and task queue controls; allocations and queues precede spawning; reviewer starts first |
| Session lifecycle | Start, park, wake, resume, fresh restart, brief inspection, pre-spawn configuration, confirmed close and optional worktree/build cleanup |
| Coordination and safety | Existing engine role boundaries, claims, mailbox priority, review routing, task approval and exact held-operation inspection; agent permission and human task approval remain distinct |
| Board | Five columns, search/type/priority filtering, drag moves, task editor, metadata, labels, subtasks, dependencies, duplicates, changelog, commit links, attachments, dispatch and approval |
| Modules, Plan and Notes | Module editing/archive, changelog generation, Plan note, searchable note list, persistent editor tabs, formatting, find/replace, dirty-close protection and note satellites |
| Code and Git | Primary/agent checkout selection, expandable tree, content search, file creation/rename/trash/restore, highlighted editing, find/replace, dirty protection, worktree diff, staging, commits, history, branch management, fetch/push/PR and disposable merge tests |
| Dashboard and notifications | Cross-project overview, correct project/task/session destinations, read acknowledgements, category settings, generated audio patterns, volume and preview |
| Skills | Local editing, GitHub installation and replacement, source refresh, per-project enablement and deletion |
| Settings and usage | Relay-2 palettes and SVG geometry, wallpaper library, opacity/dimming/contrast, terminal font, configurable Relay-2 shortcuts, provider/device paths, parking, guardrail configuration, backups and provider usage windows |
| Android | Device/AVD listing, boot/create, selected-worktree run/build, APK/AAB, optional publishing, signing configuration, bounded logs and native mirror/control/capture |
| Plugins | Reserved extension surface, as in the reference |

## Reliability changes

- Task, note and module updates accept optional expected editable fields, checked
  in the write transaction. Stale drafts are rejected without changing the row.
- File writes accept the expected original SHA-256, checked inside the engine
  handler. This closes the competing bus-writer gap; unrelated external filesystem
  writers cannot be made atomic by this API.
- Session allocation persists assignments before spawning, so partially launched
  groups can recover from the wall without losing their prompt.
- Native mirror connections own their capture lifetime. Input queue overflow
  stops capture rather than silently losing a touch release.
- Device watchers are leased per socket. Duplicate requests retain all schema and
  actor validation; disconnect releases only that connection's watcher.
- Hidden note windows are destroyed on closure. Drafts remain explicit and
  project-qualified when navigating between projects.

## Validation and remaining acceptance

[VERIFICATION.md](VERIFICATION.md) records the checks and their limits. Core
workflows have native implementations; this is not a claim of pixel identity,
release readiness or physical-device signoff.

Remaining acceptance covers real Claude/Codex TUI sessions, physical Android
build/signing/mirror behavior, external publishing, notification audio on the
user's sound setup, accessibility and prolonged performance. No real provider,
release upload, signing identity or user-store migration was used for smoke tests.

The reference's first-run wizard is adapted to direct repository registration
and Settings. Git history uses native rows rather than the reference's graph
presentation. Advanced docking trees, packaging, migration and subsequent
product features stay in the imported roadmap.
