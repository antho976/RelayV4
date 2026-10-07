//! The native client's pure logic, kept out of the GTK crate so a headless `cargo test` (and CI)
//! runs it. `git_view` is the Git panel's reading of `git.status`, `git.diff` and `git.log`
//! results; `mirror` is the device mirror's video gate and the `device.mirror.input` payloads
//! it sends, which the engine's `tests/native_input.rs` builds with these same functions.
pub mod git_view;
pub mod mirror;
