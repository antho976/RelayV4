//! Bank statements as their banks export them. Port of `BankStatements.kt`.
//!
//! No bank offers an open API to an offline app, so the CSV the owner downloads is the bridge:
//! Desjardins AccèsD (with French or English headers, or its header-less positional layout),
//! Wealthsimple's monthly statements, and any other bank that writes a date and an amount (or
//! money out and money in) per line.
//!
//! The Kotlin's patterns are Java regexes, where `\d`, `\s` and `.` are narrower than Rust's
//! defaults; the patterns here spell out the Java classes (`[0-9]`, `[ \t\n\x0B\f\r]`, and `.`
//! without the line terminators) so both read the same text the same way.

use crate::csv::{kt_is_blank, kt_trim, parse_records, BOM};
use crate::money::{has_sign, parse_amount};
use jiff::civil::Date;
use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// Where a statement came from, as the reader recognised it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementFormat {
    Desjardins,
    Wealthsimple,
    Bank,
}

impl StatementFormat {
    pub fn label(self) -> &'static str {
        match self {
            StatementFormat::Desjardins => "Desjardins AccèsD",
            StatementFormat::Wealthsimple => "Wealthsimple",
            StatementFormat::Bank => "Bank CSV",
        }
    }
}

/// One line of a bank statement. `amount` is signed as the bank wrote it: below zero is money out
/// of the account, above zero money in. `account` names the account the line belongs to when the
/// file holds several (a Desjardins export can), else `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementRow {
    pub date: Date,
    pub description: String,
    pub amount: i64,
    pub balance: Option<i64>,
    pub account: Option<String>,
    /// The bank's own transaction code when it writes one (Wealthsimple's "SPEND", "AFT_IN").
    pub code: Option<String>,
}

/// A statement read whole: its lines in file order, and the lines that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub format: StatementFormat,
    pub rows: Vec<StatementRow>,
    pub errors: Vec<String>,
}

impl Statement {
    /// The accounts the file names, in the order they first appear; empty when it names none.
    pub fn accounts(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for a in self.rows.iter().filter_map(|r| r.account.as_ref()) {
            if !out.contains(a) {
                out.push(a.clone());
            }
        }
        out
    }

    pub fn first(&self) -> Option<Date> {
        self.rows.iter().map(|r| r.date).min()
    }

    pub fn last(&self) -> Option<Date> {
        self.rows.iter().map(|r| r.date).max()
    }

    /// The share of lines above zero. A card statement that writes purchases as positive reads mostly so.
    pub fn positive_share(&self) -> f32 {
        if self.rows.is_empty() {
            0.0
        } else {
            self.rows.iter().filter(|r| r.amount > 0).count() as f32 / self.rows.len() as f32
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatementRead {
    Ok(Statement),
    /// A file in Tally's own export layout: it names its accounts and types, so the plain CSV import reads it.
    TallyCsv,
    /// The reason is shown as-is, so it names the problem and not the parser.
    Invalid(String),
}

/// Reads `text`. `day_first` settles a numeric date like 03/04/2026 that the file itself leaves
/// ambiguous (no day above 12 anywhere in it): true reads 3 April, false 4 March. The Kotlin's
/// default is `true`.
pub fn read(text: &str, fraction_digits: u32, day_first: bool) -> StatementRead {
    let body = text.strip_prefix(BOM).unwrap_or(text);
    if kt_is_blank(body) {
        return StatementRead::Invalid("The file is empty.".into());
    }
    let records: Vec<Vec<String>> = parse_records(body, delimiter_of(body))
        .into_iter()
        .map(|r| r.iter().map(|f| kt_trim(f).to_string()).collect())
        .collect();
    if records.is_empty() {
        return StatementRead::Invalid("The file is empty.".into());
    }
    if let Some(header_at) = records.iter().take(HEADER_SCAN).position(|r| is_header(r)) {
        let header: Vec<String> = records[header_at].iter().map(|h| normalize(h)).collect();
        if is_tally_header(&header) {
            return StatementRead::TallyCsv;
        }
        return finish(with_header(&header, &records[header_at + 1..], header_at + 2, fraction_digits, day_first));
    }
    if records.iter().any(|r| r.iter().any(|f| DESJARDINS_DATE.is_match(f))) {
        return finish(desjardins_positional(&records, fraction_digits));
    }
    finish(headerless(&records, fraction_digits, day_first))
}

fn finish(statement: Statement) -> StatementRead {
    if statement.rows.is_empty() {
        StatementRead::Invalid(statement.errors.into_iter().next().unwrap_or_else(|| "No transactions were found in that file.".into()))
    } else {
        StatementRead::Ok(statement)
    }
}

/// Java's windows-1252 for 0x80..=0x9F; the five bytes it leaves undefined read U+FFFD.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}',
    '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
];

/// The file's bytes as text. UTF-8 when they are UTF-8; a UTF-16 file says so with its byte
/// order mark; anything else is read as Windows-1252, which is what an older bank export (or a
/// French Excel save) writes "Épicerie" in.
pub fn decode_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && matches!((bytes[0], bytes[1]), (0xFF, 0xFE) | (0xFE, 0xFF)) {
        // Java's UTF-16 decoder: the mark picks the byte order and is dropped; a stray odd byte
        // or an unpaired surrogate reads U+FFFD.
        let le = bytes[0] == 0xFF;
        let units = bytes[2..].as_chunks::<2>().0.iter().map(|&p| if le { u16::from_le_bytes(p) } else { u16::from_be_bytes(p) });
        let mut s: String = char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect();
        if bytes.len() % 2 == 1 {
            s.push('\u{FFFD}');
        }
        return s;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| if (0x80..0xA0).contains(&b) { CP1252_HIGH[(b - 0x80) as usize] } else { b as char }).collect(),
    }
}

/// The field separator the first lines agree on: a tab, a semicolon, else a comma.
pub fn delimiter_of(text: &str) -> char {
    // `lineSequence`: lines end at \r\n, \n or \r.
    let lines: Vec<&str> = text.split("\r\n").flat_map(|l| l.split(['\n', '\r'])).filter(|l| !kt_is_blank(l)).take(8).collect();
    if lines.is_empty() {
        return ',';
    }
    let steady = |c: char| lines.iter().map(|l| count_outside_quotes(l, c)).min().unwrap_or(0);
    // The first of equals wins, as `maxByOrNull`.
    let mut best = '\t';
    let mut best_score = steady('\t') * 10;
    for c in [';', ','] {
        let score = steady(c) * 10 + usize::from(c == ',');
        if score > best_score {
            best = c;
            best_score = score;
        }
    }
    if steady(best) > 0 { best } else { ',' }
}

fn count_outside_quotes(line: &str, c: char) -> usize {
    let mut quoted = false;
    let mut n = 0;
    for ch in line.chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch == c && !quoted {
            n += 1;
        }
    }
    n
}

// ── Headers ──────────────────────────────────────────────────────────────

/// How many leading lines may come before the header (an account name, a period line).
const HEADER_SCAN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Date,
    Posted,
    Description,
    Amount,
    Debit,
    Credit,
    Balance,
    Code,
    Account,
}

static MN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{Mn}+").unwrap());
static BRACKETED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\([^\n\r\x{85}\x{2028}\x{2029}]*?\)").unwrap());
static NOT_ALNUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").unwrap());

/// Lowercase, no accents, no brackets or what is in them, punctuation as spaces: "AMOUNT (CAD)" reads "amount".
pub fn normalize(text: &str) -> String {
    let nfd: String = text.nfd().collect();
    let lower = MN.replace_all(&nfd, "").to_lowercase();
    let unbracketed = BRACKETED.replace_all(&lower, " ");
    kt_trim(&NOT_ALNUM.replace_all(&unbracketed, " ")).to_string()
}

const DATE_NAMES: &[&str] = &[
    "date", "transaction date", "date de transaction", "date de la transaction", "date de l operation",
    "date operation", "date d operation", "trans date", "date transaction", "jour",
];
const POSTED_NAMES: &[&str] = &[
    "posted date", "post date", "posting date", "date posted", "date d inscription", "date inscription",
    "date de publication", "settlement date", "date de valeur", "value date",
];
const DESCRIPTION_NAMES: &[&str] = &[
    "description", "details", "detail", "merchant", "merchant name", "payee", "name", "memo", "libelle",
    "narrative", "transaction description", "description 1", "nom", "commercant", "beneficiaire", "note",
];
const DEBIT_NAMES: &[&str] = &[
    "debit", "debits", "withdrawal", "withdrawals", "retrait", "retraits", "money out", "debit amount",
    "sortie", "sorties", "paid out", "out", "montant debit", "achat", "achats",
];
const CREDIT_NAMES: &[&str] = &[
    "credit", "credits", "deposit", "deposits", "depot", "depots", "money in", "credit amount", "entree",
    "entrees", "paid in", "in", "montant credit", "paiement", "paiements",
];
const CODE_NAMES: &[&str] = &["transaction", "type", "code", "transaction type", "type de transaction", "activity type"];
const ACCOUNT_NAMES: &[&str] = &["account", "compte", "account number", "numero de compte", "account name"];

fn role(h: &str) -> Option<Role> {
    Some(if DATE_NAMES.contains(&h) {
        Role::Date
    } else if POSTED_NAMES.contains(&h) {
        Role::Posted
    } else if DESCRIPTION_NAMES.contains(&h) {
        Role::Description
    } else if h == "amount" || h.starts_with("amount ") || h == "montant" || h.starts_with("montant ") {
        Role::Amount
    } else if DEBIT_NAMES.contains(&h) {
        Role::Debit
    } else if CREDIT_NAMES.contains(&h) {
        Role::Credit
    } else if h.starts_with("balance") || h.starts_with("solde") {
        Role::Balance
    } else if CODE_NAMES.contains(&h) {
        Role::Code
    } else if ACCOUNT_NAMES.contains(&h) {
        Role::Account
    } else {
        return None;
    })
}

fn is_header(record: &[String]) -> bool {
    let roles: Vec<Option<Role>> = record.iter().map(|f| role(&normalize(f))).collect();
    let has = |r: Role| roles.contains(&Some(r));
    let dated = has(Role::Date) || has(Role::Posted);
    let money = has(Role::Amount) || has(Role::Debit) || has(Role::Credit);
    dated && money
}

fn is_tally_header(header: &[String]) -> bool {
    ["type", "account", "amount", "to account"].iter().all(|n| header.iter().any(|h| h == n))
}

fn with_header(header: &[String], lines: &[Vec<String>], first_line: usize, fraction_digits: u32, day_first: bool) -> Statement {
    let roles: Vec<Option<Role>> = header.iter().map(|h| role(h)).collect();
    let col = |r: Role| roles.iter().position(|x| *x == Some(r));
    let date_col = col(Role::Date).or_else(|| col(Role::Posted));
    let desc_col = col(Role::Description).or_else(|| col(Role::Code));
    let code_col = if col(Role::Description).is_some() { col(Role::Code) } else { None };
    let amount_col = col(Role::Amount);
    let debit_col = col(Role::Debit);
    let credit_col = col(Role::Credit);
    let balance_col = col(Role::Balance);
    let account_col = col(Role::Account);
    let wealthsimple = (header.iter().any(|h| h == "posted date") && amount_col.is_some() && balance_col.is_some())
        || (code_col.is_some_and(|c| header[c] == "transaction") && header.len() <= 6);
    let format = if wealthsimple {
        StatementFormat::Wealthsimple
    } else if header.iter().any(|h| h == "montant" || h == "retrait" || h == "depot" || h == "solde") {
        StatementFormat::Desjardins
    } else {
        StatementFormat::Bank
    };
    let samples: Vec<&str> = lines.iter().filter_map(|l| date_col.and_then(|c| l.get(c)).map(String::as_str)).collect();
    let order = statement_dates::order(&samples).unwrap_or(day_first);
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for (i, r) in lines.iter().enumerate() {
        let at = |c: Option<usize>| c.and_then(|c| r.get(c)).map_or("", String::as_str);
        let amount = if amount_col.is_some() {
            signed_amount(at(amount_col), fraction_digits)
        } else {
            let out = magnitude(at(debit_col), fraction_digits).unwrap_or(0);
            let inn = magnitude(at(credit_col), fraction_digits).unwrap_or(0);
            if out == 0 && inn == 0 { None } else { Some(inn - out) }
        };
        let raw_date = at(date_col);
        let date = statement_dates::parse(raw_date, order);
        match (date, amount) {
            (None, None) => {} // a blank line, a total, a footnote
            (None, _) => errors.push(format!("Line {}: the date \"{raw_date}\" could not be read.", first_line + i)),
            (_, None | Some(0)) => {} // a zero line moves nothing
            (Some(date), Some(amount)) => {
                let desc = at(desc_col);
                rows.push(StatementRow {
                    date,
                    description: if kt_is_blank(desc) { at(code_col) } else { desc }.to_string(),
                    amount,
                    balance: if balance_col.is_some() { signed_amount(at(balance_col), fraction_digits) } else { None },
                    account: Some(at(account_col)).filter(|s| !kt_is_blank(s)).map(str::to_string),
                    code: Some(at(code_col)).filter(|s| !kt_is_blank(s)).map(str::to_string),
                });
            }
        }
    }
    Statement { format, rows, errors }
}

// ── Desjardins, header-less ─────────────────────────────────────────────

static DESJARDINS_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{4}/[0-9]{2}/[0-9]{2}$").unwrap());

/// AccèsD's own CSV has no header: caisse, folio, account type, date (YYYY/MM/DD), sequence,
/// description, cheque number, withdrawal, deposit, interest, capital paid, advance, repayment
/// and balance, in that order. Read relative to the date, so a leading column more or less
/// (an export of one account only) still lines up. Withdrawals and advances are money out;
/// deposits and repayments money in.
fn desjardins_positional(records: &[Vec<String>], fraction_digits: u32) -> Statement {
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for (i, r) in records.iter().enumerate() {
        let Some(d) = r.iter().position(|f| DESJARDINS_DATE.is_match(f)) else { continue };
        let Some(date) = statement_dates::parse(&r[d], false) else {
            errors.push(format!("Line {}: the date \"{}\" could not be read.", i + 1, r[d]));
            continue;
        };
        let num = |offset: usize| r.get(d + offset).and_then(|f| magnitude(f, fraction_digits)).unwrap_or(0);
        let out = num(4) + num(8);
        let inn = num(5) + num(9);
        let amount = if out == 0 && inn == 0 { num(6) } else { inn - out };
        if amount == 0 {
            continue;
        }
        let before = |back: usize| d.checked_sub(back).map(|c| r[c].as_str());
        let account = [before(1), before(2)].into_iter().flatten().filter(|s| !kt_is_blank(s)).collect::<Vec<_>>().join(" ");
        rows.push(StatementRow {
            date,
            description: r.get(d + 2).cloned().unwrap_or_default(),
            amount,
            balance: r.get(d + 10).and_then(|f| signed_amount(f, fraction_digits)),
            account: Some(account).filter(|s| !kt_is_blank(s)),
            code: None,
        });
    }
    Statement { format: StatementFormat::Desjardins, rows, errors }
}

// ── Anything else without a header ───────────────────────────────────────

/// A file with no header at all: per line, the first cell that reads as a date, the first text
/// after it as the description, then either money out and money in (one of the two blank) or a
/// signed amount, and a balance after them when there is one.
fn headerless(records: &[Vec<String>], fraction_digits: u32, day_first: bool) -> Statement {
    let samples: Vec<&str> = records.iter().filter_map(|r| r.iter().find(|f| statement_dates::looks_like_date(f)).map(String::as_str)).collect();
    let order = statement_dates::order(&samples).unwrap_or(day_first);
    let mut rows = Vec::new();
    for r in records {
        let Some(d) = r.iter().position(|f| statement_dates::looks_like_date(f)) else { continue };
        let Some(date) = statement_dates::parse(&r[d], order) else { continue };
        let Some(k) = (d + 1..r.len()).find(|&c| !kt_is_blank(&r[c]) && magnitude(&r[c], fraction_digits).is_none()) else { continue };
        let a = r.get(k + 1).map_or("", String::as_str);
        let b = r.get(k + 2).map_or("", String::as_str);
        let split = r.len() >= k + 3 && (kt_is_blank(a) != kt_is_blank(b));
        let amount = if split {
            magnitude(b, fraction_digits).unwrap_or(0) - magnitude(a, fraction_digits).unwrap_or(0)
        } else {
            let Some(v) = signed_amount(a, fraction_digits) else { continue };
            v
        };
        if amount == 0 {
            continue;
        }
        let balance = if split { r.get(k + 3).map(String::as_str) } else { Some(b).filter(|s| !kt_is_blank(s)) };
        rows.push(StatementRow {
            date,
            description: r[k].clone(),
            amount,
            balance: balance.and_then(|s| signed_amount(s, fraction_digits)),
            account: None,
            code: None,
        });
    }
    Statement { format: StatementFormat::Bank, rows, errors: vec![] }
}

// ── Amounts ──────────────────────────────────────────────────────────────

// `\b` is Unicode here, as in the JDKs before 19 and on Android (ICU); the letters are ASCII, so
// case folding agrees with Java's ASCII-only `(?i)`.
static CREDIT_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(cr|credit)\b").unwrap());
static DEBIT_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(dr|debit)\b").unwrap());

/// An amount's size, whatever its sign or symbol; `None` when the cell holds no number.
pub(crate) fn magnitude(text: &str, fraction_digits: u32) -> Option<i64> {
    // The Kotlin first asks `isDigit` (any Unicode decimal digit); `parse_amount` reads ASCII
    // digits only and says `None` without one, so the answer is the same.
    if !text.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    parse_amount(text, fraction_digits)
}

/// An amount with its sign: a minus (any of the dashes a statement prints), brackets or a "DR"
/// read as money out; "CR" as money in, the way card statements mark a payment or a refund.
pub(crate) fn signed_amount(text: &str, fraction_digits: u32) -> Option<i64> {
    let size = magnitude(text, fraction_digits)?;
    let negative = if CREDIT_MARK.is_match(text) {
        false
    } else if DEBIT_MARK.is_match(text) {
        true
    } else {
        has_sign(text)
    };
    Some(if negative { -size } else { size })
}

/// The dates statements write: ISO, numeric either way round, compact, and with month names in
/// English or French.
pub mod statement_dates {
    use super::{normalize, Regex};
    use crate::csv::kt_trim;
    use jiff::civil::Date;
    use std::sync::LazyLock;

    // Java's `.` stops at every line terminator, `\s` is ASCII whitespace and `\d` ASCII digits.
    static ISO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([0-9]{4})[-/.]([0-9]{1,2})[-/.]([0-9]{1,2})(?:$|[ T][^\n\r\x{85}\x{2028}\x{2029}]*)$").unwrap()
    });
    static COMPACT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]{4})([0-9]{2})([0-9]{2})$").unwrap());
    static NUMERIC: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([0-9]{1,2})[-/.]([0-9]{1,2})[-/.]([0-9]{2}|[0-9]{4})(?:$|[ T][^\n\r\x{85}\x{2028}\x{2029}]*)$").unwrap()
    });
    static DAY_MONTH_YEAR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([0-9]{1,2})[ -]([^0-9 \t\n\x0B\x0C\r,.\-]+)\.?,?[ -]([0-9]{4})$").unwrap()
    });
    static MONTH_DAY_YEAR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([^0-9 \t\n\x0B\x0C\r,.\-]+)\.?[ \t\n\x0B\x0C\r]+([0-9]{1,2}),?[ \t\n\x0B\x0C\r]+([0-9]{4})$").unwrap()
    });

    pub fn looks_like_date(text: &str) -> bool {
        parse(text, true).is_some()
    }

    pub fn parse(text: &str, day_first: bool) -> Option<Date> {
        let t = kt_trim(text);
        if t.is_empty() {
            return None;
        }
        if let Some(m) = ISO.captures(t).or_else(|| COMPACT.captures(t)) {
            return date(&m[1], &m[2], &m[3]);
        }
        if let Some(m) = NUMERIC.captures(t) {
            return if day_first { date(&m[3], &m[2], &m[1]) } else { date(&m[3], &m[1], &m[2]) };
        }
        if let Some(m) = DAY_MONTH_YEAR.captures(t) {
            return date(&m[3], &month_of(&m[2])?.to_string(), &m[1]);
        }
        if let Some(m) = MONTH_DAY_YEAR.captures(t) {
            return date(&m[3], &month_of(&m[1])?.to_string(), &m[2]);
        }
        None
    }

    /// Whether a file's numeric dates put the day first: true once any first part is above 12,
    /// false once any second part is, `None` when every date could be either.
    pub fn order(samples: &[&str]) -> Option<bool> {
        let mut day_first = false;
        let mut month_first = false;
        for s in samples {
            let Some(m) = NUMERIC.captures(kt_trim(s)) else { continue };
            let a: u32 = m[1].parse().ok()?;
            let b: u32 = m[2].parse().ok()?;
            day_first |= a > 12;
            month_first |= b > 12;
        }
        match (day_first, month_first) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        }
    }

    fn date(year: &str, month: &str, day: &str) -> Option<Date> {
        let y: i16 = year.parse().ok()?;
        let y = if y < 100 { 2000 + y } else { y };
        Date::new(y, month.parse().ok()?, day.parse().ok()?).ok()
    }

    /// "janv.", "Jan", "février", "Aug" → the month's number.
    fn month_of(word: &str) -> Option<u8> {
        let w = normalize(word).replace(' ', "");
        if w.len() < 3 {
            return None;
        }
        let s = |p: &str| w.starts_with(p);
        Some(match () {
            _ if s("jan") => 1,
            _ if s("feb") || s("fev") => 2,
            _ if s("mar") => 3,
            _ if s("apr") || s("avr") => 4,
            _ if s("may") || s("mai") => 5,
            _ if s("juin") || w == "jun" || s("june") => 6,
            _ if s("juil") || w == "jul" || s("july") => 7,
            _ if s("aug") || s("aou") => 8,
            _ if s("sep") => 9,
            _ if s("oct") => 10,
            _ if s("nov") => 11,
            _ if s("dec") => 12,
            _ => return None,
        })
    }
}

#[cfg(test)]
#[allow(clippy::inconsistent_digit_grouping)]
mod tests {
    use super::*;
    use crate::csv::HEADER;
    use jiff::civil::date;

    fn ok(text: &str, day_first: bool) -> Statement {
        match read(text, 2, day_first) {
            StatementRead::Ok(s) => s,
            other => panic!("{other:?}"),
        }
    }

    fn amounts(s: &Statement) -> Vec<i64> {
        s.rows.iter().map(|r| r.amount).collect()
    }

    // ── Desjardins ───────────────────────────────────────────────────────────

    #[test]
    fn desjardins_positional_export_reads_withdrawals_as_out_and_deposits_as_in() {
        let text = [
            r#""Caisse Desjardins du Plateau","0123456","EOP","2026/09/02",1,"Achat - METRO PLUS #123 MONTREAL QC","",68.42,"","","","","",1931.58"#,
            r#""Caisse Desjardins du Plateau","0123456","EOP","2026/09/03",2,"Depot direct - PAIE EMPLOYEUR INC","","",2150.00,"","","","",4081.58"#,
            r#""Caisse Desjardins du Plateau","0123456","ES1","2026/09/05",1,"Interets","","","",1.25,"","","","",501.25"#,
        ]
        .join("\n");
        let s = ok(&text, true);
        assert_eq!(StatementFormat::Desjardins, s.format);
        assert_eq!(3, s.rows.len());
        let metro = &s.rows[0];
        assert_eq!(date(2026, 9, 2), metro.date);
        assert_eq!(-68_42, metro.amount);
        assert_eq!(Some(1_931_58), metro.balance);
        assert_eq!(Some("EOP 0123456"), metro.account.as_deref());
        assert_eq!(2_150_00, s.rows[1].amount);
        assert_eq!(1_25, s.rows[2].amount, "a line with only interest is money in");
        assert_eq!(vec!["EOP 0123456", "ES1 0123456"], s.accounts());
    }

    #[test]
    fn desjardins_with_french_headers_and_semicolons() {
        let text = "Date;Description;Retrait;Dépôt;Solde\n2026-09-02;Achat IGA;54,20;;1 200,00\n2026-09-03;Virement;;300,00;1 500,00\n";
        let s = ok(text, true);
        assert_eq!(StatementFormat::Desjardins, s.format);
        assert_eq!(vec![-54_20, 300_00], amounts(&s));
        assert_eq!(Some(1_200_00), s.rows[0].balance);
    }

    #[test]
    fn windows_1252_bytes_still_read_their_accents() {
        // Every character here is in Latin-1, where windows-1252 and the code point agree.
        let bytes: Vec<u8> = "Date,Description,Montant\n2026-09-02,Épicerie,-12.00\n".chars().map(|c| c as u8).collect();
        let text = decode_text(&bytes);
        assert!(text.contains("Épicerie"), "{text}");
        let s = ok(&text, true);
        assert_eq!(1, s.rows.len());
        assert_eq!("Épicerie", s.rows[0].description);
    }

    // ── Wealthsimple ─────────────────────────────────────────────────────────

    #[test]
    fn wealthsimple_statement_with_en_dash_minus_and_dollar_signs() {
        let text = "DATE,POSTED DATE,DESCRIPTION,AMOUNT (CAD),BALANCE (CAD)\n".to_string()
            + "2026-08-02,2026-08-03,Deposit,\"$14,500.00\",\"$24,802.33\"\n"
            + "2026-08-03,2026-08-03,Transfer out to Chequing,\"–$1,000.00\",\"$23,802.33\"\n";
        let s = ok(&text, true);
        assert_eq!(StatementFormat::Wealthsimple, s.format);
        assert_eq!(vec![1_450_000, -100_000], amounts(&s));
        assert_eq!(date(2026, 8, 2), s.rows[0].date, "the transaction date, not the posted one");
    }

    #[test]
    fn older_wealthsimple_cash_export_keeps_its_codes() {
        let text = "date,transaction,description,amount,balance\n2026-09-01,SPEND,Metro,-42.10,500.00\n2026-09-02,INT,Interest,0.85,500.85\n";
        let s = ok(text, true);
        assert_eq!(StatementFormat::Wealthsimple, s.format);
        assert_eq!(Some("SPEND"), s.rows[0].code.as_deref());
        assert_eq!("Metro", s.rows[0].description);
        assert_eq!(85, s.rows[1].amount);
    }

    // ── Other banks ──────────────────────────────────────────────────────────

    #[test]
    fn debit_and_credit_columns_with_month_first_dates() {
        let text = "Transaction Date,Description,Debit,Credit\n09/14/2026,STARBUCKS,5.25,\n09/15/2026,PAYROLL,,1000.00\n";
        let s = ok(text, true);
        assert_eq!(date(2026, 9, 14), s.rows[0].date, "a day above 12 in the second place settles month first");
        assert_eq!(vec![-5_25, 1_000_00], amounts(&s));
    }

    #[test]
    fn ambiguous_numeric_dates_follow_the_hint() {
        let text = "Date,Description,Amount\n03/04/2026,Cafe,-3.00\n";
        let one = |day_first| {
            let s = ok(text, day_first);
            assert_eq!(1, s.rows.len());
            s.rows[0].date
        };
        assert_eq!(date(2026, 4, 3), one(true));
        assert_eq!(date(2026, 3, 4), one(false));
    }

    #[test]
    fn a_preamble_before_the_header_is_skipped() {
        let text = "Account,Visa 4510\nPeriod,September\n\nDate,Merchant,Amount\n\"Sep 5, 2026\",Cineplex,-24.50\n5 sept. 2026,Remboursement,12.00 CR\n";
        let s = ok(text, true);
        assert_eq!(date(2026, 9, 5), s.rows[0].date);
        assert_eq!(-24_50, s.rows[0].amount);
        assert_eq!(12_00, s.rows[1].amount, "CR marks money in");
    }

    #[test]
    fn a_header_less_file_with_money_out_and_money_in() {
        let text = "2026-09-02,STM OPUS,3.75,,996.25\n2026-09-03,REFUND,,10.00,1006.25\n";
        let s = ok(text, true);
        assert_eq!(StatementFormat::Bank, s.format);
        assert_eq!(vec![-3_75, 10_00], amounts(&s));
        assert_eq!(Some(1_006_25), s.rows[1].balance);
    }

    #[test]
    fn tab_separated_with_tab_delimiter_detected() {
        let text = "Date\tDescription\tAmount\n2026-09-02\tCafe\t-4.50\n";
        assert_eq!('\t', delimiter_of(text));
        let s = ok(text, true);
        assert_eq!(1, s.rows.len());
        assert_eq!(-4_50, s.rows[0].amount);
    }

    #[test]
    fn tallys_own_export_is_handed_to_the_tally_import() {
        let text = HEADER.join(",") + "\n2026-09-02,expense,4.50,CAD,Dining,Chequing,,Coffee\n";
        assert_eq!(StatementRead::TallyCsv, read(&text, 2, true));
    }

    #[test]
    fn a_file_with_no_transactions_says_so() {
        let read_ = read("hello,world\nfoo,bar\n", 2, true);
        assert!(matches!(read_, StatementRead::Invalid(_)));
        assert!(matches!(read("", 2, true), StatementRead::Invalid(_)));
    }

    #[test]
    fn an_unreadable_date_on_a_money_line_is_reported_a_total_line_is_not() {
        let text = "Date,Description,Amount\nsoon,Thing,-1.00\n,Total,\n2026-09-02,Cafe,-2.00\n";
        let s = ok(text, true);
        assert_eq!(1, s.rows.len());
        assert_eq!(1, s.errors.len());
        assert!(s.errors[0].contains("soon"), "{}", s.errors[0]);
    }

    #[test]
    fn positive_share_reads_a_card_statement() {
        let text = "Date,Description,Amount\n2026-09-02,A,4.00\n2026-09-03,B,6.00\n2026-09-04,Payment,-10.00\n";
        assert!((2f32 / 3f32 - ok(text, true).positive_share()).abs() <= 0.001);
    }

    #[test]
    fn dates_in_every_written_form() {
        let cases = [
            ("2026-09-05", date(2026, 9, 5)),
            ("2026/9/5", date(2026, 9, 5)),
            ("2026-09-05T13:20:00", date(2026, 9, 5)),
            ("20260905", date(2026, 9, 5)),
            ("05-Sep-2026", date(2026, 9, 5)),
            ("5 août 2026", date(2026, 8, 5)),
            ("5 juil. 2026", date(2026, 7, 5)),
            ("June 30, 2026", date(2026, 6, 30)),
            ("31/12/26", date(2026, 12, 31)),
        ];
        for (text, d) in cases {
            assert_eq!(Some(d), statement_dates::parse(text, true), "{text}");
        }
        assert_eq!(None, statement_dates::parse("2026-13-01", true));
        assert_eq!(None, statement_dates::parse("Metro", true));
    }

    #[test]
    fn normalize_drops_accents_brackets_and_punctuation() {
        assert_eq!("amount", normalize("AMOUNT (CAD)"));
        assert_eq!("depot", normalize("Dépôt"));
        assert_eq!("date de l operation", normalize("Date de l'opération"));
    }

    #[test]
    fn utf16_with_a_byte_order_mark_reads() {
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Épi".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!("Épi", decode_text(&le));
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("Épi".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!("Épi", decode_text(&be));
        assert_eq!("€", decode_text(&[0x80]));
    }
}
