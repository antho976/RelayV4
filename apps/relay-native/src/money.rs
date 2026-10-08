//! The Money space: Tally's ledger on the PC, beside the dev workspace.
//!
//! The space is a mode of the one window. Money's pages live in the main content stack as
//! `money-*`; while one of them shows, the window wears `.space-money` (css/money.css) and the
//! sidebar and title bar trade their dev keys for Money's. The ledger itself is the engine's
//! (`money.*` ops, docs/MONEY.md); this module only reads and writes it through the bus.
use crate::app::{button, clear, label, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use relay_money::money::{Locale, MoneyFormatter};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

#[path = "money_entry.rs"]
mod entry;
#[path = "money_pages.rs"]
mod pages;

/// Money's pages, in sidebar order: (stack name, caption, glyph).
pub const PAGES: [(&str, &str, &str); 4] = [
    ("money-home", "Home", "home"),
    ("money-transactions", "Transactions", "list"),
    ("money-plan", "Plan", "plan"),
    ("money-data", "Data", "data"),
];

/// What a toast's Undo key runs.
pub(crate) type Action = Rc<dyn Fn()>;

/// One Money page: a header built once (search and period keys keep their focus) and a body
/// rebuilt on every read, under an overlay that carries the page's Undo toast.
pub(crate) struct Page {
    pub head: gtk::Box,
    pub body: gtk::Box,
    toast: gtk::Revealer,
    toast_text: gtk::Label,
    toast_undo: gtk::Button,
    undo: Rc<RefCell<Option<Action>>>,
    toast_serial: Rc<Cell<u64>>,
}

#[derive(Default)]
struct State {
    money: Cell<bool>,
    /// The space last read from or written to `native.space`; `None` until it was read.
    saved: Cell<Option<bool>>,
    started: Cell<bool>,
    toggles: RefCell<Option<(gtk::ToggleButton, gtk::ToggleButton)>>,
    dev_only: RefCell<Vec<gtk::Widget>>,
    money_only: RefCell<Vec<gtk::Widget>>,
    pages: RefCell<BTreeMap<&'static str, Rc<Page>>>,
    start: RefCell<Option<gtk::Widget>>,
    period_offset: Cell<i64>,
    query: RefCell<String>,
    currency: RefCell<String>,
}

thread_local! {
    static STATE: State = State::default();
}

pub fn is_page(page: &str) -> bool {
    page.starts_with("money-")
}

/// Whether the Money space is the one showing.
pub fn active() -> bool {
    STATE.with(|s| s.money.get())
}

pub(crate) fn page(name: &str) -> Option<Rc<Page>> {
    STATE.with(|s| s.pages.borrow().get(name).cloned())
}

/// Formats amounts in the ledger's currency, as the last read reported it.
pub(crate) fn formatter() -> MoneyFormatter {
    let currency = STATE.with(|s| s.currency.borrow().clone());
    MoneyFormatter::new(if currency.is_empty() { "CAD" } else { &currency }, Locale::from_env())
}

pub(crate) fn remember_currency(v: &Value) {
    if let Some(code) = v["currency"].as_str().filter(|c| !c.is_empty()) {
        STATE.with(|s| *s.currency.borrow_mut() = code.to_string());
    }
}

pub(crate) fn period_offset() -> i64 {
    STATE.with(|s| s.period_offset.get())
}
pub(crate) fn set_period_offset(offset: i64) {
    STATE.with(|s| s.period_offset.set(offset));
}
pub(crate) fn query() -> String {
    STATE.with(|s| s.query.borrow().clone())
}
pub(crate) fn set_query(query: &str) {
    STATE.with(|s| *s.query.borrow_mut() = query.to_string());
}

/// The title bar's Dev | Threads pill. Wired by [`install`]. Threads is the Money space's name
/// while it grows into agent threads over your own data; inside, it is still Money's pages.
pub fn switcher() -> gtk::Box {
    let pill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    pill.add_css_class("space-switch");
    pill.set_valign(gtk::Align::Center);
    pill.set_margin_start(6);
    let dev = gtk::ToggleButton::with_label("Dev");
    dev.set_widget_name("space-dev");
    dev.set_tooltip_text(Some("Dev: agents, tasks and code · Ctrl `"));
    let money = gtk::ToggleButton::with_label("Threads");
    money.set_widget_name("space-money");
    money.set_tooltip_text(Some("Threads: agents on your own data, starting with your budget · Ctrl `"));
    money.set_group(Some(&dev));
    dev.set_active(true);
    pill.append(&dev);
    pill.append(&money);
    STATE.with(|s| *s.toggles.borrow_mut() = Some((dev, money)));
    pill
}

/// The title bar's add key, shown only in Money.
pub fn add_key() -> gtk::Button {
    let key = button("", "primary");
    key.set_widget_name("money-add");
    key.add_css_class("launch-key");
    key.add_css_class("money-add-key");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&crate::icons::image_with_stroke("plus", 14, 2.0));
    row.append(&label("Entry", ""));
    key.set_child(Some(&row));
    key.set_valign(gtk::Align::Center);
    key.set_tooltip_text(Some("Add an entry · Ctrl N"));
    key.set_visible(false);
    STATE.with(|s| s.money_only.borrow_mut().push(key.clone().upcast()));
    key
}

/// Money's page keys for the sidebar, hidden until the space is Money.
pub fn sidebar_keys() -> gtk::Box {
    let nav = gtk::Box::new(gtk::Orientation::Vertical, 1);
    nav.add_css_class("navigation");
    nav.add_css_class("money-nav");
    nav.set_vexpand(true);
    nav.set_visible(false);
    STATE.with(|s| s.money_only.borrow_mut().push(nav.clone().upcast()));
    nav
}

/// Money's pages, added to the window's content stack.
pub fn add_pages(content: &gtk::Stack) {
    for (name, _, _) in PAGES {
        let head = gtk::Box::new(gtk::Orientation::Vertical, 16);
        head.add_css_class("money-head");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 28);
        body.add_css_class("money-body");
        let column = gtk::Box::new(gtk::Orientation::Vertical, 24);
        column.add_css_class("money-page");
        column.append(&head);
        column.append(&body);
        let scroll = crate::app::scrolled(&column);
        scroll.add_css_class("money-scroll");
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        let root = gtk::Overlay::new();
        root.add_css_class("money-root");
        root.set_child(Some(&scroll));
        let toast = gtk::Revealer::new();
        toast.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        toast.set_transition_duration(160);
        toast.set_halign(gtk::Align::Center);
        toast.set_valign(gtk::Align::End);
        toast.set_margin_bottom(24);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        bar.add_css_class("money-toast");
        let toast_text = label("", "");
        bar.append(&toast_text);
        let toast_undo = button("Undo", "money-text-action");
        toast_undo.set_widget_name("money-undo");
        bar.append(&toast_undo);
        toast.set_child(Some(&bar));
        toast.set_can_target(false);
        root.add_overlay(&toast);
        content.add_named(&root, Some(name));
        let page = Rc::new(Page {
            head,
            body,
            toast,
            toast_text,
            toast_undo,
            undo: Rc::default(),
            toast_serial: Rc::default(),
        });
        let undo = page.undo.clone();
        let revealer = page.toast.clone();
        page.toast_undo.connect_clicked(move |_| {
            revealer.set_reveal_child(false);
            revealer.set_can_target(false);
            if let Some(run) = undo.borrow_mut().take() {
                run();
            }
        });
        STATE.with(|s| s.pages.borrow_mut().insert(name, page));
    }
}

/// A toast on the Money page showing now: `text`, and an Undo key when `undo` is given.
pub(crate) fn toast(ui: &Ui, text: &str, undo: Option<Action>) {
    let Some(page) = page(&ui.page.borrow()) else {
        ui.show_error(text);
        return;
    };
    page.toast_text.set_text(text);
    page.toast_undo.set_visible(undo.is_some());
    *page.undo.borrow_mut() = undo;
    page.toast.set_can_target(true);
    page.toast.set_reveal_child(true);
    let serial = page.toast_serial.get().wrapping_add(1);
    page.toast_serial.set(serial);
    let weak = Rc::downgrade(&page);
    glib::timeout_add_local_once(std::time::Duration::from_secs(7), move || {
        if let Some(page) = weak.upgrade().filter(|p| p.toast_serial.get() == serial) {
            page.toast.set_reveal_child(false);
            page.toast.set_can_target(false);
            page.undo.borrow_mut().take();
        }
    });
}

/// Wires the switcher, Money's sidebar keys and its add key. `nav` is Money's sidebar box,
/// `dev_only` what the dev space shows and Money hides.
pub fn install(ui: &Rc<Ui>, nav: &gtk::Box, add: &gtk::Button, dev_only: Vec<gtk::Widget>) {
    STATE.with(|s| *s.dev_only.borrow_mut() = dev_only);
    if let Some((dev, money)) = STATE.with(|s| s.toggles.borrow().clone()) {
        for (key, to_money) in [(dev, false), (money, true)] {
            let weak = Rc::downgrade(ui);
            key.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    set_space(&ui, to_money);
                }
            });
        }
    }
    for (name, caption, glyph) in PAGES {
        let key = button("", "nav");
        key.set_widget_name(&format!("nav-{name}"));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        row.append(&pages::glyph_image(glyph, 16));
        row.append(&label(caption, "nav-label"));
        key.set_child(Some(&row));
        let weak = key.downgrade();
        ui.content.connect_visible_child_name_notify(move |stack| {
            if let Some(key) = weak.upgrade() {
                if stack.visible_child_name().as_deref() == Some(name) {
                    key.add_css_class("selected");
                } else {
                    key.remove_css_class("selected");
                }
            }
        });
        let weak = Rc::downgrade(ui);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.navigate(name);
            }
        });
        nav.append(&key);
    }
    let weak = Rc::downgrade(ui);
    add.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            add_entry(&ui);
        }
    });
    pages::install(ui);
}

/// Shows `money`'s keys and look. Navigation calls it through [`follow`].
fn apply(ui: &Ui, money: bool) {
    STATE.with(|s| {
        s.money.set(money);
        for w in s.dev_only.borrow().iter() {
            w.set_visible(!money);
        }
        for w in s.money_only.borrow().iter() {
            w.set_visible(money);
        }
    });
    if money {
        ui.window.add_css_class("space-money");
    } else {
        ui.window.remove_css_class("space-money");
    }
    sync_toggles();
}

fn sync_toggles() {
    let money = active();
    if let Some((dev, key)) = STATE.with(|s| s.toggles.borrow().clone()) {
        dev.set_active(!money);
        key.set_active(money);
    }
}

/// Called by `Ui::navigate` once `page` shows. A Money page puts the window in Money, any other
/// page but Settings (which both spaces share) in Dev. `restoring` is a layout restore or an
/// engine echo: it changes the space shown but is never remembered as the person's choice.
pub fn follow(ui: &Rc<Ui>, page: &str, restoring: bool) {
    if page == "settings" {
        return;
    }
    let money = is_page(page);
    if active() != money {
        apply(ui, money);
    }
    let start_open = STATE.with(|s| s.start.borrow().is_some());
    let saved = STATE.with(|s| s.saved.get());
    if !restoring && !start_open && saved.is_some_and(|saved| saved != money) {
        remember(ui, money);
    }
}

fn remember(ui: &Rc<Ui>, money: bool) {
    STATE.with(|s| s.saved.set(Some(money)));
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let value = if money { "money" } else { "dev" };
        if let Err(e) = ui.call("settings.set", json!({"path":"native.space","value":value})).await {
            ui.show_error(&e.to_string());
        }
    });
}

/// Switch space: Money lands on its Home, Dev on the agent wall.
pub fn set_space(ui: &Rc<Ui>, money: bool) {
    dismiss_start(ui, None);
    if active() != money {
        ui.navigate(if money { "money-home" } else { "agents" });
    } else if money && !is_page(&ui.page.borrow()) {
        ui.navigate("money-home");
    }
    // A refused navigation (unsaved work in a panel) leaves the pill where the window is.
    sync_toggles();
}

pub fn toggle_space(ui: &Rc<Ui>) {
    set_space(ui, !active());
}

pub fn add_entry(ui: &Rc<Ui>) {
    entry::open(ui, None);
}

pub(crate) fn add_account(ui: &Rc<Ui>) {
    entry::account(ui);
}

pub(crate) fn edit_entry(ui: &Rc<Ui>, tx: Value) {
    entry::open(ui, Some(tx));
}

/// Re-reads the Money page showing, if one is.
pub async fn refresh(ui: &Rc<Ui>, page: &str) {
    pages::refresh(ui, page).await;
}

/// `money.changed`: the ledger moved, here or elsewhere.
pub fn changed(ui: &Rc<Ui>) {
    if is_page(&ui.page.borrow()) {
        ui.refresh_page();
    }
}

/// Entries for the command palette: (key, caption).
pub fn palette_entries() -> Vec<(&'static str, String)> {
    let mut entries = vec![
        ("space", String::from(if active() { "Switch to Dev" } else { "Switch to Threads" })),
        ("start", String::from("Start screen")),
        ("add", String::from("Add a money entry")),
    ];
    for (name, caption, _) in PAGES {
        entries.push((name, format!("Money {}", caption.to_lowercase())));
    }
    entries
}

pub fn run_palette(ui: &Rc<Ui>, key: &str) {
    match key {
        "space" => toggle_space(ui),
        "start" => show_start(ui),
        "add" => add_entry(ui),
        page => ui.navigate(page),
    }
}

/// After the first connection: read the remembered space, then greet. Display smoke runs
/// (RELAY_NATIVE_SCREENSHOT) start in Dev with no greeting, as they always have.
pub fn startup(ui: &Rc<Ui>) {
    if STATE.with(|s| s.started.replace(true)) {
        return;
    }
    if std::env::var_os("RELAY_NATIVE_SCREENSHOT").is_some() {
        return;
    }
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let money = match ui.call("settings.get", json!({"path":"native.space"})).await {
            Ok(v) => v["value"].as_str() == Some("money"),
            Err(_) => false,
        };
        STATE.with(|s| s.saved.set(Some(money)));
        show_start(&ui);
    });
}

/// "Good evening, Antho", by the local hour.
pub fn greeting() -> String {
    let hour = glib::DateTime::now_local().map(|now| now.hour()).unwrap_or(12);
    let part = match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Good night",
    };
    let name = crate::app::first_name();
    if name.is_empty() {
        part.to_string()
    } else {
        format!("{part}, {name}")
    }
}

/// The start screen: a greeting and one card per space. A card picks its space; Escape picks
/// the last one used.
pub fn show_start(ui: &Rc<Ui>) {
    if STATE.with(|s| s.start.borrow().is_some()) {
        return;
    }
    if !ui.dismiss_panels() {
        return;
    }
    let remembered = STATE.with(|s| s.saved.get()).unwrap_or(active());
    let weak = Rc::downgrade(ui);
    let pick: Rc<dyn Fn(bool)> = Rc::new(move |to_money| {
        if let Some(ui) = weak.upgrade() {
            dismiss_start(&ui, Some(to_money));
        }
    });
    let crate::start::Built { screen, lines, keys } = crate::start::build(&greeting(), remembered, pick);
    // Escape opens the space used last; 1 and 2 pick Dev and Threads, as the cards say.
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(ui);
    escape.connect_key_pressed(move |_, key, _, _| {
        use gtk::gdk::Key;
        let pick = match key {
            Key::Escape => remembered,
            Key::_1 | Key::KP_1 => false,
            Key::_2 | Key::KP_2 => true,
            _ => return glib::Propagation::Proceed,
        };
        if let Some(ui) = weak.upgrade() {
            dismiss_start(&ui, Some(pick));
        }
        glib::Propagation::Stop
    });
    screen.add_controller(escape);
    if let Some(content) = ui.overlay.child() {
        content.set_sensitive(false);
    }
    // The bars keep their place and the window controls; their keys and readings step back.
    ui.window.add_css_class("starting");
    for part in &ui.start_chrome {
        part.set_opacity(0.0);
        part.set_sensitive(false);
    }
    ui.overlay.add_overlay(&screen);
    STATE.with(|s| *s.start.borrow_mut() = Some(screen.clone().upcast()));
    keys[usize::from(remembered)].grab_focus();
    let [dev_line, money_line] = lines;
    let reader = ui.clone();
    glib::spawn_future_local(async move {
        let (dashboard, summary) = tokio::join!(
            reader.call("dashboard.get", json!({})),
            reader.call("money.summary", json!({}))
        );
        match dashboard {
            Ok(d) => crate::start::reading(&dev_line, &dev_reading(&d), !d["sessions_live"].as_array().is_none_or(Vec::is_empty)),
            Err(e) => crate::start::reading(&dev_line, &e.to_string(), false),
        }
        match summary {
            Ok(s) => crate::start::reading(&money_line, &money_reading(&s), s["empty"] != true),
            Err(e) => crate::start::reading(&money_line, &pages::unavailable(&e), false),
        }
    });
}

/// "3 agents running · 2 in review", from `dashboard.get`.
fn dev_reading(d: &Value) -> String {
    let live = d["sessions_live"].as_array().map_or(0, Vec::len);
    let review = d["in_review"].as_array().map_or(0, Vec::len);
    let agents = match live {
        0 => String::from("No agents running"),
        1 => String::from("1 agent running"),
        n => format!("{n} agents running"),
    };
    if review == 0 { agents } else { format!("{agents} · {review} in review") }
}

/// "Budget · $412 left · on pace", from `money.summary`: short enough for its card's one line.
fn money_reading(s: &Value) -> String {
    if s["empty"] == true {
        return String::from("Start with your budget");
    }
    remember_currency(s);
    let margin = s["lines"]["margin"].as_str().unwrap_or("");
    match s["pace"]["status"].as_str().unwrap_or("") {
        "ON_PACE" | "UNDER_PACE" | "OVER_PACE" => {
            let left = formatter().format_whole(s["pace"]["remaining"].as_i64().unwrap_or(0));
            let pace = match s["pace"]["status"].as_str() {
                Some("OVER_PACE") => "over pace",
                Some("UNDER_PACE") => "under pace",
                _ => "on pace",
            };
            format!("Budget · {left} left · {pace}")
        }
        _ if margin.is_empty() => String::from("Budget"),
        _ => format!("Budget · {margin}"),
    }
}

/// Escape while the start screen shows: open the space last used. False when it is not showing.
pub fn escape_start(ui: &Rc<Ui>) -> bool {
    if STATE.with(|s| s.start.borrow().is_none()) {
        return false;
    }
    let remembered = STATE.with(|s| s.saved.get()).unwrap_or(active());
    dismiss_start(ui, Some(remembered));
    true
}

/// Closes the start screen; with `pick`, into that space.
fn dismiss_start(ui: &Rc<Ui>, pick: Option<bool>) {
    let Some(screen) = STATE.with(|s| s.start.borrow_mut().take()) else {
        return;
    };
    // It fades out over what it opened into; the bars come back at once.
    let overlay = ui.overlay.clone();
    let leaving = screen.clone();
    crate::start::leave(&screen, move || overlay.remove_overlay(&leaving));
    ui.window.remove_css_class("starting");
    for part in &ui.start_chrome {
        part.set_opacity(1.0);
        part.set_sensitive(true);
    }
    if ui.panels.borrow().is_empty() {
        if let Some(content) = ui.overlay.child() {
            content.set_sensitive(true);
        }
    }
    if let Some(money) = pick {
        set_space(ui, money);
        // Picking the space already showing navigates nowhere, so `follow` never saw it.
        if STATE.with(|s| s.saved.get()).is_some_and(|saved| saved != money) && active() == money {
            remember(ui, money);
        }
    }
}

/// An empty column with a message: what a page shows before its first read, or when it fails.
pub(crate) fn message(body: &gtk::Box, text: &str) {
    clear(body);
    let l = label(text, "money-muted");
    l.set_wrap(true);
    body.append(&l);
}
