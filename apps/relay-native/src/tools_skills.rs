use super::{action, current, paragraph, section};
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
        ui.page_projects
            .borrow_mut()
            .insert("skills".into(), project);
        page.append(&label("Skills", "title"));
        page.append(&paragraph("Install reusable instructions, inspect their source, and choose which projects use them."));
        install_form(ui, page);
        let create = button("Write a local skill", "quiet");
        page.append(&create);
        let weak = Rc::downgrade(ui);
        create.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                edit(&ui, None);
            }
        });
        page.append(&gtk::Box::new(gtk::Orientation::Vertical, 8));
    }
    let list = page.last_child().unwrap().downcast::<gtk::Box>().unwrap();
    clear(&list);
    if data.is_empty() {
        list.append(&paragraph(
            "No installed skills. Install from a GitHub repository or write a local skill.",
        ));
    }
    for skill in data {
        let row = section(&list, text(&skill, "name"));
        let source = skill["source_url"].as_str().unwrap_or("Local skill");
        row.append(&label(
            &format!("{} · {}", source, text(&skill, "source_path")),
            "dim",
        ));
        let preview = gtk::Expander::new(Some("Read instructions"));
        preview.set_child(Some(&paragraph(text(&skill, "body"))));
        row.append(&preview);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&controls);
        let enabled = skill["enabled_in"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id.as_i64() == Some(project)));
        if project > 0 {
            action(
                ui,
                &controls,
                if enabled {
                    "Disable for project"
                } else {
                    "Enable for project"
                },
                "skill.enable",
                json!({"skill_id":skill["id"],"project_id":project,"enabled":!enabled}),
            );
        }
        let edit_key = button("Edit", "quiet");
        controls.append(&edit_key);
        let weak = Rc::downgrade(ui);
        let selected = skill.clone();
        edit_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                edit(&ui, Some(selected.clone()));
            }
        });
        if skill["source_url"].is_string() {
            action(
                ui,
                &controls,
                "Refresh from source",
                "skill.install",
                json!({"url":skill["source_url"],"subdir":skill["source_path"]}),
            );
        }
        let remove = button("Delete", "quiet");
        controls.append(&remove);
        let armed = Cell::new(false);
        let weak = Rc::downgrade(ui);
        remove.connect_clicked(move |key| {
            if !armed.replace(true) {
                key.set_label("Confirm deletion");
                return;
            }
            if let Some(ui) = weak.upgrade() {
                ui.mutate("skill.delete", json!({"skill_id":skill["id"]}), key);
            }
        });
    }
}

fn install_form(ui: &Rc<Ui>, page: &gtk::Box) {
    let source = gtk::Entry::builder()
        .placeholder_text("https://github.com/owner/repository")
        .hexpand(true)
        .build();
    field("GitHub repository", &source, page);
    let subdir = gtk::Entry::builder()
        .placeholder_text("Optional skill directory")
        .hexpand(true)
        .build();
    field("Subdirectory", &subdir, page);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let install = button("Install skills", "primary");
    let replace = button("Replace conflicting skill", "quiet");
    replace.set_visible(false);
    controls.append(&install);
    controls.append(&replace);
    page.append(&controls);
    let feedback = paragraph("");
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
    let window = gtk::Window::builder()
        .transient_for(&ui.window)
        .modal(true)
        .title(if skill.is_some() {
            "Edit skill"
        } else {
            "New skill"
        })
        .default_width(700)
        .default_height(560)
        .build();
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(16);
    page.set_margin_bottom(16);
    page.set_margin_start(16);
    page.set_margin_end(16);
    window.set_child(Some(&page));
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
    window.connect_close_request(move |_| {
        if changed.get() || running.get() {
            message.set_text("Save your changes or choose Discard and close.");
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    let weak_window = window.downgrade();
    let changed = dirty.clone();
    discard.connect_clicked(move |_| {
        changed.set(false);
        if let Some(window) = weak_window.upgrade() {
            window.close();
        }
    });
    let weak = Rc::downgrade(ui);
    let weak_window = window.downgrade();
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
