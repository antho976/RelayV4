# Relay from a phone

The Relay mobile app (`apps/relay-mobile`, "Relay" on the phone) reaches the engine on this
machine through `crates/relay-remote`: the same bus lines the desktop client and the
CLI speak (`BUS.md` §6.2), carried over a WebSocket to a phone that paired once.

Two routes, tried in privacy order by the phone:

| route | when | what travels where |
| --- | --- | --- |
| **direct** | phone and PC on the same network | LAN only; nothing leaves the building |
| **via a rendezvous** | phone anywhere | PC dials *out* to a server you host; the phone joins there; the server copies lines and reads none of them |

Both end in `bridge.rs` on the PC, which checks the phone's credential and forwards its
requests to the socket door as actor `user` — the same door, the same rights, the same
guardrails and audit rows as the CLI. There is no third route and no third party.

## 0. Tonight, in three steps

On the PC, from the RelayV4 checkout: `./run.sh` as usual. The engine it starts now carries
the phone door (`relay serve --remote`), so there is nothing else to keep running. Then:

```fish
./target/debug/relay remote pair              # prints the QR code (finds the dev engine on its own)
```

On the phone: install the app (the `relay-mobile-apk` artifact of the **Mobile APK** workflow,
run from the Actions tab, or `npm run android` in `apps/relay-mobile` with the Android SDK),
open the drawer, tap **PC**, **Pair a PC**, scan. If the
engine was already running from before this change, restart it once (close the app, then
`./target/debug/relay --instance dev cmd app.quit '{}'`, then `./run.sh`), or run
`./target/debug/relay remote serve --pair` alongside it instead. For the phone to reach the
PC when you are out, see §2.

## 1. Serve the door on the PC

With an engine running (`relay serve`, or the desktop app):

```fish
relay remote serve --pair
```

Or start both in one process, which is what a login service wants:

```fish
relay serve --remote            # engine + phone door; `deploy/relay-remote.service` runs this
```

`relay remote serve` listens on `0.0.0.0:7420` for phones on the network, opens a ten-minute pairing window
and prints a QR code. Scan it from the app's **PC** tab. To pair another phone later, in a
second terminal:

```fish
relay remote pair
```

Useful flags and subcommands:

| command | does |
| --- | --- |
| `relay remote serve --bind 192.168.1.20:7420` | listen on one address only |
| `relay remote devices` | list paired phones |
| `relay remote revoke <id>` | forget a phone; its next connection is refused |
| `relay remote name "antho desktop"` | the name phones show |
| `RELAY_INSTANCE=dev relay remote serve` | serve the dev engine instead of stable |

State lives in `~/.local/share/relay-v4/<instance>/remote.json`, mode 0600. It holds each
phone's token, so treat it like a private key.

## 2. Reach the PC from anywhere: your own rendezvous

The PC never listens to the internet. Instead it keeps one outbound WebSocket open to a
small server you run, and phones join through it — the shape of a remote-control service,
with the service being yours.

On any machine with a public address (a VPS, a Raspberry Pi behind a port forward, a
Tailscale node):

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

`via` mints a room secret and stores it; `serve` dials the server, and every pairing link
from then on carries the join address as well as the LAN ones. The phone tries the LAN first
and the server second; a host card in the app can pin either.

The rendezvous keeps nothing on disk. A room is named by the digest of its secret, so only
the PC that holds the secret can host it, and a restart forgets nothing worth keeping.
`GET /health` reports how many hosts and lanes are up.

`relay remote via off` clears the configuration.

## 3. What the phone can do

Everything goes through existing bus ops, so the list is the bus's:

- **Requests**: a project, a piece of text, and an agent starts on it — `task.create` and
  `task.dispatch` with a fresh session by default, or `session.create` + `session.spawn`
  with the text as the opening prompt. **Mail** to a running session is `mailbox.send` with
  priority, which reaches a busy agent at its next bus call.
- **Sessions**: the wall as a list with Relay's lamps; open one for its terminal
  (`session.scrollback` + `session.attach`), type to it (`session.input`), park, wake, resume.
- **Board**: `task.list` by column; `task.approve` from in review, `task.dispatch` from
  backlog or ready.
- **Changes**: a session's worktree through `git.status`, `git.diff` and `git.diff.file`.
- **Needs you, while away**: the phone turns `guardrail.held` and `notify.new`
  (`agent_done`, `agent_blocked`) into local notifications when the app is in the background.
  They ride the same socket; no push service is involved.
- **On-device summary**: the phone's own model reads a terminal's text and says what the
  agent did. Nothing is sent anywhere; the PC is not involved beyond the output it already
  streamed.
- **Needs you**: open guardrail holds with *Allow once* / *Deny* (`guardrail.confirm` /
  `guardrail.reject`), unread notifications, the count of tasks in review.
- The phone's own chats and on-device models never touch this link.

The terminal on the phone is a plain-text view — escape sequences are dropped, carriage
returns overwrite — not a terminal emulator. It is for reading what an agent says and
answering it, not for `vim`.

## 4. Security model, honestly

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
- None of this is a sandbox for a hostile phone: a paired phone is you.

## 5. Verifying

```fish
cargo test -p relay-remote
```

covers pairing, proof, revocation, the actor gate, `GET /info`, event interleaving and a
two-phone rendezvous session against a real engine. It runs in CI with the rest of the
headless crates.
