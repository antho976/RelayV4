//! The keypad's state. Port of `AmountInput.kt`.

use crate::money::{parse_amount, pow10};

/// The keypad's state: what has been typed, held as text so "12." and "12.0" stay distinct while
/// typing, and converted to minor units only when read. Immutable; every key returns a new value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AmountInput {
    pub text: String,
    pub fraction_digits: u32,
}

impl Default for AmountInput {
    fn default() -> Self {
        AmountInput::new("", 2)
    }
}

impl AmountInput {
    /// Ten billion in any currency is not a purchase; the cap keeps an `i64` far from overflow.
    pub const MAX_INTEGER_DIGITS: usize = 10;

    pub fn new(text: &str, fraction_digits: u32) -> Self {
        AmountInput { text: text.to_string(), fraction_digits }
    }

    /// An empty keypad for a currency with `fraction_digits` decimals.
    pub fn empty(fraction_digits: u32) -> Self {
        AmountInput::new("", fraction_digits)
    }

    pub fn minor(&self) -> i64 {
        parse_amount(if self.text.is_empty() { "0" } else { &self.text }, self.fraction_digits).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.minor() == 0
    }

    fn has_decimal(&self) -> bool {
        self.text.contains('.')
    }

    fn decimals(&self) -> usize {
        self.text.split_once('.').map_or(0, |(_, after)| after.chars().count())
    }

    fn integer_digits(&self) -> usize {
        self.text.split('.').next().unwrap_or("").chars().count()
    }

    fn with_text(&self, text: String) -> Self {
        AmountInput { text, fraction_digits: self.fraction_digits }
    }

    /// # Panics
    /// When `d` is not a single digit.
    pub fn digit(&self, d: u32) -> Self {
        assert!(d <= 9, "Failed requirement.");
        if self.has_decimal() && self.decimals() >= self.fraction_digits as usize {
            return self.clone();
        }
        if !self.has_decimal() && self.integer_digits() >= Self::MAX_INTEGER_DIGITS {
            return self.clone();
        }
        // No leading zeros: "0" then "5" is "5", not "05".
        if self.text == "0" {
            return self.with_text(d.to_string());
        }
        self.with_text(format!("{}{d}", self.text))
    }

    pub fn decimal(&self) -> Self {
        if self.fraction_digits == 0 || self.has_decimal() {
            return self.clone();
        }
        self.with_text(if self.text.is_empty() { "0.".to_string() } else { format!("{}.", self.text) })
    }

    pub fn backspace(&self) -> Self {
        let mut text = self.text.clone();
        text.pop();
        self.with_text(text)
    }

    pub fn clear(&self) -> Self {
        self.with_text(String::new())
    }

    pub fn of(minor: i64, fraction_digits: u32) -> Self {
        if minor <= 0 {
            return AmountInput::empty(fraction_digits);
        }
        let scale = pow10(fraction_digits);
        let (whole, frac) = (minor / scale, minor % scale);
        let text = if fraction_digits == 0 || frac == 0 {
            whole.to_string()
        } else {
            let padded = format!("{frac:0width$}", width = fraction_digits as usize);
            format!("{whole}.{}", padded.trim_end_matches('0'))
        };
        AmountInput::new(&text, fraction_digits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_digits_and_a_decimal() {
        let a = AmountInput::default().digit(1).digit(2).decimal().digit(5);
        assert_eq!(a.text, "12.5");
        assert_eq!(a.minor(), 1_250);
    }

    #[test]
    fn decimals_stop_at_the_currencys_digits() {
        let a = AmountInput::default().digit(1).decimal().digit(2).digit(3).digit(4);
        assert_eq!(a.text, "1.23");
    }

    #[test]
    fn no_leading_zeros_and_a_bare_decimal_gets_a_zero() {
        assert_eq!(AmountInput::default().digit(0).digit(5).text, "5");
        assert_eq!(AmountInput::default().decimal().text, "0.");
    }

    #[test]
    fn zero_decimal_currencies_ignore_the_decimal_key() {
        assert_eq!(AmountInput::empty(0).digit(1).digit(2).decimal().text, "12");
    }

    #[test]
    fn integer_digits_are_capped() {
        let mut a = AmountInput::default();
        for _ in 0..20 {
            a = a.digit(9);
        }
        assert_eq!(a.text.len(), AmountInput::MAX_INTEGER_DIGITS);
    }

    #[test]
    fn backspace_and_clear() {
        let a = AmountInput::default().digit(4).digit(2);
        assert_eq!(a.backspace().text, "4");
        assert!(a.clear().is_empty());
    }

    #[test]
    fn of_minor_units_reads_back_the_same() {
        for minor in [1, 50, 1_250, 10_000, 123_456_789] {
            assert_eq!(AmountInput::of(minor, 2).minor(), minor);
        }
        assert_eq!(AmountInput::of(1_250, 2).text, "12.5");
    }
}
