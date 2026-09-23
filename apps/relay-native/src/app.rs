use crate::client::{Client, Error, Notice};
use crate::terminal::Pane;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use tokio::runtime::Handle;
#[path = "launch.rs"]
mod launch;
#[path = "onboarding.rs"]
mod onboarding;
#[path = "shell.rs"]
mod shell;
#[path = "status.rs"]
mod status;

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
    if !class.is_empty() {
        b.add_css_class(class);
    }
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
    let image = crate::icons::image(icon, 16);
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
    sidebar_sessions: RefCell<Vec<Value>>,
    pub projects: RefCell<Vec<Value>>,
    pub page: RefCell<String>,
    /// Whether the compositor still shows this window. A terminal that nobody can see does not
    /// need its output: the engine keeps the scrollback ring either way, so detaching costs
    /// nothing and saves the whole per-frame chain — socket read, parse, VTE feed and redraw —
    /// for as long as the window is minimized or covered.
    window_visible: Cell<bool>,
    applying_ui: Cell<bool>,
    navigation_pending: RefCell<Option<(i64, String)>>,
    navigation_sending: Cell<bool>,
    navigation_echoes: RefCell<BTreeSet<uuid::Uuid>>,
    pub content: gtk::Stack,
    pub pages: BTreeMap<String, gtk::Box>,
    pub page_projects: RefCell<BTreeMap<String, i64>>,
    pub notice: gtk::Label,
    status: gtk::Label,
    usage_meters: gtk::Box,
    device_status: gtk::Label,
    pub(super) resource_status: gtk::Box,
    notification_count: gtk::Label,
    notification_revision: Cell<u64>,
    setup_checked: Cell<bool>,
    status_project: gtk::Label,
    status_branch: gtk::Label,
    pub(crate) sidebar: gtk::Box,
    settings_sidebar: Cell<bool>,
    focus_tabs: gtk::Box,
    mode: RefCell<String>,
    pub overlay: gtk::Overlay,
    pub page_overlay: gtk::Overlay,
    pub panels: Rc<RefCell<Vec<Rc<crate::panel::Panel>>>>,
    registry_dirty: Cell<bool>,
    pub(crate) workspaces: RefCell<Vec<Value>>,
    rendered_sessions: RefCell<BTreeMap<String, Value>>,
    restored_project: Cell<i64>,
    layout_revision: Cell<u64>,
    layout_saves: RefCell<BTreeMap<i64, Value>>,
    layout_saving: Cell<bool>,
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
    wall_stack: gtk::Stack,
    empty_title: gtk::Label,
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
    launch_caption: gtk::Label,
    launch_busy: Cell<bool>,
    launch_box: gtk::Box,
    pub editor: Rc<crate::editor::Editor>,
    pub note_tabs: gtk::Notebook,
    pub note_drafts: RefCell<BTreeMap<i64, Rc<crate::pages::Draft>>>,
    pub notes_window: RefCell<Option<Rc<crate::pages::NotesWindow>>>,
    pub(crate) wallpaper_rotation: RefCell<crate::wallpaper_rotation::Rotation>,
    pub(crate) wallpaper_draft: RefCell<Option<crate::tools::settings::WallpaperDraft>>,
    pub(crate) provider_updates_checked: Cell<bool>,
    /// Status-bar refreshes coalesce like `refresh`; `status_devices` asks the next pass to
    /// run `device.list` too (an `adb` process), which only device events and the first load do.
    status_pending: Cell<bool>,
    status_dirty: Cell<bool>,
    status_devices: Cell<bool>,
    /// Per-project lamp and live count in the sidebar, updated in place on session changes.
    project_live: RefCell<BTreeMap<i64, (gtk::Box, gtk::Label)>>,
    reconnect_timer: RefCell<Option<glib::SourceId>>,
    reconnect_attempt: Cell<u32>,
    /// Hash of the wallpaper value last decoded, so an unchanged image is not decoded again.
    wallpaper_key: Cell<u64>,
    /// Palette and font size last applied to the panes.
    pane_style: RefCell<(String, f64)>,
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
        ui.track_window_visibility();
        ui.window.present();
        crate::pages::show_notes(&ui);
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
        crate::fonts::install(&window);
        crate::icons::install_app_icon(&window);
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        top.add_css_class("topbar");
        let sidebar_key = icon_button("sidebar-show-symbolic", "Toggle sidebar");
        sidebar_key.set_widget_name("sidebar-toggle");
        let top_left = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top_left.add_css_class("topbar-left");
        top_left.set_size_request(194, -1);
        sidebar_key.set_child(Some(&crate::icons::image("sidebar", 14)));
        top_left.append(&sidebar_key);
        let mark = label("R", "brand-mark");
        mark.set_valign(gtk::Align::Center);
        mark.set_xalign(0.5);
        top_left.append(&mark);
        let brand = label("RELAY", "brand");
        top_left.append(&brand);
        top.append(&top_left);
        let top_space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        top_space.set_hexpand(true);
        top.append(&top_space);
        let top_actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        top_actions.add_css_class("topbar-actions");
        top.append(&top_actions);
        let palette_key = icon_button("system-search-symbolic", "Command palette · Ctrl K");
        palette_key.set_widget_name("command-palette");
        let layouts_key = icon_button("view-grid-symbolic", "Window presets");
        layouts_key.set_widget_name("window-presets");
        let skills_key = icon_button("skills", "Skills for this project");
        let notifications_key = icon_button("alarm-symbolic", "Notifications");
        let bell = gtk::Overlay::new();
        bell.set_child(Some(&crate::icons::image("bell", 16)));
        let notification_count = label("", "notification-count");
        notification_count.set_halign(gtk::Align::End);
        notification_count.set_valign(gtk::Align::Start);
        notification_count.set_visible(false);
        notification_count.set_can_target(false);
        bell.add_overlay(&notification_count);
        notifications_key.set_child(Some(&bell));
        for key in [&palette_key, &skills_key, &layouts_key, &notifications_key] {
            top_actions.append(key);
        }
        let reconnect = icon_button("view-refresh-symbolic", "Reconnect to engine");
        top_actions.append(&reconnect);
        reconnect.set_visible(false);
        let launch_key = button("New session", "primary");
        launch_key.set_widget_name("new-session");
        let launch_label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        launch_label.append(&crate::icons::image_with_stroke("plus", 14, 2.0));
        let launch_caption = label("New session", "");
        launch_label.append(&launch_caption);
        launch_key.set_child(Some(&launch_label));
        launch_key.set_valign(gtk::Align::Center);
        launch_key.add_css_class("launch-key");
        top_actions.append(&launch_key);
        let window_controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        window_controls.add_css_class("window-controls");
        for (icon, caption, size) in [
            ("minimize", "Minimize", 14),
            ("maximize", "Maximize", 12),
            ("close", "Close", 14),
        ] {
            let key = icon_button(icon, caption);
            key.remove_css_class("icon-key");
            key.add_css_class("window-control");
            key.set_margin_top(0);
            key.set_margin_bottom(0);
            key.set_child(Some(&crate::icons::image(icon, size)));
            if icon == "close" {
                key.add_css_class("window-close");
            }
            let window = window.downgrade();
            key.connect_clicked(move |_| {
                if let Some(window) = window.upgrade() {
                    match icon {
                        "minimize" => window.minimize(),
                        "maximize" if window.is_maximized() => window.unmaximize(),
                        "maximize" => window.maximize(),
                        _ => window.close(),
                    }
                }
            });
            window_controls.append(&key);
        }
        top.append(&window_controls);
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
        let sidebar_footer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_footer.add_css_class("sidebar-footer");
        sidebar_footer.append(&settings_key);
        sidebar.append(&sidebar_footer);
        body.append(&sidebar);
        let content = gtk::Stack::new();
        content.set_hhomogeneous(false);
        content.set_vhomogeneous(false);
        content.set_hexpand(true);
        content.set_vexpand(true);
        let page_overlay = gtk::Overlay::new();
        page_overlay.set_child(Some(&content));
        body.append(&page_overlay);
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
        wall_split.add_css_class("wall-split");
        wall_split.set_start_child(Some(&wall));
        wall_split.set_end_child(Some(&wall_right));
        wall_split.set_position(600);
        wall_split.set_shrink_start_child(false);
        wall_split.set_shrink_end_child(false);
        wall.set_size_request(280, -1);
        wall_right.set_size_request(280, -1);
        let wall_scroll = scrolled(&wall_split);
        // Files and Git can leave less room than the terminal panes' minimum.
        // Keep their content reachable instead of allocating below that minimum.
        wall_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        wall_scroll.set_overlay_scrolling(false);
        wall_stack.add_named(&wall_scroll, Some("wall"));
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
        empty.add_css_class("wall-empty");
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        let empty_glyph = crate::icons::image_with_stroke("grid", 28, 1.0);
        empty_glyph.add_css_class("empty-glyph");
        empty.append(&empty_glyph);
        let empty_title = label("No sessions", "title");
        empty_title.set_halign(gtk::Align::Center);
        empty.append(&empty_title);
        let empty_description = label(
            "Launch a solo agent in its own worktree, or a builder–reviewer pair on one branch. Each session lands here as a tile.",
            "empty-description",
        );
        empty_description.set_wrap(true);
        empty_description.set_max_width_chars(52);
        empty_description.set_justify(gtk::Justification::Center);
        empty.append(&empty_description);
        let empty_launch = button("", "primary");
        let empty_action = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        empty_action.append(&crate::icons::image_with_stroke("plus", 14, 2.0));
        empty_action.append(&label("Session", ""));
        empty_launch.set_child(Some(&empty_action));
        empty_launch.set_halign(gtk::Align::Center);
        empty.append(&empty_launch);
        wall_stack.add_named(&empty, Some("empty"));
        wall_stack.set_visible_child_name("empty");
        let agents = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let focus_tabs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        focus_tabs.add_css_class("focus-tabs");
        agents.append(&focus_tabs);
        let wall_body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        wall_body.append(&wall_stack);
        agents.append(&wall_body);
        let editor = crate::editor::Editor::new();
        editor.mount_agents(&agents);
        content.add_named(&editor.root, Some("agents"));
        let mut pages = BTreeMap::new();
        for name in [
            "board",
            "modules",
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
            if name == "notes" {
                // The Notes workspace lives in its own retained native window.
            } else if name == "board" {
                content.add_named(&page, Some(name));
            } else {
                content.add_named(&scrolled(&page), Some(name));
            }
            pages.insert(name.into(), page);
        }
        let launch_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        launch_box.add_css_class("launch");
        launch_box.set_size_request(780, -1);
        let launch = gtk::Revealer::new();
        // Its fixed-width allocation remains above the page when collapsed.
        launch.set_can_target(false);
        launch.set_transition_duration(0);
        launch.set_hexpand(false);
        launch.set_child(Some(&launch_box));
        launch.set_halign(gtk::Align::End);
        launch.set_size_request(780, -1);
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
            launch.set_can_target(launch.reveals_child());
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
        bottom.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        bottom.append(&label("linux · local", "status-platform"));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        bottom.append(&spacer);
        let status = label("Engine disconnected", "");
        bottom.append(&status);
        let usage_key = button("", "quiet");
        usage_key.set_tooltip_text(Some("Provider usage"));
        usage_key.set_widget_name("status-usage");
        let usage_meters = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        usage_key.set_child(Some(&usage_meters));
        let devices_key = button("", "quiet");
        devices_key.set_widget_name("status-devices");
        let device_status = label("No device", "mono");
        let device_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        device_content.append(&crate::icons::image("device", 12));
        device_content.append(&device_status);
        devices_key.set_child(Some(&device_content));
        let resources_key = icon_button("cpu", "Resources");
        resources_key.set_widget_name("status-resources");
        let resource_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        resource_content.append(&crate::icons::image("cpu", 12));
        let resource_status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        resource_content.append(&resource_status);
        resources_key.set_child(Some(&resource_content));
        bottom.append(&usage_key);
        bottom.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        bottom.append(&devices_key);
        bottom.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        bottom.append(&resources_key);
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
            sidebar_sessions: RefCell::default(),
            projects: RefCell::default(),
            page: RefCell::new("agents".into()),
            window_visible: Cell::new(true),
            applying_ui: Cell::new(false),
            navigation_pending: RefCell::default(),
            navigation_sending: Cell::new(false),
            navigation_echoes: RefCell::default(),
            content,
            pages,
            page_projects: RefCell::default(),
            notice,
            status,
            usage_meters,
            device_status,
            resource_status,
            notification_count,
            notification_revision: Cell::new(0),
            setup_checked: Cell::new(false),
            status_project,
            status_branch,
            sidebar,
            settings_sidebar: Cell::new(true),
            focus_tabs,
            mode: RefCell::new("grid".into()),
            overlay: panel_host,
            page_overlay,
            panels: Rc::default(),
            registry_dirty: Cell::new(true),
            workspaces: RefCell::default(),
            rendered_sessions: RefCell::default(),
            restored_project: Cell::new(0),
            layout_revision: Cell::new(0),
            layout_saves: RefCell::default(),
            layout_saving: Cell::new(false),
            appearance: gtk::CssProvider::new(),
            wallpaper,
            wallpaper_dim,
            font_size: Cell::new(9.75),
            palette: RefCell::new("matte".into()),
            keybindings: RefCell::new(json!({})),
            sound_busy: Cell::new(false),
            projects_box,
            wall,
            wall_right,
            wall_split,
            wall_stack,
            empty_title,
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
            launch_caption,
            launch_busy: Cell::new(false),
            launch_box,
            editor,
            note_tabs: gtk::Notebook::new(),
            note_drafts: RefCell::default(),
            notes_window: RefCell::default(),
            wallpaper_rotation: RefCell::default(),
            wallpaper_draft: RefCell::default(),
            provider_updates_checked: Cell::new(false),
            status_pending: Cell::new(false),
            status_dirty: Cell::new(false),
            status_devices: Cell::new(false),
            project_live: RefCell::default(),
            reconnect_timer: RefCell::new(None),
            reconnect_attempt: Cell::new(0),
            wallpaper_key: Cell::new(0),
            pane_style: RefCell::new((String::new(), 0.0)),
        });
        for (name, caption, icon) in [
            ("dashboard", "Dashboard", "view-app-grid-symbolic"),
            ("skills", "Skills", "applications-science-symbolic"),
            ("plugins", "Plugins", "application-x-addon-symbolic"),
            ("board", "Board", "view-list-symbolic"),
            ("notes", "Notes", "accessories-text-editor-symbolic"),
        ] {
            let b = nav_button(caption, icon);
            b.set_widget_name(&format!("nav-{name}"));
            track_navigation(&ui.content, &b, name);
            nav.append(&b);
            let weak = Rc::downgrade(&ui);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.toggle_page(name);
                }
            });
        }
        for (key, page) in [
            (settings_key, "settings"),
            (notifications_key, "notifications"),
        ] {
            let weak = Rc::downgrade(&ui);
            key.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.toggle_page(page);
                }
            });
        }
        let weak = Rc::downgrade(&ui);
        skills_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.project_skills();
            }
        });
        let settings_return = Rc::new(RefCell::new((String::from("agents"), true)));
        let return_state = settings_return.clone();
        let weak = Rc::downgrade(&ui);
        let back_key = sidebar_key.clone();
        let mut_previous = Rc::new(RefCell::new(String::from("agents")));
        ui.content.connect_visible_child_name_notify(move |stack| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let page = stack.visible_child_name().unwrap_or_default();
            let settings = page == "settings";
            if settings && mut_previous.borrow().as_str() != "settings" {
                ui.settings_sidebar.set(ui.sidebar.is_visible());
                *return_state.borrow_mut() =
                    (mut_previous.borrow().clone(), ui.sidebar.is_visible());
                ui.sidebar.set_visible(false);
            } else if !settings && mut_previous.borrow().as_str() == "settings" {
                ui.sidebar.set_visible(ui.settings_sidebar.get());
            }
            brand.set_text(if settings { "SETTINGS" } else { "RELAY" });
            back_key.set_child(Some(&crate::icons::image(
                if settings { "chevron-left" } else { "sidebar" },
                14,
            )));
            back_key.set_tooltip_text(Some(if settings {
                "Back from settings"
            } else {
                "Toggle sidebar"
            }));
            top_actions.set_visible(!settings);
            bottom.set_visible(!settings);
            skills_key.set_visible(page == "agents");
            *mut_previous.borrow_mut() = page.into();
        });
        let weak = Rc::downgrade(&ui);
        sidebar_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                if ui.page.borrow().as_str() == "settings" {
                    let page = settings_return.borrow().0.clone();
                    ui.navigate(&page);
                } else {
                    ui.sidebar.set_visible(!ui.sidebar.is_visible());
                    ui.save_layout();
                }
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
        devices_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                crate::tools::devices::open(&ui);
            }
        });
        let weak = Rc::downgrade(&ui);
        resources_key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.resources();
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.editor.connect_view_changed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.update_attachments();
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
                    if ui.launch.reveals_child() && !ui.launch_busy.get() {
                        ui.launch.set_reveal_child(false);
                    } else {
                        ui.show_launch(None);
                    }
                }
            });
        }
        let weak = Rc::downgrade(&ui);
        // Information (`show_info`) carries the "info" class and never offers Reconnect.
        let recovery_label_key = reconnect.clone();
        ui.notice.connect_label_notify(move |notice| {
            recovery_label_key
                .set_visible(!notice.text().is_empty() && !notice.has_css_class("info"));
        });
        let recovery_key = reconnect.clone();
        ui.notice.connect_visible_notify(move |notice| {
            recovery_key.set_visible(notice.is_visible() && !notice.has_css_class("info"))
        });
        reconnect.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.reconnect_attempt.set(0);
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
                owned.show_info("Wait for agent launch to finish before closing.");
                return glib::Propagation::Stop;
            }
            if owned
                .note_drafts
                .borrow()
                .values()
                .any(|d| d.busy.get() || d.dirty())
            {
                owned.show_info("Save or discard note changes before closing.");
                return glib::Propagation::Stop;
            }
            if owned.editor.is_dirty() {
                owned.show_info("Save or discard your editor changes before closing.");
                return glib::Propagation::Stop;
            }
            let panels = owned.panels.borrow().clone();
            if panels.iter().any(|panel| !panel.can_close()) {
                owned.show_info("Save or discard panel changes before closing.");
                return glib::Propagation::Stop;
            }
            for panel in panels.iter().rev() {
                panel.close();
            }
            let drafts: Vec<_> = owned.note_drafts.borrow().values().cloned().collect();
            for draft in drafts {
                draft.close();
            }
            if let Some(notes) = owned.notes_window.borrow_mut().take() {
                notes.window.destroy();
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
        self.notice.remove_css_class("info");
        self.notice.set_text(message);
        self.notice.set_visible(true);
    }
    /// A message that needs no recovery: no Reconnect key, and it clears itself.
    pub fn show_info(&self, message: &str) {
        self.notice_info(message, true);
    }
    fn notice_info(&self, message: &str, expire: bool) {
        self.notice.add_css_class("info");
        self.notice.set_text(message);
        self.notice.set_visible(true);
        if !expire {
            return;
        }
        let notice = self.notice.downgrade();
        let shown = message.to_string();
        glib::timeout_add_local_once(std::time::Duration::from_secs(8), move || {
            if let Some(notice) = notice.upgrade() {
                if notice.has_css_class("info") && notice.text() == shown {
                    notice.set_visible(false);
                }
            }
        });
    }
    pub async fn call(&self, op: &str, payload: Value) -> Result<Value, Error> {
        let client = self.client.borrow().clone().ok_or(Error::Disconnected)?;
        if crate::client::is_lifecycle_request(op) {
            return Client::lifecycle_request(&self.rt, self.path.clone(), op, payload).await;
        }
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
    /// After the engine goes away, try again on our own: 1 s, 2 s, 4 s … up to 30 s. The
    /// Reconnect key stays available and resets the delay.
    fn schedule_reconnect(self: &Rc<Self>) {
        if let Some(timer) = self.reconnect_timer.borrow_mut().take() {
            timer.remove();
        }
        let attempt = self.reconnect_attempt.get();
        self.reconnect_attempt.set(attempt.saturating_add(1));
        let delay = (1u64 << attempt.min(5)).min(30);
        let weak = Rc::downgrade(self);
        *self.reconnect_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_secs(delay),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.reconnect_timer.borrow_mut().take();
                    if !ui.connected.get() {
                        ui.connect();
                    }
                }
            },
        ));
    }
    fn connect(self: &Rc<Self>) {
        if let Some(timer) = self.reconnect_timer.borrow_mut().take() {
            timer.remove();
        }
        self.navigation_echoes.borrow_mut().clear();
        self.registry_dirty.set(true);
        self.rendered_sessions.borrow_mut().clear();
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        self.client.borrow_mut().take();
        self.connected.set(false);
        // Keep the panes: each owns its own attachment and remembers the last sequence it
        // fed, so re-attaching resumes the output instead of replaying only the engine's
        // bounded tail into a fresh terminal. `reconcile` drops panes whose session is gone.
        for p in self.panes.borrow().values() {
            p.set_active(false);
        }
        crate::pages::mark_notes_stale(self);
        self.notice_info("Connecting to the Relay engine…", false);
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match Client::connect(&ui.rt, ui.path.clone()).await {
                Ok((client, notices)) => {
                    if generation != ui.generation.get() {
                        return;
                    }
                    *ui.client.borrow_mut() = Some(client);
                    if let Err(e)=ui.call("bus.subscribe",json!({"events":["project.changed","project.deleted","workspace.changed","session.changed","task.changed","task.deleted","mailbox.new","mailbox.changed","guardrail.held","guardrail.resolved","overlap.changed","notes.changed","notes.deleted","file.changed","git.changed","worktree.changed","module.changed","module.deleted","skill.changed","skill.deleted","plugin.changed","settings.changed","provider.update.changed","notify.new","notify.changed","device.changed","run.changed","run.crash","device.signing.changed","avd.changed","layout.changed","ui.changed","ui.toast","usage.changed","integration.changed","integration.result"]})).await { ui.show_error(&e.to_string()); ui.client.borrow_mut().take(); ui.schedule_reconnect(); return; }
                    ui.connected.set(true);
                    ui.reconnect_attempt.set(0);
                    crate::provider_updates::startup(&ui);
                    ui.status.set_text("");
                    ui.refresh_status(true);
                    ui.refresh_notification_count();
                    ui.notice.set_visible(false);
                    ui.refresh();
                    ui.load_appearance("");
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
                                // Local navigation already updated the widgets. Consume its
                                // echo before project filtering, including a project just left.
                                if e.ev == "ui.changed"
                                    && e.cause.is_some_and(|id| {
                                        ui.navigation_echoes.borrow_mut().remove(&id)
                                    })
                                {
                                    continue;
                                }
                                if matches!(
                                    e.ev.as_str(),
                                    "usage.changed" | "device.changed" | "run.changed"
                                ) {
                                    ui.refresh_status(e.ev == "device.changed");
                                }
                                if e.ev == "notify.new" {
                                    crate::sounds::notify(&ui, &e.payload);
                                }
                                if e.ev.starts_with("notify.") {
                                    ui.refresh_notification_count();
                                }
                                if e.ev == "provider.update.changed" {
                                    crate::provider_updates::event(&ui, &e.payload);
                                    continue;
                                }
                                // Notes retains its own project when the main window switches.
                                if matches!(e.ev.as_str(), "notes.changed" | "notes.deleted") {
                                    crate::pages::refresh_notes(&ui);
                                    continue;
                                }
                                // The Skills and Plugins pages switch every project, so they
                                // need enablement changes for projects other than the current.
                                let catalog = matches!(ui.page.borrow().as_str(), "skills" | "plugins")
                                    && (e.ev.starts_with("skill.") || e.ev.starts_with("plugin."));
                                if e.project_id.is_some_and(|id| id != ui.project.get())
                                    && !e.ev.starts_with("project.")
                                    && !e.ev.starts_with("session.")
                                    && !catalog
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
                                    ui.editor.invalidate_for(&ui, &e.ev, &e.payload);
                                } else if e.ev == "layout.changed"
                                    && e.payload["action"] == "applied"
                                {
                                    ui.apply_layout(&e.payload["state"]);
                                } else if e.ev == "ui.toast" {
                                    ui.show_info(text(&e.payload, "text"));
                                } else if e.ev == "ui.changed" {
                                    ui.apply_ui_event(&e.payload);
                                } else if e.ev == "settings.changed" {
                                    let path = text(&e.payload, "path");
                                    if path.starts_with("keybindings") || path.is_empty() {
                                        ui.load_keybindings();
                                    }
                                    if path.starts_with("appearance")
                                        || path.starts_with("terminal")
                                        || path.is_empty()
                                    {
                                        ui.load_appearance(path);
                                    }
                                    crate::tools::settings::external_change(
                                        &ui,
                                        path,
                                        &e.payload["value"],
                                    );
                                    if *ui.page.borrow() == "plugins"
                                        && (path.is_empty() || path == "mcp" || path.starts_with("mcp."))
                                    {
                                        ui.refresh_page();
                                    }
                                } else if *ui.page.borrow() == "plugins"
                                    && !e.ev.starts_with("plugin.")
                                {
                                    // Nothing else changes what the Plugins page shows.
                                } else {
                                    ui.refresh_page();
                                }
                            }
                            Notice::Disconnected(e) => {
                                ui.navigation_echoes.borrow_mut().clear();
                                ui.connected.set(false);
                                ui.client.borrow_mut().take();
                                ui.status.set_text(
                                    "Engine disconnected · sessions remain owned by the engine",
                                );
                                ui.show_error(&format!("Engine connection lost: {e} Reconnecting automatically…"));
                                ui.schedule_reconnect();
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    if generation != ui.generation.get() {
                        return;
                    }
                    ui.show_error(&format!(
                        "{e}. Start relay serve for this instance; Relay keeps trying to reconnect."
                    ));
                    ui.schedule_reconnect();
                }
            }
        });
    }
    pub fn dismiss_panels(&self) -> bool {
        let panels = self.panels.borrow().clone();
        if panels.iter().any(|panel| !panel.can_close()) {
            self.show_info("Save or discard your changes before leaving this page.");
            return false;
        }
        for panel in panels.iter().rev() {
            panel.close();
        }
        true
    }
    pub fn toggle_page(self: &Rc<Self>, page: &str) {
        let active = *self.page.borrow() == page;
        self.navigate(if active { "agents" } else { page });
    }
    pub fn navigate(self: &Rc<Self>, page: &str) {
        if page == "notes" {
            crate::pages::show_notes(self);
            return;
        }
        let files = page == "code";
        let page = if files { "agents" } else { page };
        if *self.page.borrow() != page && !self.dismiss_panels() {
            return;
        }
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
                    | "dashboard"
                    | "skills"
                    | "plugins"
                    | "settings"
            )
        {
            self.sync_navigation(page);
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
        if page == "agents" {
            self.editor.prepare_project(self);
            if files {
                self.editor.show_files();
            } else {
                self.editor.show_agents();
            }
        }
        self.save_layout();
        self.layout();
        self.refresh_page();
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
                let layout_revision = ui.layout_revision.get();
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
                        ui.show_info("The selected project was removed. Save or copy your editor changes before selecting another project.");
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
                    ui.editor.prepare_project(&ui);
                }
                // Structure only when the registry changed; live counts update in place.
                ui.render_projects();
                if !ui.setup_checked.replace(true) && ui.projects.borrow().is_empty() {
                    ui.open_repository();
                }
                let project = ui.project.get();
                if project == 0 {
                    ui.sessions.borrow_mut().clear();
                    ui.reconcile();
                    continue;
                }
                match ui
                    .call("session.list", json!({"include_closed":false}))
                    .await
                {
                    Ok(v) if ui.project.get() == project && generation == ui.generation.get() => {
                        let all = rows(&v, "sessions");
                        *ui.sessions.borrow_mut() = all
                            .iter()
                            .filter(|s| s["project_id"].as_i64() == Some(project))
                            .cloned()
                            .collect();
                        *ui.sidebar_sessions.borrow_mut() = all;
                        ui.update_project_live();
                        ui.reconcile();
                    }
                    Ok(_) => {}
                    Err(e) => ui.show_error(&e.to_string()),
                }
                ui.restore_layout(layout_revision).await;
                // Notes follow notes.* events; this only covers its first load and a reconnect.
                crate::pages::catch_up_notes(&ui);
                if matches!(ui.page.borrow().as_str(), "board" | "dashboard") {
                    ui.refresh_page();
                }
            }
            ui.refresh_pending.set(false);
        });
    }
    fn reconcile(self: &Rc<Self>) {
        self.render_status_counts();
        let sessions = self.sessions.borrow().clone();
        self.launch_caption.set_text(if sessions.is_empty() {
            "New session"
        } else {
            "Add agents"
        });
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
            self.rendered_sessions.borrow_mut().remove(&n);
            if let Some(p) = self.panes.borrow_mut().remove(&n) {
                p.stop();
                if let Some(grid) = p.root.parent().and_downcast::<gtk::Grid>() {
                    grid.remove(&p.root);
                }
            }
        }
        for s in &sessions {
            let name = text(s, "name");
            let new_pane = !self.panes.borrow().contains_key(name);
            if new_pane {
                let pane = Pane::new(name, self.path.clone(), self.rt.clone());
                self.install_pane_controls(&pane, name);
                pane.apply_appearance(&self.palette.borrow(), self.font_size.get());
                self.panes.borrow_mut().insert(name.into(), pane);
            }
            let pane = self.panes.borrow().get(name).cloned().unwrap();
            // Everything the pane and its session menu show, including the menu's switches.
            let signature = json!({"state":s["state"], "role":s["role"], "provider":s["provider"], "branch":s["branch"], "worktree":s["worktree"], "intent":s["intent"], "pair_with":s["pair_with"], "bus_writes":s["bus_writes"], "allow_ui":s["allow_ui"], "model":s["model"], "effort":s["effort"], "spawned_at":s["spawned_at"]});
            // Render signatures describe widgets, not just sessions. A recreated pane has
            // an empty header and visible slate even when the session itself is unchanged.
            if new_pane || self.rendered_sessions.borrow().get(name) != Some(&signature) {
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
        let name = self
            .projects
            .borrow()
            .iter()
            .find(|p| p["id"].as_i64() == Some(self.project.get()))
            .map(|p| text(p, "name").to_string())
            .unwrap_or_else(|| "No project".into());
        self.status_project.set_text(&name);
        self.empty_title.set_text(&format!("No sessions in {name}"));
        self.status_branch.set_text(
            self.projects
                .borrow()
                .iter()
                .find(|p| p["id"].as_i64() == Some(self.project.get()))
                .map(|p| text(p, "base_branch"))
                .unwrap_or(""),
        );

        self.wall_stack
            .set_visible_child_name(if sessions.is_empty() { "empty" } else { "wall" });
    }
    /// Follow the compositor's own view of whether this window is on screen.
    ///
    /// `SUSPENDED` is the compositor saying the surface is not visible — minimized, fully
    /// covered, or on another workspace — and `MINIMIZED` covers the backends that do not send
    /// it. Either way the terminals detach, and re-attach from their last sequence with bounded
    /// catch-up when the window comes back, which is the same path a reconnect already takes.
    /// The surface only exists once the window is realized, so this is installed from there.
    pub fn track_window_visibility(self: &Rc<Self>) {
        let ui = self.clone();
        self.window.connect_realize(move |window| {
            let Some(toplevel) = window
                .surface()
                .and_then(|surface| surface.downcast::<gtk::gdk::Toplevel>().ok())
            else {
                return;
            };
            let hidden = |state: gtk::gdk::ToplevelState| {
                state.contains(gtk::gdk::ToplevelState::MINIMIZED)
                    || state.contains(gtk::gdk::ToplevelState::SUSPENDED)
            };
            ui.window_visible.set(!hidden(toplevel.state()));
            ui.update_attachments();
            let weak = Rc::downgrade(&ui);
            toplevel.connect_state_notify(move |toplevel| {
                let Some(ui) = weak.upgrade() else { return };
                let visible = !hidden(toplevel.state());
                if ui.window_visible.replace(visible) != visible {
                    ui.update_attachments();
                }
            });
        });
    }
    fn update_attachments(&self) {
        let focus = self.focused.borrow();
        let mode = self.mode.borrow();
        for s in self.sessions.borrow().iter() {
            if let Some(p) = self.panes.borrow().get(text(s, "name")) {
                let active = (self.window_visible.get()
                    && *self.page.borrow() == "agents"
                    && self.editor.agents_visible()
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
        for session in self.sessions.borrow().iter() {
            panes[text(session, "name")].verify_session_rendered(session);
        }
        let mode = self.mode.borrow().clone();
        for mode in ["grid", "focus", "review", "mosaic"] {
            self.set_mode(mode);
            for (name, pane) in &panes {
                assert!(
                    Rc::ptr_eq(pane, &self.panes.borrow()[name]),
                    "Layout replaced a terminal"
                );
            }
        }
        self.set_mode(&mode);
        println!("Shell layouts verified: grid, focus, review, mosaic, retained terminals");
    }
    pub fn verify_launch(&self) {
        assert!(self.launch.reveals_child());
        assert!(
            self.launch.width() == 780 && self.launch_box.width() == 779,
            "Launch dimensions: sheet={}, form={}",
            self.launch.width(),
            self.launch_box.width()
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
    pub fn terminal_contents_contain(&self, marker: &str) -> bool {
        use vte4::prelude::TerminalExt;
        let panes = self.panes.borrow();
        !panes.is_empty() && panes.values().all(|pane| {
            let (_, row) = pane.terminal.cursor_position();
            let (text, _) = pane.terminal.text_range_format(vte4::Format::Text, 0, 0, row, 500);
            pane.terminal.is_mapped() && text.is_some_and(|text| text.contains(marker))
        })
    }
    pub fn verify_burst(&self, check: bool) {
        use vte4::prelude::TerminalExt;
        assert_eq!(self.panes.borrow().len(), 11);
        if !check {
            println!("Burst layout: {}", self.mode.borrow());
        }
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
    pub fn open_repository(self: &Rc<Self>) {
        self.open_repository_in(None);
    }
    fn open_repository_in(self: &Rc<Self>, workspace: Option<Value>) {
        if self.editor.is_dirty() || !self.dismiss_panels() {
            self.show_info("Save or discard your changes before adding a project.");
            return;
        }
        onboarding::open(self, workspace);
    }
}
