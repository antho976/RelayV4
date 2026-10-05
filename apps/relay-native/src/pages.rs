use crate::app::{button, clear, field, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[path = "guardrail_pages.rs"]
mod guardrail_pages;
#[path = "note_pages.rs"]
mod note_pages;
#[path = "notes_window.rs"]
mod notes_window;
#[path = "task_pages.rs"]
mod task_pages;
#[path = "board_view.rs"]
mod board_view;
pub use notes_window::{refresh_notes, show_notes, NotesWindow};
pub use task_pages::Draft;
pub use guardrail_pages::{guardrail_event, hold_summary, restore_prompts, settings_editor as guardrail_settings};
#[allow(unused_imports)] // entry points for the workspace and project menus
pub use guardrail_pages::{open_project_guardrails, open_workspace_guardrails};
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
    if name == "board" {
        board_view::show(ui, &ui.pages[name], project, data);
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
            "notes" => note_composer(ui, page, project),
            "modules" => note_pages::module_composer(ui, page, project),
            _ => page.append(&paragraph(
                "Answer agents that need an exception, decide which held actions may proceed, and see the exceptions still in force.",
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
            "guardrails" => "Nothing is waiting for you: no exception requests and no held actions.",
            _ => "No project notes yet.",
        }));
    }
    match name {
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
            guardrail_pages::page(ui, &body, project, data).await;
            if ui.project.get() != project || *ui.page.borrow() != name {
                return;
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
            ui.show_error("Enter a recipient and a message.");
            return;
        }
        let priority_control = priority.clone();
        let recipient_control = to.clone();
        let priority = priority.is_active();
        if priority && recipient == "*" {
            ui.show_error("Priority mail needs one session name.");
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
