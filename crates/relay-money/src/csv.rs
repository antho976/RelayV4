//! Transactions as CSV, both ways. Port of `Csv.kt`.
//!
//! RFC 4180 CSV for transactions. Amounts are written in major units with a dot ("12.50") whatever
//! the locale, because a CSV is read by spreadsheets in every locale and a comma there splits the
//! column.

use crate::model::TxType;
use crate::money::{has_sign, parse_amount, pow10};
use jiff::civil::Date;

/// One exported or imported row, with names instead of ids so a spreadsheet can read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvRow {
    pub date: Date,
    pub r#type: TxType,
    pub amount: i64,
    pub category: Option<String>,
    pub account: String,
    pub to_account: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvImport {
    pub rows: Vec<CsvRow>,
    pub errors: Vec<String>,
}

pub const HEADER: [&str; 8] = ["date", "type", "amount", "currency", "category", "account", "to_account", "note"];

/// The UTF-8 byte order mark. Excel opens a CSV without one in the legacy code page, so
/// "Épicerie" would read "Ã‰picerie"; the reader drops it again.
pub const BOM: &str = "\u{FEFF}";

/// The characters that start a formula in a spreadsheet cell.
const FORMULA_START: &str = "=+-@";

/// Kotlin's `Char.isWhitespace()`: Java's `isWhitespace` or `isSpaceChar`. Unlike Rust's
/// `char::is_whitespace` it counts U+001C..U+001F and not U+0085, so `trim` and `isBlank` agree
/// with the phone's.
pub(crate) fn kt_is_whitespace(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Kotlin's `String.trim()`.
pub(crate) fn kt_trim(s: &str) -> &str {
    s.trim_matches(kt_is_whitespace)
}

/// Kotlin's `String.isBlank()`.
pub(crate) fn kt_is_blank(s: &str) -> bool {
    s.chars().all(kt_is_whitespace)
}

/// `LocalDate.parse`: strictly `YYYY-MM-DD` (a leading minus for a year before 1), and a real date.
/// Years past 9999 are out of `jiff`'s range, so read `None` here where the JDK would read them.
pub fn parse_iso_date(text: &str) -> Option<Date> {
    let (neg, rest) = match text.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, text),
    };
    let b = rest.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i32> {
        if !b[r.clone()].iter().all(u8::is_ascii_digit) {
            return None;
        }
        rest[r].parse().ok()
    };
    let year = num(0..4)?;
    let year = if neg { -year } else { year };
    Date::new(year as i16, num(5..7)? as i8, num(8..10)? as i8).ok()
}

fn type_name(t: TxType) -> &'static str {
    match t {
        TxType::Expense => "expense",
        TxType::Income => "income",
        TxType::Transfer => "transfer",
    }
}

pub fn encode(rows: &[CsvRow], currency: &str, fraction_digits: u32) -> String {
    let mut sb = String::from(BOM);
    sb.push_str(&HEADER.join(","));
    sb.push_str("\r\n");
    for r in rows {
        let fields = [
            r.date.to_string(),
            type_name(r.r#type).to_string(),
            major_string(r.amount, fraction_digits),
            currency.to_string(),
            r.category.clone().unwrap_or_default(),
            r.account.clone(),
            r.to_account.clone().unwrap_or_default(),
            r.note.clone(),
        ];
        sb.push_str(&fields.iter().map(|f| quote(f)).collect::<Vec<_>>().join(","));
        sb.push_str("\r\n");
    }
    sb
}

pub fn major_string(minor: i64, fraction_digits: u32) -> String {
    if fraction_digits == 0 {
        return minor.to_string();
    }
    let scale = pow10(fraction_digits) as u64;
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    format!("{sign}{}.{:0width$}", abs / scale, abs % scale, width = fraction_digits as usize)
}

fn quote(field: &str) -> String {
    // A leading =, +, - or @ turns into a formula in a spreadsheet; an apostrophe defuses it.
    // A field that already starts with apostrophes before one gets one more, so [unguard]
    // always takes off exactly the one that was added.
    let safe = if needs_guard(field.trim_start_matches('\'')) { format!("'{field}") } else { field.to_string() };
    if safe.chars().any(|c| matches!(c, ',' | '"' | '\n' | '\r')) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

/// `[+-]?\d+(\.\d+)?`, whole.
fn plain_number(text: &str) -> bool {
    let t = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (int, frac) = match t.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (t, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    digits(int) && frac.is_none_or(digits)
}

/// True when `text` would start a formula: it opens with =, +, - or @ and is not a plain number.
fn needs_guard(text: &str) -> bool {
    text.chars().next().is_some_and(|c| FORMULA_START.contains(c)) && !plain_number(text)
}

/// Takes off the apostrophe [`quote`] put in front of a formula-like field, and nothing else.
pub fn unguard(field: &str) -> String {
    if field.starts_with('\'') && needs_guard(field.trim_start_matches('\'')) { field[1..].to_string() } else { field.to_string() }
}

/// Splits CSV text into records of fields, honouring quotes, doubled quotes and CRLF. A quote
/// opens a quoted section only at the start of a field (spaces before it are dropped); anywhere
/// else it is a plain character, as spreadsheets read it, so the inch mark in `TV 55" stand`
/// cannot swallow the rows that follow it. `delimiter` separates fields: a comma by default, a
/// semicolon or a tab for the bank exports that use one.
pub fn parse_records(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut field = String::new();
    let mut record = Vec::new();
    let mut in_quotes = false;
    // Whether the current field already had its quoted section; a second quote is plain text.
    let mut quoted = false;
    let src = text.strip_prefix(BOM).unwrap_or(text);
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if !quoted && kt_is_blank(&field) => {
                    field.clear();
                    in_quotes = true;
                    quoted = true;
                }
                '"' => field.push(c),
                c if c == delimiter => {
                    record.push(std::mem::take(&mut field));
                    quoted = false;
                }
                '\r' => {}
                '\n' => {
                    record.push(std::mem::take(&mut field));
                    quoted = false;
                    records.push(std::mem::take(&mut record));
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records.retain(|r: &Vec<String>| r.iter().any(|f| !kt_is_blank(f)));
    records
}

/// Reads a CSV with a header row. Needs `date` and `amount`; `type`, `category`, `account`,
/// `to_account` and `note` are optional. Without a type column a negative amount is an expense
/// and a positive one income, which is how most bank exports write it.
pub fn decode(text: &str, fraction_digits: u32, default_account: &str) -> CsvImport {
    let records = parse_records(text, ',');
    let Some(first) = records.first() else {
        return CsvImport { rows: vec![], errors: vec!["The file is empty.".into()] };
    };
    let header: Vec<String> = first.iter().map(|h| kt_trim(h).to_lowercase()).collect();
    let col = |name: &str| header.iter().position(|h| h == name);
    let (Some(date_col), Some(amount_col)) = (col("date"), col("amount")) else {
        return CsvImport { rows: vec![], errors: vec!["The first row needs a date column and an amount column.".into()] };
    };
    let type_col = col("type");
    let cat_col = col("category");
    let acc_col = col("account");
    let to_col = col("to_account");
    let note_col = col("note").or_else(|| col("description"));
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for (idx, r) in records.iter().enumerate().skip(1) {
        let line = idx + 1;
        let at = |i: Option<usize>| i.and_then(|i| r.get(i)).map_or("", |f| kt_trim(f));
        let Some(date) = parse_iso_date(at(Some(date_col))) else {
            errors.push(format!("Line {line}: the date \"{}\" is not in YYYY-MM-DD form.", at(Some(date_col))));
            continue;
        };
        // The names and the note come back without the apostrophe the export put before a formula.
        let text_at = |i: Option<usize>| unguard(at(i));
        let raw_amount = at(Some(amount_col));
        // "-4.25", "−4.25", "-$4.25", "$-4.25", "4.25-" and "(4.25)" are all money going out.
        let negative = has_sign(raw_amount);
        let amount = match parse_amount(raw_amount, fraction_digits) {
            Some(a) if a != 0 => a,
            _ => {
                errors.push(format!("Line {line}: \"{raw_amount}\" is not an amount."));
                continue;
            }
        };
        let r#type = match at(type_col).to_lowercase().as_str() {
            "expense" => TxType::Expense,
            "income" => TxType::Income,
            "transfer" => TxType::Transfer,
            "" => if negative { TxType::Expense } else { TxType::Income },
            _ => {
                errors.push(format!("Line {line}: the type \"{}\" is not expense, income or transfer.", at(type_col)));
                continue;
            }
        };
        let to_account = Some(text_at(to_col)).filter(|s| !s.is_empty());
        if r#type == TxType::Transfer && to_account.is_none() {
            errors.push(format!("Line {line}: a transfer needs a to_account."));
            continue;
        }
        let account = text_at(acc_col);
        rows.push(CsvRow {
            date,
            r#type,
            amount,
            category: Some(text_at(cat_col)).filter(|s| !s.is_empty()),
            account: if account.is_empty() { default_account.to_string() } else { account },
            to_account,
            note: text_at(note_col),
        });
    }
    CsvImport { rows, errors }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn row(d: Date, t: TxType, amount: i64, cat: Option<&str>, acc: &str, to: Option<&str>, note: &str) -> CsvRow {
        CsvRow { date: d, r#type: t, amount, category: cat.map(Into::into), account: acc.into(), to_account: to.map(Into::into), note: note.into() }
    }

    fn rows() -> Vec<CsvRow> {
        vec![
            row(date(2026, 10, 3), TxType::Expense, 1_250, Some("Dining"), "Visa", None, "Lunch, with \"Sam\""),
            row(date(2026, 10, 4), TxType::Transfer, 90_000, None, "Chequing", Some("Visa"), "Card payment"),
            row(date(2026, 10, 5), TxType::Income, 215_000, Some("Salary"), "Chequing", None, "=SUM(A1)"),
            row(date(2026, 10, 6), TxType::Expense, 8_431, Some("@Home"), "+Savings", None, "@Costco"),
            row(date(2026, 10, 7), TxType::Transfer, 5_000, None, "-Cash", Some("+Savings"), "'=already quoted"),
            row(date(2026, 10, 8), TxType::Expense, 499, Some("Épicerie"), "Visa", None, "'tis the season"),
            row(date(2026, 10, 9), TxType::Expense, 10_000, Some("Shopping"), "Visa", None, "TV 55\" stand"),
        ]
    }

    fn records(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
    }

    #[test]
    fn round_trips_through_encode_and_decode() {
        let text = encode(&rows(), "CAD", 2);
        let back = decode(&text, 2, "Chequing");
        assert!(back.errors.is_empty(), "{:?}", back.errors);
        assert_eq!(rows(), back.rows, "names and notes come back exactly, guard apostrophes and all");
    }

    #[test]
    fn starts_with_a_byte_order_mark_so_excel_reads_the_accents() {
        let text = encode(&rows(), "CAD", 2);
        assert!(text.starts_with("\u{FEFF}date,type,amount"));
        assert_eq!("date", parse_records(&text, ',')[0][0], "the reader drops it again");
    }

    #[test]
    fn the_guard_apostrophe_comes_off_only_where_the_export_put_it() {
        assert_eq!("@Costco", unguard("'@Costco"));
        assert_eq!("'=x", unguard("''=x"));
        assert_eq!("'hello", unguard("'hello"));
        assert_eq!("'-5", unguard("'-5"));
        assert_eq!("=x", unguard("=x"));
    }

    #[test]
    fn a_quote_inside_a_field_is_an_inch_mark_not_the_start_of_a_quoted_section() {
        let text = ["date,note,amount", "2026-10-01,TV 55\" stand,-100.00", "2026-10-02,Coffee,-4.25", "2026-10-03,Cable 6\" long,-9.99", "2026-10-04,Lunch,-12.00"].join("\n");
        let r = decode(&text, 2, "Visa");
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(vec!["TV 55\" stand", "Coffee", "Cable 6\" long", "Lunch"], r.rows.iter().map(|r| r.note.as_str()).collect::<Vec<_>>());
        assert_eq!(vec![10_000, 425, 999, 1_200], r.rows.iter().map(|r| r.amount).collect::<Vec<_>>());
        assert_eq!((1..=4).map(|d| date(2026, 10, d)).collect::<Vec<_>>(), r.rows.iter().map(|r| r.date).collect::<Vec<_>>());
    }

    #[test]
    fn a_quote_at_the_start_of_a_field_still_opens_a_quoted_section_spaces_before_it_or_not() {
        let recs = parse_records("a, \"b, c\",\"say \"\"hi\"\"\" \r\n", ',');
        assert_eq!(records(&[&["a", "b, c", "say \"hi\" "]]), recs);
    }

    #[test]
    fn every_way_of_writing_a_minus_reads_as_money_going_out() {
        let text = "date,amount\n2026-10-01,-4.25\n2026-10-01,−4.25\n2026-10-01,$-4.25\n2026-10-01,4.25-\n2026-10-01,(4.25)\n2026-10-01,4.25\n";
        let r = decode(text, 2, "Visa");
        let mut want = vec![TxType::Expense; 5];
        want.push(TxType::Income);
        assert_eq!(want, r.rows.iter().map(|r| r.r#type).collect::<Vec<_>>());
        assert!(r.rows.iter().all(|r| r.amount == 425));
    }

    #[test]
    fn quotes_commas_and_doubles_quotes() {
        let text = encode(&rows()[..1], "CAD", 2);
        assert!(text.contains("\"Lunch, with \"\"Sam\"\"\""), "{text}");
    }

    #[test]
    fn defuses_spreadsheet_formulas() {
        let text = encode(&rows()[2..], "CAD", 2);
        assert!(text.contains("'=SUM(A1)"), "{text}");
    }

    #[test]
    fn amounts_use_a_dot_whatever_the_locale() {
        assert_eq!("12.50", major_string(1_250, 2));
        assert_eq!("1500", major_string(1_500, 0));
        assert_eq!("0.05", major_string(5, 2));
    }

    #[test]
    fn bank_export_without_a_type_column_reads_signs() {
        let text = "Date,Description,Amount\n2026-10-01,Coffee,-4.25\n2026-10-02,Refund,12.00\n";
        let r = decode(text, 2, "Chequing");
        assert_eq!(vec![TxType::Expense, TxType::Income], r.rows.iter().map(|r| r.r#type).collect::<Vec<_>>());
        assert_eq!(vec![425, 1_200], r.rows.iter().map(|r| r.amount).collect::<Vec<_>>());
        assert_eq!("Coffee", r.rows[0].note);
        assert_eq!("Chequing", r.rows[0].account);
    }

    #[test]
    fn bad_lines_are_reported_by_number_and_skipped() {
        let text = "date,amount,type\n2026-13-01,5,expense\n2026-10-01,abc,expense\n2026-10-01,5,gift\n2026-10-02,5,expense\n";
        let r = decode(text, 2, "Cash");
        assert_eq!(1, r.rows.len());
        assert_eq!(3, r.errors.len());
        assert!(r.errors[0].starts_with("Line 2"), "{}", r.errors[0]);
    }

    #[test]
    fn a_file_without_the_needed_columns_says_so() {
        let r = decode("foo,bar\n1,2\n", 2, "Cash");
        assert!(r.rows.is_empty());
        assert_eq!(1, r.errors.len());
    }

    #[test]
    fn parses_quoted_newlines_and_a_byte_order_mark() {
        let recs = parse_records("\u{FEFF}a,b\r\n\"x\ny\",2\r\n", ',');
        assert_eq!(records(&[&["a", "b"], &["x\ny", "2"]]), recs);
    }

    #[test]
    fn iso_dates_read_strictly() {
        assert_eq!(parse_iso_date("2026-10-04"), Some(date(2026, 10, 4)));
        assert_eq!(parse_iso_date("2026-13-01"), None);
        assert_eq!(parse_iso_date("2026-1-01"), None);
        assert_eq!(parse_iso_date("20261004"), None);
        assert_eq!(parse_iso_date("2026-10-04T00:00"), None);
    }
}
