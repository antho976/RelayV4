//! Low-severity session fixes from the 2026-10 audit: blocked dones keep their claims, review
//! groups read both ways and survive a builder leaving, a reviewer links no commits, exits do
//! not fail a task a partner is still on, session.update undoes and refuses the primary's
//! branch, and session.restorable stays inside an agent's project. All through the bus.

use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn call(e: &Engine, actor: Actor, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(actor, op, payload), Door::InProcess)
}

fn ok(e: &Engine, actor: Actor, op: &str, payload: Value) -> Value {
    call(e, actor, op, payload).into_result()
        .unwrap_or_else(|error| panic!("{op} failed: {} {}", error.code, error.message))
}

fn code(e: &Engine, actor: Actor, op: &str, payload: Value) -> String {
    call(e, actor, op, payload).into_result().expect_err(&format!("{op} was expected to refuse")).code
}

fn user(e: &Engine, op: &str, payload: Value) -> Value { ok(e, Actor::User, op, payload) }

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn repo(path: &Path) {
    std::fs::create_dir_all(path.join("src")).unwrap();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.email", "t@t"]);
    git(path, &["config", "user.name", "t"]);
    std::fs::write(path.join("src/lib.rs"), "pub fn one() -> i32 { 1 }\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-qm", "init"]);
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    engine: Arc<Engine>,
}

/// One workspace, project 1 with an active task 1.
fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let ws = root.join("ws");
    repo(&ws.join("app"));
    let engine = Engine::new(Instance::Test, Store::open(&root.join("store/store.db"), false).unwrap());
    user(&engine, "workspace.create", json!({"path": ws}));
    user(&engine, "project.add", json!({"workspace_id": 1, "path": ws.join("app")}));
    engine.store.with_tx(|tx| {
        tx.execute(
            "INSERT INTO tasks(project_id,title,body,changelog,col,state,position,created_at,updated_at)
             VALUES (1,'Ship it','','','active','working',0,?1,?1)",
            ["2026-10-01T00:00:00Z"],
        )?;
        Ok(())
    }).unwrap();
    Fixture { _tmp: tmp, root, engine }
}

fn name(session: &Value) -> String { session["name"].as_str().unwrap().to_string() }

fn count(e: &Engine, sql: &str) -> i64 {
    e.store.lock().query_row(sql, [], |row| row.get(0)).unwrap()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// RA-400, RA-396.
#[test]
fn a_blocked_done_keeps_its_claims_and_a_reviewer_links_no_commit() {
    let f = fixture();
    let e = &f.engine;
    let builder = name(&user(e, "session.create", json!({"project_id": 1, "provider": "claude", "role": "builder", "task_id": 1})));
    let reviewer = name(&user(e, "session.create", json!({"project_id": 1, "provider": "codex", "role": "reviewer", "pair_with": builder, "task_id": 1})));
    let b = || Actor::agent(&builder);

    ok(e, b(), "session.claim", json!({"paths": ["src/lib.rs"]}));
    ok(e, b(), "session.done", json!({"session": builder, "status": "blocked", "blockers": ["waiting on a decision"]}));
    assert_eq!(count(e, "SELECT COUNT(*) FROM claims"), 1, "a blocked task stays current, and so do its claims");
    ok(e, b(), "session.report", json!({"session": builder, "kind": "stop"}));

    ok(e, b(), "session.done", json!({"session": builder, "status": "completed", "sha": "  abc123  "}));
    assert_eq!(count(e, "SELECT COUNT(*) FROM claims"), 0, "completion releases them");
    assert_eq!(count(e, "SELECT COUNT(*) FROM task_commits WHERE sha='abc123'"), 1, "the builder's sha is linked, trimmed");

    ok(e, Actor::agent(&reviewer), "session.done", json!({"session": reviewer, "status": "completed", "sha": "def456"}));
    assert_eq!(count(e, "SELECT COUNT(*) FROM task_commits"), 1, "a reviewer links no commit");
}

/// RA-398.
#[test]
fn a_review_groups_second_builder_reads_its_reviewer_and_the_group_survives_the_first_leaving() {
    let f = fixture();
    let e = &f.engine;
    let b1 = name(&user(e, "session.create", json!({"project_id": 1, "provider": "claude", "role": "builder", "task_id": 1})));
    let r = name(&user(e, "session.create", json!({"project_id": 1, "provider": "codex", "role": "reviewer", "pair_with": b1})));
    let b2 = name(&user(e, "session.create", json!({"project_id": 1, "provider": "claude", "role": "builder", "pair_with": r})));

    ok(e, Actor::agent(&b2), "session.brief", json!({"session": r}));
    // No PTY and nothing saved: past the ownership check, the scrollback simply is not there.
    assert_eq!(code(e, Actor::agent(&b2), "session.scrollback", json!({"session": r})), "session.not_spawned");
    assert_eq!(code(e, Actor::agent(&b2), "session.brief", json!({"session": b1})), "actor.scope", "builders are not each other's PAIR");

    user(e, "session.close", json!({"session": b1, "remove_worktree": false}));
    assert_eq!(user(e, "session.get", json!({"session": r}))["pair_with"], b2.as_str(), "the reviewer keeps its second builder");
    assert_eq!(
        code(e, Actor::User, "session.create", json!({"project_id": 1, "provider": "codex", "role": "reviewer", "pair_with": r})),
        "session.pair_exists",
    );
}

/// RA-403, RA-402.
#[test]
fn session_update_undoes_after_spawn_and_never_renames_the_primary_branch() {
    let f = fixture();
    let e = &f.engine;
    let s = user(e, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let s_name = name(&s);
    e.store.lock().execute("UPDATE sessions SET spawned_at='2026-10-01T00:00:00Z', epoch=1 WHERE id=?1", [s["id"].as_i64().unwrap()]).unwrap();
    user(e, "session.update", json!({"session": s_name, "allow_ui": true}));
    let audit = user(e, "audit.list", json!({"op_prefix": "session.update", "limit": 1}))["rows"][0]["id"].as_i64().unwrap();
    user(e, "audit.undo", json!({"audit_id": audit}));
    assert_eq!(user(e, "session.get", json!({"session": s_name}))["allow_ui"], false);

    let primary = user(e, "session.create", json!({"project_id": 1, "provider": "claude", "worktree": "primary"}));
    assert_eq!(code(e, Actor::User, "session.update", json!({"session": name(&primary), "branch": "relay/renamed"})), "session.branch_primary");
    let head = Command::new("git").arg("-C").arg(f.root.join("ws/app")).args(["branch", "--show-current"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "main");

    // A pooled checkout of its own still renames, and the rename undoes.
    let own = name(&user(e, "session.create", json!({"project_id": 1, "provider": "claude"})));
    assert_eq!(user(e, "session.update", json!({"session": own, "branch": "relay/custom"}))["branch"], "relay/custom");
    let audit = user(e, "audit.list", json!({"op_prefix": "session.update", "limit": 1}))["rows"][0]["id"].as_i64().unwrap();
    user(e, "audit.undo", json!({"audit_id": audit}));
    assert_ne!(user(e, "session.get", json!({"session": own}))["branch"], "relay/custom");
}

fn last_update(e: &Engine) -> i64 {
    user(e, "audit.list", json!({"op_prefix": "session.update", "limit": 1}))["rows"][0]["id"].as_i64().unwrap()
}

/// RA-403: every report of a running session moves its `updated_at`, so the undo checks the
/// fields the update wrote instead; an older row that recorded only `updated_at` still works.
#[test]
fn session_update_undo_checks_the_fields_it_wrote_not_updated_at() {
    let f = fixture();
    let e = &f.engine;
    let s = name(&user(e, "session.create", json!({"project_id": 1, "provider": "claude"})));
    let get = |field: &str| user(e, "session.get", json!({"session": s}))[field].clone();
    user(e, "session.update", json!({"session": s, "allow_ui": true}));
    let audit = last_update(e);
    let stamp = get("updated_at");
    std::thread::sleep(Duration::from_millis(5));
    ok(e, Actor::agent(&s), "session.report", json!({"session": s, "kind": "session_start"}));
    ok(e, Actor::agent(&s), "session.report", json!({"session": s, "kind": "tool_use"}));
    assert_ne!(get("updated_at"), stamp, "a report moves updated_at");
    user(e, "audit.undo", json!({"audit_id": audit}));
    assert_eq!(get("allow_ui"), false);

    // A field the update wrote was written again since: stale, unless forced.
    user(e, "session.update", json!({"session": s, "allow_ui": true}));
    let audit = last_update(e);
    user(e, "session.update", json!({"session": s, "allow_ui": false}));
    assert_eq!(code(e, Actor::User, "audit.undo", json!({"audit_id": audit})), "audit.stale");
    user(e, "audit.undo", json!({"audit_id": audit, "force": true}));

    // A row recorded before the change carries only `updated_at`, and is checked against it.
    user(e, "session.update", json!({"session": s, "allow_ui": true}));
    let audit = last_update(e);
    let expect = |updated_at: &str| e.store.lock().execute(
        "UPDATE audit SET undo_op=json_set(undo_op,'$.expect',json_object('updated_at',?1)) WHERE id=?2",
        rusqlite::params![updated_at, audit],
    ).unwrap();
    expect("2000-01-01T00:00:00Z");
    assert_eq!(code(e, Actor::User, "audit.undo", json!({"audit_id": audit})), "audit.stale");
    expect(get("updated_at").as_str().unwrap());
    user(e, "audit.undo", json!({"audit_id": audit}));
    assert_eq!(get("allow_ui"), false);
}

/// RA-403: undoing an update that queued a task takes back the queue row it added, and only
/// that one.
#[test]
fn undoing_a_task_attach_drops_the_queue_row_it_added_and_keeps_an_older_one() {
    let f = fixture();
    let e = &f.engine;
    let created = user(e, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let (s, sid) = (name(&created), created["id"].as_i64().unwrap());
    let queued = || count(e, &format!("SELECT COUNT(*) FROM task_sessions WHERE task_id=1 AND session_id={sid}"));
    assert_eq!(queued(), 0);
    user(e, "session.update", json!({"session": s, "task_id": 1}));
    assert_eq!(queued(), 1);
    user(e, "audit.undo", json!({"audit_id": last_update(e)}));
    assert_eq!(queued(), 0, "the row the update added outlived its undo");
    assert_eq!(user(e, "session.get", json!({"session": s}))["task_id"], Value::Null);

    // Queued before the update: the row is the queue's, and stays.
    e.store.lock().execute("INSERT INTO task_sessions(task_id,session_id,ord,queue_ord) VALUES (1,?1,0,0)", [sid]).unwrap();
    user(e, "session.update", json!({"session": s, "task_id": 1}));
    user(e, "audit.undo", json!({"audit_id": last_update(e)}));
    assert_eq!(queued(), 1, "an undo removed a row the update did not add");
}

/// RA-404.
#[test]
fn an_agent_lists_only_its_own_projects_restorable_sessions() {
    let f = fixture();
    let e = &f.engine;
    repo(&f.root.join("ws/other"));
    user(e, "project.add", json!({"workspace_id": 1, "path": f.root.join("ws/other")}));
    let mine = user(e, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let theirs = user(e, "session.create", json!({"project_id": 2, "provider": "claude"}));
    e.store.lock().execute("UPDATE sessions SET state='restorable' WHERE id=?1", [theirs["id"].as_i64().unwrap()]).unwrap();

    let agent = || Actor::agent(name(&mine));
    assert!(ok(e, agent(), "session.restorable", json!({}))["sessions"].as_array().unwrap().is_empty());
    assert_eq!(code(e, agent(), "session.restorable", json!({"project_id": 2})), "actor.scope");
    assert_eq!(user(e, "session.restorable", json!({}))["sessions"][0]["session"]["name"], name(&theirs).as_str());
}

/// RA-399.
#[test]
fn a_builders_exit_fails_its_task_only_when_nobody_else_is_working_on_it() {
    let f = fixture();
    let e = &f.engine;
    let provider = f.root.join("fake-claude.sh");
    std::fs::write(&provider, "#!/bin/sh\necho hello-from-pty\nwhile IFS= read -r line; do [ \"$line\" = exit ] && exit 3; done\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&provider, std::fs::Permissions::from_mode(0o755)).unwrap();
    user(e, "settings.set", json!({"path": "providers.claude.path", "value": provider}));
    let b1 = name(&user(e, "session.create", json!({"project_id": 1, "provider": "claude", "role": "builder", "task_id": 1})));
    let b2 = user(e, "session.create", json!({"project_id": 1, "provider": "codex", "role": "builder", "pair_with": b1, "task_id": 1}));
    let task_state = || user(e, "task.get", json!({"task_id": 1}))["state"].as_str().unwrap().to_string();
    let exit = |launch: &str| {
        user(e, launch, json!({"session": b1}));
        user(e, "session.input", json!({"session": b1, "data": "exit\n"}));
        wait_until("builder exit", || user(e, "session.get", json!({"session": b1}))["state"] == "exited");
    };

    // The partner is still at work on the task: one builder leaving does not fail it.
    e.store.lock().execute("UPDATE sessions SET state='running' WHERE id=?1", [b2["id"].as_i64().unwrap()]).unwrap();
    exit("session.spawn");
    assert_ne!(task_state(), "failed");

    // Nobody left on it: the exit fails it, as before.
    e.store.lock().execute("UPDATE sessions SET state='created' WHERE id=?1", [b2["id"].as_i64().unwrap()]).unwrap();
    exit("session.resume");
    assert_eq!(task_state(), "failed");
}
