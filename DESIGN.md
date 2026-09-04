# Relay native design

Visual authority: Relay-2 at `1745dd3f68b7bc786f48142ad7da591537aeb057`, especially
its `DESIGN.md`, `theme.ts`, `AgentsWall.svelte` and shell components. The user
explicitly requested this identity and layout; V3's UI is not the reference.

The native theme lives in `apps/relay-native/src/theme.css`.

## Console and wall

- Compact top navigation, 200px project sidebar and 24px status line.
- Screen-black terminal plates with 2px gutters, square corners and a bottom
  session label strip. Agent identity remains visible when a terminal is busy.
- Neutral console `#141416`, wall `#0e0e10`, screen `#0a0a0b`, off-white
  `#ececea`, secondary text `#a5a5a3`, hairline `#252529`.
- White primary keys. Green means live, red means needs attention, amber is
  reserved for waiting states. No decorative accent color.
- Fira Sans reading text, Fira Sans Condensed navigation labels, Fira Mono
  terminal text, with installed Linux font fallbacks.
- 13px body, 12px compact controls, 11px navigation/status, 15px titles.
- Native focus indicators, keyboard navigation, selectable message/error text,
  and VTE clipboard shortcuts. No idle animation or cursor blinking.

## Native adaptations

GTK owns window controls, menus, focus and text editing. VTE owns terminal
rendering and scrolling. GtkSourceView owns source highlighting and the gutter.
The launch form is a right-side sheet. Mail composition remains mounted while
new messages refresh the history. Approval controls distinguish task approval
from permission to execute a held action.

This records the current foundation, not full Relay-2 feature parity. Full
docking, worktree-aware Code tools, Notes satellite, skills/settings and other
future surfaces remain in the imported roadmap.
