use super::{action, current, paragraph, section};
use crate::app::{button, clear, field, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::rc::Rc;

pub async fn refresh(ui: &Rc<Ui>, project: i64) {
    if ui.page_projects.borrow().get("settings") == Some(&project) {
        return;
    }
    let generation = ui.generation.get();
    // Read narrow subtrees: wallpaper libraries can be megabytes and are not needed here.
    let result = async {
        let mut data = json!({});
        for path in [
            "providers",
            "device",
            "parking",
            "keybindings",
            "appearance.mode",
            "appearance.panel_alpha",
            "appearance.wallpaper_dim",
            "appearance.content_contrast",
            "appearance.wallpapers",
            "terminal.font_size",
        ] {
            data[path] = ui.call("settings.get", json!({"path":path})).await?["value"].clone();
        }
        data["notifications"] = ui.call("notify.settings.get", json!({})).await?;
        data["guardrails"] = ui.call("guardrail.config.get", json!({})).await?;
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
    let page = &ui.pages["settings"];
    clear(page);
    ui.page_projects
        .borrow_mut()
        .insert("settings".into(), project);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = label("Settings", "title");
    title.set_hexpand(true);
    header.append(&title);
    let reload = button("Reload saved values", "quiet");
    header.append(&reload);
    page.append(&header);
    let weak = Rc::downgrade(ui);
    reload.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.page_projects.borrow_mut().remove("settings");
            ui.refresh_page();
        }
    });
    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    let nav = gtk::StackSidebar::new();
    nav.set_stack(&stack);
    nav.set_size_request(160, -1);
    shell.append(&nav);
    shell.append(&stack);
    page.append(&shell);

    let appearance = category(&stack, "appearance", "Appearance");
    appearance.append(&paragraph(
        "Relay-2 console palettes and native terminal typography.",
    ));
    let modes = gtk::ComboBoxText::new();
    for (id, title) in [("matte", "Matte"), ("dark", "Dark"), ("oled", "OLED")] {
        modes.append(Some(id), title);
    }
    modes.set_active_id(Some(data["appearance.mode"].as_str().unwrap_or("matte")));
    field("Console palette", &modes, &appearance);
    let save = button("Apply palette", "primary");
    appearance.append(&save);
    let weak = Rc::downgrade(ui);
    save.connect_clicked(move |key| {if let Some(ui)=weak.upgrade(){ui.mutate("settings.set",json!({"path":"appearance.mode","value":modes.active_id().map(|v|v.to_string()).unwrap_or_else(||"matte".into())}),key);}});
    setting_number(
        ui,
        &appearance,
        "Terminal font size (points)",
        "terminal.font_size",
        data["terminal.font_size"].as_f64().unwrap_or(10.),
        8.,
        24.,
    );

    for (path, title, default, min, max) in [
        ("appearance.panel_alpha", "Panel opacity", 1., 0.5, 1.),
        (
            "appearance.wallpaper_dim",
            "Wallpaper dimming",
            0.28,
            0.,
            0.85,
        ),
        (
            "appearance.content_contrast",
            "Content contrast",
            0.,
            0.,
            1.,
        ),
    ] {
        let input = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 0.01);
        input.set_value(data[path].as_f64().unwrap_or(default));
        input.set_hexpand(true);
        input.set_draw_value(true);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&input);
        let save = button("Apply", "quiet");
        row.append(&save);
        field(title, &row, &appearance);
        let weak = Rc::downgrade(ui);
        save.connect_clicked(move |key| {
            if let Some(ui) = weak.upgrade() {
                ui.mutate(
                    "settings.set",
                    json!({"path":path,"value":input.value()}),
                    key,
                );
            }
        });
    }
    wallpaper_library(ui, &appearance, &data["appearance.wallpapers"]);

    let agents = category(&stack, "agents", "Agents");
    agents.append(&paragraph(
        "Provider discovery, executable overrides and process parking.",
    ));
    for provider in rows(&data["detected"], "providers") {
        let info = section(&agents, text(&provider, "provider"));
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
    action(
        ui,
        &agents,
        "Refresh providers",
        "provider.refresh",
        json!({}),
    );
    for provider in ["claude", "codex"] {
        setting_entry(
            ui,
            &agents,
            &format!("{provider} executable"),
            &format!("providers.{provider}.path"),
            data["providers"][provider]["path"].as_str().unwrap_or(""),
        );
    }
    setting_number(
        ui,
        &agents,
        "Park idle sessions after (minutes)",
        "parking.idle_minutes",
        data["parking"]["idle_minutes"].as_f64().unwrap_or(30.),
        0.,
        1440.,
    );

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
            ui,
            &android,
            title,
            &format!("device.{key}"),
            text(&data["device"], key),
        );
    }
    let safety = category(&stack, "safety", "Guardrails");
    safety.append(&paragraph("Hard operating limits enforced by the engine. These values apply globally; project overrides are preserved."));
    guardrails(ui, &safety, &data["guardrails"]);
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
    field("Sound", &sound, &notifications);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0., 1., 0.01);
    volume.set_value(data["notifications"]["volume"].as_f64().unwrap_or(0.7));
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
    let save = button("Save audio", "quiet");
    row.append(&save);
    let weak = Rc::downgrade(ui);
    save.connect_clicked(move|key|{if let Some(ui)=weak.upgrade(){ui.mutate("notify.settings.set",json!({"patch":{"sound":sounds[sound.selected() as usize],"volume":volume.value()}}),key);}});
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
        notifications.append(&check);
        let weak = Rc::downgrade(ui);
        check.connect_toggled(move |check| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let enabled = check.is_active();
            let check = check.clone();
            check.set_sensitive(false);
            glib::spawn_future_local(async move {
                if let Err(error) = ui
                    .call(
                        "notify.settings.set",
                        json!({"patch":{"categories":{category:enabled}}}),
                    )
                    .await
                {
                    ui.show_error(&error.to_string());
                }
                check.set_sensitive(true);
            });
        });
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
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&input);
        let save = button("Save", "quiet");
        row.append(&save);
        field(title, &row, &keyboard);
        let weak = Rc::downgrade(ui);
        save.connect_clicked(move |key| {
            if let Some(ui) = weak.upgrade() {
                let chord = input.text();
                if !chord.is_empty() && !crate::shortcuts::valid(&chord) {
                    ui.show_error("Use a modified key such as Ctrl+K, or leave empty to disable.");
                    return;
                }
                ui.mutate(
                    "settings.set",
                    json!({"path":format!("keybindings.{name}"),"value":chord.as_str()}),
                    key,
                );
            }
        });
    }
    keyboard.append(&paragraph("Escape closes the session sheet. Ctrl+Shift+C / Ctrl+Shift+V copies and pastes in terminals."));
}

fn category(stack: &gtk::Stack, name: &str, title: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.append(&label(title, "title"));
    let scroll = crate::app::scrolled(&page);
    stack.add_titled(&scroll, Some(name), title);
    page
}

fn setting_entry(ui: &Rc<Ui>, parent: &gtk::Box, title: &str, path: &str, value: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let input = gtk::Entry::builder()
        .text(value)
        .hexpand(true)
        .placeholder_text("Automatic")
        .build();
    row.append(&input);
    let save = button("Save", "quiet");
    row.append(&save);
    field(title, &row, parent);
    let weak = Rc::downgrade(ui);
    let path = path.to_string();
    save.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let value = input.text().trim().to_string();
        let value = if value.is_empty() {
            Value::Null
        } else {
            json!(value)
        };
        ui.mutate("settings.set", json!({"path":path,"value":value}), key);
    });
}

fn setting_number(
    ui: &Rc<Ui>,
    parent: &gtk::Box,
    title: &str,
    path: &str,
    value: f64,
    min: f64,
    max: f64,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let input = gtk::SpinButton::with_range(min, max, 1.);
    input.set_value(value);
    row.append(&input);
    let save = button("Save", "quiet");
    row.append(&save);
    field(title, &row, parent);
    let weak = Rc::downgrade(ui);
    let path = path.to_string();
    save.connect_clicked(move |key| {
        if let Some(ui) = weak.upgrade() {
            ui.mutate(
                "settings.set",
                json!({"path":path,"value":input.value_as_int()}),
                key,
            );
        }
    });
}

fn guardrails(ui: &Rc<Ui>, page: &gtk::Box, data: &Value) {
    let mut numbers = Vec::new();
    for (group, key, title, max) in [
        ("caps", "files", "Maximum changed files", 1_000_000.),
        ("caps", "lines", "Maximum changed lines", 100_000_000.),
        (
            "destructive_write",
            "min_removed_lines",
            "Destructive change: minimum removed lines",
            1_000_000.,
        ),
        (
            "destructive_write",
            "min_removed_pct",
            "Destructive change: minimum removed percent",
            100.,
        ),
    ] {
        let input = gtk::SpinButton::with_range(0., max, 1.);
        input.set_value(data[group][key].as_f64().unwrap_or(0.));
        field(title, &input, page);
        numbers.push((group, key, input));
    }
    let mut lists = Vec::new();
    for (key, title) in [
        ("protected_paths", "Protected paths, one per line"),
        ("denied_commands", "Denied commands, one per line"),
    ] {
        let view = gtk::TextView::new();
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_top_margin(8);
        view.set_bottom_margin(8);
        view.buffer().set_text(
            &data[key]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
        );
        let scroll = crate::app::scrolled(&view);
        scroll.set_min_content_height(110);
        scroll.set_vexpand(false);
        field(title, &scroll, page);
        lists.push((key, view));
    }
    let save = button("Save guardrail limits", "primary");
    page.append(&save);
    let weak = Rc::downgrade(ui);
    save.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let mut patch = json!({"caps":{},"destructive_write":{}});
        for (group, name, input) in &numbers {
            patch[*group][*name] = json!(input.value_as_int());
        }
        for (name, input) in &lists {
            let buffer = input.buffer();
            let content = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            patch[*name] = json!(content
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>());
        }
        ui.mutate("guardrail.config.set", json!({"patch":patch}), key);
    });
}

fn wallpaper_library(ui: &Rc<Ui>, parent: &gtk::Box, value: &Value) {
    let block = section(parent, "Wallpapers");
    let library = value.as_array().cloned().unwrap_or_default();
    for (index, item) in library.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&label(text(item, "name"), "dim"));
        action(
            ui,
            &row,
            "Use",
            "settings.set",
            json!({"path":"appearance.wallpaper","value":item["image"]}),
        );
        let remove = button("Remove", "quiet");
        row.append(&remove);
        let mut remaining = library.clone();
        remaining.remove(index);
        let weak = Rc::downgrade(ui);
        remove.connect_clicked(move |key| {
            if let Some(ui) = weak.upgrade() {
                ui.page_projects.borrow_mut().remove("settings");
                ui.mutate(
                    "settings.set",
                    json!({"path":"appearance.wallpapers","value":remaining}),
                    key,
                );
            }
        });
        block.append(&row);
    }
    action(
        ui,
        &block,
        "Clear background",
        "settings.set",
        json!({"path":"appearance.wallpaper","value":null}),
    );
    let add = button("Add wallpaper", "quiet");
    block.append(&add);
    let weak = Rc::downgrade(ui);
    add.connect_clicked(move|key|{
        let Some(ui)=weak.upgrade()else{return;}; let key=key.clone(); key.set_sensitive(false);
        glib::spawn_future_local(async move{
            let result=async{
                let dialog=gtk::FileDialog::builder().title("Choose wallpaper").build();
                let file=dialog.open_future(Some(&ui.window)).await.map_err(|e|e.to_string())?;
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
                let value=ui.call("settings.get",json!({"path":"appearance.wallpapers"})).await.map_err(|e|e.to_string())?;
                let mut library=value["value"].as_array().cloned().unwrap_or_default();
                library.push(json!({"id":uuid::Uuid::new_v4().to_string(),"name":name,"image":image,"preview":image}));
                if serde_json::to_vec(&library).unwrap_or_default().len()>1500000{return Err("Wallpaper library is full. Remove an image before adding another.".into());}
                ui.call("settings.set",json!({"path":"appearance.wallpapers","value":library})).await.map_err(|e|e.to_string())?;
                ui.call("settings.set",json!({"path":"appearance.wallpaper","value":image})).await.map_err(|e|e.to_string())?;
                Ok::<(),String>(())
            }.await;
            if let Err(e)=result {ui.show_error(&e);} else {ui.page_projects.borrow_mut().remove("settings");ui.refresh_page();}
            key.set_sensitive(true);
        });
    });
}
