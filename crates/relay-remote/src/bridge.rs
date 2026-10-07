//! One paired conversation: greet, verify, then carry bus lines between a phone and the
//! engine's socket door. The transport underneath is either a WebSocket the phone opened
//! directly (`direct.rs`) or one lane of the rendezvous tunnel (`tunnel.rs`); both hand this
//! module a pair of channels and nothing else.

use crate::registry::{Outcome, Presented, Registry};
use crate::wire::{self, Gate, Greeting, Hello, Welcome, WIRE_V};
use anyhow::{Context, Result};
use relay_core::Instance;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

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

/// How long a phone has to answer the greeting. The transport bounds the time before the
/// greeting (`direct.rs`'s handshake deadline); this bounds the time after it.
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a phone presenting a code that needs the PC's approval waits for it. The phone
/// gives up on its welcome after 20 s (RelayClient.ts `HANDSHAKE_TIMEOUT_MS`), so this stays
/// under that; a person who is slower runs `relay remote pair` again.
const PAIR_CONFIRM_WAIT: Duration = Duration::from_secs(18);
const PAIR_POLL: Duration = Duration::from_millis(200);

/// How often an admitted connection checks that its device is still paired. A revoke from
/// another process only edits `remote.json`; this is what cuts a connected phone off. The check
/// is a `stat` until the file changes.
const RECHECK_EVERY: Duration = Duration::from_secs(1);

/// Lines queued toward the phone before the transport applies backpressure. Terminal output
/// arrives in bursts; a phone on a poor link must not park unbounded memory here.
pub const OUTBOUND_QUEUE: usize = 256;

/// Aborts a task when dropped, so a conversation that is itself aborted takes its helpers with it.
pub struct AbortOnDrop(pub AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

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
    #[error("the device was revoked while connected")]
    Revoked,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The outcome of a handshake, with the registry already written back.
pub struct Admitted {
    pub device_id: String,
    pub welcome: Welcome,
    /// The credential this connection was admitted with; it stays admitted while the registry
    /// still holds exactly this device and token.
    token: String,
}

/// Verify a `Hello` against the registry on disk and record the result. Pure enough to test
/// without a transport: the transport-side `run` calls exactly this. `origin` is how the
/// connection arrived, shown to the person asked to approve a pairing.
pub async fn admit(ctx: &Ctx, challenge: &str, hello: &Hello, origin: &str) -> std::result::Result<Admitted, BridgeEnd> {
    if hello.v != WIRE_V {
        return Err(BridgeEnd::BadHello(format!("wire v{} is not v{WIRE_V}", hello.v)));
    }
    let path = &ctx.registry_path;
    if let Some(code) = hello.pair.as_deref() {
        let name = hello.device_name.as_deref().unwrap_or("");
        let presented = Registry::update(path, |r| Ok::<_, BridgeEnd>(r.present(code, name, origin)))?;
        let device = match presented {
            Presented::Invalid => return Err(BridgeEnd::Denied("pair.invalid")),
            Presented::Paired(device) => device,
            Presented::Waiting => {
                tracing::warn!(device_name = %name, origin = %origin, instance = %ctx.instance,
                    "a phone presented the pairing code; waiting for `relay remote pair` to approve it");
                let deadline = tokio::time::Instant::now() + PAIR_CONFIRM_WAIT;
                loop {
                    match Registry::update(path, |r| Ok::<_, BridgeEnd>(r.outcome(code)))? {
                        Outcome::Approved(device) => break device,
                        Outcome::Declined => return Err(BridgeEnd::Denied("pair.declined")),
                        Outcome::Waiting if tokio::time::Instant::now() >= deadline => {
                            // Unanswered is not yes: the code is spent, and the person pairs again.
                            Registry::update(path, |r| {
                                r.abandon(code);
                                Ok::<_, BridgeEnd>(())
                            })?;
                            return Err(BridgeEnd::Denied("pair.unconfirmed"));
                        }
                        Outcome::Waiting => tokio::time::sleep(PAIR_POLL).await,
                    }
                }
            }
        };
        // Every pairing is announced where the PC's owner looks: the engine log, at warn.
        tracing::warn!(device = %device.id, device_name = %device.name, origin = %origin, instance = %ctx.instance,
            "a new phone paired with this PC; `relay remote devices` lists it, `relay remote revoke {}` removes it", device.id);
        return Ok(Admitted {
            device_id: device.id.clone(),
            welcome: Welcome::paired(&device.id, &device.token),
            token: device.token,
        });
    }
    let (Some(id), Some(proof)) = (hello.device.as_deref(), hello.proof.as_deref()) else {
        return Err(BridgeEnd::BadHello("expected `pair` or `device`+`proof`".into()));
    };
    Registry::update(path, |registry| {
        let device = registry.device(id).ok_or(BridgeEnd::Denied("auth.unknown_device"))?;
        if !wire::digest_eq(proof, &wire::proof(challenge, &device.token)) {
            return Err(BridgeEnd::Denied("auth.bad_proof"));
        }
        let token = device.token.clone();
        registry.touch(id);
        Ok(Admitted { device_id: id.to_string(), welcome: Welcome::admitted(id), token })
    })
}

/// What `still_admitted` compares to notice that `remote.json` changed without reading it.
fn stamp(path: &std::path::Path) -> Option<(std::time::SystemTime, u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.ino(), meta.len()))
}

/// Whether the registry still holds this device with this token. A registry that cannot be
/// read admits no one: it is the only record of who may be here.
fn still_admitted(ctx: &Ctx, device_id: &str, token: &str) -> bool {
    Registry::load(&ctx.registry_path).is_ok_and(|r| r.device(device_id).is_some_and(|d| d.token == token))
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
/// carries lines to it. Returns when either side goes away, or when the device is revoked.
/// `unproven` is whatever the transport holds for a connection that has not yet proved itself;
/// it is dropped once the device is admitted. `origin` says where the connection came from.
pub async fn run(
    ctx: Arc<Ctx>,
    mut inbound: mpsc::Receiver<String>,
    outbound: mpsc::Sender<String>,
    unproven: impl Send,
    origin: String,
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
    let admitted = match admit(&ctx, &challenge, &hello, &origin).await {
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
    drop(unproven);

    // Only now does the engine hear about this connection.
    let stream = UnixStream::connect(&ctx.socket_path)
        .await
        .with_context(|| format!("no engine at {}", ctx.socket_path.display()))?;
    relay_core::socket::same_user(&stream)?;
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
    // A transport that aborts this conversation must not leave the engine connection open.
    let pump_guard = AbortOnDrop(pump.abort_handle());

    let instance = ctx.instance;
    let mut recheck = tokio::time::interval(RECHECK_EVERY);
    recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut seen = stamp(&ctx.registry_path);
    let mut revoked = false;
    let result: Result<()> = async {
        loop {
            let line = tokio::select! {
                line = inbound.recv() => match line {
                    Some(line) => line,
                    None => break,
                },
                _ = recheck.tick() => {
                    let now = stamp(&ctx.registry_path);
                    if now != seen {
                        seen = now;
                        if !still_admitted(&ctx, &admitted.device_id, &admitted.token) {
                            revoked = true;
                            break;
                        }
                    }
                    continue;
                }
            };
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
    drop(pump_guard);
    if revoked {
        tracing::warn!(device = %admitted.device_id, "remote device revoked; its connection is closed");
        return Err(BridgeEnd::Revoked);
    }
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

    #[tokio::test]
    async fn pairing_mints_a_token_that_then_proves_itself() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        let mut reg = Registry::fresh();
        let code = reg.begin_pair(false).code;
        reg.save(&ctx.registry_path).unwrap();

        let hello = Hello { v: 1, pair: Some(code.clone()), device_name: Some("Pixel".into()), ..Default::default() };
        let paired = admit(&ctx, "c0", &hello, "test").await.unwrap();
        let token = paired.welcome.token.clone().unwrap();

        // The code is spent.
        assert!(matches!(admit(&ctx, "c0", &hello, "test").await, Err(BridgeEnd::Denied("pair.invalid"))));

        // The token proves itself against a fresh challenge, and only that challenge.
        let ok = Hello { v: 1, device: Some(paired.device_id.clone()), proof: Some(wire::proof("c1", &token)), ..Default::default() };
        let admitted = admit(&ctx, "c1", &ok, "test").await.unwrap();
        assert!(admitted.welcome.token.is_none(), "the token is never sent twice");
        assert!(matches!(admit(&ctx, "c2", &ok, "test").await, Err(BridgeEnd::Denied("auth.bad_proof"))));

        let stranger = Hello { v: 1, device: Some("nobody".into()), proof: Some("00".into()), ..Default::default() };
        assert!(matches!(admit(&ctx, "c1", &stranger, "test").await, Err(BridgeEnd::Denied("auth.unknown_device"))));

        let reg = Registry::load(&ctx.registry_path).unwrap();
        assert!(reg.device(&paired.device_id).unwrap().last_seen.is_some());
        assert!(reg.pending.is_empty());
    }

    #[tokio::test]
    async fn a_hello_with_neither_shape_or_the_wrong_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        assert!(matches!(admit(&ctx, "c", &Hello { v: 1, ..Default::default() }, "test").await, Err(BridgeEnd::BadHello(_))));
        assert!(matches!(admit(&ctx, "c", &Hello { v: 2, pair: Some("x".into()), ..Default::default() }, "test").await, Err(BridgeEnd::BadHello(_))));
    }

    #[tokio::test]
    async fn a_confirmed_code_pairs_only_when_the_pc_says_yes() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Arc::new(ctx(&dir));
        let code = Registry::update(&ctx.registry_path, |r| Ok::<_, anyhow::Error>(r.begin_pair(true).code)).unwrap();
        let hello = Hello { v: 1, pair: Some(code.clone()), device_name: Some("Pixel".into()), ..Default::default() };

        // The PC approves while the phone waits.
        let waiting = {
            let (ctx, hello) = (ctx.clone(), hello.clone());
            tokio::spawn(async move { admit(&ctx, "c", &hello, "direct 192.0.2.7").await })
        };
        let approved = loop {
            let decided = Registry::update(&ctx.registry_path, |r| {
                let asked = r.pending.first().and_then(|p| p.request.clone());
                Ok::<_, anyhow::Error>(asked.filter(|q| q.origin == "direct 192.0.2.7").map(|_| r.decide(&code, true)))
            })
            .unwrap();
            if let Some(decided) = decided {
                break decided;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(approved);
        let paired = waiting.await.unwrap().unwrap();
        assert!(paired.welcome.token.is_some());

        // A declined phone gets nothing, and the code is spent.
        let code = Registry::update(&ctx.registry_path, |r| Ok::<_, anyhow::Error>(r.begin_pair(true).code)).unwrap();
        let hello = Hello { v: 1, pair: Some(code.clone()), device_name: Some("Stranger".into()), ..Default::default() };
        let waiting = {
            let ctx = ctx.clone();
            tokio::spawn(async move { admit(&ctx, "c", &hello, "rendezvous").await })
        };
        while !Registry::update(&ctx.registry_path, |r| Ok::<_, anyhow::Error>(r.decide(&code, false))).unwrap() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(matches!(waiting.await.unwrap(), Err(BridgeEnd::Denied("pair.declined"))));
        let reg = Registry::load(&ctx.registry_path).unwrap();
        assert_eq!(reg.devices.len(), 1);
        assert!(reg.pending.is_empty());
    }
}
