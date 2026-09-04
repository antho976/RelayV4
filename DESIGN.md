---
name: Relay native
description: A compact native console with screen-black terminal plates and off-white action keys.
colors:
  wall: "#0e0e10"
  console: "#141416"
  slab: "#1b1b1e"
  wash: "#232327"
  screen: "#0a0a0b"
  ink: "#ececea"
  secondary: "#a5a5a3"
  edge: "#252529"
  live: "#2ec469"
  held: "#e5382e"
  waiting: "#f0a828"
typography:
  body:
    fontFamily: "Fira Sans, Noto Sans, sans-serif"
    fontSize: "13px"
  title:
    fontFamily: "Fira Sans, Noto Sans, sans-serif"
    fontSize: "15px"
    fontWeight: 600
  control:
    fontFamily: "Fira Sans, Noto Sans, sans-serif"
    fontSize: "12px"
  label:
    fontFamily: "Fira Sans Condensed, Fira Sans, sans-serif"
    fontSize: "11px"
    fontWeight: 600
    letterSpacing: "0.08em"
  terminal:
    fontFamily: "Fira Mono"
    fontSize: "10pt"
rounded:
  plate: "0px"
  key: "2px"
spacing:
  gutter: "2px"
  small: "4px"
  medium: "8px"
  record: "12px"
  page: "20px"
components:
  button:
    backgroundColor: "{colors.slab}"
    textColor: "{colors.ink}"
    rounded: "{rounded.key}"
    padding: "0 10px"
  button-primary:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.wall}"
    rounded: "{rounded.key}"
    padding: "0 10px"
  button-quiet:
    backgroundColor: "transparent"
    textColor: "{colors.secondary}"
    rounded: "{rounded.key}"
    padding: "0 10px"
  field:
    backgroundColor: "{colors.screen}"
    textColor: "{colors.ink}"
    rounded: "{rounded.key}"
  navigation:
    backgroundColor: "transparent"
    textColor: "{colors.secondary}"
    rounded: "{rounded.plate}"
  record:
    backgroundColor: "{colors.slab}"
    rounded: "{rounded.plate}"
    padding: "12px"
  terminal-plate:
    backgroundColor: "{colors.screen}"
    textColor: "{colors.ink}"
    rounded: "{rounded.plate}"
---

# Design System: Relay native

## Overview

**Creative North Star: "The Multiviewer"**

Relay is a compact console surrounding a wall of agent terminals. Screen-black
plates, narrow gutters and persistent bottom identity strips make sessions
readable at a glance. In Operate mode, terminal work and attention states lead.

Visual authority is Relay-2 at `1745dd3f68b7bc786f48142ad7da591537aeb057`, especially
its `DESIGN.md`, `theme.ts`, `AgentsWall.svelte` and shell components. The user
explicitly requested this identity and layout; V3's UI is not the reference.
The native theme lives in `apps/relay-native/src/theme.css`. This documents the
built GTK4 foundation, not full Relay-2 feature parity.

**Key Characteristics:**
- Screen-black square plates with bottom session identity strips.
- Compact neutral console, off-white primary keys and state-only color.
- Fira reading text, condensed labels and monospace terminals.
- Native GTK controls, VTE terminals and GtkSourceView source editing.

## Colors

Frontmatter records the native theme palette; names below describe its roles.

### Primary
- **Lit Ink** (`ink`): primary text, primary action fill and selected top tabs.
  Primary keys use `wall` text and brighten to white on hover.

### Secondary
- **Live Green** (`live`): running terminal frame and identity-strip top rule.
- **Attention Red** (`held`): held terminal frame, strip rule and session name;
  also the notice underline.
- **Waiting Amber** (`waiting`): reserved in the theme for waiting states.
  Waiting terminal frames currently remain neutral; amber is not yet applied.

### Neutral
- **Wall**: canvas and gaps between plates. **Console**: bars, sidebar and strips.
- **Slab**: ordinary keys, records, dropdown buttons and popovers.
- **Wash**: hover, selected navigation and text selection backgrounds.
- **Screen**: terminals and text-entry ground. **Secondary**: metadata and quiet text.
- **Edge**: hairline divisions and neutral frames.

**The Tally Rule.** Chrome color communicates state. Primary actions stay off-white; ANSI and syntax colors remain inside their content surfaces.

## Typography

Fira Sans carries reading text; Fira Sans Condensed carries tracked navigation
labels. Fonts use installed Linux fallbacks. VTE requests Fira Mono at 10pt,
which is a Pango point size, not a CSS pixel size. GTK owns scaling and metrics.

Body, titles, controls and labels use their frontmatter roles. Status text is
11px plain sans; session names are 12px semibold. The RELAY wordmark increases
tracking to 0.16em. GtkSourceView uses system monospace and the installed
Adwaita-dark syntax scheme when available; it does not yet reproduce the full
Relay-2 editor palette.

## Layout

The sidebar requests 200px. The top bar has a 40px minimum height, the status
line 24px and toolbars 38px; native theme metrics can increase these minimums.
The default window is 1440 by 900. Pages and the right-side launch sheet use
20px insets; the sheet requests 380px width.

Terminal plates form an equal-cell GTK grid with 2px gutters. Single, Split and
Grid select one, two or three columns; Focus shows one session. Plates request
at least 280 by 280 and the wall scrolls vertically. There are no automatic
breakpoints or mobile layouts in this foundation. The Code view uses a native
resizable split initially positioned at 240px for the tree.

## Elevation & Depth

Depth comes from tonal surface steps and thin dividers. Custom buttons and the
titlebar suppress shadows. Popovers use slab fill and an edge stroke; remaining
popup decoration belongs to GTK. No custom shadow or motion token system is
implemented. Launch reveal duration is zero and VTE cursor blinking is off.

## Shapes

Plates, records and navigation rows are square. Keys and entries use the small
key radius. Terminal boundaries and strip top rules are 1px strokes; live and
held states change their color without changing their width.

## Components

### Buttons
Keys have a 28px minimum height. Ordinary keys use slab, primary keys use ink,
and quiet keys are transparent with secondary text. Hover uses wash except for
primary keys, which brighten to white. Keyboard focus uses an inset 2px ink
outline; disabled keys use 45% opacity.

### Inputs / Fields
Entries use screen fill, an edge stroke and a 30px minimum height. Focus changes
the stroke to secondary. Native text views have 8px padding and ink carets.
GTK supplies editing, selection and accessibility semantics.

### Navigation
Top tabs invert when selected. Sidebar navigation selects with wash and ink.
Project rows have a 44px minimum height; the active project adds a 2px ink rail.
GTK owns window controls and menus.

### Records / Containers
Records are flat square slabs with 12px padding. Board lanes use console fill
and 4px padding. Notices use slab, ink and an attention-colored bottom rule.

### Terminal plate
VTE owns rendering and scrolling. Terminal margins are 10px left, 2px right,
8px top and 4px bottom. The bottom identity strip uses console fill, 8px side
padding and a 28px minimum height. Session names ellipsize in the middle; status
and compact action keys follow. Identity remains visible while terminals are
busy. VTE supports Ctrl+Shift+C/V, preserves scroll position on output and
scrolls on keystrokes.

### Native adaptations
Mail composition remains mounted while messages refresh. Message/error text is
selectable. Approval controls distinguish task approval from permission to
execute a held action. GtkSourceView owns highlighting and the gutter.
Full docking, worktree-aware Code tools, Notes satellite, skills/settings and
other future surfaces remain in the imported roadmap.

## Do's and Don'ts

### Do:
- Do preserve the compact console and square terminal plate anatomy.
- Do keep session identity visible beneath terminal content.
- Do use off-white primary keys and state color only where it has meaning.
- Do preserve native keyboard focus, selection and clipboard behavior.

### Don't:
- Don't add decorative accent colors, pill surfaces or idle animation.
- Don't copy web layout rules into GTK without checking native behavior.
- Don't treat imported roadmap surfaces as implemented UI.
- Don't claim full parity, sustained performance or complete accessibility from foundation captures.
