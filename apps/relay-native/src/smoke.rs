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

fn verify_pointer_target(ui: &Ui, name: &str) {
    let target = named(&ui.window, name).unwrap();
    assert!(
        target.is_sensitive() && target.is_mapped(),
        "{name} must be interactive"
    );
    let bounds = target.compute_bounds(&ui.window).unwrap();
    let hit = ui
        .window
        .pick(
            (bounds.x() + bounds.width() / 2.0) as f64,
            (bounds.y() + bounds.height() / 2.0) as f64,
            gtk::PickFlags::DEFAULT,
        )
        .expect("Control center must hit a widget");
    assert!(
        hit == target || hit.is_ancestor(&target),
        "{name} is blocked by {} ({})",
        hit.type_().name(),
        hit.widget_name()
    );
    println!("Pointer target verified: {name}");
}

fn click_control(ui: &Ui, name: &str) {
    verify_pointer_target(ui, name);
    let target = named(&ui.window, name).unwrap();
    if let Some(driver) = std::env::var_os("RELAY_NATIVE_POINTER_DRIVER") {
        let bounds = target.compute_bounds(&ui.window).unwrap();
        let status = std::process::Command::new("python3")
            .arg(driver)
            .arg(ui.window.title().unwrap())
            .arg(((bounds.x() + bounds.width() / 2.0) as i32).to_string())
            .arg(((bounds.y() + bounds.height() / 2.0) as i32).to_string())
            .status()
            .unwrap();
        assert!(status.success(), "Pointer injection failed for {name}");
        println!("Mouse click sent: {name}");
    } else {
        target.downcast::<gtk::Button>().unwrap().emit_clicked();
    }
}

fn verify_control_contrast(root: &impl IsA<gtk::Widget>) {
    fn walk(widget: &gtk::Widget, highlighted: bool, light_tabs: bool) {
        let light_tabs = light_tabs
            || widget.has_css_class("utility-panel")
            || widget.has_css_class("setup-body");
        let highlighted = highlighted
            || (widget.is::<gtk::Button>() && widget.has_css_class("primary"))
            || (light_tabs
                && widget.is::<gtk::Button>()
                && widget.state_flags().contains(gtk::StateFlags::CHECKED)
                && widget
                    .parent()
                    .is_some_and(|p| p.is::<gtk::StackSwitcher>()));
        if highlighted && (widget.is::<gtk::Label>() || widget.is::<gtk::Image>()) {
            let original = widget.state_flags();
            for state in [
                gtk::StateFlags::NORMAL,
                gtk::StateFlags::BACKDROP,
                gtk::StateFlags::PRELIGHT,
                gtk::StateFlags::ACTIVE,
                gtk::StateFlags::INSENSITIVE,
            ] {
                widget.set_state_flags(original | state, true);
                let color = widget.style_context().color();
                assert!(
                    color.red().max(color.green()).max(color.blue()) < 0.3 && color.alpha() > 0.95,
                    "Light button content must stay dark in {state:?}: {color:?}"
                );
            }
            widget.set_state_flags(original, true);
        }
        let mut child = widget.first_child();
        while let Some(item) = child {
            walk(&item, highlighted, light_tabs);
            child = item.next_sibling();
        }
    }
    walk(root.as_ref(), false, false);
    println!("Primary and selected-tab contrast verified in normal, inactive, hover, pressed and disabled states");
}

fn verify_panel_toggles(ui: &Rc<Ui>) {
    fn click(ui: &Ui, name: &str) {
        named(&ui.window, name)
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
    }
    for control in [
        "status-usage",
        "status-resources",
        "status-devices",
        "command-palette",
        "window-presets",
    ] {
        for index in 0..50 {
            click(ui, control);
            assert_eq!(
                ui.panels.borrow().len(),
                (index + 1) % 2,
                "{control} must toggle, never stack"
            );
        }
        click(ui, control);
        click(ui, "panel-close");
        assert!(ui.panels.borrow().is_empty(), "One X closes {control}");
    }
    click(ui, "status-usage");
    click(ui, "status-resources");
    assert_eq!(
        ui.panels.borrow().len(),
        1,
        "Different controls replace the old panel"
    );
    let guarded = ui.panels.borrow().last().unwrap().clone();
    guarded.set_guard(|| false);
    click(ui, "status-resources");
    click(ui, "window-presets");
    assert_eq!(
        ui.panels.borrow().len(),
        1,
        "Busy or dirty panels cannot be replaced"
    );
    assert!(Rc::ptr_eq(ui.panels.borrow().last().unwrap(), &guarded));
    guarded.set_guard(|| true);
    guarded.close();
    click(ui, "nav-board");
    assert_eq!(ui.page.borrow().as_str(), "board");
    click(ui, "nav-board");
    assert_eq!(ui.page.borrow().as_str(), "agents");
    println!("Panel toggles verified: 50 clicks per control, one-close, switching, guards and page toggle");
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
    if std::env::var_os("RELAY_NATIVE_POINTER_DRIVER").is_some() {
        ui.window
            .set_title(Some(&format!("Relay pointer smoke {}", std::process::id())));
    }
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
            if page == "workspace-project-submit" {
                named(&navigate.window, "new-session")
                    .unwrap()
                    .downcast::<gtk::Button>()
                    .unwrap()
                    .emit_clicked();
                assert!(
                    named(&navigate.window, "setup-local-path").is_some(),
                    "New Session must guide an empty workspace to project selection"
                );
                navigate.dismiss_panels();
                named(&navigate.window, "workspace-add-1")
                    .unwrap()
                    .downcast::<gtk::Button>()
                    .unwrap()
                    .emit_clicked();
                let ui = navigate.clone();
                glib::timeout_add_local_once(Duration::from_millis(700), move || {
                    let key = named(&ui.window, "setup-add")
                        .unwrap()
                        .downcast::<gtk::Button>()
                        .unwrap();
                    assert!(
                        key.is_sensitive(),
                        "The discovered local project must be actionable"
                    );
                    click_control(&ui, "setup-add");
                });
                return;
            }
            if page == "project-launch" {
                named(&navigate.window, "new-session")
                    .unwrap()
                    .downcast::<gtk::Button>()
                    .unwrap()
                    .emit_clicked();
                let ui = navigate.clone();
                glib::timeout_add_local_once(Duration::from_millis(700), move || {
                    let start = named(&ui.window, "launch-start")
                        .unwrap()
                        .downcast::<gtk::Button>()
                        .unwrap();
                    assert!(start.is_sensitive());
                    click_control(&ui, "launch-start");
                });
                return;
            }

            if page == "toggles" {
                verify_panel_toggles(&navigate);
                return;
            }

            if matches!(page.as_str(), "device-run" | "device-release" | "resources") {
                if page == "resources" {
                    navigate.resources();
                } else {
                    crate::tools::devices::open(&navigate);
                    if page == "device-run" {
                        let ui = navigate.clone();
                        glib::timeout_add_local_once(Duration::from_millis(700), move || {
                            named(&ui.window, "device-refresh")
                                .unwrap()
                                .downcast::<gtk::Button>()
                                .unwrap()
                                .emit_clicked();
                            assert_eq!(
                                ui.panels.borrow().len(),
                                1,
                                "Device refresh must keep a panel open"
                            );
                        });
                    }
                    if page == "device-release" {
                        named(&navigate.window, "device-tabs")
                            .unwrap()
                            .downcast::<gtk::Stack>()
                            .unwrap()
                            .set_visible_child_name("release");
                    }
                }
                return;
            }
            if page.starts_with("setup") {
                if named(&navigate.window, "setup-continue").is_none() {
                    navigate.open_repository();
                }
                if page == "setup" {
                    return;
                }
                let navigate = navigate.clone();
                glib::timeout_add_local_once(Duration::from_millis(100), move || {
                    if let Ok(path) = std::env::var("RELAY_NATIVE_SETUP_PATH") {
                        named(&navigate.window, "setup-path")
                            .unwrap()
                            .downcast::<gtk::Entry>()
                            .unwrap()
                            .set_text(&path);
                    }
                    click_control(&navigate, "setup-continue");
                    let ui = navigate.clone();
                    glib::timeout_add_local_once(Duration::from_millis(500), move || {
                        if page.starts_with("setup-github") {
                            named(&ui.window, "setup-source")
                                .unwrap()
                                .downcast::<gtk::Stack>()
                                .unwrap()
                                .set_visible_child_name("github");
                        }
                        if page.contains("connect") {
                            let pending = ui.clone();
                            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                                named(&pending.window, "setup-connect")
                                    .unwrap()
                                    .downcast::<gtk::Button>()
                                    .unwrap()
                                    .emit_clicked();
                            });
                        }
                        if page.ends_with("submit") {
                            glib::timeout_add_local_once(Duration::from_millis(1000), move || {
                                let key = named(&ui.window, "setup-add")
                                    .unwrap()
                                    .downcast::<gtk::Button>()
                                    .unwrap();
                                assert!(
                                    key.is_sensitive(),
                                    "A selected repository must be actionable"
                                );
                                click_control(&ui, "setup-add");
                            });
                        }
                    });
                });
                return;
            }

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
        for name in [
            "setup-continue",
            "setup-path",
            "setup-local-path",
            "setup-add",
        ] {
            if named(&ui.window, name).is_some_and(|w| w.is_mapped() && w.is_sensitive()) {
                verify_pointer_target(&ui, name);
            }
        }
        if std::env::var_os("RELAY_NATIVE_VERIFY_CONTRAST").is_some() {
            verify_control_contrast(&ui.window);
        }

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
