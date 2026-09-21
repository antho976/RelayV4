//! The server a person hosts themselves so a phone away from home can still reach the engine:
//! the PC dials out and keeps one WebSocket open; each phone that joins the PC's room gets a
//! lane on it. The server copies lines between the two and understands none of them — every
//! credential check still happens on the PC, in `bridge.rs`.
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
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
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

#[derive(Default)]
struct Room {
    /// Lines for the host's socket.
    host: Option<mpsc::Sender<String>>,
    /// Lines for each joined phone, by lane id.
    lanes: HashMap<String, mpsc::Sender<String>>,
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
        let accept = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        let rooms = rooms.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle(rooms, stream).await {
                                tracing::debug!(peer = %peer, error = %e, "rendezvous connection ended");
                            }
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

async fn handle(rooms: Rooms, mut stream: TcpStream) -> Result<()> {
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
        return Ok(());
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
    match role {
        Role::Host { room } => host(rooms, room, ws).await,
        Role::Join { room } => join(rooms, room, ws).await,
    }
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
        entry.host = Some(tx);
        for lane in entry.lanes.values() {
            let _ = lane.try_send(String::new());
        }
        entry.lanes.clear();
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
                let target = rooms.lock().unwrap().get(&room).and_then(|r| r.lanes.get(&c).cloned());
                if let Some(target) = target {
                    let _ = target.send(l).await;
                }
            }
            Lane::Close { c } => {
                let target = rooms.lock().unwrap().get_mut(&room).and_then(|r| r.lanes.remove(&c));
                drop(target);
            }
            Lane::Open { .. } => {}
        }
    }
    {
        let mut rooms = rooms.lock().unwrap();
        if let Some(r) = rooms.get_mut(&room) {
            r.host = None;
            r.lanes.clear();
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
    let (tx, mut rx) = mpsc::channel::<String>(crate::bridge::OUTBOUND_QUEUE);
    let host = {
        let mut rooms = rooms.lock().unwrap();
        match rooms.get_mut(&room) {
            Some(r) if r.host.is_some() => {
                r.lanes.insert(lane_id.clone(), tx);
                r.host.clone()
            }
            _ => None,
        }
    };
    let Some(host) = host else {
        let _ = sink
            .send(Message::text(
                serde_json::json!({"v":1,"ok":false,"error":"host.offline"}).to_string(),
            ))
            .await;
        let _ = sink.close().await;
        return Ok(());
    };
    if host.send(Lane::Open { c: lane_id.clone() }.to_line()).await.is_err() {
        return Ok(());
    }
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            // An empty line is the host-replaced signal: close the phone's socket so it
            // reconnects against the new host.
            if line.is_empty() {
                break;
            }
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
        if host.send(Lane::Data { c: lane_id.clone(), l: text }.to_line()).await.is_err() {
            break;
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

    #[test]
    fn lanes_serialize_compactly() {
        assert_eq!(Lane::Open { c: "a".into() }.to_line(), r#"{"t":"open","c":"a"}"#);
        let back: Lane = serde_json::from_str(r#"{"t":"data","c":"a","l":"{}"}"#).unwrap();
        assert_eq!(back, Lane::Data { c: "a".into(), l: "{}".into() });
    }
}
