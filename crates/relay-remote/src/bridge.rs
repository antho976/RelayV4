//! One paired conversation: greet, verify, then carry bus lines between a phone and the
//! engine's socket door. The transport underneath is either a WebSocket the phone opened
//! directly (`direct.rs`) or one lane of the rendezvous tunnel (`tunnel.rs`); both hand this
//! module a pair of channels and nothing else.

use crate::registry::Registry;
use crate::wire::{self, Gate, Greeting, Hello, Welcome, WIRE_V};
use anyhow::{Context, Result};
use relay_core::Instance;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

/// What every bridge needs to know about the engine it fronts.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub instance: Instance,
    pub registry_path: PathBuf,
    pub socket_path: PathBuf,
    pub version: String,
}

impl Ctx {
    pub fn for_instance(instance: Instance) -> Ctx {
        Ctx {
            instance,
            registry_path: Registry::path_for(instance),
            socket_path: instance.socket_path(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// How long a phone has to answer the greeting. A TCP connection that never speaks holds
/// nothing but a task.
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);

/// Lines queued toward the phone before the transport applies backpressure. Terminal output
/// arrives in bursts; a phone on a poor link must not park unbounded memory here.
pub const OUTBOUND_QUEUE: usize = 256;

/// Why a connection ended before or during the bus phase. Logged, never fatal.
#[derive(Debug, thiserror::Error)]
pub enum BridgeEnd {
    #[error("the phone did not answer the greeting in time")]
    HelloTimeout,
    #[error("the phone closed before saying hello")]
    ClosedEarly,
    #[error("hello was not understood: {0}")]
    BadHello(String),
    #[error("denied: {0}")]
    Denied(&'static str),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The outcome of a handshake, with the registry already written back.
pub struct Admitted {
    pub device_id: String,
    pub welcome: Welcome,
}

/// Verify a `Hello` against the registry on disk and record the result. Pure enough to test
/// without a transport: the transport-side `run` calls exactly this.
pub fn admit(ctx: &Ctx, challenge: &str, hello: &Hello) -> std::result::Result<Admitted, BridgeEnd> {
    if hello.v != WIRE_V {
        return Err(BridgeEnd::BadHello(format!("wire v{} is not v{WIRE_V}", hello.v)));
    }
    let mut registry = Registry::load(&ctx.registry_path).map_err(BridgeEnd::Other)?;
    let admitted = if let Some(code) = hello.pair.as_deref() {
        let device = registry
            .redeem(code, hello.device_name.as_deref().unwrap_or(""))
            .ok_or(BridgeEnd::Denied("pair.invalid"))?;
        Admitted {
            device_id: device.id.clone(),
            welcome: Welcome::paired(&device.id, &device.token),
        }
    } else if let (Some(id), Some(proof)) = (hello.device.as_deref(), hello.proof.as_deref()) {
        let device = registry.device(id).ok_or(BridgeEnd::Denied("auth.unknown_device"))?;
        if !wire::digest_eq(proof, &wire::proof(challenge, &device.token)) {
            return Err(BridgeEnd::Denied("auth.bad_proof"));
        }
        registry.touch(id);
        Admitted {
            device_id: id.to_string(),
            welcome: Welcome::admitted(id),
        }
    } else {
        return Err(BridgeEnd::BadHello("expected `pair` or `device`+`proof`".into()));
    };
    registry.save(&ctx.registry_path).map_err(BridgeEnd::Other)?;
    Ok(admitted)
}

pub fn greeting(ctx: &Ctx, registry: &Registry, challenge: &str) -> Greeting {
    Greeting {
        v: WIRE_V,
        relay: "remote".into(),
        host: registry.host_name.clone(),
        host_id: registry.host_id.clone(),
        instance: ctx.instance.as_str().into(),
        version: ctx.version.clone(),
        challenge: challenge.into(),
    }
}

/// Run one conversation to its end. `inbound` carries text frames from the phone; `outbound`
/// carries lines to it. Returns when either side goes away.
pub async fn run(
    ctx: Arc<Ctx>,
    mut inbound: mpsc::Receiver<String>,
    outbound: mpsc::Sender<String>,
) -> std::result::Result<(), BridgeEnd> {
    let registry = Registry::load(&ctx.registry_path).map_err(BridgeEnd::Other)?;
    let challenge = crate::registry::random_hex(16);
    let greet = greeting(&ctx, &registry, &challenge);
    outbound
        .send(serde_json::to_string(&greet).context("greeting")?)
        .await
        .map_err(|_| BridgeEnd::ClosedEarly)?;

    let first = match tokio::time::timeout(HELLO_TIMEOUT, inbound.recv()).await {
        Err(_) => return Err(BridgeEnd::HelloTimeout),
        Ok(None) => return Err(BridgeEnd::ClosedEarly),
        Ok(Some(line)) => line,
    };
    let hello: Hello = match serde_json::from_str(&first) {
        Ok(h) => h,
        Err(e) => {
            let _ = outbound.send(serde_json::to_string(&Welcome::denied("hello.parse")).unwrap_or_default()).await;
            return Err(BridgeEnd::BadHello(e.to_string()));
        }
    };
    let admitted = match admit(&ctx, &challenge, &hello) {
        Ok(a) => a,
        Err(end) => {
            let code = match &end {
                BridgeEnd::Denied(code) => code,
                BridgeEnd::BadHello(_) => "hello.shape",
                _ => "internal",
            };
            let _ = outbound.send(serde_json::to_string(&Welcome::denied(code)).unwrap_or_default()).await;
            return Err(end);
        }
    };
    outbound
        .send(serde_json::to_string(&admitted.welcome).context("welcome")?)
        .await
        .map_err(|_| BridgeEnd::ClosedEarly)?;
    tracing::info!(device = %admitted.device_id, instance = %ctx.instance, "remote device admitted");

    // Only now does the engine hear about this connection.
    let stream = UnixStream::connect(&ctx.socket_path)
        .await
        .with_context(|| format!("no engine at {}", ctx.socket_path.display()))?;
    let (reader, mut writer) = stream.into_split();

    let to_phone = outbound.clone();
    let pump = tokio::spawn(async move {
        let mut lines = BufReader::with_capacity(256 * 1024, reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if to_phone.send(line).await.is_err() {
                break;
            }
        }
    });

    let instance = ctx.instance;
    let result: Result<()> = async {
        while let Some(line) = inbound.recv().await {
            if line.trim().is_empty() {
                continue;
            }
            match wire::gate(&line, instance) {
                Gate::Forward => {
                    writer.write_all(line.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                }
                Gate::Reject(resp) => {
                    if outbound.send(serde_json::to_string(&resp)?).await.is_err() {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
    .await;
    pump.abort();
    tracing::info!(device = %admitted.device_id, "remote device left");
    result.map_err(BridgeEnd::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(dir: &tempfile::TempDir) -> Ctx {
        Ctx {
            instance: Instance::Test,
            registry_path: dir.path().join("remote.json"),
            socket_path: dir.path().join("none.sock"),
            version: "test".into(),
        }
    }

    #[test]
    fn pairing_mints_a_token_that_then_proves_itself() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        let mut reg = Registry::fresh();
        let code = reg.begin_pair().code;
        reg.save(&ctx.registry_path).unwrap();

        let hello = Hello { v: 1, pair: Some(code.clone()), device_name: Some("Pixel".into()), ..Default::default() };
        let paired = admit(&ctx, "c0", &hello).unwrap();
        let token = paired.welcome.token.clone().unwrap();

        // The code is spent.
        assert!(matches!(admit(&ctx, "c0", &hello), Err(BridgeEnd::Denied("pair.invalid"))));

        // The token proves itself against a fresh challenge, and only that challenge.
        let ok = Hello { v: 1, device: Some(paired.device_id.clone()), proof: Some(wire::proof("c1", &token)), ..Default::default() };
        let admitted = admit(&ctx, "c1", &ok).unwrap();
        assert!(admitted.welcome.token.is_none(), "the token is never sent twice");
        assert!(matches!(admit(&ctx, "c2", &ok), Err(BridgeEnd::Denied("auth.bad_proof"))));

        let stranger = Hello { v: 1, device: Some("nobody".into()), proof: Some("00".into()), ..Default::default() };
        assert!(matches!(admit(&ctx, "c1", &stranger), Err(BridgeEnd::Denied("auth.unknown_device"))));

        let reg = Registry::load(&ctx.registry_path).unwrap();
        assert!(reg.device(&paired.device_id).unwrap().last_seen.is_some());
        assert!(reg.pending.is_empty());
    }

    #[test]
    fn a_hello_with_neither_shape_or_the_wrong_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        assert!(matches!(admit(&ctx, "c", &Hello { v: 1, ..Default::default() }), Err(BridgeEnd::BadHello(_))));
        assert!(matches!(admit(&ctx, "c", &Hello { v: 2, pair: Some("x".into()), ..Default::default() }), Err(BridgeEnd::BadHello(_))));
    }
}
