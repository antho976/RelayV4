//! `task.comment`: GitHub-issue-style comments on a task, read back whole and oldest first by
//! `task.activity`. Every call crosses the bus door.

mod common;

use common::{call_as as call, code, committed_repo, engine_with_project, ok, ok_as};
use relay_bus::Actor;
use relay_core::engine::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A provider that answers the discovery probe, so `task.dispatch` resolves one without the
/// machine having a real `claude` (see `board.rs`).
fn fake_provider(dir: &Path, binary: &str) -> PathBuf {
    let path = dir.join(binary);
    std::fs::write(
        &path,
        "#!/bin/sh\ncase \"${1:-}\" in\n  --version) echo 'fixture 1.0'; exit 0;;\n  auth|login) echo '{\"loggedIn\":true}'; exit 0;;\nesac\nwhile IFS= read -r line; do :; done\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

struct Fixture {
    _root: tempfile::TempDir,
    engine: Arc<Engine>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        let repo = ws.join("app");
        committed_repo(&repo, &[("README.md", "comments\n")]);
        let engine = engine_with_project(root.path(), &ws, &repo);
        let claude = fake_provider(root.path(), "claude");
        ok(&engine, "settings.set", json!({"path":"providers.claude.path","value":claude}));
        Self { _root: root, engine }
    }
    fn task(&self, title: &str) -> i64 {
        ok(&self.engine, "task.create", json!({"project_id":1,"title":title}))["id"].as_i64().unwrap()
    }
}

#[test]
fn a_comment_returns_its_row_and_activity_lists_them_oldest_first() {
    let f = Fixture::new();
    let e = &f.engine;
    let task = f.task("Discuss");
    let other = f.task("Elsewhere");

    let first = ok(e, "task.comment", json!({"task_id":task,"body":"First thought"}));
    assert_eq!(first["task_id"], task);
    assert_eq!(first["author"], "user");
    assert_eq!(first["body"], "First thought");
    assert!(first["id"].as_i64().is_some_and(|id| id > 0));
    assert!(first["created_at"].as_str().is_some_and(|at| !at.is_empty()));
    let keys: Vec<&str> = first.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys.len(), 5, "TaskComment is id, task_id, author, body, created_at: {keys:?}");

    let second = ok(e, "task.comment", json!({"task_id":task,"body":"  indented\nsecond line  "}));
    assert_eq!(second["body"], "  indented\nsecond line  ", "the body is stored as written");
    ok(e, "task.comment", json!({"task_id":other,"body":"not this task"}));

    let activity = ok(e, "task.activity", json!({"task_id":task}));
    let comments = activity["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0], first, "oldest first, and each equals what task.comment returned");
    assert_eq!(comments[1], second);

    // The task's updated_at is untouched, so a comment never trips `expected_updated_at`.
    let before = ok(e, "task.get", json!({"task_id":other}))["updated_at"].clone();
    ok(e, "task.comment", json!({"task_id":other,"body":"again"}));
    assert_eq!(ok(e, "task.get", json!({"task_id":other}))["updated_at"], before);

    // A task with no comments reads an empty list.
    let quiet = f.task("Quiet");
    assert_eq!(ok(e, "task.activity", json!({"task_id":quiet}))["comments"], json!([]));
}

#[test]
fn comments_are_not_reported_again_in_history() {
    let f = Fixture::new();
    let e = &f.engine;
    let task = f.task("Discuss");
    ok(e, "task.comment", json!({"task_id":task,"body":"one"}));
    ok(e, "task.comment", json!({"task_id":task,"body":"two"}));
    ok(e, "task.move", json!({"task_id":task,"column":"ready"}));

    let activity = ok(e, "task.activity", json!({"task_id":task}));
    let ops: Vec<&str> = activity["history"].as_array().unwrap().iter().map(|row| row["op"].as_str().unwrap()).collect();
    assert_eq!(ops, vec!["task.move", "task.create"], "history leaves task.comment out");
    assert_eq!(activity["comments"].as_array().unwrap().len(), 2);
    // The comment is still audited, as every mutation is.
    let audited = ok(e, "audit.list", json!({"op_prefix":"task.comment"}))["rows"].as_array().unwrap().len();
    assert_eq!(audited, 2);
}

#[test]
fn empty_oversized_and_deleted_are_refused() {
    let f = Fixture::new();
    let e = &f.engine;
    let task = f.task("Discuss");
    for body in ["", "   ", "\n\t \n"] {
        let error = call(e, Actor::User, "task.comment", json!({"task_id":task,"body":body})).error.expect("refused");
        assert_eq!(error.code, "task.comment_empty");
        assert_eq!(error.kind, relay_bus::error::ErrorKind::Invalid);
    }
    let long = "x".repeat(64 * 1024 + 1);
    assert_eq!(code(call(e, Actor::User, "task.comment", json!({"task_id":task,"body":long}))), "task.comment_size");
    assert_eq!(code(call(e, Actor::User, "task.comment", json!({"task_id":9999,"body":"hi"}))), "task.not_found");

    ok(e, "task.delete", json!({"task_id":task}));
    assert_eq!(code(call(e, Actor::User, "task.comment", json!({"task_id":task,"body":"hi"}))), "task.not_found");
    ok(e, "task.restore", json!({"task_id":task}));
    assert_eq!(ok(e, "task.activity", json!({"task_id":task}))["comments"], json!([]), "nothing refused was stored");
}

#[test]
fn an_agent_comments_on_its_own_task_under_its_session_name() {
    let f = Fixture::new();
    let e = &f.engine;
    let mine = f.task("Mine");
    let theirs = f.task("Theirs");
    let session = ok(e, "session.create", json!({"project_id":1,"provider":"claude","role":"builder"}));
    let name = session["name"].as_str().unwrap();
    ok(e, "task.dispatch", json!({"task_id":mine,"session":name,"start":false}));
    let agent = Actor::agent(name);

    let comment = ok_as(e, agent.clone(), "task.comment", json!({"task_id":mine,"body":"Done the first half"}));
    assert_eq!(comment["author"], name);
    assert_eq!(code(call(e, agent, "task.comment", json!({"task_id":theirs,"body":"hi"}))), "actor.scope");
    let comments: Vec<Value> = ok(e, "task.activity", json!({"task_id":mine}))["comments"].as_array().unwrap().clone();
    assert_eq!(comments, vec![comment]);
}
