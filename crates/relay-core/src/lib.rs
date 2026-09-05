//! RELAY v4 engine (BUS.md §5, §6): the store, the audit log, the request pipeline and the
//! socket door. Headless: `relay serve` runs exactly this; the Tauri shell embeds it.

// `BusError` is ~250 bytes and is the *normal* path for typed refusals; boxing it in every
// handler signature would obscure the API for no measurable gain.
#![allow(clippy::result_large_err)]

pub mod audit;
pub mod awareness;
pub mod device;
pub mod engine;
pub mod guardrail;
pub mod github;
pub mod handlers;
pub mod hooks;
pub mod mirror;
pub mod paths;
pub mod proc;
pub mod providers;
mod provider_updates;
pub mod pty;
pub mod recovery;
pub mod serve;
pub mod sessions;
pub mod skills;
pub mod socket;
pub mod store;
pub mod time;
pub mod usage;
pub mod watch;
pub mod worktree;

pub use engine::{Door, Engine};
pub use paths::Instance;
pub use store::Store;
