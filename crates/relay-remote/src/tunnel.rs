//! The PC's end of the rendezvous: dial the server, hold the socket open, and run one bridge
//! per lane the server opens. This is the Claude-remote-control shape — the machine at home
//! reaches out, nothing on it listens to the internet — with a server the person hosts.

use crate::bridge::{self, Ctx};
use crate::registry::Rendezvous;
use crate::rendezvous::Lane;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

/// The host URL for a rendezvous config.
pub fn host_url(r: &Rendezvous) -> String {
    format!("{}/host/{}?secret={}", r.url, r.room, r.secret)
}

/// The URL a phone uses to reach this room.
pub fn join_url(r: &Rendezvous) -> String {
    format!("{}/join/{}", r.url, r.room)
}

/// Keep the tunnel up until the task is dropped. Reconnects with backoff; every attempt and
/// every drop is logged so a person can see why a phone was told "host offline".
pub fn spawn(ctx: Arc<Ctx>, rendezvous: Rendezvous) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            match connect_once(ctx.clone(), &rendezvous).await {
                Ok(()) => {
                    tracing::info!(url = %rendezvous.url, "rendezvous closed; reconnecting");
                    backoff = Duration::from_secs(1);
                }
                Err(e) => {
                    tracing::warn!(url = %rendezvous.url, error = %e, retry_s = backoff.as_secs(), "rendezvous unreachable");
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    })
}

/// One connection's lifetime. `Ok` means the server closed cleanly.
pub async fn connect_once(ctx: Arc<Ctx>, rendezvous: &Rendezvous) -> Result<()> {
    let url = host_url(rendezvous);
    let (ws, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .with_context(|| format!("dialing {}", rendezvous.url))?;
    tracing::info!(url = %rendezvous.url, room = %rendezvous.room, "rendezvous connected");
    let (mut sink, mut source) = ws.split();

    // Every lane's outbound lines funnel through one sender to the socket.
    let (to_server, mut from_lanes) = mpsc::channel::<String>(1024);
    let writer = tokio::spawn(async move {
        while let Some(line) = from_lanes.recv().await {
            if sink.send(Message::text(line)).await.is_err() {
                break;
            }
        }
    });

    let mut lanes: HashMap<String, (mpsc::Sender<String>, JoinHandle<()>)> = HashMap::new();
    while let Some(msg) = source.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => break,
            Err(e) => {
                for (_, (_, task)) in lanes.drain() {
                    task.abort();
                }
                writer.abort();
                return Err(e).context("rendezvous socket");
            }
            Ok(_) => continue,
        };
        let Ok(lane) = serde_json::from_str::<Lane>(&text) else {
            continue;
        };
        match lane {
            Lane::Open { c } => {
                let (in_tx, in_rx) = mpsc::channel::<String>(64);
                let (out_tx, mut out_rx) = mpsc::channel::<String>(bridge::OUTBOUND_QUEUE);
                let ctx = ctx.clone();
                let to_server = to_server.clone();
                let id = c.clone();
                let task = tokio::spawn(async move {
                    let forward = {
                        let to_server = to_server.clone();
                        let id = id.clone();
                        tokio::spawn(async move {
                            while let Some(l) = out_rx.recv().await {
                                if to_server.send(Lane::Data { c: id.clone(), l }.to_line()).await.is_err() {
                                    break;
                                }
                            }
                        })
                    };
                    if let Err(e) = bridge::run(ctx, in_rx, out_tx).await {
                        tracing::debug!(lane = %id, error = %e, "lane ended");
                    }
                    let _ = forward.await;
                    let _ = to_server.send(Lane::Close { c: id }.to_line()).await;
                });
                lanes.insert(c, (in_tx, task));
            }
            Lane::Data { c, l } => {
                if let Some((tx, _)) = lanes.get(&c) {
                    if tx.send(l).await.is_err() {
                        lanes.remove(&c);
                    }
                }
            }
            Lane::Close { c } => {
                if let Some((tx, task)) = lanes.remove(&c) {
                    drop(tx);
                    // Give the bridge a moment to notice the closed inbound and clean up.
                    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
                }
            }
        }
    }
    for (_, (_, task)) in lanes.drain() {
        task.abort();
    }
    writer.abort();
    Ok(())
}
