//! Opt-in screenshot run. The only clock in the client is this verification path.
use crate::app::Ui;
use gtk::prelude::*;
use gtk4 as gtk;
use std::rc::Rc;
use std::time::Duration;

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
            navigate.navigate(&page);
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
