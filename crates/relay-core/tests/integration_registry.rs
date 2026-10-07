//! Integrations and the project/workspace registry: agent scope on `integration.*`, merge
//! failures that name the branches that really conflict, a discard that stops a running build,
//! ids that are never recycled, and the store backup taken before a removal.

use relay_bus::{Actor, BusError, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn call_as(e: &Engine, actor: Actor, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(actor, op, payload), Door::InProcess)
}
fn ok(e: &Engine, op: &str, payload: Value) -> Value {
    match call_as(e, Actor::User, op, payload).into_result() {
        Ok(v) => v,
        Err(err) => panic!("{op} failed: {} {}", err.code, err.message),
    }
}
fn refused(r: Response) -> BusError {
    r.error.expect("expected an error")
}

fn git(repo: &Path, args: &[&str]) {
    let st = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
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

/// A branch off `main` that writes `file` with `text`.
fn branch(repo: &Path, name: &str, file: &str, text: &str) {
    git(repo, &["checkout", "-q", "-b", name, "main"]);
    std::fs::write(repo.join(file), text).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", name]);
    git(repo, &["checkout", "-q", "main"]);
}

struct Fixture {
    _tmp: tempfile::TempDir,
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
    Fixture { _tmp: tmp, root, ws, repo, engine }
}

fn settle(e: &Engine, id: i64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let value = ok(e, "integration.get", json!({"integration_id": id}));
        if !matches!(value["state"].as_str(), Some("queued" | "merging" | "building" | "deploying")) {
            return value;
        }
        assert!(Instant::now() < deadline, "integration {id} never finished: {value}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn an_agent_reaches_only_its_own_projects_integrations_and_cannot_deploy() {
    let f = fixture();
    let e = &f.engine;
    let other = repo_in(&f.ws, "other");
    ok(e, "project.add", json!({"workspace_id": 1, "path": other}));
    branch(&other, "a", "a.txt", "a\n");
    branch(&other, "b", "b.txt", "b\n");
    let session = ok(e, "session.create", json!({"project_id": 1, "provider": "codex", "role": "builder"}));
    let agent = Actor::agent(session["name"].as_str().unwrap());

    let foreign = call_as(e, agent.clone(), "integration.request", json!({"project_id": 2, "branches": ["a", "b"], "build": false}));
    assert_eq!(refused(foreign).code, "actor.scope");
    let listed = call_as(e, agent.clone(), "integration.list", json!({"project_id": 2}));
    assert_eq!(refused(listed).code, "actor.scope");

    branch(&f.repo, "a", "a.txt", "a\n");
    branch(&f.repo, "b", "b.txt", "b\n");
    let deploy = call_as(e, agent.clone(), "integration.request", json!({"project_id": 1, "branches": ["a", "b"], "build": false, "deploy": "emulator-5554"}));
    assert_eq!(refused(deploy).code, "actor.allowlist");
    let own = call_as(e, agent, "integration.request", json!({"project_id": 1, "branches": ["a", "b"], "build": false})).into_result().unwrap();
    assert_eq!(settle(e, own["id"].as_i64().unwrap())["state"], "passed");
}

#[test]
fn an_agents_build_waits_for_a_person_unless_the_project_trusts_agent_builds() {
    let f = fixture();
    let e = &f.engine;
    branch(&f.repo, "a", "a.txt", "a\n");
    branch(&f.repo, "b", "b.txt", "b\n");
    let session = ok(e, "session.create", json!({"project_id": 1, "provider": "codex", "role": "builder"}));
    let agent = Actor::agent(session["name"].as_str().unwrap());
    let request = json!({"project_id": 1, "branches": ["a", "b"]});
    let count = || e.store.lock().query_row("SELECT COUNT(*) FROM integrations", [], |r| r.get::<_, i64>(0)).unwrap();

    // Build defaults to on: held, and nothing is queued until someone says yes.
    let held = refused(call_as(e, agent.clone(), "integration.request", request.clone()));
    assert_eq!((held.code.as_str(), held.kind), ("integration.agent_build", relay_bus::ErrorKind::Held), "{held:?}");
    assert_eq!(count(), 0);
    let hold_id = held.confirm.as_ref().expect("a hold to confirm").payload["hold_id"].as_i64().unwrap();
    let confirmed = ok(e, "guardrail.confirm", json!({"hold_id": hold_id}));
    let queued = &confirmed["outcome"]["result"];
    assert_eq!(settle(e, queued["id"].as_i64().unwrap())["state"], "passed", "{confirmed}");

    // A merge-only request needs nobody.
    ok_as(e, agent.clone(), "integration.request", json!({"project_id": 1, "branches": ["a", "b"], "build": false}));
    // The user is never held.
    ok(e, "integration.request", request.clone());
    // A project that trusts agent builds lets them straight through.
    ok(e, "guardrail.config.set", json!({"project_id": 1, "patch": {"agent_builds": true}}));
    ok_as(e, agent, "integration.request", request);
    assert_eq!(count(), 4);
}

fn ok_as(e: &Engine, actor: Actor, op: &str, payload: Value) -> Value {
    call_as(e, actor, op, payload).into_result().unwrap_or_else(|err| panic!("{op}: {err:?}"))
}

#[test]
fn a_failed_merge_names_the_branches_that_really_conflict() {
    let f = fixture();
    let e = &f.engine;
    // Sorted, these are a, b, c: the old report always blamed a and b, which merge cleanly.
    branch(&f.repo, "a-left", "shared.txt", "left\n");
    branch(&f.repo, "b-clean", "other.txt", "clean\n");
    branch(&f.repo, "c-right", "shared.txt", "right\n");
    let queued = ok(e, "integration.request", json!({"project_id": 1, "branches": ["a-left", "b-clean", "c-right"], "build": false}));
    let done = settle(e, queued["id"].as_i64().unwrap());
    assert_eq!(done["state"], "conflict", "{done}");
    assert_eq!(done["conflict"], json!(["a-left", "c-right"]), "{done}");
    assert!(done["log_tail"].as_str().unwrap().contains("shared.txt"), "{done}");

    // A branch name git would read as an option is refused outright.
    git(&f.repo, &["update-ref", "refs/heads/-Xours", "HEAD"]);
    let dashed = call_as(e, Actor::User, "integration.request", json!({"project_id": 1, "branches": ["-Xours", "b-clean"], "build": false}));
    assert_eq!(refused(dashed).code, "integration.branch");
}

#[test]
fn discarding_an_integration_stops_its_build_and_frees_the_project() {
    if Command::new("fish").arg("-c").arg("true").output().is_err() {
        eprintln!("fish is not installed; skipping");
        return;
    }
    let f = fixture();
    let e = &f.engine;
    branch(&f.repo, "a", "a.txt", "a\n");
    branch(&f.repo, "b", "b.txt", "b\n");
    let marker = f.root.join("build-finished");
    ok(e, "project.update", json!({"project_id": 1, "build_cmd": format!("sleep 30; touch '{}'", marker.display())}));
    let queued = ok(e, "integration.request", json!({"project_id": 1, "branches": ["a", "b"]}));
    let id = queued["id"].as_i64().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while ok(e, "integration.get", json!({"integration_id": id}))["state"] != "building" {
        assert!(Instant::now() < deadline, "the build never started");
        std::thread::sleep(Duration::from_millis(25));
    }
    let started = Instant::now();
    ok(e, "integration.discard", json!({"integration_id": id}));
    assert!(started.elapsed() < Duration::from_secs(10), "discard waited out the build");
    let value = ok(e, "integration.get", json!({"integration_id": id}));
    assert_eq!(value["state"], "discarded");
    assert!(!Path::new(value["worktree"].as_str().unwrap()).exists());
    // Nothing left live: the project can go.
    ok(e, "project.remove", json!({"project_id": 1}));
    std::thread::sleep(Duration::from_millis(200));
    assert!(!marker.exists());
}

fn stored(e: &Engine, like: &str) -> i64 {
    e.store.lock().query_row("SELECT COUNT(*) FROM settings WHERE path LIKE ?1", [like], |r| r.get(0)).unwrap()
}

#[test]
fn removed_ids_are_never_handed_out_again() {
    let f = fixture();
    let e = &f.engine;
    let second = repo_in(&f.ws, "second");
    assert_eq!(ok(e, "project.add", json!({"workspace_id": 1, "path": second}))["id"], 2);
    ok(e, "settings.set", json!({"path": "guardrails.projects.2.caps", "value": {"files": 3, "lines": 30}}));
    assert_eq!(stored(e, "guardrails.projects.2.%"), 2);
    ok(e, "project.remove", json!({"project_id": 2}));
    assert_eq!(ok(e, "project.add", json!({"workspace_id": 1, "path": second}))["id"], 3);
    assert_eq!(stored(e, "guardrails.projects.2.%"), 0);

    ok(e, "settings.set", json!({"path": "guardrails.workspaces.1.caps", "value": {"files": 3, "lines": 30}}));
    assert_eq!(stored(e, "guardrails.workspaces.1.%"), 2);
    ok(e, "workspace.remove", json!({"workspace_id": 1, "force": true}));
    assert_eq!(stored(e, "guardrails.workspaces.1.%"), 0);
    let ws = ok(e, "workspace.create", json!({"path": f.ws}));
    assert_eq!(ws["id"], 2);
    assert_eq!(ok(e, "project.add", json!({"workspace_id": 2, "path": f.repo}))["id"], 4);
}

#[test]
fn a_removal_backs_the_store_up_first() {
    let f = fixture();
    let e = &f.engine;
    ok(e, "notes.create", json!({"project_id": 1, "title": "keep me", "body": "precious"}));
    ok(e, "project.remove", json!({"project_id": 1}));
    let backups: Vec<PathBuf> = std::fs::read_dir(f.root.join("store").join("backups")).unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with("-project-remove.db"))
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    let copy = rusqlite::Connection::open(&backups[0]).unwrap();
    let kept: String = copy.query_row("SELECT body FROM notes WHERE project_id = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, "precious");
}

#[test]
fn a_v3_import_keeps_attachments_inside_and_refuses_unusable_caps() {
    let f = fixture();
    let e = &f.engine;
    let dir = f.repo.join(".relay");
    std::fs::create_dir_all(dir.join("attachments")).unwrap();
    std::fs::write(dir.join("attachments/shot.png"), b"png").unwrap();
    std::fs::write(f.root.join("secret.txt"), b"private key").unwrap();
    let c = rusqlite::Connection::open(dir.join("relay.db")).unwrap();
    c.execute_batch(&format!(r#"
        CREATE TABLE columns (id INTEGER PRIMARY KEY, name TEXT NOT NULL, position INTEGER NOT NULL);
        CREATE TABLE modules (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL);
        CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
            acceptance_criteria TEXT NOT NULL DEFAULT '', target_files TEXT NOT NULL DEFAULT '', size_hint TEXT,
            status TEXT NOT NULL DEFAULT 'idle', column_id INTEGER NOT NULL, module_id INTEGER,
            priority TEXT NOT NULL DEFAULT 'medium', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
        CREATE TABLE attachments (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL, path TEXT NOT NULL, original_name TEXT NOT NULL, created_at INTEGER NOT NULL);
        CREATE TABLE comments (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL);
        INSERT INTO columns VALUES (1,'Backlog',0);
        INSERT INTO tasks (id,title,column_id,created_at,updated_at) VALUES (1,'One',1,1786000000,1786000000);
        INSERT INTO attachments VALUES (1,1,'attachments/shot.png','../../../escaped.png',1786000001),
            (2,1,'attachments/shot.png','sub/escaped.png',1786000002),
            (3,1,'{}','secret.txt',1786000003);
    "#, f.root.join("secret.txt").display())).unwrap();
    drop(c);
    let sessions = f.root.join("sessions.json");
    std::fs::write(&sessions, json!([{"name": "app", "repo": f.repo, "max_files": 0, "max_lines": -5}]).to_string()).unwrap();
    std::env::set_var("RELAY_V3_SESSIONS_JSON", &sessions);

    let out = ok(e, "app.import.v3", json!({"source": dir, "project_id": 1}));
    let warnings = out["warnings"].to_string();
    assert!(warnings.contains("outside the repository"), "{warnings}");
    assert!(warnings.contains("max_files") && warnings.contains("max_lines"), "{warnings}");
    assert_eq!(stored(e, "guardrails.projects.1.%"), 0);

    let attachments = f.root.join("store").join("attachments");
    let conn = e.store.lock();
    let mut stmt = conn.prepare("SELECT name, path FROM attachments ORDER BY id").unwrap();
    let rows: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].0, "escaped.png");
    for (_, path) in &rows {
        assert!(Path::new(path).starts_with(&attachments), "{path}");
    }
    // The second file of the same name is kept beside the first, not over it.
    assert!(rows[1].1.ends_with("escaped-1.png"), "{rows:?}");
    assert!(!f.root.join("escaped.png").exists());
}
