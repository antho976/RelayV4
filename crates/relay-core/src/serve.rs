//! `relay serve` and the Tauri shell both do this: open the store exclusively, build the
//! engine, open the socket door, run until quit.

use crate::engine::Engine;
use crate::paths::Instance;
use crate::socket::{BindError, SocketServer};
use crate::store::Store;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

/// A running engine with its socket door. Keep it alive; drop to stop.
pub struct Served {
    pub engine: Arc<Engine>,
    pub socket: SocketServer,
}

/// Open the store (exclusive), build the engine, bind the socket. Does not block.
pub async fn start(instance: Instance, store_path: Option<PathBuf>) -> Result<Served, BindError> {
    let path = store_path.unwrap_or_else(|| instance.store_path());
    let store = Store::open(&path, true).map_err(BindError::Other)?;
    let engine = Engine::new(instance, store);
    if let Err(e) = crate::recovery::run(&engine) {
        tracing::warn!(error = %e, "crash recovery failed; continuing");
    }
    // Every checkout picks up the app-wide skill folders once at start, so a workspace that
    // already existed when a skill was installed does not wait for its next launch (D147).
    // One shot on a worker, not a timer: nothing here repeats.
    let skills = engine.clone();
    std::thread::Builder::new()
        .name("skill-materialize".into())
        .spawn(move || crate::skills::refresh_all(&skills))
        .ok();
    let socket = SocketServer::start(engine.clone()).await?;
    Ok(Served { engine, socket })
}

/// `relay serve`: start, then block until `app.quit` or SIGINT/SIGTERM.
pub async fn serve(instance: Instance, store_path: Option<PathBuf>) -> Result<(), BindError> {
    let served = start(instance, store_path).await?;
    tracing::info!(instance = %instance, store = %served.engine.store.path().display(), "engine up");
    let quit = served.engine.clone();
    tokio::select! {
        _ = quit.wait_quit() => tracing::info!("app.quit received"),
        _ = tokio::signal::ctrl_c() => tracing::info!("SIGINT"),
        _ = sigterm() => tracing::info!("SIGTERM"),
    }
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
