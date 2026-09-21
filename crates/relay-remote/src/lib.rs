//! Relay's remote door: the same bus lines the Unix socket carries (BUS.md §6.2), over a
//! WebSocket, for a phone that paired with this machine once.
//!
//! Two routes, one protocol:
//!
//! - **direct** (`direct.rs`): the engine listens on the LAN; the phone connects when it is on
//!   the same WiFi. Nothing leaves the network.
//! - **via a rendezvous** (`rendezvous.rs`, `tunnel.rs`): the engine dials out to a server the
//!   person hosts, and phones anywhere join through it. The server copies lines and reads none
//!   of them; every check still happens here.
//!
//! Both routes hand `bridge.rs` a pair of channels. It greets, verifies the device against
//! `remote.json` (`registry.rs`), then forwards bus lines to the socket door as actor `user`
//! and back. The phone never gets a door the CLI does not have.

pub mod bridge;
pub mod direct;
pub mod pairlink;
pub mod registry;
pub mod rendezvous;
pub mod tunnel;
pub mod wire;

pub use bridge::Ctx;
pub use registry::{Registry, DEFAULT_PORT, DEFAULT_RENDEZVOUS_PORT};
