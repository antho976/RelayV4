---
version: 1
slug: "invest"
primary_target: "app/src/main/java/com/tally/app/ui/invest/InvestScreen.kt"
related_targets: ["app/src/main/java/com/tally/app/ui/invest/InvestParts.kt", "app/src/main/java/com/tally/app/ui/invest/InvestLogic.kt", "app/src/main/java/com/tally/app/ui/invest/RoomEditScreen.kt", "app/src/main/java/com/tally/app/ui/settings/ImportScreen.kt", "app/src/main/java/com/tally/app/ui/home/HomeScreen.kt", "app/src/main/java/com/tally/app/ui/insights/InsightsScreen.kt"]
---

# Surface: Investments (Tally)

Scope: the Investments page, pushed from Home's accounts (one Investments row), Insights' Worth
(its investment rows and the Invested reading's "portfolio →"), Accounts ("investments →") and
the Settings search; the room editor it opens; and the import of Wealthsimple's investment files,
which shares the import page. Visitor mode: **Operate (review)**. Audience: one person, a few times
a month, at home. Task: know what the portfolio is worth against what the holdings cost, whether
this year's registered room is being used on pace, bring the numbers up to date, leave.
Constraints: offline; no live quotes on the phone (figures arrive by import or from the paired
PC, so every figure carries its "as of" day); one ledger currency; Avex's open world inherited;
the numbers are core's `Invest.portfolio`, the same reading the PC makes (`docs/INVESTMENTS.md`).

Structural candidates, ordered by resonance (dealt by hand, no seed): 1 cost-and-worth ledger with
the room meters · 2 holdings-first ticker list · 3 room-first tax tracker · 4 per-account envelope
stack · 5 performance line first · 6 allocation donut first. 1 leads. 3 is its signature section,
2 its Holdings section, 4 its Accounts section and its kind lens. 5 waits for prices that arrive
often enough to draw a line honestly. 6 is refused: a donut is the category default, and a stacked
bar reads shares better.

## Direction contract

THESIS: A portfolio is the same two-sided ledger as the month: what the holdings cost against what
they are worth, with the margin between them drawn, not stated. And a tax room is a budget you are
trying to fill, read as a pace. Refuses the brokerage default of a ticker wall in red and green
under a day-change percentage the phone cannot know.

OWN-WORLD: Home's world unchanged: Pearl ground, serif figures, sans rows, mono uppercase anchors,
open sections with no boxes and 28dp of air between them, one serif hero, the owner's accent as
the only light. Symbols are mono letters on a 15% hue wash (the hue is the kind of security, a data
series); account kinds (TFSA, RRSP, FHSA, non-registered...) are a second series on the same
tiles. Red and green appear only as marks: a shortfall below cost, an over-contribution, a
holding's arrow; never as text, figures included, and every figure carries its own sign.

STORY: The owner opens Investments, reads the portfolio's worth and how far above its cost it
stands, checks this year's TFSA, RRSP and FHSA room against the pace tick ("$934 a month uses it by
31 Dec"), glances at the holdings and what they paid, sees how old the numbers are, imports a fresh
Wealthsimple CSV or lets the PC bring it, and leaves.

FIRST VIEWPORT: top bar with a back capsule and one import capsule at the end. Serif title
"Investments", context "3 accounts · valued 9 Oct" ("last valued 12 Aug" once a month old). The
kind lens (All · TFSA · RRSP · FHSA) as a wrapping row of chips, only when two or more kinds are
held. The hero: PORTFOLIO anchor (the kind's name under a lens) with "AS OF 9 OCT", the worth as
the display figure, "$3,240 above what the holdings cost", the cost-and-gain bar with its legend,
chips only for what the bar cannot say (cash not invested, holdings converted without the day's
rate, holdings with no price), and, when stale, "40 days since the last report" with "update →".
Under it RETURN (on cost, said not to be time-weighted) and PUT IN (this year, of the room) tiles
side by side. Then the ROOM section's anchor and its first meter.

FORM: cost-and-worth ledger, candidate 1 of 6. Signature move: the room meter, Home's pace meter
re-read as a tax room: the track is this year's room, the fill is what went in, the tick is where
an even pace to use it by its deadline stands today, and an over-contribution is a red length past
the room's notch, because it is a real penalty. Signature interaction: the kind lens re-reads the
whole page (hero, figures, room, holdings, payouts, accounts) for one account kind. Motion grammar:
Home's: fills draw in once (900ms DrawDecelerate), press = 0.97 bounce, all of it collapses under
Remove animations. Nothing ticks or counts up.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## As built (2026-10-09)

Sections, in order, each an open section on the 24dp gutter, the column capped at 640dp and centred
from 600dp:

1. **Head**: `TopBar` (back, import capsule) and `PageTitle` with the freshness context.
2. **Kind lens**: `ChoiceChip`s announcing as tabs. Chips, not `SlidingSegments`: the kinds are the
   owner's data and can run to five or more, and "Non-registered" does not fit a sixth of a phone.
3. **Hero**: `HeroNumber` (capped at 1.3x), the cost line, `CostGainBar` (cost in the secondary
   rung, gain in the accent; below cost, a red shortfall; values recorded by hand close the bar in
   muted so it adds up to the figure). Accounts with no holdings on record read from their balance
   (the value recorded by hand, or what went in), and with none at all the bar gives way to "Import
   a holdings report to see the cost and the gain".
4. **Figures**: RETURN and PUT IN (CASH when no registered account is held).
5. **Room**: one `RoomRow` per TFSA, RRSP and FHSA held, each opening the room editor; a caption says
   what the figure is and what the tick means.
6. **Holdings**: eight `HoldingRow`s by value (symbol tile, name, units · kind · weight, value, a
   signed gain with its arrow as the mark), "show N more →" for the rest. Passive in this version.
7. **Allocation**: by account kind (with every account read) and by kind of security, each a
   stacked bar with legend rows.
8. **Income**: dividends, interest and reinvested distributions; twelve months as bars drawn the
   way the week's day bars are, then the four latest payouts. Under a lens the months are hidden,
   since core reads them for every account together, and the payouts are the kind's.
9. **Accounts**: one row per investment account (kind tile, name, kind · valued day · holdings ·
   money-weighted return), each opening its editor to record a value or set its kind.
10. **Bring it up to date**: a slab group: import a Wealthsimple file, bring them in on the PC
    (opens Relay's pairing page), add an account by hand.

States: **loading** draws only the head; **zero** shows $0 over an empty bar with the hero action
"Import a Wealthsimple file" and "Add an account by hand", then "How to get the file" in three
numbered steps; **values only** shows the worth with no bar; **stale** says how old with "update →";
**error** (the portfolio could not be added up) says so plainly and keeps the ways to bring it up to
date.

Room editor (`Routes.ROOM_EDIT`): the budget editor's shape. The typed room as the one figure,
what went in this year read against it on a 14dp room meter, "$X a month uses it by <deadline>",
a quick fill from the year's limit (TFSA and FHSA only: an RRSP's dollar limit is a ceiling, not
anyone's room), where CRA keeps the real figure, then the keypad and "Save room"; the top bar's
delete removes the figure with Undo.

Import: a Wealthsimple investment file shared to Tally or chosen on the import page never reaches
the bank planner (where a TFSA statement once became income and spending). Its preview shows the
file and its day, "Ready to import" with the count, one group per account in the file mapping it to
an investment account already here (by its number, else by a name that says its kind) or "A new
account", or, for a monthly statement, the account it belongs to; the lines left out and why; then
"Import 12 holdings". Done offers "See your portfolio". The import page lists Wealthsimple's two
files with where they live; opened from Investments, they lead the page.

Not built yet, on purpose: the month's move chip ("Up $412 since 1 Oct", which needs a value at the
month's start the portfolio does not give), a holding's sheet with its payouts, the FHSA's lifetime
chip, an INVEST goal chip on the room, and target allocation ticks.
