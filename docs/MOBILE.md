# Relay from a phone

The Relay mobile app (`apps/relay-mobile`, "Relay" on the phone) reaches the engine on this
machine through `crates/relay-remote`: the same bus lines the desktop client and the CLI speak
(`BUS.md` §6.2), carried over a WebSocket to a phone that paired once.

Two routes, tried in privacy order by the phone:

| route | when | what travels where |
| --- | --- | --- |
| **direct** | phone and PC on the same network, or the same Tailscale tailnet | LAN only (plain `ws://`, readable on the network), or Tailscale's end-to-end encrypted tunnel |
| **via a rendezvous** | phone anywhere | PC dials *out* to a server you host; the phone joins there; the server copies lines between the two and can read them (§6) |

Both end in `bridge.rs` on the PC, which checks the phone's credential and forwards its
requests to the socket door as actor `user` — the same door, the same guardrails and audit
rows as the CLI, but not the whole bus: a phone may call only the ops the app uses
(`PHONE_OPS` in `crates/relay-remote/src/wire.rs`, plus `bus.unsubscribe`). Anything else,
`settings.set` or `app.quit` for example, is refused at the door with `refused` /
`remote.op` and never reaches the engine; run it on the PC. There is no third route:
Tailscale, if you use it, only carries the direct link, and sees encrypted packets, not bus lines.

## 1. Tonight, in three steps

On the PC, from the RelayV4 checkout, `./run.sh` as usual. The engine it starts carries the
phone door (`relay serve --remote`), so there is nothing else to keep running. Then:

```fish
./target/debug/relay remote pair              # prints the QR code (finds the dev engine on its own)
```

Leave it running: when the phone presents the code, it asks `Pair "<phone>" (…) with this PC?
[y/N]`, and only a yes pairs it. The phone waits up to 90 seconds for that answer; the PC gives
up after 75, and the phone then says nobody approved it (`pair.unconfirmed`). A no tells the
phone the PC declined (`pair.declined`). Either way the code is spent: run `relay remote pair`
again. A phone that gives up sooner withdraws its request, so a late yes pairs nothing.

On the phone: install the app (the `relay-mobile-apk` artifact of the **Mobile APK** workflow,
run from the Actions tab, or `npm run android` in `apps/relay-mobile` with the Android SDK),
open it, tap **Pair a PC**, scan. If the engine was running from before the
door existed, restart it once (`./target/debug/relay --instance dev cmd app.quit '{}'`, then
`./run.sh`), or run `./target/debug/relay remote serve --pair` alongside it. The quit is refused
while agents are live (`app.sessions_live`); `app.quit '{"force":true}'` stops them, and their
sessions come back as restorable. For the phone to
reach the PC when you are out, see §3 (Tailscale) or §4 (your own server).

## 2. On the PC

With an engine running (`relay serve`, or the desktop app):

```fish
relay remote serve --pair       # front the engine for phones, open a pairing window, print the QR
```

Or both in one process, which is what a login service wants:

```fish
relay serve --remote            # engine + phone door; `deploy/relay-remote.service` runs this
```

The unit runs the checkout's `target/debug/relay` on the `dev` instance, the engine `./run.sh`
and the desktop app use; edit its `ExecStart` path to your checkout before installing it.

The door listens on `0.0.0.0:7420` for phones on the network. Until a phone is paired or a
pairing window is open it answers nobody: connections are closed unread, `GET /info` included.
A pairing window lasts ten minutes and admits one phone; to pair another later,
`relay remote pair` in a second terminal. Opening a window replaces any older code still open,
and a code answers one phone at a time. Each pairing is confirmed at the terminal that opened
the window (`--no-confirm` on `pair` or `serve --pair` lets the first phone with the code pair
without asking; without a terminal, `pair` refuses unless it is given). Every new pairing is
logged at warn in the engine log, naming the phone, and shown as a toast in the desktop app if
it is open (`ui.toast`). Nothing lasting lands in the notification centre yet: no bus op lets a
client create a notification, so that needs one in the engine.

Every connection has ten seconds to finish its WebSocket upgrade and thirty more to say hello.
The door holds at most 64 connections, of which at most 16 may be unproven and at most 4 from
any one address, so a stranger on the network cannot exhaust the engine's descriptors or lock
a paired phone out. A browser page on another site cannot open it (the `Origin` must match).

| command | does |
| --- | --- |
| `relay remote serve --bind 192.168.1.20:7420` | listen on one address only |
| `relay remote devices` | list paired phones |
| `relay remote revoke <id>` | forget a phone; a connection it has open closes within seconds and its next one is refused (once no phone is left, the door goes quiet, so that phone sees the PC as unreachable rather than "no longer paired") |
| `relay remote name "antho desktop"` | the name phones show |
| `RELAY_INSTANCE=dev relay remote serve` | serve the dev engine instead of stable |

State lives in `~/.local/share/relay-v4/<instance>/remote.json`, mode 0600. It holds each
phone's token and the rendezvous room secret (§4), so treat it like a private key.

## 3. From anywhere without a server: Tailscale

The simplest way to reach the PC away from home needs no server at all. Install
[Tailscale](https://tailscale.com) on the PC and on the phone and sign both in to the same
account. The PC then has a `100.x.y.z` address that the phone can reach from any network, and
the direct door already listens on it: `relay remote pair` prints it on a `Tailscale:` line and
puts it in the pairing link, so a phone paired after Tailscale was set up just works.

For a phone paired before that, open **Paired PCs**, tap **Add Tailscale or other address** on
the PC's card, and enter the PC's Tailscale address (`100.x.y.z`, or its MagicDNS name
`my-pc.tail1234.ts.net`). Added addresses are tried with the WiFi ones, survive re-pairing, and
the home screen's card says **Tailscale · private** when that is the link in use. Traffic travels
over Tailscale's encrypted WireGuard tunnel; nothing on the PC listens to the internet.

## 4. From anywhere: your own rendezvous

The PC never listens to the internet. Instead it keeps one outbound WebSocket open to a small
server you run, and phones join through it — the shape of a remote-control service, with the
service being yours.

On any machine with a public address (a VPS, a Raspberry Pi behind a port forward, a Tailscale
node):

```fish
relay remote rendezvous --bind 0.0.0.0:7430
```

`deploy/rendezvous.Dockerfile` builds it as a container and `deploy/relay-rendezvous.service`
runs the binary under systemd; both bind loopback and expect a TLS proxy in front. The PC
pings the server every 30 s and redials after 90 s of silence, so a dropped NAT flow does not
leave phones told "offline" for long.

Put TLS in front of it. The phone sends its pairing code and, once, receives its token over
this link; `wss://` keeps that from anyone *between* the phone and the server, but TLS ends at
the proxy, so the rendezvous process itself sees the token and every line (§6). A Caddyfile is
enough:

```
relay.example.org {
    reverse_proxy 127.0.0.1:7430
}
```

Then on the PC:

```fish
relay remote via wss://relay.example.org
relay --instance dev cmd app.quit '{"force":true}'    # then ./run.sh
relay remote pair
```

`via` mints a room secret and stores it. The PC presents it to the server as
`Authorization: Bearer <secret>`, so it stays out of URLs and proxy access logs; a server
that predates the header refuses that, and the PC dials it once more with `?secret=` in the
URL (and logs a warning to update the server). The process that carries the phone door
dials the server only when it starts, so restart it: for the `./run.sh` engine (`relay serve --remote`),
quit it and run `./run.sh` again. `force` stops live agents, whose sessions come back as
restorable; without it the quit is refused while any is live. (`via` says to restart `relay
remote serve`; that applies only when a separate `relay remote serve` carries the door. It
cannot run beside a `serve --remote` engine, which already holds the port.) Every pairing link
from then on carries the join address as well as the LAN ones. The phone tries the LAN first
and the server second; a PC card in the app can pin either. `relay remote via off` clears it.

The rendezvous keeps nothing on disk. A room is named by the digest of its secret, so only the
PC that holds the secret can host it, and a restart forgets nothing worth keeping.
`GET /health` reports how many hosts and lanes are up.

The server never waits on one phone. Each phone may have 8 MiB queued toward it; a phone that
drops off a mobile link without closing fills that and is cut off (it reconnects), and a phone
silent for 90 s is closed, so it cannot stall the other phones in the room. A room carries at
most 32 phones at once.

## 5. On the phone

The app is Relay first: it opens on the PC. The home screen is one line that says which PC is
connected and how (WiFi, Tailscale or your server), a row when something needs you (holds,
blocked agents, tasks to review), a slim card of figures and provider usage, the live agents
grouped by project, the stopped agents folded into one row, and a **New terminal** button.
The header has three buttons:

- **The menu** (left, or swipe from the left edge) opens the app's drawer: the PC at the
  top with its workspaces, inbox and paired PCs, then **Local** — the characters, models
  and recent chats that run on the phone itself.
- **The bell** opens the **Inbox**, with a badge for how much is waiting: guardrail holds,
  tasks in review, and the notification feed. Tap a notification to mark it read on the PC.
- **The folder** (right, or swipe from the right edge) opens the workspaces sidebar: every
  workspace on the PC and the projects in it, each with how many agents are live there. Tap
  one to narrow the home screen — and the New terminal sheet — to it; the board icon next
  to a project opens its board.

Tapping the PC's line opens **Paired PCs**: every PC this phone knows, its routes, adding an
address, pairing another, and the two things the link may do to the phone (notify you, keep
the screen on in a terminal).

**Pairing.** Scan the QR that `relay remote pair` prints. Without a camera, paste the
`relay://pair?…` link, or type the address the PC printed and the eight-character code, then
approve the phone in the terminal on the PC when it asks. The code is spent on use, whether
the PC said yes or no.

**Routes.** Each PC card shows where it can be reached and lets you pin a route: **Auto** tries
the direct addresses (WiFi and Tailscale) first and the server second; **Direct only** never
uses the server; **Server only** always goes through the rendezvous. The route in use is marked.

**What you can do.** Everything is an existing bus op, limited to the ones the door lets a
phone call (see the top of this page):

- **New terminal**: pick a project and an agent (claude or codex), optionally a first message,
  and a session starts in its own worktree (`session.create` + `session.spawn`); its terminal
  opens. **More options** opens the full launcher (several agents, a review group, staged
  tasks). To track work on the board, create the task there and dispatch it.
- **Sessions**: every live agent session grouped by project, with Relay's lamps (green
  running, red held, amber spawning). Tap one for its terminal. Agents stopped by a PC restart
  sit in one folded "stopped agents" row at the end, with Resume per agent or Resume all.
- **Terminal**: a real terminal emulator (xterm.js, headless) rebuilt from the engine's raw
  replay (`session.attach` with no position replays up to 256 KiB, then streams live), so
  Claude Code and Codex redraws look as they do on the PC. While it is open it borrows the
  PTY at the phone's own width (see below); pinch or the ⋯ menu's text size reflows it.
  Type in the composer and send;
  the text and its Enter go as separate writes so a TUI does not take them for a paste
  (several lines go as one bracketed paste when the program asks for it). The key row has
  Esc, Tab, arrows, Enter, Ctrl-C, `y`/`n`; the ⋯ menu carries **Summarize**, **Changes**,
  **Mail**, copy, text size, **Use the PC's width** / **Fit to this phone**, and **Park** /
  **Wake** / **Resume** as the session's state allows. Keystrokes go through `session.input`, which the engine answers without touching
  its store.
- **Mail**: priority mail to that agent (`mailbox.send`). It reaches an agent that is busy at
  its next step, where a typed line would wait in the terminal until it reads its prompt.
- **Inbox**: guardrail holds with **Allow once** / **Deny** (`guardrail.confirm` /
  `guardrail.reject`), unread notifications, and how many tasks wait for review. With the app
  in the background, a held, blocked or finished agent shows up as a notification; it comes
  from the PC link, not from a push service.
- **Board**: the project's tasks by column (`task.list`). Approve what is in review
  (`task.approve`), dispatch what is waiting (`task.dispatch`), jump to a task's terminal or
  changes.
- **Changes**: the files an agent changed in its worktree (`git.status`, `git.diff`), with a
  tap for the hunks (`git.diff.file`).
- **Summary**: with a local model loaded, the phone's own model reads the terminal text and
  says what the agent did and what it needs. The text never leaves the phone.

While a terminal is open, the phone borrows the session's PTY at the phone's own width and
height (`session.resize {until_detach}`), so the agent lays itself out for the phone instead
of being shrunk or cut off at the PC's width; the desktop shows the narrow layout meanwhile.
Changing the text size reflows it to as many columns as fit. The PC's size comes back by
itself when you leave the terminal, put the phone away, or the link drops. **Use the PC's
width** in the ⋯ menu keeps the PC's size instead, fitted to the screen with pinch to zoom.
A PC whose Relay predates this keeps its width.

The link reconnects by itself after a drop, and again the moment the app comes back to the
foreground; **Disconnect** on a PC card stops that until you connect again.

The phone's own chats and on-device models never touch this link.

## 6. Security model, honestly

- A phone is the user at the keyboard. The door refuses any envelope whose actor is not
  `user` (or `test` on a dev/test instance) before the engine sees it, so a phone cannot
  borrow an agent's identity even with a guessed token.
- A phone also gets fewer ops than the user at the keyboard: only those in `PHONE_OPS`
  (`crates/relay-remote/src/wire.rs`). Ops that reconfigure what the engine runs
  (`settings.set`, which holds provider and adb paths), quit it, or that no screen of the app
  uses are refused (`refused` / `remote.op`). A WebSocket frame holding several lines is
  gated line by line, so a permitted first line cannot carry a refused second one through.
- **App developers:** an op the phone starts calling must be added to `PHONE_OPS` (it is
  sorted; the gate binary-searches it), or the door refuses it.
- Pairing codes live ten minutes and are single use, a new code replaces any older one, and the
  PC approves each phone that presents one before it gets a token (unless `--no-confirm`), so
  a code seen by someone else — a screen share, scrollback — pairs nothing on its own. Every
  pairing is logged and toasted on the desktop. A device token is 256 bits, stored on
  the PC in `remote.json` and on the phone in its private storage.
- After pairing, the token is never sent again: each connection gets a random challenge and
  the phone answers with `sha256(challenge:token)`. Listening on the LAN yields proofs that
  are only good for that connection.
- The one message that carries the token is the pairing reply. On your own WiFi that is a
  LAN packet; through a rendezvous it should ride `wss://`, which is why `via` warns on
  `ws://`. Anyone who captures the pairing reply holds the phone's credential for good.
- The rendezvous server sees the same bytes the LAN would — encrypted by TLS in flight if
  you set it up as above, but readable by whoever runs the server. Run it yourself.
- **The channel is not authenticated after the handshake, and the PC never proves itself.**
  The proof shows the PC that the phone holds its token, once, at the start of the
  connection; nothing seals the lines that follow. Someone *on the path* — an active attacker
  on the same WiFi (ARP spoofing, a rogue access point) or whoever runs the rendezvous — can
  read every line, and can inject bus requests as `user` into a connection the phone already
  opened. That is command execution on the PC: `session.spawn` and `session.input` are among
  the phone's ops. The phone also cannot tell its PC from an
  impostor answering at a stored address. Passive listening alone gets proofs that are no
  good on another connection, and nothing reaches the engine from a peer that has no proof.
- Until the link is sealed end to end (a planned wire v2: the PC's public key in the pairing
  QR, an ephemeral key exchange with the token as a pre-shared key, every line encrypted and
  authenticated), the route that resists an active attacker is **Tailscale**: bind the door
  to the tailnet address only, `relay serve --remote --remote-bind <100.x.y.z>:7420` (or
  `relay remote serve --bind …`), and pair over it. Use the LAN route only on a network you
  trust, and treat a rendezvous you do not run as able to act as you.
- To cut a phone off from the PC side: `relay remote revoke <device id>` (the id is on the
  PC card in the app). An open connection closes within a few seconds.
- The op list narrows what a lost phone or an injected line can do; it is not a sandbox for a
  hostile phone. A paired phone can still start agents and type into their terminals, which
  is acting as you.

## 7. Verifying

```fish
cargo test -p relay-remote
```

covers pairing, proof, revocation, the actor gate, the op gate (`settings.set` refused),
`GET /info`, event interleaving and a
two-phone rendezvous session against a real engine. It runs in CI with the rest of the
headless crates. The app itself is typechecked and linted in CI
(`npx tsc --noEmit -p tsconfig.json && npm run lint` in `apps/relay-mobile`).
