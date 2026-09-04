//! Opt-in screenshot run. The only clock in the client is this verification path.
use crate::app::Ui;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::json;
use std::rc::Rc;
use std::time::Duration;

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

fn edit_fixture(ui: Rc<Ui>, page: String) {
    glib::spawn_future_local(async move {
        let project = ui.project.get();
        if page == "board" {
            let data = ui
                .call("task.list", json!({"project_id":project}))
                .await
                .unwrap();
            crate::pages::open_task(&ui, data["tasks"][0]["id"].as_i64().unwrap());
        } else if page == "notes" {
            let data = ui
                .call("notes.list", json!({"project_id":project}))
                .await
                .unwrap();
            crate::pages::open_note(&ui, data["notes"][0].clone());
        } else if page == "launch" {
            let task = ui
                .call(
                    "task.create",
                    json!({"project_id":project,"title":"Native launch queue","column":"ready"}),
                )
                .await
                .unwrap();
            ui.show_launch(task["id"].as_i64());
        }
        glib::timeout_add_local_once(Duration::from_millis(700), move || {
            if page == "launch" {
                named(&ui.window, "launch-mode")
                    .unwrap()
                    .downcast::<gtk::DropDown>()
                    .unwrap()
                    .set_selected(1);
                named(&ui.window, "launch-builders")
                    .unwrap()
                    .downcast::<gtk::DropDown>()
                    .unwrap()
                    .set_selected(1);
                named(&ui.window, "launch-start")
                    .unwrap()
                    .downcast::<gtk::Button>()
                    .unwrap()
                    .emit_clicked();
                return;
            }
            for window in gtk::Window::list_toplevels() {
                let field = if page == "board" {
                    "task-title"
                } else {
                    "note-body"
                };
                if let Some(widget) = named(&window, field) {
                    if page == "board" {
                        widget
                            .downcast::<gtk::Entry>()
                            .unwrap()
                            .set_text("Native task edit verified");
                    } else {
                        widget
                            .downcast::<gtk::TextView>()
                            .unwrap()
                            .buffer()
                            .set_text("Native note save verified.");
                    }
                    named(&window, "draft-save")
                        .unwrap()
                        .downcast::<gtk::Button>()
                        .unwrap()
                        .emit_clicked();
                    break;
                }
            }
        });
    });
}

pub fn install(ui: &Rc<Ui>) {
    let Ok(path) = std::env::var("RELAY_NATIVE_SCREENSHOT") else {
        return;
    };
    if let Ok(size) = std::env::var("RELAY_NATIVE_SIZE") {
        if let Some((w, h)) = size.split_once(',') {
            if let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) {
                ui.window.set_default_size(w, h);
            }
        }
    }
    let ui = ui.clone();
    let fixture = std::env::var("RELAY_NATIVE_FIXTURE").as_deref() == Ok("1");
    if fixture {
        let input = ui.clone();
        glib::timeout_add_local_once(Duration::from_secs(1), move || {
            input.verify_terminal_input()
        });
    }
    let duration = std::env::var("RELAY_NATIVE_SMOKE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let navigate = ui.clone();
    glib::timeout_add_local_once(Duration::from_secs(2), move || {
        if let Ok(page) = std::env::var("RELAY_NATIVE_PAGE") {
            navigate.navigate(if page == "launch" { "agents" } else { &page });
            if fixture && matches!(page.as_str(), "board" | "notes" | "launch") {
                edit_fixture(navigate.clone(), page.clone());
            }
            if fixture && std::env::var("RELAY_NATIVE_BURST").as_deref() == Ok("1") {
                navigate.verify_burst(false);
            }
            if page == "code" && fixture {
                navigate.editor.verify_open(&navigate);
                let ui = navigate.clone();
                glib::timeout_add_local_once(Duration::from_millis(500), move || {
                    ui.editor.verify_save(&ui)
                });
            }
        }
    });
    glib::timeout_add_local_once(Duration::from_secs(duration), move || {
        if std::env::var("RELAY_NATIVE_VERIFY_CONNECTION").as_deref() == Ok("1") {
            assert!(
                ui.client.borrow().is_some() && !ui.notice.is_visible(),
                "Native engine connection failed: {}",
                ui.notice.text()
            );
            println!("Native engine connection verified: {}", ui.path.display());
        }
        if fixture && std::env::var("RELAY_NATIVE_BURST").as_deref() == Ok("1") {
            ui.verify_burst(true);
        }
        let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            ui.window.width() as f64,
            ui.window.height() as f64,
        );
        if let (Some(node), Some(renderer)) = (snapshot.to_node(), ui.window.renderer()) {
            let texture = renderer.render_texture(&node, None);
            match texture.save_to_png(&path) {
                Ok(()) => println!("Screenshot saved: {path}"),
                Err(e) => eprintln!("Screenshot failed: {e}"),
            }
        } else {
            eprintln!("Screenshot failed: window has no render node");
        }
        ui.window.close();
    });
}
