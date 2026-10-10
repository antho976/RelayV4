//! Avex, the person's gym app, in the Threads space (docs/GYM.md).
//!
//! Avex has no internet permission, so its data reaches the PC as the file it already writes:
//! Settings, Export, Training history (`avex_export.json`). [`export::read`] turns that file into a
//! [`model::History`]; the readings Relay draws and a thread's agent asks for are pure functions
//! of it ([`views`]). The PC's copy is read-only: each import replaces it whole, because the file
//! is the phone's whole history. The SQLite copy (`gym.db`) sits behind the `store` feature, so
//! the bus contract does not pull in SQLite.
//!
//! Weights are stored as Avex stores them, in pounds, and shown in the person's unit (Avex's
//! `useKg`), rounded as Avex rounds them on screen.

pub mod export;
pub mod model;
pub mod views;

#[cfg(feature = "store")]
pub mod store;
