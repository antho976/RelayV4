//! `relay remote …` — the phone door (crates/relay-remote), driven from the CLI.
//!
//! `serve` fronts a running engine on the LAN and, when a rendezvous is configured, dials out
//! to it. `pair` opens a ten-minute window and prints the code as a QR for the phone's camera.
//! Everything else edits `remote.json`.

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use relay_core::Instance;
use relay_remote::direct::{lan_addresses, DirectServer};
use relay_remote::pairlink::PairLink;
use relay_remote::rendezvous::RendezvousServer;
use relay_remote::{Ctx, Registry, DEFAULT_PORT, DEFAULT_RENDEZVOUS_PORT};
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Subcommand)]
pub enum RemoteCommand {
    /// Front the running engine for paired phones: on the LAN, and through the rendezvous if one is set
    Serve {
        /// Address to listen on for phones on the same network
        #[arg(long, default_value_t = format!("0.0.0.0:{DEFAULT_PORT}"))]
        bind: String,
        /// Also open a pairing window and print its QR code at start
        #[arg(long)]
        pair: bool,
        /// Do not dial the configured rendezvous this time
        #[arg(long)]
        no_rendezvous: bool,
    },
    /// Open a ten-minute pairing window and print the code and QR for the phone
    Pair,
    /// List paired phones
    Devices,
    /// Forget a paired phone by id
    Revoke { id: String },
    /// Rename this machine as phones see it
    Name { name: String },
    /// Set (or with `off`, clear) the rendezvous server this engine dials out to
    Via { url: String },
    /// Run a rendezvous server: what `via` points at. Host it anywhere you trust, behind TLS.
    Rendezvous {
        #[arg(long, default_value_t = format!("0.0.0.0:{DEFAULT_RENDEZVOUS_PORT}"))]
        bind: String,
    },
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RELAY_LOG").unwrap_or_else(|_| "info".into()),
        )
        .init();
}

fn print_pair_link(link: &PairLink) {
    println!();
    if let Some(qr) = link.qr() {
        println!("{qr}");
    }
    println!("Pairing code: {}   (valid for 10 minutes, one phone)", link.code);
    println!("Host: {} ({})", link.host, link.instance);
    if link.direct.is_empty() {
        println!("Direct: no LAN address found; the phone can only reach this engine through a rendezvous");
    } else {
        println!("Direct: {}", link.direct.join("  "));
    }
    match &link.via {
        Some(via) => println!("Via:    {via}"),
        None => println!("Via:    none — set one with `relay remote via wss://your-server` to reach this PC away from home"),
    }
    println!("\nScan the code from the Relay app's PC tab, or type the link:\n{}\n", link.to_url());
}

/// `relay serve --remote`: the engine and its phone door in one process, so a machine that
/// starts Relay at login is reachable from the phone without a second command.
pub async fn serve_with_door(
    instance: Instance,
    store: Option<std::path::PathBuf>,
    bind: &str,
) -> std::result::Result<(), relay_core::socket::BindError> {
    let addr: SocketAddr = bind
        .parse()
        .with_context(|| format!("bad --remote-bind {bind:?}"))
        .map_err(relay_core::socket::BindError::Other)?;
    let served = relay_core::serve::start(instance, store).await?;
    tracing::info!(instance = %instance, store = %served.engine.store.path().display(), "engine up");
    let ctx = Arc::new(Ctx {
        instance,
        registry_path: Registry::path_for(instance),
        socket_path: served.socket.path.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    });
    // The door is a convenience on top of the engine, never a reason for the engine not to
    // start: a port already taken by another instance's door is logged and skipped.
    let door = match DirectServer::bind(ctx.clone(), addr).await {
        Ok(door) => Some(door),
        Err(e) => {
            eprintln!("relay serve: phone door not opened ({e:#}); the engine runs without it — `relay remote serve --bind <addr>` opens one later");
            None
        }
    };
    let mut registry = Registry::load(&ctx.registry_path).map_err(relay_core::socket::BindError::Other)?;
    if let Some(door) = &door {
        registry.direct_port = Some(door.local_addr.port());
    }
    registry.save(&ctx.registry_path).map_err(relay_core::socket::BindError::Other)?;
    let tunnel = registry
        .rendezvous
        .as_ref()
        .map(|r| relay_remote::tunnel::spawn(ctx.clone(), r.clone()));
    if let Some(door) = &door {
        println!("relay serve: phone door on {} ({}); `relay remote pair` adds a phone", door.local_addr,
            match &registry.rendezvous { Some(r) => format!("dialing {}", r.url), None => "no rendezvous".to_string() });
    }
    let quit = served.engine.clone();
    tokio::select! {
        _ = quit.wait_quit() => tracing::info!("app.quit received"),
        _ = tokio::signal::ctrl_c() => tracing::info!("SIGINT"),
        _ = sigterm() => tracing::info!("SIGTERM"),
    }
    if let Some(t) = tunnel {
        t.abort();
    }
    drop(door);
    served.engine.shutdown();
    drop(served);
    Ok(())
}

async fn sigterm() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut s) => {
            s.recv().await;
        }
        Err(_) => std::future::pending::<()>().await,
    }
}

/// The desktop launcher runs the `dev` engine by default and `relay` defaults to `stable`, so
/// the first thing a person types would otherwise be answered with "no engine". When the
/// asked-for instance has no engine and exactly one other does, use that one and say so;
/// `remote.json` is per instance, so pairing follows the engine it belongs to.
async fn resolve_instance(requested: Instance) -> Instance {
    if relay_core::socket::probe(&requested.socket_path()).await {
        return requested;
    }
    let mut alive = Vec::new();
    for candidate in [Instance::Stable, Instance::Dev, Instance::Test] {
        if candidate != requested && relay_core::socket::probe(&candidate.socket_path()).await {
            alive.push(candidate);
        }
    }
    match alive.as_slice() {
        [only] => {
            eprintln!("relay remote: no {requested} engine is running; using the {only} engine (pass --instance to choose)");
            *only
        }
        _ => requested,
    }
}

pub async fn run(requested: Instance, cmd: RemoteCommand) -> Result<u8> {
    let instance = match cmd {
        RemoteCommand::Rendezvous { .. } => requested,
        _ => resolve_instance(requested).await,
    };
    let ctx = Arc::new(Ctx::for_instance(instance));
    match cmd {
        RemoteCommand::Serve { bind, pair, no_rendezvous } => {
            init_logging();
            let addr: SocketAddr = bind.parse().with_context(|| format!("bad --bind {bind:?}"))?;
            if !relay_core::socket::probe(&ctx.socket_path).await {
                eprintln!(
                    "relay remote: no engine answering at {} — start one with `./run.sh`, `relay serve`, or `relay serve --remote` (which needs no second command)",
                    ctx.socket_path.display()
                );
                return Ok(5);
            }
            let door = DirectServer::bind(ctx.clone(), addr).await?;
            let mut registry = Registry::load(&ctx.registry_path)?;
            registry.direct_port = Some(door.local_addr.port());
            let link_code = pair.then(|| registry.begin_pair().code);
            registry.save(&ctx.registry_path)?;

            let tunnel = match (&registry.rendezvous, no_rendezvous) {
                (Some(r), false) => Some(relay_remote::tunnel::spawn(ctx.clone(), r.clone())),
                _ => None,
            };
            println!("relay remote: listening on {} for phones on this network", door.local_addr);
            match (&registry.rendezvous, tunnel.is_some()) {
                (Some(r), true) => println!("relay remote: dialing rendezvous {} (room {})", r.url, r.room),
                (Some(_), false) => println!("relay remote: rendezvous configured but skipped (--no-rendezvous)"),
                (None, _) => println!("relay remote: no rendezvous configured; phones must be on this network"),
            }
            if let Some(code) = link_code {
                // A door bound to one address is reachable at that address only.
                let addresses = match door.local_addr.ip() {
                    std::net::IpAddr::V4(ip) if !ip.is_unspecified() => vec![ip],
                    _ => lan_addresses(),
                };
                let link = PairLink::build(&registry, instance.as_str(), &code, door.local_addr.port(), &addresses);
                print_pair_link(&link);
            } else {
                println!("relay remote: run `relay remote pair` in another terminal to add a phone");
            }
            tokio::signal::ctrl_c().await?;
            if let Some(t) = tunnel {
                t.abort();
            }
            drop(door);
            Ok(0)
        }
        RemoteCommand::Pair => {
            let mut registry = Registry::load(&ctx.registry_path)?;
            let port = registry.direct_port.unwrap_or(DEFAULT_PORT);
            let code = registry.begin_pair().code;
            registry.save(&ctx.registry_path)?;
            let link = PairLink::build(&registry, instance.as_str(), &code, port, &lan_addresses());
            if registry.direct_port.is_none() {
                println!("note: `relay remote serve` has not run yet on this instance; the link assumes port {DEFAULT_PORT}");
            }
            print_pair_link(&link);
            Ok(0)
        }
        RemoteCommand::Devices => {
            let registry = Registry::load(&ctx.registry_path)?;
            if registry.devices.is_empty() {
                println!("no paired phones (run `relay remote pair`)");
                return Ok(0);
            }
            println!("{:<14} {:<24} {:<26} last seen", "id", "name", "paired");
            for d in &registry.devices {
                println!("{:<14} {:<24} {:<26} {}", d.id, d.name, d.created_at, d.last_seen.as_deref().unwrap_or("never"));
            }
            if !registry.pending.is_empty() {
                println!("\n{} pairing window(s) open", registry.pending.len());
            }
            Ok(0)
        }
        RemoteCommand::Revoke { id } => {
            let mut registry = Registry::load(&ctx.registry_path)?;
            if !registry.revoke(&id) {
                return Err(anyhow!("no paired phone with id {id:?}"));
            }
            registry.save(&ctx.registry_path)?;
            println!("revoked {id}; its next connection will be refused");
            Ok(0)
        }
        RemoteCommand::Name { name } => {
            let mut registry = Registry::load(&ctx.registry_path)?;
            let name = name.trim();
            if name.is_empty() {
                return Err(anyhow!("a name cannot be empty"));
            }
            registry.host_name = name.chars().take(64).collect();
            registry.save(&ctx.registry_path)?;
            println!("phones will see this machine as {:?}", registry.host_name);
            Ok(0)
        }
        RemoteCommand::Via { url } => {
            let mut registry = Registry::load(&ctx.registry_path)?;
            if url.eq_ignore_ascii_case("off") {
                registry.rendezvous = None;
                registry.save(&ctx.registry_path)?;
                println!("rendezvous cleared; phones must be on this network");
                return Ok(0);
            }
            if !(url.starts_with("ws://") || url.starts_with("wss://")) {
                return Err(anyhow!("the rendezvous URL must start with ws:// or wss:// (wss:// is what you want on the internet)"));
            }
            if url.starts_with("ws://") {
                eprintln!("warning: ws:// is unencrypted; a pairing through it exposes the phone's token to the network in between. Put TLS in front (see docs/MOBILE.md).");
            }
            let r = registry.set_rendezvous(&url).clone();
            registry.save(&ctx.registry_path)?;
            println!("rendezvous: {}\nroom:       {}\nphones join at {}", r.url, r.room, relay_remote::tunnel::join_url(&r));
            println!("restart `relay remote serve` to dial it");
            Ok(0)
        }
        RemoteCommand::Rendezvous { bind } => {
            init_logging();
            let addr: SocketAddr = bind.parse().with_context(|| format!("bad --bind {bind:?}"))?;
            let server = RendezvousServer::bind(addr).await?;
            println!("relay rendezvous: listening on {} (health: GET /health)", server.local_addr);
            tokio::signal::ctrl_c().await?;
            drop(server);
            Ok(0)
        }
    }
}
