//! The WiFi door: a WebSocket listener on the LAN. A phone that scanned the pairing code
//! connects here first, because a link that never leaves the room is the private one.
//!
//! `GET /info` answers plain HTTP with the host's name so a phone can tell "wrong network"
//! from "wrong machine" before it starts a handshake.

use crate::bridge::{self, Ctx, OUTBOUND_QUEUE};
use crate::registry::Registry;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

pub struct DirectServer {
    pub local_addr: SocketAddr,
    accept: JoinHandle<()>,
}

impl Drop for DirectServer {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

impl DirectServer {
    pub async fn bind(ctx: Arc<Ctx>, addr: SocketAddr) -> Result<DirectServer> {
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("binding {addr}"))?;
        let local_addr = listener.local_addr()?;
        tracing::info!(addr = %local_addr, "remote door open (direct)");
        let accept = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        let ctx = ctx.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle(ctx, stream).await {
                                tracing::debug!(peer = %peer, error = %e, "direct connection ended");
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
        Ok(DirectServer { local_addr, accept })
    }
}

/// The body of `GET /info`.
pub fn info_json(ctx: &Ctx) -> String {
    let registry = Registry::load(&ctx.registry_path).unwrap_or_else(|_| Registry::fresh());
    serde_json::json!({
        "relay": "remote",
        "host": registry.host_name,
        "host_id": registry.host_id,
        "instance": ctx.instance.as_str(),
        "version": ctx.version,
        "pairing_open": !registry.pending.is_empty(),
    })
    .to_string()
}

async fn handle(ctx: Arc<Ctx>, mut stream: TcpStream) -> Result<()> {
    // A plain HTTP probe never upgrades; answer it by hand and get out of the way.
    let mut head = [0u8; 16];
    let n = stream.peek(&mut head).await?;
    if head[..n].starts_with(b"GET /info") {
        let body = info_json(&ctx);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await?;
        return Ok(());
    }

    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .context("websocket handshake")?;
    let (mut sink, mut source) = ws.split();
    let (in_tx, in_rx) = mpsc::channel::<String>(64);
    let (out_tx, mut out_rx) = mpsc::channel::<String>(OUTBOUND_QUEUE);

    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if sink.send(Message::text(line)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let reader = tokio::spawn(async move {
        while let Some(msg) = source.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if in_tx.send(text.to_string()).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Binary(bytes)) => {
                    if in_tx.send(String::from_utf8_lossy(&bytes).into_owned()).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });

    let result = bridge::run(ctx, in_rx, out_tx).await;
    reader.abort();
    // Let queued lines drain, then the writer closes the socket.
    let _ = writer.await;
    result.map_err(|e| anyhow::anyhow!(e))
}

/// A Tailscale address: tailnets hand out 100.64.0.0/10. A phone on the same tailnet reaches
/// the direct door there from anywhere, with no rendezvous and nothing listening publicly.
pub fn is_tailnet(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (b & 0xc0) == 64
}

/// Every address a phone on the same network could reach this machine at.
pub fn lan_addresses() -> Vec<std::net::Ipv4Addr> {
    let mut out: Vec<std::net::Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(v4) => Some(v4),
            _ => None,
        })
        .filter(|v4| !v4.is_link_local())
        .collect();
    out.sort_by_key(|a| (!a.is_private(), a.octets()));
    out.dedup();
    out
}

#[cfg(test)]
mod tailnet_tests {
    use super::is_tailnet;

    #[test]
    fn tailnet_is_the_cgnat_block_only() {
        assert!(is_tailnet("100.64.0.1".parse().unwrap()));
        assert!(is_tailnet("100.101.102.103".parse().unwrap()));
        assert!(is_tailnet("100.127.255.254".parse().unwrap()));
        assert!(!is_tailnet("100.63.0.1".parse().unwrap()));
        assert!(!is_tailnet("100.128.0.1".parse().unwrap()));
        assert!(!is_tailnet("192.168.1.20".parse().unwrap()));
    }
}
