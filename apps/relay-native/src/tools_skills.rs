use super::{action, current, paragraph};
use crate::app::{button, clear, field, label, rows, text, Ui};
use crate::client::Error;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub async fn refresh(ui: &Rc<Ui>, project: i64) {
    let generation = ui.generation.get();
    let result = ui.call("skill.list", json!({})).await;
    if !current(ui, "skills", project, generation) {
        return;
    }
    let data = match result {
        Ok(v) => rows(&v, "skills"),
        Err(e) => {
            ui.show_error(&e.to_string());
            return;
        }
    };
    let page = &ui.pages["skills"];
    if ui.page_projects.borrow().get("skills") != Some(&project) {
        clear(page);
        page.add_css_class("skills-page");
        page.set_spacing(0);
        ui.page_projects
            .borrow_mut()
            .insert("skills".into(), project);
        let head = gtk::Box::new(gtk::Orientation::Vertical, 3);
        head.add_css_class("skills-head");
        head.append(&label("Skills", "title"));
        let copy = paragraph("Install reusable agent instructions from GitHub. Every skill is enabled everywhere and written into every checkout and provider home as a real skill folder; switch one off below for a project that should not see it.");
        copy.add_css_class("skills-description");
        head.append(&copy);
        page.append(&head);
        install_form(ui, page);
        let workspace = gtk::Paned::new(gtk::Orientation::Horizontal);
        workspace.add_css_class("skills-workspace");
        workspace.set_widget_name("skills-split");
        workspace.set_vexpand(true);
        workspace.set_resize_start_child(true);
        workspace.set_resize_end_child(true);
        workspace.set_shrink_start_child(false);
        workspace.set_shrink_end_child(false);
        workspace.set_position(((ui.window.width() - 240) / 3).clamp(250, 420));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("skills-list");
        list.set_size_request(220, -1);
        let scroll = crate::app::scrolled(&list);
        scroll.set_hexpand(true);
        scroll.set_min_content_width(220);
        scroll.set_min_content_height(240);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        workspace.set_start_child(Some(&scroll));
        let preview = gtk::Box::new(gtk::Orientation::Vertical, 10);
        preview.add_css_class("skills-preview");
        preview.set_hexpand(true);
        preview.set_vexpand(true);
        preview.set_size_request(300, -1);
        preview.set_widget_name("skill-0");
        workspace.set_end_child(Some(&preview));
        page.append(&workspace);
    }
    let workspace = page.last_child().unwrap().downcast::<gtk::Paned>().unwrap();
    let list = workspace
        .start_child()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::Viewport>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::Box>()
        .unwrap();
    let preview = workspace
        .end_child()
        .unwrap()
        .downcast::<gtk::Box>()
        .unwrap();
    let scope = list
        .widget_name()
        .strip_prefix("skills-scope-")
        .and_then(|id| id.parse().ok())
        .unwrap_or(project);
    render_library(ui, &list, &preview, &data, scope);
}

fn render_library(ui: &Rc<Ui>, list: &gtk::Box, preview: &gtk::Box, data: &[Value], project: i64) {
    list.set_widget_name(&format!("skills-scope-{project}"));
    let selected = preview
        .widget_name()
        .strip_prefix("skill-")
        .and_then(|id| id.parse::<i64>().ok());
    let selected = data
        .iter()
        .find(|skill| skill["id"].as_i64() == selected)
        .or(data.first())
        .cloned();
    clear(list);
    let scope = gtk::Box::new(gtk::Orientation::Vertical, 3);
    scope.add_css_class("skills-scope");
    scope.append(&label("PROJECT ENABLEMENT", "section-label"));
    let name = ui
        .projects
        .borrow()
        .iter()
        .find(|p| p["id"].as_i64() == Some(project))
        .map(|p| text(p, "name").to_owned())
        .unwrap_or_else(|| "No active project".into());
    let projects = ui.projects.borrow().clone();
    let picker =
        gtk::DropDown::from_strings(&projects.iter().map(|p| text(p, "name")).collect::<Vec<_>>());
    picker.set_widget_name("skills-project");
    picker.set_enable_search(true);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let name = label("", "");
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_width_chars(1);
        name.set_max_width_chars(28);
        item.downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&name));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let value = item.item().and_downcast::<gtk::StringObject>().unwrap();
        let name = item.child().and_downcast::<gtk::Label>().unwrap();
        name.set_text(&value.string());
        name.set_tooltip_text(Some(&value.string()));
    });
    picker.set_factory(Some(&factory));
    picker.set_list_factory(Some(&factory));
    picker.set_selected(
        projects
            .iter()
            .position(|p| p["id"].as_i64() == Some(project))
            .map(|i| i as u32)
            .unwrap_or(gtk::INVALID_LIST_POSITION),
    );
    picker.set_sensitive(!projects.is_empty());
    let weak = Rc::downgrade(ui);
    let target_list = list.downgrade();
    let target_preview = preview.downgrade();
    picker.connect_selected_notify(move |picker| {
        let (Some(ui), Some(list), Some(preview)) = (
            weak.upgrade(),
            target_list.upgrade(),
            target_preview.upgrade(),
        ) else {
            return;
        };
        if let Some(project) = projects
            .get(picker.selected() as usize)
            .and_then(|p| p["id"].as_i64())
        {
            // Enablement changes for other projects are not in this page's snapshot: read the
            // library again. The scope is recorded first so a page refresh keeps it too.
            let scope = format!("skills-scope-{project}");
            list.set_widget_name(&scope);
            glib::spawn_future_local(async move {
                let generation = ui.generation.get();
                let result = ui.call("skill.list", json!({})).await;
                if generation != ui.generation.get() || list.widget_name() != scope {
                    return;
                }
                match result {
                    Ok(v) => render_library(&ui, &list, &preview, &rows(&v, "skills"), project),
                    Err(e) => ui.show_error(&e.to_string()),
                }
            });
        }
    });
    scope.append(&picker);
    list.append(&scope);
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for skill in data {
        let key = skill["source_url"]
            .as_str()
            .map(|s| s.trim_end_matches(".git").to_lowercase())
            .unwrap_or_else(|| format!("local:{}", skill["id"]));
        if let Some((_, rows)) = groups.iter_mut().find(|(name, _)| name == &key) {
            rows.push(skill.clone());
        } else {
            groups.push((key, vec![skill.clone()]));
        }
    }
    let keys = Rc::new(RefCell::new(Vec::<gtk::ToggleButton>::new()));
    for (source, mut skills) in groups {
        skills.sort_by_key(|s| text(s, "source_path").to_string());
        let children = gtk::Box::new(gtk::Orientation::Vertical, 0);
        if skills.len() > 1 {
            let group = gtk::Expander::new(Some(&format!(
                "{}   {} skills · {} on",
                source.rsplit('/').next().unwrap_or(&source),
                skills.len(),
                skills.iter().filter(|s| enabled(s, project)).count()
            )));
            group.add_css_class("skills-group");
            group.set_expanded(false);
            group.set_child(Some(&children));
            list.append(&group);
        } else {
            list.append(&children);
        }
        let mut directories = std::collections::BTreeMap::new();
        for skill in skills {
            let mut parent = children.clone();
            let source_path = text(&skill, "source_path");
            let mut prefix = String::new();
            let parts: Vec<_> = source_path
                .trim_end_matches("/SKILL.md")
                .split('/')
                .filter(|s| !s.is_empty() && *s != ".")
                .collect();
            for component in parts.iter().take(parts.len().saturating_sub(1)) {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(component);
                parent = directories
                    .entry(prefix.clone())
                    .or_insert_with(|| {
                        let group = gtk::Expander::new(Some(component));
                        group.add_css_class("skills-directory");
                        group.set_expanded(true);
                        let nested = gtk::Box::new(gtk::Orientation::Vertical, 0);
                        nested.set_margin_start(12);
                        group.set_child(Some(&nested));
                        parent.append(&group);
                        nested
                    })
                    .clone();
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.add_css_class("skills-row");
            let key = gtk::ToggleButton::new();
            key.add_css_class("skill-pick");
            key.set_hexpand(true);
            if let Some(first) = keys.borrow().first() {
                key.set_group(Some(first));
            }
            let words = gtk::Box::new(gtk::Orientation::Vertical, 3);
            let title = label(text(&skill, "name"), "body");
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            words.append(&title);
            let path = label(
                skill["source_path"].as_str().unwrap_or("local instruction"),
                "mono",
            );
            path.set_ellipsize(gtk::pango::EllipsizeMode::End);
            words.append(&path);
            key.set_child(Some(&words));
            key.set_active(selected.as_ref().is_some_and(|v| v["id"] == skill["id"]));
            row.append(&key);
            keys.borrow_mut().push(key.clone());
            let switch = gtk::Switch::new();
            switch.set_widget_name(&format!("skill-enabled-{}", skill["id"]));
            switch.set_valign(gtk::Align::Center);
            switch.set_active(enabled(&skill, project));
            switch.set_sensitive(project > 0);
            switch.set_tooltip_text(Some(&format!("Enable {} for {name}", text(&skill, "name"))));
            switch.add_css_class("skill-switch");
            row.append(&switch);
            let weak = Rc::downgrade(ui);
            let id = skill["id"].clone();
            switch.connect_state_set(move |key, on| {
                // A rejected write restores the switch while it is disabled.
                if !key.is_sensitive() {
                    return glib::Propagation::Proceed;
                }
                if let Some(ui) = weak.upgrade() {
                    let key = key.clone();
                    let id = id.clone();
                    key.set_sensitive(false);
                    glib::spawn_future_local(async move {
                        if let Err(e) = ui
                            .call(
                                "skill.enable",
                                json!({"skill_id":id,"project_id":project,"enabled":on}),
                            )
                            .await
                        {
                            ui.show_error(&e.to_string());
                            key.set_active(!on);
                        }
                        key.set_sensitive(true);
                    });
                }
                glib::Propagation::Proceed
            });
            let weak = Rc::downgrade(ui);
            let preview = preview.clone();
            key.connect_toggled(move |key| {
                if key.is_active() {
                    if let Some(ui) = weak.upgrade() {
                        skill_preview(&ui, &preview, Some(&skill));
                    }
                }
            });
            parent.append(&row);
        }
    }
    if data.is_empty() {
        list.append(&paragraph("No skills installed. Paste a GitHub skill repository above. Relay finds its SKILL.md files."));
    }
    let create = button("Write a local skill", "quiet");
    let weak = Rc::downgrade(ui);
    create.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, None);
        }
    });
    if data.is_empty() {
        list.append(&create);
    }
    skill_preview(ui, preview, selected.as_ref());
}

fn enabled(skill: &Value, project: i64) -> bool {
    skill["enabled_in"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|id| id.as_i64() == Some(project)))
}

fn skill_preview(ui: &Rc<Ui>, preview: &gtk::Box, skill: Option<&Value>) {
    clear(preview);
    let Some(skill) = skill else {
        preview.append(&label("Select an installed skill", "title"));
        preview.append(&paragraph("Its source and instructions will appear here."));
        return;
    };
    preview.set_widget_name(&format!("skill-{}", skill["id"]));
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Box::new(gtk::Orientation::Vertical, 2);
    title.set_hexpand(true);
    title.append(&label("INSTALLED SKILL", "section-label"));
    title.append(&label(text(skill, "name"), "title"));
    head.append(&title);
    if let Some(url) = skill["source_url"].as_str() {
        action(ui, &head, "Open source", "os.open_url", json!({"url":url}));
        action(
            ui,
            &head,
            "Refresh",
            "skill.install",
            json!({"url":url,"subdir":skill["source_path"]}),
        );
    }
    let edit_key = button("Edit", "quiet");
    let weak = Rc::downgrade(ui);
    let editing = skill.clone();
    edit_key.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, Some(editing.clone()));
        }
    });
    if !skill["source_url"].is_string() {
        head.append(&edit_key);
    }
    preview.append(&head);
    let source = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let url = label(
        skill["source_url"].as_str().unwrap_or("Local skill"),
        "mono",
    );
    url.set_ellipsize(gtk::pango::EllipsizeMode::End);
    url.set_hexpand(true);
    source.append(&url);
    source.append(&label(
        &text(skill, "revision").chars().take(8).collect::<String>(),
        "mono",
    ));
    preview.append(&source);
    let body = gtk::TextView::new();
    body.set_editable(false);
    body.set_cursor_visible(false);
    body.set_monospace(true);
    body.set_wrap_mode(gtk::WrapMode::WordChar);
    body.add_css_class("skill-source");
    body.buffer().set_text(text(skill, "body"));
    let scroll = crate::app::scrolled(&body);
    scroll.set_min_content_height(200);
    scroll.set_vexpand(true);
    preview.append(&scroll);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let remove = button("Remove", "quiet");
    remove.add_css_class("danger");
    let weak = Rc::downgrade(ui);
    let id = skill["id"].clone();
    let armed = Cell::new(false);
    remove.connect_clicked(move |key| {
        if !armed.replace(true) {
            key.set_label("Remove now");
            return;
        }
        if let Some(ui) = weak.upgrade() {
            ui.mutate("skill.delete", json!({"skill_id":id}), key);
        }
    });
    footer.append(&remove);
    let hint = label("Refresh pulls the current GitHub version.", "faint");
    hint.set_hexpand(true);
    hint.set_xalign(1.);
    footer.append(&hint);
    preview.append(&footer);
}

fn install_form(ui: &Rc<Ui>, page: &gtk::Box) {
    let source = gtk::Entry::builder()
        .placeholder_text("https://github.com/owner/repository")
        .hexpand(true)
        .build();
    let installer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    installer.add_css_class("skills-installer");
    installer.set_valign(gtk::Align::End);
    let repository = gtk::Box::new(gtk::Orientation::Vertical, 6);
    repository.set_hexpand(true);
    field("GitHub repository or SKILL.md URL", &source, &repository);
    installer.append(&repository);
    let subdir = gtk::Entry::builder()
        .placeholder_text("skills/review")
        .hexpand(false)
        .build();
    let folder = gtk::Box::new(gtk::Orientation::Vertical, 6);
    folder.set_size_request(240, -1);
    folder.set_hexpand(false);
    field("Folder · optional", &subdir, &folder);
    installer.append(&folder);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let install = button("Install", "primary");
    let replace = button("Replace conflicting skill", "quiet");
    replace.set_visible(false);
    controls.append(&install);
    controls.append(&replace);
    controls.set_valign(gtk::Align::End);
    installer.append(&controls);
    page.append(&installer);
    let feedback = paragraph("");
    feedback.set_visible(false);
    page.append(&feedback);
    let conflict = Rc::new(RefCell::new(None::<(i64, String, String)>));
    for (key, replacing) in [(install.clone(), false), (replace.clone(), true)] {
        let weak = Rc::downgrade(ui);
        let source = source.clone();
        let subdir = subdir.clone();
        let install = install.clone();
        let replace = replace.clone();
        let feedback = feedback.clone();
        let conflict = conflict.clone();
        key.connect_clicked(move |_|{
            let Some(ui)=weak.upgrade()else{return;};
            feedback.set_visible(true);
            let url=source.text().trim().to_string();let dir=subdir.text().trim().to_string();
            if url.is_empty(){feedback.set_text("Enter a GitHub repository URL.");return;}
            let mut payload=json!({"url":url,"subdir":if dir.is_empty(){None}else{Some(&dir)}});
            if replacing {
                let saved=conflict.borrow();
                let Some((id,old_url,old_dir))=saved.as_ref()else{return;};
                if old_url!=&url || old_dir!=&dir{feedback.set_text("Source changed. Install again to check the current conflict.");replace.set_visible(false);return;}
                payload["replace_skill_id"]=json!(id);
            }
            install.set_sensitive(false);replace.set_sensitive(false);
            let (source,subdir,install,replace,feedback,conflict)=(source.clone(),subdir.clone(),install.clone(),replace.clone(),feedback.clone(),conflict.clone());
            glib::spawn_future_local(async move{
                match ui.call("skill.install",payload).await {
                    Ok(result)=>{
                        feedback.set_text(&format!("Installed {} skills.",rows(&result,"skills").len()));replace.set_visible(false);conflict.borrow_mut().take();
                        if source.text().trim()==url && subdir.text().trim()==dir{source.set_text("");subdir.set_text("");}
                        ui.refresh_page();
                    }
                    Err(error)=>{
                        feedback.set_text(&error.to_string());replace.set_visible(false);conflict.borrow_mut().take();
                        if let Error::Bus(error)=error {
                            if error.code=="skill.name_exists" {
                                if let Some(details)=error.details {
                                    if details["can_replace"].as_bool()==Some(true) {
                                        if let Some(id)=details["installed_skill"]["id"].as_i64(){
                                            feedback.set_text(&format!("A skill named '{}' already exists. Replace it with the downloaded instructions?",text(&details["installed_skill"],"name")));
                                            *conflict.borrow_mut()=Some((id,url,dir));replace.set_visible(true);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                install.set_sensitive(true);replace.set_sensitive(true);
            });
        });
    }
}

fn edit(ui: &Rc<Ui>, skill: Option<Value>) {
    let window = crate::panel::Panel::new(
        ui,
        if skill.is_some() {
            "Edit skill"
        } else {
            "New skill"
        },
        700,
    );
    let page = window.body.clone();
    let name = gtk::Entry::builder()
        .text(skill.as_ref().map(|v| text(v, "name")).unwrap_or(""))
        .build();
    field("Name", &name, &page);
    let body = gtk::TextView::new();
    body.set_monospace(true);
    body.set_wrap_mode(gtk::WrapMode::WordChar);
    body.buffer()
        .set_text(skill.as_ref().map(|v| text(v, "body")).unwrap_or(""));
    let scroll = crate::app::scrolled(&body);
    field("Instructions", &scroll, &page);
    let status = paragraph("");
    page.append(&status);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let save = button("Save skill", "primary");
    let discard = button("Discard and close", "quiet");
    controls.append(&save);
    controls.append(&discard);
    page.append(&controls);
    let dirty = Rc::new(Cell::new(false));
    let busy = Rc::new(Cell::new(false));
    let changed = dirty.clone();
    name.connect_changed(move |_| changed.set(true));
    let changed = dirty.clone();
    body.buffer().connect_changed(move |_| changed.set(true));
    let changed = dirty.clone();
    let running = busy.clone();
    let message = status.clone();
    window.set_guard(move || {
        if changed.get() || running.get() {
            message.set_text("Save your changes or choose Discard and close.");
            false
        } else {
            true
        }
    });
    let weak_window = Rc::downgrade(&window);
    let changed = dirty.clone();
    discard.connect_clicked(move |_| {
        changed.set(false);
        if let Some(window) = weak_window.upgrade() {
            window.close();
        }
    });
    let weak = Rc::downgrade(ui);
    let weak_window = Rc::downgrade(&window);
    save.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let title = name.text().trim().to_string();
        let buffer = body.buffer();
        let content = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        if title.is_empty() || content.trim().is_empty() {
            status.set_text("A name and instructions are required.");
            return;
        }
        let (op, payload) = match &skill {
            Some(skill) => (
                "skill.update",
                json!({"skill_id":skill["id"],"name":title,"body":content}),
            ),
            None => ("skill.create", json!({"name":title,"body":content})),
        };
        busy.set(true);
        key.set_sensitive(false);
        discard.set_sensitive(false);
        name.set_sensitive(false);
        body.set_editable(false);
        let (key, discard, name, body, status, dirty, busy, weak_window) = (
            key.clone(),
            discard.clone(),
            name.clone(),
            body.clone(),
            status.clone(),
            dirty.clone(),
            busy.clone(),
            weak_window.clone(),
        );
        glib::spawn_future_local(async move {
            match ui.call(op, payload).await {
                Ok(_) => {
                    dirty.set(false);
                    busy.set(false);
                    if let Some(window) = weak_window.upgrade() {
                        window.close();
                    }
                    ui.refresh_page();
                }
                Err(error) => status.set_text(&error.to_string()),
            }
            busy.set(false);
            key.set_sensitive(true);
            discard.set_sensitive(true);
            name.set_sensitive(true);
            body.set_editable(true);
        });
    });
    window.present();
}
