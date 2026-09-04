# Relay 3 Native Rebuild: Context, Decisions, and Build Brief

Location note (2026-09-03): this file was committed to the Relay-V3 repository root as
`Relay Native rebuild` and moved here unchanged, because section 22 of the brief names this path.
Relay 3 now lives in this repository (D200); `/home/anthony/dev/Relay-2` below is the Relay 2
reference checkout, not the place where Relay 3 code goes. `apps/relay-native/` means
`apps/relay-native/` in this repository. Everything else reads as written.

Status: planning and feasibility spike  
Repository: /home/anthony/dev/Relay-2  
Target platform: Linux only  
Current application: Relay 2, a Tauri 2, Svelte 5, TypeScript, and Rust desktop app  
Proposed application: Relay 3, a fully native Linux desktop client over the existing Rust engine and bus

## 1. Read this first

This document is the self-contained handoff for an agent beginning the Relay 3 native work.

The immediate assignment is not to rewrite all of Relay. Build the bounded native feasibility spike described in sections 8 through 13. The spike must prove or disprove the risky parts before the full rebuild begins.

Before changing anything, read these files completely:

- docs/SPEC.md
- docs/BUS.md
- docs/DECISIONS.md
- AGENTS.md
- Cargo.toml
- crates/relay-core/src/pty.rs
- crates/relay-core/src/socket.rs
- crates/relay-core/src/serve.rs
- apps/relay-app/src-tauri/src/lib.rs

Relay 2 remains the working reference application. Do not remove, replace, or destabilize it during the spike.

### Naming note

The checkout path is Relay-2, and Antho refers to the current product as Relay 2 and the planned rebuild as Relay 3. The repository AGENTS.md currently labels its internal project specification as Relay v4. Treat this as historical/internal naming, not permission to rename anything. Use the repository paths and bus contracts as the source of truth, and ask Antho before changing product-version names.

## 2. Why Relay 3 exists

Relay 2 proved the product. The Rust engine, event model, worktree isolation, provider integration, PTY ownership, task coordination, device tools, and safety rules are valuable and should survive.

The part being reconsidered is the desktop presentation layer.

The current Tauri and web UI creates recurring friction in the areas Relay depends on most:

- Terminal scrolling and nested scroll regions fight each other.
- Web terminal lifetime, fit, replay, and hidden-pane behavior require continual special handling.
- Detached and secondary windows do not feel like one coherent Linux application.
- Some controls inherit browser-like or default-looking behavior rather than a deliberate desktop interaction model.
- Large terminal walls and complex panes put pressure on WebKitGTK, xterm.js, and DOM layout.
- File, task, project, notes, device, and settings surfaces have accumulated inconsistent interaction patterns.
- The application is now large enough that fixing the shell one component at a time risks preserving the wrong structure.

The goal is therefore not a rewrite for its own sake. The goal is to keep the proven backend and replace the UI boundary with a native Linux shell that has first-class terminals, windows, text rendering, keyboard behavior, drag and drop, menus, and accessibility.

## 3. What fully native means here

For Relay 3, fully native means:

- No embedded browser or webview.
- No Svelte, DOM, CSS, xterm.js, or CodeMirror in the shipped UI.
- GTK widgets, GTK windows, GTK input and focus handling, and GTK accessibility.
- VTE for terminal emulation.
- GtkSourceView for source viewing and editing.
- GSK and GTK rendering for custom presentation where ordinary widgets are insufficient.
- Pango for text layout and typography.
- Rust for both the application shell and the existing engine.
- The existing Relay bus remains the contract between UI and engine.
- The app follows Linux desktop conventions without being forced to imitate a generic GNOME utility.

Native does not mean drawing every control by hand. GTK, VTE, GtkSourceView, Pango, and GSK are the native platform stack. Reimplementing their behavior would be less native in practice and much riskier.

## 4. Recommended stack

Use this stack unless the spike finds a concrete blocker:

| Layer | Choice | Reason |
| --- | --- | --- |
| Language | Rust | Matches the existing engine and removes a second UI language |
| Desktop toolkit | GTK 4 through gtk4-rs | Best fit for a Linux-only Rust application |
| Application framework | Plain gtk4-rs first | Keeps the spike small and avoids committing to another abstraction |
| Terminal | VTE 4 through vte4-rs | Native terminal widget with selection, scrollback, input, resize, and accessibility |
| Editor/viewer | GtkSourceView 5 through sourceview5-rs | Native source buffer, syntax highlighting, gutters, search, and editing |
| Styling | GTK CSS plus application-owned tokens | Preserves a coherent visual system without a web runtime |
| Custom rendering | GSK only where needed | Useful for wallpaper, overlays, and uncommon visual effects |
| Text | Pango | Native layout, font metrics, shaping, and truncation |
| Async bridge | GLib main context plus existing Tokio runtime | GTK stays on its main thread while Relay work remains asynchronous |
| Engine contract | Existing relay-bus protocol | Prevents the UI rewrite from becoming a backend rewrite |
| Initial process model | Native client connected to relay serve | Exercises the real socket door and creates a clean crash boundary |
| Likely final process model | Native client plus a long-lived relayd service | Lets terminals and agent work survive UI restarts, if the spike supports it |
| Persistence | Existing SQLite store | No data migration should be invented for the UI rewrite |
| Git and worktrees | Existing relay-core implementation | Preserve the current safety and isolation model |
| Packaging | Cargo plus the normal Arch and Linux packaging path | Keep packaging conventional and inspectable |

Expected Arch development packages:

    sudo pacman -S --needed gtk4 vte4 gtksourceview5 pkgconf

Do not install packages without Antho's approval. First run the preflight checks in section 8.

Official references:

- GTK 4: https://docs.gtk.org/gtk4/
- gtk4-rs: https://gtk-rs.org/gtk4-rs/stable/latest/docs/gtk4/
- VTE: https://gnome.pages.gitlab.gnome.org/vte/gtk4/
- vte4-rs: https://docs.rs/vte4/
- GtkSourceView 5: https://gnome.pages.gitlab.gnome.org/gtksourceview/gtksourceview5/
- sourceview5-rs: https://docs.rs/sourceview5/

## 5. Why GTK instead of the alternatives

### Qt

Qt is capable and polished, but the Rust binding and build story add more cross-language machinery. It is a reasonable fallback only if GTK or VTE fails a measured requirement. Do not switch based on taste alone.

### Slint, Iced, egui, or another Rust-first renderer

These can produce good applications, but Relay needs mature terminal embedding, source editing, accessibility, input methods, drag and drop, window behavior, and desktop integration. Choosing one would likely require embedding GTK components anyway or rebuilding platform behavior.

### Continue with Tauri

That would preserve the exact boundary being replaced. It is still a valid product stack, but it does not answer the native-terminal and native-window goal.

### libadwaita

Do not make it a foundational dependency during the spike. It can help with some controls later, but Relay has a dense, custom application shell. First prove plain GTK 4. Adopt individual libadwaita patterns only when they materially help.

## 6. Contracts that must survive

The existing UI is replaceable. These contracts are not.

### 6.1 The bus is the product boundary

All UI actions go through relay-bus operations. Do not call relay-core handlers directly from widgets. Do not add native-only side doors.

The native client must understand:

- Request and response envelopes.
- Typed errors with stable kind and code fields.
- Durable events and transient events.
- Request correlation.
- Subscription lifecycle.
- Data-plane frames.
- Reconnection and resynchronization.

If a feature cannot be expressed through the bus, add or repair the bus operation first, update the schema, and add bus-driven tests.

### 6.2 The socket is a multiplexed stream

The existing socket door uses newline-delimited JSON. A connection can carry:

- Responses.
- Events.
- PTY frames.
- Mirror frames.
- Logcat frames.
- Log frames.

Do not base the GUI client on the current tiny sequential request helper if it cannot safely demultiplex concurrent traffic. Build one connection reader that routes messages to pending requests and subscriptions.

Keep the protocol implementation small. Do not create a generic networking framework.

### 6.3 PTYs remain core-owned

The engine owns PTY processes, output sequencing, scrollback, attach and detach semantics, input, and resize.

The VTE widget is a renderer and input surface. It must not spawn the provider process independently.

Current PTY expectations include:

- 8 MiB retained scrollback in relay-core.
- Bounded catch-up.
- Epoch and sequence tracking.
- Live broadcast.
- Explicit attach and detach.
- Input through session.input.
- Resize through the bus.
- Sessions surviving a UI pane disappearing.
- Correct teardown order.

### 6.4 Worktrees and user work are preserved

Every implementation agent works in an isolated worktree. Removing a project or workspace must not silently destroy terminals, repositories, branches, or user work.

Never introduce automatic cleanup that discards state.

### 6.5 No polling

Relay is event-driven. Do not add UI polling timers, idle subprocesses, or repeated filesystem scans. Watchers and burst coalescing already exist for a reason.

### 6.6 Performance requirements remain real

The rebuild is only successful if it preserves or improves the product-level targets:

- Startup below two seconds with six restored terminal panes.
- Eleven streaming panes remaining visually smooth.
- Idle CPU below one percent.
- Near-instant perceived terminal creation and removal.
- Bounded memory and scrollback behavior.
- No output loss when panes are hidden, moved, or detached.

Measure these. Do not claim them from code inspection.

### 6.7 UI and product ownership

Claude will own the detailed Relay 3 UI and UX design. The spike may establish widget capability, information architecture constraints, and performance facts. It must not lock in the final visual design.

Use intentionally plain temporary widgets in the spike. Separate proof code from product styling.

## 7. Repository strategy

Create the spike alongside Relay 2:

    apps/relay-native/

Recommended minimal structure:

    apps/relay-native/
      Cargo.toml
      README.md
      src/
        main.rs
        app.rs
        bus_client.rs
        terminal.rs
        editor.rs

Add the crate to workspace members only when needed for normal Cargo operation. Do not add it to default-members during the spike, because GTK system dependencies should not break the existing default Relay checks.

Do not:

- Rename the repository yet.
- Delete the Tauri application.
- Move relay-core.
- Fork the bus schema.
- Extract speculative shared UI abstractions.
- Create a design system before Claude provides the UI direction.
- Add a second persistence layer.
- Start the entire migration in one branch.

The spike should be easy to delete if it fails.

## 8. Agent startup protocol

The assigned implementation agent must begin this way:

1. Call session.bootstrap.
2. Read the files listed in section 1.
3. Inspect the current branch, worktree, and dirty state.
4. Query the live bus schema for any operation used by the spike.
5. Claim the exact files before editing.
6. Use the assigned development and test instance names.
7. Keep the guardrail active.
8. Make small commits at proven checkpoints.
9. Report any missing native package before attempting installation.
10. Stop if the worktree is not isolated or the branch is unexpectedly behind.

The bootstrap brief should explicitly report how far the branch is behind its base. If that information is unavailable or wrong, record it as a Relay 2 defect rather than guessing.

## 9. Phase zero: preflight and baseline

Before writing native UI code, establish facts.

### 9.1 Check native dependencies

Run:

    pkg-config --modversion gtk4
    pkg-config --modversion vte-2.91-gtk4
    pkg-config --modversion gtksourceview-5

Record exact versions.

If a package is missing, stop and request approval for the smallest package installation. Do not substitute another toolkit silently.

### 9.2 Prove the headless engine

Start the existing headless server through the documented Relay CLI path. Confirm:

- The socket is created under the documented runtime directory.
- A basic request succeeds.
- Events can arrive independently of responses.
- A PTY session can be launched and attached without Tauri.
- The engine continues when the test client disconnects, if current lifecycle rules allow it.

If relay serve cannot support this, diagnose the contract gap before creating more GUI code.

### 9.3 Record the Relay 2 baseline

Measure or record, on the same machine:

- Cold and warm startup.
- Idle CPU.
- Memory after restoring six terminals.
- Terminal creation latency.
- Terminal removal latency.
- Eleven-pane streaming behavior.
- Scroll and resize behavior.
- Whether the terminal session survives closing its visible pane.

The comparison does not need laboratory precision, but it needs reproducible commands and conditions.

## 10. Phase one: native shell proof

Build the smallest GTK application that proves:

- A main application window opens.
- A header or command area, sidebar, center content, and lower status area can be arranged.
- Keyboard focus moves correctly.
- A secondary GTK window can be opened and closed.
- The main window can restore basic geometry.
- The application shuts down without leaving the test process or socket client behind.

Use placeholders. Do not recreate the full Relay chrome.

Success condition: the shell is stable across repeated open, close, resize, maximize, and secondary-window cycles.

## 11. Phase two: native bus client

Implement one small asynchronous client for the existing socket protocol.

Required behavior:

- Connect and reconnect deliberately.
- Allocate unique request IDs.
- Send requests.
- Resolve responses to the correct pending caller.
- Dispatch events independently.
- Route PTY frames by subscription.
- Reject malformed or unknown envelopes with a typed local error.
- Cancel pending requests cleanly on disconnect.
- Avoid blocking the GTK main loop.
- Avoid polling.

Suggested shape:

- One socket reader task.
- One socket writer path.
- A pending-request map keyed by request ID.
- Explicit subscription routing.
- GLib main-context delivery for state that changes GTK widgets.

Do not add an actor framework or a generalized reactive store. The bus already supplies the architecture.

Add focused tests using a local Unix socket fixture or the real in-process test engine, whichever gives the most faithful and smallest test.

## 12. Phase three: VTE terminal proof

This is the main go or no-go test.

Build one terminal pane connected to a real Relay session through the bus.

### Output path

- Attach to the session.
- Feed catch-up bytes to VTE in order.
- Feed live bytes to VTE in order.
- Track epoch and sequence.
- Detect an invalid sequence instead of silently corrupting output.
- Detach cleanly when the widget disappears.
- Reattach without losing engine-owned history.

### Input path

- Translate VTE user input into session.input.
- Preserve pasted text and control sequences.
- Verify modifier behavior.
- Do not echo locally unless VTE and the PTY protocol require it.

### Resize path

- Read rows and columns from the VTE terminal.
- Coalesce resize bursts through the UI event loop.
- Send the final size through the existing bus operation.
- Do not poll size.
- Verify that full-screen TUIs redraw correctly.

### Terminal interaction matrix

Manually test at minimum:

- Normal shell prompt.
- Claude Code.
- Codex CLI.
- Kimi or Qwen if available.
- Mouse selection and copy.
- Paste.
- Wheel and touchpad scrolling.
- Shift plus Page Up and Page Down.
- Search, if enabled.
- A full-screen TUI.
- Rapid output.
- Unicode, emoji, combining marks, and wide characters.
- Window resize during output.
- Hide, show, move, detach, and reattach.
- Closing the native client while the engine owns a live session.
- Relaunching the client and restoring the session.

Success condition: no lost or duplicated output, no stuck input, no scroll ownership conflict, correct resize, and stable reconnection.

## 13. Phase four: wall and editor proofs

### 13.1 Terminal wall

Create a temporary grid capable of showing eleven VTE panes.

Measure:

- Frame smoothness during simultaneous output.
- CPU use while streaming.
- Idle CPU.
- Memory.
- Input latency in the focused pane.
- Resize behavior.
- Cost of hiding and revealing panes.
- Cost of removing a pane.

Do not optimize blindly. Capture the profile or measurement that justifies any optimization.

### 13.2 GtkSourceView

Build a small source viewer and editor that proves:

- Loading a repository-relative file.
- Syntax highlighting.
- Line numbers.
- Search.
- Editable and read-only modes.
- Dirty state.
- Save through the Relay bus, not direct widget-owned filesystem logic.
- Reasonable behavior on a large file.
- Correct UTF-8 handling.

Do not build the final file tree or editor workspace during this phase.

## 14. Spike deliverables and decision gates

The spike is complete only when it produces all of these:

- apps/relay-native with the minimum proof implementation.
- apps/relay-native/README.md with exact build and run commands.
- docs/RELAY-3-NATIVE-SPIKE-RESULTS.md with measurements and findings.
- Automated tests for the socket demultiplexer and any sequence handling.
- A list of native dependency versions.
- A list of blockers, workarounds, and unresolved risks.
- Screenshots or a short capture of the running proof, when practical.
- A recommendation: proceed, change one dependency, or stop.

### Proceed gate

Proceed to the full rebuild only if all are true:

- VTE works with the existing core-owned PTY model.
- The socket client handles responses, events, and frames concurrently.
- Eleven panes are feasible on the target machine.
- GtkSourceView covers the required editor behavior.
- Window and focus behavior are stable.
- The native client does not require a second business-logic implementation.
- Relay 2 remains intact.

### Stop or reconsider gate

Stop and report if any of these occurs:

- VTE requires owning the child process in a way that breaks Relay session persistence.
- The bus protocol cannot safely support a standalone client without a large redesign.
- Native rendering is materially worse for the terminal wall after a fair implementation.
- Required UI behavior depends on inaccessible or unmaintained bindings.
- The spike begins duplicating relay-core logic.

A failed spike is useful evidence. Do not hide it with a larger rewrite.

## 15. Decisions to record after the spike

Do not rewrite old decisions. Append new D-entries to docs/DECISIONS.md for accepted decisions.

Likely decisions:

- Relay 3 uses GTK 4 as the Linux application toolkit.
- Relay 3 uses VTE as a renderer for core-owned PTYs.
- Relay 3 uses GtkSourceView for source viewing and editing.
- The UI connects through the socket door rather than directly invoking handlers.
- Engine lifetime is independent from UI lifetime, if proven.
- Relay 2 remains available until migration parity and rollback criteria are met.
- UI state ownership and persistence boundaries.
- Native theming and accessibility rules.
- The exact deprecation path for the Tauri application.

Regenerate schema/bus.v1.json whenever bus operations or payloads change.

## 16. Full migration roadmap after a successful spike

Astra should receive the spike, this document, and the Relay 2 repository. Implement the
migration in vertical slices while treating the rendered Relay 2 interface as the visual source
of truth.

### Stage 1: foundation

- Productionize the native bus client.
- Define app startup, reconnection, shutdown, and crash recovery.
- Decide whether relay serve becomes relayd.
- Restore window geometry and shell state.
- Add native error presentation and diagnostics.
- Add theme tokens, typography, icons, and accessibility foundation from Claude's design.

### Stage 2: Agents wall

- Session creation and provider/model selection.
- Native terminal panes.
- Layout, focus, resize, drag, undock, and presets.
- Resume, park, wake, clear context, and discard placement.
- Session status and activity.
- Correct terminal creation and removal latency.
- Completion and notification deduplication.

This is the first useful daily-driver milestone.

### Stage 3: Code workspace

Keep the Code tab. Its multiview puts the useful coding surfaces on one page:

- File tree.
- File creation, rename, move, drag and drop, trash, and restore.
- Native source editor.
- Git behind one top-right button beside Skills.
- Workspace and branch switching.
- Branch ahead and behind truth.
- Branch cleanup with merged-PR truth.
- Watcher-driven refresh with burst coalescing.
- Agent file dock.
- Build logs without nested-scroll conflicts.

### Stage 4: tasks, board, and coordination

- Condensed task board.
- Clicking a task opens a dedicated detail surface.
- Edit task.
- Message the task or assigned agent.
- View subtasks and linked tasks.
- View movement and state history.
- Improve review-group orchestration and following behavior.
- Preserve approval and authorization contracts.
- Remove stale copy that says a task is recorded for review when session.bootstrap is the real action.

### Stage 5: Notes

Notes should be a first-class satellite window rather than a weak duplicate page inside the main shell:

- Opens instantly.
- Custom native header.
- No useless outer borders.
- Resizable and collapsible document sidebar.
- Correct close and reopen behavior.
- Full-height content.
- Preserve durable note storage and bus operations.

Do not recreate the current in-app Notes page merely for parity.

### Stage 6: skills, settings, and shell polish

- Skill library redesign.
- Consistent project switching and skill classification.
- Draggable projects and workspaces.
- Clear project and workspace hierarchy.
- Settings information architecture.
- Settings search.
- Wallpaper gallery and rotation.
- Correct custom-wallpaper preview.
- Wallpaper dim and content-projection placement.
- Custom Relay icon.
- VS Code or OpenAI-quality file and folder icon system.
- More useful window presets.

### Stage 7: devices

- Device controls.
- Branch and build selection that never overflows.
- Mirror and run panes.
- Logcat and crash reporting.
- Requested-only resource behavior.
- Preserve scrcpy discovery and lifecycle rules.

### Stage 8: advanced agent surfaces

Treat these as separate product features, not launch blockers:

- Text and voice assistant that can control Relay.
- Smaller conversational model that can notify Antho when intervention is needed.
- Agent mode: a GUI conversation surface over provider CLIs with access to chosen workspace context.
- Deep links or continuity with official Claude and Codex apps where their public contracts allow it.
- Optional pre-commit change summary and terminal naming, using an inexpensive model only if its value exceeds latency and cost.
- Safe automatic updates for provider CLIs.
- Mobile or remote Relay companion.

### Stage 9: replacement and release

- Feature-by-feature parity ledger.
- Migration of shell state and user preferences.
- Fresh install test.
- Upgrade test with an existing Relay database.
- Crash recovery test.
- Multi-window test.
- Accessibility pass.
- Packaging and desktop integration.
- Performance comparison with the Relay 2 baseline.
- Daily-driver period with Relay 2 retained as rollback.
- Remove Tauri only after explicit acceptance.

## 17. Current product backlog classified

This is the current list translated into ownership. Preserve the original intent even when wording changes.

### Do now in Relay 2 when it produces reusable truth

These are worthwhile before Astra because they clarify backend behavior, correctness, or product rules that Relay 3 should inherit:

- Deduplicate completion notifications. Six notifications from one agent is a contract bug, not a visual-design issue.
- Fix merged-PR truth and safe local branch removal.
- Expose branch ahead and behind state and include it in agent bootstrap.
- Fix project removal warnings so removal never forces terminal or repository destruction.
- Improve review-group coordination logic where the backend contract is wrong.
- Make file refresh watcher-driven and burst-coalesced, with no high-frequency polling.
- Fix stale task/session copy that conflicts with session.bootstrap.
- Measure terminal creation and removal latency and fix core-side delay if present.
- Define provider and model selection in the session bus contract if it is missing.
- Define update policy and safety for Codex and Claude Code before adding automatic mutation.
- Capture a Relay 2 performance baseline.
- Complete the native feasibility spike.

Only implement these in Relay 2 when the fix changes reusable Rust, bus, data, or safety behavior. Do not spend the remaining window polishing throwaway Svelte.

### Give Astra as implementation context

These need broad architectural work and belong naturally in Relay 3:

- Native GTK application shell.
- VTE terminal wall and lifecycle.
- GtkSourceView editor.
- Native Code tab with files, editor, and output together on one page.
- Native multi-window Notes app.
- Task detail, history, links, subtasks, and messages.
- Branch and workspace navigation across the project surface.
- Durable daemon or engine lifetime independent from the UI, if the spike proves it.
- Voice and text assistant control surface.
- GUI-style agent conversation mode.
- Mobile and remote companion architecture.
- Provider CLI update system.
- Full migration, packaging, state migration, and rollback plan.

Astra should see Relay 2 as both the executable specification and the visual template. Port the
rendered interface one-for-one unless Antho explicitly requests a change.

### Give Claude for UI and UX design

Claude owns the detailed interaction and visual specification for:

- Skill library hierarchy, subskills, sizing, and project switching.
- Condensed task board and full task-detail view.
- Placement of resume, clear context, and discard.
- Draggable workspace and project navigation.
- Editable project branch control.
- Rename New session to Add agents when appropriate.
- Prevent the previous task from leaking into new-project flow.
- Device build selector overflow.
- File sidebar parity, drag behavior, controls, and native context menus.
- Branch selector styling.
- Terminal and build-log scroll ownership.
- Notes window layout, header, sidebar collapse, and resizing.
- Project and workspace menu hierarchy.
- Code-page file and output surfaces, with Git behind one top-right button beside Skills.
- Dashboard redesign or evidence-based removal.
- Editor visual design and code-writing ergonomics.
- File and folder icon system.
- Window presets.
- Application icon.
- Wallpaper rotation and preview behavior.
- Settings hierarchy, search, wallpaper dim, and content projection.
- Model-selection presentation.
- Session and terminal naming presentation.

Claude should design against real GTK, VTE, and GtkSourceView capabilities discovered by the spike. Do not ask Claude to design browser interactions and translate them afterward.

### Antho must decide

These are product choices, not tasks an implementation agent should silently settle:

- Whether the dashboard has a valuable job or should be removed.
- Whether Plan remains removed.
- Whether Notes exists only as a satellite window.
- Whether editable project branch means default branch, active browsing branch, new-agent base, or all three as separate concepts.
- What Agent mode promises regarding continuity with official Claude and Codex apps.
- Whether the mobile app requires the desktop to be online or uses a hosted durable service.
- How much automatic provider updating is acceptable.
- When Relay 3 becomes the daily driver and when Relay 2 may be retired.

## 18. Product-specific acceptance notes

The rebuild must solve the intent behind the complaints, not copy each complaint as a disconnected checkbox.

### Notifications

One logical completion should create one notification. Multiple lifecycle reports, provider hooks, task transitions, and reconnect replay must collapse to the same durable identity.

### Branches

Do not infer merged state from local branch shape. Use authoritative PR state where available and expose uncertainty. Show ahead and behind counts with the exact base and last refresh time.

### Terminals

There must be one obvious scroll owner under the pointer. Build logs and terminal panes must not create overlapping narrow scroll targets. Removing a pane should feel immediate while session destruction remains an explicit separate action.

### Code workspace

The Code tab keeps files, editor, agent context, and relevant output on the same page. Git state
and branch tools open from one top-right button beside Skills; they are not another always-visible
panel.

### Notes

The Notes window is a tool in its own right. It should open immediately, close normally, restore predictably, and use the available window space.

### Removal safety

Removing something from Relay's navigation is not permission to delete its repository, worktree, branch, terminal, or unsaved work. Make destructive scope explicit.

## 19. Non-goals for the first native milestone

Do not include these in the first daily-driver milestone:

- Mobile client.
- Hosted Relay service.
- Voice recognition.
- General autonomous assistant.
- Cross-provider chat synchronization.
- Automatic provider self-updates.
- Rotating wallpapers.
- Elaborate settings redesign.
- Final dashboard.
- Every historical Relay feature.

The first milestone is the native shell, Agents wall, reliable terminals, and enough project context to perform real work.

## 20. Verification commands

Use the repository's existing commands where applicable. Keep environment setup compatible with fish.

Core checks:

    cargo fmt --all --check
    cargo check
    cargo test

Schema drift after any bus change:

    cargo run -p relay-bus --example dump_schema > schema/bus.v1.json
    cargo test -p relay-bus

Native crate checks, once the crate exists:

    cargo fmt --all --check
    cargo check -p relay-native
    cargo test -p relay-native
    cargo clippy -p relay-native --all-targets -- -D warnings

Also run:

    git diff --check

GUI behavior and performance require an actual Linux display pass. Compilation and unit tests are not visual validation.

## 21. Handoff format at the end of every stage

Report:

- Outcome first.
- Exact commit.
- Files changed.
- Bus operations added or changed.
- Schema regeneration status.
- Tests and checks run.
- Native dependency versions.
- Measured performance where relevant.
- Visual or device checks actually performed.
- Remaining risks.
- Any behavior that differs from Relay 2 and why.
- The next smallest vertical slice.

Never report a native visual result that was not observed on a real display.

## 22. Immediate task to assign

Use this exact task when handing the work to another Relay agent:

> Build the bounded Relay 3 native feasibility spike described in docs/RELAY-3-NATIVE-BUILD-BRIEF.md. Start with session.bootstrap and the required reading. Create apps/relay-native alongside Relay 2, keep the existing Tauri app intact, and prove GTK 4, a concurrently multiplexed Relay socket client, a core-owned PTY rendered through VTE, an eleven-pane terminal wall, and a GtkSourceView file proof. Do not begin the full migration or final UI design. Record reproducible measurements and write docs/RELAY-3-NATIVE-SPIKE-RESULTS.md. Stop and report rather than inventing a second backend or silently changing toolkit.

## 23. The governing principle

Improve Relay 2 now when the work produces reusable truth: backend correctness, bus contracts, performance measurements, safety rules, and the native spike.

Give Astra the proven architecture and migration.

Give Claude the UI and UX.

Do not spend the remaining time perfecting Svelte surfaces that will be deleted, and do not ask Astra to discover basic native feasibility while simultaneously rebuilding the entire application.
