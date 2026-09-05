use super::{action, current, paragraph, section};
use crate::app::{button, clear, field, label, rows, text, Ui};
use crate::client::{Client, Notice};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::rc::Rc;

pub async fn refresh(ui: &Rc<Ui>, project: i64) {
    let generation = ui.generation.get();
    let result = ui.call("device.list", json!({})).await;
    if !current(ui, "devices", project, generation) {
        return;
    }
    let devices = match result {
        Ok(v) => rows(&v, "devices"),
        Err(e) => {
            ui.show_error(&e.to_string());
            return;
        }
    };
    let page = &ui.pages["devices"];
    if ui.page_projects.borrow().get("devices") != Some(&project) {
        ui.page_projects.borrow_mut().remove("devices");
        clear(page);
        page.append(&label("Device control", "title"));
        page.append(&paragraph("Run Android builds from the selected checkout. A release build needs no connected device."));
        let reload = button("Refresh devices and worktrees", "quiet");
        page.append(&reload);
        let weak = Rc::downgrade(ui);
        reload.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.page_projects.borrow_mut().remove("devices");
                ui.refresh_page();
            }
        });
        let worktrees = match ui
            .call("worktree.list", json!({"project_id":project}))
            .await
        {
            Ok(v) => rows(&v, "worktrees"),
            Err(_) => Vec::new(),
        };
        if !current(ui, "devices", project, generation) {
            return;
        }
        build_form(ui, page, project, &devices, &worktrees);
        avd_form(ui, page);
        page.append(&gtk::Box::new(gtk::Orientation::Vertical, 8));
        // Navigation can cancel the await above. Mark only a fully built form reusable.
        ui.page_projects
            .borrow_mut()
            .insert("devices".into(), project);
    }
    let list = page.last_child().unwrap().downcast::<gtk::Box>().unwrap();
    clear(&list);
    let connected = section(&list, "Connected devices");
    if devices.is_empty() {
        connected.append(&paragraph(
            "No Android device. Connect a phone with USB debugging enabled or boot an AVD below.",
        ));
    }
    for device in devices {
        connected.append(&label(
            &format!("{} · {}", text(&device, "model"), text(&device, "state")),
            "title",
        ));
        connected.append(&paragraph(&format!(
            "{} · {}",
            text(&device, "serial"),
            text(&device, "kind")
        )));
        let mirror = button("Open mirror", "primary");
        connected.append(&mirror);
        let weak = Rc::downgrade(ui);
        let target = text(&device, "serial").to_string();
        mirror.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                crate::mirror::open(&ui, target.clone());
            }
        });
        let copy = button("Copy device ID", "quiet");
        connected.append(&copy);
        let serial = text(&device, "serial").to_string();
        copy.connect_clicked(move |b| {
            b.clipboard().set_text(&serial);
        });
    }
    match ui.call("avd.list", json!({})).await {
        Ok(value) => {
            if !current(ui, "devices", project, generation) {
                return;
            }
            let avds = section(&list, "Virtual devices");
            for avd in rows(&value, "avds") {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let name = label(text(&avd, "name"), "title");
                name.set_hexpand(true);
                row.append(&name);
                if let Some(serial) = avd["running_serial"].as_str() {
                    row.append(&label(serial, "dim"));
                } else {
                    action(
                        ui,
                        &row,
                        "Boot",
                        "avd.boot",
                        json!({"name":avd["name"],"cold":false}),
                    );
                    action(
                        ui,
                        &row,
                        "Cold boot",
                        "avd.boot",
                        json!({"name":avd["name"],"cold":true}),
                    );
                }
                avds.append(&row);
            }
        }
        Err(error) => {
            list.append(&paragraph(&format!("Virtual devices unavailable: {error}")));
        }
    }
    if project <= 0 {
        return;
    }
    if let Ok(signing) = ui
        .call("device.signing.get", json!({"project_id":project}))
        .await
    {
        if !current(ui, "devices", project, generation) {
            return;
        }
        let row = section(&list, "Release signing");
        if signing["configured"].as_bool() == Some(true) {
            let enabled = signing["enabled"].as_bool() == Some(true);
            row.append(&paragraph(&format!(
                "Alias: {}\nKeystore: {}",
                text(&signing, "key_alias"),
                text(&signing, "keystore")
            )));
            action(
                ui,
                &row,
                if enabled {
                    "Use project signing"
                } else {
                    "Use Relay signing"
                },
                "device.signing.set_enabled",
                json!({"project_id":project,"enabled":!enabled}),
            );
        } else {
            row.append(&paragraph(
                "Using the project's Gradle signing configuration.",
            ));
            signing_form(ui, &row, project);
        }
    }
    match ui
        .call("device.run.list", json!({"project_id":project}))
        .await
    {
        Ok(value) => {
            if !current(ui, "devices", project, generation) {
                return;
            }
            let history = section(&list, "Build and run history");
            let runs = rows(&value, "runs");
            if runs.is_empty() {
                history.append(&paragraph("No builds or runs yet."));
            }
            for run in runs {
                let row = section(
                    &history,
                    &format!(
                        "#{} · {} · {}",
                        run["id"],
                        text(&run, "kind"),
                        text(&run, "state")
                    ),
                );
                row.append(&paragraph(&format!(
                    "{}\n{} · {}",
                    text(&run, "worktree"),
                    text(&run, "variant"),
                    text(&run, "started_at")
                )));
                if let Some(artifact) = run["artifact"].as_str() {
                    row.append(&paragraph(&format!(
                        "{}\nSigning: {}",
                        artifact,
                        text(&run, "signing")
                    )));
                    let copy = button("Copy artifact path", "quiet");
                    row.append(&copy);
                    let path = artifact.to_string();
                    copy.connect_clicked(move |b| b.clipboard().set_text(&path));
                }
                if matches!(
                    text(&run, "state"),
                    "building" | "running" | "installing" | "launching"
                ) {
                    action(
                        ui,
                        &row,
                        "Stop",
                        "device.run.stop",
                        json!({"run_id":run["id"]}),
                    );
                }
            }
        }
        Err(error) => {
            list.append(&paragraph(&error.to_string()));
        }
    }
}

#[derive(Clone)]
struct WorktreePicker {
    widget: gtk::DropDown,
    paths: Rc<Vec<String>>,
}

impl WorktreePicker {
    fn new(worktrees: &[Value]) -> Self {
        let mut names = vec!["Primary · project branch".to_string()];
        let mut paths = vec![String::new()];
        for tree in worktrees {
            names.push(format!("{} · {}", text(tree, "branch"), text(tree, "path")));
            paths.push(text(tree, "path").to_string());
        }
        let widget =
            gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
        widget.set_widget_name("device-worktree");
        widget.set_enable_search(true);
        widget.set_hexpand(true);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let name = label("", "");
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_max_width_chars(38);
            item.set_child(Some(&name));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(value) = item.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            let name = item.child().and_downcast::<gtk::Label>().unwrap();
            name.set_text(&value.string());
            name.set_tooltip_text(Some(&value.string()));
        });
        widget.set_factory(Some(&factory));
        widget.set_list_factory(Some(&factory));
        Self {
            widget,
            paths: Rc::new(paths),
        }
    }

    fn active_id(&self) -> Option<String> {
        self.paths.get(self.widget.selected() as usize).cloned()
    }
}

pub(crate) async fn verify_worktree_picker(ui: &Rc<Ui>) {
    assert_eq!(std::env::var("RELAY_INSTANCE").as_deref(), Ok("test"));
    let path = format!("/tmp/{}", "long-checkout-name/".repeat(30));
    let picker =
        WorktreePicker::new(&[json!({"branch":"very-long-branch-name/".repeat(30),"path":path})]);
    let window = gtk::Window::builder()
        .transient_for(&ui.window)
        .default_width(400)
        .default_height(80)
        .build();
    window.set_child(Some(&picker.widget));
    window.present();
    picker.widget.set_selected(1);
    glib::timeout_future(std::time::Duration::from_millis(100)).await;
    assert_eq!(picker.active_id().as_deref(), Some(path.as_str()));
    assert!(picker.widget.enables_search());
    assert!(
        window.width() <= 430,
        "Long branch and path must not expand the device surface: {}px",
        window.width()
    );
    window.close();
}

fn build_form(ui: &Rc<Ui>, page: &gtk::Box, project: i64, devices: &[Value], worktrees: &[Value]) {
    let form = section(page, "Build & run");
    let tree = WorktreePicker::new(worktrees);
    field("Build from", &tree.widget, &form);
    let target = gtk::ComboBoxText::new();
    for device in devices {
        if text(device, "state") == "device" {
            target.append(
                Some(text(device, "serial")),
                &format!("{} · {}", text(device, "model"), text(device, "serial")),
            );
        }
    }
    target.set_active(Some(0));
    field("Target device", &target, &form);
    let variant = gtk::Entry::builder().text("debug").build();
    field("Gradle variant", &variant, &form);
    let format = gtk::ComboBoxText::new();
    format.append(Some("apk"), "APK");
    format.append(Some("aab"), "Android App Bundle");
    format.set_active(Some(0));
    field("Artifact format", &format, &form);
    let publish =
        gtk::CheckButton::with_label("Publish artifact to the configured GitHub repository");
    form.append(&publish);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    form.append(&controls);
    for (title, op) in [
        ("Run on device", "device.run"),
        ("Build artifact", "device.build"),
    ] {
        let key = button(title, "primary");
        key.set_sensitive(project > 0 && (op != "device.run" || target.active_id().is_some()));
        controls.append(&key);
        let weak = Rc::downgrade(ui);
        let tree = tree.clone();
        let target = target.clone();
        let variant = variant.clone();
        let format = format.clone();
        let publish = publish.clone();
        key.connect_clicked(move |_|{
            let Some(ui)=weak.upgrade()else{return;};let selected=tree.active_id().map(|v|v.to_string()).unwrap_or_default();let variant=variant.text().trim().to_string();
            if variant.is_empty(){ui.show_error("Enter the Gradle variant to build.");return;}
            let mut payload=json!({"project_id":project,"worktree":if selected.is_empty(){None}else{Some(selected)},"variant":variant});
            if op=="device.run"{let Some(device)=target.active_id()else{return;};payload["device"]=json!(device.as_str());}
            else{payload["format"]=json!(format.active_id().map(|v|v.to_string()).unwrap_or_else(||"apk".into()));payload["publish"]=json!(publish.is_active());}
            run_window(&ui,op,payload,title);
        });
    }
}

fn avd_form(ui: &Rc<Ui>, page: &gtk::Box) {
    let expander = gtk::Expander::new(Some("Create virtual device"));
    page.append(&expander);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    expander.set_child(Some(&form));
    let name = gtk::Entry::builder()
        .placeholder_text("Relay_API_35")
        .build();
    field("AVD name", &name, &form);
    let image = gtk::ComboBoxText::new();
    field("Installed system image", &image, &form);
    let profile = gtk::ComboBoxText::new();
    profile.append(Some(""), "SDK default");
    profile.set_active(Some(0));
    field("Device profile", &profile, &form);
    let load = button("Load installed images", "quiet");
    form.append(&load);
    let weak = Rc::downgrade(ui);
    let images = image.clone();
    let profiles = profile.clone();
    load.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        key.set_sensitive(false);
        let key = key.clone();
        let images = images.clone();
        let profiles = profiles.clone();
        glib::spawn_future_local(async move {
            match ui.call("avd.catalog", json!({})).await {
                Ok(value) => {
                    images.remove_all();
                    profiles.remove_all();
                    profiles.append(Some(""), "SDK default");
                    for item in rows(&value, "system_images") {
                        if let Some(id) = item.as_str() {
                            images.append(Some(id), id);
                        }
                    }
                    for item in rows(&value, "devices") {
                        if let Some(id) = item.as_str() {
                            profiles.append(Some(id), id);
                        }
                    }
                    images.set_active(Some(0));
                    profiles.set_active(Some(0));
                }
                Err(error) => ui.show_error(&error.to_string()),
            }
            key.set_sensitive(true);
        });
    });
    let create = button("Create AVD", "primary");
    form.append(&create);
    let weak = Rc::downgrade(ui);
    create.connect_clicked(move |key| {
        if let Some(ui) = weak.upgrade() {
            let title = name.text().trim().to_string();
            let Some(package) = image.active_id() else {
                ui.show_error("Load and select an installed system image first.");
                return;
            };
            if title.is_empty() {
                ui.show_error("Enter an AVD name.");
                return;
            }
            let device = profile
                .active_id()
                .map(|v| v.to_string())
                .filter(|v| !v.is_empty());
            ui.mutate(
                "avd.create",
                json!({"name":title,"package":package.as_str(),"device":device}),
                key,
            );
        }
    });
}

// A separate socket keeps noisy Gradle/logcat output away from the UI control plane.
fn run_window(ui: &Rc<Ui>, op: &'static str, payload: Value, title: &str) {
    let window = crate::panel::Panel::new(ui, title, 780);
    let page = window.body.clone();
    page.append(&paragraph("Closing this log panel leaves the build or run active. Use Stop in device history to stop it."));
    let output = gtk::TextView::new();
    output.set_editable(false);
    output.set_monospace(true);
    output.set_wrap_mode(gtk::WrapMode::WordChar);
    page.append(&crate::app::scrolled(&output));
    let status = label("Connecting…", "dim");
    page.append(&status);
    let ui = ui.clone();
    let task = glib::spawn_future_local(async move {
        match Client::connect(&ui.rt, ui.path.clone()).await {
            Ok((client, notices)) => {
                match client.request(&ui.rt, op, payload).await {
                    Ok(run) => {
                        status.set_text(&format!("Run #{} · {}", run["id"], text(&run, "state")))
                    }
                    Err(error) => {
                        status.set_text(&error.to_string());
                        return;
                    }
                }
                ui.refresh_page();
                while let Ok(notice) = notices.recv().await {
                    match notice {
                        Notice::Frame(frame) if frame.stream == "logcat" => {
                            let buffer = output.buffer();
                            let mut end = buffer.end_iter();
                            buffer.insert(
                                &mut end,
                                &format!("{}\n", frame.data.as_str().unwrap_or("")),
                            );
                            if buffer.line_count() > 2000 {
                                if let Some(mut trim) =
                                    buffer.iter_at_line(buffer.line_count() - 2000)
                                {
                                    buffer.delete(&mut buffer.start_iter(), &mut trim);
                                }
                            }
                        }
                        Notice::Disconnected(error) => {
                            status.set_text(&error.to_string());
                            break;
                        }
                        _ => {}
                    }
                }
                drop(client);
            }
            Err(error) => status.set_text(&error.to_string()),
        }
    });
    window.on_closed(move || task.abort());
    window.present();
}

fn signing_form(ui: &Rc<Ui>, row: &gtk::Box, project: i64) {
    let expander = gtk::Expander::new(Some("Create Relay signing key"));
    row.append(&expander);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    expander.set_child(Some(&form));
    let alias = gtk::Entry::builder().text("upload").build();
    field("Key alias", &alias, &form);
    let password = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let passwords = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    passwords.set_homogeneous(true);
    let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let right = gtk::Box::new(gtk::Orientation::Vertical, 6);
    field("Password", &password, &left);
    let confirmation = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    field("Confirm", &confirmation, &right);
    passwords.append(&left);
    passwords.append(&right);
    form.append(&passwords);
    form.append(&paragraph("The key is stored privately and its password goes to Linux Secret Service. Export a backup before publishing releases with this key."));
    let create = button("Create signing key", "primary");
    form.append(&create);
    let weak = Rc::downgrade(ui);
    create.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let alias = alias.text().trim().to_string();
        let secret = password.text().to_string();
        if secret != confirmation.text() {
            ui.show_error("Passwords do not match.");
            return;
        }
        if alias.is_empty() || secret.len() < 6 {
            ui.show_error("Enter a key alias and a password of at least six characters.");
            return;
        }
        key.set_sensitive(false);
        let key = key.clone();
        let password = password.clone();
        let confirmation = confirmation.clone();
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "device.signing.create",
                    json!({"project_id":project,"key_alias":alias,"password":secret}),
                )
                .await
            {
                Ok(_) => {
                    password.set_text("");
                    confirmation.set_text("");
                    ui.refresh_page();
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            key.set_sensitive(true);
        });
    });
}

/// Footer utility. The full device page retains build history and advanced controls.
pub fn open(ui: &Rc<Ui>) {
    let Some(panel) = crate::panel::Panel::toggle(ui, "Device control", 430) else {
        return;
    };
    panel.bottom(340);
    panel.add_css_class("device-panel");
    let refresh = crate::app::icon_button("refresh", "Refresh devices");
    refresh.set_widget_name("device-refresh");
    let weak = Rc::downgrade(ui);
    refresh.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            if ui.dismiss_panels() {
                open(&ui);
            }
        }
    });
    panel.header_action(&refresh);
    let tabs = gtk::Stack::new();
    tabs.set_widget_name("device-tabs");
    let switch = gtk::StackSwitcher::new();
    switch.set_stack(Some(&tabs));
    switch.set_halign(gtk::Align::Fill);
    panel.body.append(&switch);
    let run = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let release = gtk::Box::new(gtk::Orientation::Vertical, 12);
    tabs.add_titled(&run, Some("run"), "RUN");
    tabs.add_titled(&release, Some("release"), "RELEASE");
    panel.body.append(&tabs);
    run.append(&paragraph("Loading devices…"));
    release.append(&paragraph("Loading worktrees…"));
    let ui = ui.clone();
    let task = glib::spawn_future_local(async move {
        let project = ui.project.get();
        let (devices, trees) = tokio::join!(
            ui.call("device.list", json!({})),
            ui.call("worktree.list", json!({"project_id":project}))
        );
        clear(&run);
        clear(&release);
        let devices = match devices {
            Ok(v) => rows(&v, "devices"),
            Err(e) => {
                run.append(&paragraph(&e.to_string()));
                Vec::new()
            }
        };
        let trees = trees.map(|v| rows(&v, "worktrees")).unwrap_or_default();
        avd_form(&ui, &run);
        if let Ok(v) = ui.call("avd.list", json!({})).await {
            for avd in rows(&v, "avds") {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let name = label(text(&avd, "name"), "body");
                name.set_hexpand(true);
                row.append(&name);
                if avd["running_serial"].is_string() {
                    row.append(&label("Running", "dim"));
                } else {
                    action(
                        &ui,
                        &row,
                        "Boot",
                        "avd.boot",
                        json!({"name":avd["name"],"cold":false}),
                    );
                    action(
                        &ui,
                        &row,
                        "Cold",
                        "avd.boot",
                        json!({"name":avd["name"],"cold":true}),
                    );
                }
                run.append(&row);
            }
        }
        if devices.is_empty() {
            let empty = gtk::Box::new(gtk::Orientation::Vertical, 8);
            empty.set_margin_top(20);
            empty.set_margin_bottom(20);
            empty.append(&crate::icons::image("device", 24));
            let title = label("No Android device", "title");
            title.set_xalign(0.5);
            empty.append(&title);
            let copy = paragraph("Connect a phone with USB debugging enabled, or boot an AVD above. A release build needs no device.");
            copy.set_justify(gtk::Justification::Center);
            empty.append(&copy);
            run.append(&empty);
        }
        release.append(&label("Release artifact", "title"));
        release.append(&paragraph(
            "Build the selected worktree with its Gradle release configuration.",
        ));
        for (form, is_release) in [(&run, false), (&release, true)] {
            if project <= 0 {
                form.append(&paragraph("Select a project to build."));
                continue;
            }
            let tree = WorktreePicker::new(&trees);
            field("Build from", &tree.widget, form);
            let target = gtk::ComboBoxText::new();
            for device in &devices {
                if text(device, "state") == "device" {
                    target.append(
                        Some(text(device, "serial")),
                        &format!("{} · {}", text(device, "model"), text(device, "serial")),
                    );
                }
            }
            target.set_active(Some(0));
            if !is_release && !devices.is_empty() {
                field("Device", &target, form);
                let facts = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                facts.add_css_class("device-facts");
                let serial = label(
                    target.active_id().as_deref().unwrap_or("unavailable"),
                    "mono",
                );
                serial.set_hexpand(true);
                serial.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                facts.append(&serial);
                let copy = button("Copy ID", "quiet");
                facts.append(&copy);
                let selected = target.downgrade();
                copy.connect_clicked(move |key| {
                    if let Some(target) = selected.upgrade() {
                        if let Some(id) = target.active_id() {
                            key.clipboard().set_text(&id);
                            key.set_label("Copied");
                        }
                    }
                });
                let selected_copy = copy.downgrade();
                target.connect_changed(move |target| {
                    serial.set_text(target.active_id().as_deref().unwrap_or("unavailable"));
                    if let Some(copy) = selected_copy.upgrade() {
                        copy.set_label("Copy ID");
                    }
                });
                form.append(&facts);
                let mirror = button("Open mirror", "quiet");
                form.append(&mirror);
                let weak = Rc::downgrade(&ui);
                let target = target.clone();
                mirror.connect_clicked(move |_| {
                    if let (Some(ui), Some(serial)) = (weak.upgrade(), target.active_id()) {
                        crate::mirror::open(&ui, serial.to_string());
                    }
                });
            }
            let format = gtk::ComboBoxText::new();
            format.append(Some("aab"), "Android App Bundle (.aab) · Google Play");
            format.append(Some("apk"), "Android package (.apk)");
            format.set_active(Some(0));
            if is_release {
                field("Artifact", &format, form);
                if let Ok(signing) = ui
                    .call("device.signing.get", json!({"project_id":project}))
                    .await
                {
                    if signing["configured"] == true {
                        form.append(&label(
                            if signing["enabled"] == true {
                                "Relay signing configured"
                            } else {
                                "Gradle signing"
                            },
                            "body",
                        ));
                        action(
                            &ui,
                            form,
                            if signing["enabled"] == true {
                                "Use Gradle signing"
                            } else {
                                "Use Relay signing"
                            },
                            "device.signing.set_enabled",
                            json!({"project_id":project,"enabled":signing["enabled"] != true}),
                        );
                    } else {
                        form.append(&label("Gradle signing", "body"));
                        signing_form(&ui, form, project);
                    }
                }
            }
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            form.append(&actions);
            for publish in [false, true] {
                if publish && !is_release {
                    continue;
                }
                let key = button(
                    if publish {
                        "Build and publish"
                    } else if is_release {
                        "Build release"
                    } else {
                        "Run debug"
                    },
                    if publish { "quiet" } else { "primary" },
                );
                key.set_sensitive(is_release || target.active_id().is_some());
                actions.append(&key);
                let weak = Rc::downgrade(&ui);
                let tree = tree.clone();
                let target = target.clone();
                let format = format.clone();
                key.connect_clicked(move |_| {
                    let Some(ui) = weak.upgrade() else { return; };
                    let path=tree.active_id().map(|s|s.to_string()).filter(|s|!s.is_empty());
                    let mut payload=json!({"project_id":project,"worktree":path,"variant":if is_release {"release"} else {"debug"}});
                    if is_release { payload["format"]=json!(format.active_id().map(|s|s.to_string())); payload["publish"]=json!(publish); }
                    else { let Some(serial)=target.active_id() else {return;}; payload["device"]=json!(serial.as_str()); }
                    run_window(&ui, if is_release {"device.build"} else {"device.run"}, payload, if is_release {"Release build"} else {"Run debug"});
                });
            }
        }
        if project > 0 {
            if let Ok(value) = ui
                .call("device.run.list", json!({"project_id":project}))
                .await
            {
                let runs = rows(&value, "runs");
                for current in runs.iter().filter(|r| {
                    matches!(
                        text(r, "state"),
                        "running" | "building" | "installing" | "launching"
                    )
                }) {
                    let state = gtk::Box::new(gtk::Orientation::Horizontal, 9);
                    state.add_css_class("device-release-state");
                    let copy = label(
                        &format!("{} · {}", text(current, "kind"), text(current, "state")),
                        "body",
                    );
                    copy.set_hexpand(true);
                    state.append(&copy);
                    action(
                        &ui,
                        &state,
                        "Stop",
                        "device.run.stop",
                        json!({"run_id":current["id"]}),
                    );
                    if text(current, "kind") == "build" {
                        release.append(&state);
                    } else {
                        run.append(&state);
                    }
                }
                if let Some(last) = runs.iter().find(|r| r["artifact"].is_string()) {
                    let output = gtk::Box::new(gtk::Orientation::Horizontal, 9);
                    output.add_css_class("device-release-state");
                    let copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
                    copy.set_hexpand(true);
                    copy.append(&label(
                        &format!("{} artifact", text(last, "signing")),
                        "body",
                    ));
                    let artifact = text(last, "artifact").to_owned();
                    let path = label(artifact.rsplit('/').next().unwrap_or(&artifact), "mono");
                    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                    path.set_tooltip_text(Some(&artifact));
                    copy.append(&path);
                    output.append(&copy);
                    let key = button("Copy path", "quiet");
                    key.connect_clicked(move |key| {
                        key.clipboard().set_text(&artifact);
                        key.set_label("Copied");
                    });
                    output.append(&key);
                    release.append(&output);
                }
            }
        }
        let history = button("Build history and advanced controls", "quiet");
        release.append(&history);
        let weak = Rc::downgrade(&ui);
        history.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.dismiss_panels();
                ui.navigate("devices");
            }
        });
    });
    panel.on_closed(move || task.abort());
    panel.present();
}
