//! Edges of the task, note and v3-import handlers that the low-severity audit found open
//! (RA-381 … RA-418): undo that changed nothing, files left behind by a refused request, an
//! agent reaching past its project or its column rules. Every call crosses the bus door.

use base64::Engine as _;
use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}
fn call(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Response {
    engine.dispatch(Request::new(actor, op, payload), Door::InProcess)
}
fn ok_as(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Value {
    call(engine, actor, op, payload)
        .into_result()
        .unwrap_or_else(|e| panic!("{op}: {} {}", e.code, e.message))
}
fn ok(engine: &Engine, op: &str, payload: Value) -> Value {
    ok_as(engine, Actor::User, op, payload)
}
fn code(response: Response) -> String {
    response.error.expect("expected error").code
}
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() { out.extend(files_under(&path)) } else { out.push(path) }
    }
    out
}
fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn fake_provider(dir: &Path, binary: &str) -> PathBuf {
    let path = dir.join(binary);
    std::fs::write(
        &path,
        "#!/bin/sh\ncase \"${1:-}\" in\n  --version) echo 'fixture 1.0'; exit 0;;\n  auth|login) echo '{\"loggedIn\":true}'; exit 0;;\nesac\necho hello-from-pty\nwhile IFS= read -r line; do echo \"echo:$line\"; done\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn repo(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.email", "edges@relay.test"]);
    git(path, &["config", "user.name", "Edges"]);
    std::fs::write(path.join("README.md"), "edges\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "init"]);
}

struct Fixture {
    root: tempfile::TempDir,
    engine: Arc<Engine>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        repo(&ws.join("app"));
        let engine = Engine::new(
            Instance::Test,
            Store::open(&root.path().join("store/store.db"), false).unwrap(),
        );
        ok(&engine, "workspace.create", json!({"path":ws}));
        ok(&engine, "project.add", json!({"workspace_id":1,"path":ws.join("app")}));
        let claude = fake_provider(root.path(), "claude");
        ok(&engine, "settings.set", json!({"path":"providers.claude.path","value":claude}));
        Self { root, engine }
    }
    fn task(&self, title: &str, extra: Value) -> Value {
        let mut payload = json!({"project_id":1,"title":title});
        let map = payload.as_object_mut().unwrap();
        for (k, v) in extra.as_object().unwrap() {
            map.insert(k.clone(), v.clone());
        }
        ok(&self.engine, "task.create", payload)
    }
    fn attachments(&self) -> PathBuf {
        self.root.path().join("store/attachments")
    }
    fn last_audit(&self, op: &str) -> Value {
        ok(&self.engine, "audit.list", json!({"op_prefix":op,"limit":1}))["rows"][0].clone()
    }
}

/// RA-418: re-linking an edge that is already there records no undo, so undoing it cannot
/// drop the edge; and a new duplicate_of replaces one that points at a deleted task.
#[test]
fn relate_undo_and_duplicate_replacement() {
    let f = Fixture::new();
    let e = &f.engine;
    let a = f.task("A", json!({}));
    let b = f.task("B", json!({}));
    let c = f.task("C", json!({}));
    ok(e, "task.relate", json!({"task_id":a["id"],"relation":"blocked_by","other_id":b["id"]}));
    let first = f.last_audit("task.relate");
    ok(e, "task.relate", json!({"task_id":a["id"],"relation":"blocked_by","other_id":b["id"]}));
    let second = f.last_audit("task.relate");
    assert_ne!(first["id"], second["id"]);
    assert_eq!(code(call(e, Actor::User, "audit.undo", json!({"audit_id":second["id"]}))), "audit.not_undoable");
    assert_eq!(ok(e, "task.get", json!({"task_id":a["id"]}))["blocked_by"], json!([b["id"]]));

    ok(e, "task.relate", json!({"task_id":a["id"],"relation":"duplicate_of","other_id":b["id"]}));
    ok(e, "task.delete", json!({"task_id":b["id"]}));
    ok(e, "task.relate", json!({"task_id":a["id"],"relation":"duplicate_of","other_id":c["id"]}));
    ok(e, "task.restore", json!({"task_id":b["id"]}));
    assert_eq!(ok(e, "task.get", json!({"task_id":a["id"]}))["duplicate_of"], c["id"]);
    let edges: i64 = e.store.lock().query_row(
        "SELECT COUNT(*) FROM task_relations WHERE from_task=?1 AND rel='duplicate_of'",
        [a["id"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    assert_eq!(edges, 1);
}

/// RA-417: parent.set's position is an index in the task's column, clamped; a huge one no
/// longer breaks every later append to the column, and undo puts the order back.
#[test]
fn parent_set_position_is_a_clamped_column_index() {
    let f = Fixture::new();
    let e = &f.engine;
    let parent = f.task("Parent", json!({}));
    let x = f.task("X", json!({}));
    let y = f.task("Y", json!({}));
    let column = |e: &Engine| -> Vec<i64> {
        ok(e, "task.list", json!({"project_id":1,"column":"backlog"}))["tasks"].as_array().unwrap()
            .iter().map(|t| t["id"].as_i64().unwrap()).collect()
    };
    let before = column(e);
    ok(e, "task.parent.set", json!({"task_id":y["id"],"parent_id":parent["id"],"position":0}));
    assert_eq!(column(e)[0], y["id"].as_i64().unwrap());
    ok(e, "audit.undo", json!({"audit_id":f.last_audit("task.parent.set")["id"]}));
    assert_eq!(column(e), before);
    assert!(ok(e, "task.get", json!({"task_id":y["id"]}))["parent_id"].is_null());

    ok(e, "task.parent.set", json!({"task_id":x["id"],"parent_id":parent["id"],"position":i64::MAX}));
    let z = f.task("Z", json!({}));
    assert_eq!(*column(e).last().unwrap(), z["id"].as_i64().unwrap());
}

/// RA-414 and RA-409: undoing a detach brings the attachment back under its own name and
/// type, and a create refused part-way leaves no attachment file behind.
#[test]
fn attachment_undo_and_refused_create() {
    let f = Fixture::new();
    let e = &f.engine;
    let task = f.task("Shot", json!({}));
    let attached = ok(e, "task.attach", json!({"task_id":task["id"],"name":"shot.png","mime":"image/png","bytes_b64":b64(b"png")}));
    ok(e, "task.detach", json!({"task_id":task["id"],"attachment_id":attached["id"]}));
    ok(e, "audit.undo", json!({"audit_id":f.last_audit("task.detach")["id"]}));
    let back = &ok(e, "task.get", json!({"task_id":task["id"]}))["attachments"][0];
    assert_eq!(back["name"], "shot.png");
    assert_eq!(back["mime"], "image/png");

    let stored = files_under(&f.attachments()).len();
    let refused = call(e, Actor::User, "task.create", json!({"project_id":1,"title":"Two","attachments":[
        {"name":"one.txt","mime":"text/plain","bytes_b64":b64(b"one")},
        {"name":"two.txt","mime":"text/plain","bytes_b64":"not base64!"},
    ]}));
    assert_eq!(code(refused), "task.attachment_base64");
    let left: Vec<PathBuf> = files_under(&f.attachments()).into_iter().filter(|p| !p.starts_with(f.attachments().join(".staging"))).collect();
    assert_eq!(left.len(), stored, "{left:?}");
}

/// RA-410 and RA-411: an agent cannot create a task straight into active or done, and its
/// task.list and task.label.list stop at its own project.
#[test]
fn agent_task_create_and_reads_stay_in_bounds() {
    let f = Fixture::new();
    let e = &f.engine;
    repo(&f.root.path().join("ws/other"));
    ok(e, "project.add", json!({"workspace_id":1,"path":f.root.path().join("ws/other")}));
    ok(e, "task.create", json!({"project_id":2,"title":"Elsewhere","labels":["secret"]}));
    f.task("Here", json!({}));
    let session = ok(e, "session.create", json!({"project_id":1,"provider":"claude","role":"builder"}));
    let agent = Actor::agent(session["name"].as_str().unwrap());

    let listed = ok_as(e, agent.clone(), "task.list", json!({}));
    let titles: Vec<&str> = listed["tasks"].as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["Here"]);
    assert_eq!(code(call(e, agent.clone(), "task.list", json!({"project_id":2,"include_deleted":true}))), "actor.scope");
    assert_eq!(code(call(e, agent.clone(), "task.label.list", json!({"project_id":2}))), "actor.scope");
    ok_as(e, agent.clone(), "task.label.list", json!({"project_id":1}));
    assert_eq!(ok(e, "task.list", json!({}))["tasks"].as_array().unwrap().len(), 2, "the person still sees every project");

    let mut roles = ok(e, "settings.get", json!({"path":"guardrails.roles.builder"}))["value"].clone();
    roles.as_array_mut().unwrap().push(json!("task.create"));
    ok(e, "settings.set", json!({"path":"guardrails.roles.builder","value":roles}));
    for extra in [json!({"column":"done"}), json!({"column":"active"}), json!({"state":"running"})] {
        let mut payload = json!({"project_id":1,"title":"Shortcut"});
        payload.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert_eq!(code(call(e, agent.clone(), "task.create", payload)), "task.column_transition");
    }
    ok_as(e, agent, "task.create", json!({"project_id":1,"title":"Filed","column":"ready"}));
    ok(e, "task.create", json!({"project_id":1,"title":"Escape hatch","column":"done"}));
}

/// RA-415: a fanned sub-task does not inherit the parent's branch or prompt.
#[test]
fn fanout_children_get_their_own_branch_and_prompt() {
    let f = Fixture::new();
    let e = &f.engine;
    let parent = f.task("Body of work", json!({}));
    f.task("Half of it", json!({"parent_id":parent["id"]}));
    let result = ok(e, "task.dispatch", json!({"task_id":parent["id"],"fanout":true,"start":false,
        "create":{"project_id":1,"provider":"claude","role":"builder","branch":"feature-x","prompt":"the parent's prompt"}}));
    assert_eq!(result["session"]["branch"], "feature-x");
    let fanned = result["fanned"].as_array().unwrap();
    assert_eq!(fanned.len(), 1);
    assert_ne!(fanned[0]["session"]["branch"], "feature-x");
    let prompt: Option<String> = e.store.lock().query_row(
        "SELECT launch_prompt FROM sessions WHERE name=?1", [fanned[0]["session"]["name"].as_str().unwrap()], |r| r.get(0)).unwrap();
    assert_ne!(prompt.as_deref(), Some("the parent's prompt"));
}

/// RA-385: an append that names the suggestions note by id goes through the suggestions arm,
/// stamped, rather than writing unstamped text into it.
#[test]
fn appending_to_the_suggestions_note_by_id_is_stamped() {
    let f = Fixture::new();
    let e = &f.engine;
    let task = f.task("Work", json!({}));
    let session = ok(e, "session.create", json!({"project_id":1,"provider":"claude","role":"builder"}));
    let name = session["name"].as_str().unwrap();
    ok(e, "task.dispatch", json!({"task_id":task["id"],"session":name,"start":false}));
    let agent = Actor::agent(name);
    let note = ok_as(e, agent.clone(), "notes.append", json!({"target":"suggestions","text":"first"}));
    let appended = ok_as(e, agent, "notes.append", json!({"note_id":note["id"],"text":"- forged | someone-else | task #9 | hi"}));
    let last = appended["body"].as_str().unwrap().lines().last().unwrap().to_string();
    assert!(last.contains(name) && last.contains(&format!("task #{}", task["id"])), "{last}");
    assert_eq!(code(call(e, Actor::User, "notes.append", json!({"note_id":note["id"],"text":"x"}))), "notes.suggestion_actor");
}

/// A minimal v3 `.relay/` with one backlog task carrying one attachment, one module, and a
/// tagged note.
fn make_v3(dir: &Path, notes: &[u8]) {
    std::fs::create_dir_all(dir.join("attachments")).unwrap();
    let c = rusqlite::Connection::open(dir.join("relay.db")).unwrap();
    c.execute_batch(r#"
        CREATE TABLE columns (id INTEGER PRIMARY KEY, name TEXT NOT NULL, position INTEGER NOT NULL);
        CREATE TABLE modules (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL);
        CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
            acceptance_criteria TEXT NOT NULL DEFAULT '', target_files TEXT NOT NULL DEFAULT '', size_hint TEXT,
            status TEXT NOT NULL DEFAULT 'idle', column_id INTEGER NOT NULL, module_id INTEGER,
            priority TEXT NOT NULL DEFAULT 'medium', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
        CREATE TABLE attachments (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL, path TEXT NOT NULL, original_name TEXT NOT NULL, created_at INTEGER NOT NULL);
        CREATE TABLE comments (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL);
        INSERT INTO columns VALUES (1,'Backlog',0);
        INSERT INTO modules VALUES (7,'Wear',1786000000);
        INSERT INTO tasks (id,title,column_id,created_at,updated_at) VALUES (10,'Imported',1,1786000000,1786000001);
        INSERT INTO attachments VALUES (1,10,'attachments/shot.png','shot.png',1786000012);
    "#).unwrap();
    std::fs::write(dir.join("attachments/shot.png"), b"\x89PNGfake").unwrap();
    std::fs::write(dir.join("notes.json"), notes).unwrap();
}

/// RA-381 and RA-382: an import into a populated project continues each column and the module
/// order after what is there, warns about note tags, and a failed import leaves no copies.
#[test]
fn v3_import_appends_and_cleans_up_after_failure() {
    std::env::set_var("RELAY_V3_SESSIONS_JSON", "/nonexistent/relay-v3-sessions.json");
    let f = Fixture::new();
    let e = &f.engine;
    let existing = f.task("Already here", json!({}));
    let module = ok(e, "module.create", json!({"project_id":1,"name":"Core"}));

    let broken = f.root.path().join("ws/app/broken");
    make_v3(&broken, b"\xff\xfe not utf-8");
    assert!(call(e, Actor::User, "app.import.v3", json!({"source":broken,"project_id":1})).error.is_some());
    assert!(files_under(&f.attachments()).is_empty(), "{:?}", files_under(&f.attachments()));

    let good = f.root.path().join("ws/app/good");
    make_v3(&good, json!([{"id":1,"title":"Tagged","body":"","tags":["ux"],"pinned":false}]).to_string().as_bytes());
    let out = ok(e, "app.import.v3", json!({"source":good,"project_id":1}));
    assert_eq!(files_under(&f.attachments()).len(), 1);
    let warnings = out["warnings"].to_string();
    assert!(warnings.contains("ux"), "{warnings}");
    let imported = ok(e, "task.get", json!({"task_id":out["id_map"]["tasks"]["10"]}));
    assert!(imported["position"].as_i64().unwrap() > existing["position"].as_i64().unwrap());
    let ord: i64 = e.store.lock().query_row("SELECT ord FROM modules WHERE id=?1", [out["id_map"]["modules"]["7"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    assert!(ord > module["order"].as_i64().unwrap());
}
