//! Plugins: bundled bundles of skills, agent rules, docs and MCP servers, switched on per
//! project (D159). The page lists each plugin with one switch per project; the project row in
//! the sidebar opens the same switches for that project alone.
use super::{current, paragraph};
use crate::app::{button, clear, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Names of the expanders that are open under `widget`, so a rebuild can reopen them.
fn expanded_names(widget: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(expander) = widget.downcast_ref::<gtk::Expander>() {
        if expander.is_expanded() {
            out.push(expander.widget_name().to_string());
        }
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        expanded_names(&next, out);
        child = next.next_sibling();
    }
}

pub async fn refresh(ui: &Rc<Ui>, project: i64) {
    let generation = ui.generation.get();
    let project_path = format!("mcp.projects.{project}.servers");
    let (result, global, local) = tokio::join!(
        ui.call("plugin.list", json!({})),
        ui.call("settings.get", json!({"path":"mcp.servers"})),
        ui.call("settings.get", json!({"path":project_path}))
    );
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
    let global = global.map(|v| v["value"].clone()).unwrap_or(Value::Null);
    let local = local.map(|v| v["value"].clone()).unwrap_or(Value::Null);
    // Most events that reach this page change nothing it shows. Rebuild only when the data
    // does, so open expanders and the scroll position survive unrelated engine traffic.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (
        project,
        json!([plugins, global, local]).to_string(),
        serde_json::to_string(&*ui.projects.borrow()).unwrap_or_default(),
        serde_json::to_string(&*ui.workspaces.borrow()).unwrap_or_default(),
    )
        .hash(&mut hash);
    let signature = format!("plugins-{:x}", hash.finish());
    let page = &ui.pages["plugins"];
    if page.widget_name() == signature {
        return;
    }
    page.set_widget_name(&signature);
    let mut open = Vec::new();
    expanded_names(page.upcast_ref(), &mut open);
    let scroll = page
        .first_child()
        .and_downcast::<gtk::ScrolledWindow>()
        .map(|s| s.vadjustment().value());
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
        content.append(&card(ui, plugin, &open));
    }
    content.append(&mcp_servers(ui, project, &global, &local));
    let scrolled = crate::app::scrolled(&content);
    page.append(&scrolled);
    if let Some(value) = scroll {
        glib::idle_add_local_once(move || scrolled.vadjustment().set_value(value));
    }
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

fn card(ui: &Rc<Ui>, plugin: &Value, open: &[String]) -> gtk::Box {
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
    let id = text(plugin, "id").to_string();
    let reopen = |expander: &gtk::Expander| {
        expander.set_expanded(open.iter().any(|name| *name == expander.widget_name().as_str()));
    };
    let skills_expander = expander(
        &format!("Skills ({})", skills.len()),
        &skill_list,
        &format!("plugin-{id}-skills"),
    );
    reopen(&skills_expander);
    card.append(&skills_expander);

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
    let tools_expander = expander(
        &format!("MCP tools ({tools})"),
        &tool_list,
        &format!("plugin-{id}-tools"),
    );
    reopen(&tools_expander);
    card.append(&tools_expander);

    // Rules and documentation are fetched when first opened.
    let docs = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let docs_expander = expander(
        "Agent rules and documentation",
        &docs,
        &format!("plugin-{id}-docs"),
    );
    let weak = Rc::downgrade(ui);
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
    // After the handler, so reopening it loads the documents again.
    reopen(&docs_expander);
    card.append(&docs_expander);
    card
}

fn expander(title: &str, child: &gtk::Box, name: &str) -> gtk::Expander {
    let expander = gtk::Expander::new(Some(title));
    expander.add_css_class("plugin-section");
    expander.set_widget_name(name);
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

/// A server name Relay can write as one settings path segment. `relay` is Relay's own.
fn valid_server_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name the server.".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("Use only letters, digits, _ and - in the server name.".into());
    }
    if name.eq_ignore_ascii_case("relay") {
        return Err("\"relay\" is reserved for Relay's own server.".into());
    }
    Ok(())
}

/// Arguments: one per line, or a single line split the way a shell would.
fn parse_args(body: &str) -> Result<Vec<String>, String> {
    let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    match lines.as_slice() {
        [] => Ok(Vec::new()),
        [line] => shell_split(line),
        _ => Ok(lines.iter().map(|l| l.to_string()).collect()),
    }
}

fn shell_split(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            Some(_) => word.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c == '\\' => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    started = true;
                }
            }
            None if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            None => {
                word.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("An argument has an unclosed quote.".into());
    }
    if started {
        out.push(word);
    }
    Ok(out)
}

/// `KEY=VALUE` lines. Keys become settings path segments, so they stay plain identifiers.
fn parse_env(body: &str) -> Result<serde_json::Map<String, Value>, String> {
    let mut env = serde_json::Map::new();
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("Write environment lines as KEY=VALUE: {line}"));
        };
        let key = key.trim();
        if key.is_empty()
            || key.starts_with(|c: char| c.is_ascii_digit())
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!("Invalid environment variable name: {key}"));
        }
        env.insert(key.to_string(), json!(value));
    }
    Ok(env)
}

fn text_area(hint: &str) -> gtk::TextView {
    let view = gtk::TextView::new();
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    view.set_left_margin(6);
    view.set_size_request(-1, 54);
    view.set_tooltip_text(Some(hint));
    view
}

fn area_text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

/// Read the server map at `path` again, apply one change and write it back, so a server
/// added elsewhere in the meantime is kept.
fn write_server(
    ui: &Rc<Ui>,
    path: String,
    name: String,
    server: Option<Value>,
    key: gtk::Button,
    feedback: gtk::Label,
) {
    key.set_sensitive(false);
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let result = async {
            let current = ui.call("settings.get", json!({"path":path})).await?;
            let mut servers = current["value"].as_object().cloned().unwrap_or_default();
            match server {
                Some(server) => {
                    servers.insert(name, server);
                }
                None => {
                    servers.remove(&name);
                }
            }
            let saved = ui
                .call("settings.set", json!({"path":path,"value":Value::Object(servers)}))
                .await?;
            Ok::<Value, crate::client::Error>(saved)
        }
        .await;
        key.set_sensitive(true);
        match result {
            Ok(_) => {
                feedback.set_visible(false);
                ui.refresh_page();
            }
            Err(error) => {
                feedback.set_text(&error.to_string());
                feedback.set_visible(true);
            }
        }
    });
}

/// Your own MCP servers, for this project and for every project. The engine hands them to
/// agents next to Relay's server when they launch or resume.
fn mcp_servers(ui: &Rc<Ui>, project: i64, global: &Value, local: &Value) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("record");
    card.add_css_class("plugin-card");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    top.append(&crate::icons::image("plugins", 22));
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    words.append(&label("Custom MCP servers", "title"));
    words.append(&label("Your own tools, next to Relay's", "dim"));
    top.append(&words);
    card.append(&top);
    let about = paragraph("Add an MCP server that agents should start with. Changes apply to agents launched or resumed after saving; running agents keep the servers they started with.");
    about.set_max_width_chars(110);
    card.append(&about);
    let project_name = ui
        .projects
        .borrow()
        .iter()
        .find(|p| p["id"].as_i64() == Some(project))
        .map(|p| text(p, "name").to_string());
    let mut scopes = Vec::new();
    if let Some(name) = project_name {
        scopes.push((
            format!("THIS PROJECT · {}", name.to_uppercase()),
            format!("mcp.projects.{project}.servers"),
            local.clone(),
        ));
    }
    scopes.push((
        String::from("ALL PROJECTS"),
        String::from("mcp.servers"),
        global.clone(),
    ));
    for (caption, path, servers) in scopes {
        card.append(&label(&caption, "section-label"));
        let feedback = label("", "dim");
        feedback.set_wrap(true);
        feedback.set_visible(false);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let entries = servers.as_object().cloned().unwrap_or_default();
        if entries.is_empty() {
            list.append(&label("No custom servers.", "dim"));
        }
        for (name, server) in &entries {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
            copy.set_hexpand(true);
            copy.append(&label(name, "mono"));
            let args: Vec<String> = rows(server, "args")
                .iter()
                .filter_map(|a| a.as_str().map(str::to_string))
                .collect();
            let command = label(
                &format!("{} {}", text(server, "command"), args.join(" ")),
                "dim",
            );
            command.set_ellipsize(gtk::pango::EllipsizeMode::End);
            command.set_selectable(true);
            copy.append(&command);
            let keys: Vec<&str> = server["env"]
                .as_object()
                .map(|env| env.keys().map(String::as_str).collect())
                .unwrap_or_default();
            if !keys.is_empty() {
                copy.append(&label(&format!("Environment: {}", keys.join(", ")), "dim"));
            }
            row.append(&copy);
            let remove = button("Remove", "quiet");
            remove.set_valign(gtk::Align::Center);
            let weak = Rc::downgrade(ui);
            let path = path.clone();
            let name = name.clone();
            let feedback = feedback.clone();
            remove.connect_clicked(move |key| {
                if let Some(ui) = weak.upgrade() {
                    write_server(
                        &ui,
                        path.clone(),
                        name.clone(),
                        None,
                        key.clone(),
                        feedback.clone(),
                    );
                }
            });
            row.append(&remove);
            list.append(&row);
        }
        card.append(&list);
        let form = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("my-server"));
        crate::app::field("Name (letters, digits, _ and -)", &name, &form);
        let command = gtk::Entry::new();
        command.set_placeholder_text(Some("npx"));
        crate::app::field("Command", &command, &form);
        let args = text_area("One argument per line, or one line split like a shell");
        crate::app::field(
            "Arguments (one per line, or one line split like a shell)",
            &args,
            &form,
        );
        let env = text_area("KEY=VALUE, one per line");
        crate::app::field("Environment (KEY=VALUE, one per line)", &env, &form);
        let add = button("Save server", "primary");
        add.set_halign(gtk::Align::Start);
        form.append(&add);
        let adder = expander(
            "Add a server",
            &form,
            &format!("mcp-add-{}", path.replace('.', "-")),
        );
        let weak = Rc::downgrade(ui);
        let feedback_for_add = feedback.clone();
        let taken: Vec<String> = entries.keys().cloned().collect();
        add.connect_clicked(move |key| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let server_name = name.text().trim().to_string();
            let parsed = valid_server_name(&server_name)
                .and_then(|_| {
                    if taken.contains(&server_name) {
                        Err(format!(
                            "{server_name} already exists. Remove it first to replace it."
                        ))
                    } else {
                        Ok(())
                    }
                })
                .and_then(|_| {
                    let program = command.text().trim().to_string();
                    if program.is_empty() {
                        return Err(String::from("Enter the command that starts the server."));
                    }
                    let args = parse_args(&area_text(&args))?;
                    let env = parse_env(&area_text(&env))?;
                    Ok(json!({"command":program,"args":args,"env":Value::Object(env)}))
                });
            match parsed {
                Ok(server) => write_server(
                    &ui,
                    path.clone(),
                    server_name,
                    Some(server),
                    key.clone(),
                    feedback_for_add.clone(),
                ),
                Err(error) => {
                    feedback_for_add.set_text(&error);
                    feedback_for_add.set_visible(true);
                }
            }
        });
        card.append(&adder);
        card.append(&feedback);
    }
    card
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_split_like_a_shell_or_by_line() {
        assert_eq!(
            parse_args("-y @scope/server --root 'My Files'").unwrap(),
            vec!["-y", "@scope/server", "--root", "My Files"]
        );
        assert_eq!(
            parse_args("--flag\nvalue with spaces\n").unwrap(),
            vec!["--flag", "value with spaces"]
        );
        assert!(parse_args("\"open").is_err());
        assert!(parse_args("").unwrap().is_empty());
    }
    #[test]
    fn names_and_environment_are_path_safe() {
        assert!(valid_server_name("my_server-2").is_ok());
        assert!(valid_server_name("relay").is_err());
        assert!(valid_server_name("a.b").is_err());
        assert_eq!(parse_env("TOKEN=a=b\n\nMODE=x").unwrap().len(), 2);
        assert!(parse_env("1BAD=x").is_err());
        assert!(parse_env("novalue").is_err());
    }
}
