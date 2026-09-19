# Shared code graph

**Status: proposal.** Nothing in this document is implemented. It exists so the
design can be argued with before any of it is written, and so the two questions
that could invalidate it are on the record.

## The problem

An agent looking for where something is defined greps. On a repository this size
that is several `rg` invocations per question, each one a subprocess and a full
tree walk, and the answer is a list of lines rather than a list of symbols.
[graphify](https://github.com/Graphify-Labs/graphify) extracts a symbol graph —
definitions, references, the edges between them — and answers that question
directly.

The naive integration is to run it per session, in each worktree, on demand.
That is the wrong shape: it is the same extraction over the same code, once per
agent, on a machine the user is also playing games on. The whole point of the
performance work was to stop doing uninvited background work.

## The shape

One graph per project, not per session and not per branch. It is built from the
project's base branch and updated when a commit lands there. Every agent, in
every worktree, queries the same graph.

- **Storage.** `<data_dir>/graphs/<project_id>/`, where `<data_dir>` is
  `paths::Instance::data_dir()` — `~/.local/share/relay-v4/<instance>/`. The
  graph is derived data: deleting the directory costs one re-extraction and
  nothing else.
- **Trigger.** A commit whose worktree is on `project.base_branch`.
- **Access.** One bus op, `graph.query`.
- **Default.** Off.

An agent on a feature branch therefore queries a graph that does not contain the
symbols it wrote five minutes ago. That is the trade for having one graph, and
it is the right one: the graph is for finding code that already exists, not for
reading back your own diff.

## Trigger

`handlers/git.rs:491` registers `CommitOp` with `register_staged`. Its commit
phase already ends by producing the new sha:

```rust
worktree::git_mutate(&root, &["commit", "--no-verify", "-m", &p.message])?;
let repo = gix::open(&root)?;
let sha = repo.head_id()?.to_string();
changed(ctx, project.id, &root);
Ok(CommitOut { sha })
```

The hook goes after `changed`: read the committing worktree's branch, compare it
to `project.base_branch`, and on a match call `ctx.after_commit(move |engine| …)`
with the sha.

`after_commit` is not a convenience here, it is the requirement. Handlers run
inside one SQLite transaction with the store mutex held (`BUS.md` §5.1, D144,
D149). graphify is a subprocess and a tree walk — precisely the two things that
must never run there. A 20-second extraction inside the transaction is a
20-second freeze on every other request, keystrokes included.

## Coalescing

Rebases, merges and squashes land several base-branch commits in a row, and each
one would otherwise queue an extraction of a tree that is about to change again.

The pattern already exists at `handlers/app.rs:156`: an
`(in_flight: bool, last: Option<Instant>)` pair behind a mutex on `Engine`, with
a guard struct whose `Drop` clears the flag and stamps the completion time.

```rust
{
    let mut refresh = engine.graph_refresh.lock().unwrap();
    if refresh.0 || refresh.1.is_some_and(|at| at.elapsed() < COOLDOWN) {
        return;
    }
    refresh.0 = true;
}
```

Copy it as `graph_refresh` with a cooldown in the tens of seconds. A burst of
commits produces one extraction, and because graphify reads the checked-out tree
rather than a commit object, the single run it does produce reflects the newest
state rather than the oldest.

## Execution

From `after_commit`, on a dedicated thread:

- **`relay_core::background_priority()` first.** `setpriority(PRIO_PROCESS, 0, n)`
  is per-thread on Linux and the forked child inherits it, so both the walk and
  graphify itself sit behind the UI — and behind a game — rather than competing
  with them.
- **`proc::output_with_timeout(&mut cmd, timeout)`, always.** It is the rule for
  every subprocess forked from the engine. It sets `process_group(0)`, so a
  graphify that wedges is killed as a group instead of leaking children. Two
  minutes is a reasonable ceiling for a first cut; the real number comes out of
  the measurement below.
- **Discovery through `which::which("graphify")`.** The `which` crate is already
  a workspace dependency. Absent means the feature is off — one `tracing` line,
  no error surfaced to the user, no install prompt.

## Where it runs

graphify needs a checkout to walk, and the obvious candidate is the wrong one.
The project root is the user's own working copy: it may be dirty, mid-rebase, or
sitting on an unrelated branch. Indexing it would build a graph of a half-written
edit.

Instead, a dedicated worktree pinned to the base branch, created once through the
existing `worktree::create(repo, &path, branch, from)`:

```
<data_dir>/graphs/<project_id>/tree/          the checkout graphify walks
<data_dir>/graphs/<project_id>/graphify-out/  the graph itself
```

Before each run, `worktree::git_mutate(&tree, &["checkout", "--detach", &sha])`
moves it to the sha the commit produced. The graph is then reproducible from a
sha, which makes "is the graph stale?" a string comparison rather than a guess.

## Agent access

One op — `graph.query`, `OpKind::Query`, registered with
`Engine::register_unlocked` so that reading the graph never takes the store
mutex and never serializes against a write.

Registering it is the whole integration. `BUS.md` §6.4: `relay mcp` advertises
each implemented control-plane op the caller may actually call as a tool of the
same name, so `graph.query` becomes a Relay MCP tool for Claude with no further
work, and `$RELAY_BIN q graph.query '<json>'` reaches it from Codex, which takes
no `--mcp-config`.

It must stay **one** op. D104 is the reason: the tool list is a budget, already
cut from 128 tools / ~146 KB to roughly 84 / ~35 KB because a payload that large
gets dropped by the harness rather than kept. A graph namespace of five ops
spends that budget for no gain over one op with a good `inputSchema`.

**Rejected: symlinking the graph into each worktree.** It needs no new op, and
that is exactly the problem — a filesystem path has no authorization layer in
front of it. One agent running `/graphify .` through the link corrupts the graph
for every other session on the machine. A bus op goes through all three §9.1
layers; a symlink goes through none.

## Default off

Behind a setting, in the shape of the existing provider keys:

```
settings.set graph.enabled true
```

It forks a subprocess and walks a tree on every base-branch commit. That is a
reasonable thing to opt into and an unreasonable thing to discover.

## Open questions

The first is blocking — it decides whether the design above survives.

1. **Does `--update` remove deleted symbols?** Index a repository, delete a file
   that defines a symbol, run `graphify --update`, query for that symbol. If it
   is still there, incremental updates accumulate ghosts, and the design needs a
   periodic full re-extract — every Nth update, or once N deletions accumulate.
   A full re-extract is a much slower operation than an update, so the cooldown
   above and the `output_with_timeout` ceiling both change with the answer.

2. **Does the graph worktree show up in the UI?** `worktree::list(repo)` returns
   the repository's git worktrees, and the graph checkout is one. It should not
   appear in the session/worktree lists as something the user can open, close or
   purge. Either filter it by path prefix or keep it out of the repository's
   worktree list entirely.

3. **Skill filename case.** graphify ships its skill as
   `skills/graphify/skill.md`, lowercase. Relay matches `SKILL.md`
   case-sensitively (`github.rs:204`, `github.rs:213`), so the skill would not be
   found if it were ever materialized into a worktree. This does not affect the
   bus-op path above; it matters only if we also ship graphify's own skill.

4. **Cost.** Wall-clock and peak RSS for one extraction and one update on this
   repository, measured under `background_priority()`. Until those numbers exist,
   "default off" is the only defensible default.

## Not in scope

- Per-branch graphs. See the trade above.
- Replacing grep. The graph answers questions about symbols; full-text search over
  file contents stays where it is.
