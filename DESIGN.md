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
plates, narrow gutters and persistent top identity strips make sessions
readable at a glance. In Operate mode, terminal work and attention states lead.

Visual authority is Relay-2 at `1745dd3f68b7bc786f48142ad7da591537aeb057`, especially
its `theme.ts`, `TerminalPane.svelte`, `AgentsWall.svelte`, `Icon.svelte` and
shell components. Rendered component anatomy takes precedence over stale prose. The user
explicitly requested this identity and layout; V3's UI is not the reference.
The native theme lives in `apps/relay-native/src/theme.css`, with runtime palette
and wallpaper application in `shell.rs`. This records the implemented GTK4
surfaces; it does not certify complete feature parity, visual fidelity or device
runtime behavior. Validation and remaining coverage belong in
`docs/VERIFICATION.md` and `docs/PARITY.md`.

**Key Characteristics:**
- Screen-black square plates with top session identity strips.
- Compact neutral console, off-white primary keys and state-only color.
- Fira reading text, condensed labels and monospace terminals.
- Native GTK controls, VTE terminals and GtkSourceView source editing.
- Sidebar navigation, worktree tools and persistent note tabs/satellites.
- Matte, Dark and OLED palettes with optional wallpaper controls.

## Colors

Frontmatter records the default Matte palette; names below describe its roles.
Dark and OLED modes replace the neutral palette through the same semantic roles.
Live and held colors retain their meanings in every mode.

### Primary
- **Lit Ink** (`ink`): primary text and primary action fill.
  Primary keys use `wall` text and brighten to white on hover.

### Secondary
- **Live Green** (`live`): running session lamp.
- **Attention Red** (`held`): held session lamp and session name;
  also the notice underline.
- **Waiting Amber** (`waiting`): reserved in the theme for waiting states.
  Waiting lamps currently use secondary ink. All terminal frames remain neutral.

### Neutral
- **Wall**: canvas and gaps between plates. **Console**: bars, sidebar and strips.
- **Slab**: ordinary keys, records, dropdown buttons and popovers.
- **Wash**: hover, selected navigation and text selection backgrounds.
- **Screen**: terminals and text-entry ground. **Secondary**: metadata and quiet text.
- **Edge**: hairline divisions and neutral frames.

**The Tally Rule.** Chrome color communicates state. Primary actions stay off-white; ANSI and syntax colors remain inside their content surfaces.

## Typography

Fira Sans carries reading text; Fira Sans Condensed carries tracked navigation
labels. Fonts use installed Linux fallbacks. VTE requests Fira Mono at 10pt by
default, adjustable from 8pt to 24pt in Settings. These are Pango point sizes,
not CSS pixels. GTK owns scaling and metrics.

Body, titles, controls and labels use their frontmatter roles. Status text is
11px plain sans; session names are 13px semibold. The RELAY wordmark increases
tracking to 0.16em. GtkSourceView uses system monospace and the installed
Adwaita-dark syntax scheme when available; it does not yet reproduce the full
Relay-2 editor palette.

## Layout

The sidebar requests 200px. The top bar has a 42px minimum height, the status
line 24px and toolbars 38px; native theme metrics can increase these minimums.
Task and module details occupy the content page. The command palette is centered
inside the app and window presets open below the title bar. Only Notes and the
emulator use separate app windows.

The default window is 1440 by 900. Pages and the right-side launch sheet use
20px insets; the sheet overlays the wall at 780px including its insets. It does not shrink
the terminal wall. A scrim blocks background actions and Escape dismisses an idle sheet.

Navigation lives in the sidebar, followed by projects grouped under workspaces.
Settings sits in its footer. The top bar carries identity and utility controls,
without a second row of page tabs. The bottom status bar exposes resource and
device tools.

Grid mode uses square terminal plates with 2px gutters and one, two or three
columns. The two-column wall has a draggable divider. Focus shows one session
with session tabs; Review places the focused session beside the remaining
stack. Plates request at least 280 by 280 and the wall scrolls vertically.
Sessions can be reordered or focused within the wall. A collapsible
file rail sits beside the wall. Layout selection, order and split position can
be saved. There are no automatic mobile breakpoints.

Code uses resizable file-tree, editor and Git regions with a worktree selector.
Its tree split starts at 240px; file and Git regions can be hidden. Notes uses a
list beside a scrollable native tab strip and editor. Tabs identify project and
note, can reorder, and can open in separate windows.

## Elevation & Depth

Depth comes from tonal surface steps and thin dividers. Custom buttons and the
titlebar suppress shadows. Compact in-app panels use a thin edge and a restrained
shadow; OS file choosers keep their native decoration. Launch reveal duration is zero and VTE cursor blinking is off.

Optional PNG/JPEG wallpaper fills the window with cover sizing, beneath a black
dim layer. Settings exposes a wallpaper library, panel opacity, wallpaper dim
and content contrast. Increasing content contrast raises effective panel
opacity. The top bar sits inside the wallpaper backdrop like the status bar, so
both chrome strips show it through one console layer. Terminal plates follow
panel opacity too, as one slightly denser layer (`@plate`, about 5 points above
the chrome) over the page; VTE's own ground is clear, and a parked session's
slate hides its stale screen. The workspace strip above the wall adds no layer
of its own. Text-entry grounds remain opaque for legibility. The Matte default
uses fully opaque panels, which keeps the screen-black plates.

The status bar's usage meters show, per enabled provider, the 5-hour, weekly
and Fable windows chosen in the limits popup, with amber at 70% and red at 90%,
and faint once a window has reset since its report. Beside them sit when the
limits were last read and a refresh key.

## Shapes

Plates, records and navigation rows are square. Keys and entries use the small
key radius. Terminal boundaries and strip separators are neutral 1px strokes
in every session state. Live and held lamps carry the tally color; held session
names also turn red.

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
Sidebar navigation selects with wash and ink. Compact project rows have a 30px
minimum height; the active project adds a 2px ink rail. Focus-session tabs use
neutral wash for selection. GTK owns window controls and menus.

### Icons
`apps/relay-native/src/icons.rs` carries the pinned Relay-2 SVG path geometry.
Shared shell/action icons use a 16-unit view box, 1.5-unit strokes and rounded
stroke caps and joins, rendered by GTK's SVG paintable. Preserve that geometry
when adding native controls. File-type glyphs in the explorer and Git panel are the
one exception to neutral chrome: like syntax colour, they describe content, so each
carries a muted per-language tint (`css/git_files.css`), and Git status letters use
VS Code's status colours. Native widget fallback icons remain GTK-owned;
matching path geometry alone is not proof of complete rendering fidelity.

### Records / Containers
Records are flat square slabs with 12px padding. Board lanes use console fill
and 4px padding. Notices use slab, ink and an attention-colored bottom rule.

### Terminal plate
VTE owns rendering and scrolling. Terminal margins are 10px left, 2px right,
8px top and 4px bottom. The top identity strip uses console fill, 8px side
padding and a 26px minimum height, with a neutral bottom separator. A colored
lamp leads the session name, provider/role metadata, state and compact actions.
Session names ellipsize in the middle. Identity remains visible while terminals are
busy. VTE supports Ctrl+Shift+C/V, preserves scroll position on output and
scrolls on keystrokes.

### Code and notes
GtkSourceView provides the Code gutter, highlighting, find/replace and native
text editing. Worktree selection scopes the file tree, content search, file
operations and Git controls. Read-only comparisons sit beside editable source;
staging, commits, history and branch tools stay in the Git region.

Notes uses native text editing with Markdown tools and find/replace. Open drafts
remain mounted in tabs or satellite windows, with explicit Save and Discard
controls and dirty-close protection. Plan uses the same editor with its title
fixed. Preserve project identity when a tab outlives navigation.

### Native adaptations
Mail composition remains mounted while messages refresh. Message/error text is
selectable. Approval controls distinguish task approval from permission to
execute a held action. Task details, skills, settings, launch profiles and
device tooling use native controls. Imported roadmap documents are future
requirements, not evidence that every corresponding surface is complete.

## Do's and Don'ts

### Do:
- Do preserve the compact console and square terminal plate anatomy.
- Do keep session identity visible above terminal content in its top strip.
- Do use off-white primary keys and state color only where it has meaning.
- Do preserve native keyboard focus, selection and clipboard behavior.

### Don't:
- Don't add decorative accent colors, pill surfaces or idle animation.
- Don't copy web layout rules into GTK without checking native behavior.
- Don't treat imported roadmap surfaces as implemented UI.
- Don't claim full parity, sustained performance or complete accessibility from screenshots alone.

The selected project's 2px white edge intentionally matches Relay-2's
Sidebar.svelte despite the generic detector's side-tab warning.
