//! `project.remove` / `workspace.remove` with `force`: one confirmation closes the agents and
//! removes the children, without touching the repository and, unless asked, without deleting
//! any worktree.

use relay_bus::{Actor, Request, Response};
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
    match call(e, op, payload).into_result() {
        Ok(v) => v,
        Err(err) => panic!("{op} failed: {} {}", err.code, err.message),
    }
}
fn refused(r: Response) -> relay_bus::BusError {
    r.error.expect("expected an error")
}

fn git(repo: &Path, args: &[&str]) {
    let st = Command::new("git").arg("-C").arg(repo).args(args).status().unwrap();
    assert!(st.success(), "git {args:?}");
}

fn repo_in(ws: &Path, name: &str) -> PathBuf {
    let repo = ws.join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README.md"), "hi\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    repo
}

struct Fixture {
    _root: tempfile::TempDir,
    root: PathBuf,
    ws: PathBuf,
    repo: PathBuf,
    engine: Arc<Engine>,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let ws = root.join("ws");
    let repo = repo_in(&ws, "app");
    let store = Store::open(&root.join("store").join("store.db"), false).unwrap();
    let engine = Engine::new(Instance::Test, store);
    ok(&engine, "workspace.create", json!({"path": ws}));
    ok(&engine, "project.add", json!({"workspace_id": 1, "path": repo}));
    let provider = root.join("fake-claude.sh");
    std::fs::write(&provider, "#!/bin/sh\necho ready\nwhile IFS= read -r line; do :; done\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&provider, std::fs::Permissions::from_mode(0o755)).unwrap();
    ok(&engine, "settings.set", json!({"path": "providers.claude.path", "value": provider}));
    Fixture { _root: tmp, root, ws, repo, engine }
}

fn alive(pid: i64) -> bool {
    relay_core::pty::pid_alive(pid as u32)
        && std::fs::read_to_string(format!("/proc/{pid}/stat")).map(|s| !s.contains(") Z ")).unwrap_or(false)
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A spawned agent: (name, worktree, branch, pid).
fn spawn(e: &Engine, project_id: i64) -> (String, PathBuf, String, i64) {
    let s = ok(e, "session.create", json!({"project_id": project_id, "provider": "claude"}));
    let name = s["name"].as_str().unwrap().to_string();
    let spawned = ok(e, "session.spawn", json!({"session": name}));
    (name, PathBuf::from(s["worktree"].as_str().unwrap()), s["branch"].as_str().unwrap().to_string(), spawned["pid"].as_i64().unwrap())
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    Command::new("git").arg("-C").arg(repo).args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
        .output().unwrap().status.success()
}

fn count(e: &Engine, sql: &str) -> i64 {
    e.store.lock().query_row(sql, [], |row| row.get(0)).unwrap()
}

#[test]
fn project_remove_refuses_open_sessions_and_force_closes_them_keeping_every_checkout() {
    let f = fixture();
    let e = &f.engine;
    let (first, first_wt, first_branch, first_pid) = spawn(e, 1);
    let (_, second_wt, _, second_pid) = spawn(e, 1);
    // A session created but never spawned is still open.
    ok(e, "session.create", json!({"project_id": 1, "provider": "claude"}));

    let error = refused(call(e, "project.remove", json!({"project_id": 1})));
    assert_eq!(error.code, "project.sessions_live");
    assert_eq!(error.details.as_ref().unwrap()["open_sessions"], 3);
    assert!(alive(first_pid) && alive(second_pid), "a refused removal must not touch the agents");

    let out = ok(e, "project.remove", json!({"project_id": 1, "force": true}));
    assert_eq!(out["sessions_closed"], 3);
    wait_until("closed agents' processes gone", || !alive(first_pid) && !alive(second_pid));
    assert_eq!(e.live_pty_count(), 0);
    assert!(f.repo.join("README.md").is_file(), "the repository is never touched");
    assert!(first_wt.join("README.md").is_file() && second_wt.join("README.md").is_file(),
        "worktrees stay on disk unless remove_worktrees");
    assert!(branch_exists(&f.repo, &first_branch));
    assert_eq!(count(e, "SELECT COUNT(*) FROM projects"), 0);
    assert_eq!(count(e, "SELECT COUNT(*) FROM sessions"), 0);
    assert_eq!(refused(call(e, "session.get", json!({"session": first}))).code, "session.not_found");
}

#[test]
fn force_with_remove_worktrees_deletes_pool_checkouts_but_keeps_branches_even_for_a_pair() {
    let f = fixture();
    let e = &f.engine;
    let (builder, worktree, branch, pid) = spawn(e, 1);
    let reviewer = ok(e, "session.create", json!({"project_id": 1, "provider": "claude", "role": "reviewer", "pair_with": builder}));
    assert_eq!(reviewer["worktree"].as_str().unwrap(), worktree.to_str().unwrap());
    let (_, solo_wt, solo_branch, _) = spawn(e, 1);

    let out = ok(e, "project.remove", json!({"project_id": 1, "force": true, "remove_worktrees": true}));
    assert_eq!(out["sessions_closed"], 3);
    wait_until("closed agent gone", || !alive(pid));
    assert!(!worktree.exists(), "the pair's shared checkout is removed once both have closed");
    assert!(!solo_wt.exists());
    assert!(branch_exists(&f.repo, &branch) && branch_exists(&f.repo, &solo_branch), "branches are always kept");
    assert!(f.repo.join("README.md").is_file());
}

#[test]
fn force_stops_device_runs_but_never_interrupts_an_integration() {
    let f = fixture();
    let e = &f.engine;
    let (_, _, _, pid) = spawn(e, 1);
    e.store.lock().execute(
        "INSERT INTO integrations(project_id, branches, state, created_at) VALUES (1, '[]', 'merging', 'now')", [],
    ).unwrap();
    let error = refused(call(e, "project.remove", json!({"project_id": 1, "force": true})));
    assert_eq!(error.code, "project.activity_live");
    assert!(alive(pid), "a refused forced removal closes nothing");
    assert_eq!(count(e, "SELECT COUNT(*) FROM sessions WHERE state != 'closed'"), 1);

    {
        let conn = e.store.lock();
        conn.execute("UPDATE integrations SET state = 'done'", []).unwrap();
        conn.execute(
            "INSERT INTO device_runs(project_id, device, worktree, state, started_at) VALUES (1, 'emulator-5554', '/x', 'running', 'now')", [],
        ).unwrap();
    }
    assert_eq!(refused(call(e, "project.remove", json!({"project_id": 1}))).code, "project.sessions_live");
    let out = ok(e, "project.remove", json!({"project_id": 1, "force": true}));
    assert_eq!(out["sessions_closed"], 1);
    assert_eq!(out["runs_stopped"], 1);
    wait_until("closed agent gone", || !alive(pid));
}

#[test]
fn workspace_remove_force_removes_its_projects_and_their_agents() {
    let f = fixture();
    let e = &f.engine;
    let other = repo_in(&f.ws, "lib");
    let second = ok(e, "project.add", json!({"workspace_id": 1, "path": other}))["id"].as_i64().unwrap();
    let (_, wt, _, pid) = spawn(e, 1);
    let (_, _, _, other_pid) = spawn(e, second);
    // An unrelated workspace is left alone.
    let elsewhere = f.root.join("elsewhere");
    let kept_repo = repo_in(&elsewhere, "kept");
    ok(e, "workspace.create", json!({"path": elsewhere}));
    let kept = ok(e, "project.add", json!({"workspace_id": 2, "path": kept_repo}))["id"].as_i64().unwrap();
    let (_, _, _, kept_pid) = spawn(e, kept);

    let error = refused(call(e, "workspace.remove", json!({"workspace_id": 1})));
    assert_eq!(error.code, "workspace.has_projects");
    assert_eq!(error.details.as_ref().unwrap()["projects"], 2);
    assert!(alive(pid) && alive(other_pid));

    let out = ok(e, "workspace.remove", json!({"workspace_id": 1, "force": true}));
    assert_eq!(out["projects_removed"], 2);
    assert_eq!(out["sessions_closed"], 2);
    wait_until("workspace agents gone", || !alive(pid) && !alive(other_pid));
    assert!(alive(kept_pid), "another workspace's agent keeps running");
    assert!(wt.join("README.md").is_file());
    assert!(f.repo.join("README.md").is_file() && other.join("README.md").is_file());
    assert_eq!(ok(e, "workspace.list", json!({}))["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(count(e, "SELECT COUNT(*) FROM projects"), 1);

    // An empty workspace still removes without force.
    ok(e, "project.remove", json!({"project_id": kept, "force": true}));
    let out = ok(e, "workspace.remove", json!({"workspace_id": 2}));
    assert_eq!(out["projects_removed"], 0);
}

#[test]
fn a_project_with_labelled_tasks_removes_and_so_does_its_workspace() {
    let f = fixture();
    let e = &f.engine;
    ok(e, "task.create", json!({"project_id": 1, "title": "Tagged", "labels": ["ui"]}));
    let untagged = ok(e, "task.create", json!({"project_id": 1, "title": "Was tagged"}));
    ok(e, "task.label.add", json!({"task_id": untagged["id"], "label": "stale"}));
    ok(e, "task.label.remove", json!({"task_id": untagged["id"], "label": "stale"}));
    let (_, _, _, pid) = spawn(e, 1);
    // RA-419: both clients' saved layouts go with the project, whole subtrees included; a
    // project 10's key only shares the prefix.
    for path in ["layout.current.1", "layout.current.1.grid", "native.layout.current.1", "native.layout.current.1.panes", "native.layout.current.10"] {
        e.store.lock().execute("INSERT INTO settings (path, value, updated_at) VALUES (?1, '{}', '')", [path]).unwrap();
    }

    let out = ok(e, "project.remove", json!({"project_id": 1, "force": true}));
    assert_eq!(out["sessions_closed"], 1);
    wait_until("closed agent gone", || !alive(pid));
    assert_eq!(count(e, "SELECT COUNT(*) FROM labels"), 0);
    assert_eq!(count(e, "SELECT COUNT(*) FROM settings WHERE path LIKE '%layout.current.1%' AND path != 'native.layout.current.10'"), 0);
    assert_eq!(count(e, "SELECT COUNT(*) FROM settings WHERE path = 'native.layout.current.10'"), 1);
    assert_eq!(count(e, "SELECT COUNT(*) FROM projects"), 0);

    let again = ok(e, "project.add", json!({"workspace_id": 1, "path": f.repo}))["id"].clone();
    ok(e, "task.create", json!({"project_id": again, "title": "Tagged", "labels": ["ui"]}));
    let out = ok(e, "workspace.remove", json!({"workspace_id": 1, "force": true}));
    assert_eq!(out["projects_removed"], 1);
    assert_eq!(count(e, "SELECT COUNT(*) FROM labels"), 0);
}

#[test]
fn an_integration_interrupted_by_a_restart_no_longer_blocks_removal() {
    let f = fixture();
    let e = &f.engine;
    e.store.lock().execute(
        "INSERT INTO integrations(project_id, branches, state, created_at) VALUES (1, '[]', 'building', 'now')", [],
    ).unwrap();
    assert_eq!(refused(call(e, "project.remove", json!({"project_id": 1}))).code, "project.activity_live");
    relay_core::recovery::run(e).unwrap();
    let state: String = e.store.lock().query_row("SELECT state FROM integrations", [], |r| r.get(0)).unwrap();
    assert_eq!(state, "failed");
    ok(e, "project.remove", json!({"project_id": 1}));
}
