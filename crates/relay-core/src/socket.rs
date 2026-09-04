//! The Unix-socket door (BUS.md §6.2): newline-delimited JSON, one `Request` per line,
//! responses correlated by id, `bus.subscribe` turns the connection into a subscriber.
//! One engine per instance, enforced with a lock file + probe.

use crate::engine::{Door, Engine};
use crate::paths::Instance;
use anyhow::{Context, Result};
use relay_bus::envelope::{Event, Frame, Response};
use relay_bus::error::BusError;
use relay_bus::ops::bus::{SubscribeIn, SubscribeOut, WaitIn, WaitOut};
use std::time::Duration;
use relay_bus::registry::Op;
use relay_bus::{Empty, Request};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

async fn handle_conn(engine: Arc<Engine>, stream: UnixStream) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(1024);
    let writer = tokio::spawn(async move {
        while let Some(mut line) = out_rx.recv().await {
            line.push('\n');
            if w.write_all(line.as_bytes()).await.is_err() {
                break;
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
        let resp = if req.op == relay_bus::ops::bus::Subscribe::NAME {
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
                    let waited = tokio::time::timeout(timeout, async {
                        loop {
                            match rx.recv().await {
                                Ok(ev) if filter.matches(&ev.ev) => return Some(ev),
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
                            use base64::Engine as _;
                            let b64 = base64::engine::general_purpose::STANDARD;
                            let mut rx = att.rx;
                            if !att.catch_up.is_empty() {
                                let f = Frame { v: 1, stream: "pty".into(), session: Some(name.clone()), run_id: None, mirror_id: None, epoch: Some(att.epoch), seq: att.seq, data: serde_json::Value::String(b64.encode(&att.catch_up)) };
                                if let Ok(s) = serde_json::to_string(&f) { if tx.send(s).await.is_err() { return; } }
                            }
                            loop {
                                match rx.recv().await {
                                    Ok(fr) => {
                                        if fr.seq <= att.seq && fr.epoch == att.epoch { continue; } // already in catch-up
                                        let f = Frame { v: 1, stream: "pty".into(), session: Some(name.clone()), run_id: None, mirror_id: None, epoch: Some(fr.epoch), seq: fr.seq, data: serde_json::Value::String(b64.encode(&fr.data)) };
                                        if let Ok(s) = serde_json::to_string(&f) { if tx.send(s).await.is_err() { break; } }
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
                }
            }
            resp
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
