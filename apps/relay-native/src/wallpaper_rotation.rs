//! One window-owned timer; only the canonical wallpaper changes between rotations.
use crate::app::Ui;
use crate::client::Error;
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Offer bundled gradients only before the library has been configured.
pub(crate) fn library_or_defaults(library: &Value) -> Value {
    use base64::Engine;
    if !library.is_null() {
        return library.clone();
    }
    let presets: [(&str, &str, &[u8]); 3] = [
        (
            "charcoal",
            "Charcoal",
            include_bytes!("../resources/wallpapers/charcoal.png"),
        ),
        (
            "teal",
            "Deep teal",
            include_bytes!("../resources/wallpapers/teal.png"),
        ),
        (
            "indigo",
            "Midnight indigo",
            include_bytes!("../resources/wallpapers/indigo.png"),
        ),
    ];
    presets
        .into_iter()
        .map(|(id, name, bytes)| {
            let image = format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            json!({"id":format!("builtin-{id}"),"name":name,"image":image,"preview":image})
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct Rotation {
    timer: Option<glib::SourceId>,
    signature: u64,
    revision: u64,
    running: bool,
}

impl Drop for Rotation {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
    }
}

fn signature(config: &Value, library: &Value) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    config.to_string().hash(&mut hash);
    library.to_string().hash(&mut hash);
    hash.finish()
}

fn images(library: &Value) -> Vec<&str> {
    let mut images = Vec::new();
    if let Some(library) = library.as_array() {
        for item in library {
            if let Some(image) = item["image"]
                .as_str()
                .filter(|image| image.starts_with("data:image/"))
            {
                if !images.contains(&image) {
                    images.push(image);
                }
            }
        }
    }
    images
}

pub(crate) fn refresh(ui: &Rc<Ui>) {
    let revision = {
        let mut state = ui.wallpaper_rotation.borrow_mut();
        state.revision += 1;
        state.revision
    };
    let generation = ui.generation.get();
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let (config, library) = tokio::join!(
            ui.call(
                "settings.get",
                json!({"path":"appearance.wallpaper_rotation"})
            ),
            ui.call("settings.get", json!({"path":"appearance.wallpapers"}))
        );
        if generation != ui.generation.get() || revision != ui.wallpaper_rotation.borrow().revision
        {
            return;
        }
        if let (Ok(config), Ok(library)) = (config, library) {
            configure(&ui, &config["value"], &library["value"]);
        }
    });
}

fn configure(ui: &Rc<Ui>, config: &Value, library: &Value) {
    let signature = signature(config, library);
    let mut state = ui.wallpaper_rotation.borrow_mut();
    if state.signature == signature {
        return;
    }
    state.signature = signature;
    if let Some(timer) = state.timer.take() {
        timer.remove();
    }
    if config["enabled"] != true || images(library).len() < 2 {
        return;
    }
    let minutes = config["interval_minutes"]
        .as_u64()
        .unwrap_or(15)
        .clamp(1, 1440);
    let weak = Rc::downgrade(ui);
    state.timer = Some(glib::timeout_add_local(
        std::time::Duration::from_secs(minutes * 60),
        move || {
            let Some(ui) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if ui.wallpaper_rotation.borrow().running {
                return glib::ControlFlow::Continue;
            }
            ui.wallpaper_rotation.borrow_mut().running = true;
            glib::spawn_future_local(async move {
                if let Err(error) = rotate_once(&ui).await {
                    ui.show_error(&format!("Wallpaper rotation failed: {error}"));
                }
                ui.wallpaper_rotation.borrow_mut().running = false;
            });
            glib::ControlFlow::Continue
        },
    ));
}

pub(crate) async fn rotate_once(ui: &Rc<Ui>) -> Result<bool, Error> {
    let generation = ui.generation.get();
    let revision = ui.wallpaper_rotation.borrow().revision;
    let (config, library, current) = tokio::join!(
        ui.call(
            "settings.get",
            json!({"path":"appearance.wallpaper_rotation"})
        ),
        ui.call("settings.get", json!({"path":"appearance.wallpapers"})),
        ui.call("settings.get", json!({"path":"appearance.wallpaper"}))
    );
    let (config, library, current) = (config?, library?, current?);
    if generation != ui.generation.get() || revision != ui.wallpaper_rotation.borrow().revision {
        return Ok(false);
    }
    if config["value"]["enabled"] != true {
        return Ok(false);
    }
    let images = images(&library["value"]);
    if images.len() < 2 {
        return Ok(false);
    }
    let candidates: Vec<_> = images
        .into_iter()
        .filter(|image| Some(*image) != current["value"].as_str())
        .collect();
    let index = uuid::Uuid::new_v4().as_u128() % candidates.len() as u128;
    ui.call(
        "settings.set",
        json!({"path":"appearance.wallpaper","value":candidates[index as usize]}),
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_library_only_fills_an_unset_library() {
        let defaults = library_or_defaults(&Value::Null);
        assert_eq!(defaults.as_array().unwrap().len(), 3);
        assert_eq!(images(&defaults).len(), 3);
        assert_eq!(library_or_defaults(&json!([])), json!([]));
        let custom = json!([{"id":"mine","image":"data:image/png;base64,custom"}]);
        assert_eq!(library_or_defaults(&custom), custom);
    }
    #[test]
    fn rotation_counts_distinct_images_and_ignores_metadata() {
        let library = json!([{ "image":"data:image/png;base64,one" }, {"image":"data:image/png;base64,one"}, {"image":"data:image/png;base64,two"}, {"image":null}]);
        assert_eq!(images(&library).len(), 2);
        let config = json!({"enabled":true,"interval_minutes":15});
        assert_ne!(
            signature(&config, &library),
            signature(&json!({"enabled":false}), &library)
        );
    }
}
