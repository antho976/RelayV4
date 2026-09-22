//! Plugins: bundled bundles of skills, agent rules, docs and MCP servers, switched on per
//! project (D159). The page lists each plugin with one switch per project; the project row in
//! the sidebar opens the same switches for that project alone.
use super::{current, paragraph};
use crate::app::{clear, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::rc::Rc;

pub async fn refresh(ui: &Rc<Ui>, project: i64) {
    let generation = ui.generation.get();
    let result = ui.call("plugin.list", json!({})).await;
    if !current(ui, "plugins", project, generation) {
        return;
    }
    let plugins = match result {
        Ok(value) => rows(&value, "plugins"),
        Err(error) => {
            ui.show_error(&error.to_string());
            return;
        }
    };
    let page = &ui.pages["plugins"];
    clear(page);
    page.add_css_class("plugins-page");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.add_css_class("plugins-content");
    let head = gtk::Box::new(gtk::Orientation::Vertical, 3);
    head.append(&label("Plugins", "title"));
    let copy = paragraph("A plugin gives every agent of a project a whole toolkit at once: skill folders, standing rules in its brief, and MCP tools. Switch one on per project. Skills reach running agents immediately; rules and tools apply when an agent starts or resumes.");
    copy.set_max_width_chars(96);
    head.append(&copy);
    content.append(&head);
    if plugins.is_empty() {
        content.append(&paragraph("This build bundles no plugins."));
    }
    for plugin in &plugins {
        content.append(&card(ui, plugin));
    }
    page.append(&crate::app::scrolled(&content));
}

fn enabled(plugin: &Value, project: i64) -> bool {
    plugin["enabled_in"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|id| id.as_i64() == Some(project)))
}

fn suggested(plugin: &Value, project: i64) -> bool {
    plugin["suggested_for"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|id| id.as_i64() == Some(project)))
}

/// A switch that enables `plugin` for `project`, restoring itself when the engine refuses.
pub(crate) fn switch(
    ui: &Rc<Ui>,
    plugin: &Value,
    project: i64,
    feedback: Option<gtk::Label>,
) -> gtk::Switch {
    let toggle = gtk::Switch::new();
    toggle.set_valign(gtk::Align::Center);
    toggle.set_active(enabled(plugin, project));
    toggle.set_widget_name(&format!("plugin-{}-{project}", text(plugin, "id")));
    toggle.set_tooltip_text(Some(&format!(
        "Use {} in this project",
        text(plugin, "name")
    )));
    let weak = Rc::downgrade(ui);
    let id = text(plugin, "id").to_string();
    toggle.connect_state_set(move |key, on| {
        // A rejected write restores the switch while it is disabled.
        if !key.is_sensitive() {
            return glib::Propagation::Proceed;
        }
        if let Some(ui) = weak.upgrade() {
            let key = key.clone();
            let id = id.clone();
            let feedback = feedback.clone();
            key.set_sensitive(false);
            glib::spawn_future_local(async move {
                match ui
                    .call(
                        "plugin.enable",
                        json!({"plugin_id":id,"project_id":project,"enabled":on}),
                    )
                    .await
                {
                    Ok(_) => {
                        if let Some(feedback) = &feedback {
                            feedback.set_visible(false);
                        }
                    }
                    Err(error) => {
                        key.set_active(!on);
                        match &feedback {
                            Some(feedback) => {
                                feedback.set_text(&error.to_string());
                                feedback.set_visible(true);
                            }
                            None => ui.show_error(&error.to_string()),
                        }
                    }
                }
                key.set_sensitive(true);
            });
        }
        glib::Propagation::Proceed
    });
    toggle
}

fn card(ui: &Rc<Ui>, plugin: &Value) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("record");
    card.add_css_class("plugin-card");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    top.append(&crate::icons::image("plugins", 22));
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    words.append(&label(text(plugin, "name"), "title"));
    let skills = rows(plugin, "skills");
    let servers = rows(plugin, "mcp_servers");
    let tools: usize = servers.iter().map(|s| rows(s, "tools").len()).sum();
    words.append(&label(
        &format!(
            "{} · v{} · {} skills · {} MCP tools",
            text(plugin, "category"),
            text(plugin, "version"),
            skills.len(),
            tools
        ),
        "dim",
    ));
    top.append(&words);
    card.append(&top);
    let summary = paragraph(text(plugin, "description"));
    summary.set_max_width_chars(110);
    card.append(&summary);

    // One switch per project, grouped under its workspace.
    card.append(&label("PROJECTS", "section-label"));
    let projects = ui.projects.borrow().clone();
    let workspaces = ui.workspaces.borrow().clone();
    if projects.is_empty() {
        card.append(&label(
            "Add a project to switch this plugin on for it.",
            "dim",
        ));
    }
    let grid = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(6)
        .build();
    let mut line = 0;
    for workspace in &workspaces {
        let mine: Vec<&Value> = projects
            .iter()
            .filter(|p| p["workspace_id"] == workspace["id"])
            .collect();
        if mine.is_empty() {
            continue;
        }
        let caption = label(text(workspace, "name"), "dim");
        caption.set_xalign(0.);
        grid.attach(&caption, 0, line, 3, 1);
        line += 1;
        for project in mine {
            let id = project["id"].as_i64().unwrap_or(0);
            let name = label(text(project, "name"), "body");
            name.set_xalign(0.);
            name.set_hexpand(true);
            name.set_margin_start(12);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            grid.attach(&name, 0, line, 1, 1);
            if suggested(plugin, id) {
                let hint = label("Suggested", "task-chip");
                hint.set_tooltip_text(Some("This project's files match what the plugin is for"));
                grid.attach(&hint, 1, line, 1, 1);
            }
            grid.attach(&switch(ui, plugin, id, None), 2, line, 1, 1);
            line += 1;
        }
    }
    card.append(&grid);

    let skill_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for skill in &skills {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 1);
        row.append(&label(text(skill, "name"), "mono"));
        let about = paragraph(text(skill, "description"));
        about.add_css_class("dim");
        row.append(&about);
        skill_list.append(&row);
    }
    card.append(&expander(
        &format!("Skills ({})", skills.len()),
        &skill_list,
    ));

    let tool_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for server in &servers {
        tool_list.append(&label(
            &format!("Server `{}`", text(server, "name")),
            "body",
        ));
        tool_list.append(&paragraph(text(server, "description")));
        let names: Vec<String> = rows(server, "tools")
            .iter()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect();
        tool_list.append(&label(&names.join("  "), "mono"));
    }
    card.append(&expander(&format!("MCP tools ({tools})"), &tool_list));

    // Rules and documentation are fetched when first opened.
    let docs = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let docs_expander = expander("Agent rules and documentation", &docs);
    let weak = Rc::downgrade(ui);
    let id = text(plugin, "id").to_string();
    docs_expander.connect_expanded_notify(move |expander| {
        if !expander.is_expanded() || docs.first_child().is_some() {
            return;
        }
        let Some(ui) = weak.upgrade() else { return };
        let docs = docs.clone();
        let id = id.clone();
        docs.append(&label("Loading…", "dim"));
        glib::spawn_future_local(async move {
            let result = ui.call("plugin.get", json!({"plugin_id": id})).await;
            clear(&docs);
            match result {
                Ok(detail) => {
                    docs.append(&label("RULES EVERY AGENT RECEIVES", "section-label"));
                    docs.append(&source(text(&detail, "instructions")));
                    for doc in rows(&detail, "docs") {
                        docs.append(&label(&text(&doc, "path").to_uppercase(), "section-label"));
                        docs.append(&source(text(&doc, "body")));
                    }
                }
                Err(error) => docs.append(&paragraph(&error.to_string())),
            }
        });
    });
    card.append(&docs_expander);
    card
}

fn expander(title: &str, child: &gtk::Box) -> gtk::Expander {
    let expander = gtk::Expander::new(Some(title));
    expander.add_css_class("plugin-section");
    expander.set_expanded(false);
    child.set_margin_start(12);
    child.set_margin_top(6);
    expander.set_child(Some(child));
    expander
}

fn source(body: &str) -> gtk::ScrolledWindow {
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.add_css_class("skill-source");
    view.buffer().set_text(body);
    let scroll = crate::app::scrolled(&view);
    scroll.set_min_content_height(160);
    scroll.set_max_content_height(420);
    scroll.set_propagate_natural_height(true);
    scroll
}
