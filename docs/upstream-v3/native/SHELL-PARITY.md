# Native shell parity with Relay 2

What the GTK shell shows today, captured on this machine's display. The Dashboard, Board, Code
and Agents rasters are against the live `dev` engine — real holds, real tasks, real provider
usage — so the numbers in them are the engine's, not fixtures. Every raster was taken with
`spectacle -b -n -a` on the real desktop; none is a mock-up.

Source of truth: the running Relay 2 window and `~/dev/Relay-2/apps/relay-app/src`.

## Captured

| Surface | Raster | What it shows |
| --- | --- | --- |
| Agents + Files | [`files.png`](artifacts/files.png) | The wall's Files rail expanded beside it, the sidebar hierarchy, and the status strip's provider meters and resource readout |
| Dashboard | [`dashboard.png`](artifacts/dashboard.png) | One `dashboard.get`: metric row, decision queue, resource pulse, project pulse, agents, activity |
| Board | [`board.png`](artifacts/board.png) | Five columns, type markers, priority chips, id/size/label meta, copy and open keys |
| Code | [`code.png`](artifacts/code.png) | File tree with git badges, the editor under Relay's own style scheme, Git as one top-right tool |
| Notes | [`notes.png`](artifacts/notes.png) | Note list with pins, the draft editor, dirty state, word and line count |
| Skills | [`skills.png`](artifacts/skills.png) | Installed skills grouped by repository, per-project enable, install from a GitHub URL |
| Settings | [`settings.png`](artifacts/settings.png) | Relay 2's six sections over dotted `settings.*` paths |
| Command palette | [`palette.png`](artifacts/palette.png) | Every page, New session, the sidebar, reconnect and the wall layouts, with their shortcuts |
| Board | [`board.png`](artifacts/board.png) | Re-captured: the `Board \| Modules` switch, the Lens key, Undo, and the key hint bar |
| Modules | [`modules.png`](artifacts/modules.png) | Completion ring, header stats, one row per module with its counts and progress |
| Notes | [`notes.png`](artifacts/notes.png) | Re-captured: the nine markdown keys, find, timestamp, wrap and the text-size stepper, and the status line |
| Device control | [`devices.png`](artifacts/devices.png) | Run and Release tabs, virtual devices, target, build-from, and the door note |

The Notes, Skills, Settings and command-palette rasters predate the glyph-ink repair and the
status-strip trim, so they show correct content in the older chrome; the four re-captured above
show the current shell.

## Built, but not yet seen working

Compiled, tested and clippy-clean, and reachable from a surface that was captured — but the
state itself was never on screen, because driving it needs a pointer or a keystroke and these
captures are driven by actions over D-Bus. Not claimed as observed:

- The lens bar expanded, its seven axes and grouping, and the board keys (`F`, `/`, `G`, `[`,
  `]`, the arrows). The Lens key, Undo and the hint bar that names them are in `board.png`.
- Drag-to-nest, and drag-to-column.
- The Notes find bar open, and what the markdown keys write.
- First run and the registry sheet. The engine's only instance names are `dev` and `test`
  (`relay: bad --instance "probe"`), and both already have projects registered, so the empty
  state that opens it could not be produced without disturbing one of them. The `+` keys in the
  sidebar open the same sheet: click one to see it.

## Not portable to this client

- **Screen mirroring** (`device.mirror.start`) and **creating a Relay-owned signing key**
  (`device.signing.create`) are `Doors::TauriOnly`; the engine returns `bus.door` for them on the
  socket the native client uses (D224). The device panel says so where they would be.

## Not yet ported

- The Agents wall's `focus` and `review` layouts are column counts here (one column, and two)
  rather than Relay 2's shapes: `focus` is a tab strip over a single cell, and `review` is a
  1.4fr main pane beside a 0.8fr side stack. The wall also has no drag-between-columns and no
  `RunPane` device strip beneath it.
- The board's drag rail — the "Drop to set" row of priority, size and type targets that appears
  while a card is airborne.

## How to reproduce

    ./target/debug/relay serve --instance test &
    RELAY_INSTANCE=test cargo run -p relay-native -- --page board

`--page <id>` opens straight onto a destination. The window's actions are also reachable over
D-Bus while it runs, which is how these captures were driven without a pointer:

    gdbus call --session --dest com.quietsoftware.relay.dev \
      --object-path /com/quietsoftware/relay/dev \
      --method org.gtk.Actions.Activate page-dashboard '[]' '{}'
