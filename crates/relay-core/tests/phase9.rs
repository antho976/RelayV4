//! Phase 9 bus coverage: chrome state, layouts, notifications, dashboard, usage, resources.

mod common;

use common::{call, engine, init_repo};
use relay_bus::{Actor, Request};
use relay_core::engine::{Door, Engine};
use serde_json::json;

fn project(engine: &Engine) -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    let repo = workspace.path().join("repo");
    init_repo(&repo);
    call(engine, "workspace.create", json!({"path":workspace.path()}))
        .into_result()
        .unwrap();
    call(engine, "project.add", json!({"workspace_id":1,"path":repo}))
        .into_result()
        .unwrap();
    workspace
}

/// RA-490 / RA-507: `unread` counts every unread notification under the filters, past
/// `limit` and regardless of `unread_only`; `count_only` returns the count without rows.
#[test]
fn notify_list_counts_unread_beyond_the_page() {
    let engine = engine();
    let _workspace = project(&engine);
    {
        let conn = engine.store.lock();
        for n in 0..5 {
            let (category, read) = if n < 3 { ("system", 0) } else { ("disk", n % 2) };
            conn.execute(
                "INSERT INTO notifications(project_id,category,title,body,link,read,created_at) VALUES (1,?1,'n','',NULL,?2,?3)",
                rusqlite::params![category, read, format!("2026-08-17T12:00:0{n}Z")],
            ).unwrap();
        }
    }
    let ok = |payload: serde_json::Value| call(&engine, "notify.list", payload).into_result().unwrap();
    // Rows 0-2 are unread system ones, 3 is an unread disk one, 4 a read disk one.
    let page = ok(json!({"limit":1}));
    assert_eq!(page["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(page["unread"], 4, "{page}");
    assert_eq!(ok(json!({"unread_only":true,"limit":2}))["unread"], 4);
    assert_eq!(ok(json!({"category":"disk"}))["unread"], 1);
    assert_eq!(ok(json!({"project_id":1,"category":"system"}))["unread"], 3);
    assert_eq!(ok(json!({"project_id":2}))["unread"], 0);
    let count = ok(json!({"count_only":true}));
    assert!(count["notifications"].as_array().unwrap().is_empty(), "{count}");
    assert_eq!(count["unread"], 4);
    call(&engine, "notify.ack_all", json!({})).into_result().unwrap();
    assert_eq!(ok(json!({"count_only":true}))["unread"], 0);
    // An older reply without `unread` still reads as the typed result.
    let old: relay_bus::ops::notify::ListOut = serde_json::from_value(json!({"notifications":[]})).unwrap();
    assert_eq!(old.unread, 0);
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
}

/// The watch loop is a tokio task, so this needs a runtime: on a plain #[test] thread
/// `app.resources.watch` answers Ok and never starts sampling. Client counting and the epoch
/// are covered in socket.rs; this is the loop itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resource_watch_samples_while_on_and_stops_when_off() {
    use std::time::Duration;
    use tokio::sync::broadcast::error::RecvError;
    use tokio::time::{timeout_at, Instant};
    let engine = engine();
    let _workspace = project(&engine);
    let mut events = engine.subscribe();
    call(&engine, "app.resources.watch", json!({"on":true}))
        .into_result()
        .unwrap();
    // One tick every 2 s.
    let deadline = Instant::now() + Duration::from_secs(10);
    let sample = loop {
        match timeout_at(deadline, events.recv()).await.expect("no resource.sample while watching") {
            Ok(event) if event.ev == "resource.sample" => break event.payload,
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => panic!("event bus closed"),
        }
    };
    assert!(sample["panes"].is_array(), "{sample}");
    call(&engine, "app.resources.watch", json!({"on":false}))
        .into_result()
        .unwrap();
    // A tick that read the store just before the switch may still land; let it, then expect
    // silence for two whole periods.
    tokio::time::sleep(Duration::from_millis(500)).await;
    while events.try_recv().is_ok() {}
    let quiet = Instant::now() + Duration::from_secs(4);
    while let Ok(event) = timeout_at(quiet, events.recv()).await {
        if let Ok(event) = event {
            assert_ne!(event.ev, "resource.sample", "sampling continued after the watch was turned off");
        }
    }
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
