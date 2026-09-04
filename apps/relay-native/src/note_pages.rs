use super::task_pages::{buffer_text, choose, chosen, multiline, Draft};
use super::*;

fn editor(ui: &Rc<Ui>, note: Value) -> Rc<Draft> {
    let id = note["id"].as_i64().unwrap_or(0);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Entry::builder().text(text(&note, "title")).build();
    title.set_widget_name("note-title");
    field("Title", &title, &form);
    let is_plan = text(&note, "title") == "Plan";
    let pin = gtk::CheckButton::with_label("Pinned · included in agent standing notes");
    pin.set_active(is_plan || note["pinned"].as_bool().unwrap_or(false));
    pin.set_sensitive(!is_plan);
    title.set_editable(!is_plan);
    form.append(&pin);
    let body = multiline(text(&note, "body"), 420);
    body.set_monospace(true);
    body.set_widget_name("note-body");
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    for (caption, before, after) in [
        ("Bold", "**", "**"),
        ("Italic", "_", "_"),
        ("Heading", "\n## ", ""),
        ("List", "\n- ", ""),
        ("Checklist", "\n- [ ] ", ""),
        ("Code", "\n```\n", "\n```\n"),
    ] {
        let key = button(caption, "quiet");
        toolbar.append(&key);
        let buffer = body.buffer();
        key.connect_clicked(move |_| {
            let (mut start, mut end) = buffer.selection_bounds().unwrap_or_else(|| {
                let cursor = buffer.iter_at_offset(buffer.cursor_position());
                (cursor, cursor)
            });
            let selected = buffer.text(&start, &end, false);
            let replacement = format!("{before}{selected}{after}");
            buffer.begin_user_action();
            buffer.delete(&mut start, &mut end);
            buffer.insert(&mut start, &replacement);
            buffer.end_user_action();
        });
    }
    form.append(&toolbar);
    let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let query = gtk::Entry::builder()
        .placeholder_text("Find")
        .hexpand(true)
        .build();
    let replacement = gtk::Entry::builder()
        .placeholder_text("Replace with")
        .hexpand(true)
        .build();
    let find = button("Find next", "quiet");
    let replace = button("Replace all", "quiet");
    find_row.append(&query);
    find_row.append(&replacement);
    find_row.append(&find);
    find_row.append(&replace);
    form.append(&find_row);
    let buffer = body.buffer();
    let q = query.clone();
    find.connect_clicked(move |_| {
        let needle = q.text();
        if needle.is_empty() {
            return;
        }
        let cursor = buffer.iter_at_offset(buffer.cursor_position());
        if let Some((start, end)) = cursor
            .forward_search(&needle, gtk::TextSearchFlags::CASE_INSENSITIVE, None)
            .or_else(|| {
                buffer.start_iter().forward_search(
                    &needle,
                    gtk::TextSearchFlags::CASE_INSENSITIVE,
                    None,
                )
            })
        {
            buffer.select_range(&end, &start);
        }
    });
    let buffer = body.buffer();
    replace.connect_clicked(move |_| {
        let needle = query.text();
        if !needle.is_empty() {
            let next = buffer_text(&buffer).replace(needle.as_str(), replacement.text().as_str());
            buffer.begin_user_action();
            buffer.set_text(&next);
            buffer.end_user_action();
        }
    });
    let wrap = gtk::CheckButton::with_label("Wrap lines");
    wrap.set_active(true);
    let view = body.clone();
    wrap.connect_toggled(move |key| {
        view.set_wrap_mode(if key.is_active() {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        })
    });
    form.append(&wrap);
    form.append(&crate::app::scrolled(&body));
    let count = label("", "dim");
    let buffer = body.buffer();
    form.append(&count);
    let update = move |b: &gtk::TextBuffer| {
        let value = buffer_text(b);
        count.set_text(&format!(
            "{} lines · {} words · {} characters",
            value.lines().count().max(1),
            value.split_whitespace().count(),
            value.chars().count()
        ));
    };
    let status = label("Ctrl+S to save", "dim");
    form.append(&status);
    buffer.connect_changed(update);
    let snapshot: Rc<dyn Fn() -> Value> = Rc::new(
        move || json!({"title":if title.text().trim().is_empty(){Value::Null}else{json!(title.text().trim())},"body":buffer_text(&body.buffer()),"pinned":pin.is_active()}),
    );
    let draft = Draft::new_note(
        ui,
        if text(&note, "title") == "Plan" {
            "Plan"
        } else {
            "Note"
        },
        note,
        snapshot,
        form,
    );
    draft.controls(ui, "notes.get", "notes.update", "note_id", id);
    super::task_pages::action(
        ui,
        &draft,
        &draft.footer,
        "Delete note",
        "notes.delete",
        move || json!({"note_id":id}),
        None,
    );
    draft
}
pub fn edit(ui: &Rc<Ui>, note: Value) {
    let id = note["id"].as_i64().unwrap_or(0);
    if let Some(d) = ui.note_drafts.borrow().get(&id) {
        if let Some(page) = ui.note_tabs.page_num(&d.layout) {
            ui.note_tabs.set_current_page(Some(page));
        } else {
            d.present();
        }
        return;
    }
    let d = editor(ui, note.clone());
    d.window.as_ref().unwrap().set_modal(false);
    d.window
        .as_ref()
        .unwrap()
        .set_application(ui.window.application().as_ref());
    d.window.as_ref().unwrap().set_child(gtk::Widget::NONE);
    let title = if text(&note, "title").is_empty() {
        "Untitled"
    } else {
        text(&note, "title")
    };
    let tab = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let project = note["project_id"].as_i64().unwrap_or(ui.project.get());
    let project_name = ui
        .projects
        .borrow()
        .iter()
        .find(|p| p["id"].as_i64() == Some(project))
        .map(|p| text(p, "name").to_string())
        .unwrap_or_else(|| format!("Project {project}"));
    tab.append(&label(&format!("{project_name} · {title}"), "dim"));
    let close = crate::app::icon_button("window-close-symbolic", "Close note");
    tab.append(&close);
    let pop = crate::app::icon_button("window-new-symbolic", "Open note in separate window");
    tab.append(&pop);
    let page = ui.note_tabs.append_page(&d.layout, Some(&tab));
    ui.note_tabs.set_tab_reorderable(&d.layout, true);
    ui.note_tabs.set_current_page(Some(page));
    ui.note_tabs.set_scrollable(true);
    let weak = Rc::downgrade(ui);
    let draft = Rc::downgrade(&d);
    *d.on_close.borrow_mut() = Some(Box::new(move || {
        if let (Some(ui), Some(d)) = (weak.upgrade(), draft.upgrade()) {
            if let Some(page) = ui.note_tabs.page_num(&d.layout) {
                ui.note_tabs.remove_page(Some(page));
            }
            ui.note_drafts.borrow_mut().remove(&id);
        }
    }));
    let draft = d.clone();
    close.connect_clicked(move |_| draft.close());
    let weak = Rc::downgrade(ui);
    let draft = d.clone();
    pop.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            if let Some(page) = ui.note_tabs.page_num(&draft.layout) {
                ui.note_tabs.remove_page(Some(page));
            }
            draft
                .window
                .as_ref()
                .unwrap()
                .set_child(Some(&draft.layout));
            draft.present();
        }
    });
    ui.note_drafts.borrow_mut().insert(id, d);
}

pub fn workspace(ui: &Rc<Ui>, name: &str, project: i64, notes: &[Value]) {
    let page = &ui.pages[name];
    // Move the open editors together, preserving every draft across Notes/Plan navigation.
    if let Some(paned) = ui.note_tabs.parent().and_downcast::<gtk::Paned>() {
        paned.set_end_child(gtk::Widget::NONE);
    }
    clear(page);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let heading = label(if name == "plan" { "Plan" } else { "Notes" }, "title");
    heading.set_hexpand(true);
    header.append(&heading);
    let add = button("New note", "primary");
    header.append(&add);
    let weak = Rc::downgrade(ui);
    let plan = name == "plan";
    add.connect_clicked(move |key|{let Some(ui)=weak.upgrade()else{return;};let key=key.clone();key.set_sensitive(false);glib::spawn_future_local(async move{match ui.call("notes.create",json!({"project_id":project,"title":if plan{"Plan"}else{"Untitled"},"body":"","pinned":plan})).await{Ok(note)=>{edit(&ui,note);ui.refresh_page();},Err(e)=>ui.show_error(&e.to_string())}key.set_sensitive(true);});});
    page.append(&header);
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_position(240);
    split.set_vexpand(true);
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    sidebar.set_size_request(180, -1);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search notes"));
    sidebar.append(&search);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    sidebar.append(&crate::app::scrolled(&list));
    let mut filters = Vec::new();
    for note in notes.iter().filter(|n| !plan || text(n, "title") == "Plan") {
        let title = text(note, "title");
        let b = button(
            &format!(
                "{}{}",
                if note["pinned"] == true {
                    "Pinned · "
                } else {
                    ""
                },
                if title.is_empty() { "Untitled" } else { title }
            ),
            "file",
        );
        if let Some(label) = b.child().and_downcast::<gtk::Label>() {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        }
        list.append(&b);
        filters.push((
            format!("{} {}", title, text(note, "body")).to_lowercase(),
            b.clone(),
        ));
        let weak = Rc::downgrade(ui);
        let note = note.clone();
        b.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                edit(&ui, note.clone());
            }
        });
    }
    search.connect_search_changed(move |s| {
        let query = s.text().to_lowercase();
        for (text, b) in &filters {
            b.set_visible(text.contains(&query));
        }
    });
    split.set_start_child(Some(&sidebar));
    split.set_end_child(Some(&ui.note_tabs));
    page.append(&split);
    if plan {
        if let Some(note) = notes.iter().find(|n| text(n, "title") == "Plan") {
            add.set_visible(false);
            edit(ui, note.clone());
        }
    } else if ui.note_tabs.n_pages() == 0
        || ui
            .note_tabs
            .current_page()
            .and_then(|p| ui.note_tabs.nth_page(Some(p)))
            .is_none_or(|current| {
                !ui.note_drafts.borrow().values().any(|d| {
                    d.layout == current && d.base.borrow()["project_id"].as_i64() == Some(project)
                })
            })
    {
        if let Some(note) = notes.first() {
            edit(ui, note.clone());
        }
    }
}

pub fn note_row(ui: &Rc<Ui>, body: &gtk::Box, note: Value) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    row.add_css_class("record");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = label(
        if text(&note, "title").is_empty() {
            "Untitled"
        } else {
            text(&note, "title")
        },
        "title",
    );
    title.set_hexpand(true);
    heading.append(&title);
    let edit_key = button("Open editor", "quiet");
    let weak = Rc::downgrade(ui);
    let n = note.clone();
    edit_key.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, n.clone())
        }
    });
    heading.append(&edit_key);
    let pin = button(
        if note["pinned"] == true {
            "Unpin"
        } else {
            "Pin"
        },
        "quiet",
    );
    let weak = Rc::downgrade(ui);
    let id = note["id"].clone();
    let pinned = note["pinned"] != true;
    pin.connect_clicked(move |b| {
        if let Some(ui) = weak.upgrade() {
            ui.mutate("notes.pin", json!({"note_id":id,"pinned":pinned}), b)
        }
    });
    heading.append(&pin);
    row.append(&heading);
    let preview: String = text(&note, "body").chars().take(500).collect();
    row.append(&paragraph(&preview));
    row.append(&label(
        if note["pinned"] == true {
            "PINNED · shared with agents"
        } else {
            "Project note"
        },
        "dim",
    ));
    body.append(&row);
}

pub fn plan(ui: &Rc<Ui>, page: &gtk::Box, notes: &[Value], project: i64) {
    page.append(&paragraph("One Markdown scratch page for you and your agents. The pinned Plan note is shared at dispatch."));
    if let Some(note) = notes.iter().find(|n| text(n, "title") == "Plan") {
        note_row(ui, page, note.clone());
    } else {
        let key = button("Create project plan", "primary");
        let weak = Rc::downgrade(ui);
        key.connect_clicked(move |b| {
            let Some(ui) = weak.upgrade() else { return };
            b.set_sensitive(false);
            let b = b.clone();
            glib::spawn_future_local(async move {
                match ui
                    .call(
                        "notes.create",
                        json!({"project_id":project,"title":"Plan","body":"","pinned":true}),
                    )
                    .await
                {
                    Ok(note) => {
                        edit(&ui, note);
                        ui.refresh_page()
                    }
                    Err(e) => ui.show_error(&e.to_string()),
                }
                b.set_sensitive(true);
            });
        });
        page.append(&key);
    }
}

pub fn modules(ui: &Rc<Ui>, body: &gtk::Box, modules: &[Value]) {
    for module in modules {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("record");
        let name = label(text(module, "name"), "title");
        name.set_hexpand(true);
        row.append(&name);
        row.append(&label(
            &format!(
                "{} · {:.0}% done{}",
                text(module, "priority"),
                module["progress_pct"].as_f64().unwrap_or(0.),
                if module["completed_at"].is_string() {
                    " · archived"
                } else {
                    ""
                }
            ),
            "dim",
        ));
        let key = button("Open module", "quiet");
        let weak = Rc::downgrade(ui);
        let id = module["id"].as_i64().unwrap_or(0);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                open_module(&ui, id)
            }
        });
        row.append(&key);
        body.append(&row);
    }
}
pub fn module_composer(ui: &Rc<Ui>, page: &gtk::Box, project: i64) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Entry::builder()
        .placeholder_text("Release outcome")
        .hexpand(true)
        .build();
    let priority = choose(&["low", "medium", "high", "urgent"], "medium");
    let key = button("Create module", "primary");
    row.append(&name);
    row.append(&priority);
    row.append(&key);
    page.append(&row);
    let weak = Rc::downgrade(ui);
    key.connect_clicked(move |b| {
        let Some(ui) = weak.upgrade() else { return };
        let value = name.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        let priority = chosen(&priority);
        b.set_sensitive(false);
        let b = b.clone();
        let name = name.clone();
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "module.create",
                    json!({"project_id":project,"name":value,"priority":priority}),
                )
                .await
            {
                Ok(m) => {
                    if name.text().trim() == value {
                        name.set_text("")
                    }
                    ui.refresh_page();
                    open_module(&ui, m["id"].as_i64().unwrap_or(0));
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            b.set_sensitive(true);
        });
    });
}
fn open_module(ui: &Rc<Ui>, id: i64) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match ui.call("module.get", json!({"module_id":id})).await {
            Ok(module) => module_detail(&ui, module),
            Err(e) => ui.show_error(&e.to_string()),
        }
    });
}
fn module_detail(ui: &Rc<Ui>, module: Value) {
    let id = module["id"].as_i64().unwrap_or(0);
    let project = module["project_id"].as_i64().unwrap_or(0);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let name = gtk::Entry::builder().text(text(&module, "name")).build();
    let priority = choose(
        &["low", "medium", "high", "urgent"],
        text(&module, "priority"),
    );
    let icon = gtk::Entry::builder()
        .text(text(&module, "icon"))
        .placeholder_text("Icon name")
        .build();
    field("Module name", &name, &form);
    field("Priority", &priority, &form);
    field("Icon", &icon, &form);
    let snapshot: Rc<dyn Fn() -> Value> = Rc::new(
        move || json!({"name":name.text().trim(),"priority":chosen(&priority),"icon":if icon.text().trim().is_empty(){Value::Null}else{json!(icon.text().trim())}}),
    );
    let d = Draft::new(ui, "Module", module.clone(), snapshot, form);
    d.controls(ui, "module.get", "module.update", "module_id", id);
    for column in super::task_pages::COLUMNS {
        d.form.append(&label(
            &column.replace('_', " ").to_uppercase(),
            "section-label",
        ));
        for task in rows(&module["tasks_by_state"], column) {
            let key = button(
                &format!("#{} {}", task["id"], text(&task, "title")),
                "quiet",
            );
            let weak = Rc::downgrade(ui);
            let task_id = task["id"].as_i64().unwrap_or(0);
            key.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    super::task_pages::open(&ui, task_id)
                }
            });
            d.form.append(&key);
        }
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Entry::builder()
        .placeholder_text("Add module task")
        .hexpand(true)
        .build();
    row.append(&title);
    super::task_pages::action(
        ui,
        &d,
        &row,
        "Add task",
        "task.create",
        move || json!({"project_id":project,"module_id":id,"title":title.text().trim(),"column":"backlog"}),
        None,
    );
    d.form.append(&row);
    let key = button("Draft patch notes", "quiet");
    d.form.append(&key);
    let preview = multiline("", 140);
    preview.set_editable(false);
    d.form.append(&preview);
    let copy = button("Copy patch notes", "quiet");
    let buffer = preview.buffer();
    copy.connect_clicked(move |b| b.clipboard().set_text(&buffer_text(&buffer)));
    d.form.append(&copy);
    let weak = Rc::downgrade(ui);
    let status = d.status.clone();
    key.connect_clicked(move |b| {
        let Some(ui) = weak.upgrade() else { return };
        let buffer = preview.buffer();
        let status = status.clone();
        b.set_sensitive(false);
        let b = b.clone();
        glib::spawn_future_local(async move {
            match ui
                .call("module.changelog.draft", json!({"module_id":id}))
                .await
            {
                Ok(v) => buffer.set_text(text(&v, "markdown")),
                Err(e) => status.set_text(&e.to_string()),
            }
            b.set_sensitive(true);
        });
    });
    let archived = module["completed_at"].is_string();
    super::task_pages::action(
        ui,
        &d,
        &d.footer,
        if archived {
            "Reopen"
        } else {
            "Complete module"
        },
        if archived {
            "module.reopen"
        } else {
            "module.complete"
        },
        move || json!({"module_id":id}),
        None,
    );
    super::task_pages::action(
        ui,
        &d,
        &d.footer,
        "Delete module",
        "module.delete",
        move || json!({"module_id":id}),
        None,
    );
    d.present();
}
