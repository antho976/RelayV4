//! Amounts: reading what people type, rescaling between currencies, and formatting. Port of
//! `Money.kt`.
//!
//! The phone formats with the JDK's `NumberFormat`; there is no ICU here, so [`Locale`] carries
//! the few facts formatting needs (separators, which side the symbol sits on) for the languages
//! the owner uses, English and French, with English as the fallback. Symbols follow CLDR for the
//! owner's own region (`$` for CAD in Canada) and fall back to the ISO code elsewhere.

/// Digits after the decimal point for an ISO 4217 code, or `None` for a code that is not one.
/// The JDK's `Currency.defaultFractionDigits`.
pub fn fraction_digits(code: &str) -> Option<u32> {
    if ZERO_DIGITS.contains(&code) {
        Some(0)
    } else if THREE_DIGITS.contains(&code) {
        Some(3)
    } else if FOUR_DIGITS.contains(&code) {
        Some(4)
    } else if TWO_DIGITS.contains(&code) {
        Some(2)
    } else {
        None
    }
}

const ZERO_DIGITS: &[&str] = &[
    "BIF", "BYR", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG", "RWF", "UGX", "UYI",
    "VND", "VUV", "XAF", "XOF", "XPF",
];
const THREE_DIGITS: &[&str] = &["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"];
const FOUR_DIGITS: &[&str] = &["CLF", "UYW"];
const TWO_DIGITS: &[&str] = &[
    "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
    "BGN", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD", "CDF", "CHF",
    "CNY", "COP", "CRC", "CUP", "CVE", "CZK", "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR",
    "FJD", "FKP", "GBP", "GEL", "GHS", "GIP", "GMD", "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF",
    "IDR", "ILS", "INR", "IRR", "JMD", "KES", "KGS", "KHR", "KPW", "KYD", "KZT", "LAK", "LBP",
    "LKR", "LRD", "LSL", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP", "MRU", "MUR", "MVR",
    "MWK", "MXN", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR", "NZD", "PAB", "PEN", "PGK",
    "PHP", "PKR", "PLN", "QAR", "RON", "RSD", "RUB", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD",
    "SHP", "SLE", "SOS", "SRD", "SSP", "STN", "SVC", "SYP", "SZL", "THB", "TJS", "TMT", "TOP",
    "TRY", "TTD", "TWD", "TZS", "UAH", "USD", "UYU", "UZS", "VES", "WST", "XCD", "YER", "ZAR",
    "ZMW", "ZWL",
];

pub fn pow10(n: u32) -> i64 {
    10i64.pow(n)
}

/// A minus (hyphen, true minus, or the en dash some bank statements print) or an opening
/// bracket: the text says the amount is negative.
pub fn has_sign(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '-' | '\u{2212}' | '\u{2013}' | '('))
}

/// Locale-tolerant reading of an amount's MAGNITUDE: every character but digits and the two
/// separators is ignored, a sign included. Callers that care about the sign read it themselves:
/// [`MoneyFormatter::parse`] refuses one, and the CSV reader turns it into a type.
pub fn parse_amount(text: &str, fraction_digits: u32) -> Option<i64> {
    let cleaned: Vec<char> = text.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    if !cleaned.iter().any(char::is_ascii_digit) {
        return None;
    }
    // The LAST separator is the decimal one when it is followed by 1..fraction_digits digits;
    // every other separator is grouping. "1,234.56" and "1.234,56" both read as 1234.56, and
    // "1,234" (three digits after) reads as one thousand two hundred thirty-four.
    let fd = fraction_digits as usize;
    let (int_part, frac_part): (&[char], &[char]) = match cleaned.iter().rposition(|c| *c == '.' || *c == ',') {
        Some(last) => {
            let tail = &cleaned[last + 1..];
            if fd > 0 && (1..=fd).contains(&tail.len()) {
                (&cleaned[..last], tail)
            } else if fd > 0 && tail.is_empty() {
                (&cleaned[..last], &[])
            } else {
                (&cleaned[..], &[])
            }
        }
        None => (&cleaned[..], &[]),
    };
    let mut digits: String = int_part.iter().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        digits.push('0');
    }
    if digits.len() > 13 {
        return None;
    }
    let mut frac: String = frac_part.iter().take(fd).collect();
    while frac.len() < fd {
        frac.push('0');
    }
    format!("{digits}{frac}").parse().ok()
}

/// `n / d` rounded half-even. `d` is positive.
fn div_half_even(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    let twice = 2 * r.abs();
    if twice > d || (twice == d && q % 2 != 0) {
        q + n.signum()
    } else {
        q
    }
}

/// `minor` written with `from_digits` decimals, rewritten with `to_digits`: the same figure in a
/// currency with a different number of decimals. Gaining digits is exact (12 yen becomes 1200
/// cents); losing them rounds half-even (1250 cents becomes 12 yen, 1251 becomes 13), and a
/// non-zero amount never rounds to zero, so no entry is left without an amount. `None` when the
/// result does not fit in an `i64` (the Kotlin throws `ArithmeticException`).
pub fn rescale(minor: i64, from_digits: u32, to_digits: u32) -> Option<i64> {
    if to_digits >= from_digits {
        return minor.checked_mul(pow10(to_digits - from_digits));
    }
    let rounded = div_half_even(minor as i128, pow10(from_digits - to_digits) as i128) as i64;
    Some(if rounded == 0 && minor != 0 { minor.signum() } else { rounded })
}

/// True when [`rescale`] cannot keep `minor` exactly, because it drops digits `minor` uses.
pub fn rescale_rounds(minor: i64, from_digits: u32, to_digits: u32) -> bool {
    to_digits < from_digits && minor % pow10(from_digits - to_digits) != 0
}

/// The facts formatting needs about a locale. Built from a tag such as `en-CA`, `fr_CA.UTF-8`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    pub language: String,
    pub country: String,
}

impl Locale {
    pub fn new(tag: &str) -> Self {
        let tag = tag.split(['.', '@']).next().unwrap_or("");
        let mut parts = tag.split(['-', '_']);
        let language = parts.next().unwrap_or("").to_ascii_lowercase();
        let country = parts.next().unwrap_or("").to_ascii_uppercase();
        Locale { language, country }
    }

    /// The process's locale, from `LC_ALL`, `LC_MONETARY` or `LANG`; `en-CA` when none is set.
    pub fn from_env() -> Self {
        ["LC_ALL", "LC_MONETARY", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
            .map_or_else(|| Locale::new("en-CA"), |v| Locale::new(&v))
    }

    fn french(&self) -> bool {
        self.language == "fr"
    }

    pub fn decimal_separator(&self) -> char {
        if self.french() { ',' } else { '.' }
    }

    fn grouping_separator(&self) -> char {
        if self.french() { '\u{a0}' } else { ',' }
    }
}

/// Formats and reads amounts in one currency for one locale.
#[derive(Debug, Clone)]
pub struct MoneyFormatter {
    pub currency: String,
    pub locale: Locale,
    /// Digits after the decimal point for this currency: 2 for CAD, 0 for JPY.
    pub fraction_digits: u32,
    pub symbol: String,
    scale: i64,
}

impl MoneyFormatter {
    /// An unknown currency code falls back to CAD instead of failing.
    pub fn new(currency_code: &str, locale: Locale) -> Self {
        let (currency, fraction_digits) = match fraction_digits(currency_code) {
            Some(d) => (currency_code.to_string(), d),
            None => ("CAD".to_string(), 2),
        };
        let symbol = symbol(&currency, &locale);
        MoneyFormatter { scale: pow10(fraction_digits), currency, locale, fraction_digits, symbol }
    }

    /// "$1,284.50" (en-CA), "1 284,50 $" (fr-CA). Negative amounts take a true minus sign.
    pub fn format(&self, minor: i64) -> String {
        let abs = minor.unsigned_abs() as i128;
        let scale = self.scale as i128;
        let body = self.number((abs / scale) as u128, Some(((abs % scale) as u128, self.fraction_digits)));
        self.signed(minor, &self.dress(&body))
    }

    /// "$1,285": whole units, for figures where cents are noise. Rounds half-even so a column of
    /// rounded figures does not drift one way.
    pub fn format_whole(&self, minor: i64) -> String {
        let whole = div_half_even(minor.unsigned_abs() as i128, self.scale as i128) as u128;
        self.signed(minor, &self.dress(&self.number(whole, None)))
    }

    /// "$12.4k" at 10,000 and above, whole units below.
    pub fn format_compact(&self, minor: i64) -> String {
        let abs = minor.unsigned_abs() as i128;
        let scale = self.scale as i128;
        if abs < 10_000 * scale {
            return self.format_whole(minor);
        }
        let (per_tenth, unit) = if abs >= 1_000_000 * scale { (100_000 * scale, "M") } else { (100 * scale, "k") };
        let tenths = div_half_even(abs, per_tenth) as u128;
        let frac = (!tenths.is_multiple_of(10)).then_some((tenths % 10, 1));
        let body = format!("{}{unit}", self.number(tenths / 10, frac));
        self.signed(minor, &self.dress(&body))
    }

    /// Income reads "+$1,200.00"; an expense is just the amount.
    pub fn format_signed(&self, minor: i64) -> String {
        if minor > 0 { format!("+{}", self.format(minor)) } else { self.format(minor) }
    }

    /// The bare number for an editable field: "12.50", no symbol, no grouping.
    pub fn format_input(&self, minor: i64) -> String {
        let abs = minor.unsigned_abs();
        if self.fraction_digits == 0 {
            return abs.to_string();
        }
        let scale = self.scale as u64;
        format!("{}.{:0width$}", abs / scale, abs % scale, width = self.fraction_digits as usize)
    }

    /// Parses what a person types into an amount field: "12", "12.5", "12,50", "$1,234.56",
    /// "1 234,56 $". `None` for anything that is not a non-negative amount, a minus sign or
    /// accounting brackets included: "-120.50" is refused, never read as 120.50.
    pub fn parse(&self, text: &str) -> Option<i64> {
        if has_sign(text) { None } else { parse_amount(text, self.fraction_digits) }
    }

    /// Whole digits grouped by three, then an optional `(value, digits)` fraction.
    fn number(&self, whole: u128, fraction: Option<(u128, u32)>) -> String {
        let digits = whole.to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(self.locale.grouping_separator());
            }
            out.push(c);
        }
        if let Some((value, width)) = fraction.filter(|(_, w)| *w > 0) {
            out.push(self.locale.decimal_separator());
            out.push_str(&format!("{value:0width$}", width = width as usize));
        }
        out
    }

    fn dress(&self, number: &str) -> String {
        if self.locale.french() {
            format!("{number}\u{a0}{}", self.symbol)
        } else {
            format!("{}{number}", self.symbol)
        }
    }

    fn signed(&self, minor: i64, body: &str) -> String {
        if minor < 0 { format!("\u{2212}{body}") } else { body.to_string() }
    }
}

/// The currency's symbol as CLDR writes it for the owner's languages: the bare local symbol for a
/// region's own dollar, a qualified one for a foreign dollar, the code for anything unlisted.
fn symbol(code: &str, locale: &Locale) -> String {
    let fr = locale.french();
    let home = match locale.country.as_str() {
        "CA" => "CAD",
        "US" => "USD",
        "AU" => "AUD",
        "NZ" => "NZD",
        "MX" => "MXN",
        _ => "",
    };
    let s = match code {
        c if c == home => "$",
        "CAD" if fr => "$\u{a0}CA",
        "CAD" => "CA$",
        "USD" if fr => "$\u{a0}US",
        "USD" => "US$",
        "AUD" if fr => "$\u{a0}AU",
        "AUD" => "A$",
        "NZD" if fr => "$\u{a0}NZ",
        "NZD" => "NZ$",
        "MXN" if fr => "$\u{a0}MX",
        "MXN" => "MX$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" if fr => "¥",
        "JPY" => "JP¥",
        "INR" => "₹",
        "CNY" if fr => "CNY",
        "CNY" => "CN¥",
        "BRL" => "R$",
        other => other,
    };
    s.to_string()
}

// Amounts in tests are written in cents as the Kotlin tests write them: 99_999_99 is $99,999.99.
#[cfg(test)]
#[allow(clippy::inconsistent_digit_grouping)]
mod tests {
    use super::*;

    fn en_ca() -> MoneyFormatter {
        MoneyFormatter::new("CAD", Locale::new("en-CA"))
    }
    fn fr_ca() -> MoneyFormatter {
        MoneyFormatter::new("CAD", Locale::new("fr-CA"))
    }
    fn jpy() -> MoneyFormatter {
        MoneyFormatter::new("JPY", Locale::new("ja-JP"))
    }

    #[test]
    fn formats_cents_in_english_canada() {
        assert_eq!(en_ca().format(128_450), "$1,284.50");
        assert_eq!(en_ca().format(5), "$0.05");
    }

    #[test]
    fn french_canada_puts_the_symbol_after_the_number() {
        let s = fr_ca().format(128_450);
        assert!(s.ends_with('$'), "{s}");
        assert!(s.contains("284,50"), "{s}");
    }

    #[test]
    fn negative_amounts_use_a_true_minus_sign() {
        assert_eq!(en_ca().format(-1_200), "\u{2212}$12.00");
    }

    #[test]
    fn whole_formatting_rounds_half_even() {
        assert_eq!(en_ca().format_whole(1_250), "$12");
        assert_eq!(en_ca().format_whole(1_350), "$14");
    }

    #[test]
    fn compact_switches_to_k_at_ten_thousand() {
        assert_eq!(en_ca().format_compact(999_900), "$9,999");
        assert_eq!(en_ca().format_compact(1_243_100), "$12.4k");
        assert_eq!(en_ca().format_compact(150_000_000), "$1.5M");
    }

    #[test]
    fn compact_keeps_the_french_symbol_on_its_side() {
        let s = fr_ca().format_compact(1_243_100);
        assert!(s.contains("12,4k"), "{s}");
        assert!(s.ends_with('$'), "{s}");
    }

    #[test]
    fn zero_decimal_currencies_have_no_cents() {
        assert_eq!(jpy().fraction_digits, 0);
        assert_eq!(jpy().parse("1500"), Some(1500));
    }

    #[test]
    fn parses_what_people_type() {
        let f = en_ca();
        assert_eq!(f.parse("12.5"), Some(1_250));
        assert_eq!(f.parse("12,50"), Some(1_250));
        assert_eq!(f.parse("$1,234.56"), Some(123_456));
        assert_eq!(f.parse("1 234,56 $"), Some(123_456));
        assert_eq!(f.parse("1,234"), Some(123_400));
        assert_eq!(f.parse("12."), Some(1_200));
        assert_eq!(f.parse(""), None);
        assert_eq!(f.parse("abc"), None);
    }

    #[test]
    fn a_typed_sign_is_refused_never_dropped() {
        assert_eq!(en_ca().parse("-120.50"), None, "an overdrawn balance must not save as a positive one");
        assert_eq!(en_ca().parse("\u{2212}120.50"), None);
        assert_eq!(en_ca().parse("(120.50)"), None);
        assert_eq!(en_ca().parse("$-5"), None);
        assert_eq!(fr_ca().parse("-1 234,56 $"), None);
        assert_eq!(en_ca().parse("+12"), Some(1_200), "a plus sign is still a non-negative amount");
    }

    #[test]
    fn the_magnitude_reader_ignores_the_sign_for_callers_that_read_it_themselves() {
        assert_eq!(parse_amount("-120.50", 2), Some(12_050));
        assert!(has_sign("4.25-"));
        assert!(!has_sign("$4.25"));
    }

    #[test]
    fn rescaling_to_more_decimals_is_exact() {
        assert_eq!(rescale(1_250, 0, 2), Some(125_000));
        assert_eq!(rescale(1_250, 2, 3), Some(12_500));
        assert_eq!(rescale(-500, 0, 2), Some(-50_000));
        assert!(!rescale_rounds(1_250, 0, 2));
    }

    #[test]
    fn rescaling_to_fewer_decimals_keeps_the_figure_rounding_half_even() {
        assert_eq!(rescale(1_250, 2, 0), Some(12), "$12.50 reads 12 yen, not 1,250");
        assert_eq!(rescale(1_350, 2, 0), Some(14));
        assert_eq!(rescale(1_251, 2, 0), Some(13));
        assert_eq!(rescale(-1_250, 2, 0), Some(-12));
        assert_eq!(rescale(-1_260, 2, 0), Some(-13));
        assert_eq!(rescale(12_345, 3, 2), Some(1_234));
        assert_eq!(rescale(12_355, 3, 2), Some(1_236));
        assert_eq!(rescale(0, 2, 0), Some(0));
    }

    #[test]
    fn a_non_zero_amount_never_rescales_to_zero() {
        assert_eq!(rescale(40, 2, 0), Some(1));
        assert_eq!(rescale(-40, 2, 0), Some(-1));
    }

    #[test]
    fn rescale_says_when_it_rounds() {
        assert!(rescale_rounds(1_250, 2, 0));
        assert!(!rescale_rounds(1_200, 2, 0));
        assert!(rescale_rounds(-1_201, 2, 0));
        assert!(!rescale_rounds(0, 2, 0));
    }

    #[test]
    fn rescaling_past_an_i64_fails_instead_of_wrapping() {
        assert_eq!(rescale(i64::MAX / 10, 0, 2), None);
    }

    #[test]
    fn input_formatting_round_trips_through_parse() {
        let f = en_ca();
        for minor in [0, 5, 1_250, 99_999_99] {
            assert_eq!(f.parse(&f.format_input(minor)), Some(minor));
        }
    }

    #[test]
    fn signed_formatting_marks_income() {
        assert_eq!(en_ca().format_signed(120_000), "+$1,200.00");
        assert_eq!(en_ca().format_signed(0), "$0.00");
        assert_eq!(en_ca().format_signed(-500), "\u{2212}$5.00");
    }

    #[test]
    fn unknown_currency_codes_fall_back_to_cad_instead_of_crashing() {
        assert_eq!(MoneyFormatter::new("NOPE", Locale::new("en-CA")).currency, "CAD");
    }

    #[test]
    fn locales_read_from_posix_tags() {
        assert_eq!(Locale::new("fr_CA.UTF-8"), Locale { language: "fr".into(), country: "CA".into() });
        assert_eq!(MoneyFormatter::new("USD", Locale::new("en-CA")).format(100), "US$1.00");
    }
}
