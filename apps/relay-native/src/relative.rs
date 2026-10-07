//! How long ago a moment was, worded the same way in every panel. Each unit is whole and
//! truncated, so 90 minutes is "1 h ago" in the guardrail tray, the session card and the
//! notification list alike, and "1h" on the board.
use glib::DateTime;

/// The wording a panel has room for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Form {
    /// "just now", "5 min ago", "3 h ago", "yesterday", "Oct 4", "Oct 4, 2025".
    Long,
    /// A card's corner: "now", "5m", "3h", "2d", "3w", "Mar 1".
    Compact,
    /// [`Form::Compact`] read as a phrase: "just now", "5m ago", "3w ago", "on Mar 1".
    CompactAgo,
}

/// An engine timestamp (RFC 3339; a missing zone reads as UTC) relative to now, or `None`
/// when it does not parse, so each panel says what an unreadable time shows.
pub fn ago(ts: &str, form: Form) -> Option<String> {
    let then = DateTime::from_iso8601(ts, Some(&glib::TimeZone::utc())).ok()?;
    Some(between(&then, &DateTime::now_utc().ok()?, form))
}

/// A Unix time in seconds relative to now.
pub fn ago_unix(at: u64, form: Form) -> String {
    match (DateTime::from_unix_utc(at as i64), DateTime::now_utc()) {
        (Ok(then), Ok(now)) => between(&then, &now, form),
        _ => String::new(),
    }
}

/// `then` as seen from `now`. A moment in the future reads as now: a clock a little ahead of
/// the engine's should not make anything "in 2s".
pub fn between(then: &DateTime, now: &DateTime, form: Form) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const FIVE_WEEKS: i64 = 5 * WEEK;
    let seconds = now.difference(then).as_seconds().max(0);
    let local = |d: &DateTime| d.to_local().ok();
    let date = |pattern: &str| local(then).and_then(|d| d.format(pattern).ok()).map(|s| s.to_string()).unwrap_or_default();
    if form == Form::Long {
        return match seconds {
            0..MIN => "just now".into(),
            MIN..HOUR => format!("{} min ago", seconds / MIN),
            HOUR..DAY => format!("{} h ago", seconds / HOUR),
            _ => {
                let (Some(now), Some(then)) = (local(now), local(then)) else {
                    return String::new();
                };
                if seconds < 2 * DAY && now.add_days(-1).is_ok_and(|y| y.ymd() == then.ymd()) {
                    "yesterday".into()
                } else {
                    date(if now.year() == then.year() { "%b %-d" } else { "%b %-d, %Y" })
                }
            }
        };
    }
    let (short, ago) = match seconds {
        0..MIN => return if form == Form::Compact { "now" } else { "just now" }.into(),
        MIN..HOUR => (format!("{}m", seconds / MIN), true),
        HOUR..DAY => (format!("{}h", seconds / HOUR), true),
        DAY..WEEK => (format!("{}d", seconds / DAY), true),
        WEEK..FIVE_WEEKS => (format!("{}w", seconds / WEEK), true),
        _ => (date("%b %-d"), false),
    };
    match (form, ago) {
        (Form::CompactAgo, true) => format!("{short} ago"),
        (Form::CompactAgo, false) => format!("on {short}"),
        _ => short,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noon in July, local time, so no daylight-saving change falls in the days before it.
    fn noon() -> DateTime {
        DateTime::from_local(2026, 7, 15, 12, 0, 0.).unwrap()
    }

    fn back(seconds: i64) -> DateTime {
        noon().add_seconds(-seconds as f64).unwrap()
    }

    fn read(seconds: i64, form: Form) -> String {
        between(&back(seconds), &noon(), form)
    }

    #[test]
    fn every_unit_truncates() {
        assert_eq!(read(0, Form::Long), "just now");
        assert_eq!(read(59, Form::Long), "just now");
        assert_eq!(read(60, Form::Long), "1 min ago");
        assert_eq!(read(5 * 60 + 59, Form::Long), "5 min ago");
        assert_eq!(read(90 * 60, Form::Long), "1 h ago", "90 minutes is one hour, not two");
        assert_eq!(read(23 * 3600 + 59 * 60, Form::Long), "23 h ago");
        assert_eq!(read(90 * 60, Form::Compact), "1h");
        assert_eq!(read(90 * 60, Form::CompactAgo), "1h ago");
    }

    #[test]
    fn the_long_form_names_the_day_once_a_day_has_passed() {
        assert_eq!(read(30 * 3600, Form::Long), "yesterday");
        assert_eq!(read(9 * 86_400, Form::Long), "Jul 6");
        assert_eq!(read(400 * 86_400, Form::Long), "Jun 10, 2025");
    }

    #[test]
    fn the_compact_form_counts_up_to_weeks() {
        assert_eq!(read(30, Form::Compact), "now");
        assert_eq!(read(45 * 60, Form::Compact), "45m");
        assert_eq!(read(10 * 3600, Form::Compact), "10h");
        assert_eq!(read(3 * 86_400, Form::Compact), "3d");
        assert_eq!(read(21 * 86_400, Form::Compact), "3w");
        assert_eq!(read(136 * 86_400, Form::Compact), "Mar 1");
        assert_eq!(read(30, Form::CompactAgo), "just now");
        assert_eq!(read(45 * 60, Form::CompactAgo), "45m ago");
        assert_eq!(read(136 * 86_400, Form::CompactAgo), "on Mar 1");
    }

    #[test]
    fn a_future_moment_reads_as_now() {
        assert_eq!(between(&noon().add_minutes(5).unwrap(), &noon(), Form::Long), "just now");
        assert_eq!(between(&noon().add_minutes(5).unwrap(), &noon(), Form::Compact), "now");
    }

    #[test]
    fn engine_timestamps_parse_and_others_do_not() {
        let now = DateTime::now_utc().unwrap();
        let stamp = |d: &DateTime| d.format_iso8601().unwrap().to_string();
        assert_eq!(ago(&stamp(&now.add_minutes(-5).unwrap()), Form::Long).as_deref(), Some("5 min ago"));
        // The engine writes nanoseconds and a Z; a stamp without a zone reads as UTC.
        assert!(ago("2026-10-05T11:59:30.123456789Z", Form::Compact).is_some());
        let bare = now.add_hours(-3).unwrap().format("%Y-%m-%dT%H:%M:%S").unwrap();
        assert_eq!(ago(&bare, Form::Compact).as_deref(), Some("3h"));
        assert_eq!(ago("not a time", Form::Long), None);
        assert_eq!(ago("", Form::Compact), None);
        assert_eq!(ago_unix(now.to_unix() as u64 - 3 * 60 - 5, Form::CompactAgo), "3m ago");
    }
}
