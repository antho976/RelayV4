//! Phase 9 bus coverage: chrome state, layouts, notifications, dashboard, usage, resources.

use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::process::Command;
use std::sync::Arc;

fn engine() -> Arc<Engine> {
    Engine::new(Instance::Test, Store::open_memory().unwrap())
}
fn call(engine: &Engine, op: &str, payload: Value) -> Response {
    engine.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}
fn git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn project(engine: &Engine) -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    let repo = workspace.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    call(engine, "workspace.create", json!({"path":workspace.path()}))
        .into_result()
        .unwrap();
    call(engine, "project.add", json!({"workspace_id":1,"path":repo}))
        .into_result()
        .unwrap();
    workspace
}

#[test]
fn notifications_settings_dashboard_usage_and_resources() {
    let engine = engine();
    let _workspace = project(&engine);
    let session = call(
        &engine,
        "session.create",
        json!({"project_id":1,"provider":"claude","role":"builder"}),
    )
    .into_result()
    .unwrap();
    let name = session["name"].as_str().unwrap();
    {
        let conn = engine.store.lock();
        conn.execute(
            "UPDATE sessions SET state='running',pid=NULL WHERE id=1",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO notifications(project_id,category,title,body,link,read,created_at) VALUES (1,'system','Ready to test','Phase 9 is ready',NULL,0,'2026-08-17T12:00:00Z')", []).unwrap();
    }
    call(
        &engine,
        "task.create",
        json!({"project_id":1,"title":"Review shell","column":"in_review"}),
    )
    .into_result()
    .unwrap();
    let list = call(&engine, "notify.list", json!({"unread_only":true}))
        .into_result()
        .unwrap();
    assert_eq!(list["notifications"].as_array().unwrap().len(), 1);
    call(&engine, "notify.ack", json!({"notification_id":1}))
        .into_result()
        .unwrap();
    assert!(call(&engine, "notify.list", json!({"unread_only":true}))
        .into_result()
        .unwrap()["notifications"]
        .as_array()
        .unwrap()
        .is_empty());
    let settings = call(
        &engine,
        "notify.settings.set",
        json!({"patch":{"sound":false,"categories":{"disk":false}}}),
    )
    .into_result()
    .unwrap();
    assert_eq!(settings["sound"], false);
    assert_eq!(settings["categories"]["disk"], false);
    let report = engine.dispatch(
        Request::new(
            Actor::agent(name),
            "usage.report",
            json!({"session":name,"provider":"claude","payload":{"five_hour":{"used_pct":41}}}),
        ),
        Door::InProcess,
    );
    report.into_result().unwrap();
    let usage = call(&engine, "usage.get", json!({"provider":"claude"}))
        .into_result()
        .unwrap();
    assert_eq!(usage["usage"][0]["windows"]["five_hour"]["used_pct"], 41);
    let dashboard = call(&engine, "dashboard.get", json!({}))
        .into_result()
        .unwrap();
    assert_eq!(dashboard["sessions_live"][0]["session"], name);
    assert_eq!(dashboard["in_review"][0]["title"], "Review shell");
    assert_eq!(dashboard["projects"][0]["name"], "repo");
    assert_eq!(dashboard["projects"][0]["in_review"], 1);
    assert_eq!(dashboard["projects"][0]["live_sessions"], 1);
    let resources = call(&engine, "app.resources.get", json!({}))
        .into_result()
        .unwrap();
    assert_eq!(resources["panes"][0]["session"], name);
    call(&engine, "app.resources.watch", json!({"on":true}))
        .into_result()
        .unwrap();
    call(&engine, "app.resources.watch", json!({"on":false}))
        .into_result()
        .unwrap();
}

#[test]
fn ui_state_panes_windows_and_durable_layouts() {
    let engine = engine();
    let _workspace = project(&engine);
    call(
        &engine,
        "ui.page.switch",
        json!({"page":"notes","project_id":1}),
    )
    .into_result()
    .unwrap();
    assert_eq!(
        call(&engine, "ui.state", json!({})).into_result().unwrap()["page"],
        "notes"
    );
    call(
        &engine,
        "ui.page.switch",
        json!({"page":"plan","project_id":1}),
    )
    .into_result()
    .unwrap();
    let first = call(
        &engine,
        "ui.pane.open",
        json!({"kind":"terminal","target":{"session":"calm-otter"}}),
    )
    .into_result()
    .unwrap();
    let second = call(&engine, "ui.pane.open", json!({"kind":"notes"}))
        .into_result()
        .unwrap();
    call(
        &engine,
        "ui.pane.move",
        json!({"pane":second["pane"],"to":first["pane"],"edge":"left"}),
    )
    .into_result()
    .unwrap();
    call(&engine, "ui.pane.focus", json!({"pane":first["pane"]}))
        .into_result()
        .unwrap();
    let popped = call(&engine, "ui.window.popout", json!({"pane":second["pane"]}))
        .into_result()
        .unwrap();
    assert!(popped["window_id"].as_str().unwrap().starts_with("popout-"));
    call(
        &engine,
        "ui.window.close",
        json!({"window_id":popped["window_id"]}),
    )
    .into_result()
    .unwrap();
    let state = call(&engine, "ui.state", json!({})).into_result().unwrap();
    assert_eq!(state["page"], "plan");
    assert_eq!(state["panes"].as_array().unwrap().len(), 2);
    call(
        &engine,
        "ui.layout.save",
        json!({"project_id":1,"name":"Review","state":{"page":"board","agent_layout":"review"}}),
    )
    .into_result()
    .unwrap();
    let layouts = call(&engine, "ui.layout.list", json!({"project_id":1}))
        .into_result()
        .unwrap();
    assert_eq!(layouts["layouts"], json!(["Review"]));
    let mut events = engine.subscribe();
    call(
        &engine,
        "ui.layout.apply",
        json!({"project_id":1,"name":"Review"}),
    )
    .into_result()
    .unwrap();
    let applied = events.try_recv().unwrap();
    assert_eq!(applied.ev, "layout.changed");
    assert_eq!(applied.payload["state"]["agent_layout"], "review");
    call(
        &engine,
        "ui.layout.delete",
        json!({"project_id":1,"name":"Review"}),
    )
    .into_result()
    .unwrap();
    assert!(call(&engine, "ui.layout.list", json!({"project_id":1}))
        .into_result()
        .unwrap()["layouts"]
        .as_array()
        .unwrap()
        .is_empty());
    call(&engine, "ui.pane.close", json!({"pane":first["pane"]}))
        .into_result()
        .unwrap();
}
