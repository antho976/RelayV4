//! RFC 3339 UTC timestamps, the one format the bus speaks (BUS.md §1.3).

use jiff::Timestamp;

pub fn now() -> String {
    stamp(Timestamp::now())
}

/// The stored form of an instant: always nine fractional digits, so stamps are one width and
/// their text sorts in time order. jiff's plain `Display` drops trailing zeros, which puts
/// `…21Z` after `…21.5Z`.
pub fn stamp(ts: Timestamp) -> String {
    format!("{ts:.9}")
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
    let cutoff = Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_hours(hours))
        .unwrap_or(if days >= 0 { Timestamp::MIN } else { Timestamp::MAX });
    stamp(cutoff)
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
    parsed.map(stamp).ok_or_else(|| relay_bus::error::BusError::invalid(
        "time.invalid",
        format!("{field} must be an RFC 3339 timestamp with an offset, a date, or epoch seconds; got {raw:?}"),
    ))
}

/// The smallest unit a [`span`] shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit { Second, Minute }

/// A length of time as its largest whole unit and the next one down, each truncated: "2d 5h",
/// "4h 30m", "3m 12s", "45s". Below `finest` it reads as none of it ("0m"). The one wording the
/// engine uses for how long something has run and how long until something happens (RA-689).
pub fn span(seconds: u64, finest: Unit) -> String {
    const UNITS: [(u64, &str); 4] = [(86_400, "d"), (3600, "h"), (60, "m"), (1, "s")];
    let units = match finest { Unit::Second => &UNITS[..], Unit::Minute => &UNITS[..3] };
    let first = units.iter().position(|(size, _)| seconds >= *size).unwrap_or(units.len() - 1);
    let (size, name) = units[first];
    match units.get(first + 1) {
        Some((next, next_name)) => format!("{}{name} {}{next_name}", seconds / size, seconds % size / next),
        None => format!("{}{name}", seconds / size),
    }
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
    fn stamps_are_one_width_so_text_order_is_time_order() {
        let whole = Timestamp::new(1_760_000_000, 0).unwrap();
        let later = Timestamp::new(1_760_000_000, 500_000_000).unwrap();
        assert!(stamp(whole) < stamp(later), "{} / {}", stamp(whole), stamp(later));
        assert_eq!(stamp(whole).len(), stamp(later).len());
        assert_eq!(bound("since", "2025-10-09").unwrap(), "2025-10-09T00:00:00.000000000Z");
    }

    #[test]
    fn spans_show_the_largest_unit_and_the_next_down() {
        assert_eq!(span(0, Unit::Second), "0s");
        assert_eq!(span(45, Unit::Second), "45s");
        assert_eq!(span(3 * 60 + 12, Unit::Second), "3m 12s");
        assert_eq!(span(2 * 3600 + 5 * 60 + 59, Unit::Second), "2h 5m");
        assert_eq!(span(26 * 3600 + 3 * 60, Unit::Second), "1d 2h");
        assert_eq!(span(0, Unit::Minute), "0m", "the client reads 0m as a window that has reset");
        assert_eq!(span(59, Unit::Minute), "0m");
        assert_eq!(span(5 * 60 + 59, Unit::Minute), "5m");
        assert_eq!(span(4 * 3600 + 30 * 60, Unit::Minute), "4h 30m");
        assert_eq!(span(2 * 86_400 + 5 * 3600 + 9 * 60, Unit::Minute), "2d 5h");
    }

    #[test]
    fn bounds_are_normalised_to_the_stored_utc_format() {
        let utc = "2026-10-07T02:00:00.000000000Z";
        for raw in ["2026-10-07T02:00:00Z", "2026-10-07T04:00:00+02:00", "2026-10-07 02:00:00Z", "1791338400", "1791338400000"] {
            assert_eq!(bound("since", raw).unwrap(), utc, "{raw}");
        }
        assert_eq!(bound("since", "2026-10-07").unwrap(), "2026-10-07T00:00:00.000000000Z");
        for raw in ["yesterday", "2026-10-07T02:00:00", "", "07/10/2026"] {
            assert_eq!(bound("since", raw).unwrap_err().code, "time.invalid", "{raw}");
        }
    }
}
