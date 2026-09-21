# Relay from a phone

The Relay mobile app (the `antho976/ChatterUI` fork, "Relay" on the phone) reaches the engine
on this machine through `crates/relay-remote`: the same bus lines the desktop client and the
CLI speak (`BUS.md` §6.2), carried over a WebSocket to a phone that paired once.

Two routes, tried in privacy order by the phone:

| route | when | what travels where |
| --- | --- | --- |
| **direct** | phone and PC on the same network | LAN only; nothing leaves the building |
| **via a rendezvous** | phone anywhere | PC dials *out* to a server you host; the phone joins there; the server copies lines and reads none of them |

Both end in `bridge.rs` on the PC, which checks the phone's credential and forwards its
requests to the socket door as actor `user` — the same door, the same rights, the same
guardrails and audit rows as the CLI. There is no third route and no third party.

## 1. Serve the door on the PC

With an engine running (`relay serve`, or the desktop app):

```fish
relay remote serve --pair
```

That listens on `0.0.0.0:7420` for phones on the network, opens a ten-minute pairing window
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

- **Sessions**: the wall as a list with Relay's lamps; open one for its terminal
  (`session.scrollback` + `session.attach`), type to it (`session.input`), park, wake, resume.
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
