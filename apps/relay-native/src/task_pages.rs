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

// Editor-local drafts stay alive while project lists refresh. A close request cannot
// drop a changed draft, and controls are locked while its save is in flight.
pub struct Draft {
    pub window: Option<gtk::Window>,
    panel: Option<Rc<crate::panel::Panel>>,
    pub layout: gtk::Box,
    pub form: gtk::Box,
    pub status: gtk::Label,
    pub footer: gtk::Box,
    pub base: Rc<RefCell<Value>>,
    pub busy: Rc<Cell<bool>>,
    unsent_message: Cell<bool>,
    pub snapshot: Rc<dyn Fn() -> Value>,
    /// The form as it was opened, then as last saved: Save sends only what differs from it.
    initial: RefCell<Value>,
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
        Self::build(ui, title, base, snapshot, form, false)
    }
    pub fn new_note(
        ui: &Rc<Ui>,
        title: &str,
        base: Value,
        snapshot: Rc<dyn Fn() -> Value>,
        form: gtk::Box,
    ) -> Rc<Self> {
        Self::build(ui, title, base, snapshot, form, true)
    }
    fn build(
        ui: &Rc<Ui>,
        title: &str,
        base: Value,
        snapshot: Rc<dyn Fn() -> Value>,
        form: gtk::Box,
        note: bool,
    ) -> Rc<Self> {
        let window: Option<gtk::Window> = None;
        let panel = (!note).then(|| crate::panel::Panel::page(ui, title));
        form.set_valign(gtk::Align::Start);
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
        if let Some(window) = &window {
            window.set_child(Some(&layout));
        }
        if let Some(panel) = &panel {
            panel.body.append(&layout);
        }
        let draft = Rc::new(Self {
            window,
            panel,
            layout,
            form,
            status,
            footer,
            base: Rc::new(RefCell::new(base)),
            busy: Rc::new(Cell::new(false)),
            unsent_message: Cell::new(false),
            initial: RefCell::new(snapshot()),
            snapshot,
            on_close: RefCell::new(None),
        });
        if let Some(window) = &draft.window {
            let weak = Rc::downgrade(&draft);
            window.connect_close_request(move |_| {
                if let Some(d) = weak.upgrade() {
                    if !d.can_close() {
                        return glib::Propagation::Stop;
                    }
                    d.cleanup();
                }
                glib::Propagation::Proceed
            });
        }
        if let Some(panel) = &draft.panel {
            let weak = Rc::downgrade(&draft);
            panel.set_guard(move || weak.upgrade().is_none_or(|d| d.can_close()));
            let weak = Rc::downgrade(&draft);
            panel.on_closed(move || {
                if let Some(d) = weak.upgrade() {
                    d.cleanup();
                }
            });
        }
        draft
    }
    fn can_close(&self) -> bool {
        if self.unsent_message.get() {
            self.status
                .set_text("Send or clear your message before closing.");
            return false;
        }
        if self.busy.get() || self.dirty() {
            self.status
                .set_text("Save your changes or choose Discard and close.");
            false
        } else {
            true
        }
    }
    fn cleanup(&self) {
        if let Some(close) = self.on_close.borrow_mut().take() {
            close();
        }
        clear(&self.footer);
        clear(&self.form);
    }
    pub fn present(&self) {
        if let Some(panel) = &self.panel {
            panel.present();
        }
        if let Some(window) = &self.window {
            window.present();
        }
    }
    pub fn close(&self) {
        if !self.can_close() {
            return;
        }
        if let Some(panel) = &self.panel {
            panel.close();
        }
        if let Some(window) = &self.window {
            self.cleanup();
            window.destroy();
        } else if self.panel.is_none() {
            self.cleanup();
        }
    }
    /// Drops the edits and an unsent message together, so the close that follows cannot be
    /// refused and the base is never moved under a draft that stays open.
    fn discard(&self) {
        if self.busy.get() {
            return;
        }
        *self.base.borrow_mut() = (self.snapshot)();
        self.unsent_message.set(false);
        self.close();
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
        let pending = self.clone();
        crate::app::confirm_inline_if(&discard, "Confirm discard", move || pending.dirty() || pending.unsent_message.get(), move |_| d.discard());
        let d = self.clone();
        let weak = Rc::downgrade(ui);
        save.connect_clicked(move |_| {
            let Some(ui) = weak.upgrade() else { return }; if d.busy.get() { return; }
            // Send and check only the edited fields. The engine moves a task's state, and an
            // agent writes its changelog, while the form is open; neither conflicts with an edit
            // elsewhere, and the engine's own check of `expected` is the atomic one.
            let next = (d.snapshot)();
            let initial = d.initial.borrow().clone();
            let keys: Vec<String> = next.as_object().map(|m| m.iter().filter(|(k, v)| initial[k.as_str()] != **v).map(|(k, _)| k.clone()).collect()).unwrap_or_default();
            if keys.is_empty() { d.status.set_text("Nothing to save"); return; }
            d.busy.set(true); let d = d.clone(); d.form.set_sensitive(false); d.footer.set_sensitive(false); d.status.set_text("Saving…");
            glib::spawn_future_local(async move {
                let base = d.base.borrow().clone();
                let mut payload = json!({id_key: id});
                let mut expected = serde_json::Map::new();
                for k in &keys { payload[k.as_str()] = next[k.as_str()].clone(); expected.insert(k.clone(), base[k.as_str()].clone()); }
                payload["expected"] = Value::Object(expected);
                match ui.call(update, payload).await {
                    Ok(v) => { *d.base.borrow_mut() = v; *d.initial.borrow_mut() = next; d.status.set_text("Saved"); ui.refresh_page(); }
                    Err(crate::client::Error::Bus(e)) if e.code.ends_with(".edit_conflict") => d.status.set_text("This item changed elsewhere. Your draft is preserved. Copy it before discarding and reopening the latest version."),
                    Err(e) => d.status.set_text(&e.to_string()),
                }
                d.busy.set(false); d.form.set_sensitive(true); d.footer.set_sensitive(true);
            });
        });
        // Weak: the controller lives on the layout, and a strong Save here would keep the
        // Draft, its panel and its widgets alive after the panel closes.
        let save = save.downgrade();
        let key = gtk::EventControllerKey::new();
        key.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::s && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                if let Some(save) = save.upgrade() {
                    save.emit_clicked();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.layout.add_controller(key);
    }
}

fn classification_choices(
    control: &gtk::ComboBoxText,
    kind: &'static str,
    choices: &[&str],
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    row.set_homogeneous(true);
    row.add_css_class("task-choice-row");
    let mut first = None::<gtk::ToggleButton>;
    for (index, value) in choices.iter().enumerate() {
        let key = gtk::ToggleButton::new();
        key.add_css_class("task-choice");
        if let Some(first) = &first {
            key.set_group(Some(first));
        } else {
            first = Some(key.clone());
        }
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        if kind == "type" {
            content.append(&super::task_mark(value, 10));
        } else {
            let mark = gtk::DrawingArea::new();
            mark.set_content_width(if kind == "size" { 25 } else { 13 });
            mark.set_content_height(12);
            mark.set_halign(gtk::Align::Center);
            mark.set_draw_func(move |widget, cr, _, _| {
                let name = if kind == "priority" && index == 3 {
                    "held"
                } else if kind == "priority" && index == 2 {
                    "waiting"
                } else {
                    "secondary"
                };
                let Some(color) = widget.style_context().lookup_color(name) else {
                    return;
                };
                for i in 0..3 {
                    let lit = if kind == "size" {
                        i < index
                    } else {
                        index >= 2 || (index == 1 && i < 2)
                    };
                    cr.set_source_rgba(
                        color.red() as f64,
                        color.green() as f64,
                        color.blue() as f64,
                        if lit {
                            1.
                        } else if kind == "size" {
                            0.2
                        } else {
                            0.28
                        },
                    );
                    if kind == "size" {
                        let size = [4., 7., 10.][i];
                        let left = [0., 6., 15.][i];
                        cr.rectangle(left + 0.5, 12. - size + 0.5, size - 1., size - 1.);
                        cr.set_line_width(1.);
                        if lit {
                            let _ = cr.fill();
                        } else {
                            let _ = cr.stroke();
                        }
                    } else {
                        let height = if index >= 2 { [3., 7., 11.][i] } else { 3. };
                        cr.rectangle(i as f64 * 5., 11. - height, 3., height);
                        let _ = cr.fill();
                    }
                }
            });
            content.append(&mark);
        }
        content.append(&label(
            if value.is_empty() { "None" } else { value },
            "task-choice-label",
        ));
        key.set_child(Some(&content));
        key.set_active(chosen(control) == *value);
        let control = control.downgrade();
        let value = value.to_string();
        key.connect_toggled(move |key| {
            if key.is_active() {
                if let Some(control) = control.upgrade() {
                    control.set_active_id(Some(&value));
                }
            }
        });
        row.append(&key);
    }
    row
}

pub fn compose(ui: &Rc<Ui>, project: i64) {
    let panel = crate::panel::Panel::page(ui, "New task");
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    form.add_css_class("task-compose");
    let primary = gtk::Box::new(gtk::Orientation::Vertical, 12);
    primary.add_css_class("task-compose-section");
    primary.append(&label("TASK", "section-label"));
    primary.append(&paragraph("The outcome and context the agent receives."));
    let title = gtk::Entry::builder()
        .placeholder_text("A concrete outcome")
        .build();
    field("Title", &title, &primary);
    let description = multiline("", 160);
    field("Description · Markdown", &description, &primary);
    form.append(&primary);
    let classify = gtk::Box::new(gtk::Orientation::Vertical, 12);
    classify.add_css_class("task-compose-section");
    let kind = choose(&["task", "feature", "bug", "chore", "spike"], "task");
    let priority = choose(&["low", "medium", "high", "urgent"], "medium");
    let size = choose(&["", "S", "M", "L"], "");
    classify.append(&label("CLASSIFY", "section-label"));
    classify.append(&paragraph(
        "Type is the first-class classification; labels are the free-form tags under it.",
    ));
    field(
        "Type",
        &classification_choices(&kind, "type", &["task", "feature", "bug", "chore", "spike"]),
        &classify,
    );
    let labels = gtk::Entry::builder()
        .placeholder_text("Add a label, separated by commas")
        .build();
    field("Labels · optional", &labels, &classify);
    let parent = choose(&[""], "");
    field("Parent · makes this a sub-task", &parent, &classify);
    form.append(&classify);
    let organize = gtk::Box::new(gtk::Orientation::Vertical, 12);
    organize.add_css_class("task-compose-section");
    organize.append(&label("ORGANIZE", "section-label"));
    organize.append(&paragraph(
        "Only priority is required. Everything else can be filled later.",
    ));
    let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    metadata.set_homogeneous(true);
    let priority_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    field(
        "Priority",
        &classification_choices(&priority, "priority", &["low", "medium", "high", "urgent"]),
        &priority_box,
    );
    metadata.append(&priority_box);
    let size_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    field(
        "Size",
        &classification_choices(&size, "size", &["", "S", "M", "L"]),
        &size_box,
    );
    metadata.append(&size_box);
    let module = choose(&[""], "");
    let module_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    field("Module", &module, &module_box);
    metadata.append(&module_box);
    organize.append(&metadata);
    let changelog = gtk::Entry::builder().placeholder_text("Added…").build();
    field("Changelog sentence · optional", &changelog, &organize);
    form.append(&organize);
    let advanced = gtk::Expander::new(Some("Advanced · column and execution state"));
    let advanced_fields = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let column = choose(COLUMNS, "backlog");
    let state = choose(
        &[
            "none",
            "dispatched",
            "running",
            "blocked",
            "failed",
            "awaiting_review",
        ],
        "none",
    );
    field("Column", &column, &advanced_fields);
    field("State", &state, &advanced_fields);
    advanced.set_child(Some(&advanced_fields));
    form.append(&advanced);
    for control in [&kind, &priority, &size] {
        control.set_visible(false);
        form.append(control);
    }
    let weak = Rc::downgrade(ui);
    let parents = parent.clone();
    let modules = module.clone();
    glib::spawn_future_local(async move {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let (tasks, available) = tokio::join!(
            ui.call("task.list", json!({"project_id":project})),
            ui.call("module.list", json!({"project_id":project}))
        );
        if let Ok(tasks) = tasks {
            for task in rows(&tasks, "tasks")
                .iter()
                .filter(|task| task["depth"].as_i64().unwrap_or(0) < 2)
            {
                parents.append(
                    Some(&task["id"].to_string()),
                    &format!("#{} {}", task["id"], text(task, "title")),
                );
            }
        }
        if let Ok(available) = available {
            for module in rows(&available, "modules") {
                modules.append(Some(&module["id"].to_string()), text(&module, "name"));
            }
        }
    });
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let status = paragraph("");
    status.set_hexpand(true);
    footer.append(&status);
    let cancel = button("Cancel", "quiet");
    footer.append(&cancel);
    let create = button("Create task", "primary");
    footer.append(&create);
    form.append(&footer);
    panel.body.append(&form);
    let permit_close = Rc::new(Cell::new(false));
    let busy = Rc::new(Cell::new(false));
    let guard_title = title.clone();
    let guard_body = description.buffer();
    let guard_status = status.clone();
    let permit = permit_close.clone();
    let working = busy.clone();
    panel.set_guard(move || {
        if working.get() {
            return false;
        }
        if permit.get() || (guard_title.text().is_empty() && buffer_text(&guard_body).is_empty()) {
            return true;
        }
        guard_status.set_text("Create the task or choose Discard draft before leaving.");
        false
    });
    cancel.set_label("Discard draft");
    // Weak captures: both keys sit inside the form the panel owns, so strong ones would keep
    // the form and the panel alive after it closes.
    let p = Rc::downgrade(&panel);
    let permit = permit_close.clone();
    let working = busy.clone();
    cancel.connect_clicked(move |_| {
        if !working.get() {
            permit.set(true);
            if let Some(p) = p.upgrade() {
                p.close();
            }
        }
    });
    let weak = Rc::downgrade(ui);
    let p = Rc::downgrade(&panel);
    let form = form.downgrade();
    create.connect_clicked(move |_| {
        let Some(ui) = weak.upgrade() else { return; };
        let name = title.text().trim().to_string();
        if name.is_empty() { title.grab_focus(); return; }
        if busy.replace(true) { return; }
        let payload = json!({"project_id": project, "title": name, "body": buffer_text(&description.buffer()), "type": chosen(&kind), "priority": chosen(&priority), "size": if chosen(&size).is_empty() {Value::Null} else {json!(chosen(&size))}, "column":chosen(&column),"state":chosen(&state),"parent_id":chosen(&parent).parse::<i64>().ok(),"module_id":chosen(&module).parse::<i64>().ok(),"changelog":changelog.text().to_string(),"labels":labels.text().split(',').map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>()});
        let Some(form) = form.upgrade() else { return; };
        form.set_sensitive(false);
        let status = status.clone(); let p = p.clone();
        let busy = busy.clone(); let permit = permit_close.clone();
        glib::spawn_future_local(async move {
            let result = ui.call("task.create", payload).await;
            busy.set(false);
            match result {
                Ok(_) => { permit.set(true); if let Some(p) = p.upgrade() { p.close(); } ui.refresh_page(); },
                Err(e) => { status.set_text(&e.to_string()); form.set_sensitive(true); }
            }
        });
    });
    panel.present();
}

thread_local! {
    /// Bumped by every open: only the newest open still in flight may present its detail.
    static OPEN_SERIAL: Cell<u64> = const { Cell::new(0) };
    /// Task details on screen, so opening one again shows it instead of stacking a second.
    static OPEN_DETAILS: RefCell<Vec<(i64, std::rc::Weak<Draft>)>> = const { RefCell::new(Vec::new()) };
}
fn open_detail(id: i64) -> Option<Rc<Draft>> {
    OPEN_DETAILS.with(|open| {
        open.borrow_mut().retain(|(_, d)| d.strong_count() > 0);
        open.borrow().iter().find(|(task, _)| *task == id).and_then(|(_, d)| d.upgrade())
    })
}
pub fn open(ui: &Rc<Ui>, id: i64) {
    if let Some(d) = open_detail(id) {
        return d.present();
    }
    let serial = OPEN_SERIAL.with(|s| { s.set(s.get() + 1); s.get() });
    let (project, page) = (ui.project.get(), ui.page.borrow().clone());
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        // The task almost always belongs to the project on screen, so its lists are asked for
        // alongside it; only a task from another project waits for a second pair.
        // Completed modules too: a task keeps its module after the module is completed,
        // and a combo that cannot show it reads as an edit and Save would unassign it.
        let (task, mut modules, mut tasks) = tokio::join!(
            ui.call("task.get", json!({"task_id":id})),
            ui.call("module.list", json!({"project_id":project,"include_archived":true})),
            ui.call("task.list", json!({"project_id":project}))
        );
        if let Ok(task) = &task {
            let owner = task["project_id"].as_i64().unwrap_or(0);
            if owner != project {
                (modules, tasks) = tokio::join!(
                    ui.call("module.list", json!({"project_id":owner,"include_archived":true})),
                    ui.call("task.list", json!({"project_id":owner}))
                );
            }
        }
        // A later open, or a move to another page or project, makes this one stale: presenting
        // it now would cover (and disable) whatever the user went on to.
        let current = OPEN_SERIAL.with(|s| s.get()) == serial;
        if !current || ui.project.get() != project || *ui.page.borrow() != page {
            return;
        }
        if let Some(d) = open_detail(id) {
            return d.present();
        }
        let task = match task {
            Ok(task) => task,
            Err(e) => return ui.show_error(&e.to_string()),
        };
        let modules = match modules {
            Ok(modules) => modules,
            Err(e) => return ui.show_error(&format!("Could not load modules: {e}")),
        };
        detail(&ui, task, rows(&modules, "modules"), tasks.map(|tasks| rows(&tasks, "tasks")).map_err(|e| e.to_string()));
    });
}
fn detail(ui: &Rc<Ui>, task: Value, modules: Vec<Value>, tasks: Result<Vec<Value>, String>) {
    // Without the project's tasks the parent and link choices would be empty, and an empty
    // parent choice reads as "Root task": say so, and keep those rows from acting.
    let (tasks, tasks_error) = match tasks {
        Ok(tasks) => (tasks, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let id = task["id"].as_i64().unwrap_or(0);
    let project = task["project_id"].as_i64().unwrap_or(0);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.append(&super::board_view::identity_strip(&task));
    let title = gtk::Entry::builder().text(text(&task, "title")).build();
    title.set_widget_name("task-title");
    field("Title", &title, &form);
    let meta = gtk::FlowBox::new();
    meta.set_selection_mode(gtk::SelectionMode::None);
    meta.set_min_children_per_line(1);
    meta.set_max_children_per_line(5);
    meta.set_column_spacing(8);
    meta.set_row_spacing(6);
    meta.add_css_class("task-metadata");
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
    let current_module = task["module_id"].as_i64();
    for m in &modules {
        let completed = !m["completed_at"].is_null();
        if completed && m["id"].as_i64() != current_module {
            continue;
        }
        let name = if completed { format!("{} (completed)", text(m, "name")) } else { text(m, "name").to_string() };
        module.append(Some(&m["id"].to_string()), &name);
    }
    if let Some(id) = current_module.filter(|id| !modules.iter().any(|m| m["id"].as_i64() == Some(*id))) {
        module.append(Some(&id.to_string()), &format!("Module #{id}"));
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
        meta.insert(&group, -1);
    }
    form.append(&meta);
    let body = multiline(text(&task, "body"), 120);
    body.set_vexpand(false);
    field("Description · Markdown", &body, &form);
    let changelog = multiline(text(&task, "changelog"), 48);
    changelog.set_vexpand(false);
    field("Changelog sentence", &changelog, &form);
    let snapshot: Rc<dyn Fn() -> Value> = Rc::new(
        move || json!({"title":title.text().trim(),"body":buffer_text(&body.buffer()),"changelog":buffer_text(&changelog.buffer()),"priority":chosen(&priority),"state":chosen(&state),"type":chosen(&kind),"size":if chosen(&size).is_empty(){Value::Null}else{json!(chosen(&size))},"module_id":chosen(&module).parse::<i64>().ok()}),
    );
    let d = Draft::new(ui, &format!("Task #{id}"), task.clone(), snapshot, form);
    d.controls(ui, "task.update", "task_id", id);
    OPEN_DETAILS.with(|open| open.borrow_mut().push((id, Rc::downgrade(&d))));
    if let Some(panel) = &d.panel {
        let weak = Rc::downgrade(&d);
        panel.on_closed(move || OPEN_DETAILS.with(|open| open.borrow_mut().retain(|(_, d)| !d.ptr_eq(&weak))));
    }
    let transitions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    transitions.append(&label(
        &format!("{} · {}", text(&task, "column"), text(&task, "state")),
        "dim",
    ));
    // Done is reached by approval (the engine refuses a plain move there), so Move offers
    // the other columns and Approve stands beside it until the task is done.
    let done = text(&task, "column") == "done";
    let column = choose(&COLUMNS[..4], if done { "in_review" } else { text(&task, "column") });
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
    if !done {
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
        if draft.dirty() || draft.unsent_message.get() {
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
    // The task names its own children; the project list only adds their titles.
    for child_id in rows(&task, "children").iter().filter_map(Value::as_i64) {
        let key = button(
            &match tasks.iter().find(|t| t["id"] == child_id) {
                Some(child) => format!("#{child_id}  {}  · {}", text(child, "title"), text(child, "column")),
                None => format!("#{child_id}"),
            },
            "quiet",
        );
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
    if let Some(pid) = task["parent_id"].as_i64().filter(|pid| !tasks.iter().any(|t| t["id"] == *pid)) {
        parent.append(Some(&pid.to_string()), &format!("#{pid}"));
    }
    row.set_sensitive(tasks_error.is_none());
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
    other.append(Some(""), "Choose a task");
    for t in tasks.iter().filter(|t| t["id"] != id) {
        other.append(
            Some(&t["id"].to_string()),
            &format!("#{} {}", t["id"], text(t, "title")),
        );
    }
    other.set_active(Some(0));
    row.append(&relation);
    row.append(&other);
    let choice = other.clone();
    action(
        ui,
        &d,
        &row,
        "Link",
        "task.relate",
        move || json!({"task_id":id,"relation":chosen(&relation),"other_id":chosen(&choice).parse::<i64>().ok()}),
        Some(id),
    );
    // Link stays off until a task is chosen. Weak: the key's handler already holds the combo.
    if let Some(link) = row.last_child() {
        link.set_sensitive(false);
        let link = link.downgrade();
        other.connect_changed(move |other| {
            if let Some(link) = link.upgrade() {
                link.set_sensitive(!chosen(other).is_empty());
            }
        });
    }
    row.set_sensitive(tasks_error.is_none());
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
    // Relay-2 keeps editing on the left and relationships/actions on the right.
    let edit = gtk::Box::new(gtk::Orientation::Vertical, 8);
    edit.set_hexpand(true);
    edit.set_valign(gtk::Align::Start);
    edit.add_css_class("task-edit");
    let side = gtk::Box::new(gtk::Orientation::Vertical, 8);
    side.set_valign(gtk::Align::Start);
    side.add_css_class("task-relations");
    side.set_size_request(340, -1);
    let mut relations = false;
    let mut group: Option<gtk::Box> = None;
    while let Some(child) = d.form.first_child() {
        relations |= child == transitions.clone().upcast::<gtk::Widget>();
        d.form.remove(&child);
        if relations {
            if child.has_css_class("section-label") {
                let title = child
                    .clone()
                    .downcast::<gtk::Label>()
                    .map(|label| label.text().to_string())
                    .unwrap_or_default();
                let disclosure = gtk::Expander::new(Some(&title));
                disclosure.set_expanded(title == "Subtasks & dependencies");
                let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
                disclosure.set_child(Some(&content));
                side.append(&disclosure);
                group = Some(content);
            } else if let Some(group) = &group {
                group.append(&child);
            } else {
                side.append(&child);
            }
        } else {
            edit.append(&child);
        }
    }
    d.layout.remove(&d.status);
    d.layout.remove(&d.footer);
    edit.append(&d.footer);
    let overview = gtk::Box::new(
        if ui.window.width() >= 1000 {
            gtk::Orientation::Horizontal
        } else {
            gtk::Orientation::Vertical
        },
        12,
    );
    overview.append(&edit);
    overview.append(&side);
    let sections = gtk::Stack::new();
    sections.set_vexpand(true);
    sections.set_hhomogeneous(false);
    sections.set_vhomogeneous(false);
    sections.add_titled(&overview, Some("info"), "Info & edit");
    let messages = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let history = gtk::Box::new(gtk::Orientation::Vertical, 8);
    sections.add_titled(&messages, Some("messages"), "Messages");
    sections.add_titled(&history, Some("history"), "History");
    let tabs = gtk::StackSwitcher::new();
    tabs.set_stack(Some(&sections));
    tabs.add_css_class("task-detail-tabs");
    d.form.set_spacing(8);
    d.form.set_orientation(gtk::Orientation::Vertical);
    d.form.append(&d.status);
    d.form.append(&tabs);
    d.form.append(&sections);
    // Messages and history load when one of their tabs is first shown, not with every open.
    let refresh = task_activity(ui, &d, id, project, &messages, &history).downgrade();
    let loaded = Cell::new(false);
    sections.connect_visible_child_name_notify(move |sections| {
        if sections.visible_child_name().is_some_and(|name| name != "info") && !loaded.replace(true) {
            if let Some(refresh) = refresh.upgrade() {
                refresh.emit_clicked();
            }
        }
    });
    if let Some(scroll) = d.layout.first_child().and_downcast::<gtk::ScrolledWindow>() {
        scroll.set_child(gtk::Widget::NONE);
        d.layout.remove(&scroll);
        d.layout.append(&d.form);
    }
    if let (Some(surface), Some(panel)) = (ui.window.surface(), &d.panel) {
        let weak = overview.downgrade();
        let handler = surface.connect_layout(move |_, width, _| {
            if let Some(form) = weak.upgrade() {
                form.set_orientation(if width >= 1000 {
                    gtk::Orientation::Horizontal
                } else {
                    gtk::Orientation::Vertical
                });
            }
        });
        let handler = RefCell::new(Some(handler));
        panel.on_closed(move || {
            if let Some(handler) = handler.borrow_mut().take() {
                surface.disconnect(handler);
            }
        });
    }
    if let Some(e) = tasks_error {
        d.status.set_text(&format!("Could not load the project's tasks: {e}. Subtask titles, Set parent and Link are unavailable."));
    }
    d.present();
}
// Auxiliary changes never invalidate an unsaved editor. Successful actions reopen
// a fresh detail, so labels, relations and state always reflect the engine response.
fn task_activity(
    ui: &Rc<Ui>,
    draft: &Rc<Draft>,
    id: i64,
    project: i64,
    messages: &gtk::Box,
    history: &gtk::Box,
) -> gtk::Button {
    let compose = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let recipient = gtk::ComboBoxText::new();
    recipient.append(Some(""), "Choose an agent");
    for session in ui
        .sessions
        .borrow()
        .iter()
        .filter(|s| s["project_id"] == project && text(s, "state") != "closed")
    {
        recipient.append(Some(text(session, "name")), text(session, "name"));
    }
    recipient.set_active(Some(0));
    let text_input = multiline("", 64);
    text_input.set_widget_name("task-message");
    text_input.set_hexpand(true);
    let weak_draft = Rc::downgrade(draft);
    text_input.buffer().connect_changed(move |buffer| {
        if let Some(draft) = weak_draft.upgrade() {
            draft
                .unsent_message
                .set(!buffer_text(buffer).trim().is_empty());
        }
    });
    let send = button("Send message", "primary");
    send.set_valign(gtk::Align::End);
    compose.append(&recipient);
    compose.append(&text_input);
    compose.append(&send);
    messages.append(&compose);
    let notice = label("Messages are explicitly linked to this task.", "dim");
    messages.append(&notice);
    let message_rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let history_rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
    messages.append(&message_rows);
    history.append(&history_rows);
    let refresh = button("Refresh", "quiet");
    messages.append(&refresh);
    let more = button("Load older activity", "quiet");
    history.append(&more);
    let message_more = button("Load older messages", "quiet");
    messages.append(&message_more);
    // Shown once the first load says there is more.
    more.set_visible(false);
    message_more.set_visible(false);
    let cursors = Rc::new(RefCell::new((None::<i64>, None::<i64>)));
    let loading = Rc::new(Cell::new(false));
    // A refresh asked for while a page loads (a message just sent) runs when that load ends.
    let pending = Rc::new(Cell::new(false));
    // Each "older" key pages its own stream: a cursor of 0 asks the other stream for nothing.
    for (button, older_history, older_messages) in [(&refresh, true, true), (&more, true, false), (&message_more, false, true)] {
        let reset = older_history && older_messages;
        let weak = Rc::downgrade(ui);
        let refresh = refresh.downgrade();
        let pending = pending.clone();
        let messages = message_rows.downgrade();
        let history = history_rows.downgrade();
        let notice = notice.downgrade();
        let more = more.downgrade();
        let message_more = message_more.downgrade();
        let cursors = cursors.clone();
        let loading = loading.clone();
        button.connect_clicked(move |_| {
            let (Some(ui),Some(messages),Some(history),Some(notice),Some(more),Some(message_more)) =
                (weak.upgrade(),messages.upgrade(),history.upgrade(),notice.upgrade(),more.upgrade(),message_more.upgrade()) else { return; };
            if loading.replace(true) { if reset { pending.set(true); } return; }
            let (audit, message) = match (reset, older_history) { (true, _) => (None,None), (false, true) => (cursors.borrow().0, Some(0)), (false, false) => (Some(0), cursors.borrow().1) };
            let cursors = cursors.clone();
            let loading = loading.clone();
            let (refresh, pending) = (refresh.clone(), pending.clone());
            glib::spawn_future_local(async move {
                match ui.call("task.activity",json!({"task_id":id,"before_audit":audit,"before_message":message,"limit":100})).await {
                    Ok(data) => {
                        if reset { clear(&messages); clear(&history); }
                        for message in rows(&data,"messages").into_iter().filter(|_| older_messages) {
                            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
                            row.add_css_class("task-activity-row");
                            row.append(&label(&format!("{} → {} · {}",text(&message,"from"),text(&message,"to"),text(&message,"sent_at")),"dim"));
                            row.append(&paragraph(text(&message,"text")));
                            messages.append(&row);
                        }
                        for event in rows(&data,"history").into_iter().filter(|_| older_history) {
                            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
                            row.add_css_class("task-activity-row");
                            let old = event["undo_op"]["payload"]["column"].as_str();
                            let new = event["payload"]["column"].as_str().or(event["result_summary"]["column"].as_str()).or(event["result_summary"]["task"]["column"].as_str());
                            let change = match (old,new) { (Some(old),Some(new)) if old != new => format!("{old} → {new}"), _ => text(&event,"op").to_string() };
                            row.append(&label(&change,"title"));
                            row.append(&label(&format!("{} · {} · {}",text(&event,"ts"),text(&event,"actor"),text(&event,"kind")),"dim"));
                            history.append(&row);
                        }
                        if reset && messages.first_child().is_none() { messages.append(&label("No messages linked to this task yet.","dim")); }
                        if reset && history.first_child().is_none() { history.append(&label("No recorded activity.","dim")); }
                        let next_audit = data["next_audit"].as_i64();
                        let next_message = data["next_message"].as_i64();
                        let mut cursors = cursors.borrow_mut();
                        if older_history { cursors.0 = Some(next_audit.unwrap_or(0)); more.set_visible(next_audit.is_some()); }
                        if older_messages { cursors.1 = Some(next_message.unwrap_or(0)); message_more.set_visible(next_message.is_some()); }
                        notice.set_text("Messages are explicitly linked to this task.");
                    }
                    Err(error) => notice.set_text(&error.to_string()),
                }
                loading.set(false);
                if pending.replace(false) {
                    if let Some(refresh) = refresh.upgrade() { refresh.emit_clicked(); }
                }
            });
        });
    }
    let weak = Rc::downgrade(ui);
    let refresh_key = refresh.downgrade();
    send.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let to = chosen(&recipient);
        let body = buffer_text(&text_input.buffer());
        if to.is_empty() || body.trim().is_empty() {
            notice.set_text("Choose an agent and enter a message.");
            return;
        }
        key.set_sensitive(false);
        recipient.set_sensitive(false);
        text_input.set_sensitive(false);
        let (key, recipient, text_input, notice, refresh) = (
            key.clone(),
            recipient.clone(),
            text_input.clone(),
            notice.clone(),
            refresh_key.clone(),
        );
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "mailbox.send",
                    json!({"project_id":project,"to":to,"text":body,"re_task":id}),
                )
                .await
            {
                Ok(_) => {
                    text_input.buffer().set_text("");
                    if let Some(refresh) = refresh.upgrade() {
                        refresh.emit_clicked();
                    }
                }
                Err(error) => notice.set_text(&error.to_string()),
            }
            key.set_sensitive(true);
            recipient.set_sensitive(true);
            text_input.set_sensitive(true);
        });
    });
    refresh
}

pub fn action(
    ui: &Rc<Ui>,
    d: &Rc<Draft>,
    row: &gtk::Box,
    caption: &str,
    op: &'static str,
    payload: impl Fn() -> Value + 'static,
    reopen: Option<i64>,
) {
    let reopen = reopen.map(|id| Rc::new(move |ui: &Rc<Ui>| open(ui, id)) as Reopen);
    action_then(ui, d, row, caption, op, payload, reopen);
}

pub type Reopen = Rc<dyn Fn(&Rc<Ui>)>;
/// `action`, reopening any editor afterwards: a module editor's "Add task" passes its own
/// opener so the editor comes back with the new task listed.
pub fn action_then(
    ui: &Rc<Ui>,
    d: &Rc<Draft>,
    row: &gtk::Box,
    caption: &str,
    op: &'static str,
    payload: impl Fn() -> Value + 'static,
    reopen: Option<Reopen>,
) {
    let key = button(caption, "quiet");
    row.append(&key);
    let weak = Rc::downgrade(ui);
    let d = d.clone();
    // A delete confirms on the key itself, and only when it would actually go ahead.
    let confirm = op.ends_with(".delete").then(|| d.clone());
    let act = move |_: &gtk::Button| {
        let Some(ui) = weak.upgrade() else { return };
        if d.busy.get() {
            return;
        }
        if d.dirty() || d.unsent_message.get() {
            d.status
                .set_text("Save task changes and send or clear your message before this action.");
            return;
        }
        d.busy.set(true);
        d.form.set_sensitive(false);
        d.footer.set_sensitive(false);
        let payload = payload();
        let d = d.clone();
        let reopen = reopen.clone();
        glib::spawn_future_local(async move {
            match ui.call(op, payload).await {
                Ok(_) => {
                    d.busy.set(false);
                    d.close();
                    ui.refresh_page();
                    if let Some(reopen) = reopen {
                        reopen(&ui)
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
    };
    match confirm {
        Some(d) => crate::app::confirm_inline_if(&key, "Confirm delete", move || !d.busy.get() && !d.dirty() && !d.unsent_message.get(), act),
        None => {
            key.connect_clicked(act);
        }
    }
}
