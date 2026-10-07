//! Low-severity audit fixes in usage.get, the guardrail lists and confirm, and the ui model.

use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git").arg("-C").arg(repo).args(args).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

fn call(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Response {
    engine.dispatch(Request::new(actor, op, payload), Door::InProcess)
}

fn ok(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Value {
    call(engine, actor, op, payload)
        .into_result()
        .unwrap_or_else(|error| panic!("{op} failed: {} {}", error.code, error.message))
}

/// A workspace with two committed repositories, projects 1 and 2.
fn fixture() -> (tempfile::TempDir, Arc<Engine>) {
    let root = tempfile::tempdir().unwrap();
    let ws = root.path().join("ws");
    let store = Store::open(&root.path().join("store/store.db"), false).unwrap();
    let engine = Engine::new(Instance::Test, store);
    for name in ["app", "lib"] {
        let repo = ws.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "audit@relay.test"]);
        git(&repo, &["config", "user.name", "Audit"]);
        std::fs::write(repo.join("README.md"), "relay\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
    }
    ok(&engine, Actor::User, "workspace.create", json!({"path": ws}));
    ok(&engine, Actor::User, "project.add", json!({"workspace_id": 1, "path": ws.join("app")}));
    ok(&engine, Actor::User, "project.add", json!({"workspace_id": 1, "path": ws.join("lib")}));
    (root, engine)
}

fn session(engine: &Engine, project_id: i64, provider: &str) -> Value {
    ok(engine, Actor::User, "session.create", json!({"project_id": project_id, "provider": provider, "role": "builder"}))
}

fn drain(rx: &mut tokio::sync::broadcast::Receiver<relay_bus::Event>) -> Vec<relay_bus::Event> {
    let mut out = Vec::new();
    while let Ok(event) = rx.try_recv() {
        out.push(event);
    }
    out
}

/// RA-392: the reported fallback is the newest report, dated when it was reported, not
/// whichever session last changed state.
#[test]
fn usage_get_ranks_and_dates_reports_by_when_they_were_reported() {
    let (_root, engine) = fixture();
    let older = session(&engine, 1, "claude");
    let newer = session(&engine, 1, "claude");
    let report = |session: &Value, pct: i64| {
        let name = session["name"].as_str().unwrap();
        ok(&engine, Actor::agent(name), "usage.report",
            json!({"session": name, "provider": "claude", "payload": {"five_hour": {"used_pct": pct}}}));
    };
    report(&older, 10);
    report(&newer, 20);
    {
        let conn = engine.store.lock();
        // Back-date both reports, then let the older session change state much later.
        conn.execute("UPDATE sessions SET usage=json_set(usage,'$.relay_reported_at','2026-01-01T00:00:00Z') WHERE id=?1",
            [older["id"].as_i64().unwrap()]).unwrap();
        conn.execute("UPDATE sessions SET usage=json_set(usage,'$.relay_reported_at','2026-01-02T00:00:00Z') WHERE id=?1",
            [newer["id"].as_i64().unwrap()]).unwrap();
        conn.execute("UPDATE sessions SET updated_at='2026-06-01T00:00:00Z' WHERE id=?1", [older["id"].as_i64().unwrap()]).unwrap();
    }
    let usage = ok(&engine, Actor::User, "usage.get", json!({"provider": "claude"}));
    let usage = usage["usage"].as_array().unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0]["windows"]["five_hour"]["used_pct"], 20, "a state change made a stale report win");
    assert_eq!(usage[0]["taken_at"], "2026-01-02T00:00:00Z");
    assert!(usage[0]["windows"].get("relay_reported_at").is_none(), "the report time is not a window");
}

/// RA-379: an agent sees holds and exception requests in its own project only (D106).
#[test]
fn agents_list_holds_and_requests_in_their_own_project_only() {
    let (_root, engine) = fixture();
    let mine = session(&engine, 1, "claude");
    let theirs = session(&engine, 2, "claude");
    let ask = |session: &Value| {
        let name = session["name"].as_str().unwrap();
        ok(&engine, Actor::agent(name), "guardrail.request", json!({
            "kind": "command", "value": "git push --force origin main", "reason": "the branch was rebased upstream",
        }))["request"]["id"].as_i64().unwrap()
    };
    let own = ask(&mine);
    let other = ask(&theirs);
    let agent = Actor::agent(mine["name"].as_str().unwrap());

    let ids = |value: &Value, key: &str| -> Vec<i64> {
        value[key].as_array().unwrap().iter().map(|item| item["id"].as_i64().unwrap()).collect()
    };
    assert_eq!(ids(&ok(&engine, agent.clone(), "guardrail.holds.list", json!({})), "holds"), vec![own]);
    assert_eq!(ids(&ok(&engine, agent.clone(), "guardrail.requests.list", json!({})), "requests"), vec![own]);
    for (op, payload) in [
        ("guardrail.holds.list", json!({"project_id": 2})),
        ("guardrail.requests.list", json!({"project_id": 2})),
        ("guardrail.request.get", json!({"request_id": other})),
    ] {
        let refused = call(&engine, agent.clone(), op, payload);
        assert_eq!(refused.error.as_ref().map(|e| e.code.as_str()), Some("actor.scope"), "{op}");
    }
    assert_eq!(ok(&engine, agent, "guardrail.request.get", json!({"request_id": own}))["id"], own);
    // A person still sees everything.
    assert_eq!(ids(&ok(&engine, Actor::User, "guardrail.holds.list", json!({})), "holds"), vec![other, own]);
}

/// RA-380: a hold made while confirming another is announced like any other hold.
#[test]
fn a_hold_made_by_confirming_a_gate_hold_is_announced() {
    let (_root, engine) = fixture();
    ok(&engine, Actor::User, "guardrail.config.set", json!({"project_id": 1, "patch": {"protected_paths": ["secret/**"]}}));
    let session = session(&engine, 1, "codex");
    let name = session["name"].as_str().unwrap();
    let worktree = PathBuf::from(session["worktree"].as_str().unwrap());
    std::fs::create_dir_all(worktree.join("secret")).unwrap();
    std::fs::write(worktree.join("secret/large.txt"), (0..100).map(|line| format!("line-{line}\n")).collect::<String>()).unwrap();

    // A person's protected-path write is held; once that is waived, the rewrite size holds it.
    let held = call(&engine, Actor::User, "guardrail.gate",
        json!({"session": name, "kind": "write", "path": "secret/large.txt", "new_text": "short\n"}));
    let first = held.error.as_ref().unwrap().confirm.as_ref().unwrap().payload["hold_id"].as_i64().unwrap();
    let mut rx = engine.subscribe();
    let confirmed = ok(&engine, Actor::User, "guardrail.confirm", json!({"hold_id": first}));
    let next = confirmed["outcome"]["error"]["confirm"]["payload"]["hold_id"].as_i64()
        .unwrap_or_else(|| panic!("confirm made no second hold: {confirmed}"));
    let seen = drain(&mut rx);
    assert!(seen.iter().any(|event| event.ev == "guardrail.held" && event.payload["hold_id"] == next), "{seen:?}");
    assert!(seen.iter().any(|event| event.ev == "notify.new" && event.payload["hold_id"] == next), "{seen:?}");
}

/// RA-419: saving without a state saves what the native client last stored for the project.
#[test]
fn layout_save_without_state_saves_the_clients_arrangement() {
    let (_root, engine) = fixture();
    let arrangement = json!({"page": "code", "agent_layout": "focus", "columns": 3});
    ok(&engine, Actor::User, "settings.set", json!({"path": "native.layout.current.1", "value": arrangement}));
    ok(&engine, Actor::User, "ui.layout.save", json!({"project_id": 1, "name": "Mine"}));
    let mut rx = engine.subscribe();
    ok(&engine, Actor::User, "ui.layout.apply", json!({"project_id": 1, "name": "Mine"}));
    let applied = drain(&mut rx).into_iter().find(|event| event.ev == "layout.changed").unwrap();
    assert_eq!(applied.payload["state"], arrangement);

    ok(&engine, Actor::User, "ui.layout.save", json!({"project_id": 2, "name": "Plain"}));
    let mut rx = engine.subscribe();
    ok(&engine, Actor::User, "ui.layout.apply", json!({"project_id": 2, "name": "Plain"}));
    let applied = drain(&mut rx).into_iter().find(|event| event.ev == "layout.changed").unwrap();
    assert_eq!(applied.payload["state"]["agent_layout"], "grid");
}

/// RA-420: the model stays consistent: one pane flagged focused, no empty popouts, and a move
/// lands on the side of its target the edge names.
#[test]
fn the_ui_model_keeps_focus_windows_and_order_consistent() {
    let engine = Engine::new(Instance::Test, Store::open_memory().unwrap());
    let open = || ok(&engine, Actor::User, "ui.pane.open", json!({"kind": "notes"}))["pane"].as_str().unwrap().to_string();
    let (a, b, c) = (open(), open(), open());
    let state = || ok(&engine, Actor::User, "ui.state", json!({}));
    let order = || -> Vec<String> {
        state()["panes"].as_array().unwrap().iter().map(|pane| pane["pane"].as_str().unwrap().to_string()).collect()
    };

    ok(&engine, Actor::User, "ui.pane.close", json!({"pane": c}));
    let now = state();
    assert_eq!(now["focused"], b.as_str());
    let flagged: Vec<&Value> = now["panes"].as_array().unwrap().iter().filter(|pane| pane["focused"] == true).collect();
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0]["pane"], b.as_str());

    let mut rx = engine.subscribe();
    ok(&engine, Actor::User, "ui.pane.move", json!({"pane": a, "to": b, "edge": "right"}));
    assert_eq!(order(), vec![b.clone(), a.clone()]);
    let changed = drain(&mut rx).into_iter().find(|event| event.ev == "ui.changed").unwrap();
    assert!(changed.payload["panes"].is_array(), "ui.pane.move sends the whole model");
    assert_eq!(changed.payload["move"]["edge"], "right");
    ok(&engine, Actor::User, "ui.pane.move", json!({"pane": a, "to": b, "edge": "left"}));
    assert_eq!(order(), vec![a.clone(), b.clone()]);

    ok(&engine, Actor::User, "ui.window.popout", json!({"pane": a}));
    ok(&engine, Actor::User, "ui.window.popout", json!({"pane": a}));
    let windows = ok(&engine, Actor::User, "ui.window.list", json!({}))["windows"].clone();
    assert_eq!(windows.as_array().unwrap().len(), 2, "an emptied popout stayed: {windows}");
    ok(&engine, Actor::User, "ui.pane.move", json!({"pane": a, "to": b, "edge": "center"}));
    let windows = ok(&engine, Actor::User, "ui.window.list", json!({}))["windows"].clone();
    assert_eq!(windows, json!([{"window_id": "main", "main": true, "panes": [b, a]}]));
}

/// RA-421: reveal never hands a file to its desktop handler.
#[test]
fn reveal_refuses_a_path_whose_parent_is_a_file() {
    let engine = Engine::new(Instance::Test, Store::open_memory().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.jar");
    std::fs::write(&file, "PK").unwrap();
    let refused = call(&engine, Actor::User, "os.reveal", json!({"path": file.join("inside")}));
    assert_eq!(refused.error.as_ref().map(|e| e.code.as_str()), Some("os.path"));
}
