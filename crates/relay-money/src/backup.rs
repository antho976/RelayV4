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
use crate::model::{AccountType, ActivityType, CategoryKind, GoalKind, Registration, SecurityKind, TxType};
use crate::recurrence::Frequency;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const FORMAT: &str = "tally-backup";

/// 2 added goal kinds, investment accounts and their values; 3 added investments (securities,
/// holdings, activities, prices, rates and room) and an account's registration. Older files still
/// read.
pub const VERSION: i32 = 3;

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
    /// The investments: what is held, what was done, prices, rates and room. Absent before version 3.
    #[serde(default)]
    pub securities: Vec<SecurityDto>,
    #[serde(default)]
    pub holdings: Vec<HoldingDto>,
    #[serde(default)]
    pub activities: Vec<ActivityDto>,
    #[serde(default)]
    pub prices: Vec<PriceDto>,
    #[serde(default)]
    pub fx_rates: Vec<FxRateDto>,
    #[serde(default)]
    pub room_facts: Vec<RoomFactDto>,
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
            securities: vec![],
            holdings: vec![],
            activities: vec![],
            prices: vec![],
            fx_rates: vec![],
            room_facts: vec![],
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
    /// An investment account's tax wrapper; `None` on every other account.
    #[serde(default)]
    pub registration: Option<Registration>,
    #[serde(default)]
    pub institution: String,
    /// The institution's own number for the account, e.g. Wealthsimple's "HQ7XFMC41CAD".
    #[serde(default)]
    pub external_ref: String,
    /// The row's uid, so a restore keeps the uids an import derived (`ws:`, `sec:`, `imp:`…) and the
    /// same file imported again adds nothing. Absent (an older file) or blank: a fresh one.
    #[serde(default)]
    pub uid: Option<String>,
}

impl AccountDto {
    /// An account with every optional field at its Kotlin default.
    pub fn new(id: i64, name: impl Into<String>, r#type: AccountType, opening_balance: i64) -> Self {
        AccountDto {
            id,
            name: name.into(),
            r#type,
            opening_balance,
            archived: false,
            sort_order: 0,
            registration: None,
            institution: String::new(),
            external_ref: String::new(),
            uid: None,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityDto {
    pub id: i64,
    pub symbol: String,
    #[serde(default)]
    pub name: String,
    pub currency: String,
    pub kind: SecurityKind,
    #[serde(default)]
    pub exchange: String,
    /// As [`AccountDto::uid`].
    #[serde(default)]
    pub uid: Option<String>,
}

/// A line of an account's holdings snapshot. `book` is in the ledger currency, `book_market` in the security's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldingDto {
    pub id: i64,
    pub account_id: i64,
    pub security_id: i64,
    pub date: String,
    pub quantity: i64,
    pub book: i64,
    pub book_market: i64,
    /// As [`AccountDto::uid`].
    #[serde(default)]
    pub uid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDto {
    pub id: i64,
    pub account_id: i64,
    #[serde(default)]
    pub security_id: Option<i64>,
    pub r#type: ActivityType,
    pub date: String,
    #[serde(default)]
    pub quantity: i64,
    pub amount: i64,
    #[serde(default)]
    pub fee: i64,
    pub currency: String,
    #[serde(default)]
    pub to_amount: Option<i64>,
    #[serde(default)]
    pub to_currency: Option<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default = "manual")]
    pub source: String,
    /// As [`AccountDto::uid`].
    #[serde(default)]
    pub uid: Option<String>,
}

fn manual() -> String {
    "MANUAL".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceDto {
    pub id: i64,
    pub security_id: i64,
    pub date: String,
    pub price: i64,
    pub source: String,
    /// As [`AccountDto::uid`].
    #[serde(default)]
    pub uid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxRateDto {
    pub id: i64,
    pub base: String,
    pub quote: String,
    pub date: String,
    pub rate: i64,
    pub source: String,
    /// As [`AccountDto::uid`].
    #[serde(default)]
    pub uid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomFactDto {
    pub id: i64,
    pub registration: Registration,
    pub year: i32,
    pub amount: i64,
    /// As [`AccountDto::uid`].
    #[serde(default)]
    pub uid: Option<String>,
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
    let securities: HashSet<i64> = file.securities.iter().map(|s| s.id).collect();
    for h in &file.holdings {
        if !accounts.contains(&h.account_id) {
            return fail("A holding points at an account that is not in the file.");
        }
        if !securities.contains(&h.security_id) {
            return fail("A holding points at a security that is not in the file.");
        }
        if !readable(Some(&h.date)) {
            return fail("A holding has an unreadable date.");
        }
    }
    for a in &file.activities {
        if a.quantity < 0 || a.amount < 0 || a.fee < 0 || a.to_amount.unwrap_or(0) < 0 {
            return fail("An activity has a negative amount.");
        }
        if !accounts.contains(&a.account_id) {
            return fail("An activity points at an account that is not in the file.");
        }
        if missing(&securities, a.security_id) {
            return fail("An activity points at a security that is not in the file.");
        }
        if !readable(Some(&a.date)) {
            return fail("An activity has an unreadable date.");
        }
    }
    for p in &file.prices {
        if !securities.contains(&p.security_id) {
            return fail("A price points at a security that is not in the file.");
        }
        if !readable(Some(&p.date)) {
            return fail("A price has an unreadable date.");
        }
    }
    for r in &file.fx_rates {
        if r.rate <= 0 {
            return fail("An exchange rate is zero or less.");
        }
        if !readable(Some(&r.date)) {
            return fail("An exchange rate has an unreadable date.");
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
    fn a_version_2_file_without_investments_still_reads() {
        let v2 = r#"{"format":"tally-backup","version":2,"exportedAt":"2026-10-04","currency":"CAD",
            "accounts":[{"id":1,"name":"TFSA","type":"INVESTMENT","openingBalance":0,"archived":false,"sortOrder":0}],
            "values":[{"id":1,"accountId":1,"date":"2026-10-01","value":500000}]}"#;
        let read = ok(decode(v2));
        assert_eq!((None, ""), (read.accounts[0].registration, read.accounts[0].external_ref.as_str()));
        assert_eq!(None, read.accounts[0].uid, "A file from before uids were kept has none; the restore makes them");
        assert!(read.securities.is_empty() && read.activities.is_empty() && read.room_facts.is_empty());
        assert_eq!(1, read.values.len());
    }

    /// The sample with its TFSA (account 5) holding XEQT, a buy and an exchange, a price, a rate and room.
    fn invested() -> BackupFile {
        let mut s = sample();
        for a in s.accounts.iter_mut().filter(|a| a.r#type == AccountType::Investment) {
            a.registration = Some(Registration::Tfsa);
            a.institution = "Wealthsimple".into();
            a.external_ref = "HQ7XFMC41CAD".into();
            a.uid = Some("ws:HQ7XFMC41CAD".into());
        }
        s.securities = vec![SecurityDto {
            id: 1, symbol: "XEQT".into(), name: "iShares Core Equity ETF Portfolio".into(), currency: "CAD".into(), kind: SecurityKind::Etf,
            exchange: "TSX".into(), uid: Some("sec:XEQT".into()),
        }];
        s.holdings = vec![HoldingDto {
            id: 1, account_id: 5, security_id: 1, date: "2026-10-01".into(), quantity: 1_000_000_000, book: 38_120, book_market: 38_120,
            uid: Some("hold:ws:HQ7XFMC41CAD:sec:XEQT:2026-10-01".into()),
        }];
        s.activities = vec![
            ActivityDto {
                id: 1, account_id: 5, security_id: Some(1), r#type: ActivityType::Buy, date: "2026-10-02".into(), quantity: 100_000_000, amount: 3_900,
                fee: 0, currency: "CAD".into(), to_amount: None, to_currency: None, note: String::new(), source: "WEALTHSIMPLE".into(),
                uid: Some("imp:86fe65d31110bce166befa06e9747e9a".into()),
            },
            ActivityDto {
                id: 2, account_id: 5, security_id: None, r#type: ActivityType::Fx, date: "2026-10-03".into(), quantity: 0, amount: 10_000, fee: 0,
                currency: "CAD".into(), to_amount: Some(7_300), to_currency: Some("USD".into()), note: String::new(), source: "MANUAL".into(), uid: None,
            },
        ];
        s.prices = vec![PriceDto {
            id: 1, security_id: 1, date: "2026-10-01".into(), price: 3_812_000_000, source: "IMPORT".into(), uid: Some("px:sec:XEQT:2026-10-01".into()),
        }];
        s.fx_rates = vec![FxRateDto {
            id: 1, base: "USD".into(), quote: "CAD".into(), date: "2026-10-01".into(), rate: 137_125_000, source: "BANK_OF_CANADA".into(),
            uid: Some("fx:USD:CAD:2026-10-01".into()),
        }];
        s.room_facts = vec![RoomFactDto { id: 1, registration: Registration::Tfsa, year: 2026, amount: 700_000, uid: Some("room:TFSA:2026".into()) }];
        s
    }

    #[test]
    fn investments_round_trip() {
        assert_eq!(5, sample().accounts.iter().find(|a| a.r#type == AccountType::Investment).unwrap().id);
        let text = encode(&invested());
        assert_eq!(invested(), ok(decode(&text)), "Uids ride along, a missing one as null");
        assert!(text.contains("\"externalRef\": \"HQ7XFMC41CAD\",\n            \"uid\": \"ws:HQ7XFMC41CAD\"\n"), "{text}");
    }

    #[test]
    fn investments_must_point_at_what_is_in_the_file() {
        let with = |change: &dyn Fn(&mut BackupFile)| {
            let mut f = invested();
            change(&mut f);
            reason(&f)
        };
        let says = |s: &str| Some(s.to_string());
        assert_eq!(None, reason(&invested()));
        assert_eq!(says("A holding points at an account that is not in the file."), with(&|f| f.holdings[0].account_id = 404));
        assert_eq!(says("A holding points at a security that is not in the file."), with(&|f| f.holdings[0].security_id = 404));
        assert_eq!(says("A holding has an unreadable date."), with(&|f| f.holdings[0].date = "May".into()));
        assert_eq!(says("An activity has a negative amount."), with(&|f| f.activities.iter_mut().for_each(|a| a.fee = -1)));
        assert_eq!(says("An activity points at an account that is not in the file."), with(&|f| f.activities.iter_mut().for_each(|a| a.account_id = 404)));
        assert_eq!(says("An activity points at a security that is not in the file."), with(&|f| f.activities.iter_mut().for_each(|a| a.security_id = Some(404))));
        assert_eq!(says("An activity has an unreadable date."), with(&|f| f.activities.iter_mut().for_each(|a| a.date = String::new())));
        assert_eq!(says("A price points at a security that is not in the file."), with(&|f| f.prices[0].security_id = 404));
        assert_eq!(says("A price has an unreadable date."), with(&|f| f.prices[0].date = "2026-02-30".into()));
        assert_eq!(says("An exchange rate is zero or less."), with(&|f| f.fx_rates[0].rate = 0));
        assert_eq!(says("An exchange rate has an unreadable date."), with(&|f| f.fx_rates[0].date = "today".into()));
    }

    #[test]
    fn ignores_unknown_keys_from_a_future_minor_change() {
        let text = encode(&sample()).replacen('{', "{\"futureField\": true,", 1);
        assert!(matches!(decode(&text), BackupReadResult::Ok(_)));
    }

    #[test]
    fn writes_every_field_in_kotlin_order_with_nulls_and_four_space_indent() {
        let text = encode(&BackupFile::new("2026-10-04", "CAD"));
        let want = "{\n    \"format\": \"tally-backup\",\n    \"version\": 3,\n    \"exportedAt\": \"2026-10-04\",\n    \"currency\": \"CAD\",\n    \"monthStartDay\": null,\n    \"weekStartsMonday\": null,\n    \"accounts\": [],\n    \"categories\": [],\n    \"transactions\": [],\n    \"budgets\": [],\n    \"recurring\": [],\n    \"goals\": [],\n    \"contributions\": [],\n    \"values\": [],\n    \"securities\": [],\n    \"holdings\": [],\n    \"activities\": [],\n    \"prices\": [],\n    \"fxRates\": [],\n    \"roomFacts\": []\n}";
        assert_eq!(want, text);
        let mut account = BackupFile::new("2026-10-04", "CAD");
        account.accounts.push(AccountDto::new(1, "TFSA", AccountType::Investment, 0));
        let text = encode(&account);
        assert!(text.contains("\"sortOrder\": 0,\n            \"registration\": null,\n            \"institution\": \"\",\n            \"externalRef\": \"\",\n            \"uid\": null\n"), "{text}");
        // Every investment row's uid comes last too.
        let text = encode(&invested());
        for tail in ["\"exchange\": \"TSX\",\n            \"uid\": \"sec:XEQT\"\n", "\"bookMarket\": 38120,\n            \"uid\": \"hold:ws:HQ7XFMC41CAD:sec:XEQT:2026-10-01\"\n",
            "\"source\": \"MANUAL\",\n            \"uid\": null\n", "\"source\": \"IMPORT\",\n            \"uid\": \"px:sec:XEQT:2026-10-01\"\n",
            "\"source\": \"BANK_OF_CANADA\",\n            \"uid\": \"fx:USD:CAD:2026-10-01\"\n", "\"amount\": 700000,\n            \"uid\": \"room:TFSA:2026\"\n"]
        {
            assert!(text.contains(tail), "{tail} in {text}");
        }
    }

    #[test]
    fn a_missing_format_and_version_take_the_kotlin_defaults() {
        let read = ok(decode(r#"{"exportedAt":"2026-10-04","currency":"CAD"}"#));
        assert_eq!(FORMAT, read.format);
        assert_eq!(VERSION, read.version);
        assert!(matches!(decode(r#"{"exportedAt":"2026-10-04","currency":"CAD","accounts":null}"#), BackupReadResult::Invalid(_)), "null is not a default");
    }
}
