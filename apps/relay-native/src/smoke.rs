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
                            .set_text("Unsaved task fixture");
                    } else {
                        widget
                            .downcast::<gtk::TextView>()
                            .unwrap()
                            .buffer()
                            .set_text("Native note save verified.");
                    }
                    if page == "board" {
                        let panel = ui
                            .panels
                            .borrow()
                            .last()
                            .cloned()
                            .expect("Task editor is in the main window");
                        panel.close();
                        assert!(
                            !ui.panels.borrow().is_empty(),
                            "A dirty task must not close"
                        );
                        named(&window, "task-title")
                            .unwrap()
                            .downcast::<gtk::Entry>()
                            .unwrap()
                            .set_text("Native task edit verified");
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

fn verify_confirmations(ui: Rc<Ui>) {
    glib::spawn_future_local(async move {
        for accept in [false, true] {
            let panel = crate::panel::Panel::new(&ui, "Confirmation fixture", 480);
            panel
                .body
                .append(&gtk::Label::new(Some("Fixture only. No mutation.")));
            let pending = panel.clone();
            glib::idle_add_local_once(move || {
                if accept {
                    named(&pending.body, "panel-accept")
                        .unwrap()
                        .downcast::<gtk::Button>()
                        .unwrap()
                        .emit_clicked();
                } else {
                    pending.close();
                }
            });
            assert_eq!(panel.response("Confirm").await, accept);
            assert!(ui.panels.borrow().is_empty());
        }
        println!("In-app confirmation accept and cancel verified");
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
        if fixture {
            navigate.verify_shell();
            if std::env::var("RELAY_NATIVE_PAGE").as_deref() == Ok("agents") {
                verify_confirmations(navigate.clone());
            }
            let toggle = named(&navigate.window, "sidebar-toggle")
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap();
            toggle.emit_clicked();
            toggle.emit_clicked();
        }
        if let Ok(page) = std::env::var("RELAY_NATIVE_PAGE") {
            if matches!(page.as_str(), "palette" | "layouts") {
                named(
                    &navigate.window,
                    if page == "palette" {
                        "command-palette"
                    } else {
                        "window-presets"
                    },
                )
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap()
                .emit_clicked();
                return;
            }
            if page == "launch-preview" {
                navigate.show_launch(None);
                return;
            }
            navigate.navigate(if page == "launch" { "agents" } else { &page });
            if fixture && matches!(page.as_str(), "board" | "notes" | "launch") {
                edit_fixture(navigate.clone(), page.clone());
            }
            if fixture && std::env::var("RELAY_NATIVE_BURST").as_deref() == Ok("1") {
                let ui = navigate.clone();
                // Layout checks reconnect hidden panes. Wait for their fixture sockets
                // before feeding input, just as a user waits for a ready terminal.
                glib::timeout_add_local_once(Duration::from_millis(500), move || {
                    ui.verify_burst(false)
                });
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
        assert_eq!(
            gtk::Window::list_toplevels()
                .iter()
                .filter(|w| w.is_visible())
                .count(),
            1,
            "App flows must stay inside the main window"
        );
        // WidgetPaintable can have no node between invalidation and GTK's next
        // frame (notably after a Code save). Capture a rendered frame, bounded.
        let mut attempts = 0;
        glib::timeout_add_local(Duration::from_millis(50), move || {
            attempts += 1;
            let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                ui.window.width() as f64,
                ui.window.height() as f64,
            );
            let (Some(node), Some(renderer)) = (snapshot.to_node(), ui.window.renderer()) else {
                assert!(
                    attempts < 40,
                    "Screenshot failed: no rendered frame after two seconds"
                );
                ui.window.queue_draw();
                return glib::ControlFlow::Continue;
            };
            renderer
                .render_texture(&node, None)
                .save_to_png(&path)
                .expect("Save screenshot");
            println!("Screenshot saved: {path}");
            if fixture && std::env::var("RELAY_NATIVE_PAGE").as_deref() == Ok("launch-preview") {
                ui.verify_launch();
            }
            ui.window.close();
            assert!(
                !ui.window.is_visible(),
                "Native close blocked: {}",
                ui.notice.text()
            );
            glib::ControlFlow::Break
        });
    });
}
