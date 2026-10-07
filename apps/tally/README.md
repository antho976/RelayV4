# Tally

An offline spending and budget tracker for Android. You log what you spend in two seconds, and the
app answers one question at all times: how much room is left this month, and is the month on pace.

Kotlin, Jetpack Compose and Material 3, Room over SQLite, Hilt. No account, no servers, no cloud:
nothing leaves your own devices. The one connection it ever opens is to your own PC, if you pair
Tally with Relay there. Visual language inherited from Avex.

## Install a test build

Every CI run on this branch uploads an artifact called **tally-apks** (Actions tab, open the run,
scroll to Artifacts). It holds:

- `app-release.apk`: the minified build, signed with a throwaway debug key. Use this one to judge
  speed and feel.
- `app-debug.apk`: installs beside it as "Tally" with the `.debug` id, with LeakCanary watching
  for leaks.

Unzip, copy to the phone, open it (allow installing from your file manager once). A tagged release
(`tally-v*`) also publishes the APK on the Releases page.

To try the app with something in it: on the first screen choose **try it with sample data**, or
later go to Settings, Reset, Load sample data. Sample accounts are named "Sample ..." so
they can never pass for real data; Erase everything removes them.

## What it does

- **Home**: what is left to spend this month drawn as the budget meter with today's pace tick,
  what each remaining day can take, money in and out, the week against the daily budget, the
  envelopes furthest over pace, goals, recent entries by day, upcoming bills, account balances.
  **History** (Recent's "view all"): every entry by day, month by month, with search across all
  time and filters. On a tablet Home splits into two panes.
- **Add entry** (the + button, from every tab): keypad, category, account, date, note with
  suggestions, transfers between accounts, optional repeat; quick add from what you log most,
  save and add another, and what the month has left after it.
- **Plan**: the monthly budget and per-category budgets with pace, a month-end projection and
  budgets suggested from your three-month averages; bills and subscriptions that post themselves
  on their due date; goals by kind: save toward a target, reach a balance by a date, invest a
  share of income each month, or save an amount each month.
- **Insights**: the month line (drag to scrub any day against the pace), where the money went,
  largest expenses, top payees, six-month trend, a calendar heat grid, and Worth: net worth over
  six months, held against owed, the share of income kept and what went into investments.
- **Import from your bank**: Desjardins (AccèsD) and Wealthsimple CSV statements, or any bank's
  CSV. Read on the phone, checked for lines already logged, card payments kept apart, payees
  filed the way you filed them before. Share a CSV to Tally to open it straight in the import.
- **Settings**, searchable, in Avex's grouped style: accent colour and pure black, currency,
  payday budget month, week start, accounts (with Desjardins and Wealthsimple quick starts, and
  recorded values for investment accounts), categories (over a hundred icons, suggested from
  the name), Backup (weekly automatic copies, a folder of your choice, restore), Export (CSV and
  the backup file), Relay on your PC, sample data, erase.
- **Relay on your PC** (Settings): pair with Relay on your computer by pasting the
  `relay://pair` link that `relay remote pair` prints, or by its address and code. The phone and
  the PC then hold the same ledger and work as one: Tally syncs when it opens, a few seconds after
  a change, and every half hour while there is a network. Newest edit wins per row; the first
  sync makes the PC's ledger a copy of the phone's. The wire contract is `docs/MONEY.md` ("Sync")
  at the repository root.

## Build

```
./gradlew :core:test                 # the money arithmetic, plain JVM, seconds
./gradlew :app:testDebugUnitTest     # Room, repositories, doctrine, screenshots (Robolectric)
./gradlew :app:verifyRoborazziDebug  # the same, comparing screenshot goldens
./gradlew :app:assembleDebug
```

Needs JDK 21 and the Android SDK with `platforms;android-37.0`.

## CI

`.github/workflows/tally-ci.yml` at the RelayV4 root, on every push or pull request that touches
`apps/tally` (every `run` step starts in `apps/tally`):

| Job | What it proves |
|---|---|
| Guard | wrapper checksum, no keystores or build output committed, network code only in the PC sync (`data/sync`) |
| Core | money formatting, budget periods, pace, recurrence, CSV and backup codecs, sample data, voice rules |
| Verify | Room DAO and repository tests on SQLite, the version 2 to 3 migration, sync merge rules and the PC sync against a fake Relay door, recurring posting, backup round trip, design doctrine, theme contrast, screenshot goldens at 100% and 200% font, Android Lint, R8 release, merged manifest asks only for the expected permissions, APK size budget |
| Instrumented | the real UI flow on an emulator with LeakCanary failing any test that leaks, device SQLite, and a cold launch of the minified APK |

`tally-record-screenshots.yml` re-records the goldens and the Room schema on demand;
`tally-release.yml` turns a `tally-v*` tag into a GitHub Release (signed with the real key when the `TALLY_*` secrets are
set, otherwise a debug-signed pre-release).
