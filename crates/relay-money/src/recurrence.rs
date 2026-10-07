//! Repeating dates and where a saved bill is next due. Port of `Recurrence.kt`.
//!
//! The arithmetic is `java.time`'s: adding months or years clamps the day to the end of a shorter
//! month, and `ChronoUnit.{WEEKS,MONTHS,YEARS}.between` counts whole units only (Jan 31 to Feb 28
//! is 0 months).

use crate::period::{days_between, plus_months};
use jiff::civil::Date;
use jiff::ToSpan;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Frequency {
    Weekly,
    Monthly,
    Yearly,
}

/// A repeating date: every `interval` `frequency` units from `anchor`. Occurrence k is always
/// computed from the anchor, never chained from the previous one, so a bill anchored on Jan 31
/// lands on Feb 28 and then back on Mar 31 instead of drifting to the 28th forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Recurrence {
    pub anchor: Date,
    pub frequency: Frequency,
    pub interval: i32,
}

/// What [`Recurrence::between`] returns at most, so a bill whose anchor sits years in the past
/// cannot post a thousand rows in one pass.
pub const BETWEEN_LIMIT: usize = 366;

/// `ChronoUnit.MONTHS.between(a, b)`: whole months, a partial month not counted.
fn months_between(a: Date, b: Date) -> i64 {
    let packed = |d: Date| (i64::from(d.year()) * 12 + i64::from(d.month()) - 1) * 32 + i64::from(d.day());
    (packed(b) - packed(a)) / 32
}

impl Recurrence {
    /// # Panics
    /// When `interval` is not between 1 and 52.
    pub fn new(anchor: Date, frequency: Frequency, interval: i32) -> Self {
        assert!((1..=52).contains(&interval), "Interval must be between 1 and 52");
        Recurrence { anchor, frequency, interval }
    }

    /// Every one unit: the Kotlin constructor's default interval.
    pub fn every(anchor: Date, frequency: Frequency) -> Self {
        Recurrence::new(anchor, frequency, 1)
    }

    pub fn occurrence(&self, k: i64) -> Date {
        let n = k * i64::from(self.interval);
        match self.frequency {
            Frequency::Weekly => self.anchor.checked_add((n * 7).days()).expect("date in range"),
            Frequency::Monthly => plus_months(self.anchor, n),
            Frequency::Yearly => self.anchor.checked_add(n.years()).expect("date in range"),
        }
    }

    /// The first occurrence on or after `date`.
    pub fn on_or_after(&self, date: Date) -> Date {
        if date <= self.anchor {
            return self.anchor;
        }
        let units = match self.frequency {
            Frequency::Weekly => days_between(self.anchor, date) / 7,
            Frequency::Monthly => months_between(self.anchor, date),
            Frequency::Yearly => months_between(self.anchor, date) / 12,
        };
        let mut k = (units / i64::from(self.interval)).max(0);
        // The estimate can sit one step early (month clamping); walk forward to the true answer.
        while self.occurrence(k) < date {
            k += 1;
        }
        self.occurrence(k)
    }

    /// The first occurrence strictly after `date`.
    pub fn after(&self, date: Date) -> Date {
        self.on_or_after(date.tomorrow().expect("date in range"))
    }

    /// Every occurrence from `from` through `through`, inclusive, capped at [`BETWEEN_LIMIT`].
    pub fn between(&self, from: Date, through: Date) -> Vec<Date> {
        self.between_limited(from, through, BETWEEN_LIMIT)
    }

    /// Every occurrence from `from` through `through`, inclusive, capped at `limit` so a bill whose
    /// anchor sits years in the past cannot post a thousand rows in one pass.
    pub fn between_limited(&self, from: Date, through: Date, limit: usize) -> Vec<Date> {
        let mut out = Vec::new();
        let mut d = self.on_or_after(from);
        while d <= through && out.len() < limit {
            out.push(d);
            d = self.after(d);
        }
        out
    }

    /// Roughly how many occurrences fall in a 30.44-day month, for "per month" readings.
    pub fn per_month_factor(&self) -> f64 {
        let i = f64::from(self.interval);
        match self.frequency {
            Frequency::Weekly => 30.44 / (7.0 * i),
            Frequency::Monthly => 1.0 / i,
            Frequency::Yearly => 1.0 / (12.0 * i),
        }
    }
}

/// A bill's schedule as the database holds it: the rule it follows, the next date it is due,
/// whether it posts itself (off makes it a reminder) and whether it runs at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BillSchedule {
    pub rule: Recurrence,
    pub next: Date,
    pub auto_post: bool,
    pub active: bool,
}

impl BillSchedule {
    /// Posting itself and active, the Kotlin defaults.
    pub fn new(rule: Recurrence, next: Date) -> Self {
        BillSchedule { rule, next, auto_post: true, active: true }
    }
}

/// The date a bill is next due once it is saved with `rule`, `auto_post` and `active` over
/// `stored`, the schedule the database holds for it (`None` for a new bill). The stored schedule
/// is the authority, never an editor's copy of it, so saving cannot post a date twice and never
/// back-fills dates nobody asked for:
///
/// - A new bill starts at its first date from today, or from `not_before` when that is later.
/// - A changed rule starts again at its first date from today.
/// - A paused bill keeps its stored date; turning it back on is what moves it.
/// - A bill that was posting itself and still does keeps its stored date even when it is behind:
///   those dates are owed, and the poster catches them up.
/// - Any other bill (one turned back on, one switched to posting itself, a reminder) keeps its
///   stored date unless that date is behind, and then starts at its first date from today. A
///   paused bill does not owe the dates it was paused for, and a reminder's past dates were never
///   going to post.
///
/// Every fresh start also falls strictly after `last_posted`, the latest date already entered
/// against the bill, so changing the rule on a day the bill posted does not post that day again.
pub fn next_due_on_save(
    rule: Recurrence,
    auto_post: bool,
    active: bool,
    stored: Option<BillSchedule>,
    today: Date,
    last_posted: Option<Date>,
    not_before: Option<Date>,
) -> Date {
    let start = match last_posted {
        Some(p) if p >= today => p.tomorrow().expect("date in range"),
        _ => today,
    };
    let Some(stored) = stored else {
        return rule.on_or_after(not_before.filter(|nb| *nb > start).unwrap_or(start));
    };
    if stored.rule != rule {
        return rule.on_or_after(start);
    }
    if !active {
        return stored.next;
    }
    if stored.active && stored.auto_post && auto_post {
        return stored.next;
    }
    if stored.next < start { rule.on_or_after(start) } else { stored.next }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn d(m: i8, day: i8) -> Date {
        date(2026, m, day)
    }

    fn monthly_rent() -> Recurrence {
        Recurrence::every(d(7, 1), Frequency::Monthly)
    }

    #[test]
    fn frequencies_serialize_by_their_kotlin_names() {
        assert_eq!(serde_json::to_string(&Frequency::Weekly).unwrap(), "\"WEEKLY\"");
        assert_eq!(serde_json::from_str::<Frequency>("\"YEARLY\"").unwrap(), Frequency::Yearly);
    }

    #[test]
    fn whole_months_only_as_chrono_unit_counts_them() {
        assert_eq!(months_between(d(1, 31), d(2, 28)), 0);
        assert_eq!(months_between(d(1, 31), d(3, 31)), 2);
        assert_eq!(months_between(d(1, 15), d(10, 14)), 8);
        assert_eq!(months_between(date(2028, 2, 29), date(2029, 2, 28)) / 12, 0);
    }

    #[test]
    #[should_panic(expected = "Interval must be between 1 and 52")]
    fn an_interval_out_of_range_is_refused() {
        Recurrence::new(d(1, 1), Frequency::Weekly, 53);
    }

    #[test]
    fn month_end_anchors_do_not_drift() {
        let r = Recurrence::every(d(1, 31), Frequency::Monthly);
        assert_eq!(r.occurrence(1), d(2, 28));
        assert_eq!(r.occurrence(2), d(3, 31));
        assert_eq!(r.after(d(2, 28)), d(3, 31));
    }

    #[test]
    fn on_or_after_returns_the_anchor_before_it_starts() {
        let r = Recurrence::every(d(5, 10), Frequency::Monthly);
        assert_eq!(r.on_or_after(d(1, 1)), d(5, 10));
    }

    #[test]
    fn biweekly_salary() {
        let r = Recurrence::new(d(10, 2), Frequency::Weekly, 2);
        assert_eq!(r.after(d(10, 2)), d(10, 16));
        assert_eq!(r.between(d(10, 1), d(10, 31)), vec![d(10, 2), d(10, 16), d(10, 30)]);
    }

    #[test]
    fn yearly_on_a_leap_day_lands_on_feb_28_then_back() {
        let r = Recurrence::every(date(2028, 2, 29), Frequency::Yearly);
        assert_eq!(r.occurrence(1), date(2029, 2, 28));
        assert_eq!(r.occurrence(4), date(2032, 2, 29));
    }

    #[test]
    fn between_is_capped() {
        let r = Recurrence::every(date(2000, 1, 1), Frequency::Weekly);
        assert_eq!(r.between_limited(date(2000, 1, 1), date(2026, 1, 1), 10).len(), 10);
    }

    #[test]
    fn per_month_factor() {
        assert!((Recurrence::every(d(1, 1), Frequency::Monthly).per_month_factor() - 1.0).abs() <= 1e-9);
        assert!((Recurrence::every(d(1, 1), Frequency::Yearly).per_month_factor() - 1.0 / 12.0).abs() <= 1e-9);
    }

    // ── Where a saved bill is next due ───────────────────────────────────────

    #[test]
    fn a_new_bill_starts_at_its_first_date_from_today_and_never_back_fills() {
        let rent = monthly_rent();
        assert_eq!(next_due_on_save(rent, true, true, None, d(10, 4), None, None), d(11, 1));
        assert_eq!(next_due_on_save(rent, true, true, None, d(5, 15), None, None), d(7, 1), "An anchor ahead is its own first date");
        assert_eq!(next_due_on_save(rent, true, true, None, d(10, 1), None, None), d(10, 1), "Due today starts today");
    }

    #[test]
    fn a_new_bill_keeps_a_later_start_it_was_given_and_ignores_an_earlier_one() {
        // A repeat made from an entry dated today starts after the entry, not on it.
        let weekly = Recurrence::every(d(10, 4), Frequency::Weekly);
        assert_eq!(next_due_on_save(weekly, true, true, None, d(10, 4), None, Some(d(10, 11))), d(10, 11));
        assert_eq!(next_due_on_save(weekly, true, true, None, d(10, 4), None, Some(d(9, 1))), d(10, 4));
    }

    #[test]
    fn an_unchanged_bill_that_posts_itself_keeps_the_stored_date_ahead_or_behind() {
        let rent = monthly_rent();
        // The poster already moved it on: whatever an editor held, the stored date stands.
        let posted = BillSchedule::new(rent, d(11, 1));
        assert_eq!(next_due_on_save(rent, true, true, Some(posted), d(10, 1), None, None), d(11, 1));
        // Still owed: the poster catches these up, so saving must not drop them.
        let behind = BillSchedule::new(rent, d(9, 1));
        assert_eq!(next_due_on_save(rent, true, true, Some(behind), d(10, 4), None, None), d(9, 1));
    }

    #[test]
    fn switching_a_reminder_to_post_itself_starts_from_today_not_from_its_old_date() {
        let rent = monthly_rent();
        let reminder = BillSchedule { auto_post: false, ..BillSchedule::new(rent, d(7, 1)) };
        assert_eq!(next_due_on_save(rent, true, true, Some(reminder), d(10, 4), None, None), d(11, 1));
        let current = BillSchedule { auto_post: false, ..BillSchedule::new(rent, d(11, 1)) };
        assert_eq!(next_due_on_save(rent, true, true, Some(current), d(10, 4), None, None), d(11, 1));
    }

    #[test]
    fn a_reminder_moves_past_dates_that_are_over_and_keeps_todays() {
        let rent = monthly_rent();
        let behind = BillSchedule { auto_post: false, ..BillSchedule::new(rent, d(7, 1)) };
        assert_eq!(next_due_on_save(rent, false, true, Some(behind), d(10, 4), None, None), d(11, 1));
        let due_today = BillSchedule { auto_post: false, ..BillSchedule::new(rent, d(10, 1)) };
        assert_eq!(next_due_on_save(rent, false, true, Some(due_today), d(10, 1), None, None), d(10, 1));
    }

    #[test]
    fn resuming_starts_from_today_and_staying_paused_keeps_the_date() {
        let rent = monthly_rent();
        let paused = BillSchedule { active: false, ..BillSchedule::new(rent, d(8, 1)) };
        assert_eq!(next_due_on_save(rent, true, true, Some(paused), d(10, 4), None, None), d(11, 1));
        assert_eq!(next_due_on_save(rent, true, false, Some(paused), d(10, 4), None, None), d(8, 1));
        let paused_ahead = BillSchedule { active: false, ..BillSchedule::new(rent, d(12, 1)) };
        assert_eq!(next_due_on_save(rent, true, true, Some(paused_ahead), d(10, 4), None, None), d(12, 1));
    }

    #[test]
    fn a_rule_changed_on_a_day_the_bill_posted_starts_after_that_day() {
        let phone = Recurrence::every(d(1, 15), Frequency::Monthly);
        let stored = BillSchedule::new(phone, d(11, 15));
        let quarterly = Recurrence { interval: 3, ..phone };
        // Jan 15 every three months lands on Oct 15, today, which already posted.
        assert_eq!(next_due_on_save(quarterly, true, true, Some(stored), d(10, 15), Some(d(10, 15)), None), date(2027, 1, 15));
        // 273 days from Jan 15 is exactly 39 weeks: weekly would land on today too.
        let weekly = Recurrence { frequency: Frequency::Weekly, ..phone };
        assert_eq!(next_due_on_save(weekly, true, true, Some(stored), d(10, 15), Some(d(10, 15)), None), d(10, 22));
        // On a day it did not post, a changed rule may start today.
        assert_eq!(next_due_on_save(quarterly, true, true, Some(stored), d(10, 15), Some(d(9, 15)), None), d(10, 15));
    }

    #[test]
    fn switching_to_post_itself_never_lands_on_a_date_already_posted() {
        let rent = monthly_rent();
        let reminder = BillSchedule { auto_post: false, ..BillSchedule::new(rent, d(10, 1)) };
        assert_eq!(next_due_on_save(rent, true, true, Some(reminder), d(10, 1), Some(d(10, 1)), None), d(11, 1));
    }
}
