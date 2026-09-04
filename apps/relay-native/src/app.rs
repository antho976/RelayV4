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
#[path = "launch.rs"]
mod launch;
#[path = "shell.rs"]
mod shell;

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
pub fn icon_button(icon: &str, caption: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&crate::icons::image(icon, 16)));
    b.add_css_class("quiet");
    b.add_css_class("icon-key");
    b.set_tooltip_text(Some(caption));
    b.update_property(&[gtk::accessible::Property::Label(caption)]);
    b
}
pub fn nav_button(caption: &str, icon: &str) -> gtk::Button {
    let b = button("", "nav");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    let image = crate::icons::image(icon, 14);
    image.set_pixel_size(14);
    row.append(&image);
    row.append(&label(caption, "nav-label"));
    b.set_child(Some(&row));
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
    applying_ui: Cell<bool>,
    pub content: gtk::Stack,
    pub pages: BTreeMap<String, gtk::Box>,
    pub page_projects: RefCell<BTreeMap<String, i64>>,
    pub notice: gtk::Label,
    status: gtk::Label,
    status_project: gtk::Label,
    status_branch: gtk::Label,
    sidebar: gtk::Box,
    focus_tabs: gtk::Box,
    mode: RefCell<String>,
    pub overlay: gtk::Overlay,
    pub panels: Rc<RefCell<Vec<Rc<crate::panel::Panel>>>>,
    registry_dirty: Cell<bool>,
    workspaces: RefCell<Vec<Value>>,
    rendered_sessions: RefCell<BTreeMap<String, Value>>,
    restored_project: Cell<i64>,
    appearance: gtk::CssProvider,
    wallpaper: gtk::Picture,
    wallpaper_dim: gtk::Box,
    font_size: Cell<f64>,
    palette: RefCell<String>,
    pub keybindings: RefCell<Value>,
    pub sound_busy: Cell<bool>,
    projects_box: gtk::Box,
    wall: gtk::Grid,
    wall_right: gtk::Grid,
    wall_split: gtk::Paned,
    wall_files: gtk::Box,
    file_tree_revision: Cell<u64>,
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
    launch_busy: Cell<bool>,
    launch_box: gtk::Box,
    pub editor: Rc<crate::editor::Editor>,
    pub note_tabs: gtk::Notebook,
    pub note_drafts: RefCell<BTreeMap<i64, Rc<crate::pages::Draft>>>,
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
                .join("relay-v4")
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
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_icon_theme_name(Some("Adwaita"));
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
        let sidebar_key = icon_button("sidebar-show-symbolic", "Toggle sidebar");
        sidebar_key.set_widget_name("sidebar-toggle");
        top.append(&sidebar_key);
        let mark = label("R", "brand-mark");
        mark.set_valign(gtk::Align::Center);
        mark.set_xalign(0.5);
        top.append(&mark);
        let brand = label("RELAY", "brand");
        brand.set_hexpand(true);
        top.append(&brand);
        let palette_key = icon_button("system-search-symbolic", "Command palette · Ctrl K");
        let layouts_key = icon_button("view-grid-symbolic", "Window presets");
        let notifications_key = icon_button("alarm-symbolic", "Notifications");
        for key in [&palette_key, &layouts_key, &notifications_key] {
            top.append(key);
        }
        let reconnect = icon_button("view-refresh-symbolic", "Reconnect to engine");
        top.append(&reconnect);
        let launch_key = button("New session", "primary");
        let launch_label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        launch_label.append(&crate::icons::image("plus", 14));
        launch_label.append(&label("New session", ""));
        launch_key.set_child(Some(&launch_label));
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
        let add_project = icon_button("plus", "Open repository");
        section.append(&add_project);
        sidebar.append(&section);
        let projects_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let project_scroll = scrolled(&projects_box);
        project_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sidebar.append(&project_scroll);
        let settings_key = nav_button("Settings", "settings");
        settings_key.add_css_class("settings-key");
        sidebar.append(&settings_key);
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
        let wall_right = gtk::Grid::builder()
            .hexpand(true)
            .vexpand(true)
            .row_homogeneous(true)
            .row_spacing(2)
            .build();
        let wall_split = gtk::Paned::new(gtk::Orientation::Horizontal);
        wall_split.set_start_child(Some(&wall));
        wall_split.set_end_child(Some(&wall_right));
        wall_split.set_position(600);
        wall_split.set_shrink_start_child(false);
        wall_split.set_shrink_end_child(false);
        wall.set_size_request(280, -1);
        wall_right.set_size_request(280, -1);
        let wall_scroll = scrolled(&wall_split);
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
        let focus_tabs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        focus_tabs.add_css_class("focus-tabs");
        agents.append(&focus_tabs);
        let wall_body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let files = icon_button("view-list-symbolic", "Show agent file tree");
        files.set_valign(gtk::Align::Start);
        files.add_css_class("files-rail");
        wall_body.append(&files);
        let wall_files = gtk::Box::new(gtk::Orientation::Vertical, 2);
        wall_files.set_size_request(220, -1);
        wall_files.set_visible(false);
        wall_body.append(&wall_files);
        wall_body.append(&wall_stack);
        agents.append(&wall_body);
        content.add_named(&agents, Some("agents"));
        let editor = crate::editor::Editor::new();
        content.add_named(&editor.root, Some("code"));
        let mut pages = BTreeMap::new();
        for name in [
            "board",
            "modules",
            "plan",
            "mailbox",
            "guardrails",
            "notes",
            "dashboard",
            "settings",
            "skills",
            "plugins",
            "notifications",
            "devices",
        ] {
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
        let weak_body = body.downgrade();
        let weak_scrim = scrim.downgrade();
        launch.connect_child_revealed_notify(move |launch| {
            if let Some(scrim) = weak_scrim.upgrade() {
                scrim.set_visible(launch.reveals_child());
            }
            if let Some(body) = weak_body.upgrade() {
                body.set_sensitive(!launch.reveals_child());
            }
        });
        let panel_host = gtk::Overlay::new();
        panel_host.set_child(Some(&overlay));
        outer.append(&panel_host);
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bottom.add_css_class("statusbar");
        let status_project = label("No project", "status-project");
        status_project.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status_project.set_max_width_chars(24);
        let status_branch = label("", "mono");
        status_branch.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status_branch.set_max_width_chars(18);
        bottom.append(&status_project);
        bottom.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        bottom.append(&crate::icons::image("branch", 11));
        bottom.append(&status_branch);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        bottom.append(&spacer);
        let status = label("Engine disconnected", "");
        bottom.append(&status);
        let devices_key = button("Devices", "quiet");
        let usage_key = button("Usage", "quiet");
        let resources_key = button("Resources", "quiet");
        bottom.append(&usage_key);
        bottom.append(&resources_key);
        bottom.append(&devices_key);
        outer.append(&bottom);
        let wallpaper = gtk::Picture::new();
        wallpaper.set_can_shrink(true);
        wallpaper.set_content_fit(gtk::ContentFit::Cover);
        let backdrop = gtk::Overlay::new();
        backdrop.set_child(Some(&wallpaper));
        let wallpaper_dim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        wallpaper_dim.add_css_class("wallpaper-dim");
        wallpaper_dim.set_can_target(false);
        backdrop.add_overlay(&wallpaper_dim);
        backdrop.add_overlay(&outer);
        outer.set_hexpand(true);
        outer.set_vexpand(true);
        window.set_child(Some(&backdrop));
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
            applying_ui: Cell::new(false),
            content,
            pages,
            page_projects: RefCell::default(),
            notice,
            status,
            status_project,
            status_branch,
            sidebar,
            focus_tabs,
            mode: RefCell::new("grid".into()),
            overlay: panel_host,
            panels: Rc::default(),
            registry_dirty: Cell::new(true),
            workspaces: RefCell::default(),
            rendered_sessions: RefCell::default(),
            restored_project: Cell::new(0),
            appearance: gtk::CssProvider::new(),
            wallpaper,
            wallpaper_dim,
            font_size: Cell::new(10.0),
            palette: RefCell::new("matte".into()),
            keybindings: RefCell::new(json!({})),
            sound_busy: Cell::new(false),
            projects_box,
            wall,
            wall_right,
            wall_split,
            wall_files,
            file_tree_revision: Cell::new(0),
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
            launch_busy: Cell::new(false),
            launch_box,
            editor,
            note_tabs: gtk::Notebook::new(),
            note_drafts: RefCell::default(),
        });
        for (name, caption, icon) in [
            ("dashboard", "Dashboard", "view-app-grid-symbolic"),
            ("skills", "Skills", "applications-science-symbolic"),
            ("plugins", "Plugins", "application-x-addon-symbolic"),
            ("code", "Code", "text-x-generic-symbolic"),
            ("board", "Board", "view-list-symbolic"),
            ("plan", "Plan", "document-edit-symbolic"),
            ("notes", "Notes", "accessories-text-editor-symbolic"),
            ("agents", "Agents", "utilities-terminal-symbolic"),
            ("mailbox", "Mailbox", "mail-unread-symbolic"),
            ("guardrails", "Guardrails", "security-high-symbolic"),
        ] {
            let b = nav_button(caption, icon);
            track_navigation(&ui.content, &b, name);
            nav.append(&b);
            let weak = Rc::downgrade(&ui);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.navigate(name);
                }
            });
        }
        for (key, page) in [
            (settings_key, "settings"),
            (devices_key, "devices"),
            (notifications_key, "notifications"),
        ] {
            let weak = Rc::downgrade(&ui);
            key.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.navigate(page);
                }
            });
        }
        let weak = Rc::downgrade(&ui);
        sidebar_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.sidebar.set_visible(!ui.sidebar.is_visible());
                ui.save_layout();
            }
        });
        let weak = Rc::downgrade(&ui);
        palette_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.command_palette();
            }
        });
        let weak = Rc::downgrade(&ui);
        layouts_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.layout_menu();
            }
        });
        let weak = Rc::downgrade(&ui);
        resources_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.resources();
            }
        });
        let weak = Rc::downgrade(&ui);
        files.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let show = !ui.wall_files.is_visible();
                ui.wall_files.set_visible(show);
                if show {
                    ui.load_wall_files(String::new());
                }
            }
        });
        let weak = Rc::downgrade(&ui);
        usage_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.usage();
            }
        });
        ui.install_shortcuts();
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &ui.appearance,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
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
        scrim.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                if !ui.launch_busy.get() {
                    ui.launch.set_reveal_child(false);
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&ui);
        keys.connect_key_pressed(move |_, key, _, _| {
            if let Some(ui) = weak.upgrade() {
                if key == gtk::gdk::Key::Escape {
                    let panel = ui.panels.borrow().last().cloned();
                    if let Some(panel) = panel {
                        panel.close();
                        return glib::Propagation::Stop;
                    }
                }
                if key == gtk::gdk::Key::Escape
                    && ui.launch.reveals_child()
                    && !ui.launch_busy.get()
                {
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
                        let width = ui.wall_split.width();
                        if width >= 560 && ui.wall_right.is_visible() {
                            let position = ui.wall_split.position().clamp(280, width - 280);
                            if position != ui.wall_split.position() {
                                ui.wall_split.set_position(position);
                            }
                        }
                        for p in ui.panes.borrow().values() {
                            p.schedule_resize();
                        }
                    }
                });
            }
        });
        let owned = ui.clone();
        ui.window.connect_close_request(move |_| {
            if owned.launch_busy.get() {
                owned.show_error("Wait for agent launch to finish before closing.");
                return glib::Propagation::Stop;
            }
            if owned
                .note_drafts
                .borrow()
                .values()
                .any(|d| d.busy.get() || d.dirty())
            {
                owned.show_error("Save or discard note changes before closing.");
                return glib::Propagation::Stop;
            }
            if owned.editor.is_dirty() {
                owned.show_error("Save or discard your editor changes before closing.");
                return glib::Propagation::Stop;
            }
            let panels = owned.panels.borrow().clone();
            if panels.iter().any(|panel| !panel.can_close()) {
                owned.show_error("Save or discard panel changes before closing.");
                return glib::Propagation::Stop;
            }
            for panel in panels.iter().rev() {
                panel.close();
            }
            let drafts: Vec<_> = owned.note_drafts.borrow().values().cloned().collect();
            for draft in drafts {
                draft.close();
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
        self.registry_dirty.set(true);
        self.rendered_sessions.borrow_mut().clear();
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
                    if let Err(e)=ui.call("bus.subscribe",json!({"events":["project.changed","project.deleted","workspace.changed","session.changed","task.changed","task.deleted","mailbox.new","mailbox.changed","guardrail.held","guardrail.resolved","overlap.changed","notes.changed","file.changed","git.changed","worktree.changed","module.changed","module.deleted","skill.changed","skill.deleted","settings.changed","notify.new","notify.changed","device.changed","run.changed","run.crash","device.signing.changed","avd.changed","layout.changed","ui.changed","ui.toast","usage.changed","integration.changed","integration.result"]})).await { ui.show_error(&e.to_string()); return; }
                    ui.connected.set(true);
                    ui.notice.set_visible(false);
                    ui.refresh();
                    ui.load_appearance();
                    ui.load_keybindings();
                    if *ui.page.borrow() == "devices" {
                        let _ = ui.call("device.watch", json!({"on":true})).await;
                    }
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
                                if e.ev == "notify.new" {
                                    crate::sounds::notify(&ui, &e.payload);
                                }
                                if e.project_id.is_some_and(|id| id != ui.project.get())
                                    && !e.ev.starts_with("project.")
                                    && !matches!(
                                        ui.page.borrow().as_str(),
                                        "dashboard" | "notifications"
                                    )
                                {
                                    continue;
                                }
                                if e.ev.starts_with("project.") || e.ev.starts_with("workspace.") {
                                    ui.registry_dirty.set(true);
                                    ui.refresh();
                                } else if e.ev.starts_with("session.") {
                                    ui.refresh();
                                } else if matches!(
                                    e.ev.as_str(),
                                    "file.changed"
                                        | "git.changed"
                                        | "worktree.changed"
                                        | "integration.changed"
                                        | "integration.result"
                                ) {
                                    ui.editor.invalidate(&ui);
                                    if ui.wall_files.is_visible() {
                                        ui.load_wall_files(String::new());
                                    }
                                } else if e.ev == "layout.changed"
                                    && e.payload["action"] == "applied"
                                {
                                    ui.apply_layout(&e.payload["state"]);
                                } else if e.ev == "ui.toast" {
                                    ui.show_error(text(&e.payload, "text"));
                                } else if e.ev == "ui.changed" {
                                    ui.apply_ui_event(&e.payload);
                                } else if e.ev == "settings.changed" {
                                    let path = text(&e.payload, "path");
                                    if path.starts_with("keybindings") || path.is_empty() {
                                        ui.load_keybindings();
                                    }
                                    if path.starts_with("appearance.")
                                        || path == "terminal.font_size"
                                        || path.is_empty()
                                    {
                                        ui.load_appearance();
                                    }
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
        if self.content.child_by_name(page).is_none() {
            return;
        }
        if !self.applying_ui.get()
            && self.connected.get()
            && matches!(
                page,
                "agents"
                    | "code"
                    | "board"
                    | "modules"
                    | "plan"
                    | "notes"
                    | "dashboard"
                    | "skills"
                    | "plugins"
                    | "settings"
            )
        {
            let ui = self.clone();
            let page = page.to_string();
            let project = self.project.get();
            glib::spawn_future_local(async move {
                if let Err(e) = ui
                    .call(
                        "ui.page.switch",
                        json!({"page":page,"project_id":if project>0 {Some(project)} else {None}}),
                    )
                    .await
                {
                    ui.show_error(&e.to_string());
                }
            });
        }
        let previous = self.page.borrow().clone();
        if (previous == "devices") != (page == "devices") && self.connected.get() {
            let ui = self.clone();
            let on = page == "devices";
            glib::spawn_future_local(async move {
                if let Err(e) = ui.call("device.watch", json!({"on":on})).await {
                    ui.show_error(&e.to_string());
                }
            });
        }
        *self.page.borrow_mut() = page.into();
        self.content.set_visible_child_name(page);
        self.save_layout();
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
                if ui.registry_dirty.replace(false) {
                    let (projects, workspaces) = tokio::join!(
                        ui.call("project.list", json!({})),
                        ui.call("workspace.list", json!({}))
                    );
                    if generation != ui.generation.get() {
                        continue;
                    }
                    match (projects, workspaces) {
                        (Ok(p), Ok(w)) => {
                            *ui.projects.borrow_mut() = rows(&p, "projects");
                            *ui.workspaces.borrow_mut() = rows(&w, "workspaces");
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            ui.show_error(&e.to_string());
                            break;
                        }
                    }
                }
                if !ui
                    .projects
                    .borrow()
                    .iter()
                    .any(|p| p["id"].as_i64() == Some(ui.project.get()))
                {
                    if ui.editor.is_dirty() {
                        ui.show_error("The selected project was removed. Save or copy your editor changes before selecting another project.");
                        break;
                    }
                    ui.project.set(
                        ui.projects
                            .borrow()
                            .first()
                            .and_then(|p| p["id"].as_i64())
                            .unwrap_or(0),
                    );
                    ui.editor.reset();
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
                ui.restore_layout().await;
                if matches!(ui.page.borrow().as_str(), "board" | "dashboard") {
                    ui.refresh_page();
                }
            }
            ui.refresh_pending.set(false);
        });
    }
    fn reconcile(self: &Rc<Self>) {
        let sessions = self.sessions.borrow().clone();
        let available: Vec<String> = sessions.iter().map(|s| text(s, "name").into()).collect();
        let mut names = self.ordered.borrow().clone();
        names.retain(|n| available.contains(n));
        for n in available {
            if !names.contains(&n) {
                names.push(n);
            }
        }
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
                if let Some(grid) = p.root.parent().and_downcast::<gtk::Grid>() {
                    grid.remove(&p.root);
                }
            }
        }
        for s in &sessions {
            let name = text(s, "name");
            if !self.panes.borrow().contains_key(name) {
                let pane = Pane::new(name, self.path.clone(), self.rt.clone());
                self.install_pane_controls(&pane, name);
                pane.apply_appearance(&self.palette.borrow(), self.font_size.get());
                self.panes.borrow_mut().insert(name.into(), pane);
            }
            let pane = self.panes.borrow().get(name).cloned().unwrap();
            let signature = json!({"state":s["state"], "role":s["role"], "provider":s["provider"], "branch":s["branch"], "intent":s["intent"], "pair_with":s["pair_with"]});
            if self.rendered_sessions.borrow().get(name) != Some(&signature) {
                pane.update_session(s);
                self.session_actions(&pane, s);
                self.rendered_sessions
                    .borrow_mut()
                    .insert(name.into(), signature);
            }
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
        self.status
            .set_text(&format!("{held} needs you  ·  {live} live"));
        self.wall_stack
            .set_visible_child_name(if sessions.is_empty() { "empty" } else { "wall" });
    }
    fn update_attachments(&self) {
        let focus = self.focused.borrow();
        let mode = self.mode.borrow();
        for s in self.sessions.borrow().iter() {
            if let Some(p) = self.panes.borrow().get(text(s, "name")) {
                let active = (*self.page.borrow() == "agents"
                    && (mode.as_str() != "focus"
                        || focus.as_ref().is_none_or(|n| n == text(s, "name"))))
                    && !matches!(
                        text(s, "state"),
                        "created" | "parked" | "restorable" | "exited"
                    );
                p.set_active(active);
            }
        }
    }
    pub fn verify_shell(self: &Rc<Self>) {
        let panes = self.panes.borrow().clone();
        let mode = self.mode.borrow().clone();
        for mode in ["grid", "focus", "review"] {
            self.set_mode(mode);
            for (name, pane) in &panes {
                assert!(
                    Rc::ptr_eq(pane, &self.panes.borrow()[name]),
                    "Layout replaced a terminal"
                );
            }
        }
        self.set_mode(&mode);
        println!("Shell layouts verified: grid, focus, review, retained terminals");
    }
    pub fn verify_launch(&self) {
        assert!(self.launch.reveals_child());
        assert!(
            self.launch.width() >= 400 && self.launch_box.width() >= 350,
            "Launch form must be visible"
        );
        assert!(
            !self.sidebar.is_sensitive(),
            "The launch sheet must block background actions"
        );
    }
    pub fn verify_terminal_input(&self) {
        use vte4::prelude::TerminalExt;
        for (name, pane) in self.panes.borrow().iter() {
            pane.terminal
                .paste_text(&format!("native-paste-check-{name}\n"));
        }
    }
    pub fn verify_burst(&self, check: bool) {
        use vte4::prelude::TerminalExt;
        assert_eq!(self.panes.borrow().len(), 11);
        for (name, pane) in self.panes.borrow().iter() {
            if check {
                let (_, row) = pane.terminal.cursor_position();
                let (tail, _) = pane.terminal.text_range_format(
                    vte4::Format::Text,
                    (row - 10).max(0),
                    0,
                    row,
                    200,
                );
                assert!(
                    tail.unwrap_or_default()
                        .contains(&format!("BURST-END-{name}")),
                    "No native tail marker for {name}"
                );
            } else {
                pane.verify_ready();
                pane.terminal.paste_text("native-burst\n");
            }
        }
        if check {
            println!("BURST_RENDERED=11");
        }
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
                if matches!(page.as_str(), "agents" | "code") {
                    continue;
                }
                if matches!(
                    page.as_str(),
                    "dashboard" | "settings" | "skills" | "plugins" | "notifications" | "devices"
                ) {
                    crate::tools::refresh(&ui, &page, project).await;
                } else if project != 0 {
                    crate::pages::refresh(&ui, &page, project).await;
                }
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
}
