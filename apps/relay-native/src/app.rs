use crate::client::{Client, Error, Notice};
use crate::icons;
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
    if !class.is_empty() {
        l.add_css_class(class);
    }
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
    status_project: gtk::Label,
    status_branch: gtk::Label,
    workspaces: RefCell<Vec<Value>>,
    sidebar: gtk::Box,
    sidebar_key: gtk::Button,
    layout_keys: Vec<gtk::Button>,
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
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        top.add_css_class("topbar");
        let sidebar_key = icons::button("sidebar", "Hide sidebar", "quiet");
        top.append(&sidebar_key);
        let mark = label("R", "brand-mark");
        mark.set_valign(gtk::Align::Center);
        mark.set_xalign(0.5);
        top.append(&mark);
        let brand = label("RELAY", "brand");
        brand.set_hexpand(true);
        top.append(&brand);
        let reconnect = icons::button("refresh", "Reconnect to engine", "quiet");
        top.append(&reconnect);
        let layouts = gtk::MenuButton::new();
        layouts.set_child(Some(&icons::image("grid", 16)));
        layouts.set_tooltip_text(Some("Terminal layout"));
        layouts.update_property(&[gtk::accessible::Property::Label("Terminal layout")]);
        layouts.add_css_class("layout-menu");
        let layout_popover = gtk::Popover::new();
        let choices = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let one = button("Single column", "quiet");
        let two = button("Split columns", "quiet");
        let grid = button("Three columns", "quiet");
        two.add_css_class("selected");
        for key in [&one, &two, &grid] {
            choices.append(key);
        }
        layout_popover.set_child(Some(&choices));
        layouts.set_popover(Some(&layout_popover));
        top.append(&layouts);
        let launch_key = button("New session", "primary");
        let launch_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        launch_content.append(&icons::image("plus", 14));
        launch_content.append(&label("New session", ""));
        launch_key.set_child(Some(&launch_content));
        launch_key.set_valign(gtk::Align::Center);
        launch_key.add_css_class("launch-key");
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
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.set_size_request(200, -1);
        sidebar.set_hexpand(false);
        sidebar.add_css_class("sidebar");
        let nav = gtk::Box::new(gtk::Orientation::Vertical, 1);
        nav.add_css_class("navigation");
        sidebar.append(&nav);
        let section = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        section.add_css_class("workspace-heading");
        let heading = label("WORKSPACES", "section-label");
        heading.set_hexpand(true);
        section.append(&heading);
        let add_project = icons::button("plus", "Open repository", "quiet");
        section.append(&add_project);
        sidebar.append(&section);
        let projects_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let project_scroll = scrolled(&projects_box);
        project_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sidebar.append(&project_scroll);
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
        agents.append(&wall_stack);
        content.add_named(&agents, Some("agents"));
        let editor = crate::editor::Editor::new();
        content.add_named(&editor.root, Some("code"));
        let mut pages = BTreeMap::new();
        for name in ["board", "mailbox", "guardrails", "notes"] {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
            page.add_css_class("page");
            page.set_vexpand(true);
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
        launch.set_halign(gtk::Align::End);
        launch.set_size_request(420, -1);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&body));
        let scrim = gtk::Button::new();
        scrim.add_css_class("launch-scrim");
        scrim.set_visible(false);
        scrim.update_property(&[gtk::accessible::Property::Label("Close new session")]);
        overlay.add_overlay(&scrim);
        overlay.add_overlay(&launch);
        let weak_launch = launch.downgrade();
        scrim.connect_clicked(move |_| {
            if let Some(launch) = weak_launch.upgrade() {
                launch.set_reveal_child(false);
            }
        });
        let weak_body = body.downgrade();
        launch.connect_child_revealed_notify(move |launch| {
            scrim.set_visible(launch.reveals_child());
            if let Some(body) = weak_body.upgrade() {
                body.set_sensitive(!launch.reveals_child());
            }
        });
        outer.append(&overlay);
        let statusbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        statusbar.add_css_class("statusbar");
        let status_project = label("No project", "status-project");
        let status_branch = label("", "mono");
        status_branch.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status_branch.set_max_width_chars(24);
        statusbar.append(&status_project);
        statusbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        statusbar.append(&icons::image("branch", 11));
        statusbar.append(&status_branch);
        statusbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let machine = label("linux · local", "");
        machine.set_hexpand(true);
        statusbar.append(&machine);
        let status = label("Engine disconnected", "");
        statusbar.append(&status);
        outer.append(&statusbar);
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
            status_project,
            status_branch,
            workspaces: RefCell::default(),
            sidebar,
            sidebar_key: sidebar_key.clone(),
            layout_keys: vec![one.clone(), two.clone(), grid.clone()],
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
        for (name, caption, icon) in [
            ("agents", "Agents", "terminal"),
            ("code", "Code", "code"),
            ("board", "Board", "board"),
            ("notes", "Notes", "notes"),
            ("mailbox", "Mailbox", "send"),
            ("guardrails", "Guardrails", "shield"),
        ] {
            let b = button(caption, "nav");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            row.append(&icons::image(icon, 16));
            row.append(&label(caption, ""));
            b.set_child(Some(&row));
            track_navigation(&ui.content, &b, name);
            nav.append(&b);
            let weak = Rc::downgrade(&ui);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.navigate(name);
                }
            });
        }
        let weak = Rc::downgrade(&ui);
        sidebar_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.toggle_sidebar();
            }
        });
        for (b, cols) in [(one, 1), (two, 2), (grid, 3)] {
            let weak = Rc::downgrade(&ui);
            let popover = layout_popover.clone();
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_columns(cols);
                    popover.popdown();
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
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&ui);
        keys.connect_key_pressed(move |_, key, _, _| {
            if let Some(ui) = weak.upgrade() {
                if key == gtk::gdk::Key::Escape && ui.launch.reveals_child() {
                    ui.launch.set_reveal_child(false);
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        ui.window.add_controller(keys);
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
                match ui.call("workspace.list", json!({})).await {
                    Ok(v) if generation == ui.generation.get() => {
                        *ui.workspaces.borrow_mut() = rows(&v, "workspaces");
                    }
                    Ok(_) => continue,
                    Err(e) => ui.show_error(&e.to_string()),
                }
                ui.render_projects();
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
            pane.caption.set_text(name);
            pane.identity
                .set_text(&format!("{} · {}", text(s, "provider"), text(s, "role")));
            pane.branch.set_text(text(s, "branch"));
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
            pane.state.set_text(&state.replace('_', " ").to_uppercase());
            let class = match state {
                "running" | "spawning" => "live",
                "blocked" => "held",
                _ => "waiting",
            };
            pane.root.add_css_class(class);
            clear(&pane.actions);
            let focus = icons::button("maximize", "Zoom terminal / restore layout", "quiet");
            let weak = Rc::downgrade(self);
            let n = name.to_string();
            focus.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    let already = ui.focused.borrow().as_ref() == Some(&n);
                    *ui.focused.borrow_mut() = if already { None } else { Some(n.clone()) };
                    ui.layout();
                }
            });
            let (caption, op) = match state {
                "created" => ("Start", "session.spawn"),
                "parked" => ("Wake", "session.wake"),
                "restorable" | "exited" => ("Resume", "session.resume"),
                _ => ("Park", "session.park"),
            };
            let icon = match op {
                "session.park" => "pause",
                "session.resume" => "resume",
                _ => "play",
            };
            let action = icons::button(icon, caption, "quiet");
            let weak = Rc::downgrade(self);
            let n = name.to_string();
            action.connect_clicked(move |b| {
                if let Some(ui) = weak.upgrade() {
                    ui.mutate(op, json!({"session":n}), b);
                }
            });
            pane.actions.append(&action);
            pane.actions.append(&focus);
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
        self.status_project.set_text(&name);
        self.status_branch.set_text(
            self.projects
                .borrow()
                .iter()
                .find(|p| p["id"].as_i64() == Some(self.project.get()))
                .map(|p| text(p, "base_branch"))
                .unwrap_or(""),
        );
        self.status.set_text(&format!(
            "{held} needs you  ·  {live} live  ·  {} sessions",
            sessions.len()
        ));
        self.render_projects();
        self.wall_stack
            .set_visible_child_name(if sessions.is_empty() { "empty" } else { "wall" });
    }
    pub fn toggle_sidebar(&self) {
        let visible = !self.sidebar.is_visible();
        self.sidebar.set_visible(visible);
        let caption = if visible {
            "Hide sidebar"
        } else {
            "Show sidebar"
        };
        self.sidebar_key.set_tooltip_text(Some(caption));
        self.sidebar_key
            .update_property(&[gtk::accessible::Property::Label(caption)]);
    }
    pub fn set_columns(&self, columns: i32) {
        self.columns.set(columns);
        self.focused.borrow_mut().take();
        for (index, key) in self.layout_keys.iter().enumerate() {
            if index as i32 + 1 == columns {
                key.add_css_class("selected");
            } else {
                key.remove_css_class("selected");
            }
        }
        self.layout();
    }
    fn render_projects(self: &Rc<Self>) {
        clear(&self.projects_box);
        let projects = self.projects.borrow();
        let workspaces = self.workspaces.borrow();
        let mut groups: Vec<Option<i64>> = workspaces.iter().map(|w| w["id"].as_i64()).collect();
        // Retain projects even when their workspace has disappeared or could not be loaded.
        groups.push(None);
        for group in groups {
            let members: Vec<_> = projects
                .iter()
                .filter(|p| match group {
                    Some(id) => p["workspace_id"].as_i64() == Some(id),
                    None => !workspaces.iter().any(|w| w["id"] == p["workspace_id"]),
                })
                .collect();
            if members.is_empty() {
                continue;
            }
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            heading.add_css_class("workspace-row");
            heading.append(&icons::image("chevron-down", 12));
            let name = workspaces
                .iter()
                .find(|w| w["id"].as_i64() == group)
                .map(|w| text(w, "name"))
                .unwrap_or("Repositories");
            let title = label(name, "dim");
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            heading.append(&title);
            self.projects_box.append(&heading);
            for p in members {
                let id = p["id"].as_i64().unwrap_or(0);
                let selected = id == self.project.get();
                let key = button(text(p, "name"), "project");
                key.set_tooltip_text(Some(text(p, "path")));
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                lamp.add_css_class("lamp");
                lamp.set_valign(gtk::Align::Start);
                lamp.set_margin_top(5);
                if selected {
                    let sessions = self.sessions.borrow();
                    if sessions.iter().any(|s| text(s, "state") == "blocked") {
                        lamp.add_css_class("held");
                    } else if sessions
                        .iter()
                        .any(|s| matches!(text(s, "state"), "running" | "spawning"))
                    {
                        lamp.add_css_class("live");
                    }
                }
                row.append(&lamp);
                let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
                info.set_hexpand(true);
                let name = label(text(p, "name"), "project-name");
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                name.set_max_width_chars(20);
                info.append(&name);
                if selected {
                    key.add_css_class("selected");
                    let branch = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                    branch.add_css_class("project-branch");
                    branch.append(&icons::image("branch", 11));
                    let name = label(text(p, "base_branch"), "mono");
                    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    name.set_max_width_chars(20);
                    branch.append(&name);
                    info.append(&branch);
                }
                row.append(&info);
                key.set_child(Some(&row));
                let weak = Rc::downgrade(self);
                key.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        if ui.project.get() == id {
                            ui.navigate("agents");
                            return;
                        }
                        if ui.editor.is_dirty() {
                            ui.show_error(
                                "Save or discard your editor changes before switching projects.",
                            );
                            return;
                        }
                        ui.project.set(id);
                        ui.sessions.borrow_mut().clear();
                        ui.editor.reset();
                        ui.navigate("agents");
                        ui.refresh();
                    }
                });
                self.projects_box.append(&key);
            }
        }
        if projects.is_empty() {
            let empty = label("No projects yet. Use + to open a repository.", "dim");
            empty.set_wrap(true);
            empty.set_margin_start(12);
            empty.set_margin_end(12);
            self.projects_box.append(&empty);
        }
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
    pub fn verify_launch(&self) {
        assert!(self.launch.reveals_child());
        assert!(
            self.launch.width() >= 400 && self.launch_box.width() >= 350,
            "Launch form must be visible inside the overlay"
        );
        assert!(
            !self.sidebar.is_sensitive(),
            "The sheet must block background actions"
        );
    }
    pub fn verify_shell(self: &Rc<Self>) {
        assert_eq!(
            self.sidebar.width(),
            199,
            "Sidebar content plus its 1px edge must occupy 200px"
        );
        let panes = self.panes.borrow().clone();
        let shown = self.sidebar.is_visible();
        self.sidebar_key.emit_clicked();
        assert_ne!(self.sidebar.is_visible(), shown);
        self.sidebar_key.emit_clicked();
        assert_eq!(self.sidebar.is_visible(), shown);
        for (index, key) in self.layout_keys.iter().enumerate() {
            key.emit_clicked();
            assert_eq!(self.columns.get(), index as i32 + 1);
            assert!(key.has_css_class("selected"));
            for (name, pane) in &panes {
                assert!(
                    Rc::ptr_eq(pane, &self.panes.borrow()[name]),
                    "Relayout replaced a terminal"
                );
                assert!(pane.root.parent().is_some());
            }
        }
        self.set_columns(2);
        assert!(self.status_project.text().contains("Native verification"));
        println!("Shell controls verified: sidebar, three layouts, retained terminals");
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
        self.launch_box.append(&label("New session", "title"));
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
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        actions.append(&cancel);
        actions.append(&start);
        self.launch_box.append(&actions);
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
