# Relay from a phone

The Relay mobile app (`apps/relay-mobile`, "Relay" on the phone) reaches the engine on this
machine through `crates/relay-remote`: the same bus lines the desktop client and the CLI speak
(`BUS.md` §6.2), carried over a WebSocket to a phone that paired once.

Two routes, tried in privacy order by the phone:

| route | when | what travels where |
| --- | --- | --- |
| **direct** | phone and PC on the same network | LAN only; nothing leaves the building |
| **via a rendezvous** | phone anywhere | PC dials *out* to a server you host; the phone joins there; the server copies lines and reads none of them |

Both end in `bridge.rs` on the PC, which checks the phone's credential and forwards its
requests to the socket door as actor `user` — the same door, the same rights, the same
guardrails and audit rows as the CLI. There is no third route and no third party.

## 1. Tonight, in three steps

On the PC, from the RelayV4 checkout, `./run.sh` as usual. The engine it starts carries the
phone door (`relay serve --remote`), so there is nothing else to keep running. Then:

```fish
./target/debug/relay remote pair              # prints the QR code (finds the dev engine on its own)
```

On the phone: install the app (the `relay-mobile-apk` artifact of the **Mobile APK** workflow,
run from the Actions tab, or `npm run android` in `apps/relay-mobile` with the Android SDK),
open the drawer, tap **PC**, **Pair a PC**, scan. If the engine was running from before the
door existed, restart it once (`./target/debug/relay --instance dev cmd app.quit '{}'`, then
`./run.sh`), or run `./target/debug/relay remote serve --pair` alongside it. For the phone to
reach the PC when you are out, see §3.

## 2. On the PC

With an engine running (`relay serve`, or the desktop app):

```fish
relay remote serve --pair       # front the engine for phones, open a pairing window, print the QR
```

Or both in one process, which is what a login service wants:

```fish
relay serve --remote            # engine + phone door; `deploy/relay-remote.service` runs this
```

The door listens on `0.0.0.0:7420` for phones on the network. A pairing window lasts ten
minutes and admits one phone; to pair another later, `relay remote pair` in a second terminal.

| command | does |
| --- | --- |
| `relay remote serve --bind 192.168.1.20:7420` | listen on one address only |
| `relay remote devices` | list paired phones |
| `relay remote revoke <id>` | forget a phone; its next connection is refused |
| `relay remote name "antho desktop"` | the name phones show |
| `RELAY_INSTANCE=dev relay remote serve` | serve the dev engine instead of stable |

State lives in `~/.local/share/relay-v4/<instance>/remote.json`, mode 0600. It holds each
phone's token, so treat it like a private key.

## 3. From anywhere: your own rendezvous

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
this link; `wss://` keeps that between the phone and the PC. A Caddyfile is enough:

```
relay.example.org {
    reverse_proxy 127.0.0.1:7430
}
```

Then on the PC:

```fish
relay remote via wss://relay.example.org
relay remote serve --pair
```

`via` mints a room secret and stores it; `serve` dials the server, and every pairing link from
then on carries the join address as well as the LAN ones. The phone tries the LAN first and the
server second; a PC card in the app can pin either. `relay remote via off` clears it.

The rendezvous keeps nothing on disk. A room is named by the digest of its secret, so only the
PC that holds the secret can host it, and a restart forgets nothing worth keeping.
`GET /health` reports how many hosts and lanes are up.

## 4. On the phone

The **PC** tab is one screen: a line that says which PC is connected and how, what needs a
decision, the live sessions, and a bar at the bottom to hand an agent some work. The gear in
its corner opens **Paired PCs**: every PC this phone knows, its routes, pairing another, and
the two things the link may do to the phone (notify you, keep the screen on in a terminal).

**Pairing.** Scan the QR that `relay remote pair` prints. Without a camera, paste the
`relay://pair?…` link, or type the address the PC printed and the eight-character code. The
code is spent on use.

**Routes.** Each PC card shows where it can be reached and lets you pin a route: **Auto** tries
WiFi first and the server second; **WiFi only** never leaves the network; **Server only**
always goes through the rendezvous. The status line says which is in use.

**What you can do.** Everything is an existing bus op, so the list is the bus's:

- **Ask an agent**: pick a project, say what you want done, and an agent starts on it in its
  own worktree — `task.create` + `task.dispatch` by default, so it lands in review when the
  agent reports done; or a one-off session with your text as its opening prompt
  (`session.create` + `session.spawn`). The new session's terminal opens.
- **Sessions**: every live agent session, with Relay's lamps (green running, red held, amber
  spawning). Tap one for its terminal.
- **Terminal**: the session's recent output (`session.scrollback`), then live
  (`session.attach`). Type a line and **Send**, or **Enter** on an empty line; the key row has
  Esc, Tab, arrows, Ctrl-C and `y`/`n`. The strip above it carries **Summary**, **Changes**,
  **Mail**, and **Park** / **Wake** / **Resume** as the session's state allows. Keystrokes go
  through `session.input`, which the engine answers without touching its store.
- **Mail**: priority mail to that agent (`mailbox.send`). It reaches an agent that is busy at
  its next step, where a typed line would wait in the terminal until it reads its prompt.
- **Needs you**: guardrail holds with **Allow once** / **Deny** (`guardrail.confirm` /
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

The terminal is a plain-text view — escape sequences are dropped, carriage returns
overwrite — not a terminal emulator. It is for reading what an agent says and answering it,
not for `vim`. The link reconnects by itself after a drop, and again the moment the app comes
back to the foreground; **Disconnect** on a PC card stops that until you connect again.

The phone's own chats and on-device models never touch this link.

## 5. Security model, honestly

- A phone is the user at the keyboard. The door refuses any envelope whose actor is not
  `user` (or `test` on a dev/test instance) before the engine sees it, so a phone cannot
  borrow an agent's identity even with a guessed token.
- Pairing codes live ten minutes and are single use. A device token is 256 bits, stored on
  the PC in `remote.json` and on the phone in its private storage.
- After pairing, the token is never sent again: each connection gets a random challenge and
  the phone answers with `sha256(challenge:token)`. Listening on the LAN yields proofs that
  are only good for that connection.
- The one message that carries the token is the pairing reply. On your own WiFi that is a
  LAN packet; through a rendezvous it should ride `wss://`, which is why `via` warns on
  `ws://`.
- The rendezvous server sees the same bytes the LAN would — encrypted by TLS in flight if
  you set it up as above, but readable by whoever runs the server. Run it yourself.
- To cut a phone off from the PC side: `relay remote revoke <device id>` (the id is on the
  PC card in the app).
- None of this is a sandbox for a hostile phone: a paired phone is you.

## 6. Verifying

```fish
cargo test -p relay-remote
```

covers pairing, proof, revocation, the actor gate, `GET /info`, event interleaving and a
two-phone rendezvous session against a real engine. It runs in CI with the rest of the
headless crates. The app itself is typechecked and linted in CI
(`npx tsc --noEmit -p tsconfig.json && npm run lint` in `apps/relay-mobile`).
