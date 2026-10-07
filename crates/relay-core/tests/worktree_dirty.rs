//! RA-405: a removal that would delete uncommitted work in a Relay-pool worktree is refused
//! `worktree.dirty` unless the caller passes `discard_changes`, and nothing is closed, undone or
//! deleted by the refused request. `session.close`, `session.discard_restorable`,
//! `project.remove` and `workspace.remove`, all through the bus.

use relay_bus::{Actor, BusError, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn call(e: &Engine, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}
fn ok(e: &Engine, op: &str, payload: Value) -> Value {
    call(e, op, payload).into_result().unwrap_or_else(|err| panic!("{op} failed: {} {}", err.code, err.message))
}
fn refused(e: &Engine, op: &str, payload: Value) -> BusError {
    call(e, op, payload).into_result().expect_err(&format!("{op} was expected to refuse"))
}

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

struct Fixture {
    _tmp: tempfile::TempDir,
    engine: Arc<Engine>,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let ws = root.join("ws");
    let repo = ws.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README.md"), "hi\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let engine = Engine::new(Instance::Test, Store::open(&root.join("store/store.db"), false).unwrap());
    ok(&engine, "workspace.create", json!({"path": ws}));
    ok(&engine, "project.add", json!({"workspace_id": 1, "path": repo}));
    Fixture { _tmp: tmp, engine }
}

/// A session on its own pooled checkout: its name and that checkout.
fn session(f: &Fixture) -> (String, PathBuf) {
    let s = ok(&f.engine, "session.create", json!({"project_id": 1, "provider": "claude"}));
    (s["name"].as_str().unwrap().to_string(), PathBuf::from(s["worktree"].as_str().unwrap()))
}

/// One untracked file and one edited tracked file: two changes.
fn dirty(wt: &Path) {
    std::fs::write(wt.join("new.txt"), "unsaved\n").unwrap();
    std::fs::write(wt.join("README.md"), "edited\n").unwrap();
}

/// Read from the store: `session.get` answers only for sessions that are not closed.
fn state(f: &Fixture, name: &str) -> String {
    f.engine.store.lock()
        .query_row("SELECT state FROM sessions WHERE name=?1 ORDER BY id DESC LIMIT 1", [name], |row| row.get(0))
        .unwrap()
}

fn restorable(f: &Fixture, name: &str) {
    f.engine.store.lock().execute("UPDATE sessions SET state='restorable' WHERE name=?1", [name]).unwrap();
}

fn wait_gone(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while path.exists() {
        assert!(Instant::now() < deadline, "{} was never removed", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_dirty(error: &BusError, wt: &Path, changed: i64) {
    assert_eq!(error.code, "worktree.dirty", "{}", error.message);
    let details = error.details.as_ref().expect("details");
    assert_eq!(details["worktrees"][0]["worktree"], wt.display().to_string(), "{details}");
    assert_eq!(details["worktrees"][0]["changed"], changed, "{details}");
    assert!(error.hint.as_deref().unwrap_or("").contains("discard_changes"));
}

#[test]
fn close_refuses_a_dirty_pooled_worktree_unless_told_to_discard() {
    let f = fixture();
    let (name, wt) = session(&f);
    dirty(&wt);
    let error = refused(&f.engine, "session.close", json!({"session": name}));
    assert_dirty(&error, &wt, 2);
    assert_eq!(error.details.as_ref().unwrap()["worktrees"][0]["sessions"], json!([name]));
    assert!(wt.join("new.txt").exists(), "the refused close deleted the work");
    assert_eq!(state(&f, &name), "created", "the refused close still closed the session");

    // Keeping the checkout is never refused.
    let (kept, kept_wt) = session(&f);
    dirty(&kept_wt);
    ok(&f.engine, "session.close", json!({"session": kept, "remove_worktree": false}));
    assert!(kept_wt.join("new.txt").exists());

    ok(&f.engine, "session.close", json!({"session": name, "discard_changes": true}));
    assert!(!wt.exists(), "discard_changes removes the checkout");
    assert_eq!(state(&f, &name), "closed");

    let (clean, clean_wt) = session(&f);
    ok(&f.engine, "session.close", json!({"session": clean}));
    assert!(!clean_wt.exists(), "a clean checkout closes without the flag");
}

#[test]
fn close_skips_a_checkout_that_is_already_gone() {
    let f = fixture();
    let (name, wt) = session(&f);
    std::fs::remove_dir_all(&wt).unwrap();
    ok(&f.engine, "session.close", json!({"session": name}));
    assert_eq!(state(&f, &name), "closed");
}

#[test]
fn discard_restorable_refuses_a_dirty_pooled_worktree_unless_told_to_discard() {
    let f = fixture();
    let (name, wt) = session(&f);
    restorable(&f, &name);
    std::fs::write(wt.join("new.txt"), "unsaved\n").unwrap();
    let error = refused(&f.engine, "session.discard_restorable", json!({"session": name}));
    assert_dirty(&error, &wt, 1);
    assert!(wt.join("new.txt").exists());
    assert_eq!(state(&f, &name), "restorable");

    ok(&f.engine, "session.discard_restorable", json!({"session": name, "discard_changes": true}));
    assert!(!wt.exists());
    assert_eq!(state(&f, &name), "closed");

    let (clean, clean_wt) = session(&f);
    restorable(&f, &clean);
    ok(&f.engine, "session.discard_restorable", json!({"session": clean}));
    assert!(!clean_wt.exists());
}

#[test]
fn project_remove_refuses_before_closing_anything_when_a_checkout_it_deletes_is_dirty() {
    let f = fixture();
    let (a, a_wt) = session(&f);
    let (b, b_wt) = session(&f);
    dirty(&b_wt);
    let remove = json!({"project_id": 1, "force": true, "remove_worktrees": true});
    let error = refused(&f.engine, "project.remove", remove.clone());
    assert_dirty(&error, &b_wt, 2);
    assert_eq!(error.details.as_ref().unwrap()["worktrees"].as_array().unwrap().len(), 1, "only the dirty one is named");
    assert_eq!((state(&f, &a), state(&f, &b)), ("created".into(), "created".into()));
    assert!(a_wt.exists() && b_wt.join("new.txt").exists());
    ok(&f.engine, "project.get", json!({"project_id": 1}));

    let mut discard = remove.clone();
    discard["discard_changes"] = json!(true);
    ok(&f.engine, "project.remove", discard);
    wait_gone(&a_wt);
    wait_gone(&b_wt);
}

#[test]
fn project_remove_deletes_clean_checkouts_without_the_flag_and_keeps_them_without_remove_worktrees() {
    let f = fixture();
    let (_, wt) = session(&f);
    ok(&f.engine, "project.remove", json!({"project_id": 1, "force": true, "remove_worktrees": true}));
    wait_gone(&wt);

    let f = fixture();
    let (_, wt) = session(&f);
    dirty(&wt);
    ok(&f.engine, "project.remove", json!({"project_id": 1, "force": true}));
    assert!(wt.join("new.txt").exists(), "a removal that keeps checkouts is never refused");
}

#[test]
fn workspace_remove_refuses_a_dirty_checkout_it_would_delete() {
    let f = fixture();
    let (name, wt) = session(&f);
    std::fs::write(wt.join("new.txt"), "unsaved\n").unwrap();
    let remove = json!({"workspace_id": 1, "force": true, "remove_worktrees": true});
    assert_dirty(&refused(&f.engine, "workspace.remove", remove.clone()), &wt, 1);
    assert_eq!(state(&f, &name), "created");
    let mut discard = remove;
    discard["discard_changes"] = json!(true);
    ok(&f.engine, "workspace.remove", discard);
    wait_gone(&wt);
}
