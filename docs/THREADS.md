# Threads

Threads replace the Money space. A thread is a conversation with an agent that works on the data
of Antho's own apps, shown and editable beside it: agent first, data second. Tally's budget is the
first source; investment tracking in Tally comes next; the gym app and an AI investing bot are
later and not designed here.

Decided with Antho on 2026-10-07:

- **Like the claude.ai website.** Native message bubbles, a thread list, a message box. Not a
  terminal. Many threads, each kept.
- **One app with Dev.** Threads is a space of the same window: the same title bar, sidebar,
  you-row, Settings, status bar, colours and fonts (`DESIGN.md`). Only the middle changes. The Money
  space's own warm palette goes.
- **No resending the whole conversation on every open.** Relay keeps its own copy of every
  message, so a thread reopens without the agent. While a thread is in use its agent keeps running;
  a cold thread is resumed once by Claude's own conversation id.
- **The agent may change the data, within limits.** It adds and edits entries freely, each with an
  Undo; deleting, budgets and accounts ask the person first.
- **Charts may use colour,** and the Tally panel's views may too. A chart stores its question, not
  its numbers, so it redraws when the phone syncs.

The mockup: https://claude.ai/artifact/GPtaPk6YYLiFCHGpeNPMk4

## Phases

1. **The thread engine** (built): `threads.db`, the `thread.*` ops, the background agent.
2. **The Threads space** in `relay-native` (built): thread list in the sidebar, the conversation,
   the message box, the Tally panel beside it, and Tally's pages restyled in Dev's tokens.
3. **Cards:** charts, budget meters and the Undo card, live from the ledger.
4. **Asking first:** a confirm card for what the agent may not do alone (delete, budgets,
   accounts), run as the person when they approve.
5. **Investments in Tally:** holdings and their value tracked in the ledger, on the phone and the
   PC in the same commit (`CLAUDE.md`: a money rule changes on both sides at once).

## The engine (phase 1)

`crates/relay-core/src/threads.rs`, ops in `crates/relay-bus/src/ops/thread.rs`.

**Storage.** `threads.db` beside the store (`money.db`'s neighbour), its own SQLite file behind its
own mutex: a `thread` row (title, provider, model, `agent_ref`) and its `message` rows. The store's
lock is never held for a thread, and the store's schema is untouched.

**Messages.** `role` is `user` (`{"text"}`), `assistant` (`{"blocks"}`, each a `text` or a
`tool_use` with its `id`, `name` and `input`), `tool` (`{"tool_use_id", "is_error", "text"}`) or
`error` (`{"text"}`). Claude's thinking is not kept.

**The agent.** Claude in print mode with stream-json in and out, started in `threads/` beside the
store (Claude files conversations by working directory, so a resume must start there):

```
claude -p --input-format stream-json --output-format stream-json --verbose
       --include-partial-messages --strict-mcp-config --mcp-config <relay, as the person>
       --tools "" --allowedTools mcp__relay --setting-sources ""
       --append-system-prompt <what a thread is> [--model M] [--resume <agent_ref>]
```

- `--tools ""` removes every built-in tool: no shell, no files, no web. Verified with Claude Code
  2.1.293: the agent's tool list is only the MCP server's tools, and it may call them.
- The MCP server is `relay --actor user mcp` with `RELAY_MCP_OPS` set to `threads::AGENT_OPS`. A
  thread has no session of its own, so the agent acts as the person; what it may do is exactly that
  list, which `relay mcp` now enforces on calls as well as on the listing. Today: `bus.schema`,
  `money.summary`, `money.lists`, `money.tx.list`, `money.tx.add`, `money.tx.update`,
  `money.tx.restore`.
- Each message is one stream-json line on the agent's stdin. Its stdout is read on a thread of its
  own: `thread.delta` events carry the reply as it is written, and each finished assistant message
  and tool answer is stored and announced with `thread.message`. Claude's `result` ends the turn.
- Spawning and writing to the agent happen after the handler returns, never under the store lock.
- One turn at a time: a message sent while the agent is answering is refused with `thread.busy`.
  `thread.stop` kills the agent's process group; the next message resumes it.
- An agent idle for 15 minutes is closed; at most four run at once, the one used longest ago closing
  first. An agent that exits mid-turn leaves an `error` message saying so, with its last stderr line.

**Ops.** `thread.list`, `thread.get` (messages after an id, for catching up), and, the person's
alone: `thread.create` (optionally with the first message, which also names the thread),
`thread.send`, `thread.stop`, `thread.rename`, `thread.delete`.

**Events.** `thread.changed` (`{"thread"}`, or `{"id", "deleted": true}`), `thread.message`
(`{"thread", "message"}`), `thread.delta` (`{"thread", "text"}`, not stored).

Tests: `crates/relay-core/tests/threads.rs` drives threads end to end against a stand-in Claude
that speaks the same stream-json.

## The Threads space (phase 2)

`apps/relay-native/src/threads_view.rs`, styled by `css/threads.css`; the space itself is still
`money.rs` (the switcher, the sidebar swap, the start screen), which now says Threads.

- **The sidebar** keeps Dev's frame and footer. Its keys are New thread (Ctrl N) and Tally, then
  the threads grouped Today, Yesterday, This week and Earlier, a green lamp on one at work.
- **The middle** is a new thread (greeting, the month in a line, the message box, four questions to
  start) or the open one: the person's messages as bubbles, the agent's Markdown drawn natively
  (`relay_client::thread_view`: paragraphs, headings, lists, tables, code), each tool call as a
  quiet line saying what it did, and an entry the agent added as a card with Undo
  (`money.tx.delete`). Enter sends, Shift+Enter breaks the line; while the agent works the send key
  stops it.
- **The Tally panel** (the strip's right key hides it): Overview (what is left, budgets, recent
  entries), Entries (this month) and Budgets. Meters take the category's hue; amber is ahead of
  pace, red over budget.
- **Tally's pages** (Overview, Entries, Plan, Data) open from the sidebar's Tally key with their own
  tabs, in Dev's tokens (`css/money.css`); the Money space's warm palette is gone.

A display smoke run opens a thread with `RELAY_NATIVE_THREAD=<id>` beside `RELAY_NATIVE_PAGE=threads`.
