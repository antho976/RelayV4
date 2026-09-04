# Backlog, classified by owner

Brief §17 restated as a working list, plus Antho's "Relay V3 features and overhaul" note. The
intent behind each line is preserved (brief §18); wording is condensed. Ordering inside each
group is the suggested order, not a commitment.

## A. Relay 2 now, when it produces reusable truth

Made in `~/dev/Relay-2`, verified there with bus-driven tests, then carried into this repository's
engine copy (D203). Only when the fix changes Rust, bus, data or safety behaviour; no Svelte polish.

1. Deduplicate completion notifications: one logical completion, one notification; lifecycle
   reports, provider hooks, task transitions and reconnect replay collapse to one durable identity.
2. Merged-PR truth and safe local branch removal: authoritative PR state, uncertainty exposed.
3. Branch ahead/behind counts with exact base and last-refresh time, included in `session.bootstrap`.
4. Project removal never forces terminal, worktree, branch or repository destruction; destructive
   scope is explicit.
5. Review-group coordination where the backend contract is wrong.
6. File refresh watcher-driven and burst-coalesced; no high-frequency polling.
7. Stale task/session copy that conflicts with `session.bootstrap`.
8. Terminal creation and removal latency measured; core-side delay fixed if present.
9. Provider and model selection in the session bus contract, if missing.
10. Update policy and safety for Codex and Claude Code before any automatic mutation.
11. Relay 2 performance baseline (`docs/BASELINE-RELAY2.md`, protocol in VERIFICATION §3).
12. The failing relay-core unit test `suggested_workspace_keeps_a_non_repository_directory`
    (READINESS): decide whether it is a defect in `suggested_workspace_from` or an
    environment-dependent test, and fix whichever it is.
13. The native feasibility spike itself (this repository, `docs/ROADMAP.md`).

## B. Astra: implementation context for Relay 3

Broad architectural work; belongs in this repository after the proceed gate.

- Native GTK application shell · VTE terminal wall and lifecycle · GtkSourceView editor
- Native Code tab with files, editor, and output together on one page
- Native multi-window Notes
- Task detail, history, links, subtasks, messages
- Branch and workspace navigation across the project surface
- Durable daemon (`relayd`) or engine lifetime independent of the UI, if the spike proves it
- Voice and text assistant control surface · GUI-style agent conversation mode
- Mobile and remote companion architecture
- Provider CLI update system
- Full migration, packaging, state migration, rollback plan

## C. Claude: UI and UX specification

Designed against real GTK, VTE and GtkSourceView capabilities discovered by the spike, never as
browser interactions translated afterwards.

- Skill library hierarchy, subskills, sizing, project switching
- Condensed task board and full task-detail view
- Placement of resume, clear context, discard
- Draggable workspace and project navigation; project and workspace menu hierarchy
- Editable project branch control (after Antho's decision below); branch selector styling
- "New session" → "Add agents" where appropriate; no previous-task leak into new-project flow
- Device build selector overflow
- File sidebar parity, drag behaviour, controls, native context menus
- Terminal and build-log scroll ownership
- Notes window layout, header, sidebar collapse, resizing
- Code-page file and output surfaces, with Git behind one top-right button beside Skills
- Dashboard redesign or evidence-based removal
- Editor visual design and code-writing ergonomics
- File and folder icon system · application icon · window presets
- Wallpaper rotation, preview, dim, content projection
- Settings hierarchy and search (including the tabs from Antho's note: Agents, General,
  Terminal, Notifications, Stats & usage)
- Model-selection presentation · session and terminal naming presentation
- Status line with resource manager and plan usage

## D. Antho decides (product choices; agents report `blocked`, they do not settle these)

| question | why it blocks |
|---|---|
| Does the dashboard have a job, or is it removed? | Stage 6 scope; Claude's design input |
| Does Plan stay removed? | Stage 4 IA |
| Is Notes only a satellite window? | Stage 5 entry |
| "Editable project branch": default branch, browsing branch, new-agent base, or all three as separate concepts? | Stage 3 bus contract |
| What does Agent mode promise about continuity with the official Claude and Codex apps? | Stage 8 |
| Does the mobile companion need the desktop online, or a hosted durable service? | Stage 8 architecture |
| How much automatic provider updating is acceptable? | A.10 and Stage 8 |
| When does Relay 3 become the daily driver, and when may Relay 2 retire? | Stage 9 |
| D203: engine import by plain copy at `1745dd3`? | Phase 0 exit |
| Install `vte4` and `gtksourceview5` now? | Phase 0 exit |

Decided 2026-09-04: the Code tab stays. Its multiview keeps files, editor, and output on the same
page; Git opens from one top-right button beside Skills.

## E. Non-goals for the first native milestone (brief §19)

Mobile client · hosted service · voice recognition · general autonomous assistant · cross-provider
chat sync · automatic provider self-updates · rotating wallpapers · elaborate settings redesign ·
final dashboard · every historical feature. The first milestone is the native shell, the Agents
wall, reliable terminals, and enough project context to do real work.
