//! Budget periods. Port of `Period.kt`.

use jiff::civil::Date;
use jiff::ToSpan;

/// One budget period: `start` inclusive to `end_exclusive` exclusive. A calendar month by
/// default; with a start day of 15 it runs from the 15th to the 14th of the next month, for
/// people paid on a fixed date. Start days stop at 28 so every month has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetPeriod {
    pub start: Date,
    pub end_exclusive: Date,
}

pub const MAX_START_DAY: i8 = 28;

/// Days from `a` to `b`, negative when `b` is earlier.
pub fn days_between(a: Date, b: Date) -> i64 {
    i64::from(a.until(b).expect("civil dates are in range").get_days())
}

/// `date` moved by `n` months, clamped to the end of a shorter month, as `LocalDate.plusMonths`.
pub fn plus_months(date: Date, n: i64) -> Date {
    date.checked_add(n.months()).expect("date in range")
}

impl BudgetPeriod {
    /// # Panics
    /// When the period is not at least one day long.
    pub fn new(start: Date, end_exclusive: Date) -> Self {
        assert!(end_exclusive > start, "A period must be at least one day long");
        BudgetPeriod { start, end_exclusive }
    }

    /// The period containing `date` for a cycle that starts on `start_day` of each month.
    pub fn containing(date: Date, start_day: i8) -> Self {
        let day = start_day.clamp(1, MAX_START_DAY);
        let this_month_start = date.with().day(day).build().expect("day 1..=28 exists in every month");
        let start = if date.day() >= day { this_month_start } else { plus_months(this_month_start, -1) };
        BudgetPeriod::new(start, plus_months(start, 1))
    }

    pub fn days(&self) -> i64 {
        days_between(self.start, self.end_exclusive)
    }

    pub fn last_day(&self) -> Date {
        self.end_exclusive.yesterday().expect("date in range")
    }

    pub fn contains(&self, date: Date) -> bool {
        date >= self.start && date < self.end_exclusive
    }

    /// Days elapsed INCLUDING `today`: 1 on the first day, `days` on the last. Clamped to the period.
    pub fn elapsed_days(&self, today: Date) -> i64 {
        if today < self.start {
            0
        } else if today >= self.end_exclusive {
            self.days()
        } else {
            days_between(self.start, today) + 1
        }
    }

    /// Days still to spend in, INCLUDING `today`: `days` on the first day, 1 on the last, 0 after.
    pub fn days_left(&self, today: Date) -> i64 {
        self.days() - self.elapsed_days(today) + i64::from(self.contains(today))
    }

    pub fn dates(&self) -> impl Iterator<Item = Date> {
        self.start.series(1.day()).take_while({
            let end = self.end_exclusive;
            move |d| *d < end
        })
    }

    /// The period `n` cycles away (negative goes back).
    pub fn shift(&self, n: i64) -> Self {
        let s = plus_months(self.start, n);
        BudgetPeriod::new(s, plus_months(s, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn calendar_month_by_default() {
        let p = BudgetPeriod::containing(date(2026, 10, 4), 1);
        assert_eq!(p.start, date(2026, 10, 1));
        assert_eq!(p.end_exclusive, date(2026, 11, 1));
        assert_eq!(p.days(), 31);
    }

    #[test]
    fn a_start_day_before_today_stays_in_this_month() {
        let p = BudgetPeriod::containing(date(2026, 10, 20), 15);
        assert_eq!(p.start, date(2026, 10, 15));
        assert_eq!(p.end_exclusive, date(2026, 11, 15));
    }

    #[test]
    fn a_start_day_after_today_belongs_to_last_months_cycle() {
        let p = BudgetPeriod::containing(date(2026, 10, 4), 15);
        assert_eq!(p.start, date(2026, 9, 15));
        assert_eq!(p.end_exclusive, date(2026, 10, 15));
    }

    #[test]
    fn start_days_clamp_to_28_so_february_has_one() {
        let p = BudgetPeriod::containing(date(2027, 2, 28), 31);
        assert_eq!(p.start, date(2027, 2, 28));
    }

    #[test]
    fn elapsed_and_left_both_count_today() {
        let p = BudgetPeriod::containing(date(2026, 10, 1), 1);
        let (first, last) = (date(2026, 10, 1), date(2026, 10, 31));
        assert_eq!(p.elapsed_days(first), 1);
        assert_eq!(p.days_left(first), 31);
        assert_eq!(p.elapsed_days(last), 31);
        assert_eq!(p.days_left(last), 1);
        assert_eq!(p.days_left(date(2026, 11, 3)), 0);
        assert_eq!(p.days_left(date(2026, 9, 3)), 31);
    }

    #[test]
    fn contains_is_half_open() {
        let p = BudgetPeriod::containing(date(2026, 10, 1), 1);
        assert!(p.contains(date(2026, 10, 31)));
        assert!(!p.contains(date(2026, 11, 1)));
    }

    #[test]
    fn shift_walks_whole_cycles() {
        let p = BudgetPeriod::containing(date(2026, 1, 20), 15);
        assert_eq!(p.shift(-1).start, date(2025, 12, 15));
        assert_eq!(p.shift(1).start, date(2026, 2, 15));
    }

    #[test]
    fn dates_cover_the_period_once() {
        let p = BudgetPeriod::containing(date(2027, 2, 10), 1);
        let all: Vec<_> = p.dates().collect();
        assert_eq!(all.len() as i64, p.days());
        assert_eq!(all.last(), Some(&p.last_day()));
    }
}
