//! Avex's training history across the bus: importing its export, the readings, what a wrong file
//! says, and what an agent may not do.

mod common;

use common::{call_as, code, engine, ok};
use relay_bus::Actor;
use serde_json::json;

const EXPORT: &str = include_str!("../../relay-gym/tests/avex_export.json");

#[test]
fn an_export_comes_in_and_reads_back() {
    let engine = engine();
    let empty = ok(&engine, "gym.summary", json!({}));
    assert!(empty["imported"].is_null(), "nothing imported yet");
    assert_eq!(empty["totals"]["sessions"], 0);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("avex_export.json");
    std::fs::write(&path, EXPORT).unwrap();
    let imported = ok(&engine, "gym.import", json!({"path": path}));
    assert_eq!(imported["exported_at"], "2026-10-08 21:14");
    let sessions = imported["sessions"].as_u64().unwrap();
    assert!(sessions > 40, "{imported}");

    let summary = ok(&engine, "gym.summary", json!({}));
    assert_eq!(summary["imported"]["file"], "avex_export.json");
    assert_eq!(summary["unit"], "lb");
    // One workout in the sample is untracked: listed, not counted.
    assert_eq!(summary["totals"]["sessions"].as_u64().unwrap(), sessions - 1);
    assert_eq!(summary["goals"].as_array().unwrap().len(), 1, "the met goal is not open");

    let page = ok(&engine, "gym.sessions", json!({"limit": 3}));
    assert_eq!(page["sessions"].as_array().unwrap().len(), 3);
    assert_eq!(page["total"].as_u64().unwrap(), sessions);
    let id = page["sessions"][0]["id"].as_i64().unwrap();
    let detail = ok(&engine, "gym.session.get", json!({"id": id}));
    assert_eq!(detail["row"]["id"], id);
    assert!(!detail["exercises"].as_array().unwrap().is_empty());

    let lifts = ok(&engine, "gym.lifts", json!({"query": "bench"}));
    assert_eq!(lifts["lifts"][0]["name"], "Bench Press");
    let bench = ok(&engine, "gym.lift.get", json!({"name": "bench press", "limit": 5}));
    assert_eq!(bench["history"].as_array().unwrap().len(), 5);
    assert_eq!(code(call_as(&engine, Actor::User, "gym.lift.get", json!({"name": "press"}))), "gym.not_found", "several lifts say press");

    let weekly = ok(&engine, "gym.series", json!({"measure": "sessions", "periods": 8}));
    assert_eq!(weekly["labels"].as_array().unwrap().len(), 8);
    assert_eq!(weekly["unit"], "sessions");
    let e1rm = ok(&engine, "gym.series", json!({"measure": "e1rm", "by": "session", "lift": "Back Squat", "periods": 52}));
    assert_eq!(e1rm["lift"], "Back Squat");
    assert_eq!(code(call_as(&engine, Actor::User, "gym.series", json!({"measure": "e1rm"}))), "gym.invalid");
    assert!(!ok(&engine, "gym.cardio", json!({}))["entries"].as_array().unwrap().is_empty());

    // Importing again replaces, never doubles.
    ok(&engine, "gym.import", json!({"path": path}));
    assert_eq!(ok(&engine, "gym.sessions", json!({"limit": 1}))["total"].as_u64().unwrap(), sessions);

    ok(&engine, "gym.reset", json!({}));
    assert!(ok(&engine, "gym.summary", json!({}))["imported"].is_null());
}

#[test]
fn a_wrong_file_says_which_one_to_send() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let weekly = dir.path().join("avex_weekly_export.json");
    std::fs::write(&weekly, r#"{"exportedAt": "x", "periodStart": "2026-10-05", "periodDays": 7, "sessions": [], "cardio": []}"#).unwrap();
    let r = call_as(&engine, Actor::User, "gym.import", json!({"path": weekly}));
    let e = r.error.expect("refused");
    assert_eq!(e.code, "gym.import");
    assert!(e.message.contains("Training history"), "{}", e.message);
    let tally = dir.path().join("tally.json");
    std::fs::write(&tally, r#"{"accounts": [], "transactions": []}"#).unwrap();
    assert_eq!(code(call_as(&engine, Actor::User, "gym.import", json!({"path": tally}))), "gym.import");
    assert_eq!(code(call_as(&engine, Actor::User, "gym.import", json!({"path": "avex_export.json"}))), "gym.path");
    assert!(ok(&engine, "gym.summary", json!({}))["imported"].is_null(), "nothing came in");
}

#[test]
fn an_agent_reads_but_never_imports_or_forgets() {
    let engine = engine();
    let agent = Actor::agent("brisk-otter");
    assert!(call_as(&engine, agent.clone(), "gym.summary", json!({})).error.is_none());
    for (op, payload) in [("gym.import", json!({"path": "/tmp/avex_export.json"})), ("gym.reset", json!({}))] {
        assert!(call_as(&engine, agent.clone(), op, payload).error.is_some(), "{op} must refuse an agent");
    }
}
