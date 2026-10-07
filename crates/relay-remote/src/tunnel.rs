//! The PC's end of the rendezvous: dial the server, hold the socket open, and run one bridge
//! per lane the server opens. This is the Claude-remote-control shape — the machine at home
//! reaches out, nothing on it listens to the internet — with a server the person hosts.

use crate::bridge::{self, AbortOnDrop, Ctx};
use crate::registry::Rendezvous;
use crate::rendezvous::{Lane, MAX_LANE_LINE};
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
    ensure_crypto_provider();
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        loop {
            // Each attempt is its own task, so a panic inside a dial is a logged, retried failure
            // rather than the silent end of the tunnel; the guard takes it down with this one.
            let attempt = {
                let (ctx, rendezvous) = (ctx.clone(), rendezvous.clone());
                tokio::spawn(async move { connect_once(ctx, &rendezvous).await })
            };
            let _guard = AbortOnDrop(attempt.abort_handle());
            match attempt.await {
                Ok(Ok(())) => {
                    tracing::info!(url = %rendezvous.url, "rendezvous closed; reconnecting");
                    backoff = Duration::from_secs(1);
                }
                Ok(Err(e)) => {
                    tracing::warn!(url = %rendezvous.url, error = %e, retry_s = backoff.as_secs(), "rendezvous unreachable");
                }
                Err(e) => {
                    tracing::error!(url = %rendezvous.url, error = %e, retry_s = backoff.as_secs(), "rendezvous attempt panicked");
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    })
}

/// Make rustls's process-wide crypto provider explicit before the first `wss://` dial.
/// tokio-tungstenite builds its client config with `ClientConfig::builder()`, which panics when
/// it cannot pick a provider; naming one here keeps that true even if a second provider feature
/// is ever unified into the build. Losing the race to another installer is fine: one is set.
pub fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// How often the host pings the server, and how long silence may last before the socket is
/// declared dead. A NAT or a mobile hop that drops the connection without a FIN would
/// otherwise leave the host believing it is reachable while every phone is told "offline".
const PING_EVERY: Duration = Duration::from_secs(30);
const SILENCE_LIMIT: Duration = Duration::from_secs(90);

/// Requests from one phone waiting for its bridge. The shared loop never waits on a lane: a lane
/// that lets this fill is closed, and the phone reconnects.
const LANE_INBOUND: usize = 256;

/// One connection's lifetime. `Ok` means the server closed cleanly.
pub async fn connect_once(ctx: Arc<Ctx>, rendezvous: &Rendezvous) -> Result<()> {
    let url = host_url(rendezvous);
    let (ws, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .with_context(|| format!("dialing {}", rendezvous.url))?;
    tracing::info!(url = %rendezvous.url, room = %rendezvous.room, "rendezvous connected");
    let (mut sink, mut source) = ws.split();

    // Every lane's outbound lines funnel through one sender to the socket, pings included.
    let (to_server, mut from_lanes) = mpsc::channel::<Message>(1024);
    let writer = tokio::spawn(async move {
        while let Some(msg) = from_lanes.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let mut lanes: HashMap<String, (mpsc::Sender<String>, JoinHandle<()>)> = HashMap::new();
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_heard = tokio::time::Instant::now();
    loop {
        let msg = tokio::select! {
            next = source.next() => next,
            _ = ping.tick() => {
                if last_heard.elapsed() > SILENCE_LIMIT {
                    for (_, (_, task)) in lanes.drain() {
                        task.abort();
                    }
                    writer.abort();
                    anyhow::bail!("no traffic from the rendezvous for {}s", SILENCE_LIMIT.as_secs());
                }
                // Never wait here: with the writer backed up, the silence check above is the
                // only thing that notices a dead socket.
                if let Err(mpsc::error::TrySendError::Closed(_)) = to_server.try_send(Message::Ping(Vec::new().into())) {
                    break;
                }
                continue;
            }
        };
        let Some(msg) = msg else { break };
        last_heard = tokio::time::Instant::now();
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
                let (in_tx, in_rx) = mpsc::channel::<String>(LANE_INBOUND);
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
                                let line = Lane::Data { c: id.clone(), l }.to_line();
                                if line.len() > MAX_LANE_LINE {
                                    // Sent, it would break the socket every lane shares; closing
                                    // this lane costs one phone a reconnect instead.
                                    tracing::warn!(lane = %id, bytes = line.len(), "engine line too large for the rendezvous; closing the lane");
                                    let _ = to_server.send(Message::text(Lane::Close { c: id.clone() }.to_line())).await;
                                    break;
                                }
                                if to_server.send(Message::text(line)).await.is_err() {
                                    break;
                                }
                            }
                        })
                    };
                    // Aborting this lane must not leave the forwarder behind.
                    let _forward_guard = AbortOnDrop(forward.abort_handle());
                    if let Err(e) = bridge::run(ctx, in_rx, out_tx, (), "rendezvous".to_string()).await {
                        tracing::debug!(lane = %id, error = %e, "lane ended");
                    }
                    let _ = forward.await;
                    let _ = to_server.send(Message::text(Lane::Close { c: id }.to_line())).await;
                });
                lanes.insert(c, (in_tx, task));
            }
            Lane::Data { c, l } => {
                let Some((tx, _)) = lanes.get(&c) else { continue };
                match tx.try_send(l) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        // A lane whose bridge cannot keep up is closed rather than waited on:
                        // every other phone's lines go through this loop.
                        tracing::warn!(lane = %c, "lane fell behind; closing it");
                        if let Some((_, task)) = lanes.remove(&c) {
                            task.abort();
                        }
                        let _ = to_server.try_send(Message::text(Lane::Close { c }.to_line()));
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        lanes.remove(&c);
                    }
                }
            }
            Lane::Close { c } => {
                // The phone is gone; nothing it sent is still worth delivering.
                if let Some((_, task)) = lanes.remove(&c) {
                    task.abort();
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
