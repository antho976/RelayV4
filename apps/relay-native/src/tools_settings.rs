use super::{current, paragraph, section};
use crate::app::{button, clear, field, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub(crate) struct WallpaperDraft {
    state: Rc<RefCell<Value>>,
    saved: Rc<RefCell<Value>>,
    gallery: glib::WeakRef<gtk::Box>,
    /// The saved wallpaper changed while the gallery was hidden: redraw it on the way back.
    stale: std::cell::Cell<bool>,
}

pub(crate) fn sync_wallpaper(ui: &Rc<Ui>, image: &Value) {
    let draft = ui.wallpaper_draft.borrow();
    let Some(draft) = draft.as_ref() else {
        return;
    };
    if draft.saved.borrow()["wallpaper"] == *image {
        return;
    }
    let dirty = draft.state.borrow()["wallpaper"] != draft.saved.borrow()["wallpaper"];
    draft.saved.borrow_mut()["wallpaper"] = image.clone();
    if !dirty {
        draft.state.borrow_mut()["wallpaper"] = image.clone();
        if let Some(gallery) = draft.gallery.upgrade() {
            // Hidden, the gallery waits for Settings to show again instead of redrawing.
            if gallery.is_mapped() {
                render_wallpapers(ui, &gallery, &draft.state);
            } else {
                draft.stale.set(true);
            }
        }
    }
}

/// What the page's controls held when it was built or last saved: Save sends only what
/// differs, so it neither reverts a value changed elsewhere nor re-clamps one it never showed.
#[derive(PartialEq)]
struct Snapshot {
    settings: BTreeMap<String, Value>,
    /// `sound`, `volume` and `categories.<name>`.
    notifications: BTreeMap<String, Value>,
}

/// One control's current value: `None` when it has nothing to save (a mode with no card
/// chosen) or the control is gone.
type Reader = Box<dyn Fn() -> Result<Option<Value>, String>>;

/// Every control Save writes, registered with where it saves to as the control is built.
#[derive(Default)]
struct Fields {
    /// `settings.set` paths.
    settings: Vec<(String, Reader)>,
    /// `notify.settings.set` keys, as in `Snapshot::notifications`.
    notifications: Vec<(String, Reader)>,
}

impl Fields {
    fn setting(&mut self, path: impl Into<String>, read: Reader) {
        self.settings.push((path.into(), read));
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let read = |fields: &[(String, Reader)]| {
            let mut values = BTreeMap::new();
            for (key, read) in fields {
                if let Some(value) = read()? {
                    values.insert(key.clone(), value);
                }
            }
            Ok::<_, String>(values)
        };
        Ok(Snapshot { settings: read(&self.settings)?, notifications: read(&self.notifications)? })
    }
}

/// Reads `widget` through `get` while it exists. Weak, so the page's fields do not keep a
/// page that was cleared alive.
fn reader<W: IsA<gtk::Widget>>(widget: &W, get: impl Fn(&W) -> Result<Value, String> + 'static) -> Reader {
    let weak = widget.downgrade();
    Box::new(move || weak.upgrade().map(|widget| get(&widget)).transpose())
}

/// The mounted page. It is built once per connection, not per project: nothing on it is per
/// project, and rebuilding it on a project switch threw unsaved edits away.
struct Mounted {
    page: glib::WeakRef<gtk::Box>,
    fields: Rc<RefCell<Fields>>,
    baseline: Rc<RefCell<Option<Snapshot>>>,
    providers: glib::WeakRef<gtk::Box>,
    detected: RefCell<Value>,
}

thread_local! {
    static MOUNTED: RefCell<Option<Rc<Mounted>>> = const { RefCell::new(None) };
}

impl Mounted {
    /// Whether a control or the wallpaper draft differs from what was loaded or saved.
    fn dirty(&self, ui: &Ui) -> bool {
        let wallpaper = ui.wallpaper_draft.borrow().as_ref().is_some_and(|draft| *draft.state.borrow() != *draft.saved.borrow());
        let controls = match (self.page.upgrade(), self.baseline.borrow().as_ref()) {
            (Some(_), Some(baseline)) => self.fields.borrow().snapshot().ok().as_ref() != Some(baseline),
            _ => false,
        };
        wallpaper || controls
    }

    /// Redraws the provider cards when `provider.list` or `provider.refresh` says something new.
    fn show_providers(&self, detected: &Value) {
        let Some(parent) = self.providers.upgrade() else {
            return;
        };
        if *self.detected.borrow() == *detected && parent.first_child().is_some() {
            return;
        }
        *self.detected.borrow_mut() = detected.clone();
        clear(&parent);
        providers(&parent, detected);
    }
}

pub async fn refresh(ui: &Rc<Ui>, project: i64) {
    let generation = ui.generation.get();
    let mounted = MOUNTED.with(|m| m.borrow().clone()).filter(|m| m.page.upgrade().is_some());
    let built = ui.page_projects.borrow().get("settings").copied();
    if let Some(mounted) = mounted.filter(|m| built == Some(generation as i64) || (built.is_some() && m.dirty(ui))) {
        // Keep the form, and any edits in it, across project switches and a reconnect; only
        // re-read the providers, which change behind the page (an install, an update).
        ui.page_projects.borrow_mut().insert("settings".into(), generation as i64);
        if let Some(draft) = ui.wallpaper_draft.borrow().as_ref() {
            if draft.stale.replace(false) {
                if let Some(gallery) = draft.gallery.upgrade() {
                    render_wallpapers(ui, &gallery, &draft.state);
                }
            }
        }
        let detected = ui.call("provider.list", json!({})).await;
        if current(ui, "settings", project, generation) {
            if let Ok(detected) = detected {
                mounted.show_providers(&detected);
            }
        }
        return;
    }
    // Read narrow subtrees: wallpaper libraries can be megabytes and are not needed here.
    let result = async {
        let mut data = json!({});
        for path in [
            "providers",
            "device",
            "keybindings",
            "appearance.mode",
            "appearance.panel_alpha",
            "appearance.wallpaper_dim",
            "appearance.content_contrast",
            "appearance.wallpapers",
            "appearance.wallpaper",
            "appearance.wallpaper_rotation",
            "terminal.font_size",
        ] {
            data[path] = ui.call("settings.get", json!({"path":path})).await?["value"].clone();
        }
        data["notifications"] = ui.call("notify.settings.get", json!({})).await?;
        data["detected"] = ui.call("provider.list", json!({})).await?;
        Ok::<_, crate::client::Error>(data)
    }
    .await;
    if !current(ui, "settings", project, generation) {
        return;
    }
    let data = match result {
        Ok(data) => data,
        Err(e) => {
            ui.show_error(&e.to_string());
            return;
        }
    };
    let wallpapers = Rc::new(RefCell::new(
        json!({"wallpapers":data["appearance.wallpapers"],"wallpaper":data["appearance.wallpaper"]}),
    ));
    let saved_wallpapers = Rc::new(RefCell::new(wallpapers.borrow().clone()));
    // Keep the saved baseline null so Save persists offered presets, but never
    // select a wallpaper or replace an explicitly empty/custom library here.
    wallpapers.borrow_mut()["wallpapers"] =
        crate::wallpaper_rotation::library_or_defaults(&data["appearance.wallpapers"]);
    let page = &ui.pages["settings"];
    clear(page);
    page.add_css_class("settings-page");
    page.set_spacing(0);
    ui.page_projects
        .borrow_mut()
        .insert("settings".into(), generation as i64);
    let baseline = Rc::new(RefCell::new(None::<Snapshot>));
    let fields = Rc::new(RefCell::new(Fields::default()));
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("settings-head");
    let title = gtk::Box::new(gtk::Orientation::Vertical, 2);
    title.set_hexpand(true);
    title.append(&label("Settings", "title"));
    title.append(&label("Relay preferences and local tooling.", "dim"));
    header.append(&title);
    let search = gtk::SearchEntry::new();
    search.set_widget_name("settings-search");
    search.set_placeholder_text(Some("Search settings"));
    search.set_size_request(230, -1);
    search.set_valign(gtk::Align::Center);
    header.append(&search);
    let reload = button("Save changes", "primary");
    reload.add_css_class("settings-small-key");
    let save_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    save_row.append(&crate::icons::image("save", 13));
    let save_caption = label("Save changes", "");
    save_row.append(&save_caption);
    reload.set_child(Some(&save_row));
    reload.set_widget_name("settings-save");
    reload.set_valign(gtk::Align::Center);
    header.append(&reload);
    page.append(&header);
    let target = page.downgrade();
    let weak = Rc::downgrade(ui);
    let wallpaper_state = wallpapers.clone();
    let saved_for_save = saved_wallpapers.clone();
    let loaded = baseline.clone();
    let saved_fields = fields.clone();
    reload.connect_clicked(move |_| {
        let (Some(ui), Some(page)) = (weak.upgrade(), target.upgrade()) else {
            return;
        };
        let now = match saved_fields.borrow().snapshot() {
            Ok(now) => now,
            Err(error) => {
                ui.show_error(&error);
                return;
            }
        };
        let mut settings = Vec::new();
        let before = loaded.borrow();
        for (path, value) in &now.settings {
            if before.as_ref().and_then(|b| b.settings.get(path)) != Some(value) {
                settings.push(("settings.set", json!({"path":path,"value":value})));
            }
        }
        let mut notifications = json!({});
        for (key, value) in &now.notifications {
            if before.as_ref().and_then(|b| b.notifications.get(key)) != Some(value) {
                match key.strip_prefix("categories.") {
                    Some(category) => notifications["categories"][category] = value.clone(),
                    None => notifications[key.as_str()] = value.clone(),
                }
            }
        }
        drop(before);
        let wallpaper_snapshot = wallpaper_state.borrow().clone();
        for field in ["wallpapers", "wallpaper"] {
            if wallpaper_snapshot[field] != saved_for_save.borrow()[field] {
                settings.push((
                    "settings.set",
                    json!({"path":format!("appearance.{field}"),"value":wallpaper_snapshot[field]}),
                ));
            }
        }
        let saved_wallpapers = saved_for_save.clone();
        if notifications.as_object().is_some_and(|patch| !patch.is_empty()) {
            settings.push(("notify.settings.set", json!({"patch":notifications})));
        }
        let loaded = loaded.clone();
        let save_caption = save_caption.clone();
        page.set_sensitive(false);
        save_caption.set_text("Saving…");
        glib::spawn_future_local(async move {
            let mut error = None;
            for (op, payload) in settings {
                if let Err(e) = ui.call(op, payload).await {
                    error = Some(e.to_string());
                    break;
                }
            }
            page.set_sensitive(true);
            save_caption.set_text("Save changes");
            if let Some(error) = error {
                ui.show_error(&error);
            } else {
                *saved_wallpapers.borrow_mut() = wallpaper_snapshot;
                *loaded.borrow_mut() = Some(now);
                ui.refresh();
            }
        });
    });
    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    shell.add_css_class("settings-body");
    shell.set_hexpand(true);
    shell.set_size_request(748, -1);
    cap_width(&shell, page, 1180);
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    let nav = gtk::Box::new(gtk::Orientation::Vertical, 3);
    nav.add_css_class("settings-categories");
    nav.set_size_request(230, -1);
    nav.set_valign(gtk::Align::Start);
    let mut first = None::<gtk::ToggleButton>;
    for (name, title, icon, eyebrow, _, _) in CATEGORIES {
        let key = gtk::ToggleButton::new();
        key.add_css_class("settings-category");
        if let Some(first) = &first {
            key.set_group(Some(first));
        } else {
            first = Some(key.clone());
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.append(&crate::icons::image(icon, 16));
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 1);
        copy.append(&label(title, "body"));
        let hint = match name {
            "appearance" => "Theme, opacity, wallpaper",
            "notifications" => "Sounds and categories",
            "agents" => "Providers and updates",
            "safety" => "Caps and protected paths",
            "android" => "SDK and device tools",
            "keyboard" => "Global shortcuts",
            _ => eyebrow,
        };
        copy.append(&label(hint, "faint"));
        row.append(&copy);
        key.set_child(Some(&row));
        let target = stack.downgrade();
        key.connect_toggled(move |key| {
            if key.is_active() {
                if let Some(stack) = target.upgrade() {
                    if stack.child_by_name(name).is_some() {
                        stack.set_visible_child_name(name);
                    }
                }
            }
        });
        key.set_active(name == "appearance");
        nav.append(&key);
    }
    shell.append(&nav);
    shell.append(&stack);
    page.append(&shell);

    let appearance = category(&stack, "appearance", "Appearance");
    appearance.append(&paragraph(
        "Choose a theme and adjust how much wallpaper shows behind your editor and terminals.",
    ));
    let mode_field = gtk::Box::new(gtk::Orientation::Vertical, 5);
    mode_field.append(&label("MODE", "section-label"));
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    modes.set_homogeneous(true);
    let mut first = None::<gtk::ToggleButton>;
    let mut cards = Vec::new();
    for (id, title, hint, background, plate) in [
        (
            "matte",
            "Matte",
            "Neutral console around black plates.",
            (0.055, 0.055, 0.063),
            (0.039, 0.039, 0.043),
        ),
        (
            "dark",
            "Dark",
            "Cooler, higher contrast.",
            (0.051, 0.059, 0.071),
            (0.027, 0.031, 0.039),
        ),
        (
            "oled",
            "OLED",
            "True black; the wall disappears.",
            (0., 0., 0.),
            (0., 0., 0.),
        ),
    ] {
        let key = gtk::ToggleButton::new();
        key.add_css_class("settings-mode");
        key.set_widget_name(&format!("setting:appearance.mode={id}"));
        if let Some(first) = &first {
            key.set_group(Some(first));
        } else {
            first = Some(key.clone());
        }
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let swatch = gtk::DrawingArea::new();
        swatch.set_content_height(28);
        swatch.set_margin_bottom(4);
        swatch.set_hexpand(true);
        swatch.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgb(background.0, background.1, background.2);
            let _ = cr.paint();
            cr.set_source_rgb(plate.0, plate.1, plate.2);
            cr.rectangle(
                5.,
                5.,
                (w as f64 * 0.7 - 5.).max(0.),
                (h - 10).max(0) as f64,
            );
            let _ = cr.fill();
            cr.set_source_rgb(0.22, 0.22, 0.24);
            cr.set_line_width(1.);
            cr.rectangle(0.5, 0.5, (w - 1) as f64, (h - 1) as f64);
            let _ = cr.stroke();
        });
        card.append(&swatch);
        card.append(&label(title, "settings-mode-title"));
        let copy = label(hint, "faint");
        copy.set_wrap(true);
        card.append(&copy);
        key.set_child(Some(&card));
        key.set_active(data["appearance.mode"].as_str().unwrap_or("matte") == id);
        modes.append(&key);
        cards.push((key.downgrade(), id));
    }
    fields.borrow_mut().setting(
        "appearance.mode",
        Box::new(move || Ok(cards.iter().find(|(key, _)| key.upgrade().is_some_and(|key| key.is_active())).map(|(_, id)| json!(id)))),
    );
    mode_field.append(&modes);
    appearance.append(&mode_field);
    let legibility = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    legibility.set_homogeneous(true);
    legibility.add_css_class("settings-legibility");
    for (path, title, default, min, max) in [
        ("appearance.panel_alpha", "PANEL OPACITY", 1., 0.72, 1.),
        ("appearance.wallpaper_dim", "WALLPAPER DIM", 0.28, 0., 0.75),
        (
            "appearance.content_contrast",
            "CONTENT PROTECTION",
            0.,
            0.,
            1.,
        ),
    ] {
        let field_box = gtk::Box::new(gtk::Orientation::Vertical, 5);
        field_box.set_hexpand(true);
        let input = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 0.01);
        input.set_widget_name(&format!("setting:{path}"));
        input.set_value(data[path].as_f64().unwrap_or(default));
        fields.borrow_mut().setting(path, reader(&input, |input| Ok(json!(input.value()))));
        input.set_hexpand(true);
        input.set_draw_value(false);
        let caption = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        caption.append(&label(title, "section-label"));
        let value = label(
            &format!("{:.0}%", input.value() * 100.),
            "settings-field-value",
        );
        caption.append(&value);
        input.connect_value_changed(move |input| {
            value.set_text(&format!("{:.0}%", input.value() * 100.))
        });
        field_box.append(&caption);
        field_box.append(&input);
        if path == "appearance.panel_alpha" {
            appearance.append(&field_box);
        } else {
            legibility.append(&field_box);
        }
    }
    let legibility_group = gtk::Box::new(gtk::Orientation::Vertical, 7);
    legibility_group.append(&legibility);
    legibility_group.append(&label("Dim controls the image itself. Content protection strengthens panels independently so text stays readable over bright wallpaper areas.","settings-legibility-copy"));
    let gallery = wallpaper_library(ui, &appearance, &wallpapers);
    *ui.wallpaper_draft.borrow_mut() = Some(WallpaperDraft {
        state: wallpapers.clone(),
        saved: saved_wallpapers,
        gallery: gallery.downgrade(),
        stale: std::cell::Cell::new(false),
    });
    let rotation = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    rotation.add_css_class("settings-wallpaper-rotation");
    let enabled = gtk::CheckButton::with_label("Rotate wallpapers randomly");
    enabled.set_widget_name("setting:appearance.wallpaper_rotation.enabled");
    enabled.set_active(data["appearance.wallpaper_rotation"]["enabled"] == true);
    fields.borrow_mut().setting("appearance.wallpaper_rotation.enabled", reader(&enabled, |input| Ok(json!(input.is_active()))));
    enabled.set_hexpand(true);
    rotation.append(&enabled);
    rotation.append(&label("Every", "dim"));
    let minutes = gtk::SpinButton::with_range(1., 1440., 1.);
    minutes.set_widget_name("setting:appearance.wallpaper_rotation.interval_minutes");
    minutes.set_value(
        data["appearance.wallpaper_rotation"]["interval_minutes"]
            .as_f64()
            .unwrap_or(15.)
            .clamp(1., 1440.),
    );
    fields.borrow_mut().setting(
        "appearance.wallpaper_rotation.interval_minutes",
        reader(&minutes, |input| Ok(json!(input.value_as_int()))),
    );
    rotation.append(&minutes);
    rotation.append(&label("minutes", "dim"));
    appearance.append(&rotation);
    appearance.append(&paragraph("Uses your saved library and skips the current image. Add at least two wallpapers to rotate."));
    appearance.append(&legibility_group);

    let agents = category(&stack, "agents", "Agents");
    setting_number(
        &mut fields.borrow_mut(),
        &agents,
        "Terminal font size (points)",
        "terminal.font_size",
        data["terminal.font_size"].as_f64().unwrap_or(9.75),
        8.,
        24.,
    );

    agents.append(&paragraph(
        "Provider discovery, updates and executable overrides.",
    ));
    let detected = gtk::Box::new(gtk::Orientation::Vertical, 13);
    agents.append(&detected);
    providers(&detected, &data["detected"]);
    let mounted = Rc::new(Mounted {
        page: page.downgrade(),
        fields: fields.clone(),
        baseline: baseline.clone(),
        providers: detected.downgrade(),
        detected: RefCell::new(data["detected"].clone()),
    });
    MOUNTED.with(|m| *m.borrow_mut() = Some(mounted.clone()));
    // Called directly, not through Ui::mutate: its answer is the providers to show.
    let rediscover = button("Refresh providers", "quiet");
    agents.append(&rediscover);
    let weak = Rc::downgrade(ui);
    let shown = Rc::downgrade(&mounted);
    rediscover.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        key.set_sensitive(false);
        let key = key.clone();
        let shown = shown.clone();
        glib::spawn_future_local(async move {
            match ui.call("provider.refresh", json!({})).await {
                Ok(detected) => {
                    if let Some(mounted) = shown.upgrade() {
                        mounted.show_providers(&detected);
                    }
                }
                Err(error) => ui.show_error(&error.to_string()),
            }
            key.set_sensitive(true);
        });
    });
    for provider in ["claude", "codex"] {
        let updates = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let automatic = gtk::CheckButton::with_label(&format!("Update {provider} at startup"));
        automatic.set_widget_name(&format!("setting:providers.{provider}.auto_update"));
        automatic.set_active(data["providers"][provider]["auto_update"] == true);
        fields.borrow_mut().setting(format!("providers.{provider}.auto_update"), reader(&automatic, |input| Ok(json!(input.is_active()))));
        automatic.set_hexpand(true);
        updates.append(&automatic);
        let update = button("Update now", "quiet");
        update.set_widget_name(&format!("provider-update-{provider}"));
        let weak = Rc::downgrade(ui);
        update.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                crate::provider_updates::update(&ui, provider);
            }
        });
        updates.append(&update);
        agents.append(&updates);
        setting_entry(
            &mut fields.borrow_mut(),
            &agents,
            &format!("{provider} executable"),
            &format!("providers.{provider}.path"),
            data["providers"][provider]["path"].as_str().unwrap_or(""),
        );
    }

    let android = category(&stack, "android", "Android");
    android.append(&paragraph(
        "Standard SDK paths are discovered automatically. Leave overrides empty to use discovery.",
    ));
    for (key, title) in [
        ("sdk_path", "Android SDK"),
        ("adb_path", "ADB executable"),
        ("emulator_path", "Emulator executable"),
        ("avdmanager_path", "AVD manager executable"),
    ] {
        setting_entry(
            &mut fields.borrow_mut(),
            &android,
            title,
            &format!("device.{key}"),
            text(&data["device"], key),
        );
    }
    let safety = category(&stack, "safety", "Guardrails");
    safety.append(&paragraph("Limits the engine enforces on agents. Set them globally, for a workspace, or for one project; anything a level does not set follows the level above. These save on their own, with the button below."));
    safety.append(&crate::pages::guardrail_settings(ui));
    let notifications = category(&stack, "notifications", "Notifications");
    notifications.append(&paragraph(
        "Choose which events appear in your notification feed.",
    ));
    let sound = gtk::DropDown::from_strings(&["Off", "Chime", "Glass", "Pulse", "Signal"]);
    let sounds = ["off", "chime", "glass", "pulse", "signal"];
    sound.set_selected(
        sounds
            .iter()
            .position(|s| Some(*s) == data["notifications"]["sound"].as_str())
            .unwrap_or(1) as u32,
    );
    sound.set_widget_name("notify:sound");
    field("Sound", &sound, &notifications);
    let picked = reader(&sound, move |input| Ok(json!(sounds.get(input.selected() as usize).copied().unwrap_or("off"))));
    fields.borrow_mut().notifications.push(("sound".into(), picked));
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0., 1., 0.01);
    volume.set_widget_name("notify:volume");
    volume.set_value(data["notifications"]["volume"].as_f64().unwrap_or(0.7));
    fields.borrow_mut().notifications.push(("volume".into(), reader(&volume, |input| Ok(json!(input.value())))));
    field("Volume", &volume, &notifications);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    notifications.append(&row);
    let preview = button("Preview", "quiet");
    row.append(&preview);
    let weak = Rc::downgrade(ui);
    let s = sound.clone();
    let v = volume.clone();
    preview.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            crate::sounds::play(&ui, sounds[s.selected() as usize], v.value());
        }
    });
    for category in [
        "agent_done",
        "agent_blocked",
        "guardrail",
        "integration",
        "provider",
        "disk",
        "system",
    ] {
        let enabled = data["notifications"]["categories"][category]
            .as_bool()
            .unwrap_or(true);
        let check = gtk::CheckButton::with_label(&category.replace('_', " "));
        check.set_active(enabled);
        check.set_widget_name(&format!("notify:category:{category}"));
        notifications.append(&check);
        fields.borrow_mut().notifications.push((format!("categories.{category}"), reader(&check, |input| Ok(json!(input.is_active())))));
    }
    let maintenance = category(&stack, "maintenance", "Storage");
    maintenance.append(&paragraph(
        "Create a database backup before major workflow changes.",
    ));
    let backup = button("Back up now", "primary");
    maintenance.append(&backup);
    let status = paragraph("");
    maintenance.append(&status);
    let weak = Rc::downgrade(ui);
    backup.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        key.set_sensitive(false);
        let key = key.clone();
        let status = status.clone();
        glib::spawn_future_local(async move {
            match ui.call("app.backup.now", json!({})).await {
                Ok(value) => status.set_text(&format!(
                    "Saved {} bytes to {}",
                    value["bytes"],
                    text(&value, "path")
                )),
                Err(error) => status.set_text(&error.to_string()),
            }
            key.set_sensitive(true);
        });
    });
    let keyboard = category(&stack, "keyboard", "Keyboard");
    for (name, title, fallback) in crate::shortcuts::DEFAULTS {
        let input = gtk::Entry::new();
        input.set_text(data["keybindings"][name].as_str().unwrap_or(fallback));
        input.set_widget_name(&format!("setting:keybindings.{name}"));
        // A shortcut is saved as typed, empty included: empty turns it off.
        fields.borrow_mut().setting(
            format!("keybindings.{name}"),
            reader(&input, move |input| {
                let value = input.text();
                let value = value.trim();
                if !value.is_empty() && !crate::shortcuts::valid(value) {
                    return Err(format!("Invalid shortcut for {name}. Use Ctrl+Key notation or leave it empty."));
                }
                Ok(json!(value))
            }),
        );
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&input);
        field(title, &row, &keyboard);
    }
    keyboard.append(&paragraph("Escape closes the session sheet. Ctrl+Shift+C / Ctrl+Shift+V copies and pastes in terminals."));
    let no_results = paragraph("No settings match your search.");
    no_results.set_visible(false);
    no_results.set_hexpand(true);
    no_results.set_valign(gtk::Align::Start);
    shell.append(&no_results);
    let mut keys = Vec::new();
    let mut child = nav.first_child();
    for (name, ..) in CATEGORIES {
        let Some(key) = child.take().and_downcast::<gtk::ToggleButton>() else {
            break;
        };
        child = key.next_sibling();
        let text = stack
            .child_by_name(name)
            .map(|page| settings_search_text(&page))
            .unwrap_or_default();
        keys.push((name, key, text));
    }
    let weak_stack = stack.downgrade();
    search.connect_search_changed(move |search| {
        let Some(stack) = weak_stack.upgrade() else {
            return;
        };
        let query = search.text().to_lowercase();
        let words: Vec<_> = query.split_whitespace().collect();
        let mut first = None;
        let mut current_matches = false;
        for (name, key, text) in &keys {
            let matches = words.iter().all(|word| text.contains(word));
            key.set_visible(matches);
            if matches {
                first.get_or_insert(key);
                current_matches |= stack.visible_child_name().as_deref() == Some(*name);
            }
        }
        stack.set_visible(first.is_some());
        no_results.set_visible(first.is_none());
        if !current_matches {
            if let Some(first) = first {
                first.set_active(true);
            }
        }
    });
    // Taken from the built controls, so a value a range clamped counts as unchanged.
    *baseline.borrow_mut() = fields.borrow().snapshot().ok();
}

/// One card per provider from a `provider.list` / `provider.refresh` answer.
fn providers(parent: &gtk::Box, detected: &Value) {
    for provider in rows(detected, "providers") {
        let info = section(parent, text(&provider, "provider"));
        info.append(&paragraph(&format!(
            "{} · {}\n{}",
            if provider["installed"].as_bool() == Some(true) {
                "Installed"
            } else {
                "Not installed"
            },
            text(&provider, "version"),
            text(&provider, "path")
        )));
        if let Some(account) = provider["signed_in_as"].as_str() {
            info.append(&label(account, "dim"));
        }
        info.append(&label(
            if provider["guarded"].as_bool() == Some(true) {
                "Guardrail adapter available"
            } else {
                "No native guardrail adapter"
            },
            "dim",
        ));
    }
}

fn settings_search_text(widget: &gtk::Widget) -> String {
    let mut result = String::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        result.push_str(&label.text());
    }
    if widget.widget_name().starts_with("setting:") {
        result.push_str(&widget.widget_name());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        result.push(' ');
        result.push_str(&settings_search_text(&widget));
        child = widget.next_sibling();
    }
    result.to_lowercase()
}

const CATEGORIES: [(&str, &str, &str, &str, &str, &str); 7] = [
    (
        "appearance",
        "Appearance",
        "layout",
        "Visual system",
        "Make the workspace yours",
        "Theme, wallpaper, panel density, and safeguards for text over bright images.",
    ),
    (
        "notifications",
        "Notifications",
        "bell",
        "Attention",
        "Choose what can interrupt you",
        "Keep high-signal agent events audible and let routine activity stay quiet.",
    ),
    (
        "agents",
        "Agents",
        "terminal",
        "Runtime",
        "Provider and session behavior",
        "Local CLI discovery, authentication state, updates, and executable overrides.",
    ),
    (
        "safety",
        "Guardrails",
        "sliders",
        "Enforcement",
        "Set hard operating boundaries",
        "These are typed core limits, not prompt suggestions that an agent can ignore.",
    ),
    (
        "android",
        "Android",
        "device",
        "Android",
        "Connect the local toolchain",
        "Relay resolves standard SDK locations first; overrides are for unusual installations.",
    ),
    (
        "keyboard",
        "Keyboard",
        "code",
        "Workflow",
        "Keep navigation under your hands",
        "Shortcuts are global, durable, and use familiar Ctrl+Key notation.",
    ),
    (
        "maintenance",
        "Storage",
        "folder",
        "Backups",
        "Keep a recovery copy",
        "Create a database backup before major workflow changes.",
    ),
];

/// Lets `widget` fill the scrolled `page` up to `max` px. GTK has no max-width, so a right
/// margin takes up whatever the visible width exceeds it by, following every window resize.
fn cap_width(widget: &gtk::Box, page: &gtk::Box, max: i32) {
    let Some(adjustment) = page
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
        .map(|scroll| scroll.hadjustment())
    else {
        return;
    };
    let target = widget.downgrade();
    let fit = move |adjustment: &gtk::Adjustment| {
        if let Some(widget) = target.upgrade() {
            widget.set_margin_end((adjustment.page_size() as i32 - max).max(0));
        }
    };
    fit(&adjustment);
    let handler = std::cell::Cell::new(Some(adjustment.connect_page_size_notify(fit)));
    // The page is rebuilt per project; the adjustment outlives every body it sized.
    widget.connect_destroy(move |_| {
        if let Some(handler) = handler.take() {
            adjustment.disconnect(handler);
        }
    });
}

fn category(stack: &gtk::Stack, name: &str, title: &str) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    if let Some((_, _, icon, eyebrow, heading, hint)) = CATEGORIES.iter().find(|c| c.0 == name) {
        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        hero.add_css_class("settings-hero");
        let mark = crate::icons::image(icon, 22);
        mark.add_css_class("settings-hero-icon");
        mark.set_valign(gtk::Align::Center);
        mark.set_halign(gtk::Align::Center);
        hero.append(&mark);
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
        copy.append(&label(eyebrow, "section-label"));
        copy.append(&label(heading, "settings-hero-title"));
        let hint = paragraph(hint);
        hint.set_max_width_chars(75);
        copy.append(&hint);
        hero.append(&copy);
        outer.append(&hero);
    }
    let page = gtk::Box::new(gtk::Orientation::Vertical, 13);
    page.add_css_class("settings-panel");
    page.append(&label(title, "title"));
    outer.append(&page);
    let scroll = crate::app::scrolled(&outer);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    stack.add_titled(&scroll, Some(name), title);
    page
}

/// An override path: empty saves null, which means "discover it".
fn setting_entry(fields: &mut Fields, parent: &gtk::Box, title: &str, path: &str, value: &str) {
    let input = gtk::Entry::builder()
        .text(value)
        .hexpand(true)
        .placeholder_text("Automatic")
        .build();
    input.set_widget_name(&format!("setting:{path}"));
    fields.setting(path, reader(&input, |input| {
        let value = input.text();
        let value = value.trim();
        Ok(if value.is_empty() { Value::Null } else { json!(value) })
    }));
    field(title, &input, parent);
}

fn setting_number(
    fields: &mut Fields,
    parent: &gtk::Box,
    title: &str,
    path: &str,
    value: f64,
    min: f64,
    max: f64,
) {
    let fractional = path == "terminal.font_size";
    let input = gtk::SpinButton::with_range(min, max, if fractional { 0.25 } else { 1. });
    input.set_digits(if fractional { 2 } else { 0 });
    input.set_value(value);
    input.set_widget_name(&format!("setting:{path}"));
    fields.setting(path, reader(&input, move |input| {
        Ok(if fractional { json!(input.value()) } else { json!(input.value_as_int()) })
    }));
    field(title, &input, parent);
}

fn wallpaper_library(ui: &Rc<Ui>, parent: &gtk::Box, state: &Rc<RefCell<Value>>) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 13);
    parent.append(&block);
    render_wallpapers(ui, &block, state);
    block
}

fn render_wallpapers(ui: &Rc<Ui>, block: &gtk::Box, state: &Rc<RefCell<Value>>) {
    // Native selection follows the canonical image. Legacy wallpaper_id/preview are unused.
    clear(block);
    let library = state.borrow()["wallpapers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let active = state.borrow()["wallpaper"].clone();
    let gallery = gtk::FlowBox::new();
    gallery.set_selection_mode(gtk::SelectionMode::None);
    gallery.set_min_children_per_line(2);
    gallery.set_max_children_per_line(5);
    gallery.set_column_spacing(7);
    gallery.set_row_spacing(7);
    gallery.add_css_class("settings-wallpapers");
    for (index, item) in library.iter().enumerate() {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 5);
        card.add_css_class("settings-wallpaper");
        card.set_size_request(132, -1);
        let pick = gtk::Button::new();
        pick.add_css_class("quiet");
        let words = gtk::Box::new(gtk::Orientation::Vertical, 5);
        if let Some(texture) = item_texture(item) {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_size_request(132, 66);
            picture.set_can_shrink(true);
            words.append(&picture);
        }
        let name = label(text(item, "name"), "body");
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        words.append(&name);
        pick.set_child(Some(&words));
        card.append(&pick);
        pick.set_widget_name(&format!("settings-wallpaper-pick-{index}"));
        if item["image"] == active {
            pick.add_css_class("active");
        }
        let weak = Rc::downgrade(ui);
        let target = block.downgrade();
        let staged = state.clone();
        let item = item.clone();
        pick.connect_clicked(move |_| {
            let (Some(ui), Some(block)) = (weak.upgrade(), target.upgrade()) else {
                return;
            };
            staged.borrow_mut()["wallpaper"] = item["image"].clone();
            render_wallpapers(&ui, &block, &staged);
        });
        let remove = button("Remove", "quiet");
        card.append(&remove);
        let weak = Rc::downgrade(ui);
        let target = block.downgrade();
        let staged = state.clone();
        remove.connect_clicked(move |_| {
            let (Some(ui), Some(block)) = (weak.upgrade(), target.upgrade()) else {
                return;
            };
            let mut value = staged.borrow_mut();
            let removed = value["wallpapers"].as_array_mut().unwrap().remove(index);
            if value["wallpaper"] == removed["image"] {
                value["wallpaper"] = Value::Null;
            }
            drop(value);
            render_wallpapers(&ui, &block, &staged);
        });
        gallery.insert(&card, -1);
    }
    let wall = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    wall.add_css_class("settings-wallpaper-empty");
    let current_image = active.as_str().unwrap_or("");
    let current = library
        .iter()
        .find(|item| item["image"].as_str() == Some(current_image));
    let name = current
        .and_then(|item| item["name"].as_str())
        .unwrap_or("Wallpaper");
    let texture = match current {
        Some(item) => item_texture(item),
        None => wallpaper_texture(current_image),
    };
    // Forget images no longer in the library.
    TEXTURES.with(|t| {
        t.borrow_mut().retain(|id, _| library.iter().any(|item| item["id"].as_str() == Some(id.as_str())))
    });
    let empty = label(
        if texture.is_some() {
            name
        } else {
            "No wallpaper"
        },
        "dim",
    );
    empty.add_css_class("settings-wallpaper-caption");
    empty.set_halign(gtk::Align::Center);
    empty.set_valign(gtk::Align::Center);
    empty.set_hexpand(true);
    wall.append(&empty);
    if let Some(texture) = texture {
        let preview = gtk::Overlay::new();
        preview.set_widget_name("settings-wallpaper-preview");
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        picture.set_size_request(-1, 140);
        preview.set_child(Some(&picture));
        wall.remove_css_class("settings-wallpaper-empty");
        wall.set_valign(gtk::Align::End);
        preview.add_overlay(&wall);
        let open = button("Full preview", "quiet");
        open.set_widget_name("settings-wallpaper-open");
        open.set_halign(gtk::Align::End);
        open.set_valign(gtk::Align::Start);
        open.set_margin_top(8);
        open.set_margin_end(8);
        let weak = Rc::downgrade(ui);
        let name = name.to_string();
        open.connect_clicked(move |_| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let window = gtk::Window::builder()
                .title(&name)
                .transient_for(&ui.window)
                .modal(true)
                .default_width((ui.window.width() - 80).clamp(480, 1200))
                .default_height((ui.window.height() - 80).clamp(320, 800))
                .build();
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_can_shrink(true);
            window.set_child(Some(&picture));
            window.present();
        });
        preview.add_overlay(&open);
        block.prepend(&preview);
    } else {
        block.prepend(&wall);
    }
    let library_head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = label(
        &format!("WALLPAPER LIBRARY   {}", library.len()),
        "section-label",
    );
    title.set_hexpand(true);
    library_head.append(&title);
    block.append(&library_head);
    if !library.is_empty() {
        block.append(&gallery);
    }
    if active.is_string() {
        let clear = button("Clear background", "quiet");
        let weak = Rc::downgrade(ui);
        let target = block.downgrade();
        let staged = state.clone();
        clear.connect_clicked(move |_| {
            let (Some(ui), Some(block)) = (weak.upgrade(), target.upgrade()) else {
                return;
            };
            staged.borrow_mut()["wallpaper"] = Value::Null;
            render_wallpapers(&ui, &block, &staged);
        });
        block.append(&clear);
    }
    let add = button("Add wallpaper", "quiet");
    add.add_css_class("settings-small-key");
    let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    add_row.append(&crate::icons::image("plus", 12));
    add_row.append(&label("Add wallpaper", ""));
    add.set_child(Some(&add_row));
    library_head.append(&add);
    let weak = Rc::downgrade(ui);
    let target = block.downgrade();
    let staged = state.clone();
    add.connect_clicked(move|key|{
        let (Some(ui),Some(block))=(weak.upgrade(),target.upgrade())else{return;}; let key=key.clone(); key.set_sensitive(false);
        let staged=staged.clone();
        glib::spawn_future_local(async move{
            let result=async{
                let dialog=gtk::FileDialog::builder().title("Choose wallpaper").build();
                // Closing the chooser is not an error.
                let file=match dialog.open_future(Some(&ui.window)).await {
                    Ok(file)=>file,
                    Err(e) if e.matches(gtk::DialogError::Dismissed)||e.matches(gtk::DialogError::Cancelled)=>return Ok(false),
                    Err(e)=>return Err(e.to_string()),
                };
                let path=file.path().ok_or("Choose a local image")?;
                let name=path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let bytes=ui.rt.spawn_blocking(move||{
                    let meta=std::fs::metadata(&path).map_err(|e|e.to_string())?;
                    if meta.len()>32*1024*1024{return Err("Choose an image smaller than 32 MB".to_string());}
                    let image=gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path,1280,900,true).map_err(|e|e.to_string())?;
                    image.save_to_bufferv("jpeg",&[("quality","80")]).map_err(|e|e.to_string())
                }).await.map_err(|e|e.to_string())??;
                use base64::Engine;
                let image=format!("data:image/jpeg;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes));
                let mut library=staged.borrow()["wallpapers"].as_array().cloned().unwrap_or_default();
                // A legacy `preview` was a second copy of `image` and counted twice against the cap.
                for item in &mut library { if let Some(item)=item.as_object_mut() { item.remove("preview"); } }
                library.push(json!({"id":uuid::Uuid::new_v4().to_string(),"name":name,"image":image}));
                if serde_json::to_vec(&library).unwrap_or_default().len()>1500000{return Err("Wallpaper library is full. Remove an image before adding another.".into());}
                staged.borrow_mut()["wallpapers"]=json!(library);
                staged.borrow_mut()["wallpaper"]=json!(image);
                Ok::<bool,String>(true)
            }.await;
            match result {Err(e)=>ui.show_error(&e),Ok(true)=>render_wallpapers(&ui,&block,&staged),Ok(false)=>{}}
            key.set_sensitive(true);
        });
    });
}

thread_local! {
    /// Decoded library images by id: a gallery redraw (a pick, a removal, a new saved
    /// wallpaper) otherwise decodes every full-size image again on the UI thread.
    static TEXTURES: RefCell<std::collections::HashMap<String, gtk::gdk::Texture>> = RefCell::default();
}

fn item_texture(item: &Value) -> Option<gtk::gdk::Texture> {
    let image = item["image"].as_str()?;
    let Some(id) = item["id"].as_str() else {
        return wallpaper_texture(image);
    };
    if let Some(texture) = TEXTURES.with(|t| t.borrow().get(id).cloned()) {
        return Some(texture);
    }
    let texture = wallpaper_texture(image)?;
    TEXTURES.with(|t| t.borrow_mut().insert(id.to_string(), texture.clone()));
    Some(texture)
}

fn wallpaper_texture(image: &str) -> Option<gtk::gdk::Texture> {
    use base64::Engine;
    let (_, encoded) = image.split_once(',')?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
}
