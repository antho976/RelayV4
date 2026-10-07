//! The JSON backup: the whole database as one human-readable file. Port of `Backup.kt`.
//!
//! Ids are kept so references survive a restore; dates are ISO strings ("2026-10-04") so the file
//! reads without a decoder.
//!
//! The file is the one the phone writes and reads, so the shape follows kotlinx.serialization with
//! the phone's settings (`prettyPrint`, `encodeDefaults`, `ignoreUnknownKeys`, nulls written out):
//! camelCase names in declaration order, every field written, a missing field taking the Kotlin
//! default, an unknown one ignored, and four-space indentation.

use crate::csv::parse_iso_date;
use crate::model::{AccountType, CategoryKind, GoalKind, TxType};
use crate::recurrence::Frequency;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const FORMAT: &str = "tally-backup";

/// 2 added goal kinds, investment accounts and their values. A version 1 file still reads.
pub const VERSION: i32 = 2;

fn default_format() -> String {
    FORMAT.to_string()
}

fn default_version() -> i32 {
    VERSION
}

fn yes() -> bool {
    true
}

fn one() -> i32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupFile {
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_version")]
    pub version: i32,
    pub exported_at: String,
    pub currency: String,
    /// The day the budget month starts on, so a payday month survives a restore. `None` when the
    /// file does not say (the sample, or a file from before it was kept): a restore leaves the
    /// phone's own setting alone then.
    #[serde(default)]
    pub month_start_day: Option<i32>,
    /// Whether weeks start on Monday; `None` when the file does not say, as for `month_start_day`.
    #[serde(default)]
    pub week_starts_monday: Option<bool>,
    #[serde(default)]
    pub accounts: Vec<AccountDto>,
    #[serde(default)]
    pub categories: Vec<CategoryDto>,
    #[serde(default)]
    pub transactions: Vec<TransactionDto>,
    #[serde(default)]
    pub budgets: Vec<BudgetDto>,
    #[serde(default)]
    pub recurring: Vec<RecurringDto>,
    #[serde(default)]
    pub goals: Vec<GoalDto>,
    #[serde(default)]
    pub contributions: Vec<ContributionDto>,
    /// What investment accounts were worth on a day, as typed by the owner. Absent before version 2.
    #[serde(default)]
    pub values: Vec<AccountValueDto>,
}

impl BackupFile {
    /// An empty file of this format and version, every list empty and both settings unsaid.
    pub fn new(exported_at: impl Into<String>, currency: impl Into<String>) -> Self {
        BackupFile {
            format: FORMAT.to_string(),
            version: VERSION,
            exported_at: exported_at.into(),
            currency: currency.into(),
            month_start_day: None,
            week_starts_monday: None,
            accounts: vec![],
            categories: vec![],
            transactions: vec![],
            budgets: vec![],
            recurring: vec![],
            goals: vec![],
            contributions: vec![],
            values: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    pub id: i64,
    pub name: String,
    pub r#type: AccountType,
    pub opening_balance: i64,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryDto {
    pub id: i64,
    pub name: String,
    pub kind: CategoryKind,
    pub color: i32,
    pub icon: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub sort_order: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionDto {
    pub id: i64,
    pub r#type: TxType,
    pub amount: i64,
    pub date: String,
    pub account_id: i64,
    #[serde(default)]
    pub to_account_id: Option<i64>,
    #[serde(default)]
    pub category_id: Option<i64>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub recurring_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetDto {
    pub id: i64,
    /// `None` is the overall monthly budget.
    #[serde(default)]
    pub category_id: Option<i64>,
    pub amount: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurringDto {
    pub id: i64,
    pub name: String,
    pub r#type: TxType,
    pub amount: i64,
    pub account_id: i64,
    #[serde(default)]
    pub to_account_id: Option<i64>,
    #[serde(default)]
    pub category_id: Option<i64>,
    pub frequency: Frequency,
    #[serde(default = "one")]
    pub interval: i32,
    pub anchor_date: String,
    pub next_date: String,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default = "yes")]
    pub auto_post: bool,
    #[serde(default = "yes")]
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalDto {
    pub id: i64,
    pub name: String,
    pub target: i64,
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub color: i32,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "savings")]
    pub kind: GoalKind,
    /// The account a balance or invest goal reads; `None` reads every account (or every investment one).
    #[serde(default)]
    pub account_id: Option<i64>,
    /// A monthly goal's share of what came in, 1 to 100; 0 means `target` is a set amount a month.
    #[serde(default)]
    pub percent: i32,
    /// Where a balance goal started from: the day it was set, and what it read then.
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub start_amount: i64,
}

fn savings() -> GoalKind {
    GoalKind::Savings
}

impl GoalDto {
    /// A savings goal with every optional field at its Kotlin default.
    pub fn new(id: i64, name: impl Into<String>, target: i64) -> Self {
        GoalDto {
            id,
            name: name.into(),
            target,
            target_date: None,
            color: 0,
            archived: false,
            kind: GoalKind::Savings,
            account_id: None,
            percent: 0,
            start_date: None,
            start_amount: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountValueDto {
    pub id: i64,
    pub account_id: i64,
    pub date: String,
    pub value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContributionDto {
    pub id: i64,
    pub goal_id: i64,
    pub amount: i64,
    pub date: String,
    #[serde(default)]
    pub note: String,
}

/// The file is read once per restore, so it is not boxed to even out the two sizes.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupReadResult {
    Ok(BackupFile),
    /// The reason is shown to the owner as-is, so it names the problem and not the parser.
    Invalid(String),
}

/// The intervals `Recurrence` accepts; a bill outside them would panic where it is read.
const BILL_INTERVALS: std::ops::RangeInclusive<i32> = 1..=52;

/// The file as the phone writes it: kotlinx's pretty print, four spaces deep.
pub fn encode(file: &BackupFile) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b"    "));
    file.serialize(&mut ser).expect("a backup always serializes");
    String::from_utf8(out).expect("serde_json writes UTF-8")
}

pub fn decode(text: &str) -> BackupReadResult {
    let Ok(file) = serde_json::from_str::<BackupFile>(text) else {
        return BackupReadResult::Invalid("This file is not a Tally backup.".into());
    };
    if file.format != FORMAT {
        return BackupReadResult::Invalid("This file is not a Tally backup.".into());
    }
    if file.version > VERSION {
        return BackupReadResult::Invalid("This backup is from a newer version of Tally. Update the app, then restore.".into());
    }
    validate(file)
}

/// References must resolve and values must be ones the app itself could have written, or the
/// restore would write orphans, or rows that crash the screen or the poster that reads them.
/// The file is human-readable and may have been edited by hand, so nothing is taken on trust.
pub fn validate(file: BackupFile) -> BackupReadResult {
    match problem(&file) {
        Some(reason) => BackupReadResult::Invalid(reason),
        None => BackupReadResult::Ok(file),
    }
}

/// What [`validate`] refuses `file` for, or `None` when it passes.
pub fn problem(file: &BackupFile) -> Option<String> {
    let accounts: HashSet<i64> = file.accounts.iter().map(|a| a.id).collect();
    let categories: HashSet<i64> = file.categories.iter().map(|c| c.id).collect();
    let goals: HashSet<i64> = file.goals.iter().map(|g| g.id).collect();
    let bills: HashSet<i64> = file.recurring.iter().map(|r| r.id).collect();
    let readable = |date: Option<&str>| date.is_none_or(|d| parse_iso_date(d).is_some());
    let missing = |set: &HashSet<i64>, id: Option<i64>| id.is_some_and(|id| !set.contains(&id));
    let fail = |reason: &str| Some(reason.to_string());
    for t in &file.transactions {
        if t.amount < 0 {
            return fail("A transaction has a negative amount.");
        }
        if !accounts.contains(&t.account_id) {
            return fail("A transaction points at an account that is not in the file.");
        }
        if missing(&accounts, t.to_account_id) {
            return fail("A transfer points at an account that is not in the file.");
        }
        if missing(&categories, t.category_id) {
            return fail("A transaction points at a category that is not in the file.");
        }
        if missing(&bills, t.recurring_id) {
            return fail("A transaction points at a bill that is not in the file.");
        }
        if !readable(Some(&t.date)) {
            return fail("A transaction has an unreadable date.");
        }
    }
    for r in &file.recurring {
        if r.amount <= 0 {
            return fail("A bill has an amount of zero or less.");
        }
        if !BILL_INTERVALS.contains(&r.interval) {
            return Some(format!("A bill repeats at an interval outside {} to {}.", BILL_INTERVALS.start(), BILL_INTERVALS.end()));
        }
        if !accounts.contains(&r.account_id) || missing(&accounts, r.to_account_id) {
            return fail("A bill points at an account that is not in the file.");
        }
        if r.r#type == TxType::Transfer && (r.to_account_id.is_none() || r.to_account_id == Some(r.account_id)) {
            return fail("A transfer bill needs two different accounts.");
        }
        if missing(&categories, r.category_id) {
            return fail("A bill points at a category that is not in the file.");
        }
        if !readable(Some(&r.anchor_date)) || !readable(Some(&r.next_date)) || !readable(r.end_date.as_deref()) {
            return fail("A bill has an unreadable date.");
        }
    }
    for g in &file.goals {
        if g.target < 0 {
            return fail("A goal has a negative target.");
        }
        if !readable(g.target_date.as_deref()) || !readable(g.start_date.as_deref()) {
            return fail("A goal has an unreadable date.");
        }
        if !(0..=100).contains(&g.percent) {
            return fail("A goal asks for a share outside 0 to 100 percent.");
        }
        if missing(&accounts, g.account_id) {
            return fail("A goal points at an account that is not in the file.");
        }
    }
    for v in &file.values {
        if !accounts.contains(&v.account_id) {
            return fail("An account value points at an account that is not in the file.");
        }
        if !readable(Some(&v.date)) {
            return fail("An account value has an unreadable date.");
        }
    }
    for c in &file.contributions {
        if !goals.contains(&c.goal_id) {
            return fail("A contribution points at a goal that is not in the file.");
        }
        if !readable(Some(&c.date)) {
            return fail("A contribution has an unreadable date.");
        }
    }
    for b in &file.budgets {
        if b.amount < 0 {
            return fail("A budget has a negative amount.");
        }
        if missing(&categories, b.category_id) {
            return fail("A budget points at a category that is not in the file.");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample_data;
    use jiff::civil::date;
    use regex::Regex;

    fn sample() -> BackupFile {
        sample_data::build(date(2026, 10, 4), 2, "CAD", sample_data::DEFAULT_SEED)
    }

    fn ok(r: BackupReadResult) -> BackupFile {
        match r {
            BackupReadResult::Ok(f) => f,
            other => panic!("{other:?}"),
        }
    }

    fn reason(file: &BackupFile) -> Option<String> {
        match validate(file.clone()) {
            BackupReadResult::Invalid(r) => Some(r),
            BackupReadResult::Ok(_) => None,
        }
    }

    fn with_bill(change: impl FnOnce(&mut RecurringDto)) -> BackupFile {
        let mut s = sample();
        change(&mut s.recurring[0]);
        s
    }

    #[test]
    fn sample_data_round_trips() {
        let text = encode(&sample());
        assert_eq!(sample(), ok(decode(&text)));
    }

    #[test]
    fn rejects_files_that_are_not_backups() {
        assert!(matches!(decode("{\"hello\":1}"), BackupReadResult::Invalid(_)));
        assert!(matches!(decode("not json"), BackupReadResult::Invalid(_)));
    }

    #[test]
    fn rejects_a_newer_version_with_a_reason_that_says_what_to_do() {
        let text = encode(&BackupFile { version: VERSION + 1, ..sample() });
        let BackupReadResult::Invalid(r) = decode(&text) else { panic!("read a newer file") };
        assert!(r.contains("Update the app"), "{r}");
    }

    #[test]
    fn rejects_dangling_references() {
        let mut broken = sample();
        broken.transactions.push(TransactionDto { id: 99_999, account_id: 404, ..broken.transactions[0].clone() });
        assert!(matches!(validate(broken), BackupReadResult::Invalid(_)));
    }

    #[test]
    fn the_sample_passes_every_check() {
        assert_eq!(None, reason(&sample()));
    }

    #[test]
    fn a_bill_that_would_throw_where_it_is_read_is_refused() {
        assert_eq!(Some("A bill repeats at an interval outside 1 to 52.".into()), reason(&with_bill(|b| b.interval = 0)));
        assert_eq!(Some("A bill repeats at an interval outside 1 to 52.".into()), reason(&with_bill(|b| b.interval = 60)));
        assert_eq!(Some("A bill has an amount of zero or less.".into()), reason(&with_bill(|b| b.amount = -5_00)));
        assert_eq!(Some("A bill has an unreadable date.".into()), reason(&with_bill(|b| b.next_date = "soon".into())));
        assert_eq!(Some("A bill has an unreadable date.".into()), reason(&with_bill(|b| b.end_date = Some("2026-13-01".into()))));
    }

    #[test]
    fn a_bill_must_point_at_what_is_in_the_file() {
        assert_eq!(Some("A bill points at an account that is not in the file.".into()), reason(&with_bill(|b| b.account_id = 404)));
        assert_eq!(
            Some("A bill points at a category that is not in the file.".into()),
            reason(&with_bill(|b| {
                b.r#type = TxType::Expense;
                b.category_id = Some(404);
            })),
        );
        let account = sample().accounts[0].id;
        assert_eq!(
            Some("A transfer bill needs two different accounts.".into()),
            reason(&with_bill(|b| {
                b.r#type = TxType::Transfer;
                b.account_id = account;
                b.to_account_id = None;
                b.category_id = None;
            })),
        );
        assert_eq!(
            Some("A transfer bill needs two different accounts.".into()),
            reason(&with_bill(|b| {
                b.r#type = TxType::Transfer;
                b.account_id = account;
                b.to_account_id = Some(account);
                b.category_id = None;
            })),
        );
    }

    #[test]
    fn an_entry_must_point_at_a_bill_that_is_in_the_file() {
        let mut orphan = sample();
        orphan.transactions.push(TransactionDto { id: 99_999, recurring_id: Some(404), ..orphan.transactions[0].clone() });
        assert_eq!(Some("A transaction points at a bill that is not in the file.".into()), reason(&orphan));
    }

    #[test]
    fn budgets_and_goals_cannot_be_negative() {
        let mut b = sample();
        b.budgets.push(BudgetDto { id: 99, category_id: None, amount: -1 });
        assert_eq!(Some("A budget has a negative amount.".into()), reason(&b));
        let mut g = sample();
        g.goals.iter_mut().for_each(|g| g.target = -1);
        assert_eq!(Some("A goal has a negative target.".into()), reason(&g));
    }

    #[test]
    fn the_month_start_and_week_start_ride_along_and_an_older_file_without_them_still_reads() {
        let payday = BackupFile { month_start_day: Some(15), week_starts_monday: Some(false), ..sample() };
        let back = ok(decode(&encode(&payday)));
        assert_eq!(Some(15), back.month_start_day);
        assert_eq!(Some(false), back.week_starts_monday);

        let older = encode(&sample());
        let older = Regex::new(r#"\s*"monthStartDay": null,"#).unwrap().replace_all(&older, "");
        let older = Regex::new(r#"\s*"weekStartsMonday": null,"#).unwrap().replace_all(&older, "");
        assert!(!older.contains("monthStartDay"), "{older}");
        let read = ok(decode(&older));
        assert_eq!(None, read.month_start_day, "a file that does not say leaves the setting alone");
        assert_eq!(None, read.week_starts_monday);
    }

    #[test]
    fn a_version_1_file_without_goal_kinds_or_values_still_reads() {
        let v1 = r#"{"format":"tally-backup","version":1,"exportedAt":"2026-10-04","currency":"CAD",
            "accounts":[{"id":1,"name":"Chequing","type":"CHEQUING","openingBalance":0}],
            "goals":[{"id":1,"name":"Trip","target":100000}]}"#;
        let read = ok(decode(v1));
        assert_eq!(1, read.goals.len());
        assert_eq!(GoalKind::Savings, read.goals[0].kind);
        assert_eq!(Vec::<AccountValueDto>::new(), read.values);
    }

    #[test]
    fn goal_kinds_and_account_values_must_point_at_what_is_in_the_file() {
        let mut s = sample();
        s.goals.iter_mut().for_each(|g| {
            g.kind = GoalKind::Balance;
            g.account_id = Some(404);
        });
        assert_eq!(Some("A goal points at an account that is not in the file.".into()), reason(&s));
        let mut s = sample();
        s.goals.iter_mut().for_each(|g| {
            g.kind = GoalKind::Invest;
            g.percent = 150;
        });
        assert_eq!(Some("A goal asks for a share outside 0 to 100 percent.".into()), reason(&s));
        let s = BackupFile { values: vec![AccountValueDto { id: 1, account_id: 404, date: "2026-10-01".into(), value: 5_00 }], ..sample() };
        assert_eq!(Some("An account value points at an account that is not in the file.".into()), reason(&s));
        let account = sample().accounts[0].id;
        let s = BackupFile { values: vec![AccountValueDto { id: 1, account_id: account, date: "later".into(), value: 5_00 }], ..sample() };
        assert_eq!(Some("An account value has an unreadable date.".into()), reason(&s));
    }

    #[test]
    fn ignores_unknown_keys_from_a_future_minor_change() {
        let text = encode(&sample()).replacen('{', "{\"futureField\": true,", 1);
        assert!(matches!(decode(&text), BackupReadResult::Ok(_)));
    }

    #[test]
    fn writes_every_field_in_kotlin_order_with_nulls_and_four_space_indent() {
        let text = encode(&BackupFile::new("2026-10-04", "CAD"));
        let want = "{\n    \"format\": \"tally-backup\",\n    \"version\": 2,\n    \"exportedAt\": \"2026-10-04\",\n    \"currency\": \"CAD\",\n    \"monthStartDay\": null,\n    \"weekStartsMonday\": null,\n    \"accounts\": [],\n    \"categories\": [],\n    \"transactions\": [],\n    \"budgets\": [],\n    \"recurring\": [],\n    \"goals\": [],\n    \"contributions\": [],\n    \"values\": []\n}";
        assert_eq!(want, text);
    }

    #[test]
    fn a_missing_format_and_version_take_the_kotlin_defaults() {
        let read = ok(decode(r#"{"exportedAt":"2026-10-04","currency":"CAD"}"#));
        assert_eq!(FORMAT, read.format);
        assert_eq!(VERSION, read.version);
        assert!(matches!(decode(r#"{"exportedAt":"2026-10-04","currency":"CAD","accounts":null}"#), BackupReadResult::Invalid(_)), "null is not a default");
    }
}
