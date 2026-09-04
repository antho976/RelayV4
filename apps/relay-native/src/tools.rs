//! Auxiliary native workspaces. Forms stay mounted across engine events.
use crate::app::{button, clear, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::rc::Rc;

#[path = "tools_devices.rs"]
pub(crate) mod devices;
#[path = "tools_settings.rs"]
mod settings;
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
        "devices" => return devices::refresh(ui, project).await,
        _ => {}
    }
    let generation = ui.generation.get();
    let (op, payload) = match name {
        "dashboard" => ("dashboard.get", json!({})),
        "notifications" => ("notify.list", json!({"limit": 200})),
        "plugins" => ("plugin.list", json!({})),
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
        "plugins" => {
            page.append(&label("Plugins", "title"));
            page.append(&paragraph("The extension surface is reserved for a future release, as in Relay-2. Installed instructions are managed in Skills."));
            for plugin in rows(&data, "plugins") {
                page.append(&paragraph(text(&plugin, "name")));
            }
        }
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

fn dashboard(ui: &Rc<Ui>, page: &gtk::Box, data: &Value) {
    page.append(&label("Dashboard", "title"));
    page.append(&paragraph(
        "What needs a decision, what is moving, and what Relay is using.",
    ));
    let quick = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (title, destination) in [
        ("Agents", "agents"),
        ("Board", "board"),
        ("Code", "code"),
        ("Notes", "notes"),
    ] {
        navigate(ui, &quick, title, destination);
    }
    page.append(&quick);
    let peers = rows(data, "sessions_live");
    let holds = rows(data, "holds_open");
    let reviews = rows(data, "in_review");
    let blocked = peers
        .iter()
        .filter(|p| text(p, "state") == "blocked")
        .count();
    let projects = rows(data, "projects");
    let metrics = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    metrics.set_homogeneous(true);
    for (value, caption) in [
        (
            (holds.len() + reviews.len() + blocked).to_string(),
            "NEEDS YOU",
        ),
        (peers.len().to_string(), "OPEN AGENTS"),
        (
            projects
                .iter()
                .map(|p| p["done_recent"].as_i64().unwrap_or(0))
                .sum::<i64>()
                .to_string(),
            "DONE THIS WEEK",
        ),
        (
            format!(
                "{:.0} MB",
                data["resources"]["total_rss_mb"].as_f64().unwrap_or(0.)
            ),
            "PROCESS MEMORY",
        ),
    ] {
        let metric = section(&metrics, &value);
        metric.append(&label(caption, "dim"));
    }
    page.append(&metrics);
    let columns = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    columns.set_homogeneous(true);
    page.append(&columns);
    let left = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let right = gtk::Box::new(gtk::Orientation::Vertical, 12);
    columns.append(&left);
    columns.append(&right);
    let needs = section(&left, "Needs you");
    if holds.is_empty() && reviews.is_empty() && blocked == 0 {
        needs.append(&paragraph(
            "Nothing is waiting. Holds, blocked agents and reviews collect here.",
        ));
    }
    for hold in holds {
        let key = button(
            &format!(
                "Guardrail decision · {} · {}",
                text(&hold, "session"),
                text(&hold, "op")
            ),
            "quiet",
        );
        let weak = Rc::downgrade(ui);
        let project = hold["project_id"].as_i64();
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.open_project(project.unwrap_or(ui.project.get()), "guardrails");
            }
        });
        needs.append(&key);
    }
    for peer in &peers {
        if text(peer, "state") == "blocked" {
            navigate_session(
                ui,
                &needs,
                &format!("{} is blocked", text(peer, "session")),
                text(peer, "session"),
            );
        }
    }
    for review in reviews {
        let key = button(&format!("Review · {}", text(&review, "title")), "quiet");
        needs.append(&key);
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
    }
    let pulse = section(&right, "Resource pulse");
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
    pulse.append(&paragraph(&format!(
        "{cpu:.1}% agent CPU\n{disk:.0} MB worktrees\n{:.1} MB Relay store",
        resources["store_mb"].as_f64().unwrap_or(0.)
    )));
    for pane in panes {
        pulse.append(&paragraph(&format!(
            "{} · {:.0} MB · {:.1}% CPU",
            text(&pane, "session"),
            pane["rss_mb"].as_f64().unwrap_or(0.),
            pane["cpu_pct"].as_f64().unwrap_or(0.)
        )));
    }
    let portfolio = section(&left, "Project pulse");
    for project in projects {
        portfolio.append(&label(text(&project, "name"), "title"));
        portfolio.append(&paragraph(&format!(
            "{} ready · {} active · {} review · {} agents",
            project["ready"], project["active"], project["in_review"], project["live_sessions"]
        )));
    }
    let active = section(&right, "Active agents");
    if peers.is_empty() {
        active.append(&paragraph("No agents open."));
    }
    for peer in peers {
        navigate_session(
            ui,
            &active,
            &format!("{} · {}", text(&peer, "session"), text(&peer, "state")),
            text(&peer, "session"),
        );
        active.append(&paragraph(&format!(
            "{} · {}\n{}",
            text(&peer, "provider"),
            text(&peer, "role"),
            peer["intent"]
                .as_str()
                .or(peer["task_title"].as_str())
                .unwrap_or("No task assigned")
        )));
    }
    let activity = section(page, "Recent activity");
    for item in rows(data, "notifications") {
        activity.append(&paragraph(&format!(
            "{}\n{}",
            text(&item, "title"),
            text(&item, "body")
        )));
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
