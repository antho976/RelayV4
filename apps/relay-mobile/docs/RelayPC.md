# The PC tab: your Relay desktop from the phone

Relay on the phone keeps its chats and on-device models to itself. The **PC** tab is a
separate thing: a window onto the Relay engine running on your computer, over a link that
starts at your own WiFi and, if you set one up, continues through a server you host.

## Pairing

1. On the PC, with Relay running: `./target/debug/relay remote serve --pair` from the
   RelayV4 checkout (it finds the engine the desktop app started). It prints a QR code and a
   line like `relay://pair?…`.
2. On the phone: drawer → **PC** → **Pair a PC**, and scan the code. The camera needs
   permission the first time. Without a camera, paste the link, or type the address the PC
   printed and the eight-character code.
3. The phone stores a credential for that PC. The code is spent; to pair another phone, run
   `relay remote pair` on the PC again.

## Routes

Each paired PC card shows its routes and lets you pin one:

- **Auto** (default): try the WiFi addresses first, the server second.
- **WiFi only**: never leave the network. The phone will say "not connected" when it is away.
- **Server only**: always go through the rendezvous, even at home.

The status line at the top says which one is in use: **WIFI · PRIVATE** or **VIA YOUR
SERVER**.

The server is yours: `relay remote rendezvous` on any machine you control, behind TLS, then
`relay remote via wss://your-server` on the PC. The PC dials out to it; nothing on the PC
listens to the internet. See `docs/MOBILE.md` at the repository root for the setup.

## What you can do

- **New request**: pick a project, say what you want done, and an agent on the PC starts on
  it in its own worktree. By default the request becomes a task on the project's board and is
  dispatched (`task.create` + `task.dispatch`), so it lands in review when the agent reports
  done; switch that off for a one-off session with your text as its opening prompt. The
  terminal for the new session opens so you can watch it and answer its questions.
- **Mail**: from a session's terminal, **mail** sends priority mail to that agent
  (`mailbox.send`). It reaches an agent that is busy at its next step, where a typed line
  would wait in the terminal until it reads its prompt.
- **Sessions**: every live agent session, with Relay's lamps (green running, red held,
  amber spawning). Tap one for its terminal.
- **Terminal**: the session's recent output, then live. Type a line and **Send** (or
  **Enter** on an empty line). The key row has Esc, Tab, arrows, Ctrl-C, and `y`/`n` for the
  questions agents ask. **Park**, **Wake** and **Resume** sit in the identity strip.
- **Needs you**: guardrail holds with **Allow once** / **Deny**, unread notifications, and
  how many tasks wait for review on the desktop. With the app in the background, a held,
  blocked or finished agent shows up as a notification (Settings → Privacy to turn that off);
  it comes straight from the PC link, not from a push service.
- **Board**: the project's tasks by column. Tap a card for its body and changelog; approve
  what is in review, dispatch what is waiting, jump to a task's terminal or changes.
- **Changes** (**diff** in a terminal, or from a board card): the files the agent changed in
  its worktree, with a tap for the hunks.
- **On-device summary** (**sum** in a terminal): with a local model loaded, the phone's own
  model reads the terminal text and tells you what the agent did and what it needs. The text
  never leaves the phone.

The link reconnects by itself after a drop, and again the moment the app comes back to the
foreground. **Disconnect** on a PC card stops that until you connect again.

Every action is a normal bus request from actor `user`, so the desktop audit log shows it
as yours.

## Privacy

- Chats never use the PC link. Local mode runs the model on the phone; the fallback to an
  API (Settings → Privacy) only applies when no on-device model can run, tells you each time,
  and can be switched off.
- The pairing credential stays on the phone. Each connection proves it with a one-time
  challenge instead of sending it.
- To cut a phone off from the PC side: `relay remote revoke <device id>` (the id is on the
  PC card).
