//! Plugins: bundled bundles of skills, agent rules, docs and MCP servers, switched on per
//! project (D159). The page is a searchable catalog beside a detail pane (`tools_market.rs`);
//! the project row in the sidebar opens the same switches for that project alone.
use super::market::{self, Market, Spec};
use super::paragraph;
use crate::app::{clear, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::rc::Rc;

/// Every build of the engine bundles plugins, so an empty list or a refused `project_id` means
/// the engine that is running was started from an older build: `run.sh` reuses a running engine.
pub(crate) const STALE_ENGINE: &str = "The running Relay engine is older than this app and does not know about plugins yet. Restart it: when your agents are idle, run `target/debug/relay --instance dev q app.quit '{\"force\":true}'` (use your instance name), then ./run.sh again. Live agents come back as restorable; resume them from their tiles.";

pub(crate) fn explain(error: &str) -> String {
    if error.contains("plugin.")
        && (error.contains("unknown field") || error.contains("not implemented"))
    {
        STALE_ENGINE.to_string()
    } else {
        error.to_string()
    }
}

pub async fn refresh(ui: &Rc<Ui>, project: i64) {
    market::refresh(ui, &SPEC, project).await
}

static SPEC: Spec = Spec {
    page: "plugins",
    title: "Plugins",
    subtitle: "Whole toolkits for a project's agents: skill folders, standing rules in every brief, and MCP tools. Switch one on per project.",
    icon: "plugins",
    noun: ("plugin", "plugins"),
    placeholder: "Search plugins, skills and tools",
    filters: &["All", "Enabled", "Available", "Suggested"],
    list_op: "plugin.list",
    list_key: "plugins",
    enable_op: "plugin.enable",
    id_key: "plugin_id",
    id: |plugin| text(plugin, "id").to_string(),
    haystack,
    passes: |plugin, filter, project| match filter {
        1 => enabled(plugin, project),
        2 => !enabled(plugin, project),
        3 => suggested(plugin, project),
        _ => true,
    },
    group: None,
    sort: |plugin| text(plugin, "name").to_lowercase(),
    card,
    detail,
    empty,
    setup: None,
};

fn haystack(plugin: &Value) -> String {
    let mut words = vec![
        text(plugin, "name").to_string(),
        text(plugin, "id").to_string(),
        text(plugin, "category").to_string(),
        text(plugin, "summary").to_string(),
        text(plugin, "description").to_string(),
    ];
    for skill in rows(plugin, "skills") {
        words.push(text(&skill, "name").to_string());
        words.push(text(&skill, "description").to_string());
    }
    for server in rows(plugin, "mcp_servers") {
        words.push(text(&server, "name").to_string());
        words.extend(tool_names(&server));
    }
    words.join("\n").to_lowercase()
}

fn tool_names(server: &Value) -> Vec<String> {
    rows(server, "tools")
        .iter()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect()
}

fn tool_count(plugin: &Value) -> usize {
    rows(plugin, "mcp_servers").iter().map(|s| rows(s, "tools").len()).sum()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
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

fn card(ui: &Rc<Ui>, market: &Rc<Market>, plugin: &Value) -> gtk::Widget {
    let scope = market.scope.get();
    let id = text(plugin, "id");
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    card.add_css_class("ext-card");
    card.append(&market::monogram(text(plugin, "name"), false));
    let words = gtk::Box::new(gtk::Orientation::Vertical, 4);
    words.set_hexpand(true);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = label(text(plugin, "name"), "ext-name");
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&name);
    let version = label(&format!("v{}", text(plugin, "version")), "ext-version");
    version.set_valign(gtk::Align::Baseline);
    top.append(&version);
    words.append(&top);
    let category = label(text(plugin, "category"), "ext-meta");
    category.set_ellipsize(gtk::pango::EllipsizeMode::End);
    words.append(&category);
    let summary = label(text(plugin, "summary"), "ext-summary");
    summary.set_wrap(true);
    summary.set_lines(2);
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_max_width_chars(44);
    words.append(&summary);
    let mut tags = vec![
        market::chip(&plural(rows(plugin, "skills").len(), "skill", "skills"), ""),
        market::chip(&plural(tool_count(plugin), "MCP tool", "MCP tools"), ""),
        market::chip(&plural(rows(plugin, "docs").len(), "doc", "docs"), ""),
    ];
    if suggested(plugin, scope) && !enabled(plugin, scope) {
        let hint = market::chip("Suggested", "hint");
        hint.set_tooltip_text(Some("This project's files match what the plugin is for"));
        tags.push(hint);
    }
    let flow = market::chips(&tags);
    flow.set_margin_top(2);
    words.append(&flow);
    card.append(&words);
    let switch = market::toggle(ui, market, plugin, scope, format!("plugin-{id}-{scope}"));
    switch.set_valign(gtk::Align::Start);
    card.append(&switch);
    card.upcast()
}

fn detail(ui: &Rc<Ui>, market: &Rc<Market>, plugin: &Value, parent: &gtk::Box) {
    let id = text(plugin, "id").to_string();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    head.append(&market::monogram(text(plugin, "name"), true));
    let words = gtk::Box::new(gtk::Orientation::Vertical, 4);
    words.set_valign(gtk::Align::Center);
    words.set_hexpand(true);
    let title = label(text(plugin, "name"), "ext-detail-title");
    title.set_wrap(true);
    words.append(&title);
    words.append(&label(
        &format!(
            "{} · v{} · bundled with Relay",
            text(plugin, "category"),
            text(plugin, "version")
        ),
        "ext-meta",
    ));
    head.append(&words);
    parent.append(&head);
    let lede = market::prose(text(plugin, "summary"));
    lede.add_css_class("ext-lede");
    parent.append(&lede);
    parent.append(&market::enable_block(
        ui,
        market,
        plugin,
        (
            "Agents launched here get its skills, rules and MCP tools.",
            "Switch it on to give this project's agents the whole toolkit.",
        ),
        format!("plugin-{id}-detail"),
    ));

    let skills = rows(plugin, "skills");
    let servers = rows(plugin, "mcp_servers");
    let tools = tool_count(plugin);
    let (projects, _) = market::reach(ui, plugin);
    let switch_name = {
        let id = id.clone();
        move |project: i64| format!("plugin-{id}-in-{project}")
    };
    parent.append(&market::tabs(
        market,
        vec![
            ("overview", "Overview", None, overview(plugin).upcast()),
            ("skills", "Skills", Some(skills.len()), skill_list(ui, market, &id, &skills).upcast()),
            ("tools", "MCP tools", Some(tools), tool_list(&servers).upcast()),
            ("docs", "Rules & docs", Some(rows(plugin, "docs").len()), documents(ui, market, &id).upcast()),
            (
                "projects",
                "Projects",
                Some(projects),
                market::project_switches(ui, market, plugin, &switch_name, Some(suggested)).upcast(),
            ),
        ],
    ));
}

fn overview(plugin: &Value) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 20);
    page.append(&market::prose(text(plugin, "description")));
    let reach = market::section("How it reaches agents");
    reach.append(&market::facts(&[
        (
            "Skills",
            "Written into every checkout as real skill folders. Running agents see them at once."
                .into(),
            false,
        ),
        (
            "Rules",
            "Added to each agent's brief when it starts or resumes.".into(),
            false,
        ),
        (
            "MCP tools",
            "Registered with the provider when an agent starts or resumes.".into(),
            false,
        ),
    ]));
    page.append(&reach);
    let servers: Vec<String> = rows(plugin, "mcp_servers")
        .iter()
        .map(|s| text(s, "name").to_string())
        .collect();
    let docs: Vec<String> = rows(plugin, "docs")
        .iter()
        .filter_map(|d| d.as_str().map(str::to_string))
        .collect();
    let about = market::section("Package");
    about.append(&market::facts(&[
        ("Identifier", text(plugin, "id").to_string(), true),
        ("Version", text(plugin, "version").to_string(), true),
        ("MCP servers", if servers.is_empty() { "None".into() } else { servers.join(", ") }, true),
        ("Documents", if docs.is_empty() { "None".into() } else { docs.join("\n") }, true),
    ]));
    page.append(&about);
    page
}

fn skill_list(ui: &Rc<Ui>, market: &Rc<Market>, plugin: &str, skills: &[Value]) -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("ext-entries");
    if skills.is_empty() {
        list.append(&label("This plugin ships no skills.", "dim"));
    }
    for skill in skills {
        let name = text(skill, "name").to_string();
        let head = gtk::Box::new(gtk::Orientation::Vertical, 3);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label(&name, "ext-entry-name");
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&title);
        let files = skill["files"].as_u64().unwrap_or(0) as usize;
        if files > 0 {
            top.append(&label(&plural(files, "file", "files"), "ext-group-count"));
        }
        head.append(&top);
        let about = label(text(skill, "description"), "ext-entry-text");
        about.set_wrap(true);
        about.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        about.set_lines(3);
        about.set_ellipsize(gtk::pango::EllipsizeMode::End);
        head.append(&about);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.add_css_class("ext-entry-body");
        let expander = gtk::Expander::new(None);
        expander.add_css_class("ext-entry");
        expander.set_label_widget(Some(&head));
        expander.set_child(Some(&body));
        let weak = Rc::downgrade(ui);
        let market = Rc::downgrade(market);
        let plugin = plugin.to_string();
        expander.connect_expanded_notify(move |expander| {
            if !expander.is_expanded() || body.first_child().is_some() {
                return;
            }
            let (Some(ui), Some(market)) = (weak.upgrade(), market.upgrade()) else {
                return;
            };
            let key = format!("{plugin}/skill/{name}");
            let cached = market.cache.borrow().get(&key).cloned();
            if let Some(found) = cached {
                body.append(&market::markdown(&ui, text(&found["skill"], "body")));
                return;
            }
            body.append(&label("Loading SKILL.md…", "dim"));
            let (body, plugin, name) = (body.clone(), plugin.clone(), name.clone());
            glib::spawn_future_local(async move {
                let result = ui
                    .call("plugin.get", json!({"plugin_id": plugin, "skill": name}))
                    .await;
                clear(&body);
                match result {
                    Ok(found) => {
                        body.append(&market::markdown(&ui, text(&found["skill"], "body")));
                        market.cache.borrow_mut().insert(key, found);
                    }
                    Err(error) => body.append(&paragraph(&explain(&error.to_string()))),
                }
            });
        });
        list.append(&expander);
    }
    list
}

fn tool_list(servers: &[Value]) -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 12);
    if servers.is_empty() {
        list.append(&label("This plugin adds no MCP servers.", "dim"));
    }
    for server in servers {
        let block = gtk::Box::new(gtk::Orientation::Vertical, 10);
        block.add_css_class("ext-server");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = label(text(server, "name"), "ext-server-name");
        name.set_hexpand(true);
        top.append(&name);
        let tools = tool_names(server);
        top.append(&label(&plural(tools.len(), "tool", "tools"), "ext-group-count"));
        block.append(&top);
        block.append(&market::prose(text(server, "description")));
        let tags: Vec<gtk::Label> = tools
            .iter()
            .map(|tool| market::chip(tool, "code"))
            .collect();
        block.append(&market::chips(&tags));
        list.append(&block);
    }
    let note = label(
        "Agents get these tools when they start or resume in a project with the plugin on.",
        "ext-hint",
    );
    note.set_wrap(true);
    list.append(&note);
    list
}

/// The rules every agent receives and the plugin's documents, fetched when the tab opens.
fn documents(ui: &Rc<Ui>, market: &Rc<Market>, plugin: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 20);
    let weak = Rc::downgrade(ui);
    let market = Rc::downgrade(market);
    let plugin = plugin.to_string();
    market::lazy(&page, move |page| {
        let (Some(ui), Some(market)) = (weak.upgrade(), market.upgrade()) else {
            return;
        };
        let cached = market.cache.borrow().get(&plugin).cloned();
        if let Some(found) = cached {
            fill_documents(&ui, page, &found);
            return;
        }
        page.append(&label("Loading rules and documents…", "dim"));
        let (page, plugin) = (page.clone(), plugin.clone());
        glib::spawn_future_local(async move {
            let result = ui.call("plugin.get", json!({"plugin_id": plugin})).await;
            clear(&page);
            match result {
                Ok(found) => {
                    fill_documents(&ui, &page, &found);
                    market.cache.borrow_mut().insert(plugin, found);
                }
                Err(error) => page.append(&paragraph(&explain(&error.to_string()))),
            }
        });
    });
    page
}

fn fill_documents(ui: &Ui, page: &gtk::Box, found: &Value) {
    let rules = market::section("Rules every agent receives");
    rules.append(&market::markdown(ui, text(found, "instructions")));
    page.append(&rules);
    for doc in rows(found, "docs") {
        let block = market::section(text(&doc, "path"));
        block.append(&market::markdown(ui, text(&doc, "body")));
        page.append(&block);
    }
}

fn empty(ui: &Rc<Ui>, _market: &Rc<Market>) -> gtk::Widget {
    let retry = crate::app::button("Check again", "");
    let weak = Rc::downgrade(ui);
    retry.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.refresh_page();
        }
    });
    market::state_box(
        "plugins",
        "No plugins found",
        &label(STALE_ENGINE, "dim"),
        &[retry.upcast()],
    )
    .upcast()
}
