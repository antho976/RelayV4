# Money in Relay

Relay gains a second, casual space beside the dev workspace. Money management comes first: Tally,
the budget and spending tracker, moves into this repository and the PC gets the same ledger.
Investing and AI-planned trades come later and are not designed here.

Decided with Antho on 2026-10-07:

- **Two apps, one repository.** Tally stays its own Android app in `apps/tally` (Kotlin, Compose,
  Room). `apps/relay-mobile` stays the PC remote. Folding them into one phone app is a later idea,
  not this plan.
- **Both devices hold the whole ledger and work as one.** The phone is where nearly everything is
  logged; the PC is used rarely but must never be read-only. Either can add or change anything
  offline, and they catch each other up when they reach each other.
- **The start screen is a select screen,** not a chat: a greeting for the time of day, then a card
  per space with one live line each (Money: "$412 left · 3 days ahead"; Dev: agents and reviews).
- **The switcher sits at the top left of the top bar,** where the brand is now: Dev | Money.

## Steps

1. **Tally in the repository** (done). `apps/tally`, squashed from
   [antho976/Tally](https://github.com/antho976/Tally) at `cfae7c3`; its full history stays there.
   Its workflows run from the root as `.github/workflows/tally-*.yml`, only for changes under
   `apps/tally`; releases are `tally-v*` tags.
2. **The money rules in Rust** (`crates/relay-money`). A module-for-module port of
   `apps/tally/core` with its tests: amounts, periods, pace, recurrence, payees, CSV and bank
   statements, the backup file. A rule that changes on one side changes on the other in the same
   commit; the shared tests are what prove they agree.
3. **A ledger that can sync.** On both sides: every row gets a permanent id (a UUID, not Room's
   1, 2, 3, which collide across devices), a change time, and deletes become tombstones. A change
   log records each edit. Tally needs a Room migration; the engine gets `store::MIGRATIONS` entries
   and `money.*` bus ops.
4. **The desktop Money space.** The space switcher, the start screen, and Money pages that mirror
   Tally's Home, Transactions, Plan and Insights, drawn with Tally's tokens (`apps/tally/DESIGN.md`)
   as Relay's second theme.
5. **Sync.** Tally pairs with the PC the way `relay-mobile` does (`crates/relay-remote`, LAN or
   Tailscale; no cloud) and exchanges changes since the last sync. Newest edit wins per row; for one
   person, conflicts are rare. Totals are never synced: balances and pace are recomputed from the
   entries, so equal entries always mean equal numbers. Tally gains the INTERNET permission for
   this, and its promise becomes "nothing leaves your own devices".
6. **Warnings.** Over pace, a bill due soon, a low balance, an unusual expense: Android
   notifications on the phone (WorkManager is already there), Relay's notification center on the
   PC.

## Duplicates sync must not create

- **Recurring bills.** Both devices post rent on its due date. The posted entry's id is derived
  from the bill and the date, so the two copies are the same row and merge.
- **Bank imports.** An imported line's id is derived from the statement line, so importing the
  same CSV on both devices adds it once.
