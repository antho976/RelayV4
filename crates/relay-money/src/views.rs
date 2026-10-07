//! What the ledger answers with: the shapes the `money.*` bus ops return. Apart from
//! [`crate::ledger`] so the bus contract does not pull in SQLite.

use crate::model::{AccountType, CategoryKind, GoalKind, TxType};
use crate::pace::{PaceReading, PaceStatus};
use crate::period::BudgetPeriod;
use jiff::civil::Date;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The ledger's own settings, carried in a backup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneySettings")]
pub struct Settings {
    pub currency: String,
    pub month_start_day: i64,
    pub week_starts_monday: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyPeriod")]
pub struct PeriodView {
    pub start: String,
    pub end_exclusive: String,
    pub days: i64,
    pub days_left: i64,
}

impl PeriodView {
    pub fn of(p: &BudgetPeriod, today: Date) -> Self {
        PeriodView { start: p.start.to_string(), end_exclusive: p.end_exclusive.to_string(), days: p.days(), days_left: p.days_left(today) }
    }
}

/// One entry as a client draws it, its account and category names joined in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyTx")]
pub struct Tx {
    pub id: i64,
    pub uid: String,
    pub r#type: TxType,
    pub amount: i64,
    pub date: String,
    pub account_id: i64,
    pub account: String,
    pub to_account_id: Option<i64>,
    pub to_account: Option<String>,
    pub category_id: Option<i64>,
    pub category: Option<String>,
    pub icon: Option<String>,
    pub color: Option<i64>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyAccount")]
pub struct AccountView {
    pub id: i64,
    pub name: String,
    pub r#type: AccountType,
    pub balance: i64,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyCategory")]
pub struct CategoryView {
    pub id: i64,
    pub name: String,
    pub kind: CategoryKind,
    pub icon: String,
    pub color: i64,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyEnvelope")]
pub struct EnvelopeView {
    pub category_id: i64,
    pub name: String,
    pub icon: String,
    pub color: i64,
    pub budget: i64,
    pub spent: i64,
    pub pace_delta: i64,
    pub status: PaceStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyBill")]
pub struct BillView {
    pub id: i64,
    pub name: String,
    pub amount: i64,
    pub r#type: TxType,
    pub next_date: String,
    pub days_until: i64,
    pub due_line: String,
    pub auto_post: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyGoal")]
pub struct GoalView {
    pub id: i64,
    pub name: String,
    pub kind: GoalKind,
    pub target: i64,
    pub saved: i64,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyLines")]
pub struct Lines {
    pub margin: String,
    pub pace: String,
    pub versus_last: String,
}

/// The Home reading (docs/MONEY.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneySummary")]
pub struct Summary {
    pub currency: String,
    pub fraction_digits: u32,
    pub month_start_day: i64,
    /// No accounts and no entries: draw the honest empty state.
    pub empty: bool,
    pub period: PeriodView,
    pub pace: PaceReading,
    pub income: i64,
    pub spent: i64,
    pub lines: Lines,
    pub budgets: Vec<EnvelopeView>,
    pub accounts: Vec<AccountView>,
    pub net_worth: i64,
    pub bills: Vec<BillView>,
    pub goals: Vec<GoalView>,
    pub recent: Vec<Tx>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyLists")]
pub struct Lists {
    pub currency: String,
    pub fraction_digits: u32,
    pub accounts: Vec<AccountView>,
    pub categories: Vec<CategoryView>,
    /// Phones that sync with this ledger, the latest first.
    pub devices: Vec<DeviceView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyDevice")]
pub struct DeviceView {
    pub name: String,
    /// When it last synced (RFC 3339).
    pub last_sync: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyTxQuery")]
pub struct TxQuery {
    /// 0 is the current budget period, -1 the one before.
    pub period_offset: Option<i64>,
    pub query: Option<String>,
    pub account_id: Option<i64>,
    pub category_id: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyTxPage")]
pub struct TxPage {
    pub period: PeriodView,
    pub transactions: Vec<Tx>,
    pub income: i64,
    pub spent: i64,
}

/// What a new entry carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyTxInput")]
pub struct TxInput {
    pub r#type: TxType,
    pub amount: i64,
    pub date: String,
    pub account_id: i64,
    pub to_account_id: Option<i64>,
    pub category_id: Option<i64>,
    pub note: Option<String>,
}

/// What an edit changes; absent fields keep their value. A transfer's `to_account_id` and an
/// entry's `category_id` are cleared by the type they no longer fit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyTxPatch")]
pub struct TxPatch {
    pub r#type: Option<TxType>,
    pub amount: Option<i64>,
    pub date: Option<String>,
    pub account_id: Option<i64>,
    pub to_account_id: Option<i64>,
    pub category_id: Option<i64>,
    pub note: Option<String>,
}

/// One row's change on the sync wire (docs/MONEY.md, "Sync"). `row` holds the table's fields in
/// camelCase, references as uids; empty for a tombstone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneyChange")]
pub struct Change {
    pub table: String,
    pub uid: String,
    pub updated_at: i64,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub row: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "MoneySyncOut")]
pub struct SyncOut {
    /// Store it and send it back as `since` next time.
    pub cursor: i64,
    /// This ledger's changes the device has not seen.
    pub changes: Vec<Change>,
    /// This ledger was replaced by the device's.
    pub replaced: bool,
    /// The device's changes taken, and those older than what is here or not placeable.
    pub applied: usize,
    pub skipped: usize,
}
