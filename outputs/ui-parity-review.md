# Native UI review build

This records the initial visual pass, which the user approved. The subsequent [native roadmap results](native-roadmap-results.md) supersede the Code/Plan navigation and Notes behavior described below.

Reference: Relay-2 `1745dd3f68b7bc786f48142ad7da591537aeb057`, verified against the repository's remote HEAD. Three agents reviewed and implemented shell, workspace pages, and tools/settings in parallel.

The worktree now uses the original Fira font data, SVG icons, colors, terminal palette, compact shell dimensions, and reference page structure across the wall, Dashboard, Code/Git, Board, Notes, Plan, Skills, Settings, launch, setup and device controls. GTK, VTE and GtkSourceView remain native.

Incidental repairs cover stale navigation events, delayed layout restoration, Settings sidebar persistence, fractional settings values, the wrong document appearing in Plan, Code tree clipping, Dashboard overflow, and rejected Skills toggles resubmitting themselves.

## Verified

- Native build, all 9 native unit tests, formatting, strict Clippy with existing GTK deprecations allowed, and diff whitespace checks pass.
- Real GTK screenshots at 1440×900 and 1024×768. Assertions check the 42px top bar, 24px status bar, 28px Files rail, 26px terminal headers, actual Fira font selection, and requested visible pages.
- Editor, task and note saves; review-group launch; six terminal paste echoes; eleven sessions surviving window closure; eleven simultaneous 2,048-line bursts reaching VTE within the five-second test budget.
- All ten isolated setup/utility scenarios pass, including local and simulated GitHub imports, device controls, and repeated panel toggles.
- Final targeted rendering verifies Settings → Skills → Dashboard navigation and Dashboard fitting beside the sidebar.

## Review material

Native screenshots and measured widget bounds: [review directory](../.impeccable/review/). Rendered original: [reference directory](../.impeccable/review/reference/).

[Wall](../.impeccable/review/desktop.png) · [Board](../.impeccable/review/board.png) · [Code](../.impeccable/review/code.png) · [Settings](../.impeccable/review/settings.png) · [Dashboard](../.impeccable/review/dashboard.png)

The updated binary is `target/debug/relay-native`; `./run.sh` launches this worktree's build. Changes have not been merged or pushed.

This is a rendered and measured UI parity pass, not certification of pixel identity across every state or display scale. Native window controls, text rasterization and some existing utility controls still differ. Smoke fixtures use disposable data and fake providers; real provider TUIs and physical-device workflows were not exercised. Visual acceptance remains pending.
