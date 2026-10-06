//! The Unix-socket door (BUS.md §6.2): newline-delimited JSON, one `Request` per line,
//! responses correlated by id, `bus.subscribe` turns the connection into a subscriber.
//! One engine per instance, enforced with a lock file + probe.

use crate::engine::{Door, Engine};
use crate::paths::Instance;
use anyhow::{Context, Result};
use relay_bus::envelope::{Event, Frame, Response, ENVELOPE_V};
use relay_bus::error::BusError;
use relay_bus::ops::bus::{SubscribeIn, SubscribeOut, WaitIn, WaitOut};
use relay_bus::registry::Op;
use relay_bus::{Empty, Request};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Why we could not become the engine for this instance.
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error("engine already running for instance {instance} (pid {pid:?}) at {socket}")]
    AlreadyRunning {
        instance: Instance,
        pid: Option<u32>,
        socket: PathBuf,
    },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// A bound, listening socket door. Drop = unlink socket + release lock.
pub struct SocketServer {
    pub path: PathBuf,
    _lock: File,
    accept: Option<JoinHandle<()>>,
}

impl SocketServer {
    /// Take the instance lock, evict a stale socket, bind, and start accepting — in the
    /// instance's runtime dir (BUS.md §6.2).
    pub async fn start(engine: Arc<Engine>) -> Result<SocketServer, BindError> {
        let dir = engine.instance.runtime_dir();
        Self::start_in(engine, dir).await
    }

    /// Same, in an explicit directory (tests; `relay serve --runtime-dir`).
    pub async fn start_in(engine: Arc<Engine>, dir: PathBuf) -> Result<SocketServer, BindError> {
        let inst = engine.instance;
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let lock_path = dir.join(format!("{}.lock", inst.as_str()));
        let sock_path = dir.join(format!("{}.sock", inst.as_str()));

        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
            .with_context(|| format!("opening {}", lock_path.display()))?;
        let rc = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            // Someone holds the lock. Is their engine alive?
            let pid = std::fs::read_to_string(&lock_path)
                .ok()
                .and_then(|s| s.trim().parse().ok());
            if probe(&sock_path).await {
                return Err(BindError::AlreadyRunning {
                    instance: inst,
                    pid,
                    socket: sock_path,
                });
            }
            // Lock held but socket dead: a crashed engine's child, or a hung process. Refuse
            // rather than fight over the store (BUS.md §6.2 says take over only for a *stale*
            // socket, i.e. nobody holds the lock).
            return Err(BindError::AlreadyRunning {
                instance: inst,
                pid,
                socket: sock_path,
            });
        }
        // We hold the lock: anything at the socket path is stale.
        if sock_path.exists() {
            let _ = std::fs::remove_file(&sock_path);
        }
        {
            use std::io::Write;
            let mut l = &lock;
            let _ = l.set_len(0);
            let _ = write!(l, "{}", std::process::id());
        }
        let listener = UnixListener::bind(&sock_path)
            .with_context(|| format!("binding {}", sock_path.display()))?;
        std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| "chmod socket")?;
        *engine.socket_path.lock().unwrap() = Some(sock_path.display().to_string());
        tracing::info!(socket = %sock_path.display(), instance = %inst, "socket door open");

        let eng = engine.clone();
        let accept = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let e = eng.clone();
                        tokio::spawn(async move {
                            if let Err(err) = handle_conn(e, stream).await {
                                tracing::debug!(error = %err, "connection ended");
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "accept failed");
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        });
        Ok(SocketServer {
            path: sock_path,
            _lock: lock,
            accept: Some(accept),
        })
    }
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        if let Some(h) = self.accept.take() {
            h.abort();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Is there a live engine behind this socket? (connect + `bus.ping`, 500 ms budget)
pub async fn probe(path: &Path) -> bool {
    let fut = async {
        let stream = UnixStream::connect(path).await.ok()?;
        let (r, mut w) = stream.into_split();
        let req = Request::new(relay_bus::Actor::User, "bus.ping", serde_json::json!({}));
        let mut line = serde_json::to_string(&req).ok()?;
        line.push('\n');
        w.write_all(line.as_bytes()).await.ok()?;
        let mut lines = BufReader::new(r).lines();
        let resp = lines.next_line().await.ok()??;
        let resp: Response = serde_json::from_str(&resp).ok()?;
        Some(resp.ok)
    };
    matches!(
        tokio::time::timeout(std::time::Duration::from_millis(500), fut).await,
        Ok(Some(true))
    )
}

/// How much queued output one `write_all` may carry. Large enough to swallow a burst from
/// every attached terminal, small enough that a slow reader never parks megabytes here.
const WRITE_BATCH_BYTES: usize = 256 * 1024;

/// The constant prefix of every `pty` frame on one attachment: the session name is escaped
/// once, here, rather than on every frame.
fn pty_frame_head(session: &str) -> String {
    let name = serde_json::to_string(session).unwrap_or_else(|_| String::from("\"\""));
    format!("{{\"v\":{ENVELOPE_V},\"stream\":\"pty\",\"session\":{name}")
}

/// One `pty` frame as its wire line, formatted in a single pass over its own bytes.
///
/// `serde_json` would first allocate a `String` for the base64 payload, wrap it in a `Value`,
/// then re-scan all of it looking for characters JSON must escape. Base64's alphabet contains
/// none of them and every other field here is a number or a fixed token, so the line can be
/// built directly — on the one path that carries every byte every agent ever prints.
fn pty_frame_line(head: &str, epoch: u64, seq: u64, data: &[u8]) -> String {
    use base64::Engine as _;
    use std::fmt::Write as _;
    let mut line = String::with_capacity(head.len() + 48 + data.len().div_ceil(3) * 4);
    line.push_str(head);
    let _ = write!(line, ",\"epoch\":{epoch},\"seq\":{seq},\"data\":\"");
    base64::engine::general_purpose::STANDARD.encode_string(data, &mut line);
    line.push_str("\"}");
    line
}

/// `bus.wait {matching}`: every top-level key given must be present in the event payload with
/// an equal value. Lets a waiter take *its* answer and not the first one of the same name.
fn payload_matches(payload: &serde_json::Value, matching: Option<&serde_json::Value>) -> bool {
    let Some(wanted) = matching.and_then(serde_json::Value::as_object) else { return true };
    wanted.iter().all(|(key, value)| payload.get(key) == Some(value))
}

/// The exception request a `bus.wait` is waiting on: `guardrail.request_resolved` with a
/// `request_id` to match. That answer is stored, so a wait begun after it can still see it.
fn exception_waited(filter: &Filter, matching: Option<&serde_json::Value>) -> Option<relay_bus::types::Id> {
    if filter.0.is_empty() || !filter.matches(crate::guardrail::grants::RESOLVED_EVENT) {
        return None;
    }
    matching?.get("request_id")?.as_i64()
}

/// Event filter for `bus.subscribe {events}`: exact, `prefix.*`, or `*`. Empty = everything.
#[derive(Clone, Default)]
struct Filter(Vec<String>);

impl Filter {
    fn matches(&self, ev: &str) -> bool {
        self.0.is_empty()
            || self.0.iter().any(|p| {
                p == "*"
                    || p == ev
                    || p.strip_suffix(".*")
                        .map(|pre| ev.starts_with(pre) && ev[pre.len()..].starts_with('.'))
                        .unwrap_or(false)
            })
    }
}

// A native mirror belongs to its transport connection. Drop also runs on I/O error
// and task cancellation, so a vanished window cannot leave capture running.
struct MirrorAttachment {
    engine: Arc<Engine>,
    runtime: Arc<crate::device::MirrorRuntime>,
    task: JoinHandle<()>,
}
impl Drop for MirrorAttachment {
    fn drop(&mut self) {
        self.task.abort();
        // Out of the registry before the stop flag flips: anything that sees the mirror
        // stopped must also no longer find it.
        self.engine.mirrors.lock().unwrap().remove(&self.runtime.id);
        self.runtime.finish(crate::device::MirrorState::Stopped, None, None);
        self.runtime.stop();
        self.engine.emit_system(
            "mirror.changed",
            serde_json::json!({"mirror_id":self.runtime.id,"state":"stopped"}),
        );
    }
}

struct WatchLease {
    resources: bool,
    engine: Arc<Engine>,
    active: std::sync::Mutex<bool>,
}
impl Drop for WatchLease {
    fn drop(&mut self) {
        if !*self.active.lock().unwrap() {
            return;
        }
        if self.resources {
            let mut clients = self.engine.resource_watch_clients.lock().unwrap();
            *clients = clients.saturating_sub(1);
            if *clients == 0 {
                self.engine
                    .resource_watch
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
            return;
        }
        let runtime = {
            let mut watch = self.engine.device_watch.lock().unwrap();
            watch.clients = watch.clients.saturating_sub(1);
            if watch.clients == 0 {
                watch.runtime.take()
            } else {
                None
            }
        };
        if let Some(runtime) = runtime {
            runtime.stop();
        }
    }
}

/// PTY sizes this connection borrowed with `session.resize {until_detach}`: a phone fits an
/// agent's terminal to its own screen while it looks at it. Each is handed back when the
/// connection detaches from the session or goes away, however it goes, unless someone resized
/// the PTY since: their size is the newer wish.
#[derive(Default)]
struct Borrowed(std::collections::HashMap<String, BorrowedSize>);
struct BorrowedSize {
    pty: Arc<crate::pty::Pty>,
    before: (u16, u16),
    set: (u16, u16),
}
impl Borrowed {
    /// After a successful resize: `prior` is the PTY and its size before it.
    fn resized(&mut self, p: &relay_bus::ops::session::ResizeIn, prior: Option<(Arc<crate::pty::Pty>, (u16, u16))>) {
        let Some((pty, before)) = prior else { return };
        if p.until_detach != Some(true) {
            // A size set for good ends the loan; there is nothing left to hand back.
            self.0.remove(&p.session);
            return;
        }
        let set = (p.cols.max(2), p.rows.max(2));
        match self.0.get_mut(&p.session) {
            // Still the same process: keep the size from before the first borrow.
            Some(held) if Arc::ptr_eq(&held.pty, &pty) => held.set = set,
            _ => {
                self.0.insert(p.session.clone(), BorrowedSize { pty, before, set });
            }
        }
    }
    fn release(&mut self, session: &str) {
        if let Some(held) = self.0.remove(session) {
            held.hand_back();
        }
    }
}
impl BorrowedSize {
    fn hand_back(self) {
        if !self.pty.exited() && self.pty.size() == self.set {
            let _ = self.pty.resize(self.before.0, self.before.1);
        }
    }
}
impl Drop for Borrowed {
    fn drop(&mut self) {
        for (_, held) in self.0.drain() {
            held.hand_back();
        }
    }
}

async fn handle_conn(engine: Arc<Engine>, stream: UnixStream) -> Result<()> {
    // The blocking dispatch keeps this Arc until its ownership update completes,
    // even if the socket task is cancelled while that dispatch is in flight.
    let device_watch = Arc::new(WatchLease {
        resources: false,
        engine: engine.clone(),
        active: std::sync::Mutex::new(false),
    });
    let resource_watch = Arc::new(WatchLease {
        resources: true,
        engine: engine.clone(),
        active: std::sync::Mutex::new(false),
    });
    let (r, mut w) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(1024);
    let writer = tokio::spawn(async move {
        // Terminal output arrives as a burst of small lines, one per PTY read across every
        // attached session. Writing each of them separately is one syscall per frame; whatever
        // is already queued behind the first goes out with it instead.
        let mut batch: Vec<u8> = Vec::with_capacity(WRITE_BATCH_BYTES);
        while let Some(line) = out_rx.recv().await {
            batch.clear();
            batch.extend_from_slice(line.as_bytes());
            batch.push(b'\n');
            while batch.len() < WRITE_BATCH_BYTES {
                match out_rx.try_recv() {
                    Ok(next) => {
                        batch.extend_from_slice(next.as_bytes());
                        batch.push(b'\n');
                    }
                    Err(_) => break,
                }
            }
            if w.write_all(&batch).await.is_err() {
                break;
            }
            // One oversized frame must not hold its buffer for the rest of the connection.
            if batch.capacity() > WRITE_BATCH_BYTES * 4 {
                batch = Vec::with_capacity(WRITE_BATCH_BYTES);
            }
        }
    });
    let mut lines = BufReader::new(r).lines();
    let mut forwarder: Option<JoinHandle<()>> = None;
    // pty streams attached on this connection, by session name
    let mut attached: std::collections::HashMap<String, JoinHandle<()>> =
        std::collections::HashMap::new();
    // device.run is a socket-capable op; its bounded log stream follows the response.
    let mut attached_runs: std::collections::HashMap<relay_bus::types::Id, JoinHandle<()>> =
        std::collections::HashMap::new();

    let mut attached_mirrors: std::collections::HashMap<relay_bus::types::Id, MirrorAttachment> =
        std::collections::HashMap::new();
    // Dropped with the connection, which is when a borrowed size goes back.
    let mut borrowed = Borrowed::default();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req = match Engine::parse(&line) {
            Ok(r) => r,
            Err(resp) => {
                let _ = out_tx.send(serde_json::to_string(&resp)?).await;
                continue;
            }
        };
        let resp = if matches!(req.op.as_str(), "device.watch" | "app.resources.watch") {
            let lease = if req.op == "device.watch" {
                device_watch.clone()
            } else {
                resource_watch.clone()
            };
            tokio::task::spawn_blocking(move || {
                let mut active = lease.active.lock().unwrap();
                let on = req.payload["on"].as_bool();
                let response = lease.engine.dispatch_socket_watch(req, *active);
                if response.ok {
                    *active = on.expect("successful Watch validated its boolean payload");
                }
                response
            })
            .await?
        } else if req.op == relay_bus::ops::bus::Subscribe::NAME {
            match serde_json::from_value::<SubscribeIn>(req.payload.clone()) {
                Err(e) => Response::err(req.id, BusError::schema(&req.op, e)),
                Ok(p) => {
                    if let Some(h) = forwarder.take() {
                        h.abort();
                    }
                    let filter = Filter(p.events.clone().unwrap_or_default());
                    let mut rx = engine.subscribe();
                    let tx = out_tx.clone();
                    let f = filter.clone();
                    forwarder = Some(tokio::spawn(async move {
                        loop {
                            match rx.recv().await {
                                Ok(ev) => {
                                    if f.matches(&ev.ev) {
                                        if let Ok(s) = serde_json::to_string(&ev) {
                                            if tx.send(s).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                    tracing::warn!(lagged = n, "subscriber lagged; events dropped");
                                }
                                Err(_) => break,
                            }
                        }
                    }));
                    let subscribed = if filter.0.is_empty() {
                        vec!["*".to_string()]
                    } else {
                        filter.0.clone()
                    };
                    Response::ok(req.id, serde_json::to_value(SubscribeOut { subscribed })?)
                }
            }
        } else if req.op == relay_bus::ops::bus::Wait::NAME {
            // The wake-up an agent has instead of a poll loop. It blocks on the event
            // broadcast inside this connection's task — no timer anywhere, and the engine's
            // store lock is never held while waiting (SPEC §15, D115).
            match serde_json::from_value::<WaitIn>(req.payload.clone()) {
                Err(e) => Response::err(req.id, BusError::schema(&req.op, e)),
                Ok(p) => {
                    let filter = Filter(p.events.clone().unwrap_or_default());
                    let timeout = Duration::from_millis(
                        p.timeout_ms.unwrap_or(60_000).clamp(1_000, 3_600_000),
                    );
                    let mut rx = engine.subscribe();
                    // Subscribed first, so an answer is either already stored or still to come.
                    let answered = match exception_waited(&filter, p.matching.as_ref()) {
                        Some(id) => {
                            let e = engine.clone();
                            tokio::task::spawn_blocking(move || {
                                crate::guardrail::grants::resolved_event(&e.store.lock(), id)
                            })
                            .await?
                            .filter(|ev| payload_matches(&ev.payload, p.matching.as_ref()))
                        }
                        None => None,
                    };
                    let waited = tokio::time::timeout(timeout, async {
                        if answered.is_some() {
                            return answered;
                        }
                        loop {
                            match rx.recv().await {
                                Ok(ev) if filter.matches(&ev.ev) && payload_matches(&ev.payload, p.matching.as_ref()) => return Some(ev),
                                Ok(_) => continue,
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                    tracing::warn!(lagged = n, "waiter lagged; events dropped");
                                }
                                Err(_) => return None,
                            }
                        }
                    })
                    .await;
                    let (event, timed_out) = match waited {
                        Ok(event) => (event, false),
                        Err(_) => (None, true),
                    };
                    match serde_json::to_value(WaitOut { event, timed_out }) {
                        Ok(value) => Response::ok(req.id, value),
                        Err(e) => Response::err(req.id, BusError::internal(e.to_string())),
                    }
                }
            }
        } else if req.op == relay_bus::ops::bus::Unsubscribe::NAME {
            if let Some(h) = forwarder.take() {
                h.abort();
            }
            Response::ok(req.id, serde_json::to_value(Empty {})?)
        } else if req.op == relay_bus::ops::session::Attach::NAME {
            // control-plane validation first, then the data plane (BUS.md §7)
            let e = engine.clone();
            let r2 = req.clone();
            let resp = tokio::task::spawn_blocking(move || e.dispatch(r2, Door::Socket)).await?;
            if resp.ok {
                let p: relay_bus::ops::session::AttachIn = serde_json::from_value(
                    req.payload.clone(),
                )
                .unwrap_or(relay_bus::ops::session::AttachIn {
                    session: String::new(),
                    from_seq: None,
                    epoch: None,
                });
                match crate::handlers::session::pty_by_name(&engine, &p.session) {
                    Ok((_, pty)) => {
                        if let Some(h) = attached.remove(&p.session) {
                            h.abort();
                        }
                        let att = pty.attach(p.epoch, p.from_seq);
                        let tx = out_tx.clone();
                        let name = p.session.clone();
                        attached.insert(p.session.clone(), tokio::spawn(async move {
                            let head = pty_frame_head(&name);
                            let mut rx = att.rx;
                            if !att.catch_up.is_empty() {
                                let line = pty_frame_line(&head, att.epoch, att.seq, &att.catch_up);
                                if tx.send(line).await.is_err() { return; }
                            }
                            loop {
                                match rx.recv().await {
                                    Ok(fr) => {
                                        if fr.seq <= att.seq && fr.epoch == att.epoch { continue; } // already in catch-up
                                        let line = pty_frame_line(&head, fr.epoch, fr.seq, &fr.data);
                                        if tx.send(line).await.is_err() { break; }
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                        tracing::warn!(lagged = n, session = %name, "pty subscriber lagged; frames dropped — client should re-attach from its last seq");
                                    }
                                    Err(_) => break,
                                }
                            }
                        }));
                    }
                    Err(e) => {
                        let _ = out_tx
                            .send(serde_json::to_string(&Response::err(req.id, e))?)
                            .await;
                        continue;
                    }
                }
            }
            resp
        } else if req.op == relay_bus::ops::session::Detach::NAME {
            let e = engine.clone();
            let r2 = req.clone();
            let resp = tokio::task::spawn_blocking(move || e.dispatch(r2, Door::Socket)).await?;
            if resp.ok {
                if let Some(name) = req.payload.get("session").and_then(|v| v.as_str()) {
                    if let Some(h) = attached.remove(name) {
                        h.abort();
                    }
                    borrowed.release(name);
                }
            }
            resp
        } else if req.op == relay_bus::ops::session::Resize::NAME {
            let parsed = serde_json::from_value::<relay_bus::ops::session::ResizeIn>(req.payload.clone()).ok();
            let prior = parsed.as_ref().and_then(|p| engine.pty_named(&p.session)).map(|(_, pty)| {
                let size = pty.size();
                (pty, size)
            });
            let resp = if engine.answers_from_memory(&req) {
                engine.dispatch(req, Door::Socket)
            } else {
                let e = engine.clone();
                tokio::task::spawn_blocking(move || e.dispatch(req, Door::Socket)).await?
            };
            if let (true, Some(p)) = (resp.ok, parsed) {
                borrowed.resized(&p, prior);
            }
            resp
        } else if req.op == relay_bus::ops::device::MirrorStart::NAME {
            let e = engine.clone();
            let request = req.clone();
            let response =
                tokio::task::spawn_blocking(move || e.dispatch(request, Door::Socket)).await?;
            if response.ok {
                if let Some(id) = response
                    .result
                    .as_ref()
                    .and_then(|v| v["mirror_id"].as_i64())
                {
                    // Frames on this connection are of two kinds, told apart by `data`: a base64
                    // string is one video packet; an object is the mirror's status (state, picture
                    // size, device name, and on the way out the typed code and message). The
                    // status goes first, then on every change, and a terminal one ends the
                    // stream — the window learns that the device went away from the stream it is
                    // already reading, without subscribing to anything (it used to wait forever).
                    let status_frame = move |status: &crate::device::MirrorStatus| {
                        serde_json::to_string(&Frame {
                            v: 1,
                            stream: "mirror".into(),
                            session: None,
                            run_id: None,
                            mirror_id: Some(id),
                            epoch: None,
                            seq: 0,
                            data: serde_json::to_value(status).unwrap_or_default(),
                        })
                    };
                    match crate::handlers::device::mirror_by_id(&engine, id) {
                        Ok(runtime) => {
                            let (cursor, history, mut rx) = runtime.attach();
                            let mut status = runtime.watch_status();
                            let tx = out_tx.clone();
                            let task = tokio::spawn(async move {
                                use base64::Engine as _;
                                let frame = |item: crate::device::MirrorChunk| {
                                    serde_json::to_string(&Frame {
                                        v: 1,
                                        stream: "mirror".into(),
                                        session: None,
                                        run_id: None,
                                        mirror_id: Some(id),
                                        epoch: None,
                                        seq: item.seq,
                                        data: serde_json::Value::String(
                                            base64::engine::general_purpose::STANDARD
                                                .encode(&*item.data),
                                        ),
                                    })
                                };
                                let first = status.borrow_and_update().clone();
                                let Ok(line) = status_frame(&first) else { return };
                                if tx.send(line).await.is_err() {
                                    return;
                                }
                                if first.state.is_terminal() {
                                    return;
                                }
                                for item in history {
                                    if let Ok(line) = frame(item) {
                                        if tx.send(line).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                loop {
                                    tokio::select! {
                                        biased;
                                        changed = status.changed() => {
                                            if changed.is_err() {
                                                break;
                                            }
                                            let now = status.borrow_and_update().clone();
                                            let Ok(line) = status_frame(&now) else { break };
                                            if tx.send(line).await.is_err() || now.state.is_terminal() {
                                                break;
                                            }
                                        }
                                        item = rx.recv() => match item {
                                            Ok(item) if item.seq > cursor => {
                                                if let Ok(line) = frame(item) {
                                                    if tx.send(line).await.is_err() {
                                                        break;
                                                    }
                                                }
                                            }
                                            Ok(_) => {}
                                            // A slow window skipped packets. The seq gap tells it so;
                                            // it asks the device for a fresh key frame and resumes —
                                            // no reason to end a mirror over one hiccup.
                                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                                tracing::warn!(lagged = n, mirror_id = id, "mirror subscriber lagged; packets dropped");
                                            }
                                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                        },
                                    }
                                }
                            });
                            attached_mirrors.insert(
                                id,
                                MirrorAttachment {
                                    engine: engine.clone(),
                                    runtime,
                                    task,
                                },
                            );
                        }
                        // The worker already ended (a failure faster than this attach). Say so
                        // rather than leave the window on "Connecting".
                        Err(_) => {
                            let ended = crate::device::MirrorStatus {
                                state: crate::device::MirrorState::Failed,
                                code: Some("device.mirror_ended".into()),
                                message: Some("The mirror ended before it could start. Retry to see why.".into()),
                                ..Default::default()
                            };
                            let _ = out_tx.send(serde_json::to_string(&response)?).await;
                            if let Ok(line) = status_frame(&ended) {
                                let _ = out_tx.send(line).await;
                            }
                            continue;
                        }
                    }
                }
            }
            response
        } else if req.op == relay_bus::ops::device::RunOp::NAME
            || req.op == relay_bus::ops::device::Build::NAME
        {
            let e = engine.clone();
            let r2 = req.clone();
            let resp = tokio::task::spawn_blocking(move || e.dispatch(r2, Door::Socket)).await?;
            if resp.ok {
                if let Some(id) = resp
                    .result
                    .as_ref()
                    .and_then(|value| value.get("id"))
                    .and_then(|value| value.as_i64())
                {
                    match crate::handlers::device::run_by_id(&engine, id) {
                        Ok(runtime) => {
                            if let Some(task) = attached_runs.remove(&id) {
                                task.abort();
                            }
                            let (cursor, history, mut rx) = runtime.attach();
                            let tx = out_tx.clone();
                            attached_runs.insert(id, tokio::spawn(async move {
                                for item in history {
                                    let frame = Frame {
                                        v: 1,
                                        stream: "logcat".into(),
                                        session: None,
                                        run_id: Some(id),
                                        mirror_id: None,
                                        epoch: None,
                                        seq: item.seq,
                                        data: serde_json::Value::String((*item.line).clone()),
                                    };
                                    if let Ok(line) = serde_json::to_string(&frame) {
                                        if tx.send(line).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                loop {
                                    match rx.recv().await {
                                        Ok(item) if item.seq > cursor => {
                                            let frame = Frame {
                                                v: 1,
                                                stream: "logcat".into(),
                                                session: None,
                                                run_id: Some(id),
                                                mirror_id: None,
                                                epoch: None,
                                                seq: item.seq,
                                                data: serde_json::Value::String((*item.line).clone()),
                                            };
                                            if let Ok(line) = serde_json::to_string(&frame) {
                                                if tx.send(line).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                        Ok(_) => {}
                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                            tracing::warn!(lagged = n, run_id = id, "device run subscriber lagged; log lines dropped");
                                        }
                                        Err(_) => break,
                                    }
                                }
                            }));
                        }
                        Err(error) => {
                            let _ = out_tx
                                .send(serde_json::to_string(&Response::err(req.id, error))?)
                                .await;
                            continue;
                        }
                    }
                }
            }
            resp
        } else if req.op == relay_bus::ops::device::RunStop::NAME {
            let run_id = req.payload.get("run_id").and_then(|value| value.as_i64());
            let e = engine.clone();
            let resp = tokio::task::spawn_blocking(move || e.dispatch(req, Door::Socket)).await?;
            if resp.ok {
                if let Some(id) = run_id {
                    if let Some(task) = attached_runs.remove(&id) {
                        task.abort();
                    }
                }
            }
            resp
        } else if engine.answers_from_memory(&req) {
            // A keystroke is the one request whose latency a person can feel, and it is also the
            // one that never reaches the store. Answer it here rather than paying a trip to the
            // blocking pool and back for a map lookup and a write.
            engine.dispatch(req, Door::Socket)
        } else {
            let e = engine.clone();
            tokio::task::spawn_blocking(move || e.dispatch(req, Door::Socket)).await?
        };
        let _ = out_tx.send(serde_json::to_string(&resp)?).await;
    }
    if let Some(h) = forwarder.take() {
        h.abort();
    }
    for (_, h) in attached.drain() {
        h.abort();
    }
    for (_, h) in attached_runs.drain() {
        h.abort();
    }
    attached_mirrors.clear();
    drop(device_watch);
    drop(resource_watch);
    drop(out_tx);
    let _ = writer.await;
    Ok(())
}

/// A tiny client for tests and the CLI: one connection, request/response by line.
pub struct Client {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    pub async fn connect(path: &Path) -> Result<Client> {
        let stream = UnixStream::connect(path)
            .await
            .with_context(|| format!("connecting to {}", path.display()))?;
        let (r, w) = stream.into_split();
        Ok(Client {
            lines: BufReader::new(r).lines(),
            w,
        })
    }
    pub async fn send_raw(&mut self, line: &str) -> Result<()> {
        self.w.write_all(line.as_bytes()).await?;
        self.w.write_all(b"\n").await?;
        Ok(())
    }
    /// Next line, parsed as either a Response or an Event.
    pub async fn next(&mut self) -> Result<Option<Line>> {
        let Some(l) = self.lines.next_line().await? else {
            return Ok(None);
        };
        let v: serde_json::Value = serde_json::from_str(&l)?;
        if v.get("ev").is_some() {
            Ok(Some(Line::Event(serde_json::from_value(v)?)))
        } else if v.get("stream").is_some() {
            Ok(Some(Line::Frame(serde_json::from_value(v)?)))
        } else {
            Ok(Some(Line::Response(serde_json::from_value(v)?)))
        }
    }
    /// Send a request and wait for its response (events that arrive meanwhile are returned
    /// via `on_event`).
    pub async fn call(
        &mut self,
        req: &Request,
        mut on_event: impl FnMut(Event),
    ) -> Result<Response> {
        self.send_raw(&serde_json::to_string(req)?).await?;
        loop {
            match self.next().await? {
                None => anyhow::bail!("connection closed"),
                Some(Line::Event(e)) => on_event(e),
                Some(Line::Frame(_)) => {}
                Some(Line::Response(r)) => {
                    if r.id == Some(req.id) || r.id.is_none() {
                        return Ok(r);
                    }
                }
            }
        }
    }
}

pub enum Line {
    Response(Response),
    Event(Event),
    Frame(Frame),
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    use relay_bus::Actor;
    use serde_json::{json, Value};
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resource_watch_leases_stop_on_disconnect_and_preserve_other_clients() {
        async fn watch(client: &mut Client, on: Value) -> Response {
            client
                .call(
                    &Request::new(Actor::User, "app.resources.watch", json!({"on":on})),
                    |_| {},
                )
                .await
                .unwrap()
        }
        async fn count(engine: &Engine, expected: usize) {
            tokio::time::timeout(Duration::from_secs(2), async {
                while *engine.resource_watch_clients.lock().unwrap() != expected {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        let engine = Engine::new(crate::Instance::Test, crate::Store::open_memory().unwrap());
        let fixture = tempfile::tempdir().unwrap();
        let server = SocketServer::start_in(engine.clone(), fixture.path().join("socket"))
            .await
            .unwrap();
        let mut first = Client::connect(&server.path).await.unwrap();
        let mut second = Client::connect(&server.path).await.unwrap();
        assert!(watch(&mut first, json!(true)).await.ok);
        let epoch = engine
            .resource_watch_epoch
            .load(std::sync::atomic::Ordering::SeqCst);
        assert!(watch(&mut first, json!(true)).await.ok);
        assert!(!watch(&mut first, json!("yes")).await.ok);
        count(&engine, 1).await;
        assert!(watch(&mut second, json!(true)).await.ok);
        count(&engine, 2).await;
        drop(first);
        count(&engine, 1).await;
        assert!(engine
            .resource_watch
            .load(std::sync::atomic::Ordering::SeqCst));
        assert!(watch(&mut second, json!(false)).await.ok);
        assert!(watch(&mut second, json!(false)).await.ok);
        count(&engine, 0).await;
        assert!(!engine
            .resource_watch
            .load(std::sync::atomic::Ordering::SeqCst));
        assert!(watch(&mut second, json!(true)).await.ok);
        assert!(
            engine
                .resource_watch_epoch
                .load(std::sync::atomic::Ordering::SeqCst)
                > epoch
        );
        drop(second);
        count(&engine, 0).await;
        assert!(!engine
            .resource_watch
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_device_watch_leases_are_owned_idempotent_and_validated() {
        async fn request(client: &mut Client, actor: Actor, payload: Value) -> Response {
            client
                .call(&Request::new(actor, "device.watch", payload), |_| {})
                .await
                .unwrap()
        }
        async fn until_clients(engine: &Engine, count: usize) {
            tokio::time::timeout(Duration::from_secs(2), async {
                while engine.device_watch.lock().unwrap().clients != count {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        let e = Engine::new(crate::Instance::Test, crate::Store::open_memory().unwrap());
        let fixture = tempfile::tempdir().unwrap();
        let adb = fixture.path().join("adb");
        std::fs::write(
            &adb,
            "#!/bin/sh\nprintf 'fake-phone device\\n'\nexec sleep 20\n",
        )
        .unwrap();
        std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
        e.dispatch(
            Request::new(
                Actor::User,
                "settings.set",
                json!({"path":"device.adb_path","value":adb}),
            ),
            Door::InProcess,
        )
        .into_result()
        .unwrap();
        let server = SocketServer::start_in(e.clone(), fixture.path().join("socket"))
            .await
            .unwrap();
        let mut first = Client::connect(&server.path).await.unwrap();
        let mut second = Client::connect(&server.path).await.unwrap();
        assert!(
            request(&mut first, Actor::User, json!({"on":true}))
                .await
                .ok
        );
        let runtime = e.device_watch.lock().unwrap().runtime.clone().unwrap();
        assert!(
            request(&mut first, Actor::User, json!({"on":true}))
                .await
                .ok
        );
        assert_eq!(e.device_watch.lock().unwrap().clients, 1);
        assert!(
            request(&mut second, Actor::User, json!({"on":false}))
                .await
                .ok
        );
        assert_eq!(e.device_watch.lock().unwrap().clients, 1);
        assert!(!runtime.stopped());
        assert!(
            request(&mut second, Actor::User, json!({"on":true}))
                .await
                .ok
        );
        assert_eq!(e.device_watch.lock().unwrap().clients, 2);
        // Neither the duplicate path nor a lease release skips schema or actor validation.
        for on in [true, false] {
            let malformed = request(&mut first, Actor::User, json!({"on":on,"bogus":1})).await;
            assert_eq!(malformed.error.unwrap().code, "bus.schema");
            assert!(
                !request(&mut first, Actor::agent("unbound"), json!({"on":on}))
                    .await
                    .ok
            );
            assert_eq!(e.device_watch.lock().unwrap().clients, 2);
            assert!(!runtime.stopped());
        }
        assert!(
            !request(&mut first, Actor::User, json!({"on":"yes"}))
                .await
                .ok
        );
        assert!(
            request(&mut second, Actor::User, json!({"on":false}))
                .await
                .ok
        );
        assert!(
            request(&mut second, Actor::User, json!({"on":false}))
                .await
                .ok
        );
        drop(second);
        assert_eq!(e.device_watch.lock().unwrap().clients, 1);
        let mut third = Client::connect(&server.path).await.unwrap();
        assert!(
            request(&mut third, Actor::User, json!({"on":true}))
                .await
                .ok
        );
        drop(first);
        until_clients(&e, 1).await;
        assert!(!runtime.stopped(), "another window still owns the watcher");
        drop(third);
        until_clients(&e, 0).await;
        assert!(runtime.stopped());
        assert!(e.device_watch.lock().unwrap().runtime.is_none());
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use base64::Engine as _;

    /// The hand-written encoder must be byte-for-byte what `serde_json` would have produced
    /// for the same `Frame`, or a client that trusts the schema is reading a different protocol.
    #[test]
    fn hand_written_pty_lines_match_serde_exactly() {
        let bodies: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"hello".to_vec(),
            b"\x1b[32mgreen\x1b[0m \"quoted\" \\ backslash\n\t".to_vec(),
            (0u8..=255).collect(),
            vec![0xff; 64 * 1024],
        ];
        // Names that would break a formatter that forgot to escape.
        for session in ["brisk-otter", "quote\"name", "back\\slash", "new\nline", "unicode-é"] {
            let head = pty_frame_head(session);
            for (i, body) in bodies.iter().enumerate() {
                let epoch = i as u64 + 3;
                let seq = (i as u64 + 1) * 1_000_000;
                let expected = serde_json::to_string(&Frame {
                    v: ENVELOPE_V,
                    stream: "pty".into(),
                    session: Some(session.to_string()),
                    run_id: None,
                    mirror_id: None,
                    epoch: Some(epoch),
                    seq,
                    data: serde_json::Value::String(
                        base64::engine::general_purpose::STANDARD.encode(body),
                    ),
                })
                .unwrap();
                let actual = pty_frame_line(&head, epoch, seq, body);
                assert_eq!(actual, expected, "session {session:?}, body #{i}");
                // And it still parses back into the same frame.
                let parsed: Frame = serde_json::from_str(&actual).unwrap();
                assert_eq!(parsed.seq, seq);
                assert_eq!(parsed.epoch, Some(epoch));
                assert_eq!(parsed.session.as_deref(), Some(session));
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(parsed.data.as_str().unwrap())
                    .unwrap();
                assert_eq!(&decoded, body);
            }
        }
    }
}
