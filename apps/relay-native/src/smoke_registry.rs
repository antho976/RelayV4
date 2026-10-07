//! Registry drag/drop persists order without changing project or deleting children.
use crate::app::Ui;
use gtk4::{self as gtk, prelude::*};
use serde_json::{json, Value};
use std::{rc::Rc, time::Duration};

fn named(root: &impl IsA<gtk::Widget>, name: &str) -> Option<gtk::Widget> {
    let root = root.as_ref();
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = named(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}
async fn call(ui: &Rc<Ui>, op: &str, value: Value) -> Result<Value, String> {
    ui.call(op, value).await.map_err(|e| e.to_string())
}
async fn settle() {
    glib::timeout_future(Duration::from_millis(400)).await;
}
fn drop_on(ui: &Ui, kind: &str, source: i64, target: i64) -> Result<(), String> {
    let row =
        named(&ui.window, &format!("registry-{kind}-{target}")).ok_or("Registry row missing")?;
    let controllers = row.observe_controllers();
    let drop = (0..controllers.n_items())
        .filter_map(|i| controllers.item(i))
        .find_map(|c| c.downcast::<gtk::DropTarget>().ok())
        .ok_or("Registry drop target missing")?;
    let token = glib::BoxedValue(format!("relay-{kind}:{source}").to_value());
    if !drop.emit_by_name::<bool>("drop", &[&token, &0.0_f64, &0.0_f64]) {
        return Err("Registry rejected reorder".into());
    }
    Ok(())
}

pub async fn run(ui: &Rc<Ui>) -> Result<(), String> {
    let project = ui.project.get();
    let sidebar_width = ui.sidebar.width();
    let ws = ui.workspaces.borrow()[0].clone();
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
    settle().await;
    drop_on(ui, "project", second_id, project)?;
    settle().await;
    let list = call(ui, "project.list", json!({})).await?;
    let items = list["projects"].as_array().unwrap();
    let order = |id| {
        items.iter().find(|p| p["id"] == id).unwrap()["order"]
            .as_i64()
            .unwrap()
    };
    assert!(order(second_id) < order(project));
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
    settle().await;
    drop_on(ui, "workspace", extra_id, ws_id)?;
    settle().await;
    let spaces = call(ui, "workspace.list", json!({})).await?;
    assert_eq!(spaces["workspaces"][0]["id"], extra_id);
    call(ui, "workspace.remove", json!({"workspace_id":extra_id})).await?;
    println!("REGISTRY_REORDER_OK");
    Ok(())
}
