//! `money.*` — the Money space's ledger (docs/MONEY.md).
//!
//! The ledger is its own SQLite file, `money.db` beside the store, behind its own mutex: money has
//! its own schema versions and is what the phone will sync with. Its work is local and short, so
//! queries run unlocked and mutations inside the store's transaction like any other op; a money
//! write is committed in `money.db` whether or not the store transaction around it commits.

use crate::engine::{Engine, IntoBus};
use relay_bus::ops::money::*;
use relay_bus::{BusError, Empty};
use relay_money::backup::{self, BackupReadResult};
use relay_money::ledger::{parse_date, Ledger, LedgerError, TxInput, TxPatch, TxQuery};
use relay_money::money::Locale;
use serde_json::json;
use std::path::Path;

/// A backup bigger than this is not one Tally wrote.
const MAX_BACKUP_BYTES: u64 = 64 << 20;

fn bus(e: LedgerError) -> BusError {
    match e {
        LedgerError::Invalid(m) => BusError::invalid("money.invalid", m),
        LedgerError::NotFound(m) => BusError::not_found("money.not_found", m),
        LedgerError::Sql(e) => BusError::internal(format!("money ledger: {e}")),
    }
}

/// Runs `f` on the ledger, opening `money.db` beside the store on first use (in memory when the
/// store is, as in tests).
fn ledger<T>(engine: &Engine, f: impl FnOnce(&mut Ledger) -> Result<T, LedgerError>) -> Result<T, BusError> {
    let mut slot = engine.money.lock().unwrap_or_else(|p| p.into_inner());
    if slot.is_none() {
        let store = engine.store.path();
        let opened = if store == Path::new(":memory:") {
            Ledger::open_in_memory()
        } else {
            Ledger::open(&store.with_file_name("money.db"))
        };
        *slot = Some(opened.map_err(bus)?);
    }
    f(slot.as_mut().expect("opened above")).map_err(bus)
}

fn today(day: Option<&str>) -> Result<jiff::civil::Date, BusError> {
    match day {
        Some(d) => parse_date(d).ok_or_else(|| BusError::invalid("money.invalid", format!("Not a date: {d}"))),
        None => Ok(jiff::Zoned::now().date()),
    }
}

fn absolute(path: &str) -> Result<&Path, BusError> {
    let p = Path::new(path);
    if p.is_absolute() {
        Ok(p)
    } else {
        Err(BusError::invalid("money.path", "Give the file's full path"))
    }
}

pub fn register(e: &mut Engine) {
    e.register_unlocked::<SummaryOp>(|ctx, p| {
        let day = today(p.today.as_deref())?;
        let (posted, summary) = ledger(ctx.engine(), |l| Ok((l.post_due(day)?, l.summary(day, &Locale::from_env())?)))?;
        // Bills that came due are entries now; whoever else is drawing the ledger re-reads it.
        if posted > 0 {
            ctx.emit("money.changed", json!({"posted": posted}));
        }
        Ok(summary)
    });
    e.register_unlocked::<ListsOp>(|ctx, _: Empty| ledger(ctx.engine(), |l| l.lists()));
    e.register_unlocked::<TxList>(|ctx, p| {
        let day = today(p.today.as_deref())?;
        let q = TxQuery { period_offset: p.period_offset, query: p.query, account_id: p.account_id, category_id: p.category_id, limit: p.limit };
        ledger(ctx.engine(), |l| l.tx_list(&q, day))
    });
    e.register::<TxAdd>(|ctx, p| {
        let input = TxInput {
            r#type: p.r#type, amount: p.amount, date: p.date, account_id: p.account_id,
            to_account_id: p.to_account_id, category_id: p.category_id, note: p.note,
        };
        let tx = ledger(ctx.engine(), |l| l.tx_add(&input))?;
        ctx.emit("money.changed", json!({"tx": tx.id}));
        Ok(tx)
    });
    e.register::<TxUpdate>(|ctx, p| {
        let patch = TxPatch {
            r#type: p.r#type, amount: p.amount, date: p.date, account_id: p.account_id,
            to_account_id: p.to_account_id, category_id: p.category_id, note: p.note,
        };
        let tx = ledger(ctx.engine(), |l| l.tx_update(p.id, &patch))?;
        ctx.emit("money.changed", json!({"tx": tx.id}));
        Ok(tx)
    });
    e.register::<TxDelete>(|ctx, p| {
        ledger(ctx.engine(), |l| l.tx_delete(p.id))?;
        ctx.emit("money.changed", json!({"tx": p.id, "deleted": true}));
        Ok(Empty {})
    });
    e.register::<TxRestore>(|ctx, p| {
        let tx = ledger(ctx.engine(), |l| l.tx_restore(p.id))?;
        ctx.emit("money.changed", json!({"tx": tx.id}));
        Ok(tx)
    });
    e.register::<BudgetSet>(|ctx, p| {
        ledger(ctx.engine(), |l| l.budget_set(p.category_id, p.amount))?;
        ctx.emit("money.changed", json!({}));
        Ok(Empty {})
    });
    e.register::<AccountAdd>(|ctx, p| {
        let account = ledger(ctx.engine(), |l| l.account_add(&p.name, p.r#type, p.opening_balance.unwrap_or(0)))?;
        ctx.emit("money.changed", json!({}));
        Ok(account)
    });
    e.register::<SettingsSet>(|ctx, p| {
        let settings = ledger(ctx.engine(), |l| {
            if let Some(c) = p.currency.as_deref() {
                l.set_currency(c.trim())?;
            }
            if let Some(d) = p.month_start_day {
                l.set_month_start_day(d)?;
            }
            l.settings()
        })?;
        ctx.emit("money.changed", json!({}));
        Ok(settings)
    });
    e.register::<Import>(|ctx, p| {
        let path = absolute(&p.path)?;
        let size = std::fs::metadata(path).map_err(|e| BusError::invalid("money.path", format!("{}: {e}", path.display())))?.len();
        if size > MAX_BACKUP_BYTES {
            return Err(BusError::invalid("money.import", "That file is too big to be a Tally backup"));
        }
        let text = std::fs::read_to_string(path).bus()?;
        let file = match backup::decode(&text) {
            BackupReadResult::Ok(file) => file,
            BackupReadResult::Invalid(why) => return Err(BusError::invalid("money.import", why)),
        };
        let accounts = file.accounts.len();
        let transactions = ledger(ctx.engine(), |l| l.import_backup(&file))?;
        ctx.emit("money.changed", json!({"imported": transactions}));
        Ok(ImportOut { kind: "backup".into(), transactions, accounts })
    });
    e.register::<Export>(|ctx, p| {
        let path = absolute(&p.path)?;
        let file = ledger(ctx.engine(), |l| l.export_backup(&crate::time::now()))?;
        std::fs::write(path, backup::encode(&file)).bus()?;
        Ok(ExportOut { path: path.display().to_string(), transactions: file.transactions.len() })
    });
    e.register::<Sample>(|ctx, _: Empty| {
        let day = today(None)?;
        let transactions = ledger(ctx.engine(), |l| {
            let currency = l.settings()?.currency;
            l.load_sample(day, &currency)
        })?;
        ctx.emit("money.changed", json!({}));
        Ok(SampleOut { transactions })
    });
    e.register::<Reset>(|ctx, _: Empty| {
        ledger(ctx.engine(), |l| l.reset())?;
        ctx.emit("money.changed", json!({}));
        Ok(Empty {})
    });
}
