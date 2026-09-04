//! RFC 3339 UTC timestamps, the one format the bus speaks (BUS.md §1.3).

use jiff::Timestamp;

pub fn now() -> String {
    Timestamp::now().to_string()
}

pub fn now_ts() -> Timestamp {
    Timestamp::now()
}

/// The same format `days` in the past, for retention windows compared against stored `ts` values.
/// RFC 3339 UTC sorts lexicographically, so a plain `ts < cutoff` is the comparison.
pub fn days_ago(days: i64) -> String {
    (Timestamp::now() - jiff::SignedDuration::from_hours(days.saturating_mul(24))).to_string()
}
