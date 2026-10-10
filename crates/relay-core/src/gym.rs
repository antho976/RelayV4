//! Avex's training history on the PC (docs/GYM.md): `gym.db` beside the store, opened on first
//! use (in memory when the store is, as in tests), and the history it holds, loaded once and kept.
//!
//! Nothing here touches the store or its lock. Reads take the shelf's mutex only to clone the
//! history's `Arc`, then read without it; an import holds it for the one transaction that
//! replaces the file.

use crate::engine::Engine;
use relay_bus::BusError;
use relay_gym::model::History;
use relay_gym::store::Gym;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default)]
pub struct Shelf {
    opened: Mutex<Option<Opened>>,
}

struct Opened {
    gym: Gym,
    history: Arc<History>,
    imported: bool,
}

fn bus(e: rusqlite::Error) -> BusError {
    BusError::internal(format!("gym.db: {e}"))
}

fn opened(engine: &Engine) -> Result<MutexGuard<'_, Option<Opened>>, BusError> {
    let mut slot = engine.gym.opened.lock().unwrap_or_else(|p| p.into_inner());
    if slot.is_none() {
        let store = engine.store.path();
        let gym = if store == Path::new(":memory:") { Gym::open_in_memory() } else { Gym::open(&store.with_file_name("gym.db")) }.map_err(bus)?;
        let history = Arc::new(gym.load().map_err(bus)?);
        let imported = gym.imported().map_err(bus)?;
        *slot = Some(Opened { gym, history, imported });
    }
    Ok(slot)
}

/// The history, and whether an export was ever imported.
pub fn history(engine: &Engine) -> Result<(Arc<History>, bool), BusError> {
    let slot = opened(engine)?;
    let o = slot.as_ref().expect("opened above");
    Ok((o.history.clone(), o.imported))
}

/// Replaces the PC's copy with `h`.
pub fn replace(engine: &Engine, h: History) -> Result<(), BusError> {
    let mut slot = opened(engine)?;
    let o = slot.as_mut().expect("opened above");
    o.gym.replace(&h).map_err(bus)?;
    o.history = Arc::new(h);
    o.imported = true;
    Ok(())
}

/// Forgets the copy.
pub fn clear(engine: &Engine) -> Result<(), BusError> {
    let mut slot = opened(engine)?;
    let o = slot.as_mut().expect("opened above");
    o.gym.clear().map_err(bus)?;
    o.history = Arc::new(History::default());
    o.imported = false;
    Ok(())
}
