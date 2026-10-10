//! A rule's meaning: when it holds on a bar, when a schedule is due, when a holding should be
//! sold, whether a rule is well formed, and the plain sentences the person and the agent read.

use crate::indicators;
use crate::model::{Candle, Compare, Condition, Entry, Every, Exit, Granularity, Operand, Pricing, Rule};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// The deepest an `all`/`any` may nest, and the most conditions in a rule.
const MAX_DEPTH: usize = 4;
const MAX_CONDITIONS: usize = 16;
/// The longest an indicator's period may be, in bars.
pub const MAX_PERIOD: u32 = 500;

/// A rule's operands computed over one run of candles, so each condition reads a value per bar.
pub struct Evaluator<'a> {
    pub candles: &'a [Candle],
    values: HashMap<String, Vec<Option<f64>>>,
}

fn key(o: &Operand) -> String {
    serde_json::to_string(o).unwrap_or_default()
}

fn operands<'c>(c: &'c Condition, out: &mut Vec<&'c Operand>) {
    match c {
        Condition::Compare { left, right, .. } => {
            out.push(left);
            out.push(right);
        }
        Condition::All { of } | Condition::Any { of } => of.iter().for_each(|c| operands(c, out)),
    }
}

impl<'a> Evaluator<'a> {
    pub fn new(candles: &'a [Candle], conditions: &[&Condition]) -> Evaluator<'a> {
        let mut ops = Vec::new();
        for c in conditions {
            operands(c, &mut ops);
        }
        let mut values = HashMap::new();
        for o in ops {
            values.entry(key(o)).or_insert_with(|| indicators::series(candles, o));
        }
        Evaluator { candles, values }
    }

    /// An evaluator for everything `rule` reads.
    pub fn for_rule(candles: &'a [Candle], rule: &Rule) -> Evaluator<'a> {
        let mut conds: Vec<&Condition> = Vec::new();
        if let Entry::Signal { when } = &rule.entry {
            conds.push(when);
        }
        if let Some(when) = &rule.exit.when {
            conds.push(when);
        }
        Evaluator::new(candles, &conds)
    }

    pub fn value(&self, o: &Operand, i: usize) -> Option<f64> {
        self.values.get(&key(o)).and_then(|v| v.get(i).copied().flatten())
    }

    /// Whether `c` holds on bar `i`. False when any value it needs is not computed yet.
    pub fn holds(&self, c: &Condition, i: usize) -> bool {
        match c {
            Condition::All { of } => !of.is_empty() && of.iter().all(|c| self.holds(c, i)),
            Condition::Any { of } => of.iter().any(|c| self.holds(c, i)),
            Condition::Compare { left, op, right } => {
                let (Some(l), Some(r)) = (self.value(left, i), self.value(right, i)) else { return false };
                match op {
                    Compare::Above => l > r,
                    Compare::Below => l < r,
                    Compare::CrossesAbove | Compare::CrossesBelow => {
                        if i == 0 {
                            return false;
                        }
                        let (Some(pl), Some(pr)) = (self.value(left, i - 1), self.value(right, i - 1)) else { return false };
                        if *op == Compare::CrossesAbove { pl <= pr && l > r } else { pl >= pr && l < r }
                    }
                }
            }
        }
    }
}

/// Whether a scheduled buy falls in `(after, upto]`, Unix seconds, UTC.
pub fn schedule_due(entry: &Entry, after: i64, upto: i64) -> bool {
    next_scheduled(entry, after).is_some_and(|t| t <= upto)
}

/// The first scheduled time strictly after `after`, Unix seconds, UTC.
pub fn next_scheduled(entry: &Entry, after: i64) -> Option<i64> {
    let Entry::Schedule { every, hour, weekday, day } = entry else { return None };
    let hour = i64::from(*hour).min(23);
    let day_start = after.div_euclid(86400) * 86400;
    // Walk forward day by day; a month holds the next match within 31 days.
    for d in 0..=62 {
        let midnight = day_start + d * 86400;
        let t = midnight + hour * 3600;
        if t <= after {
            continue;
        }
        let date = jiff::Timestamp::from_second(midnight).ok()?.to_zoned(jiff::tz::TimeZone::UTC).date();
        let ok = match every {
            Every::Day => true,
            Every::Week => i64::from(date.weekday().to_monday_one_offset()) == i64::from(weekday.unwrap_or(1)),
            Every::Month => i64::from(date.day()) == i64::from(day.unwrap_or(1)),
        };
        if ok {
            return Some(t);
        }
    }
    None
}

/// Why a holding is sold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    TakeProfit,
    StopLoss,
    Trailing,
    MaxBars,
    Signal,
}

impl ExitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ExitReason::TakeProfit => "take profit",
            ExitReason::StopLoss => "stop loss",
            ExitReason::Trailing => "trailing stop",
            ExitReason::MaxBars => "time limit",
            ExitReason::Signal => "sell signal",
        }
    }
}

fn pct(d: Option<Decimal>) -> Option<f64> {
    d.and_then(|d| d.to_f64())
}

/// The price-based exits, checked against any price (a stop fires between bars, not only at a
/// close). `cost` is the average buy price, `peak` the highest price seen since.
pub fn price_exit(exit: &Exit, cost: f64, peak: f64, price: f64) -> Option<ExitReason> {
    if cost <= 0.0 {
        return None;
    }
    if let Some(sl) = pct(exit.stop_loss_pct) {
        if price <= cost * (1.0 - sl / 100.0) {
            return Some(ExitReason::StopLoss);
        }
    }
    if let Some(tr) = pct(exit.trailing_pct) {
        if peak > 0.0 && price <= peak * (1.0 - tr / 100.0) {
            return Some(ExitReason::Trailing);
        }
    }
    if let Some(tp) = pct(exit.take_profit_pct) {
        if price >= cost * (1.0 + tp / 100.0) {
            return Some(ExitReason::TakeProfit);
        }
    }
    None
}

/// The bar-based exits, on a closed bar `i`, `held` bars after the buy.
pub fn bar_exit(exit: &Exit, eval: &Evaluator, i: usize, held: u32) -> Option<ExitReason> {
    if exit.max_bars.is_some_and(|m| held >= m) {
        return Some(ExitReason::MaxBars);
    }
    if exit.when.as_ref().is_some_and(|w| eval.holds(w, i)) {
        return Some(ExitReason::Signal);
    }
    None
}

/// How many bars before the first a rule needs to decide on it.
pub fn warmup(rule: &Rule) -> u32 {
    let mut ops = Vec::new();
    if let Entry::Signal { when } = &rule.entry {
        operands(when, &mut ops);
    }
    if let Some(when) = &rule.exit.when {
        operands(when, &mut ops);
    }
    // One more for a cross, which reads the bar before.
    ops.into_iter().map(indicators::warmup).max().unwrap_or(0) + 1
}

/// A stable fingerprint of a rule: an approval binds to this, so what runs is what was shown.
pub fn hash(rule: &Rule) -> String {
    let json = serde_json::to_string(rule).unwrap_or_default();
    let digest = Sha256::digest(json.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn check_operand(o: &Operand) -> Result<(), String> {
    let n = match *o {
        Operand::Sma { period } | Operand::Ema { period } | Operand::Rsi { period } => period,
        Operand::Change { bars } | Operand::High { bars } | Operand::Low { bars } => bars,
        Operand::Number { value } => {
            return if value.is_finite() { Ok(()) } else { Err("A number in the rule is not a number".into()) };
        }
        Operand::Price => return Ok(()),
    };
    if n == 0 || n > MAX_PERIOD {
        return Err(format!("An indicator's period must be 1 to {MAX_PERIOD} bars"));
    }
    Ok(())
}

fn check_condition(c: &Condition, depth: usize, count: &mut usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("Conditions may nest {MAX_DEPTH} deep at most"));
    }
    match c {
        Condition::Compare { left, right, .. } => {
            *count += 1;
            if *count > MAX_CONDITIONS {
                return Err(format!("A rule may have {MAX_CONDITIONS} conditions at most"));
            }
            check_operand(left)?;
            check_operand(right)
        }
        Condition::All { of } | Condition::Any { of } => {
            if of.is_empty() {
                return Err("An \"all\" or \"any\" needs at least one condition".into());
            }
            of.iter().try_for_each(|c| check_condition(c, depth + 1, count))
        }
    }
}

fn check_pct(name: &str, p: Option<Decimal>) -> Result<(), String> {
    match p {
        Some(p) if p <= Decimal::ZERO || p >= Decimal::ONE_HUNDRED => Err(format!("The {name} must be between 0 and 100 percent")),
        _ => Ok(()),
    }
}

/// Whether a rule is one Arbiter can run.
pub fn validate(rule: &Rule) -> Result<(), String> {
    if rule.buy <= Decimal::ZERO {
        return Err("The amount to buy must be more than zero".into());
    }
    let mut count = 0;
    match &rule.entry {
        Entry::Signal { when } => check_condition(when, 1, &mut count)?,
        Entry::Schedule { every, hour, weekday, day } => {
            if *hour > 23 {
                return Err("The hour must be 0 to 23".into());
            }
            match every {
                Every::Week if !weekday.is_some_and(|w| (1..=7).contains(&w)) => return Err("A weekly buy needs a weekday, 1 (Monday) to 7".into()),
                Every::Month if !day.is_some_and(|d| (1..=28).contains(&d)) => return Err("A monthly buy needs a day of the month, 1 to 28".into()),
                _ => {}
            }
        }
    }
    check_pct("take profit", rule.exit.take_profit_pct)?;
    check_pct("stop loss", rule.exit.stop_loss_pct)?;
    check_pct("trailing stop", rule.exit.trailing_pct)?;
    if rule.exit.max_bars == Some(0) {
        return Err("A time limit must be at least one bar".into());
    }
    if let Some(w) = &rule.exit.when {
        check_condition(w, 1, &mut count)?;
    }
    Ok(())
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v}") }
}

fn operand(o: &Operand) -> String {
    match *o {
        Operand::Price => "the price".into(),
        Operand::Sma { period } => format!("its {period}-bar average"),
        Operand::Ema { period } => format!("its {period}-bar exponential average"),
        Operand::Rsi { period } => format!("RSI({period})"),
        Operand::Change { bars } => format!("its change over {bars} bars (%)"),
        Operand::High { bars } => format!("the highest high of the last {bars} bars"),
        Operand::Low { bars } => format!("the lowest low of the last {bars} bars"),
        Operand::Number { value } => num(value),
    }
}

/// A condition in words.
pub fn condition(c: &Condition) -> String {
    match c {
        Condition::Compare { left, op, right } => {
            let verb = match op {
                Compare::Above => "is above",
                Compare::Below => "is below",
                Compare::CrossesAbove => "rises above",
                Compare::CrossesBelow => "falls below",
            };
            format!("{} {verb} {}", operand(left), operand(right))
        }
        Condition::All { of } => of.iter().map(condition).collect::<Vec<_>>().join(" and "),
        Condition::Any { of } => {
            let parts: Vec<String> = of.iter().map(condition).collect();
            format!("either {}", parts.join(" or "))
        }
    }
}

const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// The rule as the sentences the strategy page and the agent read.
pub fn describe(rule: &Rule, product: &str, g: Granularity) -> Vec<String> {
    let (base, quote) = product.split_once('-').unwrap_or((product, ""));
    let amount = format!("{} {quote}", rule.buy.normalize()).trim().to_string();
    let how = match rule.pricing {
        Pricing::Market => "at the market price",
        Pricing::Limit => "with a limit order at the best bid",
    };
    let first = match &rule.entry {
        Entry::Schedule { every, hour, weekday, day } => {
            let when = match every {
                Every::Day => format!("Every day at {hour:02}:00 UTC"),
                Every::Week => format!("Every {} at {hour:02}:00 UTC", WEEKDAYS[usize::from(weekday.unwrap_or(1).clamp(1, 7)) - 1]),
                Every::Month => format!("On day {} of every month at {hour:02}:00 UTC", day.unwrap_or(1)),
            };
            format!("{when}, buy {amount} of {base} {how}.")
        }
        Entry::Signal { when } => {
            format!("When {} on {} {} bars, buy {amount} of {base} {how}, if none is held.", condition(when), product, g.short())
        }
    };
    let mut out = vec![first];
    let e = &rule.exit;
    let mut sells = Vec::new();
    if let Some(p) = e.take_profit_pct {
        sells.push(format!("the price is up {}%", p.normalize()));
    }
    if let Some(p) = e.stop_loss_pct {
        sells.push(format!("down {}%", p.normalize()));
    }
    if let Some(p) = e.trailing_pct {
        sells.push(format!("{}% below its high since the buy", p.normalize()));
    }
    if let Some(n) = e.max_bars {
        sells.push(format!("after {n} bars"));
    }
    if let Some(w) = &e.when {
        sells.push(condition(w));
    }
    if sells.is_empty() {
        out.push(match rule.entry {
            Entry::Schedule { .. } => "It never sells: what it buys is kept.".into(),
            Entry::Signal { .. } => "It never sells by itself, so it buys once and holds.".into(),
        });
    } else {
        let last = sells.pop().unwrap_or_default();
        let list = if sells.is_empty() { last } else { format!("{}, or {last}", sells.join(", ")) };
        out.push(format!("Sell it when {list}."));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Every, Pricing};

    fn candles(closes: &[f64]) -> Vec<Candle> {
        closes.iter().enumerate().map(|(i, &c)| Candle { start: i as i64 * 3600, open: c, high: c, low: c, close: c, volume: 1.0 }).collect()
    }

    fn rsi_rule() -> Rule {
        Rule {
            entry: Entry::Signal {
                when: Condition::Compare { left: Operand::Rsi { period: 14 }, op: Compare::Below, right: Operand::Number { value: 30.0 } },
            },
            buy: Decimal::new(50, 0),
            pricing: Pricing::Limit,
            exit: Exit { take_profit_pct: Some(Decimal::new(4, 0)), stop_loss_pct: Some(Decimal::new(3, 0)), max_bars: Some(120), ..Default::default() },
        }
    }

    #[test]
    fn crosses_need_the_bar_before() {
        let c = candles(&[1.0, 2.0, 3.0, 2.0, 1.0]);
        let cond = Condition::Compare { left: Operand::Price, op: Compare::CrossesAbove, right: Operand::Number { value: 2.5 } };
        let e = Evaluator::new(&c, &[&cond]);
        assert_eq!((0..5).map(|i| e.holds(&cond, i)).collect::<Vec<_>>(), vec![false, false, true, false, false]);
        let down = Condition::Compare { left: Operand::Price, op: Compare::CrossesBelow, right: Operand::Number { value: 2.5 } };
        let e = Evaluator::new(&c, &[&down]);
        assert!(e.holds(&down, 3));
    }

    #[test]
    fn an_indicator_without_history_never_holds() {
        let c = candles(&[1.0, 2.0]);
        let cond = Condition::Compare { left: Operand::Sma { period: 5 }, op: Compare::Below, right: Operand::Number { value: 100.0 } };
        let e = Evaluator::new(&c, &[&cond]);
        assert!(!e.holds(&cond, 1));
    }

    #[test]
    fn weekly_schedule_lands_on_its_weekday() {
        // 2026-10-05 is a Monday; 00:00 UTC that day.
        let monday = jiff::civil::date(2026, 10, 5).to_zoned(jiff::tz::TimeZone::UTC).unwrap().timestamp().as_second();
        let e = Entry::Schedule { every: Every::Week, hour: 9, weekday: Some(1), day: None };
        assert_eq!(next_scheduled(&e, monday), Some(monday + 9 * 3600));
        assert_eq!(next_scheduled(&e, monday + 9 * 3600), Some(monday + 7 * 86400 + 9 * 3600));
        assert!(schedule_due(&e, monday + 8 * 3600, monday + 10 * 3600));
        assert!(!schedule_due(&e, monday + 10 * 3600, monday + 20 * 3600));
    }

    #[test]
    fn exits_by_price() {
        let x = Exit { take_profit_pct: Some(Decimal::new(4, 0)), stop_loss_pct: Some(Decimal::new(3, 0)), trailing_pct: Some(Decimal::new(5, 0)), ..Default::default() };
        assert_eq!(price_exit(&x, 100.0, 100.0, 104.0), Some(ExitReason::TakeProfit));
        assert_eq!(price_exit(&x, 100.0, 100.0, 97.0), Some(ExitReason::StopLoss));
        assert_eq!(price_exit(&x, 100.0, 103.0, 98.0), None);
        assert_eq!(price_exit(&x, 90.0, 110.0, 104.0), Some(ExitReason::Trailing));
    }

    #[test]
    fn validation_refuses_nonsense() {
        assert!(validate(&rsi_rule()).is_ok());
        let mut r = rsi_rule();
        r.buy = Decimal::ZERO;
        assert!(validate(&r).is_err());
        let mut r = rsi_rule();
        r.exit.stop_loss_pct = Some(Decimal::new(150, 0));
        assert!(validate(&r).is_err());
        let mut r = rsi_rule();
        r.entry = Entry::Signal { when: Condition::All { of: vec![] } };
        assert!(validate(&r).is_err());
        let mut r = rsi_rule();
        r.entry = Entry::Schedule { every: Every::Week, hour: 9, weekday: None, day: None };
        assert!(validate(&r).is_err());
    }

    #[test]
    fn sentences_read_like_the_mockup() {
        let s = describe(&rsi_rule(), "ETH-CAD", Granularity::OneHour);
        assert_eq!(s[0], "When RSI(14) is below 30 on ETH-CAD 1h bars, buy 50 CAD of ETH with a limit order at the best bid, if none is held.");
        assert_eq!(s[1], "Sell it when the price is up 4%, down 3%, or after 120 bars.");
        let dca = Rule { entry: Entry::Schedule { every: Every::Week, hour: 9, weekday: Some(1), day: None }, buy: Decimal::new(25, 0), pricing: Pricing::Market, exit: Exit::default() };
        let s = describe(&dca, "BTC-CAD", Granularity::OneDay);
        assert_eq!(s[0], "Every Monday at 09:00 UTC, buy 25 CAD of BTC at the market price.");
        assert_eq!(s[1], "It never sells: what it buys is kept.");
    }

    #[test]
    fn the_hash_follows_the_rule() {
        let a = rsi_rule();
        let mut b = rsi_rule();
        assert_eq!(hash(&a), hash(&b));
        b.exit.take_profit_pct = Some(Decimal::new(3, 0));
        assert_ne!(hash(&a), hash(&b));
    }
}
