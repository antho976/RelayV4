//! Auxiliary native workspaces. Forms stay mounted across engine events.
use crate::app::{button, clear, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::rc::Rc;

#[path = "tools_devices.rs"]
pub(crate) mod devices;
#[path = "tools_market.rs"]
mod market;
#[path = "tools_plugins.rs"]
pub(crate) mod plugins;
#[path = "tools_settings.rs"]
pub(crate) mod settings;
#[path = "tools_skills.rs"]
mod skills;

pub(super) fn paragraph(value: &str) -> gtk::Label {
    let l = label(value, "body");
    l.set_wrap(true);
    l.set_selectable(true);
    l
}

pub(super) fn section(parent: &gtk::Box, title: &str) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 8);
    block.add_css_class("record");
    block.append(&label(title, "title"));
    parent.append(&block);
    block
}

pub(super) fn action(
    ui: &Rc<Ui>,
    parent: &gtk::Box,
    title: &str,
    op: &'static str,
    payload: Value,
) {
    let key = button(title, "quiet");
    let weak = Rc::downgrade(ui);
    key.connect_clicked(move |b| {
        if let Some(ui) = weak.upgrade() {
            ui.mutate(op, payload.clone(), b);
        }
    });
    parent.append(&key);
}

pub(super) fn current(ui: &Ui, name: &str, project: i64, generation: u64) -> bool {
    ui.generation.get() == generation && ui.project.get() == project && *ui.page.borrow() == name
}

pub async fn refresh(ui: &Rc<Ui>, name: &str, project: i64) {
    match name {
        "settings" => return settings::refresh(ui, project).await,
        "skills" => return skills::refresh(ui, project).await,
        "plugins" => return plugins::refresh(ui, project).await,
        "devices" => return devices::refresh(ui, project).await,
        _ => {}
    }
    let generation = ui.generation.get();
    let (op, payload) = match name {
        "dashboard" => ("dashboard.get", json!({})),
        "notifications" => ("notify.list", json!({"limit": 200})),
        _ => return,
    };
    let result = ui.call(op, payload).await;
    if !current(ui, name, project, generation) {
        return;
    }
    let data = match result {
        Ok(value) => value,
        Err(error) => {
            ui.show_error(&error.to_string());
            return;
        }
    };
    let page = &ui.pages[name];
    clear(page);
    match name {
        "dashboard" => dashboard(ui, page, &data),
        "notifications" => notifications(ui, page, &data),
        _ => {}
    }
}

fn navigate(ui: &Rc<Ui>, parent: &gtk::Box, title: &str, page: &'static str) {
    let key = button(title, "quiet");
    let weak = Rc::downgrade(ui);
    key.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.navigate(page);
        }
    });
    parent.append(&key);
}

fn navigate_session(ui: &Rc<Ui>, parent: &gtk::Box, title: &str, session: &str) {
    let key = button(title, "quiet");
    let weak = Rc::downgrade(ui);
    let session = session.to_string();
    key.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let session = session.clone();
        let key = key.clone();
        key.set_sensitive(false);
        glib::spawn_future_local(async move {
            match ui.call("session.get", json!({"session":session})).await {
                Ok(session) => {
                    if let Some(project) = session["project_id"].as_i64() {
                        ui.open_project(project, "agents");
                    }
                }
                Err(error) => ui.show_error(&error.to_string()),
            }
            key.set_sensitive(true);
        });
    });
    parent.append(&key);
}

fn dashboard_panel(title: &str, caption: &str) -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("dashboard-panel");
    panel.set_valign(gtk::Align::Start);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    head.add_css_class("dashboard-panel-head");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    copy.append(&label(caption, "dashboard-eyebrow"));
    copy.append(&label(title, "title"));
    head.append(&copy);
    panel.append(&head);
    panel
}

fn dashboard_copy(title: &str, subtitle: &str) -> gtk::Box {
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    for (value, class) in [
        (title, "dashboard-row-title"),
        (subtitle, "dashboard-row-subtitle"),
    ] {
        let line = label(value, class);
        line.set_ellipsize(gtk::pango::EllipsizeMode::End);
        line.set_max_width_chars(55);
        copy.append(&line);
    }
    copy
}

fn dashboard_empty(panel: &gtk::Box, title: &str, subtitle: &str) {
    let calm = gtk::Box::new(gtk::Orientation::Vertical, 5);
    calm.add_css_class("dashboard-calm");
    let heading = label(title, "dashboard-row-title");
    heading.set_halign(gtk::Align::Center);
    calm.append(&heading);
    if !subtitle.is_empty() {
        let copy = paragraph(subtitle);
        copy.add_css_class("dashboard-row-subtitle");
        copy.set_justify(gtk::Justification::Center);
        calm.append(&copy);
    }
    panel.append(&calm);
}

fn dashboard(ui: &Rc<Ui>, page: &gtk::Box, data: &Value) {
    page.set_spacing(0);
    page.add_css_class("dashboard-page");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    header.add_css_class("dashboard-head");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    copy.append(&label("WORKSPACE CONTROL ROOM", "dashboard-eyebrow"));
    copy.append(&label("Dashboard", "dashboard-title"));
    copy.append(&label(
        "What needs a decision, what is moving, and what Relay is using.",
        "dim",
    ));
    header.append(&copy);
    let quick = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    quick.add_css_class("dashboard-quick");
    quick.set_valign(gtk::Align::Center);
    for (title, destination, icon) in [
        ("Agents", "agents", "terminal"),
        ("Board", "board", "board"),
        ("Code", "code", "code"),
        ("Notes", "notes", "notes"),
    ] {
        navigate(ui, &quick, title, destination);
        if let Some(key) = quick.last_child().and_downcast::<gtk::Button>() {
            let contents = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            contents.append(&crate::icons::image(icon, 13));
            contents.append(&label(title, "dashboard-action"));
            key.set_child(Some(&contents));
        }
    }
    let refresh = crate::app::icon_button("refresh", "Refresh dashboard");
    let weak = Rc::downgrade(ui);
    refresh.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.refresh_page();
        }
    });
    quick.append(&refresh);
    header.append(&quick);
    page.append(&header);
    let peers = rows(data, "sessions_live");
    let holds = rows(data, "holds_open");
    let reviews = rows(data, "in_review");
    let projects = rows(data, "projects");
    let blocked = peers
        .iter()
        .filter(|p| text(p, "state") == "blocked")
        .count();
    let attention = holds.len() + reviews.len() + blocked;
    let resources = &data["resources"];
    let panes = rows(resources, "panes");
    let cpu: f64 = panes
        .iter()
        .map(|p| p["cpu_pct"].as_f64().unwrap_or(0.))
        .sum();
    let disk: f64 = rows(resources, "worktrees")
        .iter()
        .map(|p| p["disk_mb"].as_f64().unwrap_or(0.))
        .sum();
    let memory = resources["total_rss_mb"].as_f64().unwrap_or(0.);
    let mb = |value: f64| {
        if value >= 1024. {
            format!("{:.1} GB", value / 1024.)
        } else {
            format!("{value:.0} MB")
        }
    };
    let metrics = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    metrics.set_homogeneous(true);
    metrics.add_css_class("dashboard-metrics");
    let done: i64 = projects
        .iter()
        .map(|p| p["done_recent"].as_i64().unwrap_or(0))
        .sum();
    let open: i64 = projects
        .iter()
        .map(|p| p["tasks_open"].as_i64().unwrap_or(0))
        .sum();
    for (value, caption, detail, icon, destination) in [
        (
            attention.to_string(),
            "NEEDS YOU",
            format!(
                "{} holds · {blocked} blocked · {} review",
                holds.len(),
                reviews.len()
            ),
            "bell",
            "board",
        ),
        (
            peers.len().to_string(),
            "OPEN AGENTS",
            format!(
                "{} running · {} idle",
                peers
                    .iter()
                    .filter(|p| text(p, "state") == "running")
                    .count(),
                peers.iter().filter(|p| text(p, "state") == "idle").count()
            ),
            "terminal",
            "agents",
        ),
        (
            done.to_string(),
            "DONE THIS WEEK",
            format!("{open} tasks still open"),
            "check",
            "board",
        ),
        (
            mb(memory),
            "AGENT MEMORY",
            format!("{cpu:.1}% CPU · {} worktrees", mb(disk)),
            "cpu",
            "",
        ),
    ] {
        let metric = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        metric.add_css_class("dashboard-metric");
        let image = crate::icons::image(icon, 16);
        image.add_css_class("dashboard-metric-icon");
        image.set_valign(gtk::Align::Center);
        metric.append(&image);
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
        copy.set_valign(gtk::Align::Center);
        copy.append(&label(&value, "dashboard-metric-value"));
        copy.append(&label(caption, "dashboard-metric-label"));
        metric.append(&copy);
        let hint = label(&detail, "dashboard-metric-hint");
        hint.set_wrap(true);
        hint.set_hexpand(true);
        hint.set_xalign(1.0);
        metric.append(&hint);
        if destination.is_empty() {
            metrics.append(&metric);
        } else {
            let key = button("", "dashboard-metric-key");
            key.set_child(Some(&metric));
            if caption == "NEEDS YOU" && attention > 0 {
                key.add_css_class("hot");
            }
            let weak = Rc::downgrade(ui);
            key.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.navigate(destination);
                }
            });
            metrics.append(&key);
        }
    }
    page.append(&metrics);
    let grid = gtk::Grid::builder()
        .column_homogeneous(true)
        .column_spacing(10)
        .row_spacing(10)
        .build();
    grid.add_css_class("dashboard-grid");
    page.append(&grid);
    let needs = dashboard_panel("Needs you", "DECISION QUEUE");
    let pulse = dashboard_panel("Resource pulse", "RIGHT NOW");
    let portfolio = dashboard_panel("Project pulse", "PORTFOLIO");
    let active = dashboard_panel("Active agents", "IN MOTION");
    let activity = dashboard_panel("Recent activity", "EVENT STREAM");
    grid.attach(&needs, 0, 0, 7, 1);
    grid.attach(&pulse, 7, 0, 5, 1);
    grid.attach(&portfolio, 0, 1, 7, 1);
    grid.attach(&active, 7, 1, 5, 1);
    grid.attach(&activity, 0, 2, 12, 1);
    if let Some(head) = needs.first_child().and_downcast::<gtk::Box>() {
        let count = label(&attention.to_string(), "dashboard-count");
        count.set_valign(gtk::Align::Center);
        if attention > 0 {
            count.add_css_class("hot");
        }
        head.append(&count);
    }
    if attention == 0 {
        dashboard_empty(
            &needs,
            "Nothing is waiting",
            "Blocked agents, guardrail holds, and tasks ready for approval collect here.",
        );
    }
    for hold in holds {
        let key = button("", "dashboard-queue-row");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        let icon = crate::icons::image("shield", 14);
        icon.add_css_class("dashboard-signal");
        row.append(&icon);
        row.append(&dashboard_copy(
            "Guardrail decision",
            &format!(
                "{} · {} · {}",
                text(&hold, "policy"),
                text(&hold, "op"),
                text(&hold, "session")
            ),
        ));
        row.append(&crate::icons::image("chevron-right", 13));
        key.set_child(Some(&row));
        let weak = Rc::downgrade(ui);
        let project = hold["project_id"].as_i64();
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.open_project(project.unwrap_or(ui.project.get()), "guardrails");
            }
        });
        needs.append(&key);
    }
    for peer in peers.iter().filter(|p| text(p, "state") == "blocked") {
        navigate_session(ui, &needs, "", text(peer, "session"));
        if let Some(key) = needs.last_child().and_downcast::<gtk::Button>() {
            key.add_css_class("dashboard-queue-row");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            let icon = crate::icons::image("terminal", 14);
            icon.add_css_class("dashboard-signal");
            row.append(&icon);
            row.append(&dashboard_copy(
                &format!("{} is blocked", text(peer, "session")),
                peer["intent"]
                    .as_str()
                    .or(peer["task_title"].as_str())
                    .unwrap_or(text(peer, "branch")),
            ));
            key.set_child(Some(&row));
        }
    }
    for review in reviews {
        let key = button("", "dashboard-queue-row");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        let icon = crate::icons::image("check", 14);
        icon.add_css_class("dashboard-signal");
        row.append(&icon);
        row.append(&dashboard_copy(
            text(&review, "title"),
            &format!("#{}", review["id"]),
        ));
        row.append(&label("review", "dashboard-review"));
        key.set_child(Some(&row));
        let weak = Rc::downgrade(ui);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let project = review["project_id"].as_i64().unwrap_or(ui.project.get());
                ui.open_project(project, "board");
                if ui.project.get() == project {
                    if let Some(id) = review["id"].as_i64() {
                        crate::pages::open_task(&ui, id);
                    }
                }
            }
        });
        needs.append(&key);
    }
    let resource_grid = gtk::Grid::builder()
        .column_spacing(8)
        .row_spacing(8)
        .column_homogeneous(true)
        .build();
    resource_grid.add_css_class("dashboard-resource-grid");
    let meter = gtk::Box::new(gtk::Orientation::Vertical, 7);
    meter.add_css_class("dashboard-resource-meter");
    meter.append(&label("AGENT CPU", "dashboard-metric-label"));
    meter.append(&label(&format!("{cpu:.1}%"), "dashboard-cpu"));
    let bar = gtk::ProgressBar::new();
    bar.set_fraction((cpu / 100.).clamp(0., 1.));
    meter.append(&bar);
    meter.append(&label(
        &format!("{} tracked processes", panes.len()),
        "dashboard-row-subtitle",
    ));
    resource_grid.attach(&meter, 0, 0, 1, 2);
    for (index, (caption, value)) in [
        ("MEMORY", mb(memory)),
        ("WORKTREES", mb(disk)),
        (
            "RELAY STORE",
            mb(resources["store_mb"].as_f64().unwrap_or(0.)),
        ),
        ("PROCESSES", panes.len().to_string()),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
        cell.add_css_class("dashboard-resource-cell");
        cell.append(&label(caption, "dashboard-metric-label"));
        cell.append(&label(&value, "mono"));
        resource_grid.attach(&cell, 1 + index as i32 % 2, index as i32 / 2, 1, 1);
    }
    pulse.append(&resource_grid);
    let processes = gtk::Box::new(gtk::Orientation::Vertical, 4);
    processes.add_css_class("dashboard-processes");
    for pane in panes.iter().take(4) {
        let overlay = gtk::Overlay::new();
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("dashboard-process-bar");
        bar.set_fraction((pane["rss_mb"].as_f64().unwrap_or(0.) / memory.max(1.)).clamp(0.03, 1.));
        overlay.set_child(Some(&bar));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("dashboard-process-row");
        let name = label(text(pane, "session"), "dashboard-row-subtitle");
        name.set_hexpand(true);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&name);
        row.append(&label(
            &format!(
                "{} · {:.1}%",
                mb(pane["rss_mb"].as_f64().unwrap_or(0.)),
                pane["cpu_pct"].as_f64().unwrap_or(0.)
            ),
            "dashboard-row-subtitle",
        ));
        overlay.add_overlay(&row);
        processes.append(&overlay);
    }
    pulse.append(&processes);
    for project in projects {
        let key = button("", "dashboard-project-row");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&dashboard_copy(
            text(&project, "name"),
            text(&project, "base_branch"),
        ));
        let ready = project["ready"].as_f64().unwrap_or(0.);
        let moving = project["active"].as_f64().unwrap_or(0.);
        let review = project["in_review"].as_f64().unwrap_or(0.);
        let bars = gtk::DrawingArea::new();
        bars.set_content_height(4);
        bars.set_draw_func(move |widget, cr, width, height| {
            let total = (ready + moving + review).max(1.);
            let mut left = 0.;
            for (value, name) in [(ready, "secondary"), (moving, "live"), (review, "waiting")] {
                if let Some(color) = widget.style_context().lookup_color(name) {
                    cr.set_source_rgba(
                        color.red() as f64,
                        color.green() as f64,
                        color.blue() as f64,
                        color.alpha() as f64,
                    );
                    let extent = width as f64 * value / total;
                    cr.rectangle(left, 0., extent, height as f64);
                    let _ = cr.fill();
                    left += extent;
                }
            }
        });
        content.append(&bars);
        let meta = label(
            &format!(
                "{} open · {} ready · {} active · {} review · {} agents",
                project["tasks_open"],
                project["ready"],
                project["active"],
                project["in_review"],
                project["live_sessions"]
            ),
            "dashboard-project-meta",
        );
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        content.append(&meta);
        key.set_child(Some(&content));
        let weak = Rc::downgrade(ui);
        let id = project["project_id"].as_i64().unwrap_or(0);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.open_project(id, if review > 0. { "board" } else { "agents" });
            }
        });
        portfolio.append(&key);
    }
    if peers.is_empty() {
        dashboard_empty(
            &active,
            "No agents open",
            "Launch a session when there is work ready to move.",
        );
    }
    for peer in peers.iter().take(6) {
        navigate_session(ui, &active, "", text(peer, "session"));
        if let Some(key) = active.last_child().and_downcast::<gtk::Button>() {
            key.add_css_class("dashboard-agent-row");
            let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            let lamp = label("", "lamp");
            lamp.set_valign(gtk::Align::Center);
            lamp.set_halign(gtk::Align::Center);
            lamp.add_css_class(match text(peer, "state") {
                "running" | "spawning" => "live",
                "blocked" => "held",
                _ => "waiting",
            });
            row.append(&lamp);
            row.append(&dashboard_copy(
                text(peer, "session"),
                peer["intent"]
                    .as_str()
                    .or(peer["task_title"].as_str())
                    .unwrap_or("No task assigned"),
            ));
            let provider = gtk::Box::new(gtk::Orientation::Vertical, 2);
            provider.append(&label(text(peer, "provider"), "dashboard-row-subtitle"));
            provider.append(&label(text(peer, "role"), "dashboard-row-subtitle"));
            row.append(&provider);
            content.append(&row);
            content.append(&label(text(peer, "branch"), "dashboard-project-meta"));
            key.set_child(Some(&content));
        }
    }
    let events = rows(data, "notifications");
    let timeline = gtk::Grid::builder()
        .column_homogeneous(true)
        .column_spacing(18)
        .build();
    timeline.add_css_class("dashboard-timeline");
    for (index, item) in events.iter().take(8).enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("dashboard-event");
        row.append(&crate::icons::image("bell", 12));
        row.append(&dashboard_copy(text(item, "title"), text(item, "body")));
        timeline.attach(&row, index as i32 % 2, index as i32 / 2, 1, 1);
    }
    activity.append(&timeline);
    if events.is_empty() {
        dashboard_empty(&activity, "No recent activity", "");
    }
}

fn notifications(ui: &Rc<Ui>, page: &gtk::Box, data: &Value) {
    page.append(&label("Notifications", "title"));
    action(ui, page, "Mark all read", "notify.ack_all", json!({}));
    let entries = rows(data, "notifications");
    if entries.is_empty() {
        page.append(&paragraph("No notifications yet."));
    }
    for item in entries {
        let row = section(page, text(&item, "title"));
        row.append(&paragraph(text(&item, "body")));
        row.append(&label(
            &format!(
                "{} · {}",
                text(&item, "category"),
                text(&item, "created_at")
            ),
            "dim",
        ));
        if !item["read"].as_bool().unwrap_or(false) {
            action(
                ui,
                &row,
                "Mark read",
                "notify.ack",
                json!({"notification_id":item["id"]}),
            );
        }
        let key = button("Open workspace", "quiet");
        row.append(&key);
        let weak = Rc::downgrade(ui);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let project = item["project_id"].as_i64().unwrap_or(ui.project.get());
                let payload = &item["link"]["payload"];
                let page = match text(&item["link"], "op") {
                    "ui.page.switch" => payload["page"].as_str().unwrap_or("notifications"),
                    "task.get" => "board",
                    _ => match text(&item, "category") {
                        "agent_done" | "integration" => "board",
                        "guardrail" => "guardrails",
                        "provider" => "settings",
                        _ => "agents",
                    },
                };
                ui.open_project(project, page);
                if ui.project.get() == project && text(&item["link"], "op") == "task.get" {
                    if let Some(id) = payload["task_id"].as_i64() {
                        crate::pages::open_task(&ui, id);
                    }
                }
            }
        });
    }
}
