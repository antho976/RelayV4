//! Opt-in screenshot run. The only clock in the client is this verification path.
use crate::app::Ui;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::json;
use std::rc::Rc;
use std::time::Duration;
#[path = "smoke_notes.rs"]
mod notes;

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

fn record_geometry(ui: &Ui, screenshot: &str) {
    if let Ok(requested) = std::env::var("RELAY_NATIVE_PAGE") {
        if matches!(
            requested.as_str(),
            "board" | "modules" | "settings" | "skills" | "dashboard"
        ) {
            assert_eq!(
                *ui.page.borrow(),
                requested,
                "Capture must show the requested page"
            );
        }
    }
    fn collect(
        widget: &gtk::Widget,
        window: &gtk::ApplicationWindow,
        rows: &mut Vec<serde_json::Value>,
    ) {
        if !widget.is_mapped() {
            return;
        }
        if let Some(bounds) = widget.compute_bounds(window) {
            rows.push(json!({
                "type": widget.type_().name(), "name": widget.widget_name().to_string(),
                "classes": widget.css_classes().iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                "text": widget.downcast_ref::<gtk::Label>().map(|l| l.text().to_string()),
                "x": bounds.x(), "y": bounds.y(), "width": bounds.width(), "height": bounds.height()
            }));
        }
        let mut child = widget.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            collect(&item, window, rows);
        }
    }
    let context = ui.window.pango_context();
    let font = context
        .load_font(&gtk::pango::FontDescription::from_string("Fira Mono 10"))
        .expect("Bundled monospace font must load");
    assert_eq!(
        font.describe().family().as_deref(),
        Some("Fira Mono"),
        "Do not silently substitute a different terminal font"
    );
    let mut widgets = Vec::new();
    collect(ui.window.upcast_ref(), &ui.window, &mut widgets);
    for (class, dimension, expected) in [
        ("topbar", "height", 42.),
        ("statusbar", "height", 24.),
        ("files-rail", "width", 28.),
        ("umd", "height", 26.),
    ] {
        for widget in &widgets {
            if widget["classes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == class)
            {
                assert_eq!(
                    widget[dimension].as_f64(),
                    Some(expected),
                    "Reference shell geometry: {class}"
                );
            }
        }
    }
    if *ui.page.borrow() == "settings" {
        assert!(
            !widgets.iter().any(|w| w["classes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == "sidebar")),
            "Settings owns the full workspace"
        );
    }
    if *ui.page.borrow() == "plan" {
        assert!(
            !widgets.iter().any(|w| w["name"] == "note-title"),
            "Plan must not expose another document's title editor"
        );
    }
    if *ui.page.borrow() == "dashboard" {
        let page = widgets
            .iter()
            .find(|w| {
                w["classes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c == "dashboard-page")
            })
            .unwrap();
        assert!(
            page["x"].as_f64().unwrap() + page["width"].as_f64().unwrap()
                <= f64::from(ui.window.width()),
            "Dashboard must fit beside the sidebar"
        );
    }
    let path = std::path::Path::new(screenshot).with_extension("geometry.json");
    std::fs::write(path, serde_json::to_vec_pretty(&widgets).unwrap())
        .expect("Save widget geometry");
}

fn verify_settings_save(ui: Rc<Ui>) {
    let font = named(&ui.window, "setting:terminal.font_size")
        .unwrap()
        .downcast::<gtk::SpinButton>()
        .unwrap();
    let opacity = named(&ui.window, "setting:appearance.panel_alpha")
        .unwrap()
        .downcast::<gtk::Scale>()
        .unwrap();
    let threshold = named(&ui.window, "guardrail-field:destructive_write.min_removed_pct")
        .unwrap()
        .downcast::<gtk::SpinButton>()
        .unwrap();
    font.set_value(9.75);
    opacity.set_value(0.96);
    threshold.set_value(62.5);
    // Guardrails save on their own: the layered editor sends only the fields that changed.
    named(&ui.window, "guardrail-save")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    named(&ui.window, "settings-save")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    assert!(
        !ui.pages["settings"].is_sensitive(),
        "Save must lock its snapshot while persisting"
    );
    glib::timeout_add_local_once(Duration::from_millis(900), move || {
        glib::spawn_future_local(async move {
            assert!(
                ui.pages["settings"].is_sensitive(),
                "Settings save must finish"
            );
            let font = ui
                .call("settings.get", json!({"path":"terminal.font_size"}))
                .await
                .unwrap();
            let opacity = ui
                .call("settings.get", json!({"path":"appearance.panel_alpha"}))
                .await
                .unwrap();
            let guardrails = ui.call("guardrail.config.get", json!({})).await.unwrap();
            assert_eq!(font["value"].as_f64(), Some(9.75));
            assert_eq!(opacity["value"].as_f64(), Some(0.96));
            assert_eq!(
                guardrails["destructive_write"]["min_removed_pct"].as_f64(),
                Some(62.5)
            );
            println!("Settings save verified across categories, including fractional values");
        });
    });
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
                    if page == "board" {
                        named(&window, "draft-save")
                            .unwrap()
                            .downcast::<gtk::Button>()
                            .unwrap()
                            .emit_clicked();
                    } else {
                        // The Notes editor saves through its window action, not Draft's key.
                        window
                            .activate_action("notes.save", None)
                            .expect("Notes window has a save action");
                    }
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
    let fixture = std::env::var("RELAY_NATIVE_FIXTURE").as_deref() == Ok("1");
    // A fixture, a page or a roadmap part writes through the engine: it types into
    // terminals, edits notes, tasks, files and settings, and launches sessions. Arm any
    // of them only against a disposable engine the driver started under the temp
    // directory, never the live one an inherited variable happens to reach. A bare
    // screenshot (with VERIFY_CONNECTION or VERIFY_CONTRAST) only reads, so it may
    // look at any engine.
    let acts = fixture
        || std::env::var_os("RELAY_NATIVE_PAGE").is_some()
        || std::env::var_os("RELAY_NATIVE_ROADMAP").is_some();
    let isolated = ui.path.starts_with(std::env::temp_dir())
        && !ui.path.components().any(|c| c == std::path::Component::ParentDir);
    if acts && !isolated {
        eprintln!(
            "Smoke harness refused: RELAY_NATIVE_FIXTURE/PAGE/ROADMAP act on the engine, and {} is not under {}. Point RELAY_NATIVE_SOCKET at a disposable engine there.",
            ui.path.display(),
            std::env::temp_dir().display()
        );
        std::process::exit(1);
    }
    // A panic in a detached GLib future must fail the fixture, rather than leave
    // an idle window until the harness timeout.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        std::process::exit(1);
    }));
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
    if let Ok(part) = std::env::var("RELAY_NATIVE_ROADMAP") {
        assert!(fixture, "Roadmap smoke parts run with RELAY_NATIVE_FIXTURE=1");
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            glib::spawn_future_local(async move {
                let result = match part.as_str() {
                    "notes" => notes::run(&ui).await,
                    "files" => crate::smoke_project_files::run(&ui).await,
                    "lifecycle" => crate::smoke_project_files::profile_lifecycle(&ui).await,
                    "registry" => crate::smoke_registry::run(&ui).await,
                    "tools" => {
                        let value = crate::roadmap_smoke::run(&ui).await;
                        println!("ROADMAP_TOOLS={value}");
                        Ok(())
                    }
                    _ => Err(format!("Unknown roadmap smoke part: {part}")),
                };
                if let Err(error) = result {
                    panic!("Roadmap regression {part}: {error}");
                }
                ui.window.close();
                assert!(
                    !ui.window.is_visible(),
                    "Roadmap test left a dirty draft: {}",
                    ui.notice.text()
                );
                println!("ROADMAP_OK={part}");
            });
        });
        return;
    }
    if fixture {
        crate::pages::verify_note_tools();
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
            // In Settings this control is Back. Enter the wall before testing
            // the sidebar toggle, then navigate to the requested capture page.
            navigate.navigate("agents");
            navigate.verify_shell();
            if std::env::var("RELAY_NATIVE_PAGE").as_deref() == Ok("agents") {
                verify_confirmations(navigate.clone());
            }
            let toggle = named(&navigate.window, "sidebar-toggle")
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap();
            let visible = navigate.sidebar.is_visible();
            toggle.emit_clicked();
            toggle.emit_clicked();
            assert_eq!(navigate.sidebar.is_visible(), visible);
        }
        if let Ok(page) = std::env::var("RELAY_NATIVE_PAGE") {
            if page == "workspace-project-submit" {
                named(&navigate.window, "new-session")
                    .unwrap()
                    .downcast::<gtk::Button>()
                    .unwrap()
                    .emit_clicked();
                assert!(
                    named(&navigate.window, "setup-local-repos").is_some(),
                    "New Session must guide an empty workspace to project selection"
                );
                if let Ok(path) = std::env::var("RELAY_NATIVE_SETUP_PATH") {
                    named(&navigate.window, "setup-path")
                        .unwrap()
                        .downcast::<gtk::Entry>()
                        .unwrap()
                        .set_text(&path);
                }
                let ui = navigate.clone();
                glib::timeout_add_local_once(Duration::from_millis(100), move || {
                    // Let GTK allocate the newly mounted setup form before pointer checks.
                    click_control(&ui, "setup-scan");
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

            if page == "mirror" {
                // No adb in the fixture: the docked mirror shows its rail and a problem card.
                crate::mirror::prefer_dock();
                if let Ok(avd) = std::env::var("RELAY_NATIVE_MIRROR_AVD") {
                    crate::mirror::open_avd(&navigate, avd, false);
                } else {
                    let device = std::env::var("RELAY_NATIVE_MIRROR_DEVICE").unwrap_or_else(|_| "fixture-serial".into());
                    crate::mirror::open(&navigate, device);
                }
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
                if named(&navigate.window, "setup-add").is_none() {
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
                    click_control(&navigate, "setup-scan");
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
            if page == "settings" && fixture {
                let ui = navigate.clone();
                glib::timeout_add_local_once(Duration::from_millis(700), move || {
                    verify_settings_save(ui)
                });
            }
        }
    });
    glib::timeout_add_local_once(Duration::from_secs(duration), move || {
        if fixture && *ui.page.borrow() == "board" {
            assert!(
                ui.dismiss_panels(),
                "Saved task should close back to the board"
            );
        }
        for name in ["setup-scan", "setup-path", "setup-local-path", "setup-add"] {
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
            1 + usize::from(
                ui.notes_window
                    .borrow()
                    .as_ref()
                    .is_some_and(|w| w.window.is_visible())
            ),
            "Only the main workspace and retained Notes window may be visible"
        );
        // WidgetPaintable can have no node between invalidation and GTK's next
        // frame (notably after a Code save). Capture a rendered frame, bounded.
        let mut attempts = 0;
        glib::timeout_add_local(Duration::from_millis(50), move || {
            attempts += 1;
            let target: gtk::Window =
                if std::env::var("RELAY_NATIVE_PAGE").as_deref() == Ok("notes") {
                    ui.notes_window
                        .borrow()
                        .as_ref()
                        .expect("Notes window")
                        .window
                        .clone()
                        .upcast()
                } else {
                    ui.window.clone().upcast()
                };
            let paintable = gtk::WidgetPaintable::new(Some(&target));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, target.width() as f64, target.height() as f64);
            let (Some(node), Some(renderer)) = (snapshot.to_node(), target.renderer()) else {
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
            record_geometry(&ui, &path);
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
