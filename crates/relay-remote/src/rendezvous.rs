//! The server a person hosts themselves so a phone away from home can still reach the engine:
//! the PC dials out and keeps one WebSocket open; each phone that joins the PC's room gets a
//! lane on it. The server copies lines between the two and interprets none of them — every
//! credential check still happens on the PC, in `bridge.rs`. It can read them, though, and
//! could write into a lane after the check: whoever runs it is trusted (docs/MOBILE.md §6).
//!
//! The server forwards between many sockets in one loop per host, so it never waits on any
//! one phone: each lane has a byte budget, and a phone that lets it fill is cut off and told to
//! reconnect. Every connection also gets a deadline to finish its upgrade, and the number of
//! connections and lanes per room is capped.
//!
//! Rooms are not configured: a host proves a room by presenting the secret whose digest names
//! it (`registry::room_for`). The server keeps nothing on disk and forgets everything on exit.
//!
//! Paths: `GET /host/<room>?secret=<secret>` for the PC, `GET /join/<room>` for a phone,
//! `GET /health` for whoever runs it.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::Message;

/// One multiplexed message on the host's socket. Lanes are opened by the server when a phone
/// joins and closed by whichever side goes first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Lane {
    Open { c: String },
    Data { c: String, l: String },
    Close { c: String },
}

impl Lane {
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// How long a connection has to finish its WebSocket upgrade.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Connections the server holds at once, hosts and phones together.
const MAX_CONNECTIONS: usize = 1024;

/// Phones one room carries at once. Joining needs no credential, so this bounds what a stranger
/// who knows a room id can make the PC do.
const MAX_LANES_PER_ROOM: usize = 32;

/// Bytes queued toward one phone before it is cut off. Large enough for a burst of terminal
/// output or a re-attach catch-up on a slow link; a phone that drops off without a FIN fills it
/// and is closed instead of stalling every other lane in the room.
const LANE_BUDGET: usize = 8 << 20;

/// A live phone sends `bus.ping` every 25 s; a phone silent this long has gone.
const PHONE_SILENCE: Duration = Duration::from_secs(90);

/// The server's side of one joined phone.
struct LaneTx {
    lines: mpsc::UnboundedSender<String>,
    /// Bytes in `lines` not yet written to the phone's socket.
    queued: Arc<AtomicUsize>,
    /// The task writing to the phone. Aborting it drops the socket even while a write is stuck.
    writer: AbortHandle,
}

impl LaneTx {
    /// Queue a line without waiting. `false` means the phone is over budget or gone; the caller
    /// drops the lane.
    fn offer(&self, line: String) -> bool {
        let n = line.len();
        if self.queued.fetch_add(n, Ordering::AcqRel) + n > LANE_BUDGET {
            return false;
        }
        self.lines.send(line).is_ok()
    }

    fn cut(self) {
        self.writer.abort();
    }
}

#[derive(Default)]
struct Room {
    /// Lines for the host's socket.
    host: Option<mpsc::Sender<String>>,
    /// Lines for each joined phone, by lane id.
    lanes: HashMap<String, LaneTx>,
}

type Rooms = Arc<Mutex<HashMap<String, Room>>>;

pub struct RendezvousServer {
    pub local_addr: SocketAddr,
    accept: JoinHandle<()>,
}

impl Drop for RendezvousServer {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

impl RendezvousServer {
    pub async fn bind(addr: SocketAddr) -> Result<RendezvousServer> {
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("binding {addr}"))?;
        let local_addr = listener.local_addr()?;
        tracing::info!(addr = %local_addr, "rendezvous open");
        let rooms: Rooms = Arc::default();
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let accept = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        let Ok(slot) = slots.clone().try_acquire_owned() else {
                            tracing::debug!(peer = %peer, "rendezvous full; connection refused");
                            continue;
                        };
                        let rooms = rooms.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle(rooms, stream).await {
                                tracing::debug!(peer = %peer, error = %e, "rendezvous connection ended");
                            }
                            drop(slot);
                        });
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed");
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        });
        Ok(RendezvousServer { local_addr, accept })
    }
}

enum Role {
    Host { room: String },
    Join { room: String },
}

/// Parse the request path into a role, refusing what is not for us before the upgrade.
fn classify(path: &str) -> std::result::Result<Role, (u16, &'static str)> {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let secret = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("secret="))
        .unwrap_or("");
    if let Some(room) = path.strip_prefix("/host/") {
        if room.is_empty() || crate::registry::room_for(secret) != room {
            return Err((403, "room secret does not open this room"));
        }
        return Ok(Role::Host { room: room.to_string() });
    }
    if let Some(room) = path.strip_prefix("/join/") {
        if room.is_empty() {
            return Err((404, "no room"));
        }
        return Ok(Role::Join { room: room.to_string() });
    }
    Err((404, "relay rendezvous: /host/<room>?secret=… or /join/<room>"))
}

fn refuse((status, body): (u16, &str)) -> ErrorResponse {
    let mut r = ErrorResponse::new(Some(body.to_string()));
    *r.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::from_u16(status)
        .unwrap_or(tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND);
    r
}

async fn handle(rooms: Rooms, stream: TcpStream) -> Result<()> {
    let (role, ws) = match tokio::time::timeout(HANDSHAKE_TIMEOUT, upgrade(&rooms, stream)).await {
        Err(_) => anyhow::bail!("no handshake within {}s", HANDSHAKE_TIMEOUT.as_secs()),
        Ok(Err(e)) => return Err(e),
        Ok(Ok(None)) => return Ok(()),
        Ok(Ok(Some(upgraded))) => upgraded,
    };
    match role {
        Role::Host { room } => host(rooms, room, ws).await,
        Role::Join { room } => join(rooms, room, ws).await,
    }
}

/// Answer `/health` (`None`), or finish the WebSocket upgrade and say which side this is.
async fn upgrade(rooms: &Rooms, mut stream: TcpStream) -> Result<Option<(Role, Ws)>> {
    let mut head = [0u8; 16];
    let n = stream.peek(&mut head).await?;
    if head[..n].starts_with(b"GET /health") {
        use tokio::io::AsyncWriteExt as _;
        let body = {
            let rooms = rooms.lock().unwrap();
            let hosts = rooms.values().filter(|r| r.host.is_some()).count();
            let lanes: usize = rooms.values().map(|r| r.lanes.len()).sum();
            serde_json::json!({"relay":"rendezvous","hosts":hosts,"lanes":lanes}).to_string()
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await?;
        return Ok(None);
    }

    let mut role: Option<Role> = None;
    // The callback's error type is tungstenite's, an `http::Response` it will write verbatim.
    #[allow(clippy::result_large_err)]
    let on_request = |req: &Request, resp: Response| {
        let path = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
        match classify(path) {
            Ok(r) => {
                role = Some(r);
                Ok(resp)
            }
            Err(e) => Err(refuse(e)),
        }
    };
    let ws = tokio_tungstenite::accept_hdr_async(stream, on_request)
        .await
        .context("websocket handshake")?;
    let role = role.expect("callback ran on success");
    Ok(Some((role, ws)))
}

type Ws = tokio_tungstenite::WebSocketStream<TcpStream>;

async fn host(rooms: Rooms, room: String, ws: Ws) -> Result<()> {
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<String>(1024);
    {
        let mut rooms = rooms.lock().unwrap();
        let entry = rooms.entry(room.clone()).or_default();
        // A second host for the same room replaces the first: a PC that restarted is the
        // common case, and its old socket is about to time out anyway.
        entry.host = Some(tx.clone());
        // Close the old host's phones so they reconnect against this one.
        for (_, lane) in entry.lanes.drain() {
            lane.cut();
        }
    }
    tracing::info!(room = %room, "host joined");
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if sink.send(Message::text(line)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    while let Some(msg) = source.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        let Ok(lane) = serde_json::from_str::<Lane>(&text) else {
            continue;
        };
        match lane {
            Lane::Data { c, l } => {
                // Never wait on one phone here: this loop carries every lane in the room.
                let mut rooms = rooms.lock().unwrap();
                let Some(r) = rooms.get_mut(&room) else { continue };
                let delivered = r.lanes.get(&c).map(|lane| lane.offer(l));
                if delivered == Some(false) {
                    tracing::info!(room = %room, lane = %c, "phone fell behind; cutting its lane");
                    if let Some(lane) = r.lanes.remove(&c) {
                        lane.cut();
                    }
                    let _ = tx.try_send(Lane::Close { c }.to_line());
                }
            }
            Lane::Close { c } => {
                let target = rooms.lock().unwrap().get_mut(&room).and_then(|r| r.lanes.remove(&c));
                if let Some(lane) = target {
                    lane.cut();
                }
            }
            Lane::Open { .. } => {}
        }
    }
    {
        let mut rooms = rooms.lock().unwrap();
        // Only if this host is still the room's: a replacement already took the room over.
        if let Some(r) = rooms.get_mut(&room).filter(|r| r.host.as_ref().is_some_and(|h| h.same_channel(&tx))) {
            r.host = None;
            for (_, lane) in r.lanes.drain() {
                lane.cut();
            }
        }
        rooms.retain(|_, r| r.host.is_some() || !r.lanes.is_empty());
    }
    writer.abort();
    tracing::info!(room = %room, "host left");
    Ok(())
}

async fn join(rooms: Rooms, room: String, ws: Ws) -> Result<()> {
    let lane_id = crate::registry::random_hex(8);
    let (mut sink, mut source) = ws.split();
    let refuse = |error: &str| Message::text(serde_json::json!({"v":1,"ok":false,"error":error}).to_string());
    let host = {
        let rooms = rooms.lock().unwrap();
        match rooms.get(&room) {
            Some(r) if r.lanes.len() >= MAX_LANES_PER_ROOM => Err("room.full"),
            Some(r) => r.host.clone().ok_or("host.offline"),
            None => Err("host.offline"),
        }
    };
    let host = match host {
        Ok(host) => host,
        Err(error) => {
            let _ = sink.send(refuse(error)).await;
            let _ = sink.close().await;
            return Ok(());
        }
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let queued = Arc::new(AtomicUsize::new(0));
    let writer = {
        let queued = queued.clone();
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                let n = line.len();
                let sent = sink.send(Message::text(line)).await;
                queued.fetch_sub(n, Ordering::AcqRel);
                if sent.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        })
    };
    // The writer ending, for any reason, ends this lane.
    let gone = tx.clone();
    {
        let mut rooms = rooms.lock().unwrap();
        // The host may have left or been replaced since it was looked up; the phone reconnects.
        match rooms.get_mut(&room) {
            Some(r) if r.host.as_ref().is_some_and(|h| h.same_channel(&host)) => {
                r.lanes.insert(lane_id.clone(), LaneTx { lines: tx, queued, writer: writer.abort_handle() });
            }
            _ => {
                writer.abort();
                return Ok(());
            }
        }
    }
    if host.send(Lane::Open { c: lane_id.clone() }.to_line()).await.is_ok() {
        loop {
            let msg = tokio::select! {
                _ = gone.closed() => break,
                next = tokio::time::timeout(PHONE_SILENCE, source.next()) => match next {
                    Ok(Some(msg)) => msg,
                    Ok(None) | Err(_) => break,
                },
            };
            let text = match msg {
                Ok(Message::Text(t)) => t.to_string(),
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => continue,
            };
            if host.send(Lane::Data { c: lane_id.clone(), l: text }.to_line()).await.is_err() {
                break;
            }
        }
    }
    let _ = host.send(Lane::Close { c: lane_id.clone() }.to_line()).await;
    if let Some(r) = rooms.lock().unwrap().get_mut(&room) {
        r.lanes.remove(&lane_id);
    }
    writer.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_open_the_right_role_and_nothing_else() {
        let secret = "s3cret";
        let room = crate::registry::room_for(secret);
        assert!(matches!(classify(&format!("/host/{room}?secret={secret}")), Ok(Role::Host { .. })));
        assert!(classify(&format!("/host/{room}?secret=wrong")).is_err());
        assert!(classify(&format!("/host/{room}")).is_err());
        assert!(matches!(classify(&format!("/join/{room}")), Ok(Role::Join { .. })));
        assert!(classify("/join/").is_err());
        assert!(classify("/").is_err());
    }

    #[tokio::test]
    async fn a_lane_over_its_budget_is_refused_not_waited_on() {
        let (lines, mut rx) = mpsc::unbounded_channel();
        let writer = tokio::spawn(std::future::pending::<()>());
        let lane = LaneTx { lines, queued: Arc::default(), writer: writer.abort_handle() };
        let chunk = "x".repeat(LANE_BUDGET / 4);
        for _ in 0..4 {
            assert!(lane.offer(chunk.clone()));
        }
        assert!(!lane.offer("y".into()), "the budget is a ceiling");
        // What the writer sends frees budget again.
        rx.recv().await.unwrap();
        lane.queued.fetch_sub(chunk.len(), Ordering::AcqRel);
        assert!(lane.offer("y".into()));
        lane.cut();
        assert!(writer.await.unwrap_err().is_cancelled());
    }

    #[test]
    fn lanes_serialize_compactly() {
        assert_eq!(Lane::Open { c: "a".into() }.to_line(), r#"{"t":"open","c":"a"}"#);
        let back: Lane = serde_json::from_str(r#"{"t":"data","c":"a","l":"{}"}"#).unwrap();
        assert_eq!(back, Lane::Data { c: "a".into(), l: "{}".into() });
    }
}
