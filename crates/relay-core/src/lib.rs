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

/// Lowercase hex for a byte string, in one allocation.
///
/// Session tokens and every audit payload hash go through here. `bytes.iter().map(|b|
/// format!("{b:02x}")).collect()` is the same string built out of one heap allocation per byte.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod hex_tests {
    #[test]
    fn hex_matches_the_per_byte_formatter() {
        for case in [vec![], vec![0u8], vec![0x0f, 0xf0, 0xff], (0u8..=255).collect::<Vec<_>>()] {
            let expected: String = case.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(super::hex(&case), expected);
        }
    }
}
