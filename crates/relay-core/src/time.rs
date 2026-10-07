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
/// A window reaching past the earliest representable instant is clamped to it rather than
/// panicking: `retention_days` is a user setting, and an absurd one must not stop launch.
pub fn days_ago(days: i64) -> String {
    // ~27,000 years, past either end of jiff's range, and small enough not to overflow seconds.
    let hours = days.clamp(-10_000_000, 10_000_000) * 24;
    Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_hours(hours))
        .unwrap_or(if days >= 0 { Timestamp::MIN } else { Timestamp::MAX })
        .to_string()
}

/// A caller's `since`/`until` bound in the stored format, so the plain string comparison against
/// `ts` columns means what it says. Accepts RFC 3339 with any offset (`T` or a space between date
/// and time), a bare date (midnight UTC), or Unix epoch seconds or milliseconds. Anything else is
/// `invalid` rather than a comparison that silently returns the wrong rows.
pub fn bound(field: &str, raw: &str) -> Result<String, relay_bus::error::BusError> {
    let raw = raw.trim();
    let parsed = if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
        // Seconds until the year 5138; anything longer is milliseconds.
        raw.parse::<i64>().ok().and_then(|n| if n < 100_000_000_000 { Timestamp::from_second(n).ok() } else { Timestamp::from_millisecond(n).ok() })
    } else {
        raw.parse::<Timestamp>().ok()
            .or_else(|| (raw.len() == 10).then(|| raw.parse::<jiff::civil::Date>().ok()).flatten().and_then(|d| d.to_zoned(jiff::tz::TimeZone::UTC).ok()).map(|z| z.timestamp()))
    };
    parsed.map(|t| t.to_string()).ok_or_else(|| relay_bus::error::BusError::invalid(
        "time.invalid",
        format!("{field} must be an RFC 3339 timestamp with an offset, a date, or epoch seconds; got {raw:?}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absurd_retention_window_clamps_instead_of_panicking() {
        let now = now();
        for days in [i64::MAX, i64::MAX / 24, 10_000_000, 4_000_000] {
            let cutoff = days_ago(days);
            assert!(cutoff.as_str() < now.as_str(), "{days} days ago sorts after now: {cutoff}");
        }
        assert!(days_ago(i64::MIN).as_str() > now.as_str());
        assert!(days_ago(180).as_str() < now.as_str());
    }

    #[test]
    fn bounds_are_normalised_to_the_stored_utc_format() {
        let utc = "2026-10-07T02:00:00Z";
        for raw in ["2026-10-07T02:00:00Z", "2026-10-07T04:00:00+02:00", "2026-10-07 02:00:00Z", "1791338400", "1791338400000"] {
            assert_eq!(bound("since", raw).unwrap(), utc, "{raw}");
        }
        assert_eq!(bound("since", "2026-10-07").unwrap(), "2026-10-07T00:00:00Z");
        for raw in ["yesterday", "2026-10-07T02:00:00", "", "07/10/2026"] {
            assert_eq!(bound("since", raw).unwrap_err().code, "time.invalid", "{raw}");
        }
    }
}
