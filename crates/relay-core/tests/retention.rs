//! Retention, undo staleness and backups: the reconcile pass (RA-132), audit.undo's checks
//! (RA-133, RA-172), audit.list bounds (RA-085) and app.backup.now (RA-128).

mod common;

use common::{call, code, committed_repo, engine_with_project, ok, wait_until};
use relay_core::engine::Engine;
use serde_json::json;
use std::sync::Arc;

/// The id of the newest successful audit row for `op`.
fn last_audit(engine: &Engine, op: &str) -> i64 {
    let rows = ok(engine, "audit.list", json!({"op_prefix": op, "limit": 1}));
    rows["rows"][0]["id"].as_i64().unwrap_or_else(|| panic!("no audit row for {op}"))
}

struct Fixture {
    root: tempfile::TempDir,
    engine: Arc<Engine>,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let ws = root.path().join("ws");
    let repo = ws.join("app");
    committed_repo(&repo, &[("README.md", "retention\n")]);
    let engine = engine_with_project(root.path(), &ws, &repo);
    Fixture { root, engine }
}

#[test]
fn workspace_and_session_updates_can_be_undone() {
    let f = fixture();
    ok(&f.engine, "workspace.update", json!({"workspace_id": 1, "name": "renamed"}));
    let undone = ok(&f.engine, "audit.undo", json!({"audit_id": last_audit(&f.engine, "workspace.update")}));
    assert!(undone["undone"].is_i64());
    let list = ok(&f.engine, "workspace.list", json!({}));
    assert_ne!(list["workspaces"][0]["name"], "renamed", "{list}");
}

#[test]
fn a_settings_undo_never_clobbers_a_later_write_to_the_same_setting() {
    let f = fixture();
    ok(&f.engine, "settings.set", json!({"path": "parking.idle_minutes", "value": 10}));
    let first = last_audit(&f.engine, "settings.set");
    ok(&f.engine, "settings.set", json!({"path": "parking.idle_minutes", "value": 20}));
    assert_eq!(code(call(&f.engine, "audit.undo", json!({"audit_id": first}))), "audit.stale");
    // An unrelated path is no reason to refuse.
    ok(&f.engine, "settings.set", json!({"path": "undo.grace_days", "value": 3}));
    let second = ok(&f.engine, "audit.list", json!({"op_prefix": "settings.set", "limit": 2}))["rows"][1]["id"].as_i64().unwrap();
    ok(&f.engine, "audit.undo", json!({"audit_id": second}));
    assert_eq!(ok(&f.engine, "settings.get", json!({"path": "parking.idle_minutes"}))["value"], 10);
    // force still overrides.
    ok(&f.engine, "audit.undo", json!({"audit_id": first, "force": true}));
}

#[test]
fn undoing_a_notification_settings_patch_removes_the_keys_it_added() {
    let f = fixture();
    ok(&f.engine, "notify.settings.set", json!({"patch": {"categories": {"disk": false}, "sound": "bell"}}));
    ok(&f.engine, "audit.undo", json!({"audit_id": last_audit(&f.engine, "notify.settings.set")}));
    let now = ok(&f.engine, "notify.settings.get", json!({}));
    let value = now.get("value").unwrap_or(&now);
    assert_eq!(value["sound"], "chime", "{now}");
    assert!(value["categories"].get("disk").is_none(), "the added key survived the undo: {now}");
}

#[test]
fn audit_list_bounds_accept_offsets_and_refuse_garbage() {
    let f = fixture();
    let all = ok(&f.engine, "audit.list", json!({}))["rows"].as_array().unwrap().len();
    assert!(all > 0);
    // The same instant written with an offset two hours ahead of UTC.
    let past = ok(&f.engine, "audit.list", json!({"since": "2000-01-01T02:00:00+02:00"}));
    assert_eq!(past["rows"].as_array().unwrap().len(), all);
    let future = ok(&f.engine, "audit.list", json!({"since": "2999-01-01 00:00:00Z"}));
    assert!(future["rows"].as_array().unwrap().is_empty());
    assert_eq!(code(call(&f.engine, "audit.list", json!({"since": "last tuesday"}))), "time.invalid");
}

#[test]
fn reconcile_purges_rows_past_their_window_and_keeps_recent_ones() {
    let f = fixture();
    let old = ok(&f.engine, "task.create", json!({"project_id": 1, "title": "old"}));
    let fresh = ok(&f.engine, "task.create", json!({"project_id": 1, "title": "fresh"}));
    let child = ok(&f.engine, "task.create", json!({"project_id": 1, "title": "child", "parent_id": old["id"]}));
    ok(&f.engine, "task.delete", json!({"task_id": old["id"]}));
    ok(&f.engine, "task.delete", json!({"task_id": fresh["id"]}));
    {
        let conn = f.engine.store.lock();
        conn.execute("UPDATE tasks SET deleted_at='2000-01-01T00:00:00Z' WHERE id=?1", [old["id"].as_i64().unwrap()]).unwrap();
        // A child the delete left live would otherwise keep a dangling parent edge.
        conn.execute("UPDATE tasks SET deleted_at=NULL WHERE id=?1", [child["id"].as_i64().unwrap()]).unwrap();
        conn.execute_batch(
            "INSERT INTO notifications(project_id,category,title,body,read,created_at) VALUES
               (NULL,'system','read long ago','',1,'2000-01-01T00:00:00Z'),
               (NULL,'system','unread long ago','',0,'2000-01-01T00:00:00Z'),
               (NULL,'system','read today','',1,'2999-01-01T00:00:00Z');
             INSERT INTO holds(actor,op,envelope,policy,details,state,created_at,resolved_at) VALUES
               ('agent:x','file.write','{}','destructive_write','{}','rejected','2000-01-01T00:00:00Z','2000-01-01T00:00:00Z'),
               ('agent:x','file.write','{}','destructive_write','{}','open','2000-01-01T00:00:00Z',NULL);",
        ).unwrap();
    }
    let out = ok(&f.engine, "app.reconcile", json!({}));
    let actions = out["actions"].as_array().unwrap();
    assert!(actions.iter().any(|a| a == "purged 1 deleted task(s) past the undo window"), "{out}");
    assert!(actions.iter().any(|a| a == "purged 2 old notification(s)"), "{out}");
    assert!(actions.iter().any(|a| a == "purged 1 answered guardrail hold(s)"), "an open hold is never pruned: {out}");
    let conn = f.engine.store.lock();
    let tasks: Vec<i64> = conn.prepare("SELECT id FROM tasks ORDER BY id").unwrap()
        .query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(tasks, vec![fresh["id"].as_i64().unwrap(), child["id"].as_i64().unwrap()]);
    let parent: Option<i64> = conn.query_row("SELECT parent_id FROM tasks WHERE id=?1", [child["id"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    assert_eq!(parent, None);
    let left: Vec<String> = conn.prepare("SELECT title FROM notifications").unwrap()
        .query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(left, vec!["read today"]);
    drop(conn);
    // Nothing left to do: the pass is idempotent.
    assert!(ok(&f.engine, "app.reconcile", json!({}))["actions"].as_array().unwrap().is_empty());
}

#[test]
fn reconcile_removes_expired_trash_from_disk() {
    let f = fixture();
    let repo = f.root.path().join("ws/app");
    std::fs::write(repo.join("gone.txt"), "bye\n").unwrap();
    let trash = ok(&f.engine, "file.delete", json!({"project_id": 1, "path": "gone.txt"}));
    let dir = repo.join(".relay/trash").join(trash["trash_id"].as_i64().unwrap().to_string());
    assert!(dir.join("payload").exists());
    f.engine.store.lock().execute("UPDATE file_trash SET created_at='2000-01-01T00:00:00Z'", []).unwrap();
    ok(&f.engine, "app.reconcile", json!({}));
    wait_until("the trash directory to be removed", || !dir.exists());
    assert!(repo.join("README.md").exists());
}

#[test]
fn backup_now_runs_outside_the_transaction_and_copies_the_store() {
    let f = fixture();
    ok(&f.engine, "task.create", json!({"project_id": 1, "title": "kept in the backup"}));
    let out = ok(&f.engine, "app.backup.now", json!({}));
    let path = out["path"].as_str().unwrap();
    let copy = rusqlite::Connection::open(path).unwrap();
    let title: String = copy.query_row("SELECT title FROM tasks", [], |r| r.get(0)).unwrap();
    assert_eq!(title, "kept in the backup");
    // A connection with uncommitted writes fails fast instead of spinning forever.
    let mut conn = f.engine.store.lock();
    let tx = conn.transaction().unwrap();
    tx.execute("UPDATE tasks SET title='pending'", []).unwrap();
    assert!(f.engine.store.backup_with(&tx, "pending").is_err());
}
