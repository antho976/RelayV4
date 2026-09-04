//! The engine: op handlers + the request pipeline (BUS.md §5).
//!
//! ```text
//! receive → parse → idempotency → actor → validate → authorize → policy → HANDLER → audit → events → respond
//! ```
//!
//! Handlers are synchronous and run inside one SQLite transaction with the store mutex held —
//! writes are serial by design (§5.1). Doors call [`Engine::dispatch`] from `spawn_blocking`.

use crate::audit::{self, AuditEntry};
use crate::device::{DeviceWatchState, MirrorRuntime, RunRuntime};
use crate::paths::Instance;
use crate::pty::Pty;
use crate::store::Store;
use relay_bus::envelope::{Actor, Event, MailHint, Request, Response, ENVELOPE_V};
use relay_bus::error::BusError;
use relay_bus::registry::{Audit, Doors, Op, OpEntry, OpKind, Registry};
use relay_bus::types::{Id, Page, PaneInfo, PaneRef, UndoOp, WindowInfo};
use rusqlite::Transaction;
use serde_json::{json, Value};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, Notify};
use uuid::Uuid;

/// Which door a request came through (BUS.md §6). Rules differ per door (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    Tauri,
    Socket,
    /// The in-process test harness: trusted, no token checks.
    InProcess,
}

#[derive(Debug)]
pub(crate) struct UiRuntime {
    pub project_id: Option<Id>,
    pub page: Page,
    pub panes: Vec<PaneInfo>,
    pub focused: Option<PaneRef>,
    pub windows: Vec<WindowInfo>,
    pub next_pane: u64,
    pub next_window: u64,
}

impl Default for UiRuntime {
    fn default() -> Self {
        Self {
            project_id: None,
            page: Page::Dashboard,
            panes: Vec::new(),
            focused: None,
            windows: vec![WindowInfo {
                window_id: "main".into(),
                main: true,
                panes: Vec::new(),
            }],
            next_pane: 1,
            next_window: 1,
        }
    }
}

/// What a handler sees. One per request; dropped after the response is built.
/// Anything over this and the request was, for that long, the reason nothing else could run —
/// one frame at 60 Hz. It is a reporting threshold, not a limit: the pipeline never interrupts a
/// handler, it just says which op did it (D144).
const LOCK_BUDGET: std::time::Duration = std::time::Duration::from_millis(16);

/// Store-lock accounting for one request. `wait` is how long it queued behind other handlers;
/// `held` is how long it kept every other op — keystrokes included — out of the connection.
/// Reports on `finish`, or on drop for the paths that leave the lock to scope exit.
struct LockSpan {
    op: &'static str,
    queued: Instant,
    wait: std::time::Duration,
    acquired: Instant,
    reported: bool,
}

impl LockSpan {
    fn opening(op: &'static str) -> LockSpan {
        LockSpan {
            op,
            queued: Instant::now(),
            wait: std::time::Duration::ZERO,
            acquired: Instant::now(),
            reported: false,
        }
    }
    /// Call the instant the lock is in hand.
    fn acquired(&mut self) {
        self.wait = self.queued.elapsed();
        self.acquired = Instant::now();
    }
    fn finish(mut self) {
        self.report();
    }
    fn report(&mut self) {
        if self.reported {
            return;
        }
        self.reported = true;
        let held = self.acquired.elapsed();
        if held >= LOCK_BUDGET || self.wait >= LOCK_BUDGET {
            tracing::warn!(
                op = self.op,
                wait_ms = self.wait.as_secs_f64() * 1000.0,
                held_ms = held.as_secs_f64() * 1000.0,
                "store lock over budget"
            );
        } else {
            tracing::debug!(
                op = self.op,
                wait_ms = self.wait.as_secs_f64() * 1000.0,
                held_ms = held.as_secs_f64() * 1000.0,
                "store lock"
            );
        }
    }
}

impl Drop for LockSpan {
    fn drop(&mut self) {
        self.report();
    }
}

/// The context for an op that must **not** hold the store lock while it works (D144, D149): anything
/// that shells out, walks a tree, or talks to the network. There is no open transaction. The
/// handler takes the connection itself, in short bursts, through [`Unlocked::read`], and does the
/// slow part with nothing locked.
///
/// Only queries register this way, so there is no audit row, no undo envelope and no transaction
/// to fail — which is exactly why the split is safe to make without touching the write pipeline.
pub struct Unlocked<'a> {
    engine: &'a Engine,
    /// Set only when this phase is running nested inside a transaction that already holds the
    /// store (`guardrail.confirm` replaying a held op). Then [`Unlocked::read`] uses that
    /// connection instead of taking the lock again — a `std::sync::Mutex` is not reentrant, and
    /// the nested case is rare enough to be worth exactly the cost it had before the split.
    held: Option<&'a rusqlite::Connection>,
    pub actor: Actor,
    pub req_id: Uuid,
    pub now: String,
    pub op: &'static str,
    session_id: Option<Id>,
    project_id: Option<Id>,
    events: Vec<Event>,
    deferred: Vec<Box<dyn FnOnce(Arc<Engine>) + Send + 'static>>,
}

impl Unlocked<'_> {
    pub fn engine(&self) -> &Engine {
        self.engine
    }
    pub fn instance(&self) -> Instance {
        self.engine.instance
    }
    /// The bound session id when the actor is an agent (BUS.md §4.2), else `None`.
    pub fn actor_session_id(&self) -> Option<Id> {
        self.session_id
    }
    /// Take the store for one short read. Keep the closure to plain queries: whatever runs in
    /// here is back on the one global lock, which is the thing this context exists to avoid.
    pub fn read<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<T, BusError>,
    ) -> Result<T, BusError> {
        if let Some(conn) = self.held {
            return f(conn);
        }
        let mut span = LockSpan::opening(self.op);
        let conn = self.engine.store.lock();
        span.acquired();
        let out = f(&conn);
        drop(conn);
        span.finish();
        out
    }
    /// Queue an event; emitted once the handler returns.
    pub fn emit(&mut self, ev: &str, payload: Value) {
        let mut e =
            Event::new(ev, self.now.clone(), self.actor.clone(), payload).caused_by(self.req_id);
        e.project_id = self.project_id;
        self.events.push(e);
    }
    pub fn set_project(&mut self, id: Id) {
        self.project_id = Some(id);
    }
    /// Run once the handler returns. The name matches [`Ctx::after_commit`] so the same helpers
    /// serve both contexts; there is no transaction here, so "after commit" is simply "after".
    pub fn after_commit(&mut self, f: impl FnOnce(Arc<Engine>) + Send + 'static) {
        self.deferred.push(Box::new(f));
    }
}

pub struct Ctx<'a> {
    tx: &'a Transaction<'a>,
    engine: &'a Engine,
    pub actor: Actor,
    pub req_id: Uuid,
    pub now: String,
    pub op: &'static str,
    payload: Value,
    skip_policy: Option<String>,
    events: Vec<Event>,
    undo: Option<UndoOp>,
    undo_of: Option<Id>,
    mark_undone: Option<Id>,
    project_id: Option<Id>,
    session_id: Option<Id>,
    /// A guardrail gate may need its hold/refusal and audit row committed together even
    /// though the bus response is an error (BUS.md §5.1 / §9.2).
    commit_error: bool,
    hold_id: Option<Id>,
    /// Confirmation is audited as the frozen op with the original actor in
    /// `on_behalf_of`, while retaining the confirm request id (BUS.md §9.4).
    audit_op: Option<String>,
    on_behalf_of: Option<Actor>,
    after_commit: Vec<Box<dyn FnOnce(Arc<Engine>) + Send + 'static>>,
    /// What the op's read/external phase produced, for handlers registered with
    /// [`Engine::register_staged`]. Taken once, by the matching `finish` closure.
    staged: Option<Box<dyn Any + Send>>,
}

impl<'a> Ctx<'a> {
    pub fn tx(&self) -> &'a Transaction<'a> {
        self.tx
    }
    pub fn engine(&self) -> &Engine {
        self.engine
    }
    pub fn instance(&self) -> Instance {
        self.engine.instance
    }
    /// The bound session id when the actor is an agent (BUS.md §4.2), else `None`.
    pub fn actor_session_id(&self) -> Option<Id> {
        self.session_id
    }
    pub(crate) fn payload(&self) -> &Value {
        &self.payload
    }
    pub(crate) fn skip_policy(&self) -> Option<&str> {
        self.skip_policy.as_deref()
    }
    /// Queue an event; emitted after commit, stamped with ts/actor/cause (BUS.md §1.3).
    pub fn emit(&mut self, ev: &str, payload: Value) {
        let mut e =
            Event::new(ev, self.now.clone(), self.actor.clone(), payload).caused_by(self.req_id);
        e.project_id = self.project_id;
        self.events.push(e);
    }
    /// Tag the audit row (and subsequent events) with a project.
    pub fn set_project(&mut self, id: Id) {
        self.project_id = Some(id);
    }
    pub fn set_session(&mut self, id: Id) {
        self.session_id = Some(id);
    }
    /// Commit this typed error instead of rolling the handler transaction back.
    pub fn commit_error(&mut self, hold_id: Option<Id>) {
        self.commit_error = true;
        self.hold_id = hold_id;
    }
    /// Change only the audit attribution for this request (guardrail confirmation).
    pub fn audit_as(&mut self, op: impl Into<String>, on_behalf_of: Actor) {
        self.audit_op = Some(op.into());
        self.on_behalf_of = Some(on_behalf_of);
    }
    /// Record the inverse envelope for `audit.undo` (BUS.md §5.5).
    pub fn set_undo(&mut self, op: &str, payload: Value, expect: Option<Value>) {
        self.undo = Some(UndoOp {
            op: op.to_string(),
            payload,
            expect,
        });
    }
    /// Mark this request as the inverse of an earlier audit row. The pipeline links both
    /// rows only after the undo audit row exists, keeping the foreign keys atomic.
    pub fn set_undo_of(&mut self, audit_id: Id) {
        self.undo_of = Some(audit_id);
        self.mark_undone = Some(audit_id);
    }
    /// Run after the transaction commits (side effects that must not precede state).
    pub fn after_commit(&mut self, f: impl FnOnce(Arc<Engine>) + Send + 'static) {
        self.after_commit.push(Box::new(f));
    }
    /// The value this op's read/external phase produced. Exactly one caller per request.
    fn take_staged<S: Any + Send>(&mut self) -> Result<S, BusError> {
        self.staged
            .take()
            .and_then(|value| value.downcast::<S>().ok())
            .map(|value| *value)
            .ok_or_else(|| BusError::internal(format!("{} ran without its staged phase", self.op)))
    }
    /// Call another typed handler without leaving this transaction. This is used only by
    /// composite bus operations whose contract explicitly includes the nested action.
    pub(crate) fn invoke_registered(
        &mut self,
        op: &str,
        payload: Value,
    ) -> Result<Value, BusError> {
        let entry = Registry::global()
            .get(op)
            .ok_or_else(|| BusError::unknown_op(op))?;
        (entry.validate)(&payload).map_err(|e| BusError::schema(entry.name, e))?;
        let handler = self
            .engine
            .handlers
            .get(entry.name)
            .ok_or_else(|| BusError::not_implemented(entry.name, entry.meta.phase))?
            .call
            .clone();
        // A staged op reached this way — `guardrail.confirm` replaying a held `git.commit` — has
        // no read/external phase behind it, so run it here, against the transaction already open.
        // It costs what the op cost before the split, and only on this path.
        let staged = match self.engine.prepares.get(entry.name).cloned() {
            Some(prepare) => {
                let mut stage = Unlocked {
                    engine: self.engine,
                    held: Some(self.tx),
                    actor: self.actor.clone(),
                    req_id: self.req_id,
                    now: self.now.clone(),
                    op: entry.name,
                    session_id: self.session_id,
                    project_id: self.project_id,
                    events: Vec::new(),
                    deferred: Vec::new(),
                };
                let out = prepare(&mut stage, &payload)?;
                let Unlocked {
                    events, deferred, ..
                } = stage;
                self.events.extend(events);
                self.after_commit.extend(deferred);
                Some(out)
            }
            None => None,
        };
        let prior = self.op;
        let prior_payload = std::mem::replace(&mut self.payload, payload.clone());
        let prior_staged = std::mem::replace(&mut self.staged, staged);
        self.op = entry.name;
        let result = handler(self, payload);
        self.op = prior;
        self.payload = prior_payload;
        self.staged = prior_staged;
        result
    }

    pub(crate) fn replay_registered(
        &mut self,
        op: &str,
        payload: Value,
        actor: Actor,
        skip_policy: String,
    ) -> Result<Value, BusError> {
        let prior_actor = std::mem::replace(&mut self.actor, actor);
        let prior_policy = self.skip_policy.replace(skip_policy);
        let result = self.invoke_registered(op, payload);
        self.actor = prior_actor;
        self.skip_policy = prior_policy;
        result
    }
}

type HandlerFn = Arc<dyn Fn(&mut Ctx, Value) -> Result<Value, BusError> + Send + Sync>;

struct Handler {
    call: HandlerFn,
}

type UnlockedFn = Arc<dyn Fn(&mut Unlocked, Value) -> Result<Value, BusError> + Send + Sync>;

type PrepareFn =
    Arc<dyn Fn(&mut Unlocked, &Value) -> Result<Box<dyn Any + Send>, BusError> + Send + Sync>;

/// The engine. Build with [`Engine::new`] (registers phase-1 handlers) and share as `Arc`.
pub struct Engine {
    pub instance: Instance,
    pub store: Store,
    handlers: HashMap<&'static str, Handler>,
    /// Queries that run with no transaction open (D149). Kept in their own map so the write
    /// pipeline below stays exactly one shape.
    unlocked: HashMap<&'static str, UnlockedFn>,
    /// The read/external phase of a staged mutation, run before the transaction opens (D149).
    prepares: HashMap<&'static str, PrepareFn>,
    events_tx: broadcast::Sender<Event>,
    started: Instant,
    pub ui_connected: AtomicBool,
    quit: Notify,
    quitting: AtomicBool,
    pub socket_path: std::sync::Mutex<Option<String>>,
    self_ref: std::sync::Mutex<Option<std::sync::Weak<Engine>>>,
    /// Live PTYs by session id (phase 3). Not persisted: a restart makes sessions restorable.
    ptys: std::sync::Mutex<HashMap<Id, Arc<Pty>>>,
    /// Session name → id for the live PTYs above, so `session.input` can resolve a keystroke's
    /// target without a `SELECT` (D148).
    pty_ids: std::sync::Mutex<HashMap<String, Id>>,
    pub(crate) mirrors: std::sync::Mutex<HashMap<Id, Arc<MirrorRuntime>>>,
    pub(crate) device_runs: std::sync::Mutex<HashMap<Id, Arc<RunRuntime>>>,
    pub(crate) device_watch: std::sync::Mutex<DeviceWatchState>,
    pub(crate) next_mirror: AtomicI64,
    pub(crate) watchers: std::sync::Mutex<HashMap<String, notify::RecommendedWatcher>>,
    pub(crate) watcher_registrations: std::sync::Mutex<HashSet<String>>,
    pub(crate) ui: std::sync::Mutex<UiRuntime>,
    pub(crate) resource_watch: AtomicBool,
    pub(crate) resource_cpu: std::sync::Mutex<HashMap<i64, (u64, Instant)>>,
    pub(crate) resource_disk: std::sync::Mutex<HashMap<String, (f64, Option<f64>)>>,
}

impl Engine {
    pub fn new(instance: Instance, store: Store) -> Arc<Engine> {
        let (events_tx, _) = broadcast::channel(4096);
        let mut engine = Engine {
            instance,
            store,
            handlers: HashMap::new(),
            unlocked: HashMap::new(),
            prepares: HashMap::new(),
            events_tx,
            started: Instant::now(),
            ui_connected: AtomicBool::new(false),
            quit: Notify::new(),
            quitting: AtomicBool::new(false),
            socket_path: std::sync::Mutex::new(None),
            self_ref: std::sync::Mutex::new(None),
            ptys: std::sync::Mutex::new(HashMap::new()),
            pty_ids: std::sync::Mutex::new(HashMap::new()),
            mirrors: std::sync::Mutex::new(HashMap::new()),
            device_runs: std::sync::Mutex::new(HashMap::new()),
            device_watch: std::sync::Mutex::new(DeviceWatchState::default()),
            next_mirror: AtomicI64::new(1),
            watchers: std::sync::Mutex::new(HashMap::new()),
            watcher_registrations: std::sync::Mutex::new(HashSet::new()),
            ui: std::sync::Mutex::new(UiRuntime::default()),
            resource_watch: AtomicBool::new(false),
            resource_cpu: std::sync::Mutex::new(HashMap::new()),
            resource_disk: std::sync::Mutex::new(HashMap::new()),
        };
        crate::handlers::register_all(&mut engine);
        let arc = Arc::new(engine);
        *arc.self_ref.lock().unwrap() = Some(Arc::downgrade(&arc));
        arc
    }

    /// Register a typed handler for `O`. Payload validation (BUS.md §5.2) is the typed
    /// deserialization with `deny_unknown_fields`.
    pub fn register<O: Op>(
        &mut self,
        f: impl Fn(&mut Ctx, O::Payload) -> Result<O::Result, BusError> + Send + Sync + 'static,
    ) {
        debug_assert!(
            Registry::global().get(O::NAME).is_some(),
            "{} is not in the registry",
            O::NAME
        );
        let call: HandlerFn = Arc::new(move |ctx: &mut Ctx, v: Value| {
            let p: O::Payload =
                serde_json::from_value(v).map_err(|e| BusError::schema(O::NAME, e))?;
            let r = f(ctx, p)?;
            serde_json::to_value(r)
                .map_err(|e| BusError::internal(format!("serializing {} result: {e}", O::NAME)))
        });
        self.handlers.insert(O::NAME, Handler { call });
    }

    /// Register `O` as an op that runs with no transaction open. Restricted to queries: an
    /// unlocked handler has no transaction to write into and no audit row to append (D149).
    pub fn register_unlocked<O: Op>(
        &mut self,
        f: impl Fn(&mut Unlocked, O::Payload) -> Result<O::Result, BusError> + Send + Sync + 'static,
    ) {
        debug_assert!(
            Registry::global().get(O::NAME).is_some(),
            "{} is not in the registry",
            O::NAME
        );
        debug_assert!(
            matches!(O::META.kind, OpKind::Query),
            "{} mutates; it cannot run outside the transaction",
            O::NAME
        );
        let call: UnlockedFn = Arc::new(move |ctx: &mut Unlocked, v: Value| {
            let p: O::Payload =
                serde_json::from_value(v).map_err(|e| BusError::schema(O::NAME, e))?;
            let r = f(ctx, p)?;
            serde_json::to_value(r)
                .map_err(|e| BusError::internal(format!("serializing {} result: {e}", O::NAME)))
        });
        self.unlocked.insert(O::NAME, call);
    }

    /// Register a mutation as three phases instead of one (D149): `prepare` reads what it needs
    /// in short bursts and does the slow external work — a subprocess, a network call, a tree
    /// walk — with the store lock released; `finish` then runs inside the request's transaction
    /// with what `prepare` produced, and is expected to be short.
    ///
    /// Events and deferred work queued during `prepare` are carried into the request, so they
    /// still fire after the commit, in order.
    pub fn register_staged<O: Op, S: Any + Send>(
        &mut self,
        prepare: impl Fn(&mut Unlocked, &O::Payload) -> Result<S, BusError> + Send + Sync + 'static,
        finish: impl Fn(&mut Ctx, O::Payload, S) -> Result<O::Result, BusError> + Send + Sync + 'static,
    ) {
        debug_assert!(
            Registry::global().get(O::NAME).is_some(),
            "{} is not in the registry",
            O::NAME
        );
        let stage: PrepareFn = Arc::new(move |ctx: &mut Unlocked, v: &Value| {
            let p: O::Payload =
                serde_json::from_value(v.clone()).map_err(|e| BusError::schema(O::NAME, e))?;
            Ok(Box::new(prepare(ctx, &p)?) as Box<dyn Any + Send>)
        });
        self.prepares.insert(O::NAME, stage);
        let call: HandlerFn = Arc::new(move |ctx: &mut Ctx, v: Value| {
            let p: O::Payload =
                serde_json::from_value(v).map_err(|e| BusError::schema(O::NAME, e))?;
            let staged = ctx.take_staged::<S>()?;
            let r = finish(ctx, p, staged)?;
            serde_json::to_value(r)
                .map_err(|e| BusError::internal(format!("serializing {} result: {e}", O::NAME)))
        });
        self.handlers.insert(O::NAME, Handler { call });
    }

    pub fn is_implemented(&self, op: &str) -> bool {
        self.handlers.contains_key(op) || self.unlocked.contains_key(op)
    }
    pub fn uptime_s(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events_tx.subscribe()
    }
    /// Emit a system-originated event (reconcile, watchers). Never fails.
    pub fn emit_system(&self, ev: &str, payload: Value) {
        let e = Event::new(ev, crate::time::now(), Actor::System, payload);
        let _ = self.events_tx.send(e);
    }
    pub fn request_quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        self.quit.notify_waiters();
        self.quit.notify_one();
    }
    pub async fn wait_quit(&self) {
        if self.quitting.load(Ordering::SeqCst) {
            return;
        }
        self.quit.notified().await;
    }
    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }
    pub fn arc(&self) -> Option<Arc<Engine>> {
        self.self_ref
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|w| w.upgrade())
    }

    // ------------------------------------------------------------------ PTY registry

    pub fn pty(&self, session_id: Id) -> Option<Arc<Pty>> {
        self.ptys.lock().unwrap().get(&session_id).cloned()
    }
    /// The live PTY for a session name, if it has one. `None` also means "not in the map" —
    /// callers that need to tell an unknown session from an unspawned one must ask the store.
    pub fn pty_named(&self, name: &str) -> Option<(Id, Arc<Pty>)> {
        let id = *self.pty_ids.lock().unwrap().get(name)?;
        self.ptys.lock().unwrap().get(&id).cloned().map(|pty| (id, pty))
    }
    pub fn set_pty(&self, session_id: Id, name: &str, pty: Arc<Pty>) -> Option<Arc<Pty>> {
        self.pty_ids
            .lock()
            .unwrap()
            .insert(name.to_string(), session_id);
        self.ptys.lock().unwrap().insert(session_id, pty)
    }
    pub fn take_pty(&self, session_id: Id) -> Option<Arc<Pty>> {
        self.pty_ids.lock().unwrap().retain(|_, id| *id != session_id);
        self.ptys.lock().unwrap().remove(&session_id)
    }
    pub fn live_pty_count(&self) -> usize {
        self.ptys
            .lock()
            .unwrap()
            .values()
            .filter(|p| !p.exited())
            .count()
    }
    /// Kill every child (SPEC §14 persistence model: after a restart PTYs are dead and the
    /// resume flow takes over) and mark their sessions restorable. Idempotent.
    pub fn shutdown(&self) {
        for (_, mirror) in self.mirrors.lock().unwrap().drain() {
            mirror.stop();
        }
        let runs: Vec<_> = self.device_runs.lock().unwrap().drain().collect();
        for (_, run) in &runs {
            run.stop();
        }
        if !runs.is_empty() {
            let now = crate::time::now();
            let conn = self.store.lock();
            let _ = conn.execute(
                "UPDATE device_runs SET state='stopped',finished_at=COALESCE(finished_at,?1) WHERE state IN ('building','running')",
                [now],
            );
        }
        self.pty_ids.lock().unwrap().clear();
        let ptys: Vec<(Id, Arc<Pty>)> = self.ptys.lock().unwrap().drain().collect();
        if ptys.is_empty() {
            return;
        }
        // mark first, then kill: the exit callback only touches live states, so it stays a no-op
        self.mark_restorable(&ptys);
        for (_, p) in &ptys {
            p.kill(std::time::Duration::from_secs(2));
        }
    }

    fn mark_restorable(&self, ptys: &[(Id, Arc<Pty>)]) {
        let now = crate::time::now();
        let mut conn = self.store.lock();
        let tx = match conn.transaction() {
            Ok(tx) => tx,
            Err(_) => return,
        };
        for (id, pty) in ptys {
            let (text, epoch, seq) = pty.scrollback(None);
            let _ = tx.execute(
                "INSERT INTO session_scrollback(session_id,text,epoch,seq,updated_at) VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(session_id) DO UPDATE SET text=excluded.text,epoch=excluded.epoch,seq=excluded.seq,updated_at=excluded.updated_at",
                rusqlite::params![id, text.as_bytes(), epoch as i64, seq as i64, now],
            );
            let _ = tx.execute(
                "UPDATE sessions SET state='restorable',pid=NULL,restore_reason='app_restart',updated_at=?1
                 WHERE id=?2 AND state IN ('spawning','running','idle','blocked')",
                rusqlite::params![now, id],
            );
        }
        let _ = tx.commit();
    }

    /// A system-originated mutation outside any request (process exit, watcher, recovery):
    /// runs `f` in a transaction, appends an audit row as `system` (`op`, `parent_req` per
    /// BUS.md §5.1) and emits the events `f` returns. Never called with the store lock held.
    pub fn system_write<T>(
        &self,
        op: &str,
        parent_req: Option<Uuid>,
        project_id: Option<Id>,
        session_id: Option<Id>,
        summary: Value,
        f: impl FnOnce(&Transaction, &str) -> Result<(T, Vec<(String, Value)>), BusError>,
    ) -> Result<T, BusError> {
        let now = crate::time::now();
        let mut conn = self.store.lock();
        let tx = conn.transaction().map_err(internal)?;
        let (out, events) = f(&tx, &now)?;
        let req_id = Uuid::new_v4();
        let e = AuditEntry {
            ts: &now,
            req_id,
            parent_req,
            actor: &Actor::System,
            on_behalf_of: None,
            session_id,
            op,
            project_id,
            payload: &Value::Object(Default::default()),
            outcome: &Ok(summary),
            hold_id: None,
            undo_op: None,
            undo_of: None,
        };
        audit::append(&tx, &e).map_err(internal)?;
        tx.commit().map_err(internal)?;
        drop(conn);
        for (ev, payload) in events {
            let mut e = Event::new(ev, now.clone(), Actor::System, payload);
            e.project_id = project_id;
            let _ = self.events_tx.send(e);
        }
        Ok(out)
    }

    // ------------------------------------------------------------------ pipeline

    /// Parse a raw line/JSON into a request, or the `bus.parse` response for it (§1.2).
    pub fn parse(raw: &str) -> Result<Request, Response> {
        let v: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => return Err(Response::unparsed(BusError::parse(e))),
        };
        let id = v
            .get("id")
            .and_then(|i| i.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        match serde_json::from_value::<Request>(v) {
            Ok(r) => Ok(r),
            Err(e) => Err(match id {
                Some(id) => Response::err(id, BusError::envelope(e)),
                None => Response::unparsed(BusError::envelope(e)),
            }),
        }
    }

    /// The pipeline. Synchronous; holds the store lock for the handler's duration.
    pub fn dispatch(&self, req: Request, door: Door) -> Response {
        let id = req.id;
        let mut response = match self.dispatch_inner(&req, door) {
            Ok(resp) => resp,
            Err(e) => Response::err(id, e),
        };
        // Priority mail is current session state, not part of an operation's typed result or
        // audit record. Resolve the actor again after the transaction so acknowledgements and
        // idempotent replays always carry the live count. Authentication failures get no hint.
        if let Ok(Some(session_id)) = self.resolve_actor(&req, door) {
            if let Ok(priority) = self.unread_priority_count(session_id) {
                if priority > 0 {
                    response.mail = Some(MailHint { priority });
                }
            }
        }
        response
    }

    fn dispatch_inner(&self, req: &Request, door: Door) -> Result<Response, BusError> {
        // envelope
        if req.v != ENVELOPE_V {
            return Err(BusError::envelope(format!(
                "v {} is not supported (this engine speaks v{ENVELOPE_V})",
                req.v
            )));
        }
        if !req.payload.is_object() {
            return Err(BusError::envelope("payload must be an object"));
        }
        let entry = Registry::global()
            .get(&req.op)
            .ok_or_else(|| BusError::unknown_op(&req.op))?;
        // door
        match (entry.meta.doors, door) {
            (Doors::SocketOnly, Door::Tauri) | (Doors::TauriOnly, Door::Socket) => {
                return Err(BusError::invalid(
                    "bus.door",
                    format!("{} is not available on this door", entry.name),
                ));
            }
            _ => {}
        }
        // actor
        let session_id = self.resolve_actor(req, door)?;
        // An agent already *is* a project and a session. Making it restate both on every call
        // turned each op into guess-and-retry, so the engine fills the two identity fields
        // when an op requires them and the caller left them out (D110). Only required fields
        // are filled: an optional `project_id` is usually one half of an either/or.
        let filled = self.fill_identity(req, entry, session_id)?;
        let req = filled.as_ref().unwrap_or(req);
        let audited = should_audit(entry, &req.actor);
        // idempotency
        if audited {
            let conn = self.store.lock();
            if let Some(rec) = audit::lookup(&conn, req.id).map_err(internal)? {
                if rec.payload_hash != audit::payload_hash(&req.payload) {
                    return Err(BusError::conflict(
                        "bus.id_reused",
                        "this request id was already used with a different payload",
                    ));
                }
                return Ok(rec.response.replayed());
            }
        }
        // validate (before authorization — BUS.md §5; for every op, implemented or not)
        (entry.validate)(&req.payload).map_err(|e| BusError::schema(entry.name, e))?;
        // authorize — fixed op-level allowlist, then the session role/options/scope.
        if !entry.meta.actors.admits(&req.actor) {
            let err = BusError::allowlist(
                entry.name,
                if req.actor.is_agent() {
                    "agent"
                } else {
                    "anonymous"
                },
            )
            .with_hint("this op is user-only (BUS.md §9.1 layer 1)");
            if audited {
                self.audit_only(req, entry, session_id, &Err(err.clone()))?;
            }
            return Err(err);
        }
        if let Some(sid) = session_id {
            let conn = self.store.lock();
            if let Err(err) = crate::guardrail::authorize(&conn, sid, entry, &req.payload) {
                drop(conn);
                if audited {
                    self.audit_only(req, entry, session_id, &Err(err.clone()))?;
                }
                return Err(err);
            }
        }
        // Keystrokes never touch SQLite (D148). The store is one connection behind one mutex, so
        // a per-character request that opened a transaction queued behind whatever slow op held
        // it — which is exactly why typing stalled while a git scan or `adb devices` ran. Both
        // ops are `UserOnly` with `AgentOnly` audit, so by this point nothing is left to
        // persist: the envelope, door, payload and actor allowlist checks have all run, the
        // actor is privileged (never a session), and `audited` is false.
        if matches!(entry.name, "session.input" | "session.resize") && !audited && session_id.is_none()
        {
            if let Some(result) = self.pty_fast_path(entry.name, &req.payload)? {
                return Ok(Response::ok(req.id, result));
            }
        }
        // Slow reads run with nothing locked (D149). Queries are never audited, so there is no
        // row to append and no transaction to keep open across an `adb`, `gh` or `gix` call.
        if let Some(h) = self.unlocked.get(entry.name).cloned() {
            return self.dispatch_unlocked(req, entry, session_id, h);
        }
        // Implementation availability is deliberately checked after authorization: an
        // unfinished surface must not become a side door around role/scope policy.
        let Some(h) = self.handlers.get(entry.name) else {
            let err = BusError::not_implemented(entry.name, entry.meta.phase);
            if audited {
                self.audit_only(req, entry, session_id, &Err(err.clone()))?;
            }
            return Err(err);
        };

        // The read/external phase of a staged mutation, before anything is locked (D149).
        let (staged, staged_events, staged_after) = match self.prepares.get(entry.name).cloned() {
            Some(prepare) => {
                let mut stage = Unlocked {
                    engine: self,
                    held: None,
                    actor: req.actor.clone(),
                    req_id: req.id,
                    now: crate::time::now(),
                    op: entry.name,
                    session_id,
                    project_id: None,
                    events: Vec::new(),
                    deferred: Vec::new(),
                };
                let out = prepare(&mut stage, &req.payload);
                let Unlocked {
                    events, deferred, ..
                } = stage;
                match out {
                    Ok(value) => (Some(value), events, deferred),
                    Err(err) => {
                        // Same rule as an ordinary handler failure below: audited, unless the
                        // request was simply malformed.
                        if audited && err.kind != relay_bus::ErrorKind::Invalid {
                            self.audit_only(req, entry, session_id, &Err(err.clone()))?;
                        }
                        return Err(err);
                    }
                }
            }
            None => (None, Vec::new(), Vec::new()),
        };
        // handler, in a transaction
        let mut span = LockSpan::opening(entry.name);
        let mut conn = self.store.lock();
        span.acquired();
        let tx = conn.transaction().map_err(internal)?;
        let now = crate::time::now();
        let mut ctx = Ctx {
            tx: &tx,
            engine: self,
            actor: req.actor.clone(),
            req_id: req.id,
            now: now.clone(),
            op: entry.name,
            payload: req.payload.clone(),
            skip_policy: None,
            events: staged_events,
            undo: None,
            undo_of: None,
            mark_undone: None,
            project_id: None,
            session_id,
            commit_error: false,
            hold_id: None,
            audit_op: None,
            on_behalf_of: None,
            after_commit: staged_after,
            staged,
        };
        let outcome = (h.call)(&mut ctx, req.payload.clone());
        let Ctx {
            events,
            undo,
            undo_of,
            mark_undone,
            project_id,
            session_id,
            commit_error,
            hold_id,
            audit_op,
            on_behalf_of,
            after_commit,
            ..
        } = ctx;
        let audit_op = audit_op.as_deref().unwrap_or(entry.name);

        match outcome {
            Ok(result) => {
                if audited {
                    let e = AuditEntry {
                        ts: &now,
                        req_id: req.id,
                        parent_req: None,
                        actor: &req.actor,
                        on_behalf_of: on_behalf_of.as_ref(),
                        session_id,
                        op: audit_op,
                        project_id,
                        payload: &req.payload,
                        outcome: &Ok(result.clone()),
                        hold_id,
                        undo_op: undo.as_ref(),
                        undo_of,
                    };
                    let audit_id = audit::append(&tx, &e).map_err(internal)?;
                    if let Some(original) = mark_undone {
                        tx.execute(
                            "UPDATE audit SET undone_by=?1 WHERE id=?2 AND undone_by IS NULL",
                            rusqlite::params![audit_id, original],
                        )
                        .map_err(internal)?;
                    }
                }
                tx.commit().map_err(internal)?;
                drop(conn);
                span.finish();
                for ev in events {
                    let _ = self.events_tx.send(ev);
                }
                if !after_commit.is_empty() {
                    if let Some(arc) = self.arc() {
                        for f in after_commit {
                            f(arc.clone());
                        }
                    }
                }
                Ok(Response::ok(req.id, result))
            }
            Err(err) => {
                if commit_error {
                    if audited {
                        let e = AuditEntry {
                            ts: &now,
                            req_id: req.id,
                            parent_req: None,
                            actor: &req.actor,
                            on_behalf_of: on_behalf_of.as_ref(),
                            session_id,
                            op: audit_op,
                            project_id,
                            payload: &req.payload,
                            outcome: &Err(err.clone()),
                            hold_id,
                            undo_op: None,
                            undo_of: None,
                        };
                        audit::append(&tx, &e).map_err(internal)?;
                    }
                    tx.commit().map_err(internal)?;
                    drop(conn);
                    for ev in events {
                        let _ = self.events_tx.send(ev);
                    }
                    if !after_commit.is_empty() {
                        if let Some(arc) = self.arc() {
                            for f in after_commit {
                                f(arc.clone());
                            }
                        }
                    }
                    return Err(err);
                }
                // ordinary failure: roll back handler work; audit in its own transaction
                tx.rollback().map_err(internal)?;
                if audited && err.kind != relay_bus::ErrorKind::Invalid {
                    let tx = conn.transaction().map_err(internal)?;
                    let e = AuditEntry {
                        ts: &now,
                        req_id: req.id,
                        parent_req: None,
                        actor: &req.actor,
                        on_behalf_of: on_behalf_of.as_ref(),
                        session_id,
                        op: audit_op,
                        project_id,
                        payload: &req.payload,
                        outcome: &Err(err.clone()),
                        hold_id,
                        undo_op: None,
                        undo_of: None,
                    };
                    audit::append(&tx, &e).map_err(internal)?;
                    tx.commit().map_err(internal)?;
                }
                Err(err)
            }
        }
    }

    /// Write an audit row for a request that never reached a handler (refused / unavailable).
    fn audit_only(
        &self,
        req: &Request,
        entry: &OpEntry,
        session_id: Option<Id>,
        outcome: &Result<Value, BusError>,
    ) -> Result<(), BusError> {
        let now = crate::time::now();
        let mut conn = self.store.lock();
        let project_id = session_id.and_then(|session_id| {
            conn.query_row(
                "SELECT project_id FROM sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .ok()
        });
        let tx = conn.transaction().map_err(internal)?;
        let e = AuditEntry {
            ts: &now,
            req_id: req.id,
            parent_req: None,
            actor: &req.actor,
            on_behalf_of: None,
            session_id,
            op: entry.name,
            project_id,
            payload: &req.payload,
            outcome,
            hold_id: None,
            undo_op: None,
            undo_of: None,
        };
        audit::append(&tx, &e).map_err(internal)?;
        tx.commit().map_err(internal)
    }

    /// Fill `project_id` / `session` from the authenticated agent when the op requires them
    /// and the payload omits them. Returns `None` when nothing needed filling, so the common
    /// path never clones the request.
    fn fill_identity(
        &self,
        req: &Request,
        entry: &relay_bus::registry::OpEntry,
        session_id: Option<Id>,
    ) -> Result<Option<Request>, BusError> {
        let Some(session_id) = session_id else { return Ok(None) };
        let object = req.payload.as_object().expect("payload is an object");
        let registry = Registry::global();
        let wants = |field: &str| {
            !object.contains_key(field) && registry.payload_requires(entry.name, field)
        };
        let (needs_project, needs_session) = (wants("project_id"), wants("session"));
        if !needs_project && !needs_session {
            return Ok(None);
        }
        let conn = self.store.lock();
        let row = crate::sessions::by_id(&conn, session_id)?
            .ok_or_else(|| BusError::actor("bound session vanished"))?;
        drop(conn);
        let mut next = req.clone();
        let object = next.payload.as_object_mut().expect("payload is an object");
        if needs_project {
            object.insert("project_id".into(), Value::from(row.session.project_id));
        }
        if needs_session {
            object.insert("session".into(), Value::from(row.session.name.clone()));
        }
        Ok(Some(next))
    }

    /// BUS.md §4.2. Returns the agent's session id when the actor is a bound agent.
    /// Run a query registered with [`Engine::register_unlocked`]: no store lock held for the
    /// handler's duration, events emitted and deferred work run once it returns.
    fn dispatch_unlocked(
        &self,
        req: &Request,
        entry: &OpEntry,
        session_id: Option<Id>,
        h: UnlockedFn,
    ) -> Result<Response, BusError> {
        let mut ctx = Unlocked {
            engine: self,
            held: None,
            actor: req.actor.clone(),
            req_id: req.id,
            now: crate::time::now(),
            op: entry.name,
            session_id,
            project_id: None,
            events: Vec::new(),
            deferred: Vec::new(),
        };
        let outcome = h(&mut ctx, req.payload.clone());
        let Unlocked {
            events, deferred, ..
        } = ctx;
        // A failed query publishes nothing, the same way a rolled-back handler does.
        let result = outcome?;
        for ev in events {
            let _ = self.events_tx.send(ev);
        }
        if !deferred.is_empty() {
            if let Some(arc) = self.arc() {
                for f in deferred {
                    f(arc.clone());
                }
            }
        }
        Ok(Response::ok(req.id, result))
    }

    /// `session.input` / `session.resize` against a live PTY, with no store lock and no
    /// transaction. `Ok(None)` means "not resolvable from memory" — an unknown or unspawned
    /// session — and the request falls through to the registered handler, which owns the exact
    /// typed error for that case.
    fn pty_fast_path(&self, op: &str, payload: &Value) -> Result<Option<Value>, BusError> {
        use relay_bus::ops::session::{InputIn, ResizeIn};
        let name = payload.get("session").and_then(Value::as_str).unwrap_or_default();
        let Some((session_id, pty)) = self.pty_named(name) else {
            return Ok(None);
        };
        match op {
            "session.input" => {
                let p: InputIn = serde_json::from_value(payload.clone())
                    .map_err(|e| BusError::schema("session.input", e))?;
                if pty.exited() {
                    return Err(BusError::conflict(
                        "session.exited",
                        format!("session {} has exited", p.session),
                    ));
                }
                pty.write(p.data.as_bytes())
                    .map_err(|e| BusError::conflict("session.io", e.to_string()))?;
                // The one keystroke per idle period that owes the store a row: hand it to a
                // worker so the character itself is already in the PTY either way.
                if !p.data.is_empty() && pty.claim_idle_edge() {
                    if let Some(engine) = self.arc() {
                        std::thread::spawn(move || engine.mark_session_running(session_id));
                    }
                }
            }
            "session.resize" => {
                let p: ResizeIn = serde_json::from_value(payload.clone())
                    .map_err(|e| BusError::schema("session.resize", e))?;
                pty.resize(p.cols, p.rows)
                    .map_err(|e| BusError::conflict("session.io", e.to_string()))?;
            }
            _ => return Ok(None),
        }
        Ok(Some(json!({})))
    }

    /// The deferred half of the input fast path: move an idle session back to `running` and tell
    /// the UI. Off the request thread, so a keystroke never waits for the store.
    fn mark_session_running(&self, session_id: Id) {
        let _ = self.system_write(
            "session.input",
            None,
            None,
            Some(session_id),
            Value::Null,
            |tx, now| {
                let changed = tx
                    .execute(
                        "UPDATE sessions SET state='running',updated_at=?1 WHERE id=?2 AND state='idle'",
                        rusqlite::params![now, session_id],
                    )
                    .map_err(internal)?;
                if changed == 0 {
                    return Ok(((), Vec::new()));
                }
                let events = match crate::sessions::by_id(tx, session_id)? {
                    Some(row) => vec![(
                        "session.changed".to_string(),
                        serde_json::to_value(&row.session).unwrap_or(Value::Null),
                    )],
                    None => Vec::new(),
                };
                Ok(((), events))
            },
        );
    }

    fn resolve_actor(&self, req: &Request, door: Door) -> Result<Option<Id>, BusError> {
        match (&req.actor, door) {
            (Actor::System, _) => Err(BusError::actor(
                "\"system\" is internal and cannot be claimed",
            )),
            (Actor::Test, _) if !self.instance.accepts_test_actor() => Err(BusError::actor(
                "\"test\" actor is only accepted by dev/test instances",
            )),
            (Actor::User, _) | (Actor::Test, _) => Ok(None),
            (Actor::Agent(_), Door::Tauri) => {
                Err(BusError::actor("the UI door only carries the user actor"))
            }
            (Actor::Agent(name), Door::InProcess) => {
                let conn = self.store.lock();
                Ok(crate::sessions::by_name(&conn, name)
                    .ok()
                    .map(|r| r.session.id))
            }
            (Actor::Agent(name), Door::Socket) => {
                let Some(token) = req.token.as_deref() else {
                    return Err(BusError::actor(format!(
                        "agent:{name} requires a token on the socket door"
                    )));
                };
                self.check_session_token(name, token)
            }
        }
    }

    /// BUS.md §4.2: `agent:<name>` + token must match a non-closed session.
    fn check_session_token(&self, name: &str, token: &str) -> Result<Option<Id>, BusError> {
        let conn = self.store.lock();
        let row = crate::sessions::by_name(&conn, name)
            .map_err(|_| BusError::actor(format!("unknown session {name:?}")))?;
        if row.token.is_empty() || row.token != token {
            return Err(BusError::actor(format!("bad token for session {name:?}")));
        }
        Ok(Some(row.session.id))
    }

    fn unread_priority_count(&self, session_id: Id) -> Result<u32, BusError> {
        let conn = self.store.lock();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_recipients r
                 JOIN messages m ON m.id = r.message_id
                 WHERE r.session_id = ?1 AND r.acked_at IS NULL AND m.priority = 1",
                [session_id],
                |row| row.get(0),
            )
            .map_err(internal)?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}

/// BUS.md §5.1a.
pub fn should_audit(entry: &OpEntry, actor: &Actor) -> bool {
    match (entry.meta.kind, entry.meta.audit) {
        (OpKind::Query, _) => false,
        (OpKind::Mutation, Audit::Always) => true,
        (OpKind::Mutation, Audit::AgentOnly) => actor.is_agent(),
        (OpKind::Mutation, Audit::Never) => false,
    }
}

/// Map an infrastructure error to `internal`, logging it.
pub fn internal(e: impl std::fmt::Display) -> BusError {
    tracing::error!(error = %e, "internal error");
    BusError::internal(e.to_string())
}

/// `?` sugar for `anyhow`/`rusqlite` errors inside handlers.
pub trait IntoBus<T> {
    fn bus(self) -> Result<T, BusError>;
}
impl<T, E: std::fmt::Display> IntoBus<T> for Result<T, E> {
    fn bus(self) -> Result<T, BusError> {
        self.map_err(internal)
    }
}
