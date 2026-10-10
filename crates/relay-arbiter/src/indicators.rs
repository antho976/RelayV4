//! Indicators over a run of candles, each a series as long as its input. A bar without enough
//! history to compute yet is `None`, never a made-up value: a rule cannot fire on a half-warmed
//! average.

use crate::model::{Candle, Operand};

pub fn closes(candles: &[Candle]) -> Vec<f64> {
    candles.iter().map(|c| c.close).collect()
}

/// Simple moving average.
pub fn sma(xs: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; xs.len()];
    if period == 0 {
        return out;
    }
    let mut sum = 0.0;
    for (i, x) in xs.iter().enumerate() {
        sum += x;
        if i >= period {
            sum -= xs[i - period];
        }
        if i + 1 >= period {
            out[i] = Some(sum / period as f64);
        }
    }
    out
}

/// Exponential moving average, seeded with the simple average of its first `period` values.
pub fn ema(xs: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; xs.len()];
    if period == 0 || xs.len() < period {
        return out;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut prev = xs[..period].iter().sum::<f64>() / period as f64;
    out[period - 1] = Some(prev);
    for i in period..xs.len() {
        prev = xs[i] * k + prev * (1.0 - k);
        out[i] = Some(prev);
    }
    out
}

/// Wilder's RSI: average gain over average loss, smoothed by `period`.
pub fn rsi(xs: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; xs.len()];
    if period == 0 || xs.len() <= period {
        return out;
    }
    let (mut gain, mut loss) = (0.0, 0.0);
    for i in 1..=period {
        let d = xs[i] - xs[i - 1];
        if d > 0.0 { gain += d } else { loss -= d }
    }
    gain /= period as f64;
    loss /= period as f64;
    let value = |g: f64, l: f64| if l == 0.0 { if g == 0.0 { 50.0 } else { 100.0 } } else { 100.0 - 100.0 / (1.0 + g / l) };
    out[period] = Some(value(gain, loss));
    for i in period + 1..xs.len() {
        let d = xs[i] - xs[i - 1];
        let (g, l) = if d > 0.0 { (d, 0.0) } else { (0.0, -d) };
        gain = (gain * (period as f64 - 1.0) + g) / period as f64;
        loss = (loss * (period as f64 - 1.0) + l) / period as f64;
        out[i] = Some(value(gain, loss));
    }
    out
}

/// Percent change over `bars` bars: 5.0 is +5%.
pub fn change(xs: &[f64], bars: usize) -> Vec<Option<f64>> {
    (0..xs.len())
        .map(|i| (bars > 0 && i >= bars && xs[i - bars] != 0.0).then(|| (xs[i] / xs[i - bars] - 1.0) * 100.0))
        .collect()
}

/// The highest high of the `bars` bars before each bar (this bar not included, so a breakout
/// above it is possible).
pub fn prior_high(candles: &[Candle], bars: usize) -> Vec<Option<f64>> {
    (0..candles.len())
        .map(|i| (bars > 0 && i >= bars).then(|| candles[i - bars..i].iter().map(|c| c.high).fold(f64::MIN, f64::max)))
        .collect()
}

pub fn prior_low(candles: &[Candle], bars: usize) -> Vec<Option<f64>> {
    (0..candles.len())
        .map(|i| (bars > 0 && i >= bars).then(|| candles[i - bars..i].iter().map(|c| c.low).fold(f64::MAX, f64::min)))
        .collect()
}

/// An operand's value at every bar.
pub fn series(candles: &[Candle], operand: &Operand) -> Vec<Option<f64>> {
    let xs = closes(candles);
    match *operand {
        Operand::Price => xs.into_iter().map(Some).collect(),
        Operand::Sma { period } => sma(&xs, period as usize),
        Operand::Ema { period } => ema(&xs, period as usize),
        Operand::Rsi { period } => rsi(&xs, period as usize),
        Operand::Change { bars } => change(&xs, bars as usize),
        Operand::High { bars } => prior_high(candles, bars as usize),
        Operand::Low { bars } => prior_low(candles, bars as usize),
        Operand::Number { value } => vec![Some(value); candles.len()],
    }
}

/// How many bars an operand needs before it has a value: what a backtest must fetch ahead of its
/// range so the first bar can already decide.
pub fn warmup(operand: &Operand) -> u32 {
    match *operand {
        Operand::Price | Operand::Number { .. } => 0,
        Operand::Sma { period } => period,
        // An EMA keeps the memory of its seed; three periods make it close to settled.
        Operand::Ema { period } => period.saturating_mul(3),
        Operand::Rsi { period } => period.saturating_mul(3).max(period + 1),
        Operand::Change { bars } | Operand::High { bars } | Operand::Low { bars } => bars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(xs: &[f64]) -> Vec<Candle> {
        xs.iter().enumerate().map(|(i, &c)| Candle { start: i as i64 * 60, open: c, high: c + 1.0, low: c - 1.0, close: c, volume: 1.0 }).collect()
    }

    #[test]
    fn sma_waits_for_its_period() {
        let s = sma(&[1.0, 2.0, 3.0, 4.0], 3);
        assert_eq!(s, vec![None, None, Some(2.0), Some(3.0)]);
    }

    #[test]
    fn ema_seeds_with_the_simple_average() {
        let e = ema(&[2.0, 4.0, 6.0, 8.0], 3);
        assert_eq!(e[2], Some(4.0));
        // k = 0.5: 8 * 0.5 + 4 * 0.5
        assert_eq!(e[3], Some(6.0));
    }

    #[test]
    fn rsi_is_100_when_it_only_rises_and_0_when_it_only_falls() {
        let up: Vec<f64> = (0..30).map(|i| i as f64).collect();
        assert_eq!(rsi(&up, 14)[29], Some(100.0));
        let down: Vec<f64> = (0..30).map(|i| 100.0 - i as f64).collect();
        assert_eq!(rsi(&down, 14)[29], Some(0.0));
        assert_eq!(rsi(&up, 14)[13], None);
    }

    #[test]
    fn rsi_matches_wilders_worked_example() {
        // StockCharts' worked 14-period example: the first RSI is 70.53, the second 66.32.
        let closes = [44.3389, 44.0902, 44.1497, 43.6124, 44.3278, 44.8264, 45.0955, 45.4245, 45.8433, 46.0826, 45.8931, 46.0328, 45.6140, 46.2820, 46.2820, 46.0028];
        let r = rsi(&closes, 14);
        assert!((r[14].unwrap() - 70.53).abs() < 0.05, "{:?}", r[14]);
        assert!((r[15].unwrap() - 66.32).abs() < 0.05, "{:?}", r[15]);
    }

    #[test]
    fn change_and_prior_extremes() {
        assert!((change(&[100.0, 110.0, 99.0], 1)[1].unwrap() - 10.0).abs() < 1e-9);
        let c = close(&[10.0, 12.0, 11.0, 15.0]);
        // Bar 3's prior 3 bars: highs 11, 13, 12.
        assert_eq!(prior_high(&c, 3)[3], Some(13.0));
        assert_eq!(prior_low(&c, 3)[3], Some(9.0));
        assert_eq!(prior_high(&c, 3)[2], None);
    }
}
