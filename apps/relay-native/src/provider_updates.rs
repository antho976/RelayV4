//! One opt-in update attempt per application launch, with engine-owned execution.
use crate::app::{text, Ui};
use serde_json::{json, Value};
use std::rc::Rc;

pub(crate) fn startup(ui: &Rc<Ui>) {
    if ui.provider_updates_checked.replace(true) {
        return;
    }
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let Ok(config) = ui.call("settings.get", json!({"path":"providers"})).await else {
            // Nothing was attempted (the engine dropped right after connect); the next connect
            // tries again rather than skipping the update for the rest of the launch.
            ui.provider_updates_checked.set(false);
            return;
        };
        for provider in ["claude", "codex"] {
            if config["value"][provider]["auto_update"] == true {
                match ui
                    .call(
                        "provider.update",
                        json!({"provider":provider,"automatic":true}),
                    )
                    .await
                {
                    Ok(result) if result["started"] != true => {
                        ui.show_error(&format!("{provider}: {}", text(&result, "message")))
                    }
                    Err(error) => ui.show_error(&format!("{provider} update: {error}")),
                    _ => {}
                }
            }
        }
    });
}

pub(crate) fn update(ui: &Rc<Ui>, provider: &'static str) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match ui
            .call("provider.update", json!({"provider":provider}))
            .await
        {
            Ok(result) => ui.show_error(text(&result, "message")),
            Err(error) => ui.show_error(&format!("{provider} update: {error}")),
        }
    });
}

pub(crate) fn event(ui: &Rc<Ui>, value: &Value) {
    ui.show_error(&format!(
        "{}: {}",
        text(value, "provider"),
        text(value, "message")
    ));
    if text(value, "state") != "updating" {
        ui.refresh();
    }
}
