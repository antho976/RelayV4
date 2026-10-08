//! Threads (docs/THREADS.md): conversations with an agent that reads and changes the person's
//! own data, kept apart from the store.
//!
//! Relay keeps every thread's messages in `threads.db` beside the store, behind its own mutex,
//! so a thread reopens instantly and the store lock is never held for one. The agent is Claude
//! in print mode speaking stream-json, run in the background while its thread is in use: each
//! message is one line on its stdin, and its stdout is read on a thread of its own here. Nothing
//! slow happens inside a handler; spawning and writing to the agent run after the handler
//! returns (`Ctx::after_commit`).
//!
//! The agent is given no shell and no file tools, only Relay's MCP server, and that server lists
//! and answers only [`AGENT_OPS`]. It calls them as the person, because a thread is the person's
//! own conversation and has no session of its own; what it may change is exactly that list.
//!
//! A thread whose agent is not running resumes by the conversation id Claude reported
//! (`agent_ref`), so the history is sent to the model once and stays warm after. An agent left
//! idle for [`IDLE`] is closed, and at most [`MAX_LIVE`] run at once.

use crate::engine::Engine;
use relay_bus::ops::thread::{MessageView, ThreadView};
use relay_bus::types::{Id, Provider};
use relay_bus::BusError;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

/// The ops a thread's agent may call. Reads of the ledger, and the entry changes a toast can
/// undo; deleting, budgets, accounts and anything that replaces the ledger stay the person's.
pub const AGENT_OPS: &[&str] = &[
    "bus.schema",
    "money.summary",
    "money.lists",
    "money.tx.list",
    "money.tx.add",
    "money.tx.update",
    "money.tx.restore",
];

/// How long an agent may sit with nothing to do before it is closed.
pub const IDLE: Duration = Duration::from_secs(15 * 60);
/// The most agents running at once; starting another closes the one used longest ago.
pub const MAX_LIVE: usize = 4;
/// The longest message a person may send, in characters.
pub const MAX_TEXT: usize = 32_000;
/// The most of a tool's answer kept with the thread, in characters.
const MAX_TOOL_TEXT: usize = 20_000;
/// The longest a thread title may be, in characters.
const MAX_TITLE: usize = 80;

/// What the agent is told, after Claude's own instructions.
const PROMPT: &str = "You are the agent in a Relay thread: a conversation with the person who owns this \
computer about their own data. Today that data is their budget, kept by their app Tally and synced \
from their phone, which you reach through the relay tools (money.*). Amounts are integers in minor \
units (cents): 4218 is 42.18. Read money.summary for the currency and the current budget period \
before answering about spending, and money.lists for account and category ids before adding an entry. \
You may add and change entries, and restore one; say plainly what you changed. You cannot delete \
entries, change budgets or add accounts: say what you would do and let the person do it in the Tally \
panel. Answer briefly, lead with the number they asked for, and never invent figures the tools did not \
give you.";

const SCHEMA: &str = "
CREATE TABLE thread (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT,
    agent_ref TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE message (
    id INTEGER PRIMARY KEY,
    thread_id INTEGER NOT NULL REFERENCES thread(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX message_thread ON message(thread_id, id);
";

/// `threads.db`: threads and their messages.
pub struct ThreadStore {
    conn: Connection,
}

/// A thread row, before the live state is added to make a [`ThreadView`].
pub struct Stored {
    pub id: Id,
    pub title: String,
    pub provider: String,
    pub model: Option<String>,
    pub agent_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub preview: Option<String>,
}

const THREAD_COLUMNS: &str = "t.id, t.title, t.provider, t.model, t.agent_ref, t.created_at, t.updated_at,
    (SELECT m.body FROM message m WHERE m.thread_id = t.id AND m.role IN ('user', 'assistant') ORDER BY m.id DESC LIMIT 1)";

fn stored(row: &rusqlite::Row) -> rusqlite::Result<Stored> {
    let last: Option<String> = row.get(7)?;
    Ok(Stored {
        id: row.get(0)?,
        title: row.get(1)?,
        provider: row.get(2)?,
        model: row.get(3)?,
        agent_ref: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        preview: last.and_then(|body| serde_json::from_str::<Value>(&body).ok()).and_then(|body| preview(&body)),
    })
}

fn message(row: &rusqlite::Row) -> rusqlite::Result<MessageView> {
    let body: String = row.get(3)?;
    Ok(MessageView {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        role: row.get(2)?,
        body: serde_json::from_str(&body).unwrap_or(Value::Null),
        created_at: row.get(4)?,
    })
}

/// The first line of a message's text, cut for the thread list.
fn preview(body: &Value) -> Option<String> {
    let text = body["text"].as_str().map(str::to_string).or_else(|| {
        body["blocks"].as_array()?.iter().find_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten().map(str::to_string))
    })?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(cut(line, 120))
}

/// `text` cut to `max` characters, with an ellipsis when it was longer.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

/// A thread's name from its first message: the first line, cut at a word near [`MAX_TITLE`].
pub fn title_from(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New thread");
    if line.chars().count() <= 60 {
        return line.to_string();
    }
    let head: String = line.chars().take(60).collect();
    match head.rfind(' ') {
        Some(at) if at > 30 => format!("{}…", head[..at].trim_end()),
        _ => cut(line, 60),
    }
}

impl ThreadStore {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::migrate(conn)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::migrate(Connection::open_in_memory()?)
    }

    fn migrate(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 0 {
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", 1)?;
        }
        Ok(ThreadStore { conn })
    }

    pub fn list(&self) -> rusqlite::Result<Vec<Stored>> {
        let mut stmt = self.conn.prepare_cached(&format!("SELECT {THREAD_COLUMNS} FROM thread t ORDER BY t.updated_at DESC, t.id DESC"))?;
        let rows = stmt.query_map([], stored)?;
        rows.collect()
    }

    pub fn get(&self, id: Id) -> rusqlite::Result<Option<Stored>> {
        let mut stmt = self.conn.prepare_cached(&format!("SELECT {THREAD_COLUMNS} FROM thread t WHERE t.id = ?1"))?;
        stmt.query_row([id], stored).optional()
    }

    pub fn create(&self, title: &str, provider: &str, model: Option<&str>, now: &str) -> rusqlite::Result<Id> {
        self.conn
            .prepare_cached("INSERT INTO thread (title, provider, model, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)")?
            .execute(params![title, provider, model, now])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename(&self, id: Id, title: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.prepare_cached("UPDATE thread SET title = ?2 WHERE id = ?1")?.execute(params![id, title])? > 0)
    }

    pub fn set_agent_ref(&self, id: Id, agent_ref: &str) -> rusqlite::Result<()> {
        self.conn.prepare_cached("UPDATE thread SET agent_ref = ?2 WHERE id = ?1")?.execute(params![id, agent_ref])?;
        Ok(())
    }

    pub fn delete(&self, id: Id) -> rusqlite::Result<bool> {
        Ok(self.conn.prepare_cached("DELETE FROM thread WHERE id = ?1")?.execute([id])? > 0)
    }

    /// Store a message and mark its thread used now.
    pub fn add(&self, thread: Id, role: &str, body: &Value, now: &str) -> rusqlite::Result<MessageView> {
        self.conn
            .prepare_cached("INSERT INTO message (thread_id, role, body, created_at) VALUES (?1, ?2, ?3, ?4)")?
            .execute(params![thread, role, body.to_string(), now])?;
        let id = self.conn.last_insert_rowid();
        self.conn.prepare_cached("UPDATE thread SET updated_at = ?2 WHERE id = ?1")?.execute(params![thread, now])?;
        Ok(MessageView { id, thread_id: thread, role: role.into(), body: body.clone(), created_at: now.into() })
    }

    pub fn messages(&self, thread: Id, after: Option<Id>) -> rusqlite::Result<Vec<MessageView>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, thread_id, role, body, created_at FROM message WHERE thread_id = ?1 AND id > ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![thread, after.unwrap_or(0)], message)?;
        rows.collect()
    }
}

fn sql(e: rusqlite::Error) -> BusError {
    BusError::internal(format!("threads: {e}"))
}

fn not_found(id: Id) -> BusError {
    BusError::not_found("thread.not_found", format!("No thread {id}"))
}

/// One running agent.
struct Agent {
    stdin: ChildStdin,
    pid: u32,
    /// Which spawn this is, so the reader of an agent that was replaced cannot remove the new one.
    serial: u64,
    last_used: Instant,
}

#[derive(Default)]
struct Live {
    agents: HashMap<Id, Agent>,
    /// Threads whose agent is writing a reply, from the person's message to Claude's `result`.
    working: HashSet<Id>,
    next_serial: u64,
}

/// The engine's threads: the store, opened on first use, and the agents running now.
#[derive(Default)]
pub struct Hub {
    db: Mutex<Option<ThreadStore>>,
    live: Mutex<Live>,
    reaper: OnceLock<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Runs `f` on `threads.db`, opening it beside the store on first use (in memory when the store
/// is, as in tests).
pub fn with_db<T>(engine: &Engine, f: impl FnOnce(&ThreadStore) -> rusqlite::Result<T>) -> Result<T, BusError> {
    let mut slot = lock(&engine.threads.db);
    if slot.is_none() {
        let store = engine.store.path();
        let opened = if store == Path::new(":memory:") {
            ThreadStore::open_in_memory()
        } else {
            ThreadStore::open(&store.with_file_name("threads.db"))
        };
        *slot = Some(opened.map_err(sql)?);
    }
    f(slot.as_ref().expect("opened above")).map_err(sql)
}

/// Where the agents run: Claude files its conversations by working directory, so a resume must
/// start in the same one.
fn agent_dir(engine: &Engine) -> PathBuf {
    let store = engine.store.path();
    if store == Path::new(":memory:") {
        std::env::temp_dir().join(format!("relay-threads-{}", std::process::id()))
    } else {
        store.with_file_name("threads")
    }
}

fn view(engine: &Engine, t: Stored) -> ThreadView {
    let live = lock(&engine.threads.live);
    ThreadView {
        working: live.working.contains(&t.id),
        live: live.agents.contains_key(&t.id),
        id: t.id,
        title: t.title,
        provider: t.provider,
        model: t.model,
        created_at: t.created_at,
        updated_at: t.updated_at,
        preview: t.preview,
    }
}

pub fn list(engine: &Engine) -> Result<Vec<ThreadView>, BusError> {
    let rows = with_db(engine, |db| db.list())?;
    Ok(rows.into_iter().map(|t| view(engine, t)).collect())
}

pub fn get(engine: &Engine, id: Id) -> Result<ThreadView, BusError> {
    let row = with_db(engine, |db| db.get(id))?.ok_or_else(|| not_found(id))?;
    Ok(view(engine, row))
}

pub fn messages(engine: &Engine, id: Id, after: Option<Id>) -> Result<Vec<MessageView>, BusError> {
    get(engine, id)?;
    with_db(engine, |db| db.messages(id, after))
}

/// A person's message, checked: trimmed, not empty, not too long.
pub fn checked_text(text: &str) -> Result<String, BusError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(BusError::invalid("thread.invalid", "Write something to send"));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(BusError::invalid("thread.invalid", format!("A message is at most {MAX_TEXT} characters")));
    }
    Ok(text.to_string())
}

pub fn create(engine: &Engine, title: &str, model: Option<&str>, now: &str) -> Result<ThreadView, BusError> {
    let id = with_db(engine, |db| db.create(title, "claude", model, now))?;
    get(engine, id)
}

pub fn rename(engine: &Engine, id: Id, title: &str) -> Result<ThreadView, BusError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE || title.contains(char::is_control) {
        return Err(BusError::invalid("thread.invalid", format!("A title is 1 to {MAX_TITLE} printable characters")));
    }
    if !with_db(engine, |db| db.rename(id, title))? {
        return Err(not_found(id));
    }
    get(engine, id)
}

/// Store the person's message and claim the thread's turn. The agent gets the message from
/// [`deliver`], after the handler returns.
pub fn post(engine: &Engine, id: Id, text: &str, now: &str) -> Result<MessageView, BusError> {
    get(engine, id)?;
    {
        let mut live = lock(&engine.threads.live);
        if !live.working.insert(id) {
            return Err(BusError::conflict("thread.busy", "The agent is still answering in this thread")
                .with_hint("wait for its reply, or stop it with thread.stop"));
        }
    }
    with_db(engine, |db| db.add(id, "user", &json!({"text": text}), now)).inspect_err(|_| {
        lock(&engine.threads.live).working.remove(&id);
    })
}

/// Stop the thread's agent, if it runs. Its reader sees the pipe close and records nothing.
pub fn stop(engine: &Engine, id: Id) -> bool {
    let mut live = lock(&engine.threads.live);
    let was_working = live.working.remove(&id);
    let Some(agent) = live.agents.remove(&id) else { return was_working };
    drop(live);
    kill(agent.pid);
    true
}

pub fn delete(engine: &Engine, id: Id) -> Result<(), BusError> {
    stop(engine, id);
    if !with_db(engine, |db| db.delete(id))? {
        return Err(not_found(id));
    }
    Ok(())
}

#[cfg(unix)]
fn kill(pid: u32) {
    // The agent leads its own process group (`process_group(0)`), so its MCP server goes too.
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        unsafe {
            libc::killpg(pid, libc::SIGTERM);
        }
    }
}

#[cfg(not(unix))]
fn kill(_pid: u32) {}

fn emit_changed(engine: &Engine, id: Id) {
    match get(engine, id) {
        Ok(thread) => engine.emit_system("thread.changed", json!({"thread": thread})),
        Err(_) => engine.emit_system("thread.changed", json!({"id": id, "deleted": true})),
    }
}

fn record(engine: &Engine, id: Id, role: &str, body: Value) {
    match with_db(engine, |db| db.add(id, role, &body, &crate::time::now())) {
        Ok(message) => engine.emit_system("thread.message", json!({"thread": id, "message": message})),
        Err(e) => tracing::warn!(thread = id, "storing a thread message failed: {}", e.message),
    }
}

/// End the turn with `why` recorded as an error message.
fn fail(engine: &Engine, id: Id, why: &str) {
    lock(&engine.threads.live).working.remove(&id);
    record(engine, id, "error", json!({"text": why}));
    emit_changed(engine, id);
}

/// Give the agent the person's message: to the running agent, or to one started for it.
/// Runs after the handler that stored the message has returned.
pub fn deliver(engine: &Arc<Engine>, id: Id, text: &str) {
    let line = format!("{}\n", json!({"type": "user", "message": {"role": "user", "content": text}}));
    {
        let mut live = lock(&engine.threads.live);
        if let Some(agent) = live.agents.get_mut(&id) {
            agent.last_used = Instant::now();
            if agent.stdin.write_all(line.as_bytes()).and_then(|()| agent.stdin.flush()).is_ok() {
                drop(live);
                emit_changed(engine, id);
                return;
            }
            // It died between turns; its reader will find it gone. Start another.
            if let Some(dead) = live.agents.remove(&id) {
                kill(dead.pid);
            }
        }
    }
    if let Err(why) = spawn(engine, id, &line) {
        fail(engine, id, &why);
        return;
    }
    emit_changed(engine, id);
}

/// The MCP server the agent is given: Relay's, as the person, answering only [`AGENT_OPS`].
fn mcp_config(engine: &Engine) -> String {
    json!({"mcpServers": {"relay": {
        "command": crate::hooks::relay_bin().display().to_string(),
        "args": ["--instance", engine.instance.as_str(), "--actor", "user", "mcp"],
        "env": {"RELAY_MCP_OPS": AGENT_OPS.join(",")},
    }}})
    .to_string()
}

/// The agent's command line for `thread`: resumed by `agent_ref` when Claude reported one.
pub fn agent_args(mcp: &str, model: Option<&str>, agent_ref: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
        "--include-partial-messages", "--strict-mcp-config", "--mcp-config", mcp,
        "--tools", "", "--allowedTools", "mcp__relay", "--setting-sources", "",
        "--append-system-prompt", PROMPT,
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    if let Some(model) = model.filter(|m| crate::providers::is_provider_ref(m)) {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(reference) = agent_ref.filter(|r| crate::providers::is_provider_ref(r)) {
        args.extend(["--resume".into(), reference.into()]);
    }
    args
}

fn spawn(engine: &Arc<Engine>, id: Id, first_line: &str) -> Result<(), String> {
    let thread = with_db(engine, |db| db.get(id)).map_err(|e| e.message)?.ok_or_else(|| format!("No thread {id}"))?;
    let claude = {
        let conn = engine.store.lock();
        crate::providers::executable(&conn, Provider::Claude)
    }
    .map_err(|e| format!("Claude is not available: {}", e.message))?;
    let dir = agent_dir(engine);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let mut cmd = Command::new(&claude);
    cmd.args(agent_args(&mcp_config(engine), thread.model.as_deref(), thread.agent_ref.as_deref()))
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A thread is not a session: nothing of an agent session the engine may run inside leaks in.
    for var in ["RELAY_SESSION", "RELAY_TOKEN", "RELAY_PROJECT_ID", "RELAY_WORKTREE", "RELAY_BRIEF", "RELAY_MCP_OPS"] {
        cmd.env_remove(var);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start Claude: {e}"))?;
    let (Some(mut stdin), Some(stdout), Some(mut stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return Err("Could not talk to Claude".into());
    };
    if let Err(e) = stdin.write_all(first_line.as_bytes()).and_then(|()| stdin.flush()) {
        kill(child.id());
        let _ = child.wait();
        return Err(format!("Could not talk to Claude: {e}"));
    }

    let serial = {
        let mut live = lock(&engine.threads.live);
        live.next_serial += 1;
        let serial = live.next_serial;
        if live.agents.len() >= MAX_LIVE {
            let oldest = live.agents.iter()
                .filter(|(t, _)| !live.working.contains(t))
                .min_by_key(|(_, a)| a.last_used)
                .map(|(t, _)| *t);
            if let Some(oldest) = oldest {
                if let Some(agent) = live.agents.remove(&oldest) {
                    kill(agent.pid);
                }
            }
        }
        live.agents.insert(id, Agent { stdin, pid: child.id(), serial, last_used: Instant::now() });
        serial
    };

    let tail = Arc::new(Mutex::new(String::new()));
    let tail_writer = tail.clone();
    std::thread::Builder::new()
        .name(format!("thread-{id}-stderr"))
        .spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 { break; }
                let mut t = lock(&tail_writer);
                t.push_str(&String::from_utf8_lossy(&buf[..n]));
                if t.len() > 4000 {
                    let from = t.len() - 2000;
                    let from = (from..t.len()).find(|i| t.is_char_boundary(*i)).unwrap_or(t.len());
                    t.drain(..from);
                }
            }
        })
        .map_err(|e| format!("Could not watch Claude: {e}"))?;
    let weak = Arc::downgrade(engine);
    std::thread::Builder::new()
        .name(format!("thread-{id}"))
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                let Some(engine) = weak.upgrade() else { break };
                handle(&engine, id, &event);
            }
            let _ = child.wait();
            if let Some(engine) = weak.upgrade() {
                ended(&engine, id, serial, &lock(&tail));
            }
        })
        .map_err(|e| format!("Could not watch Claude: {e}"))?;
    start_reaper(engine);
    Ok(())
}

/// The agent's output ended: it was stopped, closed for idling, or it died.
fn ended(engine: &Engine, id: Id, serial: u64, stderr: &str) {
    let (ours, working) = {
        let mut live = lock(&engine.threads.live);
        let ours = live.agents.get(&id).is_some_and(|a| a.serial == serial);
        if ours {
            live.agents.remove(&id);
        }
        // Only a turn this agent was still on is lost with it.
        (ours, ours && live.working.contains(&id))
    };
    if working {
        let detail = stderr.lines().rev().find(|l| !l.trim().is_empty()).map(|l| format!(": {}", cut(l.trim(), 300))).unwrap_or_default();
        fail(engine, id, &format!("The agent stopped before it finished{detail}"));
    } else if ours {
        emit_changed(engine, id);
    }
}

/// One line of Claude's stream-json output.
fn handle(engine: &Engine, id: Id, event: &Value) {
    // A sub-agent's traffic is its parent tool call's business, not the thread's.
    if !event["parent_tool_use_id"].is_null() {
        return;
    }
    match event["type"].as_str() {
        Some("system") if event["subtype"] == "init" => {
            if let Some(reference) = event["session_id"].as_str().filter(|r| crate::providers::is_provider_ref(r)) {
                let _ = with_db(engine, |db| db.set_agent_ref(id, reference));
            }
        }
        Some("stream_event") => {
            let e = &event["event"];
            if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                if let Some(text) = e["delta"]["text"].as_str() {
                    engine.emit_system("thread.delta", json!({"thread": id, "text": text}));
                }
            }
        }
        Some("assistant") => {
            let blocks: Vec<Value> = event["message"]["content"].as_array().into_iter().flatten().filter_map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().filter(|t| !t.trim().is_empty()).map(|t| json!({"type": "text", "text": t})),
                Some("tool_use") => Some(json!({"type": "tool_use", "id": b["id"], "name": b["name"], "input": b["input"]})),
                _ => None,
            }).collect();
            if !blocks.is_empty() {
                record(engine, id, "assistant", json!({"blocks": blocks}));
            }
        }
        Some("user") => {
            for b in event["message"]["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_result") {
                let text = match &b["content"] {
                    Value::String(s) => s.clone(),
                    Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
                    _ => String::new(),
                };
                record(engine, id, "tool", json!({
                    "tool_use_id": b["tool_use_id"], "is_error": b["is_error"].as_bool().unwrap_or(false), "text": cut(&text, MAX_TOOL_TEXT),
                }));
            }
        }
        Some("result") => {
            if event["is_error"] == true || event["subtype"].as_str().is_some_and(|s| s != "success") {
                let why = event["result"].as_str().filter(|r| !r.trim().is_empty()).map(str::to_string).or_else(|| {
                    event["errors"].as_array().map(|e| e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "))
                });
                fail(engine, id, &why.filter(|w| !w.is_empty()).unwrap_or_else(|| "The agent could not answer".into()));
            } else {
                {
                    let mut live = lock(&engine.threads.live);
                    live.working.remove(&id);
                    if let Some(agent) = live.agents.get_mut(&id) {
                        agent.last_used = Instant::now();
                    }
                }
                emit_changed(engine, id);
            }
        }
        _ => {}
    }
}

/// Close agents idle for [`IDLE`]; one watcher per engine, gone with it.
fn start_reaper(engine: &Arc<Engine>) {
    let weak: Weak<Engine> = Arc::downgrade(engine);
    if engine.threads.reaper.set(()).is_ok() {
        let _ = std::thread::Builder::new().name("thread-reaper".into()).spawn(move || loop {
            std::thread::sleep(Duration::from_secs(30));
            let Some(engine) = weak.upgrade() else { return };
            let idle: Vec<(Id, Agent)> = {
                let mut live = lock(&engine.threads.live);
                let ids: Vec<Id> = live.agents.iter()
                    .filter(|(t, a)| !live.working.contains(t) && a.last_used.elapsed() >= IDLE)
                    .map(|(t, _)| *t)
                    .collect();
                ids.into_iter().filter_map(|t| live.agents.remove(&t).map(|a| (t, a))).collect()
            };
            // Closing stdin ends the agent's input; it exits on its own.
            for (id, agent) in idle {
                drop(agent);
                emit_changed(&engine, id);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_come_from_the_first_line_cut_at_a_word() {
        assert_eq!(title_from("  \nHow are groceries?\nmore"), "How are groceries?");
        let long = "How did groceries go in September compared to August, by week please";
        let title = title_from(long);
        assert!(title.ends_with('…') && title.chars().count() <= 61, "{title}");
        assert!(!title.contains("please"));
        assert_eq!(title_from("   "), "New thread");
    }

    #[test]
    fn previews_read_text_blocks() {
        assert_eq!(preview(&json!({"text": "\nhello"})).as_deref(), Some("hello"));
        let blocks = json!({"blocks": [{"type": "tool_use", "id": "x"}, {"type": "text", "text": "You spent $520"}]});
        assert_eq!(preview(&blocks).as_deref(), Some("You spent $520"));
        assert_eq!(preview(&json!({"blocks": []})), None);
    }

    #[test]
    fn the_agent_resumes_by_its_own_id_and_never_by_an_option() {
        let args = agent_args("{}", Some("opus"), Some("eac9663f-1a29"));
        assert!(args.windows(2).any(|w| w == ["--resume", "eac9663f-1a29"]));
        assert!(args.windows(2).any(|w| w == ["--model", "opus"]));
        assert!(args.windows(2).any(|w| w == ["--tools", ""]), "no built-in tools");
        let args = agent_args("{}", Some("--dangerously-skip-permissions"), Some("-x"));
        assert!(!args.iter().any(|a| a == "--resume" || a == "--model"));
    }
}
