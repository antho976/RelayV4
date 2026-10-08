//! The Threads page: the thread list in the sidebar, the conversation with its message box, and
//! the Tally panel beside it (docs/THREADS.md).
//!
//! Threads are the engine's (`thread.*`); this page draws them and follows `thread.changed`,
//! `thread.message` and `thread.delta`. The page is one stack page, `threads`, whose middle shows
//! either a new thread (a greeting and the message box) or the open one.
use super::pages::{badge, human_date, meter_in, tone, tone_class};
use crate::app::{button, clear, label, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use relay_client::thread_view::{chart_spec, markdown, tool_caption, tool_op, tool_writes, Block};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// The stack page.
pub const PAGE: &str = "threads";

/// Questions a new thread offers: (what is asked, what it does).
const SUGGESTIONS: [(&str, &str); 4] = [
    ("Where did my money go this week?", "Spending by category"),
    ("Am I on pace this month?", "Each budget against the day of the month"),
    ("What bills are coming up?", "Due dates and whether you can cover them"),
    ("Which entries have no category?", "It suggests one for each, you approve"),
];

/// The Tally panel's views: (key, caption).
const PANEL_TABS: [(&str, &str); 3] = [("overview", "Overview"), ("entries", "Entries"), ("budgets", "Budgets")];

struct View {
    title: gtk::Label,
    title_key: gtk::MenuButton,
    rename: gtk::Entry,
    middle: gtk::Stack,
    empty_slot: gtk::Box,
    empty_line: gtk::Label,
    dock: gtk::Box,
    composer: gtk::Box,
    input: gtk::TextView,
    send: gtk::Button,
    scroll: gtk::ScrolledWindow,
    column: gtk::Box,
    /// The reply being written: its label, shown while `thread.delta` text arrives.
    pending: gtk::Label,
    working: gtk::Box,
    panel: gtk::Revealer,
    panel_body: gtk::Box,
    panel_tabs: Vec<(&'static str, gtk::Button)>,
}

#[derive(Default)]
struct State {
    view: RefCell<Option<Rc<View>>>,
    list: RefCell<Option<gtk::Box>>,
    current: Cell<Option<i64>>,
    working: Cell<bool>,
    /// The newest message drawn, so an event already drawn by a read is not drawn twice.
    last: Cell<i64>,
    pending: RefCell<String>,
    /// Tool calls drawn in this thread, by id: where their card goes and what they called.
    tools: RefCell<HashMap<String, (gtk::Box, String)>>,
    /// The box the current agent turn's pieces go in.
    turn: RefCell<Option<gtk::Box>>,
    panel_tab: Cell<&'static str>,
    listing: Cell<bool>,
    ui: RefCell<std::rc::Weak<Ui>>,
}

thread_local! {
    static STATE: State = State::default();
}

/// Remember the window, for the page's own keys: `money::install` calls it.
pub fn install(ui: &Rc<Ui>) {
    STATE.with(|s| *s.ui.borrow_mut() = Rc::downgrade(ui));
    // Display smoke runs (RELAY_NATIVE_SCREENSHOT) open a thread by id to capture it.
    if std::env::var_os("RELAY_NATIVE_SCREENSHOT").is_some() {
        if let Some(id) = std::env::var("RELAY_NATIVE_THREAD").ok().and_then(|id| id.parse::<i64>().ok()) {
            let weak = Rc::downgrade(ui);
            glib::timeout_add_local_once(std::time::Duration::from_millis(2600), move || {
                if let Some(ui) = weak.upgrade() {
                    open(&ui, id);
                }
            });
        }
    }
}

fn the_ui() -> Option<Rc<Ui>> {
    STATE.with(|s| s.ui.borrow().upgrade())
}

/// The window, for keys built before it existed (the Tally tabs).
pub fn the_ui_pub() -> Option<Rc<Ui>> {
    the_ui()
}

fn view() -> Option<Rc<View>> {
    STATE.with(|s| s.view.borrow().clone())
}

/// The id of the thread showing, if one is.
pub fn current() -> Option<i64> {
    STATE.with(|s| s.current.get())
}

/// The page, built once and added to the window's stack by `money::add_pages`.
pub fn page() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("threads-root");

    // The strip above the conversation: the thread's name (its menu renames and deletes) and
    // the Tally panel's key.
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    strip.add_css_class("workspace-strip");
    strip.add_css_class("threads-strip");
    let title = label("New thread", "threads-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(48);
    let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    title_row.append(&title);
    title_row.append(&crate::icons::image("chevron-down", 12));
    let title_key = gtk::MenuButton::new();
    title_key.set_child(Some(&title_row));
    title_key.add_css_class("threads-title-key");
    title_key.set_widget_name("threads-title");
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 6);
    menu.add_css_class("threads-menu");
    let rename = gtk::Entry::new();
    rename.set_placeholder_text(Some("Thread name"));
    rename.set_width_chars(28);
    menu.append(&label("Rename", "money-label"));
    menu.append(&rename);
    let delete = button("Delete thread", "money-destructive");
    menu.append(&delete);
    let popover = gtk::Popover::new();
    popover.set_child(Some(&menu));
    title_key.set_popover(Some(&popover));
    strip.append(&title_key);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    strip.append(&spacer);
    let panel_key = crate::app::icon_button("sidebar-show-symbolic", "Show or hide the Tally panel");
    panel_key.set_widget_name("threads-panel-key");
    strip.append(&panel_key);
    root.append(&strip);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_vexpand(true);
    let conversation = gtk::Box::new(gtk::Orientation::Vertical, 0);
    conversation.set_hexpand(true);

    let middle = gtk::Stack::new();
    middle.set_vexpand(true);
    middle.set_transition_type(gtk::StackTransitionType::Crossfade);
    middle.set_transition_duration(120);

    // A new thread: the greeting, the month in a line, the message box, and questions to start.
    let empty = gtk::Box::new(gtk::Orientation::Vertical, 22);
    empty.add_css_class("threads-empty");
    empty.set_valign(gtk::Align::Center);
    empty.set_halign(gtk::Align::Center);
    empty.set_size_request(640, -1);
    let greeting = label(&super::greeting(), "threads-greeting");
    greeting.set_xalign(0.5);
    let empty_line = label("", "threads-greeting-line");
    empty_line.set_xalign(0.5);
    empty_line.set_wrap(true);
    let words = gtk::Box::new(gtk::Orientation::Vertical, 6);
    words.append(&greeting);
    words.append(&empty_line);
    empty.append(&words);
    let empty_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    empty.append(&empty_slot);
    let suggestions = gtk::Grid::new();
    suggestions.set_row_spacing(8);
    suggestions.set_column_spacing(8);
    suggestions.set_column_homogeneous(true);
    for (index, (ask, about)) in SUGGESTIONS.iter().enumerate() {
        let key = gtk::Button::new();
        key.add_css_class("threads-suggestion");
        let words = gtk::Box::new(gtk::Orientation::Vertical, 3);
        words.append(&label(ask, "threads-suggestion-ask"));
        words.append(&label(about, "threads-suggestion-about"));
        key.set_child(Some(&words));
        let ask = ask.to_string();
        key.connect_clicked(move |_| {
            if let Some(ui) = the_ui() {
                send(&ui, &ask);
            }
        });
        suggestions.attach(&key, (index % 2) as i32, (index / 2) as i32, 1, 1);
    }
    empty.append(&suggestions);
    middle.add_named(&empty, Some("empty"));

    // The open thread: its messages in a centred column, scrolled.
    let column = gtk::Box::new(gtk::Orientation::Vertical, 22);
    column.add_css_class("threads-column");
    let clamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    clamp.set_halign(gtk::Align::Center);
    column.set_size_request(680, -1);
    column.set_hexpand(false);
    clamp.append(&column);
    let scroll = crate::app::scrolled(&clamp);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.add_css_class("threads-scroll");
    middle.add_named(&scroll, Some("thread"));
    conversation.append(&middle);

    let pending = label("", "threads-reply");
    pending.set_wrap(true);
    pending.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    pending.set_selectable(false);
    let working = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    working.add_css_class("threads-working");
    let spinner = gtk::Spinner::new();
    spinner.start();
    working.append(&spinner);
    working.append(&label("Working…", "threads-working-text"));

    // The message box: what you write, where it goes, and send or stop.
    let composer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    composer.add_css_class("threads-composer");
    let input = gtk::TextView::new();
    input.set_wrap_mode(gtk::WrapMode::WordChar);
    input.add_css_class("threads-input");
    input.set_widget_name("threads-input");
    input.set_accepts_tab(false);
    input.update_property(&[gtk::accessible::Property::Label("Message")]);
    let input_scroll = gtk::ScrolledWindow::builder().child(&input).propagate_natural_height(true).max_content_height(180).build();
    input_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    composer.append(&input_scroll);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let source = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    source.add_css_class("threads-chip");
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("threads-dot");
    dot.set_valign(gtk::Align::Center);
    source.append(&dot);
    source.append(&label("Tally", ""));
    source.set_tooltip_text(Some("This thread's agent reads and changes your Tally budget"));
    row.append(&source);
    let limits = label("Can edit, asks before deleting", "threads-chip");
    limits.set_tooltip_text(Some("It adds and changes entries, each with an Undo. Deleting, budgets and accounts stay yours."));
    row.append(&limits);
    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_hexpand(true);
    row.append(&gap);
    let send_key = gtk::Button::new();
    send_key.add_css_class("primary");
    send_key.add_css_class("threads-send");
    send_key.set_widget_name("threads-send");
    send_key.set_child(Some(&crate::icons::image_with_stroke("arrow-up", 15, 2.0)));
    send_key.set_tooltip_text(Some("Send · Enter"));
    row.append(&send_key);
    composer.append(&row);
    let dock = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dock.add_css_class("threads-dock");
    dock.set_halign(gtk::Align::Center);
    conversation.append(&dock);
    body.append(&conversation);

    // The Tally panel.
    let panel = gtk::Revealer::new();
    panel.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
    panel.set_transition_duration(160);
    panel.set_reveal_child(true);
    let side = gtk::Box::new(gtk::Orientation::Vertical, 18);
    side.add_css_class("threads-panel");
    side.set_size_request(340, -1);
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tabs.add_css_class("view-tabs");
    tabs.set_homogeneous(true);
    let mut panel_tabs = Vec::new();
    for (key, caption) in PANEL_TABS {
        let tab = button(caption, "quiet");
        tab.set_widget_name(&format!("threads-panel-{key}"));
        tabs.append(&tab);
        panel_tabs.push((key, tab));
    }
    side.append(&tabs);
    let panel_body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let panel_scroll = crate::app::scrolled(&panel_body);
    panel_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    side.append(&panel_scroll);
    panel.set_child(Some(&side));
    body.append(&panel);
    root.append(&body);

    let built = Rc::new(View {
        title, title_key, rename, middle, empty_slot, empty_line, dock, composer, input,
        send: send_key, scroll, column, pending, working, panel, panel_body, panel_tabs,
    });
    STATE.with(|s| {
        s.panel_tab.set("overview");
        *s.view.borrow_mut() = Some(built.clone());
    });
    wire(&built, &delete, &panel_key);
    show_empty(&built);
    root
}

fn wire(v: &Rc<View>, delete: &gtk::Button, panel_key: &gtk::Button) {
    // Enter sends; Shift+Enter is a new line.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
            if let Some(ui) = the_ui() {
                submit(&ui);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    v.input.add_controller(keys);
    v.send.connect_clicked(|_| {
        if let Some(ui) = the_ui() {
            if STATE.with(|s| s.working.get()) {
                stop(&ui);
            } else {
                submit(&ui);
            }
        }
    });
    v.rename.connect_activate(|entry| {
        let title = entry.text().trim().to_string();
        let (Some(ui), Some(id)) = (the_ui(), current()) else { return };
        if title.is_empty() {
            return;
        }
        if let Some(popover) = view().and_then(|v| v.title_key.popover()) {
            popover.popdown();
        }
        glib::spawn_future_local(async move {
            if let Err(e) = ui.call("thread.rename", json!({"id": id, "title": title})).await {
                ui.show_error(&e.to_string());
            }
        });
    });
    crate::app::confirm_inline(delete, "Delete this thread?", |_| {
        let (Some(ui), Some(id)) = (the_ui(), current()) else { return };
        if let Some(popover) = view().and_then(|v| v.title_key.popover()) {
            popover.popdown();
        }
        glib::spawn_future_local(async move {
            match ui.call("thread.delete", json!({"id": id})).await {
                Ok(_) => new_thread(&ui),
                Err(e) => ui.show_error(&e.to_string()),
            }
        });
    });
    let panel = v.panel.clone();
    panel_key.connect_clicked(move |_| {
        let open = !panel.reveals_child();
        panel.set_reveal_child(open);
        if open {
            if let Some(ui) = the_ui() {
                refresh_panel(&ui);
            }
        }
    });
    for (key, tab) in &v.panel_tabs {
        let key = *key;
        tab.connect_clicked(move |_| {
            STATE.with(|s| s.panel_tab.set(key));
            if let Some(ui) = the_ui() {
                refresh_panel(&ui);
            }
        });
    }
    v.title_key.connect_notify_local(Some("active"), |key, _| {
        if key.is_active() {
            if let Some(v) = view() {
                v.rename.set_text(&v.title.text());
            }
        }
    });
}

/// The message box goes where the middle is: centred on a new thread, at the foot of an open one.
fn place_composer(v: &View, empty: bool) {
    let target = if empty { &v.empty_slot } else { &v.dock };
    if v.composer.parent().as_ref() != Some(target.upcast_ref()) {
        if let Some(parent) = v.composer.parent().and_downcast::<gtk::Box>() {
            parent.remove(&v.composer);
        }
        target.append(&v.composer);
    }
    v.dock.set_visible(!empty);
}

fn show_empty(v: &View) {
    v.middle.set_visible_child_name("empty");
    v.title.set_text("New thread");
    v.title_key.set_visible(false);
    place_composer(v, true);
    set_working(v, false);
}

/// Start a new thread: the page shows the greeting until the first message is sent.
pub fn new_thread(ui: &Rc<Ui>) {
    STATE.with(|s| {
        s.current.set(None);
        s.last.set(0);
        s.pending.borrow_mut().clear();
        s.tools.borrow_mut().clear();
        *s.turn.borrow_mut() = None;
    });
    if let Some(v) = view() {
        show_empty(&v);
        v.empty_line.set_text("");
        v.input.grab_focus();
    }
    if *ui.page.borrow() != PAGE {
        ui.navigate(PAGE);
    }
    refresh_list(ui);
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        if let Ok(summary) = ui.call("money.summary", json!({})).await {
            if let Some(v) = view() {
                v.empty_line.set_text(&month_line(&summary));
            }
        }
    });
}

/// "October so far: $1,616 spent, $1,284 left."
fn month_line(s: &Value) -> String {
    if s["empty"] == true {
        return String::from("Tally has nothing yet. Ask how to start, or add an account in the Tally panel.");
    }
    super::remember_currency(s);
    let fmt = super::formatter();
    let spent = fmt.format_whole(s["spent"].as_i64().unwrap_or(0));
    let left = s["pace"]["remaining"].as_i64().unwrap_or(0);
    if s["pace"]["budget"].as_i64().unwrap_or(0) > 0 {
        format!("This month so far: {spent} spent, {} left. Ask anything, or tell it what to change.", fmt.format_whole(left))
    } else {
        format!("This month so far: {spent} spent. Ask anything, or tell it what to change.")
    }
}

/// Open thread `id`: read it whole and draw it.
pub fn open(ui: &Rc<Ui>, id: i64) {
    STATE.with(|s| {
        s.current.set(Some(id));
        s.last.set(0);
        s.pending.borrow_mut().clear();
        s.tools.borrow_mut().clear();
        *s.turn.borrow_mut() = None;
    });
    if let Some(v) = view() {
        clear(&v.column);
        v.middle.set_visible_child_name("thread");
        v.title_key.set_visible(true);
        place_composer(&v, false);
    }
    if *ui.page.borrow() != PAGE {
        ui.navigate(PAGE);
    }
    mark_selected();
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let read = ui.call("thread.get", json!({"id": id})).await;
        if current() != Some(id) {
            return;
        }
        match read {
            Ok(read) => draw_thread(&read),
            Err(e) => ui.show_error(&e.to_string()),
        }
    });
}

fn draw_thread(read: &Value) {
    let Some(v) = view() else { return };
    clear(&v.column);
    STATE.with(|s| {
        s.tools.borrow_mut().clear();
        *s.turn.borrow_mut() = None;
    });
    header(&v, &read["thread"]);
    for message in read["messages"].as_array().into_iter().flatten() {
        draw_message(&v, message);
    }
    stick_to_bottom(&v, true);
    v.input.grab_focus();
}

fn header(v: &View, thread: &Value) {
    v.title.set_text(thread["title"].as_str().unwrap_or("Thread"));
    set_working(v, thread["working"] == true);
}

fn set_working(v: &View, working: bool) {
    STATE.with(|s| s.working.set(working));
    v.send.set_child(Some(&crate::icons::image_with_stroke(if working { "pause" } else { "arrow-up" }, 15, 2.0)));
    v.send.set_tooltip_text(Some(if working { "Stop the reply" } else { "Send · Enter" }));
    if working {
        if v.working.parent().is_none() && v.middle.visible_child_name().as_deref() == Some("thread") {
            v.column.append(&v.working);
        }
    } else {
        if let Some(parent) = v.working.parent().and_downcast::<gtk::Box>() {
            parent.remove(&v.working);
        }
        if let Some(parent) = v.pending.parent().and_downcast::<gtk::Box>() {
            parent.remove(&v.pending);
        }
        STATE.with(|s| s.pending.borrow_mut().clear());
    }
}

/// Keep the newest message in view: always when `force`, else only when the reader was already
/// at the bottom.
fn stick_to_bottom(v: &View, force: bool) {
    let adj = v.scroll.vadjustment();
    let at_bottom = adj.value() + adj.page_size() >= adj.upper() - 48.0;
    if force || at_bottom {
        glib::idle_add_local_once(move || {
            adj.set_value(adj.upper() - adj.page_size());
        });
    }
}

/// The box the agent's current turn draws into, made when its first piece arrives.
fn turn(v: &View) -> gtk::Box {
    if let Some(turn) = STATE.with(|s| s.turn.borrow().clone()) {
        return turn;
    }
    let turn = gtk::Box::new(gtk::Orientation::Vertical, 12);
    turn.add_css_class("threads-turn");
    v.column.append(&turn);
    STATE.with(|s| *s.turn.borrow_mut() = Some(turn.clone()));
    turn
}

fn draw_message(v: &View, m: &Value) {
    let id = m["id"].as_i64().unwrap_or(0);
    if id <= STATE.with(|s| s.last.get()) {
        return;
    }
    STATE.with(|s| s.last.set(id));
    let body = &m["body"];
    match m["role"].as_str().unwrap_or("") {
        "user" => {
            STATE.with(|s| *s.turn.borrow_mut() = None);
            let bubble = label(body["text"].as_str().unwrap_or(""), "threads-user");
            bubble.set_wrap(true);
            bubble.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            bubble.set_selectable(true);
            bubble.set_halign(gtk::Align::End);
            bubble.set_max_width_chars(60);
            v.column.append(&bubble);
        }
        "assistant" => {
            if let Some(parent) = v.pending.parent().and_downcast::<gtk::Box>() {
                parent.remove(&v.pending);
            }
            STATE.with(|s| s.pending.borrow_mut().clear());
            let turn = turn(v);
            for block in body["blocks"].as_array().into_iter().flatten() {
                match block["type"].as_str() {
                    Some("text") => draw_reply(&turn, block["text"].as_str().unwrap_or("")),
                    Some("tool_use") => draw_tool(&turn, block),
                    _ => {}
                }
            }
        }
        "tool" => draw_tool_answer(body),
        "error" => {
            let row = label(body["text"].as_str().unwrap_or("Something went wrong"), "threads-error");
            row.set_wrap(true);
            turn(v).append(&row);
        }
        _ => {}
    }
    // The working row stays last while the turn runs.
    if v.working.parent().is_some() {
        v.column.remove(&v.working);
        v.column.append(&v.working);
    }
}

/// An agent's text, its Markdown drawn as blocks.
fn draw_reply(parent: &gtk::Box, text: &str) {
    for block in markdown(text) {
        let widget: gtk::Widget = match block {
            Block::Paragraph(markup) => reply_label(&markup, "threads-reply").upcast(),
            Block::Heading { level, text } => reply_label(&text, if level <= 2 { "threads-h2" } else { "threads-h3" }).upcast(),
            Block::List { ordered, start, items } => {
                let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
                for (n, item) in items.iter().enumerate() {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let mark = if ordered { format!("{}.", start as usize + n) } else { String::from("•") };
                    let marker = label(&mark, "threads-marker");
                    marker.set_valign(gtk::Align::Start);
                    row.append(&marker);
                    let words = reply_label(item, "threads-reply");
                    words.set_hexpand(true);
                    row.append(&words);
                    list.append(&row);
                }
                list.upcast()
            }
            Block::Code { lang, text } if lang == "chart" && chart_spec(&text).is_some() => {
                super::chart::card(chart_spec(&text).expect("checked")).upcast()
            }
            Block::Code { text, .. } => {
                let code = label(&text, "threads-code");
                code.set_selectable(true);
                code.set_wrap(true);
                code.set_wrap_mode(gtk::pango::WrapMode::Char);
                code.upcast()
            }
            Block::Table(rows) => {
                let grid = gtk::Grid::new();
                grid.add_css_class("threads-table");
                grid.set_column_spacing(18);
                grid.set_row_spacing(6);
                for (r, row) in rows.iter().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        let l = reply_label(cell, if r == 0 { "threads-table-head" } else { "threads-table-cell" });
                        grid.attach(&l, c as i32, r as i32, 1, 1);
                    }
                }
                grid.upcast()
            }
            Block::Rule => {
                let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
                rule.add_css_class("threads-rule");
                rule.upcast()
            }
        };
        parent.append(&widget);
    }
}

fn reply_label(markup: &str, class: &str) -> gtk::Label {
    let l = label("", class);
    l.set_markup(markup);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_selectable(true);
    l
}

/// A tool call: a quiet line saying what the agent did. Its answer may turn it into a card.
fn draw_tool(parent: &gtk::Box, block: &Value) {
    let name = block["name"].as_str().unwrap_or("").to_string();
    let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.add_css_class("threads-tool");
    line.append(&crate::icons::image(if tool_writes(&name) { "check" } else { "search" }, 13));
    line.append(&label(&tool_caption(&name), "threads-tool-text"));
    if let Some(op) = tool_op(&name) {
        line.set_tooltip_text(Some(&op));
    }
    slot.append(&line);
    parent.append(&slot);
    if let Some(id) = block["id"].as_str() {
        STATE.with(|s| s.tools.borrow_mut().insert(id.to_string(), (slot, name)));
    }
}

/// A tool's answer: a refusal marks its line; an entry the agent added becomes a card with Undo.
fn draw_tool_answer(body: &Value) {
    let Some((slot, name)) = body["tool_use_id"].as_str().and_then(|id| STATE.with(|s| s.tools.borrow().get(id).cloned())) else { return };
    if body["is_error"] == true {
        if let Some(line) = slot.first_child() {
            line.add_css_class("threads-tool-failed");
            line.set_tooltip_text(body["text"].as_str());
        }
        return;
    }
    if !tool_writes(&name) {
        return;
    }
    let Ok(tx) = serde_json::from_str::<Value>(body["text"].as_str().unwrap_or("")) else { return };
    if tx["id"].as_i64().is_none() {
        return;
    }
    clear(&slot);
    slot.append(&entry_card(&tx, tool_op(&name).as_deref() == Some("money.tx.add")));
}

/// What the agent did to an entry, with Undo when it added it.
fn entry_card(tx: &Value, added: bool) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    card.add_css_class("threads-card");
    card.append(&badge(tx["icon"].as_str().unwrap_or("dot"), tx["color"].as_i64().unwrap_or(10)));
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    let kind = match tx["type"].as_str() {
        Some("INCOME") => "income",
        Some("TRANSFER") => "transfer",
        _ => "expense",
    };
    let what = tx["note"].as_str().filter(|n| !n.trim().is_empty()).or(tx["category"].as_str()).unwrap_or(kind);
    words.append(&label(&format!("{} {kind}: {what}", if added { "Added" } else { "Changed" }), "threads-card-title"));
    let mut detail = Vec::new();
    if let Some(category) = tx["category"].as_str() {
        detail.push(category.to_string());
    }
    if let Some(date) = tx["date"].as_str() {
        detail.push(human_date(date));
    }
    if let Some(account) = tx["account"].as_str() {
        detail.push(account.to_string());
    }
    words.append(&label(&detail.join(" · "), "threads-card-detail"));
    card.append(&words);
    let figure = label(&amount_text(tx), "money-amount");
    if kind == "income" {
        figure.add_css_class("money-in");
    }
    card.append(&figure);
    if added {
        let undo = button("Undo", "quiet");
        undo.add_css_class("threads-undo");
        let id = tx["id"].as_i64().unwrap_or(0);
        undo.connect_clicked(move |key| {
            let Some(ui) = the_ui() else { return };
            key.set_sensitive(false);
            let key = key.clone();
            glib::spawn_future_local(async move {
                match ui.call("money.tx.delete", json!({"id": id})).await {
                    Ok(_) => key.set_label("Undone"),
                    Err(e) => {
                        key.set_sensitive(true);
                        ui.show_error(&e.to_string());
                    }
                }
            });
        });
        card.append(&undo);
    }
    card
}

fn amount_text(tx: &Value) -> String {
    let fmt = super::formatter();
    let amount = fmt.format(tx["amount"].as_i64().unwrap_or(0));
    match tx["type"].as_str() {
        Some("EXPENSE") => format!("−{amount}"),
        Some("INCOME") => format!("+{amount}"),
        _ => amount,
    }
}

fn submit(ui: &Rc<Ui>) {
    let Some(v) = view() else { return };
    let buffer = v.input.buffer();
    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).trim().to_string();
    if text.is_empty() || STATE.with(|s| s.working.get()) {
        return;
    }
    buffer.set_text("");
    send(ui, &text);
}

/// Send `text`: in the thread showing, or as the first message of a new one.
fn send(ui: &Rc<Ui>, text: &str) {
    let ui = ui.clone();
    let text = text.to_string();
    glib::spawn_future_local(async move {
        let result = match current() {
            Some(id) => ui.call("thread.send", json!({"id": id, "text": text})).await.map(|_| id),
            None => ui.call("thread.create", json!({"text": text})).await.map(|t| t["id"].as_i64().unwrap_or(0)),
        };
        match result {
            Ok(id) if current() != Some(id) => open(&ui, id),
            Ok(_) => {}
            Err(e) => {
                if let Some(v) = view() {
                    v.input.buffer().set_text(&text);
                }
                ui.show_error(&e.to_string());
            }
        }
    });
}

fn stop(ui: &Rc<Ui>) {
    let Some(id) = current() else { return };
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        if let Err(e) = ui.call("thread.stop", json!({"id": id})).await {
            ui.show_error(&e.to_string());
        }
    });
}

/// `thread.*` from the engine.
pub fn event(ui: &Rc<Ui>, ev: &str, payload: &Value) {
    let Some(v) = view() else { return };
    let thread = payload["thread"].as_i64().or_else(|| payload["thread"]["id"].as_i64()).or_else(|| payload["id"].as_i64());
    match ev {
        "thread.delta" if thread == current() => {
            let text = payload["text"].as_str().unwrap_or("");
            let pending = STATE.with(|s| {
                let mut p = s.pending.borrow_mut();
                p.push_str(text);
                p.clone()
            });
            if v.pending.parent().is_none() {
                turn(&v).append(&v.pending);
            }
            v.pending.set_text(&pending);
            stick_to_bottom(&v, false);
        }
        "thread.message" if thread == current() => {
            draw_message(&v, &payload["message"]);
            stick_to_bottom(&v, false);
        }
        "thread.changed" => {
            if thread == current() {
                if payload["deleted"] == true {
                    new_thread(ui);
                } else {
                    header(&v, &payload["thread"]);
                    if payload["thread"]["working"] == true {
                        stick_to_bottom(&v, false);
                    }
                }
            }
            refresh_list(ui);
        }
        _ => {}
    }
}

/// The sidebar's thread list: a box under the Threads keys, filled from `thread.list`.
pub fn sidebar_list() -> gtk::ScrolledWindow {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 1);
    list.add_css_class("threads-list");
    STATE.with(|s| *s.list.borrow_mut() = Some(list.clone()));
    let scroll = crate::app::scrolled(&list);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll
}

/// Re-read the thread list; one read at a time, the last change winning.
pub fn refresh_list(ui: &Rc<Ui>) {
    if STATE.with(|s| s.listing.replace(true)) {
        return;
    }
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let listed = ui.call("thread.list", json!({})).await;
        STATE.with(|s| s.listing.set(false));
        match listed {
            Ok(listed) => draw_list(&listed["threads"]),
            Err(e) => {
                if let Some(list) = STATE.with(|s| s.list.borrow().clone()) {
                    clear(&list);
                    let l = label(&super::pages::unavailable(&e), "threads-list-note");
                    l.set_wrap(true);
                    list.append(&l);
                }
            }
        }
    });
}

/// Which heading a thread used last on `day` sits under.
fn age(updated_at: &str) -> &'static str {
    let (Ok(then), Ok(now)) = (glib::DateTime::from_iso8601(updated_at, None), glib::DateTime::now_local()) else { return "Earlier" };
    let Ok(then) = then.to_local() else { return "Earlier" };
    let day = |d: &glib::DateTime| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok();
    let (Some(then), Some(today)) = (day(&then), day(&now)) else { return "Earlier" };
    match today.difference(&then).as_days() {
        ..=0 => "Today",
        1 => "Yesterday",
        2..=6 => "This week",
        _ => "Earlier",
    }
}

fn draw_list(threads: &Value) {
    let Some(list) = STATE.with(|s| s.list.borrow().clone()) else { return };
    clear(&list);
    let mut heading = "";
    for t in threads.as_array().into_iter().flatten() {
        let group = age(t["updated_at"].as_str().unwrap_or(""));
        if group != heading {
            heading = group;
            list.append(&label(group, "section-label"));
        }
        let id = t["id"].as_i64().unwrap_or(0);
        let key = button("", "nav");
        key.add_css_class("threads-row");
        key.set_widget_name(&format!("thread-{id}"));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label(t["title"].as_str().unwrap_or("Thread"), "nav-label");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        row.append(&title);
        if t["working"] == true {
            let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            lamp.add_css_class("threads-lamp");
            lamp.set_valign(gtk::Align::Center);
            row.append(&lamp);
        }
        key.set_child(Some(&row));
        if let Some(preview) = t["preview"].as_str() {
            key.set_tooltip_text(Some(preview));
        }
        key.connect_clicked(move |_| {
            if let Some(ui) = the_ui() {
                open(&ui, id);
            }
        });
        list.append(&key);
    }
    if threads.as_array().is_none_or(Vec::is_empty) {
        let note = label("Your threads show here.", "threads-list-note");
        note.set_wrap(true);
        list.append(&note);
    }
    mark_selected();
}

fn mark_selected() {
    let Some(list) = STATE.with(|s| s.list.borrow().clone()) else { return };
    let selected = current().map(|id| format!("thread-{id}"));
    let mut child = list.first_child();
    while let Some(w) = child {
        if w.has_css_class("threads-row") {
            if Some(w.widget_name().to_string()) == selected {
                w.add_css_class("selected");
            } else {
                w.remove_css_class("selected");
            }
        }
        child = w.next_sibling();
    }
}

/// The page came into view, or something it shows moved: the list, the thread, the panel.
pub async fn refresh(ui: &Rc<Ui>) {
    refresh_list(ui);
    match current() {
        Some(id) => {
            let after = STATE.with(|s| s.last.get());
            if let Ok(read) = ui.call("thread.get", json!({"id": id, "after": after})).await {
                if current() == Some(id) {
                    if let Some(v) = view() {
                        header(&v, &read["thread"]);
                        for m in read["messages"].as_array().into_iter().flatten() {
                            draw_message(&v, m);
                        }
                    }
                }
            }
        }
        None => {
            if let Ok(summary) = ui.call("money.summary", json!({})).await {
                if let Some(v) = view().filter(|_| current().is_none()) {
                    v.empty_line.set_text(&month_line(&summary));
                }
            }
        }
    }
    refresh_panel(ui);
}

/// Re-read the Tally panel's view, if it is open.
pub fn refresh_panel(ui: &Rc<Ui>) {
    let Some(v) = view() else { return };
    if !v.panel.reveals_child() {
        return;
    }
    let tab = STATE.with(|s| s.panel_tab.get());
    for (key, button) in &v.panel_tabs {
        if *key == tab {
            button.add_css_class("selected");
        } else {
            button.remove_css_class("selected");
        }
    }
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let read = if tab == "entries" {
            ui.call("money.tx.list", json!({"limit": 60})).await
        } else {
            ui.call("money.summary", json!({})).await
        };
        if STATE.with(|s| s.panel_tab.get()) != tab {
            return;
        }
        let Some(v) = view() else { return };
        clear(&v.panel_body);
        match read {
            Ok(value) => match tab {
                "entries" => panel_entries(&v.panel_body, &value),
                "budgets" => panel_budgets(&v.panel_body, &value, usize::MAX),
                _ => panel_overview(&ui, &v.panel_body, &value),
            },
            Err(e) => {
                let l = label(&super::pages::unavailable(&e), "money-muted");
                l.set_wrap(true);
                v.panel_body.append(&l);
            }
        }
    });
}

fn caption(text: &str) -> gtk::Label {
    label(text, "threads-caption")
}

fn panel_overview(ui: &Rc<Ui>, body: &gtk::Box, s: &Value) {
    super::remember_currency(s);
    if s["empty"] == true {
        let note = label("Tally has nothing yet. Add an account, load the sample household, or import a backup.", "money-muted");
        note.set_wrap(true);
        body.append(&note);
        let open = button("Open Tally", "");
        open.set_halign(gtk::Align::Start);
        let weak = Rc::downgrade(ui);
        open.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.navigate("money-home");
            }
        });
        body.append(&open);
        return;
    }
    let fmt = super::formatter();
    let period = &s["period"];
    let days = period["days"].as_i64().unwrap_or(0);
    let day = days - period["days_left"].as_i64().unwrap_or(0) + 1;
    let head = gtk::Box::new(gtk::Orientation::Vertical, 4);
    head.append(&label(&format!("Day {day} of {days}"), "threads-panel-eyebrow"));
    let budget = s["pace"]["budget"].as_i64().unwrap_or(0);
    if budget > 0 {
        let left = s["pace"]["remaining"].as_i64().unwrap_or(0);
        let figure = label(&format!("{} left", fmt.format_whole(left)), "threads-panel-figure");
        if left < 0 {
            figure.add_css_class("money-over");
        }
        head.append(&figure);
        head.append(&label(&format!("of {} planned this month", fmt.format_whole(budget)), "money-muted"));
    } else {
        head.append(&label(&format!("{} spent", fmt.format_whole(s["spent"].as_i64().unwrap_or(0))), "threads-panel-figure"));
    }
    if let Some(line) = s["lines"]["pace"].as_str().filter(|l| !l.is_empty()) {
        let l = label(line, "money-muted");
        l.set_wrap(true);
        head.append(&l);
    }
    body.append(&head);
    panel_budgets(body, s, 5);
    let recent = gtk::Box::new(gtk::Orientation::Vertical, 0);
    recent.append(&caption("Recent"));
    for tx in s["recent"].as_array().into_iter().flatten().take(6) {
        recent.append(&entry_row(tx));
    }
    body.append(&recent);
}

fn panel_budgets(body: &gtk::Box, s: &Value, most: usize) {
    let budgets: Vec<&Value> = s["budgets"].as_array().into_iter().flatten().collect();
    let block = gtk::Box::new(gtk::Orientation::Vertical, 12);
    block.append(&caption("Budgets"));
    if budgets.is_empty() {
        let note = label("No budgets yet. Set them in Tally's Plan.", "money-muted");
        note.set_wrap(true);
        block.append(&note);
    }
    let fmt = super::formatter();
    let pace = s["pace"]["pace_fraction"].as_f64();
    for b in budgets.into_iter().take(most) {
        let (spent, budget) = (b["spent"].as_i64().unwrap_or(0), b["budget"].as_i64().unwrap_or(0));
        let status = b["status"].as_str().unwrap_or("");
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = label(b["name"].as_str().unwrap_or(""), "threads-panel-name");
        name.set_hexpand(true);
        line.append(&name);
        let figure = label(&format!("{} / {}", fmt.format_whole(spent), fmt.format_whole(budget)), "threads-panel-figures");
        if let Some(class) = tone_class(status) {
            figure.add_css_class(class);
        }
        line.append(&figure);
        row.append(&line);
        let fraction = if budget > 0 { spent as f64 / budget as f64 } else { 0.0 };
        row.append(&meter_in(fraction, pace, tone(status, b["color"].as_i64()), 4));
        block.append(&row);
    }
    body.append(&block);
}

fn entry_row(tx: &Value) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("threads-entry");
    let words = gtk::Box::new(gtk::Orientation::Vertical, 1);
    words.set_hexpand(true);
    let what = tx["note"].as_str().filter(|n| !n.trim().is_empty()).or(tx["category"].as_str()).unwrap_or("Entry");
    let title = label(what, "threads-panel-name");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    words.append(&title);
    let mut detail = Vec::new();
    if let Some(category) = tx["category"].as_str().filter(|c| Some(*c) != Some(what)) {
        detail.push(category.to_string());
    }
    detail.push(human_date(tx["date"].as_str().unwrap_or("")));
    words.append(&label(&detail.join(" · "), "threads-panel-detail"));
    row.append(&words);
    let figure = label(&amount_text(tx), "threads-panel-figures");
    if tx["type"] == "INCOME" {
        figure.add_css_class("money-in");
    }
    row.append(&figure);
    row
}

fn panel_entries(body: &gtk::Box, page: &Value) {
    let rows: Vec<&Value> = page["transactions"].as_array().into_iter().flatten().collect();
    let block = gtk::Box::new(gtk::Orientation::Vertical, 0);
    block.append(&caption("This month"));
    if rows.is_empty() {
        block.append(&label("No entries this month.", "money-muted"));
    }
    for tx in rows {
        block.append(&entry_row(tx));
    }
    body.append(&block);
}
