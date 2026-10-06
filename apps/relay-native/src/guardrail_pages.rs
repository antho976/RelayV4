//! Guardrail surfaces: held actions in words rather than raw JSON, the exception requests a
//! refused agent sends (with the prompt that puts one in front of the user wherever they are),
//! the grants still in force, and the layered settings editor.
use crate::app::{button, label, rows, text, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[path = "guardrail_settings.rs"]
mod settings;
pub use settings::{editor as settings_editor, open_project_guardrails, open_workspace_guardrails};

const TRAY: &str = "guardrail-requests";
/// More than this many prompts at once is a queue, and the Guardrails page is the queue.
const TRAY_LIMIT: usize = 3;
/// How long a prompt takes to slide in, and to slide away once it is answered.
const PROMPT_IN_MS: u32 = 260;
const PROMPT_OUT_MS: u32 = 180;
/// The name a prompt takes while it slides away, so it no longer counts as showing.
const LEAVING: &str = "guardrail-leaving";

fn paragraph(value: &str, class: &str) -> gtk::Label {
    let l = label(value, class);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_selectable(true);
    l
}

fn mono(value: &str) -> gtk::Label {
    let l = paragraph(value, "guardrail-subject");
    l.add_css_class("mono");
    l
}

/// "4 min ago" from an RFC 3339 timestamp; empty when it does not parse.
fn ago(ts: &str) -> String {
    let (Ok(then), Ok(now)) = (glib::DateTime::from_iso8601(ts, None), glib::DateTime::now_utc()) else {
        return String::new();
    };
    let seconds = now.difference(&then).as_seconds().max(0);
    match seconds {
        0..=44 => "just now".into(),
        45..=5399 => format!("{} min ago", (seconds + 30) / 60),
        5400..=129_599 => format!("{} h ago", (seconds + 1800) / 3600),
        _ => format!("{} days ago", (seconds + 43_200) / 86_400),
    }
}

fn is_request(hold: &Value) -> bool {
    text(hold, "op") == "guardrail.request"
}

/// A request in the shape `guardrail.requests.list` returns, from either that or a hold row.
fn as_request(item: &Value) -> Value {
    if item.get("kind").is_some() {
        return item.clone();
    }
    let details = &item["details"];
    json!({
        "id": item["id"], "project_id": item["project_id"], "session": item["session"],
        "kind": details["kind"], "value": details["value"], "reason": details["reason"],
        "requested_scope": details["requested_scope"], "state": item["state"],
        "created_at": item["created_at"],
    })
}

fn kind_verb(kind: &str) -> &'static str {
    match kind {
        "command" => "run",
        "path" => "write to",
        _ => "commit past the change caps",
    }
}

fn scope_phrase(scope: &str) -> &'static str {
    if scope == "session" { "for the rest of the session" } else { "once" }
}

/// What a policy is called where a person reads it.
fn policy_title(policy: &str) -> &'static str {
    match policy {
        "exception" => "Exception request",
        "protected_path" => "Protected path",
        "destructive_write" => "Large rewrite",
        "shape_gate" => "File shape check",
        "write_root" => "Write outside the worktree",
        "denied_command" => "Denied command",
        "cap" => "Commit over the size caps",
        _ => "Guardrail hold",
    }
}

/// The policy that actually stopped the action. A hold the user tripped wraps the original.
fn effective(hold: &Value) -> (String, Value) {
    let details = &hold["details"];
    if details["original"].is_object() {
        (text(details, "policy").to_string(), details["original"].clone())
    } else {
        (text(hold, "policy").to_string(), details.clone())
    }
}

fn number(value: &Value) -> String {
    match value.as_f64() {
        Some(n) if n.fract() == 0.0 => format!("{n:.0}"),
        Some(n) => format!("{n:.1}"),
        None => "?".into(),
    }
}

/// One sentence saying why the action is waiting.
fn explain(policy: &str, details: &Value) -> String {
    let path = text(details, "path");
    match policy {
        "destructive_write" if details["reason"] == "comparison_cap" => {
            format!("{path} is too large to compare safely, so Relay cannot tell how much of it would be lost.")
        }
        "destructive_write" => {
            let share = details["removed_pct"]
                .as_f64()
                .map(|pct| format!(" ({pct:.0}% of {} lines)", number(&details["old_lines"])))
                .unwrap_or_default();
            format!(
                "Removes {} lines{share} and adds {}. The limit is {} lines or {}% of a file, and git cannot restore what is there now.",
                number(&details["removed_lines"]), number(&details["added_lines"]),
                number(&details["limit_lines"]), number(&details["limit_pct"]),
            )
        }
        "shape_gate" => format!(
            "{path} must pass the {} check: {}.",
            text(details, "validator").replace('_', " "),
            text(details, "reason"),
        ),
        "protected_path" => format!("{path} is protected by the pattern {}.", details["pattern"]),
        "write_root" => format!("{path} is outside every directory this session may write to."),
        "denied_command" => format!("Runs {}, which is on the denied list.", details["pattern"]),
        "cap" => format!(
            "Changes {} files and {} lines; the caps are {} files and {} lines.",
            number(&details["files"]), number(&details["lines"]),
            number(&details["cap_files"]), number(&details["cap_lines"]),
        ),
        _ => String::new(),
    }
}

/// The thing being acted on, for the meta line.
fn subject(hold: &Value, details: &Value) -> String {
    let op = text(hold, "op");
    if let Some(command) = details["command"].as_str() {
        return command.to_string();
    }
    if let Some(path) = details["path"].as_str() {
        return path.to_string();
    }
    match op {
        "guardrail.gate" | "" => String::new(),
        other => other.to_string(),
    }
}

/// Title and one-line summary, for compact lists such as the dashboard's decision queue.
pub fn hold_summary(hold: &Value) -> (String, String) {
    if is_request(hold) {
        let request = as_request(hold);
        return (
            format!("{} asks for an exception", text(&request, "session")),
            format!("{} {}", kind_verb(text(&request, "kind")), text(&request, "value")),
        );
    }
    let (policy, details) = effective(hold);
    let session = text(hold, "session");
    let what = subject(hold, &details);
    (
        format!("Guardrail: {}", policy_title(&policy).to_lowercase()),
        [session, what.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · "),
    )
}

/// The label/value pairs worth showing under "Details", in reading order.
fn detail_pairs(details: &Value) -> Vec<(String, String)> {
    const NAMES: &[(&str, &str)] = &[
        ("path", "Path"), ("command", "Command"), ("pattern", "Matched rule"), ("validator", "Check"),
        ("reason", "Why"), ("removed_lines", "Lines removed"), ("added_lines", "Lines added"),
        ("old_lines", "Lines in the file"), ("removed_pct", "Share removed (%)"),
        ("limit_lines", "Line limit"), ("limit_pct", "Share limit (%)"), ("files", "Files changed"),
        ("lines", "Lines changed"), ("cap_files", "File cap"), ("cap_lines", "Line cap"),
        ("bytes", "Size (bytes)"),
    ];
    NAMES
        .iter()
        .filter_map(|(key, name)| {
            let value = details.get(*key)?;
            let shown = match value {
                Value::Null => return None,
                Value::String(s) => s.clone(),
                Value::Number(_) => number(value),
                other => other.to_string(),
            };
            Some((name.to_string(), shown))
        })
        .collect()
}

fn details_grid(details: &Value) -> Option<gtk::Expander> {
    let pairs = detail_pairs(details);
    if pairs.is_empty() {
        return None;
    }
    let grid = gtk::Grid::builder().column_spacing(16).row_spacing(4).build();
    grid.add_css_class("guardrail-details");
    for (row, (name, value)) in pairs.iter().enumerate() {
        grid.attach(&label(name, "faint"), 0, row as i32, 1, 1);
        let value = paragraph(value, "body");
        value.set_hexpand(true);
        grid.attach(&value, 1, row as i32, 1, 1);
    }
    let expander = gtk::Expander::new(Some("Details"));
    expander.add_css_class("guardrail-expander");
    expander.set_child(Some(&grid));
    Some(expander)
}

fn head(eyebrow: &str, when: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let mark = crate::icons::image("shield", 14);
    mark.add_css_class("guardrail-mark");
    row.append(&mark);
    let title = label(eyebrow, "section-label");
    title.add_css_class("guardrail-eyebrow");
    title.set_hexpand(true);
    row.append(&title);
    if !when.is_empty() {
        row.append(&label(when, "faint"));
    }
    row
}

/// A held action, in words: what it was, why it stopped, and what you can do.
pub fn hold_row(ui: &Rc<Ui>, body: &gtk::Box, hold: Value) {
    let (policy, details) = effective(&hold);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("record");
    card.add_css_class("guardrail-card");
    card.append(&head(policy_title(&policy), &ago(text(&hold, "created_at"))));
    let who = text(&hold, "session");
    let what = subject(&hold, &details);
    let heading = label(
        &if who.is_empty() || text(&hold, "actor") == "user" {
            "Your own action is held".to_string()
        } else {
            format!("{who} is waiting on you")
        },
        "title",
    );
    card.append(&heading);
    if !what.is_empty() {
        card.append(&mono(&what));
    }
    let why = explain(&policy, &details);
    if !why.is_empty() {
        card.append(&paragraph(&why, "body"));
    }
    if let Some(grid) = details_grid(&details) {
        card.append(&grid);
    }
    let exact = mono("");
    exact.add_css_class("guardrail-exact");
    exact.set_visible(false);
    card.append(&exact);
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    keys.add_css_class("guardrail-keys");
    let inspect = button("Review exact action", "quiet");
    let allow = button("Allow once", "primary");
    allow.set_sensitive(false);
    allow.set_tooltip_text(Some("Review the exact action first"));
    let reject = button("Reject…", "");
    keys.append(&allow);
    keys.append(&reject);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    keys.append(&spacer);
    keys.append(&inspect);
    card.append(&keys);
    let id = hold["id"].clone();
    let denial = denial_form(ui, &card, &keys, id.clone(), "Reason (sent to the agent, optional)");
    reject.connect_clicked(move |_| denial());

    let weak = Rc::downgrade(ui);
    let hold_id = id.clone();
    allow.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else { return };
        key.set_sensitive(false);
        let key = key.clone();
        let id = hold_id.clone();
        glib::spawn_future_local(async move {
            match ui.call("guardrail.confirm", json!({"hold_id": id})).await {
                Ok(v) => {
                    if v["outcome"]["ok"] == false {
                        let error = &v["outcome"]["error"];
                        ui.show_error(&format!("Allowed, but the action then failed: {}", text(error, "message")));
                    }
                    ui.refresh_page();
                }
                Err(e) => {
                    ui.show_error(&e.to_string());
                    key.set_sensitive(true);
                }
            }
        });
    });
    let weak = Rc::downgrade(ui);
    let allow_key = allow.downgrade();
    inspect.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else { return };
        if exact.is_visible() {
            exact.set_visible(false);
            key.set_label("Review exact action");
            return;
        }
        key.set_sensitive(false);
        let key = key.clone();
        let exact = exact.clone();
        let allow_key = allow_key.clone();
        let id = id.clone();
        glib::spawn_future_local(async move {
            match ui.call("guardrail.hold.get", json!({"hold_id": id})).await {
                Ok(v) => {
                    let request = &v["request"];
                    let mut shown = format!("{}\n", text(request, "op"));
                    shown.push_str(&serde_json::to_string_pretty(&request["payload"]).unwrap_or_default());
                    exact.set_text(&shown);
                    exact.set_visible(true);
                    key.set_label("Hide exact action");
                    if let Some(allow) = allow_key.upgrade() {
                        allow.set_sensitive(true);
                        allow.set_tooltip_text(None);
                    }
                }
                Err(e) => ui.show_error(&format!("Cannot inspect this hold: {e}")),
            }
            key.set_sensitive(true);
        });
    });
    body.append(&card);
}

/// An inline "why not" form under `keys`, sending `guardrail.reject`. Returns the opener.
fn denial_form(ui: &Rc<Ui>, card: &gtk::Box, keys: &gtk::Box, id: Value, placeholder: &str) -> impl Fn() + 'static {
    let form = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    form.add_css_class("guardrail-deny");
    let reason = gtk::Entry::builder().placeholder_text(placeholder).hexpand(true).build();
    let send = button("Send denial", "primary");
    let cancel = button("Cancel", "quiet");
    form.append(&reason);
    form.append(&send);
    form.append(&cancel);
    form.set_visible(false);
    card.append(&form);
    let weak_keys = keys.downgrade();
    let weak_form = form.downgrade();
    cancel.connect_clicked(move |_| {
        if let (Some(keys), Some(form)) = (weak_keys.upgrade(), weak_form.upgrade()) {
            form.set_visible(false);
            keys.set_visible(true);
        }
    });
    let weak = Rc::downgrade(ui);
    let entry = reason.downgrade();
    let submit = move |key: &gtk::Button| {
        let (Some(ui), Some(entry)) = (weak.upgrade(), entry.upgrade()) else { return };
        let reason = entry.text().trim().to_string();
        key.set_sensitive(false);
        let key = key.clone();
        let id = id.clone();
        glib::spawn_future_local(async move {
            let reason = (!reason.is_empty()).then_some(reason);
            match ui.call("guardrail.reject", json!({"hold_id": id, "reason": reason})).await {
                Ok(_) => {
                    close_prompt(&ui, id.as_i64().unwrap_or_default());
                    ui.refresh_page();
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            key.set_sensitive(true);
        });
    };
    let send_key = send.clone();
    reason.connect_activate(move |_| send_key.emit_clicked());
    send.connect_clicked(submit);
    let keys = keys.downgrade();
    let form = form.downgrade();
    let reason = reason.downgrade();
    move || {
        if let (Some(keys), Some(form), Some(entry)) = (keys.upgrade(), form.upgrade(), reason.upgrade()) {
            keys.set_visible(false);
            form.set_visible(true);
            entry.grab_focus();
        }
    }
}

/// An agent's request for an exception, with the three answers. `compact` is the prompt form.
pub fn exception_card(ui: &Rc<Ui>, request: &Value, compact: bool) -> gtk::Box {
    let request = as_request(request);
    let id = request["id"].as_i64().unwrap_or_default();
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("guardrail-card");
    card.add_css_class("guardrail-request");
    if compact {
        card.add_css_class("guardrail-prompt");
    } else {
        card.add_css_class("record");
    }
    card.set_widget_name(&format!("guardrail-request-{id}"));
    card.append(&head("Exception request", &ago(text(&request, "created_at"))));
    let session = text(&request, "session");
    let kind = text(&request, "kind");
    let value = text(&request, "value");
    let heading = label(&format!("{session} asks to {}", kind_verb(kind)), "title");
    heading.set_wrap(true);
    heading.set_xalign(0.0);
    card.append(&heading);
    if !value.is_empty() {
        card.append(&mono(value));
    }
    let reason = text(&request, "reason");
    if !reason.is_empty() {
        let why = paragraph(&format!("“{reason}”"), "body");
        why.add_css_class("guardrail-reason");
        card.append(&why);
    }
    let asked = text(&request, "requested_scope");
    card.append(&label(
        &format!(
            "Asked for {}. Only this rule is lifted, and only for {session}.",
            if asked == "session" { "the rest of the session" } else { "one use" },
        ),
        "faint",
    ));
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    keys.add_css_class("guardrail-keys");
    let once = button("Approve once", if asked == "session" { "" } else { "primary" });
    let always = button("Approve for this session", if asked == "session" { "primary" } else { "" });
    let deny = button("Deny…", "quiet");
    for key in [&once, &always, &deny] {
        keys.append(key);
    }
    if compact {
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        keys.append(&spacer);
        let later = crate::app::icon_button("close", "Answer later from Guardrails");
        let weak = Rc::downgrade(ui);
        later.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                close_prompt(&ui, id);
            }
        });
        keys.append(&later);
    }
    card.append(&keys);
    let opener = denial_form(ui, &card, &keys, json!(id), "Why not? The agent reads this");
    deny.connect_clicked(move |_| opener());
    for (key, scope) in [(&once, "once"), (&always, "session")] {
        let weak = Rc::downgrade(ui);
        let all = keys.downgrade();
        key.connect_clicked(move |_| {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(keys) = all.upgrade() {
                keys.set_sensitive(false);
            }
            let all = all.clone();
            glib::spawn_future_local(async move {
                match ui.call("guardrail.confirm", json!({"hold_id": id, "scope": scope})).await {
                    Ok(_) => {
                        close_prompt(&ui, id);
                        ui.show_error(&format!("Exception approved {}. The agent has been told to retry.", scope_phrase(scope)));
                        ui.refresh_page();
                    }
                    Err(e) => {
                        ui.show_error(&e.to_string());
                        if let Some(keys) = all.upgrade() {
                            keys.set_sensitive(true);
                        }
                    }
                }
            });
        });
    }
    card
}

fn tray(ui: &Rc<Ui>) -> gtk::Box {
    let mut child = ui.overlay.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == TRAY {
            if let Ok(tray) = widget.downcast::<gtk::Box>() {
                return tray;
            }
            break;
        }
        child = widget.next_sibling();
    }
    // No spacing: each card carries its own gap, so the gap closes with it as it slides away.
    let tray = gtk::Box::new(gtk::Orientation::Vertical, 0);
    tray.set_widget_name(TRAY);
    tray.add_css_class("guardrail-tray");
    tray.set_halign(gtk::Align::End);
    tray.set_valign(gtk::Align::End);
    tray.set_margin_end(16);
    tray.set_margin_bottom(40);
    tray.set_size_request(420, -1);
    // The prompt waits beside whatever you are doing; it never takes keyboard focus.
    tray.set_focus_on_click(false);
    ui.overlay.add_overlay(&tray);
    tray
}

fn prompt_ids(tray: &gtk::Box) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut child = tray.first_child();
    while let Some(widget) = child {
        if let Some(id) = widget.widget_name().strip_prefix("guardrail-request-").and_then(|id| id.parse().ok()) {
            ids.push(id);
        }
        child = widget.next_sibling();
    }
    ids
}

fn update_overflow(ui: &Rc<Ui>, tray: &gtk::Box, hidden: usize) {
    let mut child = tray.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.widget_name() == "guardrail-more" {
            tray.remove(&widget);
        }
    }
    if hidden == 0 {
        return;
    }
    let more = button(&format!("{hidden} more waiting · open Guardrails"), "quiet");
    more.set_widget_name("guardrail-more");
    more.add_css_class("guardrail-more");
    let weak = Rc::downgrade(ui);
    more.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.navigate("guardrails");
        }
    });
    tray.append(&more);
}

thread_local! {
    /// Requests that arrived while the tray was full, oldest first.
    static WAITING: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    static DISMISSED: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
}

/// Put one request in front of the user, without taking focus from a terminal.
fn show_prompt(ui: &Rc<Ui>, request: &Value) {
    let id = request["id"].as_i64().unwrap_or_default();
    if DISMISSED.with(|d| d.borrow().contains(&id)) {
        return;
    }
    let tray = tray(ui);
    if prompt_ids(&tray).contains(&id) {
        return;
    }
    if prompt_ids(&tray).len() >= TRAY_LIMIT {
        WAITING.with(|w| {
            let mut waiting = w.borrow_mut();
            if !waiting.iter().any(|r| r["id"] == request["id"]) {
                waiting.push(request.clone());
            }
        });
    } else {
        // The overflow line is re-added below, so it always ends the tray.
        tray.append(&prompt_slot(&exception_card(ui, request, true), id));
    }
    let hidden = WAITING.with(|w| w.borrow().len());
    update_overflow(ui, &tray, hidden);
}

/// A prompt card inside a revealer: it slides up and fades in on the next frame, rather than
/// appearing all at once over whatever you are doing.
fn prompt_slot(card: &gtk::Box, id: i64) -> gtk::Revealer {
    let slot = gtk::Revealer::new();
    slot.set_widget_name(&format!("guardrail-request-{id}"));
    slot.set_transition_type(gtk::RevealerTransitionType::SlideUp);
    slot.set_transition_duration(PROMPT_IN_MS);
    card.add_css_class("leaving");
    slot.set_child(Some(card));
    // A revealer clips its child, shadow included. Clip only while sliding.
    slot.connect_child_revealed_notify(|slot| {
        if slot.is_child_revealed() {
            slot.set_overflow(gtk::Overflow::Visible);
        }
    });
    // Started from a frame callback so the hidden state is drawn once and the fade has
    // something to fade from.
    slot.add_tick_callback(|slot, _| {
        if slot.widget_name() == LEAVING {
            return glib::ControlFlow::Break;
        }
        slot.set_reveal_child(true);
        if let Some(card) = slot.child() {
            card.remove_css_class("leaving");
        }
        glib::ControlFlow::Break
    });
    slot
}

/// Slide a prompt away, then remove it. Without animations (or off screen) the revealer
/// reports itself hidden at once, and it is removed at once.
fn retire(tray: &gtk::Box, slot: &gtk::Widget) {
    let Some(revealer) = slot.downcast_ref::<gtk::Revealer>() else {
        tray.remove(slot);
        return;
    };
    revealer.set_widget_name(LEAVING);
    if !revealer.reveals_child() && !revealer.is_child_revealed() {
        // Answered before its first frame: there is nothing to slide away.
        tray.remove(revealer);
        return;
    }
    revealer.set_can_target(false);
    revealer.set_overflow(gtk::Overflow::Hidden);
    if let Some(card) = revealer.child() {
        card.add_css_class("leaving");
    }
    let weak = tray.downgrade();
    revealer.connect_child_revealed_notify(move |revealer| {
        if revealer.is_child_revealed() {
            return;
        }
        if let Some(tray) = weak.upgrade() {
            if revealer.parent().as_ref() == Some(tray.upcast_ref::<gtk::Widget>()) {
                tray.remove(revealer);
            }
        }
    });
    revealer.set_transition_duration(PROMPT_OUT_MS);
    revealer.set_reveal_child(false);
}

/// Take a request's prompt down (answered, expired, or put off), and let a waiting one in.
fn close_prompt(ui: &Rc<Ui>, id: i64) {
    WAITING.with(|w| w.borrow_mut().retain(|r| r["id"].as_i64() != Some(id)));
    let tray = tray(ui);
    let mut child = tray.first_child();
    let mut removed = false;
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.widget_name() == format!("guardrail-request-{id}") {
            retire(&tray, &widget);
            removed = true;
        }
    }
    if removed {
        DISMISSED.with(|d| d.borrow_mut().push(id));
        if let Some(next) = WAITING.with(|w| (!w.borrow().is_empty()).then(|| w.borrow_mut().remove(0))) {
            show_prompt(ui, &next);
        }
    }
    let hidden = WAITING.with(|w| w.borrow().len());
    update_overflow(ui, &tray, hidden);
}

/// Every guardrail event passes through here before the page refresh, whatever page or
/// project is showing: a request for an exception has to reach the user wherever they are.
pub fn guardrail_event(ui: &Rc<Ui>, ev: &str, payload: &Value) {
    match ev {
        "guardrail.held" if payload["policy"] == "exception" => {
            let Some(id) = payload["request_id"].as_i64().or(payload["hold_id"].as_i64()) else { return };
            let ui = ui.clone();
            glib::spawn_future_local(async move {
                if let Ok(request) = ui.call("guardrail.request.get", json!({"request_id": id})).await {
                    if request["state"] == "open" {
                        show_prompt(&ui, &request);
                    }
                }
            });
        }
        "guardrail.resolved" => {
            if let Some(id) = payload["hold_id"].as_i64() {
                close_prompt(ui, id);
            } else if payload["state"] == "expired" {
                // A closed session takes its open requests with it.
                restore_prompts(ui);
            }
        }
        _ => {}
    }
}

/// Bring back prompts for requests that are still open — after a reconnect, or when a
/// session closed and the set may have shrunk.
pub fn restore_prompts(ui: &Rc<Ui>) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let Ok(open) = ui.call("guardrail.requests.list", json!({"state": "open"})).await else { return };
        let open = rows(&open, "requests");
        let tray = tray(&ui);
        for id in prompt_ids(&tray) {
            if !open.iter().any(|r| r["id"].as_i64() == Some(id)) {
                close_prompt(&ui, id);
            }
        }
        WAITING.with(|w| w.borrow_mut().retain(|r| open.iter().any(|o| o["id"] == r["id"])));
        // Oldest first: the one that has waited longest is answered first.
        for request in open.iter().rev() {
            show_prompt(&ui, request);
        }
    });
}

fn section(body: &gtk::Box, title: &str, hint: &str) {
    let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
    head.add_css_class("guardrail-section");
    head.append(&label(title, "section-label"));
    if !hint.is_empty() {
        let hint = paragraph(hint, "faint");
        hint.set_selectable(false);
        head.append(&hint);
    }
    body.append(&head);
}

/// The Guardrails page body: requests first, then held actions, then the grants in force.
pub async fn page(ui: &Rc<Ui>, body: &gtk::Box, project: i64, holds: Vec<Value>) {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("guardrail-actions");
    let edit = button("Guardrail settings for this project", "");
    let weak = Rc::downgrade(ui);
    edit.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            open_project_guardrails(&ui, project);
        }
    });
    actions.append(&edit);
    let workspace = ui.projects.borrow().iter()
        .find(|p| p["id"].as_i64() == Some(project))
        .and_then(|p| p["workspace_id"].as_i64());
    if let Some(workspace) = workspace {
        let edit = button("Workspace guardrails", "quiet");
        let weak = Rc::downgrade(ui);
        edit.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                open_workspace_guardrails(&ui, workspace);
            }
        });
        actions.append(&edit);
    }
    body.prepend(&actions);

    let (requests, held): (Vec<Value>, Vec<Value>) = holds.into_iter().partition(is_request);
    if !requests.is_empty() {
        section(body, "EXCEPTION REQUESTS", "Agents that cannot progress without passing a guardrail. Approving lifts only the named rule, for that session.");
        for request in &requests {
            body.append(&exception_card(ui, request, false));
        }
    }
    if !held.is_empty() {
        section(body, "HELD ACTIONS", "Paused until you decide. Review the exact action before allowing it.");
        for hold in held {
            hold_row(ui, body, hold);
        }
    }
    let active = ui.call("guardrail.requests.list", json!({"project_id": project, "state": "active"})).await;
    if ui.project.get() != project || *ui.page.borrow() != "guardrails" {
        return;
    }
    match active {
        Ok(active) => {
            let active = rows(&active, "requests");
            if !active.is_empty() {
                section(body, "ACTIVE EXCEPTIONS", "Approved exceptions still in force. Revoke one to put the rule back immediately.");
                for grant in active {
                    body.append(&grant_row(ui, &grant));
                }
            }
        }
        Err(e) => ui.show_error(&format!("Exceptions: {e}")),
    }
}

/// One approved exception, with Revoke.
fn grant_row(ui: &Rc<Ui>, grant: &Value) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("record");
    row.add_css_class("guardrail-grant");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 4);
    copy.set_hexpand(true);
    let scope = text(grant, "scope");
    copy.append(&label(
        &format!("{} may {}", text(grant, "session"), kind_verb(text(grant, "kind"))),
        "body",
    ));
    if !text(grant, "value").is_empty() {
        copy.append(&mono(text(grant, "value")));
    }
    let uses = grant["uses"].as_u64().unwrap_or(0);
    let lasting = if scope == "session" {
        match uses {
            0 => "For this session · not used yet".to_string(),
            1 => "For this session · used once".to_string(),
            n => format!("For this session · used {n} times"),
        }
    } else {
        "Once · not used yet".to_string()
    };
    let approved = ago(text(grant, "resolved_at"));
    copy.append(&label(
        &if approved.is_empty() { lasting } else { format!("{lasting} · approved {approved}") },
        "faint",
    ));
    row.append(&copy);
    let revoke = button("Revoke", "");
    revoke.set_valign(gtk::Align::Center);
    let weak = Rc::downgrade(ui);
    let id = grant["id"].clone();
    revoke.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else { return };
        key.set_sensitive(false);
        let key = key.clone();
        let id = id.clone();
        glib::spawn_future_local(async move {
            match ui.call("guardrail.grant.revoke", json!({"request_id": id})).await {
                Ok(_) => ui.refresh_page(),
                Err(e) => {
                    ui.show_error(&e.to_string());
                    key.set_sensitive(true);
                }
            }
        });
    });
    row.append(&revoke);
    row
}
