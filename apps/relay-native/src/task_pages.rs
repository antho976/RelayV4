use super::*;
use crate::app::scrolled;
use std::cell::Cell;

pub const COLUMNS: &[&str] = &["backlog", "ready", "active", "in_review", "done"];
pub fn choose(values: &[&str], selected: &str) -> gtk::ComboBoxText {
    let control = gtk::ComboBoxText::new();
    for value in values {
        control.append(Some(value), if value.is_empty() { "None" } else { value });
    }
    control.set_active_id(Some(selected));
    control
}
pub fn chosen(control: &gtk::ComboBoxText) -> String {
    control
        .active_id()
        .map(|s| s.to_string())
        .unwrap_or_default()
}
pub fn buffer_text(buffer: &gtk::TextBuffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}
pub fn multiline(value: &str, height: i32) -> gtk::TextView {
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_size_request(-1, height);
    view.buffer().set_text(value);
    view
}

// Dialog-local drafts stay alive while project lists refresh. A close request cannot
// drop a changed draft, and controls are locked while its save is in flight.
pub struct Draft {
    pub window: gtk::Window,
    pub layout: gtk::Box,
    pub form: gtk::Box,
    pub status: gtk::Label,
    pub footer: gtk::Box,
    pub base: Rc<RefCell<Value>>,
    pub busy: Rc<Cell<bool>>,
    pub snapshot: Rc<dyn Fn() -> Value>,
    pub on_close: RefCell<Option<Box<dyn Fn()>>>,
}
impl Draft {
    pub fn new(
        ui: &Rc<Ui>,
        title: &str,
        base: Value,
        snapshot: Rc<dyn Fn() -> Value>,
        form: gtk::Box,
    ) -> Rc<Self> {
        let window = gtk::Window::builder()
            .title(title)
            .transient_for(&ui.window)
            .modal(true)
            .default_width(780)
            .default_height(720)
            .build();
        let layout = gtk::Box::new(gtk::Orientation::Vertical, 10);
        layout.set_margin_top(16);
        layout.set_margin_bottom(16);
        layout.set_margin_start(16);
        layout.set_margin_end(16);
        let status = paragraph("");
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        layout.append(&scrolled(&form));
        layout.append(&status);
        layout.append(&footer);
        window.set_child(Some(&layout));
        let draft = Rc::new(Self {
            window,
            layout,
            form,
            status,
            footer,
            base: Rc::new(RefCell::new(base)),
            busy: Rc::new(Cell::new(false)),
            snapshot,
            on_close: RefCell::new(None),
        });
        let weak = Rc::downgrade(&draft);
        draft.window.connect_close_request(move |_| {
            if let Some(d) = weak.upgrade() {
                if d.busy.get() || d.dirty() {
                    d.status
                        .set_text("Save your changes or choose Discard and close.");
                    return glib::Propagation::Stop;
                }
                if let Some(close) = d.on_close.borrow_mut().take() {
                    close();
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&draft);
        draft.window.connect_hide(move |_| {
            if let Some(d) = weak.upgrade() {
                clear(&d.footer);
                clear(&d.form);
            }
        });
        draft
    }
    pub fn close(&self) {
        if self.busy.get() || self.dirty() {
            self.status
                .set_text("Save your changes or choose Discard and close.");
            return;
        }
        if let Some(close) = self.on_close.borrow_mut().take() {
            close();
        }
        self.window.destroy();
        clear(&self.footer);
        clear(&self.form);
    }
    pub fn dirty(&self) -> bool {
        let current = (self.snapshot)();
        current
            .as_object()
            .is_some_and(|m| m.iter().any(|(k, v)| self.base.borrow()[k] != *v))
    }
    pub fn controls(
        self: &Rc<Self>,
        ui: &Rc<Ui>,
        get: &'static str,
        update: &'static str,
        id_key: &'static str,
        id: i64,
    ) {
        let save = button("Save", "primary");
        save.set_widget_name("draft-save");
        self.footer.append(&save);
        let close = button("Close", "quiet");
        self.footer.append(&close);
        let discard = button("Discard and close", "quiet");
        self.footer.append(&discard);
        let d = self.clone();
        close.connect_clicked(move |_| d.close());
        let d = self.clone();
        discard.connect_clicked(move |_| {
            if !d.busy.get() {
                *d.base.borrow_mut() = (d.snapshot)();
                d.close();
            }
        });
        let d = self.clone();
        let weak = Rc::downgrade(ui);
        save.connect_clicked(move |_| {
            let Some(ui) = weak.upgrade() else { return }; if d.busy.replace(true) { return; }
            let next = (d.snapshot)(); let d = d.clone(); d.form.set_sensitive(false); d.footer.set_sensitive(false); d.status.set_text("Saving…");
            glib::spawn_future_local(async move {
                match ui.call(get, json!({id_key:id})).await {
                    Ok(latest) => {
                        let base = d.base.borrow().clone();
                        let keys: Vec<String> = next.as_object().unwrap().keys().cloned().collect();
                        if draft_conflicts(&base, &latest, &keys) {
                            d.status.set_text("This item changed elsewhere. Your draft is preserved. Copy it before discarding and reopening the latest version.");
                        } else {
                            let expected: serde_json::Map<String,Value> = keys.iter().map(|k|(k.clone(),base[k].clone())).collect();
                            let mut payload = next; payload[id_key] = json!(id); payload["expected"] = Value::Object(expected);
                            match ui.call(update, payload).await { Ok(v) => { *d.base.borrow_mut() = v; d.status.set_text("Saved"); ui.refresh_page(); }, Err(e) => d.status.set_text(&e.to_string()) }
                        }
                    }, Err(e) => d.status.set_text(&e.to_string())
                }
                d.busy.set(false); d.form.set_sensitive(true); d.footer.set_sensitive(true);
            });
        });
        let key = gtk::EventControllerKey::new();
        key.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::s && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                save.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.layout.add_controller(key);
    }
}

pub fn open(ui: &Rc<Ui>, id: i64) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match ui.call("task.get", json!({"task_id":id})).await {
            Ok(task) => {
                let project = task["project_id"].as_i64().unwrap_or(0);
                let modules = ui
                    .call("module.list", json!({"project_id":project}))
                    .await
                    .unwrap_or(Value::Null);
                let tasks = ui
                    .call("task.list", json!({"project_id":project}))
                    .await
                    .unwrap_or(Value::Null);
                detail(&ui, task, rows(&modules, "modules"), rows(&tasks, "tasks"));
            }
            Err(e) => ui.show_error(&e.to_string()),
        }
    });
}
fn detail(ui: &Rc<Ui>, task: Value, modules: Vec<Value>, tasks: Vec<Value>) {
    let id = task["id"].as_i64().unwrap_or(0);
    let project = task["project_id"].as_i64().unwrap_or(0);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Entry::builder().text(text(&task, "title")).build();
    title.set_widget_name("task-title");
    field("Title", &title, &form);
    let meta = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let priority = choose(
        &["low", "medium", "high", "urgent"],
        text(&task, "priority"),
    );
    let kind = choose(
        &["task", "feature", "bug", "chore", "spike"],
        text(&task, "type"),
    );
    let size = choose(&["", "S", "M", "L"], text(&task, "size"));
    let state = choose(
        &[
            "none",
            "dispatched",
            "running",
            "blocked",
            "failed",
            "awaiting_review",
        ],
        text(&task, "state"),
    );
    let module = gtk::ComboBoxText::new();
    module.append(Some(""), "No module");
    for m in &modules {
        module.append(Some(&m["id"].to_string()), text(m, "name"));
    }
    module.set_active_id(Some(
        &task["module_id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default(),
    ));
    for (caption, control) in [
        ("Priority", &priority),
        ("Type", &kind),
        ("Size", &size),
        ("Module", &module),
        ("State", &state),
    ] {
        let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
        field(caption, control, &group);
        meta.append(&group);
    }
    form.append(&meta);
    let body = multiline(text(&task, "body"), 180);
    field("Description · Markdown", &body, &form);
    let changelog = multiline(text(&task, "changelog"), 65);
    field("Changelog sentence", &changelog, &form);
    let snapshot: Rc<dyn Fn() -> Value> = Rc::new(
        move || json!({"title":title.text().trim(),"body":buffer_text(&body.buffer()),"changelog":buffer_text(&changelog.buffer()),"priority":chosen(&priority),"state":chosen(&state),"type":chosen(&kind),"size":if chosen(&size).is_empty(){Value::Null}else{json!(chosen(&size))},"module_id":chosen(&module).parse::<i64>().ok()}),
    );
    let d = Draft::new(ui, &format!("Task #{id}"), task.clone(), snapshot, form);
    d.controls(ui, "task.get", "task.update", "task_id", id);
    let transitions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    transitions.append(&label(
        &format!("{} · {}", text(&task, "column"), text(&task, "state")),
        "dim",
    ));
    let column = choose(COLUMNS, text(&task, "column"));
    transitions.append(&column);
    action(
        ui,
        &d,
        &transitions,
        "Move",
        "task.move",
        move || json!({"task_id":id,"column":chosen(&column)}),
        Some(id),
    );
    if text(&task, "column") == "in_review" {
        action(
            ui,
            &d,
            &transitions,
            "Approve task",
            "task.approve",
            move || json!({"task_id":id}),
            Some(id),
        );
    }
    d.form.append(&transitions);
    let dispatch = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let target = gtk::ComboBoxText::new();
    target.append(Some(""), "Choose an existing session");
    for session in ui
        .sessions
        .borrow()
        .iter()
        .filter(|s| text(s, "state") != "closed" && s["project_id"] == project)
    {
        target.append(
            Some(text(session, "name")),
            &format!("{} · {}", text(session, "name"), text(session, "role")),
        );
    }
    target.set_active(Some(0));
    dispatch.append(&target);
    action(
        ui,
        &d,
        &dispatch,
        "Dispatch",
        "task.dispatch",
        move || json!({"task_id":id,"session":chosen(&target)}),
        Some(id),
    );
    let launch = button("New agents / pair", "quiet");
    dispatch.append(&launch);
    let weak = Rc::downgrade(ui);
    let draft = d.clone();
    launch.connect_clicked(move |_| {
        if draft.dirty() {
            draft
                .status
                .set_text("Save task changes before dispatching.");
            return;
        }
        if let Some(ui) = weak.upgrade() {
            draft.close();
            ui.show_launch(Some(id));
        }
    });
    d.form.append(&dispatch);
    for session in rows(&task, "sessions") {
        if let Some(s) = session.as_str() {
            d.form.append(&label(s, "dim"));
        }
    }
    d.form.append(&label("Labels", "section-label"));
    for tag in rows(&task, "labels") {
        if let Some(name) = tag.as_str() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&label(name, "body"));
            let name = name.to_string();
            action(
                ui,
                &d,
                &row,
                "Remove",
                "task.label.remove",
                move || json!({"task_id":id,"label":name}),
                Some(id),
            );
            d.form.append(&row);
        }
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let tag = gtk::Entry::builder()
        .placeholder_text("New label")
        .hexpand(true)
        .build();
    row.append(&tag);
    action(
        ui,
        &d,
        &row,
        "Add label",
        "task.label.add",
        move || json!({"task_id":id,"label":tag.text().trim()}),
        Some(id),
    );
    d.form.append(&row);
    d.form
        .append(&label("Subtasks & dependencies", "section-label"));
    for child in tasks.iter().filter(|t| t["parent_id"] == id) {
        let key = button(
            &format!(
                "#{}  {}  · {}",
                child["id"],
                text(child, "title"),
                text(child, "column")
            ),
            "quiet",
        );
        let child_id = child["id"].as_i64().unwrap_or(0);
        let weak = Rc::downgrade(ui);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                open(&ui, child_id)
            }
        });
        d.form.append(&key);
    }
    if task["depth"].as_i64().unwrap_or(0) < 2 {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let child = gtk::Entry::builder()
            .placeholder_text("Subtask title")
            .hexpand(true)
            .build();
        row.append(&child);
        action(
            ui,
            &d,
            &row,
            "Add subtask",
            "task.create",
            move || json!({"project_id":project,"parent_id":id,"title":child.text().trim(),"column":"backlog"}),
            Some(id),
        );
        d.form.append(&row);
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let parent = gtk::ComboBoxText::new();
    parent.append(Some(""), "Root task");
    for task in tasks.iter().filter(|t| t["id"] != id) {
        parent.append(
            Some(&task["id"].to_string()),
            &format!("#{} {}", task["id"], text(task, "title")),
        );
    }
    parent.set_active_id(Some(
        &task["parent_id"]
            .as_i64()
            .map(|x| x.to_string())
            .unwrap_or_default(),
    ));
    row.append(&parent);
    action(
        ui,
        &d,
        &row,
        "Set parent",
        "task.parent.set",
        move || json!({"task_id":id,"parent_id":chosen(&parent).parse::<i64>().ok()}),
        Some(id),
    );
    d.form.append(&row);
    for (field, relation) in [
        ("blocked_by", "blocked_by"),
        ("duplicate_of", "duplicate_of"),
    ] {
        let ids = if field == "duplicate_of" {
            task[field].as_i64().into_iter().map(|v| json!(v)).collect()
        } else {
            rows(&task, field)
        };
        for other in ids {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&label(
                &format!("{} #{}", field.replace('_', " "), other),
                "body",
            ));
            action(
                ui,
                &d,
                &row,
                "Remove",
                "task.unrelate",
                move || json!({"task_id":id,"relation":relation,"other_id":other}),
                Some(id),
            );
            d.form.append(&row);
        }
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let relation = choose(&["blocked_by", "duplicate_of"], "blocked_by");
    let other = gtk::ComboBoxText::new();
    for t in tasks.iter().filter(|t| t["id"] != id) {
        other.append(
            Some(&t["id"].to_string()),
            &format!("#{} {}", t["id"], text(t, "title")),
        );
    }
    row.append(&relation);
    row.append(&other);
    action(
        ui,
        &d,
        &row,
        "Link",
        "task.relate",
        move || json!({"task_id":id,"relation":chosen(&relation),"other_id":chosen(&other).parse::<i64>().unwrap_or(0)}),
        Some(id),
    );
    d.form.append(&row);
    d.form
        .append(&label("Commits & attachments", "section-label"));
    for commit in rows(&task, "commits") {
        d.form.append(&paragraph(&format!(
            "{}  {}",
            text(&commit, "sha"),
            text(&commit, "branch")
        )));
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let sha = gtk::Entry::builder()
        .placeholder_text("Commit SHA")
        .hexpand(true)
        .build();
    row.append(&sha);
    action(
        ui,
        &d,
        &row,
        "Link commit",
        "task.link_commit",
        move || json!({"task_id":id,"sha":sha.text().trim()}),
        Some(id),
    );
    d.form.append(&row);
    for attachment in rows(&task, "attachments") {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&paragraph(&format!(
            "{} · {} bytes",
            text(&attachment, "name"),
            attachment["bytes"]
        )));
        let attachment_id = attachment["id"].clone();
        action(
            ui,
            &d,
            &row,
            "Detach",
            "task.detach",
            move || json!({"task_id":id,"attachment_id":attachment_id}),
            Some(id),
        );
        d.form.append(&row);
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let path = gtk::Entry::builder()
        .placeholder_text("Absolute image path")
        .hexpand(true)
        .build();
    row.append(&path);
    action(
        ui,
        &d,
        &row,
        "Attach image",
        "task.attach",
        move || json!({"task_id":id,"path":path.text().trim()}),
        Some(id),
    );
    d.form.append(&row);
    action(
        ui,
        &d,
        &d.footer,
        "Delete task",
        "task.delete",
        move || json!({"task_id":id}),
        None,
    );
    d.window.present();
}
// Auxiliary changes never invalidate an unsaved editor. Successful actions reopen
// a fresh detail, so labels, relations and state always reflect the engine response.
pub fn action(
    ui: &Rc<Ui>,
    d: &Rc<Draft>,
    row: &gtk::Box,
    caption: &str,
    op: &'static str,
    payload: impl Fn() -> Value + 'static,
    reopen: Option<i64>,
) {
    let key = button(caption, "quiet");
    row.append(&key);
    let weak = Rc::downgrade(ui);
    let d = d.clone();
    key.connect_clicked(move |_| {
        let Some(ui) = weak.upgrade() else { return };
        if d.busy.get() {
            return;
        }
        if d.dirty() {
            d.status.set_text("Save changes before this action.");
            return;
        }
        d.busy.set(true);
        d.form.set_sensitive(false);
        d.footer.set_sensitive(false);
        let payload = payload();
        let d = d.clone();
        glib::spawn_future_local(async move {
            match ui.call(op, payload).await {
                Ok(_) => {
                    d.busy.set(false);
                    d.close();
                    ui.refresh_page();
                    if let Some(id) = reopen {
                        open(&ui, id)
                    }
                }
                Err(e) => {
                    d.status.set_text(&e.to_string());
                    d.busy.set(false);
                    d.form.set_sensitive(true);
                    d.footer.set_sensitive(true);
                }
            }
        });
    });
}

fn draft_conflicts(base: &Value, latest: &Value, fields: &[String]) -> bool {
    latest["deleted_at"].is_string() || fields.iter().any(|key| latest[key] != base[key])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edited_fields_and_deleted_records_conflict_but_unrelated_activity_does_not() {
        let base = json!({"title":"Original","body":"Text","column":"ready","deleted_at":null});
        let fields = vec!["title".into(), "body".into()];
        let mut latest = base.clone();
        latest["column"] = json!("active");
        assert!(!draft_conflicts(&base, &latest, &fields));
        latest["body"] = json!("Agent edit");
        assert!(draft_conflicts(&base, &latest, &fields));
        latest = base.clone();
        latest["deleted_at"] = json!("2026-09-04");
        assert!(draft_conflicts(&base, &latest, &fields));
    }
}
