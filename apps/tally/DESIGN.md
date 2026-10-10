---
name: Tally
description: A month read as a pace, open on a warm page like Avex, with one serif hero per view.
colors:
  ember: "#D4761F"
  accent-red: "#E23D3D"
  pearl-ground: "#110F0C"
  container-lowest: "#0C0A08"
  container-low: "#16120F"
  container: "#1A1613"
  container-high: "#221C16"
  container-highest: "#2A231C"
  outline: "#38302A"
  outline-variant: "#2A241F"
  on-ground: "#F2EFEA"
  muted: "#BFB6AA"
  amoled-ground: "#000000"
  amoled-container-low: "#060606"
  amoled-surface: "#080808"
  amoled-container: "#0A0A0A"
  amoled-container-high: "#111111"
  amoled-container-highest: "#1A1A1A"
  amoled-outline-variant: "#1E1E1E"
  state-over: "#D9534A"
  state-under: "#6FB98A"
  hue-sage: "#7FB27A"
  hue-teal: "#4FA9A0"
  hue-sky: "#6A9FD8"
  hue-iris: "#8C87D9"
  hue-orchid: "#C27BC0"
  hue-rose: "#D9768E"
  hue-coral: "#E08A5F"
  hue-amber: "#D9A441"
  hue-olive: "#A3A84E"
  hue-sand: "#C2A585"
  hue-slate: "#8D99A6"
  hue-brick: "#B8664F"
typography:
  display-large:
    fontFamily: "serif"
    fontSize: "52sp"
    fontWeight: 400
    lineHeight: "58sp"
    fontFeature: "tnum"
  display-medium:
    fontFamily: "serif"
    fontSize: "44sp"
    fontWeight: 400
    lineHeight: "50sp"
    fontFeature: "tnum"
  headline-large:
    fontFamily: "serif"
    fontSize: "36sp"
    fontWeight: 400
    lineHeight: "42sp"
    fontFeature: "tnum"
  headline-medium:
    fontFamily: "serif"
    fontSize: "28sp"
    fontWeight: 400
    lineHeight: "34sp"
    fontFeature: "tnum"
  headline-small:
    fontFamily: "serif"
    fontSize: "22sp"
    fontWeight: 400
    lineHeight: "28sp"
    fontFeature: "tnum"
  title-large:
    fontFamily: "sans-serif"
    fontSize: "18sp"
    fontWeight: 500
    lineHeight: "24sp"
    fontFeature: "tnum"
  title-medium:
    fontFamily: "sans-serif"
    fontSize: "16sp"
    fontWeight: 500
    lineHeight: "22sp"
    letterSpacing: "0.15sp"
    fontFeature: "tnum"
  title-small:
    fontFamily: "sans-serif"
    fontSize: "14sp"
    fontWeight: 500
    lineHeight: "20sp"
    letterSpacing: "0.1sp"
    fontFeature: "tnum"
  body-large:
    fontFamily: "sans-serif"
    fontSize: "16sp"
    fontWeight: 400
    lineHeight: "24sp"
  body-medium:
    fontFamily: "sans-serif"
    fontSize: "14sp"
    fontWeight: 400
    lineHeight: "20sp"
  body-small:
    fontFamily: "sans-serif"
    fontSize: "12sp"
    fontWeight: 400
    lineHeight: "16sp"
  label-large:
    fontFamily: "monospace"
    fontSize: "13sp"
    fontWeight: 500
    lineHeight: "16sp"
    letterSpacing: "0.8sp"
  label-medium:
    fontFamily: "monospace"
    fontSize: "11sp"
    fontWeight: 400
    lineHeight: "14sp"
    letterSpacing: "0.6sp"
  label-small:
    fontFamily: "monospace"
    fontSize: "10sp"
    fontWeight: 400
    lineHeight: "12sp"
    letterSpacing: "0.5sp"
  mono-action:
    fontFamily: "monospace"
    fontSize: "16sp"
    fontWeight: 700
    lineHeight: "20sp"
    letterSpacing: "1sp"
rounded:
  seam-inner: "4dp"
  badge: "10dp"
  action: "12dp"
  picker: "14dp"
  group: "16dp"
  tile: "18dp"
  panel: "20dp"
  hero: "24dp"
  pill: "50%"
spacing:
  seam: "2dp"
  panel-gap: "28dp"
  figure-gap: "20dp"
  tile-pad: "16dp"
  row-pad: "18dp"
  panel-pad: "18dp"
  hero-pad: "20dp"
  gutter: "24dp"
  fab-clearance: "96dp"
  reading-width: "640dp"
components:
  button-hero:
    backgroundColor: "{colors.ember}"
    textColor: "{colors.pearl-ground}"
    typography: "{typography.mono-action}"
    rounded: "{rounded.action}"
    padding: "16dp 20dp"
    height: "56dp"
  button-secondary:
    backgroundColor: "{colors.container-high}"
    textColor: "{colors.on-ground}"
    typography: "{typography.mono-action}"
    rounded: "{rounded.action}"
    padding: "16dp 20dp"
    height: "56dp"
  button-secondary-destructive:
    backgroundColor: "{colors.container-high}"
    textColor: "{colors.state-over}"
    typography: "{typography.mono-action}"
    rounded: "{rounded.action}"
  chrome-button:
    backgroundColor: "{colors.container-high}"
    textColor: "{colors.on-ground}"
    rounded: "{rounded.pill}"
    size: "44dp"
  fab:
    backgroundColor: "{colors.ember}"
    textColor: "{colors.pearl-ground}"
    rounded: "{rounded.group}"
    size: "56dp"
  text-action:
    textColor: "{colors.ember}"
    typography: "{typography.label-large}"
    height: "48dp"
  panel:
    backgroundColor: "{colors.container}"
    rounded: "{rounded.panel}"
    padding: "{spacing.panel-pad}"
  hero-panel:
    backgroundColor: "{colors.container}"
    rounded: "{rounded.hero}"
    padding: "{spacing.hero-pad}"
  panel-header:
    textColor: "{colors.on-ground}"
    typography: "{typography.label-large}"
    height: "40dp"
  icon-badge:
    textColor: "{colors.ember}"
    rounded: "{rounded.badge}"
    size: "32dp"
  stat-tile:
    backgroundColor: "{colors.container}"
    textColor: "{colors.on-ground}"
    typography: "{typography.headline-small}"
    rounded: "{rounded.tile}"
    padding: "{spacing.tile-pad}"
  stat-chip:
    textColor: "{colors.muted}"
    typography: "{typography.body-medium}"
    padding: "0dp 10dp 0dp 0dp"
  symbol-tile:
    textColor: "{colors.on-ground}"
    typography: "{typography.label-large}"
    rounded: "{rounded.action}"
    size: "44dp"
  glyph-badge:
    backgroundColor: "{colors.container-highest}"
    textColor: "{colors.on-ground}"
    rounded: "{rounded.action}"
    size: "44dp"
  group-row:
    backgroundColor: "{colors.container-high}"
    textColor: "{colors.on-ground}"
    typography: "{typography.body-large}"
    rounded: "{rounded.group}"
    padding: "14dp 18dp"
    height: "56dp"
  text-field:
    backgroundColor: "{colors.container-high}"
    textColor: "{colors.on-ground}"
    typography: "{typography.body-large}"
    rounded: "{rounded.group}"
    padding: "14dp 18dp"
    height: "56dp"
  choice-chip:
    textColor: "{colors.muted}"
    typography: "{typography.body-medium}"
    rounded: "{rounded.pill}"
    padding: "9dp 16dp"
    height: "48dp"
  choice-chip-selected:
    textColor: "{colors.on-ground}"
    typography: "{typography.body-medium}"
    rounded: "{rounded.pill}"
  segmented-track:
    backgroundColor: "{colors.container-lowest}"
    textColor: "{colors.muted}"
    typography: "{typography.body-large}"
    rounded: "{rounded.action}"
    height: "48dp"
  keypad-key:
    backgroundColor: "{colors.container-high}"
    textColor: "{colors.on-ground}"
    rounded: "{rounded.action}"
    height: "52dp"
  nav-bar:
    backgroundColor: "{colors.container-low}"
    textColor: "{colors.muted}"
    typography: "{typography.body-small}"
    height: "72dp"
  nav-item-selected:
    textColor: "{colors.ember}"
    rounded: "{rounded.group}"
    width: "60dp"
    height: "32dp"
  snackbar:
    backgroundColor: "{colors.container-highest}"
    textColor: "{colors.on-ground}"
---

# Design System: Tally

## Overview

**Creative North Star: "The Lit Ledger"**

Tally is a month kept as a ledger on a warm near-black desk, with one lamp over the figure that matters. The world is the sibling app Avex's, and so is its page: warm Pearl ground, serif figures, sans rows, mono uppercase labels, one owner-chosen accent, and open editorial composition (the owner's call, 2026-10-05: "I don't like these big boxes, look at what I did for Avex"). Nothing that cannot be tapped sits in a box; a section is its mono header and the air around it, a reading is a serif figure over a mono label, and the figure a screen exists for is its one serif hero. Tally keeps its richer money readings and charts, drawn open on the page.

Depth is tonal, not cast, and spent only on what can be tapped. Passive content sits straight on the ground, as in Avex: no panels, no boxes, sections separated by air and a mono header; tappable things (slab rows, chips, fields, buttons) take a warm rung up. Nothing in the app is a gradient. Colour is spent with intent: the accent marks what the owner acts on and where the month stands, the twelve category hues are a data series, and red and green appear only when something is really over or under. Readings travel with their numbers: a meter always carries its pace tick, a figure always carries the line that explains it.

Motion is short and physical. Fills draw in once over 900ms and never replay when a row scrolls back, a press is a 0.97 bounce rather than a ripple, and every animation collapses to a cut when the system's Remove animations is on.

**Key Characteristics:**
- Warm near-black Pearl ground, one flat colour; a pure-black variant for OLED.
- Three type voices: serif figures and titles, sans rows and prose, mono uppercase labels.
- One owner-chosen accent (Ember by default), with a near-white fallback when colour is off.
- Open editorial, Avex's: no box around anything that cannot be tapped; one serif hero figure per view.
- Mono uppercase section anchors; a glyph badge leads tappable list rows only.
- The pace tick: every meter shows where an even spend would be today.
- No dividers anywhere; air and the section's header separate sections.

## Colors

A warm, low-chroma ladder of browns under a single saturated accent, with a twelve-hue data series and two true-state colours.

### Primary
- **Ember** (ember): the default owner accent. Fills the hero action and the add FAB, colours meter fills, today's day bar, the selected nav glyph and its 16% indicator pill, selection rings with their 15% wash, text actions in panel headers, the text cursor, the keyboard focus ring, the heat grid's steps, and the gain on the cost-and-gain bar. The hero is lit by nothing but its serif figure: no wash, no edge.
- **Owner accents**: the Appearance picker offers Ember, a clear red (accent-red), and six of the category hues as accents (sage, teal, sky, iris, rose, and amber under the name Gold). Whichever is picked takes every Ember role above; nothing else changes.
- **Accent off**: the accent becomes the near-white text colour (on-ground), so selection, meters and the primary act still read in black and white (see home-mono).

### Secondary
- **Past-day rung**: the accent at 55% for days already behind today in the week's day bars; the scheme's secondary is the accent at 60%, which past periods take in the trend bars. Never a separate hue.
- **Before-period rung**: muted at 50% for the days of this week that fall before the budget period began. They belong to the last period, so they take no accent and no state colour.

### Tertiary
- **Under-budget green** (state-under): a true state only, for money in and for a reading that is genuinely under (and a holding's up arrow, a gain). As a mark it clears 3:1 on the ground; it is never text, so a share of what is held takes the accent, not green.

### Neutral
- **Pearl ground** (pearl-ground): the background: one flat colour behind everything, the same the window paints before the first frame.
- **Container ladder** (container-lowest, container-low, container, container-high, container-highest): the warm tonal steps. Lowest is the recessed segmented-control track; low is the nav bar; container is sheets and dialogs (sections and stat tiles sit on the ground itself); high is every tappable slab (rows, fields, keys, secondary actions, chrome capsules); highest is meter and bar tracks, glyph-badge fills and the snackbar.
- **Edges** (outline, outline-variant): outline-variant is the nav bar's 1dp top edge; outline is the switch border, the ring of a day still ahead in the heat grid and, at 35%, the idle ring of a choice. Sections have no edge.
- **Text** (on-ground, muted): on-ground for figures, titles and row text (16.7:1 on the ground); muted for subtitles, captions, labels and idle choices (9.6:1, and never below 0.65 alpha, where it still holds 4.65:1).
- **Pure black** (amoled-*): the OLED ground swaps the warm ladder for neutral near-blacks from amoled-ground up to amoled-container-highest, with amoled-outline-variant as the edge. The accent, the text pair and the hues are unchanged.

### State
- **Over-budget red** (state-over): over budget, an overrun's length on a meter (an over-contribution's on a room meter), the part of a day bar past its allowance, a shortfall below cost on the cost-and-gain bar, a holding's down arrow, an error edge on a field, a destructive secondary action, and the hero's label when the month is over. It clears 3:1 as a mark on the ground; it is never body text, and a debt or a month that fell is not "over", so neither wears it.

### Category hues
- **The twelve hues** (hue-sage through hue-brick): mid-tones that each clear 3:1 as a mark on the ground and stay distinct from one another. A category's glyph is drawn in its hue on a 15% wash of that hue; the same hue colours its segment in stacked bars, its ranked bar and its legend dot. Investments draws two more series from the same twelve: account kinds (TFSA sage, RESP teal, RRSP sky, FHSA iris, LIRA olive, non-registered sand, RRIF brick, other rose, none set slate) and kinds of security (ETFs orchid, stocks coral, crypto amber, funds olive, cash slate, bonds brick, other rose). Each series keeps its own legend.

### Named Rules
**The One Light Rule.** The accent is the owner's single colour and the only light in the room. It marks what can be acted on and where the month stands; no section is tinted by it, the hero included. There are no gradients: not on the ground, not on a section, not under a chart line, not in the launcher icon.

**The Data Series Rule.** Category hues are data, not decoration: they appear on a category's badge, a symbol tile's wash and in charts, never on text, never on chrome. Their order is stored, so a new hue is appended, never inserted or reordered.

**The True State Rule.** Red means over, green means in or under, and nothing else wears them. A state colour is a mark, never a paragraph.

**The Measured Contrast Rule.** Text clears 4.5:1 and marks clear 3:1 on the surface they sit on, and content on an accent fill is chosen by measuring both ends of the palette against it, never by guess. The build's contrast test holds every accent, both grounds and the accent-off fallback to this.

## Typography

**Display Font:** the platform serif (Noto Serif on Android), the same face the reference app Avex ships; pinned by the brief's "look at Avex for style reference"
**Body Font:** the platform sans (Roboto on Android)
**Label/Mono Font:** the platform monospace

**Character:** A ledger's three hands: an old-style serif for the number or title a block exists to show, a plain sans for everything you read in a row, and a spaced mono in capitals for the labels that name a reading. Choosing the voice is choosing the meaning.

### Hierarchy
- **Display** (400, 52sp / 44sp, 58 / 50 line): the hero figure, open on the page. It steps down through display-medium, headline-large and headline-medium as the text grows, and its font scale is capped at 1.3x so a long amount at 200% stays on one line.
- **Headline** (400, 36sp / 28sp / 22sp): headline-large is every page title; headline-medium is a secondary panel figure such as the net across accounts; headline-small is the value in a stat tile.
- **Title** (500, 18sp / 16sp / 14sp, tabular): keypad digits (title-large), amounts at a row's end (title-medium), compact readings (title-small), and the sans group header over a slab group (title-small in muted).
- **Body** (400, 16sp / 14sp / 12sp): body-large for row titles, field text, segment labels and the context line under a page title; body-medium for row subtitles and the line under a hero figure; body-small for chip text, captions, tile details, legend labels and nav labels.
- **Label** (mono, 13sp / 11sp / 10sp, 0.8 / 0.6 / 0.5sp tracking, uppercase): label-large for panel names, text actions and a field's currency suffix; label-medium for the stat-tile label under its value, panel-header readings and day-bar initials; label-small for chart axis ends and the heat grid's day numbers.
- **Action** (mono 700, 16sp, 1sp tracking, sentence case): the label on the hero and secondary actions ("Save expense").

### Named Rules
**The Three Voices Rule.** Serif for THE figure or title of a block, sans for rows and prose, mono for labels and text actions. Sizes come only from the type scale; a call site never sets its own size, and the build fails if one does.

**The Tabular Figures Rule.** Every serif and title style carries tabular digits, so a changing amount never shifts its neighbours.

**The Context Below Rule.** A page title stands alone at the top of its column with its context line under it in sans, joined with " · " (for example "18 days left · resets 1 Nov"). Nothing sits above a title.

**The Serif Is A Role Rule.** The serif voice is fixed by role and size, not by face. It resolves to the platform serif because the pinned reference, Avex, resolves to exactly that face and the two apps are meant to sit side by side on one phone. If the owner ever unpins Avex, a bundled serif replaces it without changing any size, weight or role here.

## Layout

One column on a phone, a 24dp gutter (gutter) on both sides, and a vertical rhythm of 28dp (panel-gap, `PANEL_GAP`) between every section on a page: open sections need the air a box would have given them. A tab's list leaves 96dp (fab-clearance) at its end so the add button never covers the last row.

A page opens with the top bar on the page itself, inset below the status bar at 16dp horizontal and 8dp vertical, at least 48dp tall: a back capsule at the start when there is one, chrome capsules at the end, and never the screen's name. The serif page title and its context line follow, then the hero, then pairs of stat tiles side by side with a 20dp gap (figure-gap, `FIGURE_GAP`) and equal height, then the sections in reading order. The entry screen pins its keypad and hero action to the bottom and scrolls the panels above.

A section's content sits straight on the gutter, as the hero's and a stat tile's do; a slab row keeps 18dp horizontal and 14dp vertical padding (row-pad). A label and its reading share one line until they cannot; then the reading drops beneath the label instead of breaking a word. A panel header holds its label to one line: the reading or text action drops beneath the name as soon as the name would wrap at all, so "IN AND OUT" never stacks word by word beside "DAY 14 OF 31". Rows and group headers, whose start is a sentence that may wrap, drop their reading only when a word would break. Every touch target is at least 48dp, rows at least 56dp, and every height grows with its text.

At 200% font, worded segmented controls past 1.3x turn into wrapping choice chips, compact choice rows stack their control under the label, rows of small figures stack into label-and-value lines, and a hero action paired with a secondary one stacks above it. Pairs of stat tiles stack into one column past 1.5x (past 1.3x on the Settings pages, whose labels run longer). User content always wraps; only the nav labels are held to one line.

At 600dp and wider the bottom bar becomes an 88dp rail at the start with the add button at its top, so a tablet never wears a stretched phone bar. The screen inside matches its window too, measured on the width the rail leaves it, so no reading sits a screen away from its label. Home keeps the phone column below 600dp; from 600dp the same column is capped at 640dp (reading-width, `READING_WIDTH`, gutters included) and centred; from 840dp it splits into two panes on the gutter with the 28dp panel gap between, under one title and one scroll: the hero, In and Out, and the week at the start in story order, then the envelopes, recent entries, bills and accounts at the end, the pair capped at twice the reading width. In the capped column and the panes the add button is always in the rail, so Home's list ends on the gutter and the system bar instead of the fab-clearance; the phone column keeps the fab-clearance. Investments takes the capped, centred column from 600dp too. Every other screen, the tabs and the pushed screens alike, still runs one full-width column at these widths; READING_WIDTH is the cap they take next.

Layout is mirrored for right-to-left: start and end only, and every chart draws from the reading edge.

### Named Rules
**The Air Not Rules Rule.** Sections are separated by space and their mono header, never by a divider, a hairline rule or an edge. The build fails on any divider.

**The Start And End Rule.** Padding, alignment and chart direction are written in start and end so the whole app mirrors for right-to-left; left and right never appear.

## Elevation & Depth

Depth is a tonal ladder, not a shadow system, and it is spent only on what can be tapped. Passive content (sections, the hero, stat tiles) sits on the ground itself; a tappable slab is a warm rung up (container-high) without an edge; tracks sit on the top rung (container-highest); sheets and dialogs land on container. The tonal tint Material would wash over every sheet is switched off, so sheets, menus and pickers land on the same warm ladder instead of an accent-tinted grey. The hero is told apart by its serif figure, not by light or height. The one element the app gives a shadow is the add FAB, at 2dp resting and 4dp pressed, because it floats over scrolling content; the snackbar keeps the 6dp shadow Material's snackbar casts.

### Shadow Vocabulary
- **FAB lift** (Material elevation 2dp, 4dp pressed): the add button only, floating over the tab content.

### Named Rules
**The Tonal Ladder Rule.** A tappable surface rises by stepping up the warm container ladder, never by casting a shadow or taking an edge. Only the FAB floats.

**The One Hero Rule.** A view has exactly one hero, for the figure it exists to show, in the serif display voice. When that figure is over (the month, a budget), its label turns over-budget red; nothing else about the hero changes.

## Shapes

Soft, generous corners that grow with the size of the thing: 4dp for the inner corners where slabs meet in a group (seam-inner), 10dp for segment thumbs (badge), 12dp for actions, glyph badges, symbol tiles, keypad keys and the segment track (action), 14dp for picker tiles (picker), 16dp for slab groups, fields, the FAB and the nav indicator (group), and full capsules for choice chips, pills and chrome buttons (pill). The tile, panel and hero rungs (18dp, 20dp, 24dp) are held in reserve: sections, stat tiles and the hero are open, with no corners to round. Grouped slabs round only their outer corners at 16dp and meet at 4dp across a 2dp seam, so a group reads as one rounded block cut into rows. Meter and bar ends are fully round; day bars round at up to 6dp; stacked-bar segments at 4dp with 2dp gaps.

### Named Rules
**The Seam Rule.** Members of a group, keypad keys included, are separated by a 2dp gap of ground, never a line; the group's outer corners are 16dp and its inner corners 4dp.

## Components

### Buttons
Tactile and few. A view has one hero action at most.
- **Shape:** gently rounded rectangles (12dp), at least 56dp tall and growing with their text.
- **Hero action:** the accent fill with the measured on-accent foreground, mono 700 label in sentence case, 20dp by 16dp padding. Disabled drops the fill to 35%.
- **Secondary action:** the same shape on the container-high slab with an on-ground mono label; destructive swaps the label to the over-budget red.
- **Press:** every tappable thing in the kit scales to 0.97 on a soft spring and back, with no ripple; under TalkBack, where a press is never seen, the ripple returns. The nav items, Material's FAB and its dialog text buttons keep Material's ripple.
- **Focus:** focus from a keyboard, a D-pad or a Chromebook draws a 2dp ring in the accent along the target's own shape (12dp corners when a target has none of its own), on its inner edge so a clip never cuts it. On the hero action's accent fill the ring takes the label's colour instead. A bare row inside a panel draws its ring 4dp outside itself, in the panel's padding, so it never crosses the badge (the Categories and Accounts lists still draw theirs inside). Touch never moves focus, so a finger never sees the ring.
- **Chrome button:** a 44dp container-high capsule drawn inside a 48dp target, holding a 22dp glyph (back, settings, month stepping).
- **Text action:** a mono lowercase label followed by an arrow ("insights →", "manage →"), padded to 48dp, in the accent inside panel headers.
- **Add FAB:** 56dp, 16dp corners, accent fill, a 28dp plus glyph, bottom end at 20dp and 16dp; on wide screens it heads the rail.

### Chips
- **Stat chip:** a quiet line of context under a figure ("$90 a day for 18 days", "Even pace $1,310 by today", "$1,200 in cash, not invested"): a 16dp muted glyph and body-medium muted text, 10dp clear at its end, no capsule and no fill, because it is not tappable. A chip never repeats the context line under the page title, nor a reading shown elsewhere on the page; a daily allowance always says over how many days.
- **Choice chip:** a capsule choice drawn at least 40dp tall in a 48dp slot, 16dp by 9dp padding. Idle: no fill, a 1dp ring of outline at 35%, muted body-medium text. Selected: the accent ring over a 15% accent wash, text in on-ground. Ring and wash cross-fade over 150ms. Only one of a set is picked, so a chip announces as a radio button and its row is a selectable group, letting TalkBack read its place in the set. A set the owner's data decides, as Investments' kind lens (All, then each kind held), is a wrapping row of chips that announce as tabs, never a segmented control of unknown width. A chip that acts rather than picks (a note suggestion) is a plain button and never shows as picked; worded segments that wrap into chips still announce as tabs.

### Cards / Containers
- **Panel:** an open section, Avex's: its content straight on the page, no fill and no edge, because a box is a promise of a tap. The air around it (28dp between sections) and its header are the only separator. Rows inside it are bare taps.
- **Panel header:** the section's mono anchor (15sp, uppercase, muted), with either a reading in label-medium muted or a text action at its end. No glyph tile, no rule. At least 32dp tall. The name keeps one line: when it and the end cannot share the width whole, the end drops under it. A true state (over budget, money in) colours the name.
- **Hero panel:** the screen's lead section, open like every other: its anchor, the display figure, one explaining line in body-medium muted, a pace meter and quiet context lines. It is told apart by its serif figure, never by a box or a wash. On the monthly budget it is also the tap into its own editor. Home's hero names its figure for what it is, the room left of the budget: LEFT TO SPEND (OVER BUDGET when over, LEFT FROM INCOME with no budget). Its line leads with the verdict in days ("4 days ahead of your money", "3 days of room in hand", "On pace") in on-ground, then the reading it came from in muted: one sentence, two voices of one colour pair.
- **Stat tile:** one open reading, Avex's figure: the value in headline-small serif straight on the page, the label in label-medium uppercase muted 2dp under it, an optional body-small detail 6dp below. Always in pairs of different readings, 20dp apart.
- **Symbol tile:** a security's mark, or an account kind's: its letters ("XEQT", "TFSA") in mono label-large on-ground on a 15% wash of its series hue, 12dp corners, at least 44dp (36dp on a denser row, label-medium there). A long symbol widens the tile, never cuts; its font scale stops at 1.3x so it stays a tile at 200%. The hue washes the tile and never colours the letters. A tile is a mark beside a name that says the same, so TalkBack skips it.

### Inputs / Fields
- **Style:** a filled container-high slab, 16dp corners, at least 56dp, 18dp by 14dp padding, led by a 44dp glyph badge; text and placeholder in body-large (placeholder muted); an optional mono suffix (currency) at the end.
- **Focus:** the accent text cursor.
- **Error:** a 1dp over-budget red edge on the slab, with the quiet error line under its group in body-small red.
- **Switch row:** the whole row toggles and announces On or Off; the switch is drawn, not a second target. Checked: accent track, ground-coloured thumb with an accent check. Unchecked: container-highest track, muted thumb, outline border. Disabled drops the row to 35%.
- **Sliding segments:** a recessed container-lowest track with 12dp corners and equal cells at least 48dp tall; one thumb inset 3dp with 10dp corners, a 1.5dp accent ring and 15% wash, gliding to the pick on a no-bounce spring. Labels in body-large, on-ground when picked, muted when not. Each cell announces as a tab of a selectable group.
- **Keypad:** container-high keys with 12dp corners, at least 52dp, on 2dp seams, digits in sans.

### Navigation
- **Bottom bar:** container-low, a 1dp outline-variant top edge, 72dp plus the system inset, three equal full-height tab targets: Home, Plan, Insights. The ledger is not a tab: History opens from Home's Recent panel ("view all"), as Avex reads its log, so the entries are never shown twice. Each item is a 24dp glyph over a body-small label; the picked one puts its glyph in the accent on a 60 by 32dp indicator pill (16dp corners, accent at 16%) and its label in on-ground semibold; idle items are muted. The pill fades in over 150ms. Each item announces as a tab.
- **Rail (600dp and up):** the same items, 72dp tall each, in an 88dp column under the FAB.
- **Tab change:** a 150ms cross-fade; each tab keeps its scroll. Pushed screens slide in by an eighth of the width over 320ms with a fade.
- **Investments:** a pushed screen, not a tab and not an Insights lens: a portfolio is not a month, and the three tabs are each read as one. Home's accounts fold the investment accounts into one Investments row that opens it; Insights' Worth rows and its Invested reading, the Accounts list and the Settings search open it too.
- **Settings:** Avex's grouped list: a search slab at the top that filters every page and setting by its words, then General, Money, Data, Relay on your PC and Reset groups of slab rows, each led by a neutral glyph badge, then About. Backup, Export, Import, Relay on your PC and About are pages of their own, each a page head over groups; Reset's acts confirm in a dialog.
- **Snackbar:** container-highest with on-ground text and an accent action, lifted clear of the bar and the FAB. Deletes offer Undo here instead of a confirm.

### Lists and rows
- **Slab group:** a sans group header in title-small muted, then slabs on 2dp seams with 16dp outer and 4dp inner corners, then at most one footer note in body-small.
- **Row:** at least 56dp; a 44dp glyph badge at the start (12dp corners; container-highest with an on-ground glyph, or a category's hue on a 15% wash of it), a body-large title over a body-medium muted subtitle, a reading at the end, and a muted chevron only when the row opens something. A row without an action is drawn passive: no press, no chevron.
- **Rows inside a section:** bare taps on the ground, at least 56dp, led by a glyph badge (44dp on entry, bill, category and account rows; 36dp on the denser envelope, goal, room, payout and Insights rows) or a symbol tile, never further boxes. Their focus ring stands 4dp outside them. A passive row (a holding, a payout) has no press and no chevron.

### The Pace Meter (signature)
The app's signature mark, on every budget, envelope and goal with a date. A fully rounded track in container-highest (14dp in a hero, 6dp in rows), the spent fill in the accent (a goal's own hue on a goal), and a 2dp on-ground tick standing at where an even spend would be today; fill past the tick is the reading. Past the budget the scale stretches to the spend, the budget's own edge becomes a notch in the track colour, and the overrun draws in the over-budget red, so "over" is a visible length. On a goal the tick stands where an even saving from its first contribution to its date would be today; a reached goal, and the Goals total, which spans many dates, carry none. The category and account editors reuse the track as an 8dp share bar, in the category's hue or, for an account, the accent (muted for one that owes), with no tick, because a share has no pace. Every meter carries a spoken description of its reading.

### The Room Meter (the pace meter as a tax room)
Investments' signature, the pace meter re-read for a registered account's room: one per TFSA, RRSP and FHSA held, 6dp in its row, 14dp in the room editor. The track is this year's room (the owner's CRA figure, never worked out by Tally), the fill is what went in over the room's window, and the tick stands where an even pace to use the room by its deadline would be today (31 December for a TFSA or an FHSA; for an RRSP, the 60th day of the next year, moved off a weekend, counted from the day after last year's). Past the room the meter stretches as a budget's does, the room becomes a notch, and the over-contribution draws as a red length, because it is a real penalty; the row's line says what it costs ("The CRA charges 1% a month on $300", or that an RRSP's first $2,000 is untaxed). With no room set the track draws empty, with no tick, and the row asks for the figure from CRA My Account. The whole row opens the room editor.

### Charts
- **Day bars:** the week as rounded bars on full-height container-highest tracks with 8dp gaps; past days in the accent at 55%, today in the full accent, days ahead as empty tracks; a day over its allowance keeps its own colour up to the allowance and draws only what went past it in red, 1.5dp clear of the line, so over is a length, as on the pace meter, never a whole bar; the daily allowance as a 1.5dp dashed line (6dp on, 5dp off) in muted at 60%; day initials in label-medium below, today's in on-ground. Early in a month the week can open in the last one: those leading days draw in muted at 50% with their initials at 65%, are never held red against this month's allowance, and the panel's line names what they spent ("$236 spent · $176 before 1 Oct · $94 a day budgeted") so the week and the month's Out add up.
- **Month line (Insights):** cumulative spend as a 2.5dp accent line over a flat 10% accent area, ending in a dot at today, the even pace as a dashed muted diagonal (50%), the budget as a quiet 1dp line at the top (muted at 35%), a 1dp outline-variant baseline, mono date ends in label-small, with legend dots below; tap or drag across it to scrub days, the picked day marked by a 1.5dp on-ground line and a ringed dot.
- **Trend bars (Insights):** one thin rounded bar per period, at most 18dp wide, for what went out: the running period in the accent, past ones in the secondary rung; an on-ground dot for what came in; the budget as the day bars' dashed guide. The picked period stands on a recessed container-high column with 12dp corners; a tap on a bar or its label picks it.
- **Heat grid (Insights calendar):** the period as 6dp-cornered squares under weekday initials, lit by the accent at 25, 45, 70 and 100% for more spent against the busiest day, container-high for a day with nothing. Today is ringed 1.5dp in on-ground; days ahead are bare 1dp outline rings and take no tap. Each square sits 2dp inside a full-column slot at least 48dp tall, and the slot is the tap.
- **Stacked bar:** shares of a whole as segments 10dp to 14dp tall (category hues for where the month went) with 2dp gaps and 4dp corners; a share under 1% still gets a 2dp sliver.
- **Worth bars (Insights worth):** six period closes as rounded bars from zero, each in a full-height slot with 10dp gaps; the period picked in the accent, earlier ones in the secondary rung at 55%, a close in debt in the over-budget red; month initials in label-small below, the picked one in on-ground. Bars start at zero so a small change reads small; the hero's line and the Month by month rows carry the exact moves.
- **Cost-and-gain bar (Investments):** the portfolio's hero bar, a stacked bar 12dp tall. In a gain the cost draws in the secondary rung and the gain in the full accent, so the margin is a length; below cost the bar is the cost, what it is worth fills it in the secondary rung and the shortfall draws red. Values recorded by hand close it in muted at 50%, so the bar adds up to the hero figure. Legend dots under it name each part with its amount. No tick: a share has no pace.
- **Month bars (Investments income):** twelve months of dividends and interest drawn as the day bars are, the month running in the full accent, past ones in the 55% rung, narrow month initials under them, no allowance line.
- **Allocation (Investments):** a stacked bar per split (by account kind, by kind of security) in that series' hues, each with legend rows of name, amount and whole percent; a sliver under 1% says "under 1%".
- **Thin bar:** a 4dp ranked-comparison bar on a container-high track: the accent for a share of what is held, muted for a share of what is owed.
- **Legend dot:** an 8dp hue dot and a body-small muted label.
- **Draw-in:** every fill draws in once from zero over 900ms on an even ease-out and is remembered, so a row that scrolls away and back shows its settled value. Each chart is announced by a sentence of its values.

### Named Rules
**The Glyph Leads Rule.** Every list row begins with its mark: a drawn glyph on its badge, or a symbol tile on Investments. Section headers and stat tiles carry no glyph; they lead with their mono label and serif figure. The glyphs come from one rounded icon library at one stroke, and no two icon keys share a glyph.

**The Slab Is A Tap Rule.** An edgeless container-high slab is always something you press. A panel or stat tile is a section of readings and is not itself a target, the monthly budget's hero being the single exception, because it opens its own editor.

**The Pace Tick Rule.** A meter without its pace tick is only half a reading. Wherever an even spend has a meaning, the tick is drawn.

**The Bounce Rule.** A press is a 0.97 bounce and a fill is drawn once; when the system's animation scale is zero, every spec collapses to a cut.

## Do's and Don'ts

### Do:
- **Do** set every section open on the ground, Avex's way: its mono header, its content on the 24dp gutter, 28dp of air to the next. No fill, no edge, no corners around anything that cannot be tapped.
- **Do** lead every row with its mark (a glyph badge, 44dp, 36dp on a denser row; or a symbol tile on Investments) and leave section headers and stat tiles bare.
- **Do** give each view exactly one hero for its main figure, in the serif display voice capped at 1.3x font scale, its label turning red when that figure is over.
- **Do** draw the pace tick on every meter where an even spend means something, and let an overrun show as a red length past a budget notch.
- **Do** take every colour from the scheme or the category palette and every size from the type scale.
- **Do** set titles and figures in serif, rows in sans, labels in spaced mono capitals, and join readings with " · ".
- **Do** hold text to 4.5:1 and marks to 3:1 on the surface they sit on, and keep muted text at 0.65 alpha or more.
- **Do** let every row and control grow with its text at 200% and let readings drop under their labels rather than break; a panel header's label keeps one line.
- **Do** match a screen to its window: from 600dp cap a single column at READING_WIDTH (640dp) and centre it, as Home does, and from 840dp give Home its two panes.
- **Do** write layout in start and end, and draw charts from the reading edge in right-to-left.
- **Do** make a press a 0.97 bounce and make every animation collapse to a cut when animations are removed.
- **Do** draw keyboard and D-pad focus as a 2dp accent ring along the target's own shape, and announce a one-of-many choice as a radio button in a selectable group.

### Don't:
- **Don't** draw a divider, a hairline rule or an edge between sections; air and the mono header do that work.
- **Don't** use a Material top app bar or name the screen in the top bar; the serif page title names it.
- **Don't** put a label above a page title or a heading; context goes under it.
- **Don't** set a font size or a raw colour at a call site.
- **Don't** box a section, or the interactive rows inside one.
- **Don't** put a category hue on text or chrome, or reorder the hues.
- **Don't** use red or green for anything but a true over, under or money-in state (on Investments: a shortfall below cost, an over-contribution, a holding's arrow), and never for text, figures included.
- **Don't** cast shadows from panels, tiles or slabs; only the FAB lifts.
- **Don't** wash the accent over sheets and menus; tonal depth is the warm ladder.
- **Don't** cut user content to one line.
- **Don't** use left or right in layout.
