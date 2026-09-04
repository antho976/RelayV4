use crate::client::{Client, Error, Notice};
use crate::terminal::Pane;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use tokio::runtime::Handle;

pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
pub fn rows(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}
pub fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.add_css_class(class);
    l
}
pub fn button(text: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class(class);
    if matches!(class, "nav" | "project" | "file") {
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
    }
    b
}
pub fn clear(container: &gtk::Box) {
    while let Some(w) = container.first_child() {
        container.remove(&w);
    }
}
pub fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(child)
        .hexpand(true)
        .vexpand(true)
        .build()
}
pub fn field(caption: &str, widget: &impl IsA<gtk::Widget>, parent: &gtk::Box) {
    parent.append(&label(caption, "dim"));
    parent.append(widget);
}

fn track_navigation(stack: &gtk::Stack, key: &gtk::Button, name: &'static str) {
    if name == "agents" {
        key.add_css_class("selected");
    }
    let weak = key.downgrade();
    stack.connect_visible_child_name_notify(move |s| {
        if let Some(key) = weak.upgrade() {
            if s.visible_child_name().as_deref() == Some(name) {
                key.add_css_class("selected");
            } else {
                key.remove_css_class("selected");
            }
        }
    });
}

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    pub rt: Handle,
    pub path: PathBuf,
    pub client: RefCell<Option<Client>>,
    pub project: Cell<i64>,
    pub generation: Cell<u64>,
    pub sessions: RefCell<Vec<Value>>,
    pub projects: RefCell<Vec<Value>>,
    pub page: RefCell<String>,
    pub content: gtk::Stack,
    pub pages: BTreeMap<String, gtk::Box>,
    pub page_projects: RefCell<BTreeMap<String, i64>>,
    pub notice: gtk::Label,
    status: gtk::Label,
    projects_box: gtk::Box,
    wall: gtk::Grid,
    wall_stack: gtk::Stack,
    panes: RefCell<BTreeMap<String, Rc<Pane>>>,
    ordered: RefCell<Vec<String>>,
    columns: Cell<i32>,
    focused: RefCell<Option<String>>,
    refresh_pending: Cell<bool>,
    refresh_dirty: Cell<bool>,
    page_pending: Cell<bool>,
    page_dirty: Cell<bool>,
    connected: Cell<bool>,
    launch: gtk::Revealer,
    launch_box: gtk::Box,
    pub editor: Rc<crate::editor::Editor>,
}

pub fn run(rt: Handle) -> glib::ExitCode {
    let instance = std::env::var("RELAY_INSTANCE").unwrap_or_else(|_| "dev".into());
    if !matches!(instance.as_str(), "dev" | "test" | "stable") {
        eprintln!("RELAY_INSTANCE must be dev, test or stable");
        return glib::ExitCode::FAILURE;
    }
    let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        eprintln!("XDG_RUNTIME_DIR is required to locate the Relay engine");
        return glib::ExitCode::FAILURE;
    };
    let path = std::env::var_os("RELAY_NATIVE_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(runtime_dir)
                .join("relay")
                .join(format!("{instance}.sock"))
        });
    let app = gtk::Application::builder()
        .application_id("com.quietsoftware.Relay4")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        let provider = gtk::CssProvider::new();
        provider.connect_parsing_error(|_, _, e| tracing::error!("stylesheet: {e}"));
        provider.load_from_string(include_str!("theme.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let ui = Ui::build(app, rt.clone(), path.clone());
        ui.window.present();
        ui.connect();
        crate::smoke::install(&ui);
    });
    app.run_with_args::<&str>(&[])
}

impl Ui {
    fn build(app: &gtk::Application, rt: Handle, path: PathBuf) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Relay")
            .default_width(1440)
            .default_height(900)
            .build();
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.add_css_class("topbar");
        let mark = label("R", "brand-mark");
        mark.set_valign(gtk::Align::Center);
        top.append(&mark);
        top.append(&label("RELAY", "brand"));
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tabs.set_halign(gtk::Align::Center);
        tabs.set_hexpand(true);
        top.append(&tabs);
        let reconnect = button("Reconnect", "quiet");
        top.append(&reconnect);
        let launch_key = button("+ Session", "primary");
        top.append(&launch_key);
        top.append(&gtk::WindowControls::new(gtk::PackType::End));
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&top));
        window.set_titlebar(Some(&handle));
        let notice = label("Connecting to the Relay engine…", "notice");
        notice.set_wrap(true);
        notice.set_selectable(true);
        outer.append(&notice);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 2);
        sidebar.set_size_request(200, -1);
        sidebar.set_hexpand(false);
        sidebar.add_css_class("sidebar");
        let nav = gtk::Box::new(gtk::Orientation::Vertical, 2);
        sidebar.append(&nav);
        sidebar.append(&label("WORKSPACES", "section-label"));
        let projects_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        sidebar.append(&scrolled(&projects_box));
        let add_project = button("+ Open repository", "quiet");
        sidebar.append(&add_project);
        sidebar.append(&label("Claude  ·  Codex", "sidebar-footer"));
        body.append(&sidebar);
        let content = gtk::Stack::new();
        content.set_hexpand(true);
        content.set_vexpand(true);
        body.append(&content);
        let wall = gtk::Grid::builder()
            .hexpand(true)
            .vexpand(true)
            .column_homogeneous(true)
            .row_homogeneous(true)
            .column_spacing(2)
            .row_spacing(2)
            .build();
        let wall_stack = gtk::Stack::new();
        wall_stack.set_vexpand(true);
        let wall_scroll = scrolled(&wall);
        wall_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        wall_stack.add_named(&wall_scroll, Some("wall"));
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 12);
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        empty.append(&label("Your agents, in one place", "title"));
        empty.append(&label(
            "Choose a project and add an agent to start working.",
            "dim",
        ));
        let empty_launch = button("Add an agent", "primary");
        empty.append(&empty_launch);
        wall_stack.add_named(&empty, Some("empty"));
        wall_stack.set_visible_child_name("empty");
        let agents = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let wall_tools = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        wall_tools.add_css_class("toolbar");
        let heading = label("Agents", "title");
        heading.set_hexpand(true);
        wall_tools.append(&heading);
        let one = button("Single", "quiet");
        let two = button("Split", "quiet");
        let grid = button("Grid", "quiet");
        for b in [&one, &two, &grid] {
            wall_tools.append(b);
        }
        agents.append(&wall_tools);
        agents.append(&wall_stack);
        content.add_named(&agents, Some("agents"));
        let editor = crate::editor::Editor::new();
        content.add_named(&editor.root, Some("code"));
        let mut pages = BTreeMap::new();
        for name in ["board", "mailbox", "guardrails", "notes"] {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
            page.add_css_class("page");
            content.add_named(&scrolled(&page), Some(name));
            pages.insert(name.into(), page);
        }
        let launch_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        launch_box.add_css_class("launch");
        launch_box.set_size_request(380, -1);
        let launch = gtk::Revealer::new();
        launch.set_transition_duration(0);
        launch.set_hexpand(false);
        launch.set_child(Some(&scrolled(&launch_box)));
        body.append(&launch);
        outer.append(&body);
        let status = label("Engine disconnected", "statusbar");
        outer.append(&status);
        window.set_child(Some(&outer));
        let ui = Rc::new(Self {
            window,
            rt,
            path,
            client: RefCell::new(None),
            project: Cell::new(0),
            generation: Cell::new(0),
            sessions: RefCell::default(),
            projects: RefCell::default(),
            page: RefCell::new("agents".into()),
            content,
            pages,
            page_projects: RefCell::default(),
            notice,
            status,
            projects_box,
            wall,
            wall_stack,
            panes: RefCell::default(),
            ordered: RefCell::default(),
            columns: Cell::new(2),
            focused: RefCell::new(None),
            refresh_pending: Cell::new(false),
            refresh_dirty: Cell::new(false),
            page_pending: Cell::new(false),
            page_dirty: Cell::new(false),
            connected: Cell::new(false),
            launch,
            launch_box,
            editor,
        });
        for (name, caption) in [
            ("agents", "AGENTS"),
            ("code", "CODE"),
            ("board", "BOARD"),
            ("notes", "NOTES"),
        ] {
            let b = button(caption, "tab");
            track_navigation(&ui.content, &b, name);
            tabs.append(&b);
            let weak = Rc::downgrade(&ui);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.navigate(name);
                }
            });
        }
        for (name, caption) in [
            ("agents", "Agents"),
            ("board", "Tasks & reviews"),
            ("mailbox", "Mailbox"),
            ("guardrails", "Guardrails"),
        ] {
            let b = button(caption, "nav");
            track_navigation(&ui.content, &b, name);
            nav.append(&b);
            let weak = Rc::downgrade(&ui);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.navigate(name);
                }
            });
        }
        for (b, cols) in [(one, 1), (two, 2), (grid, 3)] {
            let weak = Rc::downgrade(&ui);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.columns.set(cols);
                    ui.focused.borrow_mut().take();
                    ui.layout();
                }
            });
        }
        for b in [launch_key, empty_launch] {
            let weak = Rc::downgrade(&ui);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.show_launch(None);
                }
            });
        }
        let weak = Rc::downgrade(&ui);
        reconnect.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.connect();
            }
        });
        let weak = Rc::downgrade(&ui);
        add_project.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.open_repository();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_realize(move |w| {
            if let Some(surface) = w.surface() {
                let weak = weak.clone();
                surface.connect_layout(move |_, _, _| {
                    if let Some(ui) = weak.upgrade() {
                        for p in ui.panes.borrow().values() {
                            p.schedule_resize();
                        }
                    }
                });
            }
        });
        let owned = ui.clone();
        ui.window.connect_close_request(move |_| {
            if owned.editor.is_dirty() {
                owned.show_error("Save or discard your editor changes before closing.");
                return glib::Propagation::Stop;
            }
            owned.generation.set(owned.generation.get() + 1);
            owned.connected.set(false);
            owned.client.borrow_mut().take();
            for pane in owned.panes.borrow().values() {
                pane.stop();
            }
            owned.panes.borrow_mut().clear();
            glib::Propagation::Proceed
        });
        ui
    }
    pub fn show_error(&self, message: &str) {
        self.notice.set_text(message);
        self.notice.set_visible(true);
    }
    pub async fn call(&self, op: &str, payload: Value) -> Result<Value, Error> {
        let client = self.client.borrow().clone().ok_or(Error::Disconnected)?;
        client.request(&self.rt, op, payload).await
    }
    pub fn mutate(self: &Rc<Self>, op: &'static str, payload: Value, key: &gtk::Button) {
        key.set_sensitive(false);
        let key = key.clone();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match ui.call(op, payload).await {
                Ok(_) => ui.refresh(),
                Err(e) => ui.show_error(&e.to_string()),
            }
            key.set_sensitive(true);
            ui.refresh_page();
        });
    }
    fn connect(self: &Rc<Self>) {
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        self.client.borrow_mut().take();
        self.connected.set(false);
        for p in self.panes.borrow().values() {
            p.stop();
        }
        self.panes.borrow_mut().clear();
        self.ordered.borrow_mut().clear();
        self.layout();
        self.show_error("Connecting to the Relay engine…");
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match Client::connect(&ui.rt, ui.path.clone()).await {
                Ok((client, notices)) => {
                    if generation != ui.generation.get() {
                        return;
                    }
                    *ui.client.borrow_mut() = Some(client);
                    if let Err(e)=ui.call("bus.subscribe",json!({"events":["project.changed","project.deleted","workspace.changed","session.changed","task.changed","task.deleted","mailbox.new","mailbox.changed","guardrail.held","guardrail.resolved","overlap.changed","notes.changed","file.changed"]})).await { ui.show_error(&e.to_string()); return; }
                    ui.connected.set(true);
                    ui.notice.set_visible(false);
                    ui.refresh();
                    let weak = Rc::downgrade(&ui);
                    drop(ui);
                    while let Ok(notice) = notices.recv().await {
                        let Some(ui) = weak.upgrade() else {
                            break;
                        };
                        if ui.generation.get() != generation {
                            break;
                        }
                        match notice {
                            Notice::Event(e) => {
                                if e.project_id.is_some_and(|id| id != ui.project.get())
                                    && !e.ev.starts_with("project.")
                                {
                                    continue;
                                }
                                if e.ev.starts_with("session.")
                                    || e.ev.starts_with("project.")
                                    || e.ev.starts_with("workspace.")
                                {
                                    ui.refresh();
                                } else if e.ev == "file.changed" {
                                    ui.editor.invalidate(&ui);
                                } else {
                                    ui.refresh_page();
                                }
                            }
                            Notice::Disconnected(e) => {
                                ui.connected.set(false);
                                ui.client.borrow_mut().take();
                                ui.status.set_text(
                                    "Engine disconnected · sessions remain owned by the engine",
                                );
                                ui.show_error(&e.to_string());
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => ui.show_error(&format!(
                    "{e}. Start relay serve for this instance, then reconnect."
                )),
            }
        });
    }
    pub fn navigate(self: &Rc<Self>, page: &str) {
        *self.page.borrow_mut() = page.into();
        self.content.set_visible_child_name(page);
        self.layout();
        if page == "code" {
            self.editor.load_tree(self, None);
        } else {
            self.refresh_page();
        }
    }
    pub fn refresh(self: &Rc<Self>) {
        self.refresh_dirty.set(true);
        if self.refresh_pending.replace(true) {
            return;
        }
        let ui = self.clone();
        glib::spawn_future_local(async move {
            while ui.refresh_dirty.replace(false) {
                let generation = ui.generation.get();
                let result = ui.call("project.list", json!({})).await;
                if generation != ui.generation.get() {
                    continue;
                }
                let projects = match result {
                    Ok(v) => rows(&v, "projects"),
                    Err(e) => {
                        ui.show_error(&e.to_string());
                        break;
                    }
                };
                *ui.projects.borrow_mut() = projects.clone();
                if !projects
                    .iter()
                    .any(|p| p["id"].as_i64() == Some(ui.project.get()))
                {
                    ui.project
                        .set(projects.first().and_then(|p| p["id"].as_i64()).unwrap_or(0));
                }
                clear(&ui.projects_box);
                for p in &projects {
                    let id = p["id"].as_i64().unwrap_or(0);
                    let b = button(
                        &format!("{}\n{}", text(p, "name"), text(p, "base_branch")),
                        "project",
                    );
                    if id == ui.project.get() {
                        b.add_css_class("selected");
                    }
                    let weak = Rc::downgrade(&ui);
                    b.connect_clicked(move |_| {
                        if let Some(ui)=weak.upgrade() {
                            if ui.project.get()==id { return; }
                            if ui.editor.is_dirty() { ui.show_error("Save or discard your editor changes before switching projects."); return; }
                            ui.project.set(id); ui.editor.reset(); ui.refresh(); ui.refresh_page();
                        }
                    });
                    ui.projects_box.append(&b);
                }
                let project = ui.project.get();
                if project == 0 {
                    ui.sessions.borrow_mut().clear();
                    ui.reconcile();
                    continue;
                }
                match ui
                    .call(
                        "session.list",
                        json!({"project_id":project,"include_closed":false}),
                    )
                    .await
                {
                    Ok(v) if ui.project.get() == project && generation == ui.generation.get() => {
                        *ui.sessions.borrow_mut() = rows(&v, "sessions");
                        ui.reconcile();
                    }
                    Ok(_) => {}
                    Err(e) => ui.show_error(&e.to_string()),
                }
                ui.refresh_page();
            }
            ui.refresh_pending.set(false);
        });
    }
    fn reconcile(self: &Rc<Self>) {
        let sessions = self.sessions.borrow().clone();
        let names: Vec<String> = sessions.iter().map(|s| text(s, "name").into()).collect();
        let old: Vec<String> = self
            .panes
            .borrow()
            .keys()
            .filter(|n| !names.contains(n))
            .cloned()
            .collect();
        for n in old {
            if let Some(p) = self.panes.borrow_mut().remove(&n) {
                p.stop();
                if p.root.parent().is_some() {
                    self.wall.remove(&p.root);
                }
            }
        }
        for s in &sessions {
            let name = text(s, "name");
            if !self.panes.borrow().contains_key(name) {
                let pane = Pane::new(name, self.path.clone(), self.rt.clone());
                self.panes.borrow_mut().insert(name.into(), pane);
            }
            let pane = self.panes.borrow().get(name).cloned().unwrap();
            pane.caption
                .set_text(&format!("{}  ·  {}", name, text(s, "role")));
            pane.caption.set_tooltip_text(Some(&format!(
                "{}\n{}\n{}",
                text(s, "branch"),
                text(s, "worktree"),
                text(s, "intent")
            )));
            for class in ["live", "held", "waiting"] {
                pane.root.remove_css_class(class);
            }
            let state = text(s, "state");
            let class = match state {
                "running" | "spawning" => "live",
                "blocked" => "held",
                _ => "waiting",
            };
            pane.root.add_css_class(class);
            clear(&pane.actions);
            let focus = button("Focus", "quiet");
            let weak = Rc::downgrade(self);
            let n = name.to_string();
            focus.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    let already = ui.focused.borrow().as_ref() == Some(&n);
                    *ui.focused.borrow_mut() = if already { None } else { Some(n.clone()) };
                    ui.layout();
                }
            });
            pane.actions.append(&focus);
            let (caption, op) = match state {
                "created" => ("Start", "session.spawn"),
                "parked" => ("Wake", "session.wake"),
                "restorable" | "exited" => ("Resume", "session.resume"),
                _ => ("Park", "session.park"),
            };
            let action = button(caption, "quiet");
            let weak = Rc::downgrade(self);
            let n = name.to_string();
            action.connect_clicked(move |b| {
                if let Some(ui) = weak.upgrade() {
                    ui.mutate(op, json!({"session":n}), b);
                }
            });
            pane.actions.append(&action);
        }
        if *self.ordered.borrow() != names {
            *self.ordered.borrow_mut() = names;
            self.layout();
        } else {
            self.update_attachments();
        }
        let live = sessions
            .iter()
            .filter(|s| matches!(text(s, "state"), "running" | "spawning"))
            .count();
        let held = sessions
            .iter()
            .filter(|s| text(s, "state") == "blocked")
            .count();
        let name = self
            .projects
            .borrow()
            .iter()
            .find(|p| p["id"].as_i64() == Some(self.project.get()))
            .map(|p| text(p, "name").to_string())
            .unwrap_or_else(|| "No project".into());
        self.status.set_text(&format!(
            "{name}     │     {held} needs you  ·  {live} live  ·  {} sessions     │     Native",
            sessions.len()
        ));
        self.wall_stack
            .set_visible_child_name(if sessions.is_empty() { "empty" } else { "wall" });
    }
    fn update_attachments(&self) {
        let focus = self.focused.borrow();
        for s in self.sessions.borrow().iter() {
            if let Some(p) = self.panes.borrow().get(text(s, "name")) {
                let active = *self.page.borrow() == "agents"
                    && focus.as_ref().is_none_or(|n| n == text(s, "name"))
                    && !matches!(
                        text(s, "state"),
                        "created" | "parked" | "restorable" | "exited"
                    );
                p.set_active(active);
            }
        }
    }
    pub fn verify_terminal_input(&self) {
        use vte4::prelude::TerminalExt;
        for (name, pane) in self.panes.borrow().iter() {
            pane.terminal
                .paste_text(&format!("native-paste-check-{name}\n"));
        }
    }
    fn layout(&self) {
        while let Some(w) = self.wall.first_child() {
            self.wall.remove(&w);
        }
        if self
            .focused
            .borrow()
            .as_ref()
            .is_some_and(|n| !self.ordered.borrow().contains(n))
        {
            self.focused.borrow_mut().take();
        }
        let names = self.ordered.borrow();
        let focus = self.focused.borrow();
        let columns = if focus.is_some() {
            1
        } else {
            self.columns.get()
        };
        let mut index = 0;
        for name in names.iter() {
            if let Some(p) = self.panes.borrow().get(name) {
                if focus.as_ref().is_some_and(|n| n != name) {
                    continue;
                }
                self.wall
                    .attach(&p.root, index % columns, index / columns, 1, 1);
                p.schedule_resize();
                index += 1;
            }
        }
        drop(focus);
        self.update_attachments();
    }
    pub fn refresh_page(self: &Rc<Self>) {
        self.page_dirty.set(true);
        if self.page_pending.replace(true) {
            return;
        }
        let ui = self.clone();
        glib::spawn_future_local(async move {
            while ui.page_dirty.replace(false) {
                let page = ui.page.borrow().clone();
                let project = ui.project.get();
                if project == 0 || matches!(page.as_str(), "agents" | "code") {
                    continue;
                }
                crate::pages::refresh(&ui, &page, project).await;
            }
            ui.page_pending.set(false);
        });
    }
    fn open_repository(self: &Rc<Self>) {
        if self.editor.is_dirty() {
            self.show_error(
                "Save or discard your editor changes before opening another repository.",
            );
            return;
        }
        let chooser = gtk::FileDialog::builder()
            .title("Open a Git repository")
            .build();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let Ok(file) = chooser.select_folder_future(Some(&ui.window)).await else {
                return;
            };
            let Some(path) = file.path() else {
                return;
            };
            let parent = path.parent().unwrap_or(&path);
            let result = async {
                let listed = ui.call("workspace.list", json!({})).await?;
                let workspace = if let Some(existing) = rows(&listed, "workspaces")
                    .into_iter()
                    .find(|w| text(w, "path") == parent.to_string_lossy())
                {
                    existing
                } else {
                    ui.call("workspace.create", json!({"path":parent})).await?
                };
                ui.call(
                    "project.add",
                    json!({"workspace_id":workspace["id"],"path":path}),
                )
                .await
            }
            .await;
            match result {
                Ok(p) => {
                    if ui.editor.is_dirty() {
                        ui.show_error("Repository added. The current project was kept because the editor has changes or a file operation in progress.");
                        ui.refresh();
                        return;
                    }
                    ui.project.set(p["id"].as_i64().unwrap_or(0));
                    ui.editor.reset();
                    ui.refresh();
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
        });
    }
    pub fn show_launch(self: &Rc<Self>, task: Option<i64>) {
        if self.project.get() == 0 {
            self.show_error("Open a repository before adding agents.");
            return;
        }
        clear(&self.launch_box);
        self.launch.set_reveal_child(true);
        self.launch_box.append(&label("Add agents", "title"));
        let provider = gtk::DropDown::from_strings(&["Claude", "Codex"]);
        field("Builder", &provider, &self.launch_box);
        let model = gtk::Entry::builder()
            .placeholder_text("Provider default")
            .build();
        field("Model (optional)", &model, &self.launch_box);
        let pair = gtk::CheckButton::with_label("Add a reviewer in the same worktree");
        self.launch_box.append(&pair);
        let reviewer = gtk::DropDown::from_strings(&["Codex", "Claude"]);
        field("Reviewer", &reviewer, &self.launch_box);
        let prompt = gtk::TextView::new();
        prompt.set_wrap_mode(gtk::WrapMode::WordChar);
        prompt.set_size_request(-1, 180);
        field("Assignment", &prompt, &self.launch_box);
        if let Some(task) = task {
            self.launch_box
                .append(&label(&format!("Task #{task}"), "dim"));
        }
        let hint=label("Each builder gets an isolated worktree. Reviewers share that worktree with read-only authority. Task approval stays with you.","dim");
        hint.set_wrap(true);
        self.launch_box.append(&hint);
        let start = button("Launch", "primary");
        let cancel = button("Cancel", "quiet");
        self.launch_box.append(&start);
        self.launch_box.append(&cancel);
        let weak = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.launch.set_reveal_child(false);
            }
        });
        let weak = Rc::downgrade(self);
        let project = self.project.get();
        start.connect_clicked(move |b| {
            let Some(ui)=weak.upgrade() else{return;}; b.set_sensitive(false); let key=b.clone();
            let provider=if provider.selected()==0{"claude"}else{"codex"};
            let reviewer=if reviewer.selected()==0{"codex"}else{"claude"}; let paired=pair.is_active();
            let model=model.text().to_string(); let buffer=prompt.buffer();let prompt=buffer.text(&buffer.start_iter(),&buffer.end_iter(),false).to_string();
            glib::spawn_future_local(async move {
                let result=async {
                    let builder=ui.call("session.create",json!({"project_id":project,"provider":provider,"role":"builder","model":if model.is_empty(){None}else{Some(model)},"task_id":task})).await?;
                    let name=text(&builder,"name");
                    let review=if paired {
                        let review=ui.call("session.create",json!({"project_id":project,"provider":reviewer,"role":"reviewer","pair_with":name,"task_id":task})).await?;
                        Some(review)
                    }else{None};
                    if let Some(task)=task {ui.call("task.dispatch",json!({"task_id":task,"session":name,"start":false})).await?;}
                    if let Some(review)=review{ui.call("session.spawn",json!({"session":review["name"],"prompt":format!("Review the paired builder's work. Coordinate through Relay mailbox. Assignment: {prompt}")})).await?;}
                    ui.call("session.spawn",json!({"session":name,"prompt":prompt})).await
                }.await;
                match result {Ok(_)=>{ui.launch.set_reveal_child(false);ui.navigate("agents");},Err(e)=>ui.show_error(&format!("Launch incomplete: {e}. Allocated sessions are preserved on the wall; inspect them before retrying."))}
                key.set_sensitive(true);ui.refresh();
            });
        });
    }
}
