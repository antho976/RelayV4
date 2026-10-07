//! The Money space's ledger across the bus door: what the desktop and, later, the phone call.

mod common;

use common::{call_as, code, engine, ok};
use relay_bus::Actor;
use serde_json::json;

#[test]
fn a_first_account_then_an_entry_reads_on_home() {
    let engine = engine();
    let empty = ok(&engine, "money.summary", json!({"today": "2026-10-07"}));
    assert_eq!(empty["empty"], true);
    let account = ok(&engine, "money.account.add", json!({"name": "Chequing", "type": "CHEQUING", "opening_balance": 100_000}));
    let lists = ok(&engine, "money.lists", json!({}));
    let groceries = lists["categories"].as_array().unwrap().iter().find(|c| c["name"] == "Groceries").unwrap()["id"].clone();
    ok(&engine, "money.budget.set", json!({"amount": 310_000}));
    let tx = ok(&engine, "money.tx.add", json!({
        "type": "EXPENSE", "amount": 4_250, "date": "2026-10-05", "account_id": account["id"], "category_id": groceries, "note": "Metro"
    }));
    assert_eq!(tx["category"], "Groceries");
    let home = ok(&engine, "money.summary", json!({"today": "2026-10-07"}));
    assert_eq!(home["empty"], false);
    assert_eq!(home["spent"], 4_250);
    assert_eq!(home["accounts"][0]["balance"], 95_750);
    assert_eq!(home["pace"]["status"], "UNDER_PACE");
    assert!(home["lines"]["margin"].as_str().unwrap().contains("a day for 25 days"), "{}", home["lines"]["margin"]);
    let page = ok(&engine, "money.tx.list", json!({"query": "metro", "today": "2026-10-07"}));
    assert_eq!(page["transactions"].as_array().unwrap().len(), 1);

    ok(&engine, "money.tx.delete", json!({"id": tx["id"]}));
    assert_eq!(ok(&engine, "money.summary", json!({"today": "2026-10-07"}))["spent"], 0);
    ok(&engine, "money.tx.restore", json!({"id": tx["id"]}));
    assert_eq!(ok(&engine, "money.summary", json!({"today": "2026-10-07"}))["spent"], 4_250);
}

#[test]
fn bad_entries_are_refused_with_a_reason() {
    let engine = engine();
    ok(&engine, "money.account.add", json!({"name": "Cash", "type": "CASH"}));
    let refused = call_as(&engine, Actor::User, "money.tx.add", json!({"type": "EXPENSE", "amount": 0, "date": "2026-10-05", "account_id": 1}));
    assert_eq!(code(refused), "money.invalid");
    let unknown = call_as(&engine, Actor::User, "money.tx.add", json!({"type": "EXPENSE", "amount": 5, "date": "2026-10-05", "account_id": 1, "colour": 3}));
    assert_eq!(code(unknown), "bus.schema");
}

#[test]
fn a_tally_backup_restores_and_exports_the_same_ledger() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let sample = ok(&engine, "money.sample", json!({}));
    assert!(sample["transactions"].as_u64().unwrap() > 50);
    let out = dir.path().join("tally-backup.json");
    let exported = ok(&engine, "money.export", json!({"path": out}));
    assert_eq!(exported["transactions"], sample["transactions"]);
    ok(&engine, "money.reset", json!({}));
    assert_eq!(ok(&engine, "money.summary", json!({}))["empty"], true);
    let imported = ok(&engine, "money.import", json!({"path": out}));
    assert_eq!(imported["kind"], "backup");
    assert_eq!(imported["transactions"], sample["transactions"]);
    let again = dir.path().join("again.json");
    ok(&engine, "money.export", json!({"path": again}));
    let strip = |p: &std::path::Path| {
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        v["exportedAt"] = json!(null);
        v
    };
    assert_eq!(strip(&out), strip(&again));
}

#[test]
fn an_agent_may_add_entries_but_never_wipe_the_ledger() {
    let engine = engine();
    ok(&engine, "money.account.add", json!({"name": "Cash", "type": "CASH"}));
    let agent = Actor::agent("brisk-otter");
    let add = call_as(&engine, agent.clone(), "money.tx.add", json!({"type": "INCOME", "amount": 500, "date": "2026-10-05", "account_id": 1}));
    assert!(add.error.is_none(), "{:?}", add.error);
    for op in ["money.reset", "money.sample"] {
        let r = call_as(&engine, agent.clone(), op, json!({}));
        assert!(r.error.is_some(), "{op} must refuse an agent");
    }
    assert!(call_as(&engine, agent, "money.import", json!({"path": "/tmp/x.json"})).error.is_some());
}
