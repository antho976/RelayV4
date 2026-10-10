//! Arbiter (docs/ARBITER.md): rule-based crypto trading and AI trading research on the person's
//! own exchange account, beside Tally in the Threads space.
//!
//! The rules, indicators, backtester, paper fills and the risk gate are pure functions over
//! candles and decimals, tested here. Every order from any source passes [`gate::check`] before it
//! reaches an exchange, and the gate refuses rather than resizes. The SQLite book (`arbiter.db`)
//! and the Coinbase client sit behind the `engine` feature, so the bus contract does not pull in
//! SQLite or TLS.

pub mod backtest;
pub mod exchange;
pub mod gate;
pub mod indicators;
pub mod model;
pub mod rules;
pub mod views;

#[cfg(feature = "engine")]
pub mod book;
#[cfg(feature = "engine")]
pub mod coinbase;
#[cfg(feature = "engine")]
pub mod paper;

pub use rust_decimal::Decimal;
