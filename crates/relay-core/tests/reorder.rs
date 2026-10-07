//! `project.reorder` / `workspace.reorder` (RA-502): a sidebar drag is one transaction, one
//! undo entry and one event, however many rows it renumbers.

mod common;

use common::{call, code, engine, init_repo, ok};
use relay_core::engine::Engine;
use serde_json::{json, Value};

/// Three projects in workspace 1, three workspaces in all.
fn sidebar(engine: &Engine) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for w in ["a", "b", "c"] {
        let ws = root.path().join(w);
        std::fs::create_dir_all(&ws).unwrap();
        ok(engine, "workspace.create", json!({"path": ws}));
    }
    for name in ["one", "two", "three"] {
        let repo = root.path().join("a").join(name);
        init_repo(&repo);
        ok(engine, "project.add", json!({"workspace_id": 1, "path": repo}));
    }
    root
}

fn orders(engine: &Engine, op: &str, list: &str) -> Vec<(i64, i64)> {
    let mut rows: Vec<(i64, i64)> = ok(engine, op, json!({}))[list].as_array().unwrap().iter()
        .map(|row| (row["id"].as_i64().unwrap(), row["order"].as_i64().unwrap()))
        .collect();
    rows.sort();
    rows
}

fn last_audit(engine: &Engine, op: &str) -> Value {
    ok(engine, "audit.list", json!({"op_prefix": op, "limit": 1}))["rows"][0].clone()
}

#[test]
fn project_reorder_is_one_write_one_undo_and_one_event() {
    let e = engine();
    let _root = sidebar(&e);
    let before = orders(&e, "project.list", "projects");
    assert_eq!(before, vec![(1, 0), (2, 1), (3, 2)], "project.add appends");
    let audits = ok(&e, "audit.list", json!({"limit": 1000}))["rows"].as_array().unwrap().len();
    let mut events = e.subscribe();

    let out = ok(&e, "project.reorder", json!({"orders": [
        {"project_id": 3, "order": 0}, {"project_id": 1, "order": 1}, {"project_id": 2, "order": 2},
    ]}));
    assert_eq!(out["projects"].as_array().unwrap().len(), 3, "{out}");
    assert_eq!(orders(&e, "project.list", "projects"), vec![(1, 1), (2, 2), (3, 0)]);
    let listed: Vec<i64> = ok(&e, "project.list", json!({"workspace_id": 1}))["projects"].as_array().unwrap()
        .iter().map(|p| p["id"].as_i64().unwrap()).collect();
    assert_eq!(listed, vec![3, 1, 2]);

    let fired: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    let changed: Vec<_> = fired.iter().filter(|ev| ev.ev.starts_with("project.")).collect();
    assert_eq!(changed.len(), 1, "one event for the whole drag: {fired:?}");
    assert_eq!(changed[0].ev, "project.changed");
    assert_eq!(changed[0].payload["projects"].as_array().unwrap().len(), 3);
    assert_eq!(ok(&e, "audit.list", json!({"limit": 1000}))["rows"].as_array().unwrap().len(), audits + 1, "one audit row");

    // One undo puts every project back.
    let row = last_audit(&e, "project.reorder");
    ok(&e, "audit.undo", json!({"audit_id": row["id"]}));
    assert_eq!(orders(&e, "project.list", "projects"), before);
}

#[test]
fn project_reorder_refuses_unknown_empty_and_duplicate_ids_writing_nothing() {
    let e = engine();
    let _root = sidebar(&e);
    let before = orders(&e, "project.list", "projects");
    // The unknown id comes last, after two rows the handler has already read.
    let refused = call(&e, "project.reorder", json!({"orders": [
        {"project_id": 1, "order": 9}, {"project_id": 2, "order": 8}, {"project_id": 99, "order": 7},
    ]}));
    assert_eq!(refused.error.as_ref().unwrap().kind, relay_bus::ErrorKind::NotFound);
    assert_eq!(code(refused), "project.not_found");
    assert_eq!(orders(&e, "project.list", "projects"), before, "rolled back");
    assert_eq!(code(call(&e, "project.reorder", json!({"orders": []}))), "project.orders");
    assert_eq!(code(call(&e, "project.reorder", json!({"orders": [
        {"project_id": 1, "order": 2}, {"project_id": 1, "order": 0},
    ]}))), "project.orders");
    assert_eq!(orders(&e, "project.list", "projects"), before);
    // Agents cannot reorder the sidebar.
    let agent = common::call_as(&e, relay_bus::Actor::Agent("brisk-otter".into()), "project.reorder",
        json!({"orders": [{"project_id": 1, "order": 2}]}));
    assert!(agent.error.is_some());
}

#[test]
fn workspace_reorder_undoes_in_one_step_and_refuses_a_stale_undo() {
    let e = engine();
    let _root = sidebar(&e);
    let before = orders(&e, "workspace.list", "workspaces");
    assert_eq!(before, vec![(1, 0), (2, 1), (3, 2)]);
    let mut events = e.subscribe();
    ok(&e, "workspace.reorder", json!({"orders": [
        {"workspace_id": 1, "order": 2}, {"workspace_id": 3, "order": 0},
    ]}));
    let row = last_audit(&e, "workspace.reorder");
    assert_eq!(orders(&e, "workspace.list", "workspaces"), vec![(1, 2), (2, 1), (3, 0)], "unnamed rows keep their order");
    let fired: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).filter(|ev| ev.ev.starts_with("workspace.")).collect();
    assert_eq!(fired.len(), 1, "{fired:?}");
    assert_eq!(fired[0].payload["workspaces"].as_array().unwrap().len(), 2);

    assert_eq!(code(call(&e, "workspace.reorder", json!({"orders": [{"workspace_id": 7, "order": 0}]}))), "workspace.not_found");
    assert_eq!(code(call(&e, "workspace.reorder", json!({"orders": []}))), "workspace.orders");
    assert_eq!(code(call(&e, "workspace.reorder", json!({"orders": [
        {"workspace_id": 2, "order": 0}, {"workspace_id": 2, "order": 1},
    ]}))), "workspace.orders");

    // A later edit to one of the reordered rows makes the undo stale, as for workspace.update.
    ok(&e, "workspace.update", json!({"workspace_id": 3, "name": "renamed"}));
    assert_eq!(code(call(&e, "audit.undo", json!({"audit_id": row["id"]}))), "audit.stale");
    ok(&e, "audit.undo", json!({"audit_id": row["id"], "force": true}));
    assert_eq!(orders(&e, "workspace.list", "workspaces"), before);
    // The undo is itself undoable: redo.
    let undo = last_audit(&e, "audit.undo");
    ok(&e, "audit.undo", json!({"audit_id": undo["id"]}));
    assert_eq!(orders(&e, "workspace.list", "workspaces"), vec![(1, 2), (2, 1), (3, 0)]);
}
