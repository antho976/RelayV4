use crate::app::{button, clear, field, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[path = "note_pages.rs"]
mod note_pages;
#[path = "notes_window.rs"]
mod notes_window;
#[path = "task_pages.rs"]
mod task_pages;
pub use notes_window::{catch_up_notes, mark_notes_stale, refresh_notes, show_notes, NotesWindow};
pub use task_pages::Draft;
pub fn verify_note_tools() {
    note_pages::verify_tools();
}
pub fn open_note(ui: &Rc<Ui>, note: Value) {
    notes_window::show_project(ui, note["project_id"].as_i64().unwrap_or(ui.project.get()));
    note_pages::edit(ui, note);
}
pub fn open_task(ui: &Rc<Ui>, id: i64) {
    task_pages::open(ui, id);
}

fn paragraph(value: &str) -> gtk::Label {
    let l = label(value, "body");
    l.set_wrap(true);
    l.set_selectable(true);
    l
}

pub async fn refresh(ui: &Rc<Ui>, name: &str, project: i64) {
    let (op, key) = match name {
        "board" => ("task.list", "tasks"),
        "mailbox" => ("mailbox.list", "messages"),
        "guardrails" => ("guardrail.holds.list", "holds"),
        "notes" => ("notes.list", "notes"),
        "modules" => ("module.list", "modules"),
        _ => return,
    };
    let payload = if name == "modules" {
        json!({"project_id":project,"include_archived":true})
    } else {
        json!({"project_id":project})
    };
    let result = ui.call(op, payload).await;
    if ui.project.get() != project || *ui.page.borrow() != name {
        return;
    }
    let mut data = match result {
        Ok(v) => rows(&v, key),
        Err(e) => {
            ui.show_error(&e.to_string());
            return;
        }
    };
    if name == "board" {
        if let Ok(modules) = ui.call("module.list", json!({"project_id":project})).await {
            let modules = rows(&modules, "modules");
            for task in &mut data {
                if let Some(module) = modules
                    .iter()
                    .find(|module| module["id"] == task["module_id"])
                {
                    task["_module_name"] = module["name"].clone();
                }
            }
        }
        if ui.project.get() != project || *ui.page.borrow() != name {
            return;
        }
    }
    if name == "notes" {
        note_pages::workspace(ui, name, project, &data);
        return;
    }
    let page = &ui.pages[name];
    // Keep forms mounted while events update the list below them.
    let owner = ui.page_projects.borrow().get(name).copied();
    if owner != Some(project) {
        clear(page);
        ui.page_projects.borrow_mut().insert(name.into(), project);
        if name == "modules" {
            page.append(&board_switcher(ui, "modules"));
        }
        if name != "board" {
            page.append(&label(
                match name {
                    "board" => "Board",
                    "mailbox" => "Mailbox",
                    "guardrails" => "Guardrails",
                    "modules" => "Modules",
                    _ => "Notes",
                },
                "title",
            ));
        }
        match name {
            "mailbox" => mail_composer(ui, page, project),
            "board" => task_composer(ui, page, project),
            "notes" => note_composer(ui, page, project),
            "modules" => note_pages::module_composer(ui, page, project),
            _ => page.append(&paragraph(
                "Decide which held actions may proceed. Refused actions cannot be approved here.",
            )),
        }
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.set_vexpand(true);
        page.append(&body);
    }
    let body = page.last_child().unwrap().downcast::<gtk::Box>().unwrap();
    clear(&body);
    if data.is_empty() {
        body.append(&paragraph(match name {
            "board" => "No tasks yet. Add a task above to assign work.",
            "mailbox" => "No messages in this project.",
            "guardrails" => "No actions are waiting for approval.",
            _ => "No project notes yet.",
        }));
    }
    match name {
        "board" => board(ui, &body, &data),
        "mailbox" => {
            for message in data {
                let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
                row.add_css_class("record");
                row.append(&label(
                    &format!(
                        "{} → {}{}",
                        text(&message, "from"),
                        text(&message, "to"),
                        if message["priority"].as_bool() == Some(true) {
                            " · PRIORITY"
                        } else {
                            ""
                        }
                    ),
                    "dim",
                ));
                row.append(&paragraph(text(&message, "text")));
                row.append(&label(text(&message, "sent_at"), "dim"));
                body.append(&row);
            }
        }
        "guardrails" => {
            for hold in data {
                hold_row(ui, &body, hold);
            }
            if let Ok(overlaps) = ui.call("overlap.list", json!({"project_id":project})).await {
                if ui.project.get() != project || *ui.page.borrow() != name {
                    return;
                }
                let overlaps = rows(&overlaps, "overlaps");
                if !overlaps.is_empty() {
                    body.append(&label("Shared file activity", "title"));
                }
                for overlap in overlaps {
                    body.append(&paragraph(&format!(
                        "{}\n{}",
                        text(&overlap, "path"),
                        text(&overlap, "note")
                    )));
                }
            }
        }
        "notes" => {
            for note in data {
                note_pages::note_row(ui, &body, note);
            }
        }
        "modules" => note_pages::modules(ui, &body, &data),
        _ => {}
    }
}
pub fn workspace_picker(ui: &Rc<Ui>, project: i64, destination: &str) -> gtk::MenuButton {
    let picker = gtk::MenuButton::new();
    picker.add_css_class("workspace-picker");
    picker.set_valign(gtk::Align::Center);
    let projects = ui.projects.borrow();
    let spaces = ui.workspaces.borrow();
    let current = projects.iter().find(|p| p["id"].as_i64() == Some(project));
    let workspace = current.and_then(|p| spaces.iter().find(|w| w["id"] == p["workspace_id"]));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let caption = label(
        &format!(
            "{} / {}",
            workspace.map(|w| text(w, "name")).unwrap_or("Workspace"),
            current.map(|p| text(p, "name")).unwrap_or("Project")
        ),
        "workspace-picker-label",
    );
    caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
    caption.set_max_width_chars(30);
    content.append(&caption);
    content.append(&crate::icons::image("chevron-down", 12));
    picker.set_child(Some(&content));
    let popover = gtk::Popover::new();
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    menu.set_size_request(240, -1);
    for space in spaces.iter() {
        let owned: Vec<_> = projects
            .iter()
            .filter(|p| p["workspace_id"] == space["id"])
            .collect();
        if owned.is_empty() {
            continue;
        }
        menu.append(&label(text(space, "name"), "section-label"));
        for item in owned {
            let key = button(text(item, "name"), "workspace-project");
            let id = item["id"].as_i64().unwrap_or(0);
            if id == project {
                key.add_css_class("selected");
            }
            let weak = Rc::downgrade(ui);
            let pop = popover.downgrade();
            let destination = destination.to_string();
            key.connect_clicked(move |_| {
                if let Some(pop) = pop.upgrade() {
                    pop.popdown();
                }
                if let Some(ui) = weak.upgrade() {
                    if destination == "notes" {
                        notes_window::show_project(&ui, id);
                    } else if ui.project.get() != id {
                        ui.open_project(id, &destination);
                    }
                }
            });
            menu.append(&key);
        }
    }
    let scroll = crate::app::scrolled(&menu);
    scroll.set_max_content_height(480);
    scroll.set_propagate_natural_height(true);
    popover.set_child(Some(&scroll));
    picker.set_popover(Some(&popover));
    picker
}

pub fn task_mark(kind: &str, size: i32) -> gtk::DrawingArea {
    let mark = gtk::DrawingArea::new();
    mark.set_content_width(size);
    mark.set_content_height(size);
    mark.set_valign(gtk::Align::Center);
    let kind = kind.to_string();
    mark.set_draw_func(move |widget, cr, width, height| {
        let Some(color) = widget.style_context().lookup_color("secondary") else {
            return;
        };
        cr.set_source_rgba(
            color.red() as f64,
            color.green() as f64,
            color.blue() as f64,
            color.alpha() as f64,
        );
        let w = width as f64;
        let h = height as f64;
        cr.set_line_width(1.);
        match kind.as_str() {
            "feature" => {
                cr.rectangle(0., 0., w, h);
                let _ = cr.fill();
            }
            "chore" => {
                for y in [1., 4., 7.] {
                    cr.rectangle(0., y, w, 1.);
                }
                let _ = cr.fill();
            }
            "spike" => {
                let _ = cr.save();
                cr.translate(w / 2., h / 2.);
                cr.rotate(std::f64::consts::FRAC_PI_4);
                cr.rectangle(-w / 2. + 1.5, -h / 2. + 1.5, w - 3., h - 3.);
                let _ = cr.stroke();
                let _ = cr.restore();
            }
            _ => {
                cr.rectangle(0.5, 0.5, w - 1., h - 1.);
                let _ = cr.stroke();
                if kind == "bug" {
                    cr.rectangle(3., 3., w - 6., h - 6.);
                    let _ = cr.fill();
                }
            }
        }
    });
    mark
}

fn board_switcher(ui: &Rc<Ui>, selected: &str) -> gtk::Box {
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    tabs.add_css_class("board-switcher");
    tabs.set_valign(gtk::Align::Center);
    for (name, caption) in [("board", "BOARD"), ("modules", "MODULES")] {
        let key = button(caption, "quiet");
        if name == selected {
            key.add_css_class("selected");
        }
        let weak = Rc::downgrade(ui);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.navigate(name);
            }
        });
        tabs.append(&key);
    }
    tabs
}

fn task_composer(ui: &Rc<Ui>, page: &gtk::Box, project: i64) {
    page.add_css_class("board-page");
    page.set_spacing(0);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    header.add_css_class("board-head");
    header.append(&board_switcher(ui, "board"));
    header.append(&workspace_picker(ui, project, "board"));
    header.append(&label("Board", "title"));
    let count = label("", "dim");
    count.set_widget_name("board-count");
    count.set_hexpand(true);
    header.append(&count);
    let lens = button("", "quiet");
    let contents = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    contents.append(&crate::icons::image("sliders", 12));
    contents.append(&label("Lens", ""));
    lens.set_child(Some(&contents));
    header.append(&lens);
    let undo = button("", "quiet");
    let contents = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    contents.append(&crate::icons::image("undo", 12));
    contents.append(&label("Undo", ""));
    undo.set_child(Some(&contents));
    header.append(&undo);
    let weak = Rc::downgrade(ui);
    undo.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        key.set_sensitive(false);
        let key = key.clone();
        glib::spawn_future_local(async move {
            let result = async {
                let history = ui
                    .call(
                        "audit.list",
                        json!({"project_id":project,"actor":"user","op_prefix":"task.","limit":50}),
                    )
                    .await?;
                if let Some(row) = rows(&history, "rows")
                    .iter()
                    .find(|r| !r["undo_op"].is_null() && r["undone_by"].is_null())
                {
                    ui.call("audit.undo", json!({"audit_id":row["id"]})).await?;
                }
                Ok::<(), crate::client::Error>(())
            }
            .await;
            if let Err(error) = result {
                ui.show_error(&error.to_string());
            }
            key.set_sensitive(true);
            ui.refresh_page();
        });
    });
    let add = button("+ Task", "primary");
    header.append(&add);
    for child in widgets(&header) {
        child.set_valign(gtk::Align::Center);
    }
    page.append(&header);
    let weak = Rc::downgrade(ui);
    add.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            task_pages::compose(&ui, project);
        }
    });
    let filters = gtk::FlowBox::new();
    filters.set_selection_mode(gtk::SelectionMode::None);
    filters.set_min_children_per_line(1);
    filters.set_max_children_per_line(10);
    filters.set_column_spacing(4);
    filters.set_row_spacing(4);
    filters.add_css_class("board-lens");
    let query = gtk::SearchEntry::builder()
        .placeholder_text("Search title, body, labels, agents…")
        .hexpand(true)
        .build();
    query.set_widget_name("board-query");
    query.set_size_request(220, -1);
    filters.insert(&query, -1);
    let mut choices = Vec::new();
    for (key, title, values) in [
        (
            "type",
            "Type",
            vec!["task", "feature", "bug", "chore", "spike"],
        ),
        (
            "priority",
            "Priority",
            vec!["low", "medium", "high", "urgent"],
        ),
        ("size", "Size", vec!["S", "M", "L"]),
        ("module", "Module", vec![]),
        ("label", "Label", vec![]),
        ("parent", "Parent", vec!["roots"]),
        ("session", "Agent", vec![]),
        (
            "group",
            "Group",
            vec!["parent", "type", "priority", "size", "module", "session"],
        ),
    ] {
        let control = gtk::ComboBoxText::new();
        control.append(Some(""), title);
        for value in values {
            control.append(
                Some(value),
                if value == "roots" {
                    "Top level only"
                } else {
                    value
                },
            );
        }
        control.set_active(Some(0));
        control.set_widget_name(&format!("board-{key}"));
        control.set_size_request(104, -1);
        filters.insert(&control, -1);
        let target = page.downgrade();
        control.connect_changed(move |_| {
            if let Some(page) = target.upgrade() {
                filter_board(&page);
            }
        });
        choices.push(control);
    }
    let clear = button("Clear", "quiet");
    filters.insert(&clear, -1);
    let q = query.clone();
    clear.connect_clicked(move |_| {
        q.set_text("");
        for choice in &choices {
            choice.set_active(Some(0));
        }
    });
    let target = page.downgrade();
    query.connect_search_changed(move |_| {
        if let Some(page) = target.upgrade() {
            filter_board(&page);
        }
    });
    filters.set_visible(false);
    let filter_box = filters.clone();
    lens.connect_clicked(move |key| {
        let show = !filter_box.is_visible();
        filter_box.set_visible(show);
        if show {
            key.add_css_class("active");
        } else {
            key.remove_css_class("active");
        }
    });
    page.append(&filters);
    let rail = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    rail.set_widget_name("board-drag-rail");
    rail.add_css_class("board-drag-rail");
    rail.append(&label("DROP TO SET", "dim"));
    for (field, values) in [
        ("priority", vec!["low", "medium", "high", "urgent"]),
        ("size", vec!["S", "M", "L"]),
        ("type", vec!["task", "feature", "bug", "chore", "spike"]),
    ] {
        for value in values {
            let key = button(value, "quiet");
            rail.append(&key);
            let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
            let weak = Rc::downgrade(ui);
            drop.connect_drop(move |_, token, _, _| {
                let Some(ui) = weak.upgrade() else {
                    return false;
                };
                let Some(id) = token.get::<String>().ok().and_then(|s| {
                    s.strip_prefix("relay-task:")
                        .and_then(|id| id.parse::<i64>().ok())
                }) else {
                    return false;
                };
                if ui.project.get() != project {
                    return false;
                }
                glib::spawn_future_local(async move {
                    if let Err(error) = ui
                        .call("task.update", json!({"task_id":id,field:value}))
                        .await
                    {
                        ui.show_error(&error.to_string());
                    }
                    ui.refresh_page();
                });
                true
            });
            key.add_controller(drop);
        }
    }
    rail.set_visible(false);
    page.append(&rail);
    let keyboard = gtk::EventControllerKey::new();
    keyboard.set_propagation_phase(gtk::PropagationPhase::Capture);
    let target = page.downgrade();
    let weak = Rc::downgrade(ui);
    keyboard.connect_key_pressed(move |_, key, _, mods| {
        let (Some(page), Some(ui)) = (target.upgrade(), weak.upgrade()) else {
            return glib::Propagation::Proceed;
        };
        let focus = page.root().and_then(|root| root.focus());
        let mut ancestor = focus.clone();
        while let Some(widget) = ancestor {
            if widget.is::<gtk::Editable>() || widget.is::<gtk::TextView>() {
                return glib::Propagation::Proceed;
            }
            ancestor = widget.parent();
        }
        if mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) && key == gtk::gdk::Key::z {
            undo.emit_clicked();
            return glib::Propagation::Stop;
        }
        if mods.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK,
        ) {
            return glib::Propagation::Proceed;
        }
        let controls = widgets(&page);
        let cards: Vec<_> = controls
            .iter()
            .filter(|w| w.widget_name() == "task-card" && w.is_visible())
            .collect();
        let current = cards.iter().position(|card| {
            focus
                .as_ref()
                .is_some_and(|f| f == *card || f.is_ancestor(*card))
        });
        match key {
            gtk::gdk::Key::n => add.emit_clicked(),
            gtk::gdk::Key::f => lens.emit_clicked(),
            gtk::gdk::Key::slash => {
                filters.set_visible(true);
                query.grab_focus();
            }
            gtk::gdk::Key::g => {
                if let Some(group) = controls
                    .iter()
                    .find(|w| w.widget_name() == "board-group")
                    .and_then(|w| w.clone().downcast::<gtk::ComboBoxText>().ok())
                {
                    let next = group.active().unwrap_or(0) + 1;
                    let count = group
                        .model()
                        .map(|m| m.iter_n_children(None) as u32)
                        .unwrap_or(1);
                    group.set_active(Some(next % count.max(1)));
                }
            }
            gtk::gdk::Key::Down | gtk::gdk::Key::j | gtk::gdk::Key::Up | gtk::gdk::Key::k => {
                if !cards.is_empty() {
                    let delta = if matches!(key, gtk::gdk::Key::Up | gtk::gdk::Key::k) {
                        -1
                    } else {
                        1
                    };
                    let at = current
                        .map(|i| (i as i32 + delta).clamp(0, cards.len() as i32 - 1) as usize)
                        .unwrap_or(0);
                    cards[at].grab_focus();
                }
            }
            gtk::gdk::Key::Left | gtk::gdk::Key::h | gtk::gdk::Key::Right | gtk::gdk::Key::l => {
                let columns = ["backlog", "ready", "active", "in_review", "done"];
                let delta = if matches!(key, gtk::gdk::Key::Left | gtk::gdk::Key::h) {
                    -1
                } else {
                    1
                };
                let column = current
                    .map(|i| card_task(cards[i]))
                    .and_then(|t| columns.iter().position(|c| *c == text(&t, "column")));
                let mut at = column
                    .map(|i| i as i32)
                    .unwrap_or(if delta > 0 { -1 } else { 5 })
                    + delta;
                while (0..5).contains(&at) {
                    if let Some(card) = cards
                        .iter()
                        .find(|c| text(&card_task(c), "column") == columns[at as usize])
                    {
                        card.grab_focus();
                        break;
                    }
                    at += delta;
                }
            }
            gtk::gdk::Key::Return | gtk::gdk::Key::space => {
                if let Some(at) = current {
                    if key == gtk::gdk::Key::space {
                        if let Some(preview) = widgets(cards[at])
                            .into_iter()
                            .find(|w| w.has_css_class("task-preview"))
                        {
                            preview.set_visible(!preview.is_visible());
                        }
                        return glib::Propagation::Stop;
                    }
                    let name = "board-open";
                    if let Some(key) = widgets(cards[at])
                        .into_iter()
                        .find(|w| w.widget_name() == name)
                        .and_then(|w| w.downcast::<gtk::Button>().ok())
                    {
                        key.emit_clicked();
                    }
                }
            }
            gtk::gdk::Key::bracketleft | gtk::gdk::Key::bracketright => {
                if let Some(at) = current {
                    let task = card_task(cards[at]);
                    let columns = ["backlog", "ready", "active", "in_review", "done"];
                    if let Some(column) = columns.iter().position(|c| *c == text(&task, "column")) {
                        let delta = if key == gtk::gdk::Key::bracketleft {
                            -1
                        } else {
                            1
                        };
                        let next = (column as i32 + delta).clamp(0, 4) as usize;
                        glib::spawn_future_local(async move {
                            if let Err(error) = ui
                                .call(
                                    "task.move",
                                    json!({"task_id":task["id"], "column":columns[next]}),
                                )
                                .await
                            {
                                ui.show_error(&error.to_string());
                            }
                            ui.refresh_page();
                        });
                    }
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    page.add_controller(keyboard.clone());
    let target = page.downgrade();
    header.connect_destroy(move |_| {
        if let Some(page) = target.upgrade() {
            page.remove_controller(&keyboard);
        }
    });
}
fn board(ui: &Rc<Ui>, body: &gtk::Box, tasks: &[Value]) {
    let mut enriched = tasks.to_vec();
    for task in &mut enriched {
        if let Some(parent) = tasks.iter().find(|p| p["id"] == task["parent_id"]) {
            task["_parent_title"] = parent["title"].clone();
        }
    }
    let tasks = enriched.as_slice();
    if let Some(page) = body.parent().and_downcast::<gtk::Box>() {
        for (key, title) in [
            ("module", "Module"),
            ("label", "Label"),
            ("parent", "Parent"),
            ("session", "Agent"),
        ] {
            if let Some(control) = widgets(&page)
                .into_iter()
                .find(|w| w.widget_name() == format!("board-{key}"))
                .and_then(|w| w.downcast::<gtk::ComboBoxText>().ok())
            {
                let current = task_pages::chosen(&control);
                control.remove_all();
                control.append(Some(""), title);
                let mut values = std::collections::BTreeMap::new();
                if key == "parent" {
                    values.insert("roots".to_string(), "Top level only".to_string());
                }
                for task in tasks {
                    match key {
                        "module" => {
                            if let Some(id) = task["module_id"].as_i64() {
                                values.insert(
                                    id.to_string(),
                                    task["_module_name"]
                                        .as_str()
                                        .map(str::to_string)
                                        .unwrap_or_else(|| format!("Module #{id}")),
                                );
                            }
                        }
                        "parent" => {
                            if tasks.iter().any(|t| t["parent_id"] == task["id"]) {
                                values.insert(
                                    task["id"].to_string(),
                                    text(task, "title").to_string(),
                                );
                            }
                        }
                        _ => {
                            for value in
                                rows(task, if key == "label" { "labels" } else { "sessions" })
                            {
                                if let Some(value) = value.as_str() {
                                    values.insert(value.to_string(), value.to_string());
                                }
                            }
                        }
                    }
                }
                for (id, title) in values {
                    control.append(Some(&id), &title);
                }
                if !control.set_active_id(Some(&current)) {
                    control.set_active(Some(0));
                }
            }
        }
    }
    let grid = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    grid.set_homogeneous(true);
    grid.set_vexpand(true);
    grid.add_css_class("board-grid");
    if let Some(page) = body.parent().and_downcast::<gtk::Box>() {
        let open = tasks.iter().filter(|t| text(t, "column") != "done").count();
        for widget in widgets(&page) {
            if widget.widget_name() == "board-count" {
                if let Ok(count) = widget.downcast::<gtk::Label>() {
                    count.set_text(&format!("{open} open · {} done", tasks.len() - open));
                }
            }
        }
    }
    let board_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&grid)
        .build();
    body.append(&board_scroll);
    let hints = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    hints.add_css_class("board-shortcuts");
    for (key, action) in [
        ("↑ ↓ ← →", "move"),
        ("Enter", "open"),
        ("Space", "preview"),
        ("[ ]", "change column"),
        ("N", "new"),
        ("/", "search"),
        ("G", "group"),
        ("F", "lens"),
    ] {
        hints.append(&label(key, "board-kbd"));
        hints.append(&label(action, ""));
    }
    body.append(&hints);
    for (name, title) in [
        ("backlog", "BACKLOG"),
        ("ready", "READY"),
        ("active", "ACTIVE"),
        ("in_review", "IN REVIEW"),
        ("done", "DONE"),
    ] {
        let lane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        lane.add_css_class("board-lane");
        lane.set_size_request(210, 320);
        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        let weak = Rc::downgrade(ui);
        let project = ui.project.get();
        drop.connect_drop(move |_, value, _, _| {
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            let Ok(token) = value.get::<String>() else {
                return false;
            };
            let Some(id) = token
                .strip_prefix("relay-task:")
                .and_then(|s| s.parse::<i64>().ok())
            else {
                return false;
            };
            if ui.project.get() != project {
                return false;
            }
            glib::spawn_future_local(async move {
                if let Err(e) = ui
                    .call("task.move", json!({"task_id":id,"column":name}))
                    .await
                {
                    ui.show_error(&e.to_string());
                }
                ui.refresh_page();
            });
            true
        });
        lane.add_controller(drop);
        lane.append(&label(
            &format!(
                "{title}  {}",
                tasks.iter().filter(|t| text(t, "column") == name).count()
            ),
            "lane-heading",
        ));
        grid.append(&lane);
        let cards = gtk::Box::new(gtk::Orientation::Vertical, 6);
        cards.add_css_class("board-cards");
        let scroll = crate::app::scrolled(&cards);
        scroll.set_vexpand(true);
        lane.append(&scroll);
        if !tasks.iter().any(|t| text(t, "column") == name) {
            let empty = label("EMPTY", "board-empty");
            empty.set_halign(gtk::Align::Center);
            cards.append(&empty);
        }
        for task in tasks.iter().filter(|t| text(t, "column") == name) {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
            row.add_css_class("task-card");
            row.set_widget_name("task-card");
            row.set_focusable(true);
            let data = label(&task.to_string(), "");
            data.set_widget_name("board-card-data");
            data.set_visible(false);
            row.append(&data);
            row.set_tooltip_text(Some(text(task, "title")));
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            heading.append(&task_mark(text(task, "type"), 9));
            heading.append(&label(&text(task, "type").to_uppercase(), "task-type"));
            if text(task, "priority") != "medium" {
                heading.append(&label(text(task, "priority"), "task-chip"));
            }
            let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            space.set_hexpand(true);
            heading.append(&space);
            let open = crate::app::icon_button("external", "Open task");
            open.add_css_class("task-open");
            open.set_widget_name("board-open");
            let copy = crate::app::icon_button("copy", "Copy title, body and id");
            copy.add_css_class("task-open");
            let contents = format!(
                "{}\n\n{}\n\n#{}",
                text(task, "title"),
                text(task, "body"),
                task["id"]
            );
            copy.connect_clicked(move |key| key.clipboard().set_text(&contents));
            heading.append(&copy);
            heading.append(&open);
            row.append(&heading);
            let weak = Rc::downgrade(ui);
            let task_id = task["id"].as_i64().unwrap_or(0);
            let drag = gtk::DragSource::new();
            drag.set_actions(gtk::gdk::DragAction::MOVE);
            drag.connect_prepare(move |_, _, _| {
                Some(gtk::gdk::ContentProvider::for_value(
                    &format!("relay-task:{task_id}").to_value(),
                ))
            });
            let target = body
                .parent()
                .and_downcast::<gtk::Box>()
                .map(|p| p.downgrade());
            let begin = target.clone();
            drag.connect_drag_begin(move |_, _| {
                if let Some(page) = begin.as_ref().and_then(glib::WeakRef::upgrade) {
                    for widget in widgets(&page) {
                        if widget.widget_name() == "board-drag-rail" {
                            widget.set_visible(true);
                        }
                    }
                }
            });
            drag.connect_drag_end(move |_, _, _| {
                if let Some(page) = target.as_ref().and_then(glib::WeakRef::upgrade) {
                    for widget in widgets(&page) {
                        if widget.widget_name() == "board-drag-rail" {
                            widget.set_visible(false);
                        }
                    }
                }
            });
            row.add_controller(drag);
            open.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    task_pages::open(&ui, task_id)
                }
            });
            let title = button(text(task, "title"), "task-title");
            title.set_widget_name("board-title");
            if let Some(label) = title.child().and_downcast::<gtk::Label>() {
                label.set_wrap(true);
                label.set_xalign(0.0);
            }
            row.append(&title);
            if let Some(parent) = task["_parent_title"].as_str() {
                row.set_margin_start(10);
                row.add_css_class("task-child");
                let lineage = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                lineage.append(&crate::icons::image("chevron-right", 11));
                let caption = label(parent, "task-lineage");
                caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lineage.append(&caption);
                row.append(&lineage);
            }
            let total = task["rollup"]["total"].as_i64().unwrap_or(0);
            let done = task["rollup"]["done"].as_i64().unwrap_or(0);
            if total > 0 {
                let rollup = gtk::Box::new(gtk::Orientation::Horizontal, 7);
                rollup.add_css_class("task-rollup");
                let meter = gtk::ProgressBar::new();
                meter.set_fraction((done as f64 / total as f64).clamp(0., 1.));
                meter.set_hexpand(true);
                meter.set_valign(gtk::Align::Center);
                rollup.append(&meter);
                rollup.append(&label(&format!("{done}/{total}"), "task-lineage"));
                row.append(&rollup);
            }
            let blockers: Vec<_> = rows(task, "blocked_by")
                .into_iter()
                .filter_map(|id| {
                    tasks.iter().find(|candidate| {
                        candidate["id"] == id && text(candidate, "column") != "done"
                    })
                })
                .collect();
            if !blockers.is_empty() {
                let caption = label(
                    &format!(
                        "Blocked by {}",
                        if blockers.len() == 1 {
                            text(blockers[0], "title").to_string()
                        } else {
                            format!("{} tasks", blockers.len())
                        }
                    ),
                    "task-lineage",
                );
                caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
                row.append(&caption);
            }
            if let Some(duplicate) = tasks
                .iter()
                .find(|candidate| candidate["id"] == task["duplicate_of"])
            {
                let caption = label(
                    &format!("Duplicate of {}", text(duplicate, "title")),
                    "task-lineage",
                );
                caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
                row.append(&caption);
            }
            let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
            let weak = Rc::downgrade(ui);
            let project = ui.project.get();
            drop.connect_drop(move |_, token, _, _| {
                let Some(ui) = weak.upgrade() else {
                    return false;
                };
                let Some(child) = token.get::<String>().ok().and_then(|s| {
                    s.strip_prefix("relay-task:")
                        .and_then(|s| s.parse::<i64>().ok())
                }) else {
                    return false;
                };
                if child == task_id || ui.project.get() != project {
                    return false;
                }
                glib::spawn_future_local(async move {
                    if let Err(error) = ui
                        .call(
                            "task.parent.set",
                            json!({"task_id":child,"parent_id":task_id}),
                        )
                        .await
                    {
                        ui.show_error(&error.to_string());
                    }
                    ui.refresh_page();
                });
                true
            });
            row.add_controller(drop);
            let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            metadata.append(&label(&format!("#{}", task["id"]), "dim"));
            if !text(task, "size").is_empty() {
                metadata.append(&label(text(task, "size"), "task-chip"));
            }
            if let Some(module) = task["_module_name"].as_str() {
                let caption = label(module, "task-lineage");
                caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
                metadata.append(&caption);
            }
            if let Some(commit) = task["commits"]
                .as_array()
                .and_then(|list| list.last())
                .and_then(|c| c["sha"].as_str())
            {
                metadata.append(&label(
                    &commit.chars().take(7).collect::<String>(),
                    "task-chip",
                ));
            }
            row.append(&metadata);
            if let Some(tags) = task["labels"].as_array() {
                let tags = tags
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" · ");
                if !tags.is_empty() {
                    row.append(&label(&tags, "task-tags"));
                }
            }
            let preview = paragraph(text(task, "body"));
            preview.add_css_class("task-preview");
            preview.set_visible(false);
            row.append(&preview);
            let weak = Rc::downgrade(ui);
            title.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    task_pages::open(&ui, task_id);
                }
            });
            let toggle = gtk::GestureClick::new();
            toggle.set_button(3);
            toggle.connect_released(move |_, _, _, _| preview.set_visible(!preview.is_visible()));
            row.add_controller(toggle);
            let agents = rows(task, "sessions");
            if !agents.is_empty() {
                let names = agents
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" · ");
                let session = label(&names, "task-agent");
                session.set_ellipsize(gtk::pango::EllipsizeMode::End);
                session.set_max_width_chars(25);
                session.set_tooltip_text(Some(&names));
                row.append(&session);
            }
            cards.append(&row);
        }
    }
    if let Some(page) = body.parent().and_downcast::<gtk::Box>() {
        filter_board(&page);
    }
}
fn widgets(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut result = Vec::new();
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(widgets(&widget));
        result.push(widget);
    }
    result
}
fn task_matches(task: &Value, filters: &[(&str, String)], query: &str) -> bool {
    filters.iter().all(|(key, value)| {
        value.is_empty()
            || match *key {
                "parent" if value == "roots" => task["parent_id"].is_null(),
                "parent" => task["parent_id"]
                    .as_i64()
                    .is_some_and(|id| id.to_string() == *value),
                "module" => task["module_id"]
                    .as_i64()
                    .is_some_and(|id| id.to_string() == *value),
                "label" => task["labels"]
                    .as_array()
                    .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(value))),
                "session" => task["sessions"]
                    .as_array()
                    .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(value))),
                key => text(task, key) == value,
            }
    }) && format!(
        "{} {} {} {} {}",
        task["id"],
        text(task, "title"),
        text(task, "body"),
        task["labels"],
        task["sessions"]
    )
    .to_lowercase()
    .contains(query)
}

fn card_task(card: &gtk::Widget) -> Value {
    widgets(card)
        .into_iter()
        .find(|w| w.widget_name() == "board-card-data")
        .and_then(|w| w.downcast::<gtk::Label>().ok())
        .and_then(|l| serde_json::from_str(&l.text()).ok())
        .unwrap_or(Value::Null)
}

fn filter_board(page: &gtk::Box) {
    let controls = widgets(page);
    let query = controls
        .iter()
        .find(|w| w.widget_name() == "board-query")
        .and_then(|w| w.clone().downcast::<gtk::SearchEntry>().ok())
        .map(|w| w.text().trim().to_lowercase())
        .unwrap_or_default();
    let selected = |name: &str| {
        controls
            .iter()
            .find(|w| w.widget_name() == name)
            .and_then(|w| w.clone().downcast::<gtk::ComboBoxText>().ok())
            .map(|w| task_pages::chosen(&w))
            .unwrap_or_default()
    };
    let filters: Vec<_> = [
        "type", "priority", "size", "module", "label", "parent", "session",
    ]
    .into_iter()
    .map(|key| (key, selected(&format!("board-{key}"))))
    .collect();
    let grouping = selected("board-group");
    for card in controls.iter().filter(|w| w.widget_name() == "task-card") {
        card.set_visible(task_matches(&card_task(card), &filters, &query));
    }
    for container in controls.iter().filter(|w| w.has_css_class("board-cards")) {
        let Some(container) = container.downcast_ref::<gtk::Box>() else {
            continue;
        };
        let mut cards = Vec::new();
        let mut child = container.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget.widget_name() == "task-card" {
                cards.push((card_task(&widget), widget.clone()));
            }
            container.remove(&widget);
        }
        let group = |task: &Value| -> String {
            match grouping.as_str() {
                "parent" => task["_parent_title"]
                    .as_str()
                    .unwrap_or("Top level")
                    .to_string(),
                "module" => task["_module_name"]
                    .as_str()
                    .unwrap_or("No module")
                    .to_string(),
                "session" => task["sessions"]
                    .as_array()
                    .and_then(|v| v.last())
                    .and_then(Value::as_str)
                    .unwrap_or("Unassigned")
                    .to_string(),
                "" => String::new(),
                key => task[key].as_str().unwrap_or("None").to_string(),
            }
        };
        cards.sort_by(|(a, _), (b, _)| {
            group(a)
                .cmp(&group(b))
                .then_with(|| a["position"].as_i64().cmp(&b["position"].as_i64()))
        });
        let visible = cards.iter().filter(|(_, w)| w.is_visible()).count();
        let mut previous = None;
        for (task, card) in cards {
            if !grouping.is_empty() && card.is_visible() {
                let name = group(&task);
                if previous.as_ref() != Some(&name) {
                    container.append(&label(&name, "board-group-heading"));
                    previous = Some(name);
                }
            }
            container.append(&card);
        }
        if visible == 0 {
            let empty = label(
                if query.is_empty() && filters.iter().all(|(_, v)| v.is_empty()) {
                    "EMPTY"
                } else {
                    "NO MATCH"
                },
                "board-empty",
            );
            empty.set_halign(gtk::Align::Center);
            container.append(&empty);
        }
    }
}

fn mail_composer(ui: &Rc<Ui>, page: &gtk::Box, project: i64) {
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let to = gtk::Entry::builder()
        .placeholder_text("Session name, or * for everyone")
        .build();
    let message = gtk::TextView::new();
    message.set_wrap_mode(gtk::WrapMode::WordChar);
    message.set_size_request(-1, 90);
    field("To", &to, &form);
    field("Message", &message, &form);
    let priority =
        gtk::CheckButton::with_label("Priority: ask this agent to read it at the next safe point");
    form.append(&priority);
    let send = button("Send message", "primary");
    form.append(&send);
    page.append(&form);
    let weak = Rc::downgrade(ui);
    send.connect_clicked(move |b| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let recipient = to.text().trim().to_string();
        let buffer = message.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        if recipient.is_empty() || text.trim().is_empty() {
            ui.show_info("Enter a recipient and a message.");
            return;
        }
        let priority_control = priority.clone();
        let recipient_control = to.clone();
        let priority = priority.is_active();
        if priority && recipient == "*" {
            ui.show_info("Priority mail needs one session name.");
            return;
        }
        b.set_sensitive(false);
        let b = b.clone();
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "mailbox.send",
                    json!({"project_id":project,"to":recipient,"text":text,"priority":priority}),
                )
                .await
            {
                Ok(v) => {
                    if buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), false)
                        .as_str()
                        == text
                        && recipient_control.text().trim() == recipient
                        && priority_control.is_active() == priority
                    {
                        buffer.set_text("");
                    }
                    ui.show_error(&format!("Message: {}", crate::app::text(&v, "delivery")));
                    ui.refresh_page();
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            b.set_sensitive(true);
        });
    });
}
fn hold_row(ui: &Rc<Ui>, body: &gtk::Box, hold: Value) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
    row.add_css_class("record");
    row.append(&label(
        &format!("{} · {}", text(&hold, "session"), text(&hold, "policy")),
        "title",
    ));
    row.append(&paragraph(&format!(
        "{}\n{}",
        text(&hold, "op"),
        hold["details"]
    )));
    let inspect = button("Review exact action", "quiet");
    row.append(&inspect);
    let exact = paragraph("");
    row.append(&exact);
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&keys);
    let allow = Rc::new(RefCell::new(None::<gtk::Button>));
    for (caption, op) in [
        ("Allow once", "guardrail.confirm"),
        ("Reject", "guardrail.reject"),
    ] {
        let b = button(
            caption,
            if op.ends_with("confirm") {
                "primary"
            } else {
                "quiet"
            },
        );
        keys.append(&b);
        let weak = Rc::downgrade(ui);
        let id = hold["id"].clone();
        if op.ends_with("confirm") {
            b.set_sensitive(false);
            *allow.borrow_mut() = Some(b.clone());
        }
        b.connect_clicked(move |b| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            b.set_sensitive(false);
            let b = b.clone();
            let id = id.clone();
            glib::spawn_future_local(async move {
                match ui.call(op, json!({"hold_id":id})).await {
                    Ok(v) => {
                        if v.get("outcome").is_some() && v["outcome"]["ok"] == false {
                            ui.show_error(&format!("Action failed: {}", v["outcome"]["error"]));
                        }
                        ui.refresh_page();
                    }
                    Err(e) => ui.show_error(&e.to_string()),
                }
                b.set_sensitive(true);
            });
        });
    }
    let weak = Rc::downgrade(ui);
    let id = hold["id"].clone();
    inspect.connect_clicked(move |b| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let exact = exact.clone();
        let id = id.clone();
        let allow = allow.clone();
        b.set_sensitive(false);
        let b = b.clone();
        glib::spawn_future_local(async move {
            match ui.call("guardrail.hold.get", json!({"hold_id":id})).await {
                Ok(v) => {
                    exact
                        .set_text(&serde_json::to_string_pretty(&v["request"]).unwrap_or_default());
                    if let Some(allow) = allow.borrow().as_ref() {
                        allow.set_sensitive(true);
                    }
                }
                Err(e) => ui.show_error(&format!(
                    "Cannot inspect this hold: {e}. Use the matching rebuilt engine."
                )),
            }
            b.set_sensitive(true);
        });
    });
    body.append(&row);
}
fn note_composer(ui: &Rc<Ui>, page: &gtk::Box, project: i64) {
    let title = gtk::Entry::builder().placeholder_text("Note title").build();
    let body = gtk::TextView::new();
    body.set_size_request(-1, 90);
    body.set_wrap_mode(gtk::WrapMode::WordChar);
    page.append(&title);
    page.append(&body);
    let add = button("Add note", "primary");
    page.append(&add);
    let weak = Rc::downgrade(ui);
    add.connect_clicked(move |b| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let name = title.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let buffer = body.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        let title = title.clone();
        b.set_sensitive(false);
        let b = b.clone();
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "notes.create",
                    json!({"project_id":project,"title":name,"body":text}),
                )
                .await
            {
                Ok(_) => {
                    if title.text().trim() == name
                        && buffer
                            .text(&buffer.start_iter(), &buffer.end_iter(), false)
                            .as_str()
                            == text
                    {
                        title.set_text("");
                        buffer.set_text("");
                    }
                    ui.refresh_page();
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            b.set_sensitive(true);
        });
    });
}

#[cfg(test)]
mod board_tests {
    use super::*;
    #[test]
    fn lens_combines_metadata_and_text_without_matching_unrelated_fields() {
        let task = json!({"id":9,"title":"Fix notes","body":"Keep drafts","type":"bug","priority":"high","size":"M","parent_id":2,"module_id":3,"labels":["native"],"sessions":["egret"]});
        assert!(task_matches(
            &task,
            &[
                ("type", "bug".into()),
                ("label", "native".into()),
                ("session", "egret".into()),
                ("parent", "2".into())
            ],
            "drafts"
        ));
        assert!(!task_matches(&task, &[("parent", "roots".into())], ""));
        assert!(!task_matches(&task, &[("module", "4".into())], ""));
        assert!(!task_matches(&task, &[], "high"));
    }
}
