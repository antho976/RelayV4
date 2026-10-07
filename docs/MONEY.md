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

## The ledger on the PC

The engine keeps the ledger in its own SQLite file, `money.db` beside `store.db`, owned by
`relay_money::ledger`. It is apart from the dev store on purpose: money has its own schema
versions, its own backups, and is what the phone will sync with. Every row has a local integer
`id`, a permanent `uid` (UUID text) that sync will use, `updated_at` (ms) and `deleted` (0/1).

Amounts are minor units (`i64`). Dates are `YYYY-MM-DD`. Enum values are Tally's names
(`EXPENSE`, `CHEQUING`, `OVER_PACE`). Every mutation emits `money.changed`.

### Ops (phase 12, scope global)

`money.summary {today?}` → the Home reading:

```
{ currency, fraction_digits, month_start_day, empty,
  period: { start, end_exclusive, days, days_left },
  pace: { budget, spent, expected, remaining, pace_delta, days_left, daily_allowance,
          spent_fraction, pace_fraction, status },
  income, spent,                       // this period, transfers excluded
  lines: { margin, pace, versus_last },  // Tally's sentences, from relay_money::copy
  budgets: [ { category_id, name, icon, color, budget, spent, status } ],   // per category
  accounts: [ { id, name, type, balance } ], net_worth,
  bills: [ { id, name, amount, type, next_date, days_until, due_line, auto_post } ], // next 30 days
  goals: [ { id, name, kind, target, saved, line } ],
  recent: [ Tx ] }                      // last 8
```

`Tx` = `{ id, uid, type, amount, date, account_id, account, to_account_id, to_account,
category_id, category, icon, color, note }`.

| op | payload | result |
|---|---|---|
| `money.summary` | `{ today? }` | above |
| `money.lists` | `{}` | `{ currency, fraction_digits, accounts: [{id,name,type,balance,archived}], categories: [{id,name,kind,icon,color,archived}] }` |
| `money.tx.list` | `{ period_offset?, query?, account_id?, category_id?, limit? }` | `{ period, transactions: [Tx], income, spent }` |
| `money.tx.add` | `{ type, amount, date, account_id, to_account_id?, category_id?, note? }` | `Tx` |
| `money.tx.update` | `{ id, type?, amount?, date?, account_id?, to_account_id?, category_id?, note? }` | `Tx` |
| `money.tx.delete` | `{ id }` | `{}` |
| `money.budget.set` | `{ category_id?, amount }` (no category: the overall budget; 0 removes) | `{}` |
| `money.import` | `{ path }`: a Tally backup (`.json`) replaces the ledger, as a restore does on the phone | `{ kind, transactions, accounts }` |
| `money.export` | `{ path }` | `{ path, transactions }` |
| `money.sample` | `{}`: Tally's sample household, only into an empty ledger | `{ transactions }` |
| `money.reset` | `{}`: erase everything | `{}` |

Clients format amounts themselves with `relay_money::money::MoneyFormatter`
(`currency` from the result, `Locale::from_env()`).
