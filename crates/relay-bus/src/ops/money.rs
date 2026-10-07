//! `money.*` — the Money space's ledger (docs/MONEY.md).
//!
//! The ledger is Tally's, kept by the engine in its own SQLite file (`money.db`); its rules and
//! read shapes are `relay_money`'s. Amounts are minor units, dates `YYYY-MM-DD`, enum values
//! Tally's names. Every mutation emits `money.changed`. What replaces or erases the ledger, or
//! changes its currency, is the person's alone (`Actors::UserOnly`): an agent may read and add
//! entries, never wipe them.
use crate::registry::{Actors, OpMeta, Scope};
use crate::{op, Empty};
use relay_money::model::{AccountType, TxType};
use relay_money::views::{AccountView, Lists, Settings, Summary, Tx, TxPage};

payload!(#[schemars(rename = "MoneySummaryIn")] SummaryIn {
    /// The day to read the month from; today in the engine's time zone when absent.
    pub today: Option<String>,
});
op!(SummaryOp, "money.summary", SummaryIn => Summary,
    OpMeta::query(Scope::Global, 12, "The Money Home reading: what is left this budget period and its pace, budgets, bills, accounts, goals and recent entries. Posts any bill due by today first"));
op!(ListsOp, "money.lists", Empty => Lists, OpMeta::query(Scope::Global, 12, "The ledger's accounts (with balances) and categories, for pickers"));
payload!(#[schemars(rename = "MoneyTxListIn")] TxListIn {
    /// 0 is the current budget period, -1 the one before.
    pub period_offset: Option<i64>,
    /// Matches the note, category or account in any case and alphabet.
    pub query: Option<String>,
    pub account_id: Option<i64>,
    pub category_id: Option<i64>,
    pub limit: Option<u32>,
    pub today: Option<String>,
});
op!(TxList, "money.tx.list", TxListIn => TxPage, OpMeta::query(Scope::Global, 12, "One budget period's entries, newest first"));
payload!(#[schemars(rename = "MoneyTxAddIn")] TxAddIn {
    pub r#type: TxType, pub amount: i64, pub date: String, pub account_id: i64,
    /// Transfers only: the account the money goes to.
    pub to_account_id: Option<i64>,
    /// Expenses and income only.
    pub category_id: Option<i64>,
    pub note: Option<String>,
});
op!(TxAdd, "money.tx.add", TxAddIn => Tx, OpMeta::mutation(Scope::Global, 12, "Add an expense, income or transfer").emits(&["money.changed"]));
payload!(#[schemars(rename = "MoneyTxUpdateIn")] TxUpdateIn {
    pub id: i64, pub r#type: Option<TxType>, pub amount: Option<i64>, pub date: Option<String>,
    pub account_id: Option<i64>, pub to_account_id: Option<i64>, pub category_id: Option<i64>, pub note: Option<String>,
});
op!(TxUpdate, "money.tx.update", TxUpdateIn => Tx, OpMeta::mutation(Scope::Global, 12, "Change an entry; absent fields keep their value").emits(&["money.changed"]));
payload!(#[schemars(rename = "MoneyIdIn")] IdIn { pub id: i64 });
op!(TxDelete, "money.tx.delete", IdIn => Empty, OpMeta::mutation(Scope::Global, 12, "Delete an entry (kept as a tombstone for sync)").emits(&["money.changed"]));
op!(TxRestore, "money.tx.restore", IdIn => Tx, OpMeta::mutation(Scope::Global, 12, "Bring back a deleted entry: the Undo of money.tx.delete").emits(&["money.changed"]));
payload!(#[schemars(rename = "MoneyBudgetSetIn")] BudgetSetIn {
    /// Absent: the overall monthly budget.
    pub category_id: Option<i64>,
    /// Zero or less removes the budget.
    pub amount: i64,
});
op!(BudgetSet, "money.budget.set", BudgetSetIn => Empty, OpMeta::mutation(Scope::Global, 12, "Set or remove a monthly budget").emits(&["money.changed"]));
payload!(#[schemars(rename = "MoneyAccountAddIn")] AccountAddIn { pub name: String, pub r#type: AccountType, pub opening_balance: Option<i64> });
op!(AccountAdd, "money.account.add", AccountAddIn => AccountView,
    OpMeta::mutation(Scope::Global, 12, "Add an account; the first one also seeds Tally's default categories").emits(&["money.changed"]));
payload!(#[schemars(rename = "MoneySettingsSetIn")] SettingsSetIn {
    /// An ISO 4217 code, e.g. CAD.
    pub currency: Option<String>,
    /// The day the budget month starts on, 1 to 28.
    pub month_start_day: Option<i64>,
});
op!(SettingsSet, "money.settings.set", SettingsSetIn => Settings, OpMeta::mutation(Scope::Global, 12, "Change the currency or the day the budget month starts").actors(Actors::UserOnly).emits(&["money.changed"]));
payload!(#[schemars(rename = "MoneyPathIn")] PathIn {
    /// An absolute path on the PC.
    pub path: String,
});
result!(#[schemars(rename = "MoneyImportOut")] ImportOut {
    /// `backup`: a Tally backup, which replaced the ledger.
    pub kind: String, pub transactions: usize, pub accounts: usize,
});
op!(Import, "money.import", PathIn => ImportOut,
    OpMeta::mutation(Scope::Global, 12, "Restore a Tally backup (.json): it replaces the whole ledger, as a restore does on the phone").actors(Actors::UserOnly).emits(&["money.changed"]));
result!(#[schemars(rename = "MoneyExportOut")] ExportOut { pub path: String, pub transactions: usize });
op!(Export, "money.export", PathIn => ExportOut, OpMeta::mutation(Scope::Global, 12, "Write the ledger as a Tally backup the phone can restore"));
result!(#[schemars(rename = "MoneySampleOut")] SampleOut { pub transactions: usize });
op!(Sample, "money.sample", Empty => SampleOut,
    OpMeta::mutation(Scope::Global, 12, "Load Tally's sample household, only into an empty ledger; its accounts are named Sample").actors(Actors::UserOnly).emits(&["money.changed"]));
op!(Reset, "money.reset", Empty => Empty, OpMeta::mutation(Scope::Global, 12, "Erase the whole ledger").actors(Actors::UserOnly).emits(&["money.changed"]));

entries!(SummaryOp, ListsOp, TxList, TxAdd, TxUpdate, TxDelete, TxRestore, BudgetSet, AccountAdd, SettingsSet, Import, Export, Sample, Reset);
