//! Registry drag/drop persists order without changing project or deleting children.
use crate::app::Ui;
use crate::smoke::util::{drag_payload, drop_on, named, require, wait_for, WAIT};
use gtk4::prelude::*;
use serde_json::{json, Value};
use std::{rc::Rc, time::{Duration, Instant}};

async fn call(ui: &Rc<Ui>, op: &str, value: Value) -> Result<Value, String> {
    ui.call(op, value).await.map_err(|e| e.to_string())
}
/// A registry row a person could drag onto: drawn from the refreshed list, and not
/// locked by a reorder still saving (which makes the registry insensitive).
fn row_ready(ui: &Ui, name: &str) -> bool {
    named(&ui.window, name).is_some_and(|row| row.is_sensitive())
}
/// Drags `source` onto `target` through the rows' own DragSource and DropTarget.
async fn reorder(ui: &Rc<Ui>, kind: &str, source: i64, target: i64) -> Result<(), String> {
    let (from, to) = (format!("registry-{kind}-{source}"), format!("registry-{kind}-{target}"));
    wait_for(|| row_ready(ui, &from) && row_ready(ui, &to), "Registry rows drawn").await?;
    let payload = drag_payload(&named(&ui.window, &from).unwrap())?;
    require(drop_on(&named(&ui.window, &to).unwrap(), payload)?, "Registry rejected reorder")
}
/// Reads `op` until `done` holds: the drop handler saves each row's order in turn.
async fn until(ui: &Rc<Ui>, op: &str, done: impl Fn(&Value) -> bool, reason: &str) -> Result<Value, String> {
    let deadline = Instant::now() + WAIT;
    loop {
        let value = call(ui, op, json!({})).await?;
        if done(&value) {
            return Ok(value);
        }
        require(Instant::now() < deadline, reason)?;
        glib::timeout_future(Duration::from_millis(50)).await;
    }
}

pub async fn run(ui: &Rc<Ui>) -> Result<(), String> {
    let project = ui.project.get();
    let sidebar_width = ui.sidebar.width();
    let ws = ui.workspaces.borrow().first().cloned().ok_or("Fixture workspace missing")?;
    let ws_id = ws["id"].as_i64().unwrap();
    let parent = std::path::Path::new(ws["path"].as_str().unwrap());
    assert!(parent.starts_with(std::env::temp_dir()));
    let repo = parent.join("registry-reorder");
    std::fs::create_dir(&repo).map_err(|e| e.to_string())?;
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@relay.test",
            "commit",
            "--allow-empty",
            "-qm",
            "Fixture",
        ],
    ] {
        let result = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err(String::from_utf8_lossy(&result.stderr).into());
        }
    }
    let second = call(ui, "project.add", json!({"workspace_id":ws_id,"path":repo,"name":"Long project title repeated for width verification without expanding the sidebar"})).await?;
    let second_id = second["id"].as_i64().unwrap();
    ui.refresh();
    reorder(ui, "project", second_id, project).await?;
    let order = |list: &Value, id: i64| {
        list["projects"].as_array().and_then(|items| items.iter().find(|p| p["id"] == id))
            .and_then(|p| p["order"].as_i64())
    };
    let moved = |list: &Value| matches!((order(list, second_id), order(list, project)), (Some(a), Some(b)) if a < b);
    until(ui, "project.list", moved, "Reorder did not persist").await?;
    // The reorder saves one row at a time and unlocks the registry when it is done.
    wait_for(|| row_ready(ui, &format!("registry-project-{project}")), "Reorder finished").await?;
    assert_eq!(
        ui.project.get(),
        project,
        "Reorder changed the selected project"
    );
    assert_eq!(
        ui.sidebar.width(),
        sidebar_width,
        "Long names expanded the sidebar"
    );
    assert!(
        ui.call("project.remove", json!({"project_id":project}))
            .await
            .is_err(),
        "Project removal closed active sessions"
    );
    assert!(
        ui.call("workspace.remove", json!({"workspace_id":ws_id}))
            .await
            .is_err(),
        "Workspace removal deleted its projects"
    );
    call(ui, "project.remove", json!({"project_id":second_id})).await?;
    let extra_path = parent.parent().unwrap().join("reorder-workspace");
    std::fs::create_dir(&extra_path).map_err(|e| e.to_string())?;
    let extra = call(ui, "workspace.create", json!({"path":extra_path})).await?;
    let extra_id = extra["id"].as_i64().unwrap();
    ui.refresh();
    reorder(ui, "workspace", extra_id, ws_id).await?;
    until(ui, "workspace.list", |spaces| spaces["workspaces"][0]["id"] == extra_id, "Workspace reorder did not persist").await?;
    wait_for(|| row_ready(ui, &format!("registry-workspace-{ws_id}")), "Workspace reorder finished").await?;
    call(ui, "workspace.remove", json!({"workspace_id":extra_id})).await?;
    println!("REGISTRY_REORDER_OK");
    Ok(())
}
