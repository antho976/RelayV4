use crate::app::{button, clear, field, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

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
        _ => return,
    };
    let result = ui.call(op, json!({"project_id":project})).await;
    if ui.project.get() != project || *ui.page.borrow() != name {
        return;
    }
    let data = match result {
        Ok(v) => rows(&v, key),
        Err(e) => {
            ui.show_error(&e.to_string());
            return;
        }
    };
    let page = &ui.pages[name];
    // Keep forms mounted while events update the list below them.
    let owner = ui.page_projects.borrow().get(name).copied();
    if owner != Some(project) {
        clear(page);
        ui.page_projects.borrow_mut().insert(name.into(), project);
        page.append(&label(
            match name {
                "board" => "Board",
                "mailbox" => "Mailbox",
                "guardrails" => "Guardrails",
                _ => "Notes",
            },
            "title",
        ));
        match name {
            "mailbox" => mail_composer(ui, page, project),
            "board" => task_composer(ui, page, project),
            "notes" => note_composer(ui, page, project),
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
                let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
                row.add_css_class("record");
                row.append(&label(text(&note, "title"), "title"));
                row.append(&paragraph(text(&note, "body")));
                body.append(&row);
            }
        }
        _ => {}
    }
}
fn task_composer(ui: &Rc<Ui>, page: &gtk::Box, project: i64) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Entry::builder()
        .placeholder_text("What should the agent work on?")
        .hexpand(true)
        .build();
    let add = button("Add task", "primary");
    row.append(&title);
    row.append(&add);
    page.append(&row);
    let weak = Rc::downgrade(ui);
    add.connect_clicked(move |b| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let value = title.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        b.set_sensitive(false);
        let b = b.clone();
        let title = title.clone();
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "task.create",
                    json!({"project_id":project,"title":value,"column":"ready"}),
                )
                .await
            {
                Ok(_) => {
                    if title.text().trim() == value {
                        title.set_text("");
                    }
                    ui.refresh_page();
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            b.set_sensitive(true);
        });
    });
}
fn board(ui: &Rc<Ui>, body: &gtk::Box, tasks: &[Value]) {
    let grid = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    grid.set_homogeneous(true);
    grid.set_vexpand(true);
    body.append(&grid);
    for (name, title) in [
        ("backlog", "BACKLOG"),
        ("ready", "READY"),
        ("active", "ACTIVE"),
        ("in_review", "IN REVIEW"),
        ("done", "DONE"),
    ] {
        let lane = gtk::Box::new(gtk::Orientation::Vertical, 8);
        lane.add_css_class("board-lane");
        lane.set_size_request(190, 320);
        lane.append(&label(
            &format!(
                "{title}  {}",
                tasks.iter().filter(|t| text(t, "column") == name).count()
            ),
            "lane-heading",
        ));
        grid.append(&lane);
        for task in tasks.iter().filter(|t| text(t, "column") == name) {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
            row.add_css_class("task-card");
            let meta = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let number = label(&format!("#{}", task["id"]), "mono");
            number.set_hexpand(true);
            meta.append(&number);
            let priority = text(task, "priority");
            if !priority.is_empty() && priority != "medium" {
                meta.append(&label(priority, "task-priority"));
            }
            row.append(&meta);
            let title = paragraph(text(task, "title"));
            title.add_css_class("task-title");
            row.append(&title);
            let detail = gtk::Expander::builder().label("Details").build();
            let details = gtk::Box::new(gtk::Orientation::Vertical, 6);
            details.append(&paragraph(text(task, "body")));
            details.append(&paragraph(text(task, "changelog")));
            if let Some(names) = task["sessions"].as_array() {
                for session in names {
                    if let Some(name) = session.as_str() {
                        let sessions = ui.sessions.borrow();
                        let role = sessions
                            .iter()
                            .find(|s| text(s, "name") == name)
                            .map(|s| text(s, "role"))
                            .unwrap_or("session");
                        details.append(&label(&format!("{name} · {role}"), "dim"));
                    }
                }
            }
            detail.set_child(Some(&details));
            row.append(&detail);
            row.append(&label(text(task, "state"), "dim"));
            let id = task["id"].as_i64().unwrap_or(0);
            if matches!(name, "ready" | "backlog") {
                let key = button("Assign agents", "quiet");
                let weak = Rc::downgrade(ui);
                key.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.show_launch(Some(id));
                    }
                });
                row.append(&key);
            }
            if name == "in_review" {
                let key = button("Approve task", "primary");
                let weak = Rc::downgrade(ui);
                key.connect_clicked(move |b| {
                    if let Some(ui) = weak.upgrade() {
                        ui.mutate("task.approve", json!({"task_id":id}), b);
                    }
                });
                row.append(&key);
            }
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
            lane.append(&row);
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
