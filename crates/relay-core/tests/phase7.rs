//! Phase 7: board, dispatch, modules, changelog, and undo. Assertions cross the bus door.

use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap()
        .success());
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn call(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Response {
    engine.dispatch(Request::new(actor, op, payload), Door::InProcess)
}
fn ok(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Value {
    call(engine, actor, op, payload)
        .into_result()
        .unwrap_or_else(|e| panic!("{op}: {} {}", e.code, e.message))
}
fn code(response: Response) -> String {
    response.error.expect("expected error").code
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
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "phase7@relay.test"]);
        git(&repo, &["config", "user.name", "Phase Seven"]);
        std::fs::write(repo.join("README.md"), "phase 7\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let engine = Engine::new(
            Instance::Test,
            Store::open(&root.path().join("store/store.db"), false).unwrap(),
        );
        ok(&engine, Actor::User, "workspace.create", json!({"path":ws}));
        ok(
            &engine,
            Actor::User,
            "project.add",
            json!({"workspace_id":1,"path":repo}),
        );
        Self {
            _root: root,
            engine,
        }
    }
}

#[test]
fn board_crud_attachments_and_atomic_undo() {
    let f = Fixture::new();
    let e = &f.engine;
    let task = ok(
        e,
        Actor::User,
        "task.create",
        json!({"project_id":1,"title":"Board spine","body":"Build it","priority":"high","size":"M","attachments":[{"name":"proof.txt","mime":"text/plain","bytes_b64":"aGVsbG8="}]}),
    );
    assert_eq!(task["column"], "backlog");
    assert_eq!(task["position"], 0);
    assert_eq!(task["attachments"][0]["bytes"], 5);
    assert!(Path::new(task["attachments"][0]["path"].as_str().unwrap()).is_file());
    let second = ok(
        e,
        Actor::User,
        "task.create",
        json!({"project_id":1,"title":"Second"}),
    );
    assert_eq!(second["position"], 1);
    let moved = ok(
        e,
        Actor::User,
        "task.move",
        json!({"task_id":task["id"],"column":"ready"}),
    );
    assert_eq!(moved["column"], "ready");
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.move",
            json!({"task_id":task["id"],"column":"done"})
        )),
        "task.column_transition"
    );
    let copy = ok(
        e,
        Actor::User,
        "task.copy_text",
        json!({"task_id":task["id"]}),
    );
    assert!(copy["text"]
        .as_str()
        .unwrap()
        .starts_with("#1 Board spine\n\nBuild it"));

    let rows = ok(
        e,
        Actor::User,
        "audit.list",
        json!({"op_prefix":"task.create","limit":10}),
    );
    let create_id = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["result_summary"]["id"] == second["id"])
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let undone = ok(e, Actor::User, "audit.undo", json!({"audit_id":create_id}));
    assert_eq!(undone["undone"], create_id);
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "task.get",
            json!({"task_id":second["id"]})
        )),
        "task.not_found"
    );
    let undo_row = ok(
        e,
        Actor::User,
        "audit.get",
        json!({"audit_id":undone["by"]}),
    );
    assert_eq!(undo_row["undo_of"], create_id);
    assert_eq!(undo_row["undo_op"]["op"], "task.restore");
    ok(
        e,
        Actor::User,
        "audit.undo",
        json!({"audit_id":undone["by"]}),
    );
    assert_eq!(
        ok(e, Actor::User, "task.get", json!({"task_id":second["id"]}))["title"],
        "Second"
    );

    let updated = ok(
        e,
        Actor::User,
        "task.update",
        json!({"task_id":task["id"],"title":"New title"}),
    );
    let update_row = ok(
        e,
        Actor::User,
        "audit.list",
        json!({"op_prefix":"task.update","limit":1}),
    )["rows"][0]["id"]
        .as_i64()
        .unwrap();
    ok(
        e,
        Actor::User,
        "task.update",
        json!({"task_id":task["id"],"body":"later edit"}),
    );
    assert_eq!(
        code(call(
            e,
            Actor::User,
            "audit.undo",
            json!({"audit_id":update_row})
        )),
        "audit.stale"
    );
    ok(
        e,
        Actor::User,
        "audit.undo",
        json!({"audit_id":update_row,"force":true}),
    );
    assert_ne!(
        updated["title"],
        ok(e, Actor::User, "task.get", json!({"task_id":task["id"]}))["title"]
    );
}

#[test]
fn dispatch_done_approve_and_agent_scope() {
    let f = Fixture::new();
    let e = &f.engine;
    let module = ok(
        e,
        Actor::User,
        "module.create",
        json!({"project_id":1,"name":"Phase Seven","icon":"board","priority":"urgent"}),
    );
    let task = ok(
        e,
        Actor::User,
        "task.create",
        json!({"project_id":1,"module_id":module["id"],"title":"Dispatch me","body":"Implement the flow","changelog":"Added the task board."}),
    );
    let session = ok(
        e,
        Actor::User,
        "session.create",
        json!({"project_id":1,"provider":"codex","role":"builder"}),
    );
    let name = session["name"].as_str().unwrap();
    ok(
        e,
        Actor::agent(name),
        "session.report",
        json!({"session":name,"kind":"session_start"}),
    );
    let dispatched = ok(
        e,
        Actor::User,
        "task.dispatch",
        json!({"task_id":task["id"],"session":name}),
    );
    assert_eq!(dispatched["task"]["column"], "active");
    assert_eq!(dispatched["task"]["state"], "dispatched");
    assert_eq!(dispatched["task"]["sessions"][0], name);
    let brief = ok(
        e,
        Actor::agent(name),
        "session.brief",
        json!({"session":name}),
    );
    assert!(brief["text"]
        .as_str()
        .unwrap()
        .contains(&format!("Task #{} [CURRENT]: Dispatch me", task["id"])));
    ok(
        e,
        Actor::agent(name),
        "session.report",
        json!({"session":name,"kind":"tool_use"}),
    );
    assert_eq!(
        ok(e, Actor::User, "task.get", json!({"task_id":task["id"]}))["state"],
        "running"
    );
    let mut quiet_events = e.subscribe();
    ok(
        e,
        Actor::agent(name),
        "session.report",
        json!({"session":name,"kind":"tool_use"}),
    );
    assert!(
        quiet_events.try_recv().is_err(),
        "an unchanged tool report must not publish UI work"
    );
    ok(
        e,
        Actor::agent(name),
        "session.report",
        json!({"session":name,"kind":"blocked","data":{"message":"needs input"}}),
    );
    assert_eq!(
        ok(e, Actor::User, "task.get", json!({"task_id":task["id"]}))["state"],
        "blocked"
    );
    assert_eq!(
        code(call(
            e,
            Actor::agent(name),
            "task.update",
            json!({"task_id":task["id"],"title":"agent rename"})
        )),
        "actor.allowlist"
    );
    ok(
        e,
        Actor::agent(name),
        "task.update",
        json!({"task_id":task["id"],"body":"Implemented"}),
    );
    ok(
        e,
        Actor::agent(name),
        "session.done",
        json!({"session":name,"summary":"done","sha":"abc123"}),
    );
    let review = ok(e, Actor::User, "task.get", json!({"task_id":task["id"]}));
    assert_eq!(review["column"], "in_review");
    assert_eq!(review["state"], "awaiting_review");
    let done = ok(
        e,
        Actor::User,
        "task.approve",
        json!({"task_id":task["id"],"sha":"def456"}),
    );
    assert_eq!(done["column"], "done");
    assert_eq!(done["commits"].as_array().unwrap().len(), 2);
}

#[test]
fn approve_uses_the_recorded_branch_after_the_assigned_session_is_closed() {
    let f = Fixture::new();
    let e = &f.engine;
    let task = ok(
        e,
        Actor::User,
        "task.create",
        json!({"project_id":1,"title":"Approve after cleanup","column":"ready"}),
    );
    let session = ok(
        e,
        Actor::User,
        "session.create",
        json!({"project_id":1,"provider":"codex","role":"builder","task_id":task["id"]}),
    );
    let name = session["name"].as_str().unwrap();
    let branch = session["branch"].as_str().unwrap();
    ok(
        e,
        Actor::User,
        "session.close",
        json!({"session":name}),
    );

    let done = ok(
        e,
        Actor::User,
        "task.approve",
        json!({"task_id":task["id"]}),
    );
    assert_eq!(done["column"], "done");
    assert_eq!(done["commits"].as_array().unwrap().len(), 1);
    assert_eq!(done["commits"][0]["branch"], branch);
    assert_eq!(
        done["commits"][0]["sha"],
        git_output(&f._root.path().join("ws/app"), &["rev-parse", "HEAD"])
    );

    let approve_audit = ok(
        e,
        Actor::User,
        "audit.list",
        json!({"op_prefix":"task.approve","limit":1}),
    );
    ok(
        e,
        Actor::User,
        "audit.undo",
        json!({"audit_id":approve_audit["rows"][0]["id"]}),
    );
    assert_eq!(
        ok(e, Actor::User, "task.get", json!({"task_id":task["id"]}))["column"],
        "ready"
    );
}

#[test]
fn approve_without_a_session_links_the_project_head() {
    let f = Fixture::new();
    let e = &f.engine;
    let task = ok(
        e,
        Actor::User,
        "task.create",
        json!({"project_id":1,"title":"Completed directly","column":"ready"}),
    );

    let done = ok(
        e,
        Actor::User,
        "task.approve",
        json!({"task_id":task["id"]}),
    );
    assert_eq!(done["column"], "done");
    assert_eq!(done["commits"].as_array().unwrap().len(), 1);
    assert_eq!(done["commits"][0]["branch"], "main");
    assert_eq!(
        done["commits"][0]["sha"],
        git_output(&f._root.path().join("ws/app"), &["rev-parse", "HEAD"])
    );
}

#[test]
fn module_stats_detail_archive_and_changelog() {
    let f = Fixture::new();
    let e = &f.engine;
    let module = ok(
        e,
        Actor::User,
        "module.create",
        json!({"project_id":1,"name":"Release 4.0","icon":"rocket","priority":"high"}),
    );
    let a = ok(
        e,
        Actor::User,
        "task.create",
        json!({"project_id":1,"module_id":module["id"],"title":"Finished","column":"in_review","priority":"urgent","changelog":"Shipped the board."}),
    );
    ok(
        e,
        Actor::User,
        "task.approve",
        json!({"task_id":a["id"],"sha":"aaa"}),
    );
    ok(
        e,
        Actor::User,
        "task.create",
        json!({"project_id":1,"module_id":module["id"],"title":"Open","column":"active","priority":"low"}),
    );
    let list = ok(e, Actor::User, "module.list", json!({"project_id":1}));
    assert_eq!(list["header"]["count"], 1);
    assert_eq!(list["header"]["in_flight"], 1);
    assert_eq!(list["header"]["issues"], 2);
    assert_eq!(list["header"]["completed"], 1);
    assert_eq!(list["modules"][0]["progress_pct"], 50.0);
    let detail = ok(
        e,
        Actor::User,
        "module.get",
        json!({"module_id":module["id"]}),
    );
    assert_eq!(
        detail["tasks_by_state"]["done"].as_array().unwrap().len(),
        1
    );
    let draft = ok(
        e,
        Actor::User,
        "module.changelog.draft",
        json!({"module_id":module["id"]}),
    );
    assert!(draft["markdown"]
        .as_str()
        .unwrap()
        .contains("## Urgent\n- Shipped the board."));
    ok(
        e,
        Actor::User,
        "module.delete",
        json!({"module_id":module["id"]}),
    );
    assert!(ok(e, Actor::User, "task.get", json!({"task_id":a["id"]}))["module_id"].is_null());
    ok(
        e,
        Actor::User,
        "module.restore",
        json!({"module_id":module["id"]}),
    );
    assert_eq!(
        ok(e, Actor::User, "task.get", json!({"task_id":a["id"]}))["module_id"],
        module["id"]
    );
    ok(
        e,
        Actor::User,
        "module.complete",
        json!({"module_id":module["id"]}),
    );
    assert!(
        ok(e, Actor::User, "module.list", json!({"project_id":1}))["modules"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ok(
            e,
            Actor::User,
            "module.list",
            json!({"project_id":1,"include_archived":true})
        )["modules"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
