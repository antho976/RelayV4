//! Avex in the Threads space, beside Tally and Arbiter (docs/GYM.md): its pages (Overview,
//! Workouts, Lifts, Data, and one workout or one lift opened from a row), and what the thread page
//! borrows from them, the Avex panel.
//!
//! Avex has no internet, so what shows is the training history the person last exported from it
//! and imported here. Everything is read through the engine's `gym.*` ops and never changed here:
//! the only writes are importing a newer export and forgetting the copy. Weights and volumes
//! arrive in the person's unit, named by each reading's `unit`.
use super::pages::{card, card_asking, cell, columns, glyph_image, human_date, icon_row, plural, row_key, setting_row, strip, tile};
use super::{message, page, toast, Action, Page};
use crate::app::{button, clear, confirm_inline, label, rows, text, Ui};
use crate::client::Error;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Avex's tabbed pages: (stack name, caption).
pub const PAGES: [(&str, &str); 4] = [("avex-home", "Overview"), ("avex-workouts", "Workouts"), ("avex-lifts", "Lifts"), ("avex-data", "Data")];
/// One workout and one lift, opened from a row; they carry no tab of their own.
pub const WORKOUT: &str = "avex-workout";
pub const LIFT: &str = "avex-lift";
/// Every Avex page, for `money::add_pages`.
pub const ALL: [&str; 6] = ["avex-home", "avex-workouts", "avex-lifts", "avex-data", WORKOUT, LIFT];

/// The thread panel's Avex views: (key, caption).
pub const PANEL_TABS: [(&str, &str); 3] = [("avex-overview", "Overview"), ("avex-workouts", "Workouts"), ("avex-lifts", "Lifts")];

/// How many workouts the Workouts page reads at a time.
const PAGE_SIZE: u32 = 60;

#[derive(Default)]
struct State {
    /// The workout and the lift their pages show.
    workout: Cell<Option<i64>>,
    lift: RefCell<Option<String>>,
    /// How many workouts the Workouts page shows; Show more reads further.
    shown: Cell<u32>,
}

thread_local! {
    static STATE: State = State::default();
}

// ---- Reading the engine's numbers ----------------------------------------------------------

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// `102.5`, `100`: a weight or a count without a trailing `.0`.
fn trimmed(x: f64) -> String {
    if (x - x.round()).abs() < 0.05 {
        group(x.round() as i64)
    } else {
        format!("{x:.1}")
    }
}

/// `12,500`.
pub(super) fn group(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("−{out}")
    } else {
        out
    }
}

fn volume(v: &Value, unit: &str) -> String {
    format!("{} {unit}", group(num(v).unwrap_or(0.0).round() as i64))
}

/// `225 × 5`, `× 12` for a bodyweight set.
fn set_words(weight: &Value, reps: &Value, unit: &str) -> String {
    let reps = reps.as_i64().unwrap_or(0);
    match num(weight) {
        Some(w) => format!("{} {unit} × {reps}", trimmed(w)),
        None => format!("× {reps}"),
    }
}

fn minutes(m: i64) -> String {
    if m >= 60 {
        format!("{} h {:02} min", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

pub(super) fn said(e: &Error) -> String {
    match e {
        Error::Bus(b) if matches!(b.code.as_str(), "bus.unknown_op" | "bus.not_implemented") => {
            String::from("This engine does not read Avex yet. Update Relay's engine to see your training here.")
        }
        e => e.to_string(),
    }
}

// ---- Marks ---------------------------------------------------------------------------------

/// Strokes on the 16-unit grid for Avex's own marks; the rest are Tally's glyphs.
fn glyph(key: &str) -> Option<&'static str> {
    Some(match key {
        "avex" | "lift" => r##"<path d="M5 8h6" /><rect x="2.5" y="5" width="2.5" height="6" rx=".8" /><rect x="11" y="5" width="2.5" height="6" rx=".8" /><path d="M1.5 6.5v3M14.5 6.5v3" />"##,
        "streak" => r##"<path d="M8 14a4 4 0 0 1-4-4c0-3 4-4 3-8 3 1.5 5 4.5 5 8a4 4 0 0 1-4 4z" />"##,
        "record" => r##"<path d="M5 2.5h6v3.5a3 3 0 0 1-6 0z" /><path d="M5 3.5H3a2 2 0 0 0 2 3M11 3.5h2a2 2 0 0 1-2 3M8 9v3M5.5 13.5h5" />"##,
        "clock" => r##"<circle cx="8" cy="8" r="5.5" /><path d="M8 5v3.2l2 1.3" />"##,
        "run" => r##"<circle cx="10" cy="3" r="1.2" /><path d="M6 14l2-4.5 2 1.5 1 3M8 9.5l1-4-3 1-1.5 2.5M9 5.5l2 2.5 2 .5" />"##,
        "target" => r##"<circle cx="8" cy="8" r="5.5" /><circle cx="8" cy="8" r="2.5" /><circle cx="8" cy="8" r=".5" />"##,
        _ => return None,
    })
}

/// An Avex glyph, else Tally's.
pub(super) fn icon(key: &str, size: i32) -> gtk::Image {
    match glyph(key) {
        Some(g) => crate::icons::from_geometry(g, size, 1.5),
        None => glyph_image(key, size),
    }
}

/// Tally's neutral tile, with an Avex glyph.
fn mark(key: &str) -> gtk::Box {
    let t = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    t.add_css_class("tally-tile");
    t.set_valign(gtk::Align::Center);
    t.set_hexpand(false);
    t.set_size_request(28, 28);
    let image = icon(key, 15);
    image.set_halign(gtk::Align::Center);
    image.set_hexpand(true);
    t.append(&image);
    t
}

// ---- Layout --------------------------------------------------------------------------------

/// A page's head: its title, a context line under it, and keys on the right.
fn heading(head: &gtk::Box, title: &str, context: &str, actions: &[&gtk::Widget]) {
    clear(head);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let words = gtk::Box::new(gtk::Orientation::Vertical, 4);
    words.set_hexpand(true);
    words.append(&label(title, "money-title"));
    if !context.is_empty() {
        let l = label(context, "money-context");
        l.set_wrap(true);
        words.append(&l);
    }
    row.append(&words);
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    keys.set_valign(gtk::Align::Center);
    for action in actions {
        keys.append(*action);
    }
    row.append(&keys);
    head.append(&row);
}

/// A padded block inside a card, for what is not a row.
fn block(rows: &gtk::Box) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("arbiter-block");
    rows.append(&b);
    b
}

fn quiet_row(rows: &gtk::Box, words: &str) {
    let l = label(words, "money-muted");
    l.set_wrap(true);
    l.add_css_class("arbiter-quiet-row");
    rows.append(&l);
}

/// "Exported Oct 8 21:14 · imported 2 h ago", from `gym.summary`'s `imported`.
fn exported_line(imported: &Value) -> String {
    let mut parts = vec![format!("Exported {}", text(imported, "exported_at"))];
    if let Some(when) = crate::relative::ago(text(imported, "imported_at"), crate::relative::Form::Long) {
        parts.push(format!("imported {when}"));
    }
    parts.join(" · ")
}

fn import_key(ui: &Rc<Ui>, caption: &str, class: &str) -> gtk::Button {
    let key = button(caption, class);
    key.set_widget_name("avex-import");
    key.set_tooltip_text(Some("Read the Training history file Avex exports (avex_export.json)"));
    let weak = Rc::downgrade(ui);
    key.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            import(&ui);
        }
    });
    key
}

fn go(ui: &Rc<Ui>, page: &'static str) -> Action {
    let weak = Rc::downgrade(ui);
    Rc::new(move || {
        if let Some(ui) = weak.upgrade() {
            ui.navigate(page);
        }
    })
}

/// Bars over `values`, the last in Relay's lime and the rest in a quiet ink, with a caption under
/// the first and the last; hovering one says its label and value.
fn bars(values: Vec<f64>, labels: Vec<String>, unit: &'static str, height: i32) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let area = gtk::DrawingArea::new();
    area.set_content_height(height);
    area.set_hexpand(true);
    let drawn = values.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (f64::from(w), f64::from(h));
        let n = drawn.len().max(1) as f64;
        let top = drawn.iter().copied().fold(0.0, f64::max).max(1.0);
        let gap = 4.0;
        let bw = ((w - gap * (n - 1.0)) / n).max(2.0);
        for (i, v) in drawn.iter().enumerate() {
            let last = i + 1 == drawn.len();
            let (r, g, b) = if last { (0.776, 0.949, 0.306) } else { (0.42, 0.40, 0.38) };
            cr.set_source_rgb(r, g, b);
            let bh = if *v > 0.0 { (v / top * (h - 2.0)).max(2.0) } else { 1.0 };
            let x = i as f64 * (bw + gap);
            let rad = (bw / 2.0).min(2.5);
            let y = h - bh;
            cr.move_to(x, h);
            cr.line_to(x, y + rad);
            cr.arc(x + rad, y + rad, rad, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.arc(x + bw - rad, y + rad, rad, 1.5 * std::f64::consts::PI, 0.0);
            cr.line_to(x + bw, h);
            cr.close_path();
            let _ = cr.fill();
        }
    });
    area.set_has_tooltip(true);
    let (tips, tip_labels) = (values, labels.clone());
    area.connect_query_tooltip(move |area, x, _, _, tooltip| {
        let n = tips.len();
        if n == 0 {
            return false;
        }
        let i = ((f64::from(x) / f64::from(area.width().max(1))) * n as f64).floor() as usize;
        let Some(v) = tips.get(i.min(n - 1)) else { return false };
        tooltip.set_text(Some(&format!("{}\n{} {unit}", tip_labels[i.min(n - 1)], group(v.round() as i64))));
        true
    });
    column.append(&area);
    if let (Some(first), Some(last)) = (labels.first(), labels.last()) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let a = label(first, "arbiter-range");
        a.set_hexpand(true);
        row.append(&a);
        row.append(&label(last, "arbiter-range"));
        column.append(&row);
    }
    column
}

/// A line over `points`, its records marked, on a scale that starts near the lowest value so
/// progress shows; hovering a point says its day and value.
fn progress_line(points: Vec<(String, f64, bool)>, unit: String, height: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(height);
    area.set_hexpand(true);
    let drawn = points.clone();
    area.set_draw_func(move |_, cr, w, h| {
        if drawn.is_empty() {
            return;
        }
        let (w, h) = (f64::from(w), f64::from(h));
        let lo = drawn.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let hi = drawn.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let pad = ((hi - lo) * 0.15).max(1.0);
        let (lo, hi) = (lo - pad, hi + pad);
        let x_of = |i: usize| if drawn.len() == 1 { w / 2.0 } else { 6.0 + i as f64 / (drawn.len() - 1) as f64 * (w - 12.0) };
        let y_of = |v: f64| 6.0 + (hi - v) / (hi - lo) * (h - 12.0);
        cr.set_source_rgb(0.416, 0.624, 0.847);
        cr.set_line_width(2.0);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        for (i, p) in drawn.iter().enumerate() {
            if i == 0 {
                cr.move_to(x_of(i), y_of(p.1));
            } else {
                cr.line_to(x_of(i), y_of(p.1));
            }
        }
        let _ = cr.stroke();
        for (i, p) in drawn.iter().enumerate() {
            if p.2 {
                cr.set_source_rgb(0.776, 0.949, 0.306);
                cr.arc(x_of(i), y_of(p.1), 3.5, 0.0, 2.0 * std::f64::consts::PI);
                let _ = cr.fill();
            } else if drawn.len() <= 40 {
                cr.set_source_rgb(0.416, 0.624, 0.847);
                cr.arc(x_of(i), y_of(p.1), 2.0, 0.0, 2.0 * std::f64::consts::PI);
                let _ = cr.fill();
            }
        }
    });
    area.set_has_tooltip(true);
    area.connect_query_tooltip(move |area, x, _, _, tooltip| {
        let n = points.len();
        if n == 0 {
            return false;
        }
        let w = f64::from(area.width().max(1));
        let i = if n == 1 { 0 } else { (((f64::from(x) - 6.0) / (w - 12.0)) * (n - 1) as f64).round().clamp(0.0, (n - 1) as f64) as usize };
        let (day, v, record) = &points[i];
        let mark = if *record { " · a record" } else { "" };
        tooltip.set_text(Some(&format!("{}\n{} {unit}{mark}", human_date(day), trimmed(*v))));
        true
    });
    area
}

// ---- The pages -----------------------------------------------------------------------------

/// Avex's tabs as Dev's segmented control, above each Avex page.
pub fn tabs(showing: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("money-tabs");
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tabs.add_css_class("view-tabs");
    tabs.set_halign(gtk::Align::Start);
    for (name, caption) in PAGES {
        let tab = button(caption, "quiet");
        tab.set_widget_name(&format!("{name}-tab"));
        if name == showing {
            tab.add_css_class("selected");
        }
        tab.connect_clicked(move |_| {
            if let Some(ui) = super::threads::the_ui_pub() {
                ui.navigate(name);
            }
        });
        tabs.append(&tab);
    }
    row.append(&tabs);
    row
}

/// A title and a first message on every page, before its first read.
pub fn install() {
    for (name, title) in ALL.into_iter().zip(["Avex", "Workouts", "Lifts", "Data", "Workout", "Lift"]) {
        if let Some(page) = page(name) {
            heading(&page.head, title, "", &[]);
            message(&page.body, "Reading Avex…");
        }
    }
}

/// Open workout `id` on the Workout page.
pub(super) fn open_workout(ui: &Rc<Ui>, id: i64) {
    STATE.with(|s| s.workout.set(Some(id)));
    if *ui.page.borrow() == WORKOUT {
        ui.refresh_page();
    } else {
        ui.navigate(WORKOUT);
    }
}

/// Open lift `name` on the Lift page.
pub(super) fn open_lift(ui: &Rc<Ui>, name: &str) {
    STATE.with(|s| *s.lift.borrow_mut() = Some(name.to_string()));
    if *ui.page.borrow() == LIFT {
        ui.refresh_page();
    } else {
        ui.navigate(LIFT);
    }
}

pub async fn refresh(ui: &Rc<Ui>, name: &str) {
    let Some(page) = page(name) else { return };
    match name {
        "avex-home" => {
            let read = ui.call("gym.summary", json!({})).await;
            if *ui.page.borrow() != name {
                return;
            }
            match read {
                Ok(s) => home(ui, &page, &s),
                Err(e) => failed(ui, &page, "Avex", &e),
            }
        }
        "avex-workouts" => {
            let shown = STATE.with(|s| s.shown.get()).max(PAGE_SIZE);
            let (list, summary) = tokio::join!(ui.call("gym.sessions", json!({"limit": shown})), ui.call("gym.summary", json!({})));
            if *ui.page.borrow() != name {
                return;
            }
            match list {
                Ok(l) => workouts_page(ui, &page, &l, summary.ok().as_ref()),
                Err(e) => failed(ui, &page, "Workouts", &e),
            }
        }
        "avex-lifts" => {
            let read = ui.call("gym.lifts", json!({})).await;
            if *ui.page.borrow() != name {
                return;
            }
            match read {
                Ok(l) => lifts_page(ui, &page, &l),
                Err(e) => failed(ui, &page, "Lifts", &e),
            }
        }
        "avex-data" => {
            let read = ui.call("gym.summary", json!({})).await;
            if *ui.page.borrow() != name {
                return;
            }
            match read {
                Ok(s) => data_page(ui, &page, &s),
                Err(e) => failed(ui, &page, "Data", &e),
            }
        }
        WORKOUT => {
            // A display smoke run opens a workout with RELAY_NATIVE_AVEX=<id>.
            let smoke = std::env::var("RELAY_NATIVE_AVEX").ok().and_then(|v| v.parse().ok());
            let Some(id) = STATE.with(|s| s.workout.get()).or(smoke) else {
                ui.navigate("avex-workouts");
                return;
            };
            let read = ui.call("gym.session.get", json!({"id": id})).await;
            if *ui.page.borrow() != name || STATE.with(|s| s.workout.get()).is_some_and(|w| w != id) {
                return;
            }
            match read {
                Ok(d) => workout_page(ui, &page, &d),
                Err(e) => failed(ui, &page, "Workout", &e),
            }
        }
        LIFT => {
            // A display smoke run opens a lift with RELAY_NATIVE_AVEX=<name>.
            let smoke = std::env::var("RELAY_NATIVE_AVEX").ok();
            let Some(lift) = STATE.with(|s| s.lift.borrow().clone()).or(smoke) else {
                ui.navigate("avex-lifts");
                return;
            };
            let read = ui.call("gym.lift.get", json!({"name": lift, "limit": 400})).await;
            if *ui.page.borrow() != name || STATE.with(|s| s.lift.borrow().as_deref().is_some_and(|l| l != lift)) {
                return;
            }
            match read {
                Ok(d) => lift_page(ui, &page, &d),
                Err(e) => failed(ui, &page, "Lift", &e),
            }
        }
        _ => {}
    }
}

fn failed(ui: &Rc<Ui>, page: &Page, name: &str, error: &Error) {
    heading(&page.head, name, "", &[]);
    message(&page.body, &said(error));
    let retry = button("Try again", "money-secondary");
    retry.set_halign(gtk::Align::Start);
    let weak = Rc::downgrade(ui);
    retry.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.refresh_page();
        }
    });
    page.body.append(&retry);
}

/// Before anything was imported: what Avex is to Relay, and the three steps to bring it in.
fn empty_state(ui: &Rc<Ui>, body: &gtk::Box) {
    let rows = card(body, "Bring in your training", None);
    let b = block(&rows);
    let about = label(
        "Avex keeps everything on your phone and has no internet, so Relay reads the file it exports. Nothing goes back to the phone, and Avex is never changed from here.",
        "money-muted",
    );
    about.set_wrap(true);
    b.append(&about);
    for (n, step) in [
        "In Avex: Settings, Export, Training history (JSON).",
        "Send avex_export.json to this computer: a cable, a cloud drive or Quick Share.",
        "Import it here. Import again whenever you want Relay to catch up; each import replaces the last.",
    ]
    .iter()
    .enumerate()
    {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("arbiter-steps");
        let number = label(&(n + 1).to_string(), "arbiter-step-number");
        number.set_valign(gtk::Align::Start);
        number.set_xalign(0.5);
        row.append(&number);
        let l = label(step, "arbiter-step");
        l.set_wrap(true);
        l.set_hexpand(true);
        row.append(&l);
        b.append(&row);
    }
    let key = import_key(ui, "Import Avex export…", "primary");
    key.set_halign(gtk::Align::Start);
    b.append(&key);
}

/// Overview: when the copy is from, this week beside the twelve before, then recent workouts and
/// records beside the lifts done most and open goals.
fn home(ui: &Rc<Ui>, page: &Page, s: &Value) {
    let unit = text(s, "unit").to_string();
    if s["imported"].is_null() {
        heading(&page.head, "Avex", "Your training, read from Avex's export", &[]);
        clear(&page.body);
        empty_state(ui, &page.body);
        return;
    }
    let refresh = import_key(ui, "Import newer…", "");
    let mut context = exported_line(&s["imported"]);
    if let Some(name) = s["imported"]["user_name"].as_str().filter(|n| !n.is_empty()) {
        context = format!("{name}'s training · {context}");
    }
    heading(&page.head, "Avex", &context, &[refresh.upcast_ref()]);
    clear(&page.body);
    let body = &page.body;

    let week = &s["this_week"];
    let last = &s["last_week"];
    let target = s["imported"]["days_per_week"].as_i64().unwrap_or(0);
    let (_, cells) = strip(body);
    let sessions = week["sessions"].as_i64().unwrap_or(0);
    let of = if target > 0 { format!(" of {target}") } else { String::new() };
    cell(&cells, "calendar", "This week", &format!("{sessions}{of}"), &format!("Last week {}", last["sessions"].as_i64().unwrap_or(0)), None);
    let vol = num(&week["volume"]).unwrap_or(0.0);
    let before = num(&last["volume"]).unwrap_or(0.0);
    let change = if before > 0.0 { format!("{:+.0}% on last week", (vol / before - 1.0) * 100.0) } else { String::from("Nothing last week") };
    cell(&cells, "chart", "Volume", &volume(&week["volume"], &unit), &change, None);
    let streak = s["streak_weeks"].as_i64().unwrap_or(0);
    cell(&cells, "spark", "Streak", &plural(streak.max(0) as usize, "week"), "In a row with a workout", None);
    let since = match s["days_since_last"].as_i64() {
        Some(0) => String::from("Today"),
        Some(1) => String::from("Yesterday"),
        Some(d) => format!("{d} days ago"),
        None => String::from("Never"),
    };
    let minutes_week = week["active_minutes"].as_i64().unwrap_or(0);
    cell(&cells, "clock", "Last workout", &since, &format!("{} training this week", minutes(minutes_week)), None);

    // Twelve weeks of volume, this one last.
    let weeks = rows(s, "weeks");
    if !weeks.is_empty() {
        let rows = card_asking(body, "Volume by week", None, Some((ui, "How has my training volume changed over the last few weeks?")));
        let b = block(&rows);
        let values = weeks.iter().map(|w| num(&w["volume"]).unwrap_or(0.0)).collect();
        let labels = weeks.iter().map(|w| format!("Week of {}", human_date(text(w, "start")))).collect();
        b.append(&bars(values, labels, if unit == "kg" { "kg" } else { "lb" }, 96));
    }

    let pair = columns(body, 2);
    let recent = card(&pair[0], "Recent workouts", Some(("All workouts", go(ui, "avex-workouts"))));
    let list = rows(s, "recent");
    if list.is_empty() {
        quiet_row(&recent, "No workouts in this export.");
    }
    for w in &list {
        recent.append(&workout_row(ui, w, &unit));
    }
    let records = card(&pair[0], "Records", None);
    let list = rows(s, "records");
    if list.is_empty() {
        quiet_row(&records, "No records marked by Avex yet.");
    }
    for r in &list {
        records.append(&record_row(ui, r, &unit));
    }

    let lifts = card_asking(&pair[1], "Lifts", Some(("All lifts", go(ui, "avex-lifts"))), Some((ui, "Which of my lifts are progressing and which have stalled?")));
    let list = rows(s, "lifts");
    if list.is_empty() {
        quiet_row(&lifts, "No lifts with working sets yet.");
    }
    for l in &list {
        lifts.append(&lift_row(ui, l, &unit));
    }
    let goals = rows(s, "goals");
    if !goals.is_empty() {
        let card = card(&pair[1], "Goals", None);
        for g in &goals {
            card.append(&goal_row(g));
        }
    }
}

/// A workout as a row: its title over the day and lifts, its volume; opens the Workout page.
fn workout_row(ui: &Rc<Ui>, w: &Value, unit: &str) -> gtk::Box {
    let lifts: Vec<&str> = w["lifts"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut detail = vec![human_date(text(w, "day"))];
    if w["minutes"].as_i64().unwrap_or(0) > 0 {
        detail.push(minutes(w["minutes"].as_i64().unwrap_or(0)));
    }
    if !lifts.is_empty() {
        detail.push(lifts.join(", "));
    }
    let mut title = text(w, "title").to_string();
    if w["untracked"] == true {
        title.push_str(" · untracked");
    }
    let prs = w["prs"].as_i64().unwrap_or(0);
    if prs > 0 {
        title.push_str(&format!(" · {}", plural(prs as usize, "record")));
    }
    let line = icon_row(&mark("lift"), &title, &detail.join(" · "), &volume(&w["volume"], unit), if w["untracked"] == true { "money-muted" } else { "" });
    let weak = Rc::downgrade(ui);
    let id = w["id"].as_i64().unwrap_or(0);
    row_key(
        &line,
        "Open this workout",
        Rc::new(move || {
            if let Some(ui) = weak.upgrade() {
                open_workout(&ui, id);
            }
        }),
    )
}

fn record_row(ui: &Rc<Ui>, r: &Value, unit: &str) -> gtk::Box {
    let detail = match num(&r["e1rm"]) {
        Some(e) => format!("{} · estimated max {} {unit}", human_date(text(r, "day")), trimmed(e)),
        None => human_date(text(r, "day")),
    };
    let line = icon_row(&mark("record"), text(r, "lift"), &detail, &set_words(&r["weight"], &r["reps"], unit), "money-in");
    let weak = Rc::downgrade(ui);
    let name = text(r, "lift").to_string();
    row_key(
        &line,
        "Open this lift",
        Rc::new(move || {
            if let Some(ui) = weak.upgrade() {
                open_lift(&ui, &name);
            }
        }),
    )
}

/// A lift as a row: how often, its best set, and the last four weeks beside the four before.
fn lift_row(ui: &Rc<Ui>, l: &Value, unit: &str) -> gtk::Box {
    let mut detail = vec![plural(l["sessions"].as_i64().unwrap_or(0).max(0) as usize, "workout"), format!("last {}", human_date(text(l, "last_day")))];
    if let Some(best) = l["best"].as_object() {
        detail.push(format!("best {}", set_words(&best["weight"], &best["reps"], unit)));
    }
    let (figure, class) = match (num(&l["recent_e1rm"]), num(&l["change_pct"])) {
        (Some(e), Some(c)) => (format!("{} {unit} {:+.1}%", trimmed(e), c), if c > 0.0 { "money-in" } else if c < 0.0 { "money-ahead" } else { "" }),
        (Some(e), None) => (format!("{} {unit}", trimmed(e)), ""),
        _ => (String::new(), ""),
    };
    let line = icon_row(&mark("lift"), text(l, "name"), &detail.join(" · "), &figure, class);
    if !figure.is_empty() {
        line.set_tooltip_text(Some("Best estimated one-rep max of the last four weeks, beside the four before"));
    }
    let weak = Rc::downgrade(ui);
    let name = text(l, "name").to_string();
    row_key(
        &line,
        "Open this lift",
        Rc::new(move || {
            if let Some(ui) = weak.upgrade() {
                open_lift(&ui, &name);
            }
        }),
    )
}

fn goal_row(g: &Value) -> gtk::Box {
    let kind = super::pages::word(text(g, "kind"));
    let target = match (text(g, "target_key"), num(&g["target_value"])) {
        ("", Some(v)) => trimmed(v),
        (k, Some(v)) => format!("{} · {}", super::pages::word(k), trimmed(v)),
        (k, None) => super::pages::word(k),
    };
    let detail = match text(g, "note") {
        "" => format!("Set {}", human_date(text(g, "created_day"))),
        note => note.to_string(),
    };
    icon_row(&mark("target"), &format!("{kind}: {target}"), &detail, "", "")
}

/// Workouts: every one, newest first, a page at a time.
fn workouts_page(ui: &Rc<Ui>, page: &Page, l: &Value, summary: Option<&Value>) {
    let unit = summary.map(|s| text(s, "unit").to_string()).unwrap_or_else(|| String::from("lb"));
    let total = l["total"].as_u64().unwrap_or(0);
    let context = match summary.filter(|s| !s["imported"].is_null()) {
        Some(s) => format!("{} · {}", plural(total as usize, "workout"), exported_line(&s["imported"])),
        None => String::from("Nothing imported from Avex yet"),
    };
    heading(&page.head, "Workouts", &context, &[]);
    clear(&page.body);
    let list = rows(l, "sessions");
    if list.is_empty() {
        if summary.is_some_and(|s| s["imported"].is_null()) {
            empty_state(ui, &page.body);
        } else {
            message(&page.body, "No workouts in this export.");
        }
        return;
    }
    // Grouped by month, newest first.
    let mut current = String::new();
    let mut card_rows: Option<gtk::Box> = None;
    for w in &list {
        let month = text(w, "day").get(..7).unwrap_or("").to_string();
        if month != current || card_rows.is_none() {
            current = month;
            let caption = super::pages::date(&format!("{current}-01"))
                .and_then(|d| d.format("%B %Y").ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| current.clone());
            card_rows = Some(card(&page.body, &caption, None));
        }
        if let Some(rows) = &card_rows {
            rows.append(&workout_row(ui, w, &unit));
        }
    }
    if (list.len() as u64) < total {
        let more = button(&format!("Show {} more", (total - list.len() as u64).min(u64::from(PAGE_SIZE))), "money-secondary");
        more.set_halign(gtk::Align::Center);
        let weak = Rc::downgrade(ui);
        more.connect_clicked(move |_| {
            STATE.with(|s| s.shown.set(s.shown.get().max(PAGE_SIZE) + PAGE_SIZE));
            if let Some(ui) = weak.upgrade() {
                ui.refresh_page();
            }
        });
        page.body.append(&more);
    }
}

/// One workout: its figures, the journal, and each exercise with its sets.
fn workout_page(ui: &Rc<Ui>, page: &Page, d: &Value) {
    let unit = text(d, "unit").to_string();
    let row = &d["row"];
    let back = button("All workouts", "money-secondary");
    let run = go(ui, "avex-workouts");
    back.connect_clicked(move |_| run());
    let mut context = vec![human_date(text(row, "day")), text(row, "time").to_string()];
    if row["untracked"] == true {
        context.push(String::from("untracked: left out of every total"));
    }
    heading(&page.head, text(row, "title"), &context.join(" · "), &[back.upcast_ref()]);
    clear(&page.body);
    let body = &page.body;
    let (_, cells) = strip(body);
    cell(&cells, "clock", "Training", &minutes(row["minutes"].as_i64().unwrap_or(0)), "", None);
    cell(&cells, "list", "Sets", &row["sets"].as_i64().unwrap_or(0).to_string(), "", None);
    cell(&cells, "chart", "Volume", &volume(&row["volume"], &unit), "", None);
    let prs = row["prs"].as_i64().unwrap_or(0);
    cell(&cells, "spark", "Records", &prs.to_string(), "", if prs > 0 { Some("money-in") } else { None });
    let journal = text(d, "journal");
    let mood = text(row, "mood");
    if !journal.is_empty() || !mood.is_empty() {
        let rows = card(body, "Journal", None);
        let b = block(&rows);
        if !mood.is_empty() {
            b.append(&label(&format!("Mood: {}", super::pages::word(mood)), "money-muted"));
        }
        if !journal.is_empty() {
            let l = label(journal, "arbiter-sentence");
            l.set_wrap(true);
            l.set_selectable(true);
            b.append(&l);
        }
    }
    for e in rows(d, "exercises") {
        let mut name = text(&e, "name").to_string();
        if e["was_pr"] == true {
            name.push_str(" · record");
        }
        let lift_name = text(&e, "name").to_string();
        let weak = Rc::downgrade(ui);
        let open: Action = Rc::new(move || {
            if let Some(ui) = weak.upgrade() {
                open_lift(&ui, &lift_name);
            }
        });
        let skipped = e["skipped"] == true;
        let sets = card(body, &name, if skipped { None } else { Some(("History", open)) });
        if skipped {
            quiet_row(&sets, "Skipped");
            continue;
        }
        if let Some(note) = e["note"].as_str().filter(|n| !n.is_empty()) {
            quiet_row(&sets, note);
        }
        // Working sets are numbered on their own; a warm-up shows as W.
        let mut number = 0;
        for s in rows(&e, "sets") {
            if text(&s, "kind") != "warmup" {
                number += 1;
            }
            sets.append(&set_row(number, &s, &unit));
        }
        let mut foot = vec![volume(&e["volume"], &unit)];
        if let Some(best) = num(&e["best_e1rm"]) {
            foot.push(format!("estimated max {} {unit}", trimmed(best)));
        }
        if let Some(d) = e["difficulty"].as_str().filter(|d| !d.is_empty()) {
            foot.push(format!("felt {}", super::pages::word(d).to_lowercase()));
        }
        quiet_row(&sets, &foot.join(" · "));
    }
}

/// One set: its number among the working sets (W for a warm-up), weight × reps, its marks, and
/// its estimated max.
fn set_row(number: usize, s: &Value, unit: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("tally-row");
    row.add_css_class("avex-set");
    let kind = text(s, "kind");
    let warm = kind == "warmup";
    let n = label(&if warm { String::from("W") } else { number.to_string() }, "arbiter-step-number");
    n.set_xalign(0.5);
    n.set_valign(gtk::Align::Center);
    row.append(&n);
    let words = label(&set_words(&s["weight"], &s["reps"], unit), "arbiter-stat");
    if warm {
        words.add_css_class("money-muted");
    }
    words.set_hexpand(true);
    words.set_xalign(0.0);
    row.append(&words);
    let mut marks = Vec::new();
    if !warm && kind != "work" {
        marks.push(super::pages::word(kind));
    }
    if let Some(rpe) = num(&s["rpe"]) {
        marks.push(format!("RPE {}", trimmed(rpe)));
    }
    if s["amrap"] == true {
        marks.push(String::from("AMRAP"));
    }
    if s["to_failure"] == true {
        marks.push(String::from("to failure"));
    }
    if s["assisted"] == true {
        marks.push(String::from("assisted"));
    }
    if !marks.is_empty() {
        row.append(&label(&marks.join(" · "), "money-row-detail"));
    }
    if let Some(e) = num(&s["e1rm"]) {
        let l = label(&format!("~{} {unit}", trimmed(e)), "tally-figures");
        l.set_width_chars(10);
        l.set_xalign(1.0);
        if s["best"] == true {
            l.add_css_class("money-in");
            l.set_tooltip_text(Some("The best set: its estimated one-rep max"));
        } else {
            l.set_tooltip_text(Some("Estimated one-rep max (Epley)"));
        }
        row.append(&l);
    }
    row
}

/// Lifts: every one, done most first.
fn lifts_page(ui: &Rc<Ui>, page: &Page, l: &Value) {
    let unit = text(l, "unit").to_string();
    let list = rows(l, "lifts");
    heading(&page.head, "Lifts", &format!("{} · the figure is the best estimated max of the last four weeks, beside the four before", plural(list.len(), "lift")), &[]);
    clear(&page.body);
    if list.is_empty() {
        message(&page.body, "No lifts yet. Import Avex's training history on the Data tab.");
        return;
    }
    let rows = card_asking(&page.body, "All lifts", None, Some((ui, "Which of my lifts are progressing and which have stalled?")));
    for lift in &list {
        rows.append(&lift_row(ui, lift, &unit));
    }
}

/// One lift: its best, its last four weeks, its estimated max over time, and every session.
fn lift_page(ui: &Rc<Ui>, page: &Page, d: &Value) {
    let unit = text(d, "unit").to_string();
    let row = &d["row"];
    let name = text(row, "name").to_string();
    let back = button("All lifts", "money-secondary");
    let run = go(ui, "avex-lifts");
    back.connect_clicked(move |_| run());
    heading(
        &page.head,
        &name,
        &format!("{} · {} sets · last {}", plural(row["sessions"].as_i64().unwrap_or(0).max(0) as usize, "workout"), row["sets"].as_i64().unwrap_or(0), human_date(text(row, "last_day"))),
        &[back.upcast_ref()],
    );
    clear(&page.body);
    let body = &page.body;
    let (_, cells) = strip(body);
    let best = &row["best"];
    match num(&best["e1rm"]) {
        Some(e) => cell(&cells, "spark", "Best estimated max", &format!("{} {unit}", trimmed(e)), &human_date(text(best, "day")), None),
        None => cell(&cells, "spark", "Best estimated max", "—", "No set of 1 to 12 reps", None),
    }
    if best.is_object() {
        cell(&cells, "chart", "Best set", &set_words(&best["weight"], &best["reps"], &unit), "", None);
    }
    let (recent, change) = (num(&row["recent_e1rm"]), num(&row["change_pct"]));
    let sub = change.map_or_else(|| String::from("Nothing in the four weeks before"), |c| format!("{c:+.1}% on the four weeks before"));
    let class = change.and_then(|c| if c > 0.0 { Some("money-in") } else if c < 0.0 { Some("money-ahead") } else { None });
    cell(&cells, "calendar", "Last four weeks", &recent.map_or_else(|| String::from("—"), |e| format!("{} {unit}", trimmed(e))), &sub, class);

    let history = rows(d, "history");
    let mut points: Vec<(String, f64, bool)> = history.iter().filter_map(|p| Some((text(p, "day").to_string(), num(&p["e1rm"])?, p["record"] == true))).collect();
    points.reverse();
    if points.len() >= 2 {
        let question = format!("How is my {name} progressing, and what should I change?");
        let rows = card_asking(body, "Estimated max over time", None, Some((ui, &question)));
        let b = block(&rows);
        b.append(&progress_line(points, unit.clone(), 150));
        let l = label("Each workout's best set as an estimated one-rep max; records in lime.", "money-row-detail");
        l.set_wrap(true);
        b.append(&l);
    }
    let rows = card(body, "Every workout", None);
    for p in &history {
        let mut detail = vec![format!("{} sets", p["sets"].as_i64().unwrap_or(0)), volume(&p["volume"], &unit)];
        if let Some(e) = num(&p["e1rm"]) {
            detail.push(format!("estimated max {} {unit}", trimmed(e)));
        }
        let top = if p["top_weight"].is_null() { format!("× {}", p["top_reps"].as_i64().unwrap_or(0)) } else { set_words(&p["top_weight"], &p["top_reps"], &unit) };
        let mut title = human_date(text(p, "day"));
        if p["record"] == true {
            title.push_str(" · record");
        }
        let line = icon_row(&mark(if p["record"] == true { "record" } else { "lift" }), &title, &detail.join(" · "), &top, if p["record"] == true { "money-in" } else { "" });
        let weak = Rc::downgrade(ui);
        let id = p["session_id"].as_i64().unwrap_or(0);
        rows.append(&row_key(
            &line,
            "Open this workout",
            Rc::new(move || {
                if let Some(ui) = weak.upgrade() {
                    open_workout(&ui, id);
                }
            }),
        ));
    }
}

/// Data: where the copy came from, importing a newer one, and forgetting it.
fn data_page(ui: &Rc<Ui>, page: &Page, s: &Value) {
    heading(&page.head, "Data", "Avex's history lives on this PC in gym.db beside Relay's store, read only. Nothing here goes back to the phone.", &[]);
    clear(&page.body);
    let imported = !s["imported"].is_null();
    let pair = columns(&page.body, 2);
    let file = card(&pair[0], "Avex export", None);
    let key = import_key(ui, if imported { "Import newer…" } else { "Import…" }, "");
    setting_row(&file, "download", "Import Avex's training history", "In Avex: Settings, Export, Training history (JSON). It replaces what is here.", &key);
    let forget = button("Forget…", "money-destructive");
    forget.set_widget_name("avex-reset");
    forget.set_sensitive(imported);
    setting_row(&file, "trash", "Forget the copy", "Erase the imported history on this PC. Avex keeps its own.", &forget);
    let weak = Rc::downgrade(ui);
    confirm_inline(&forget, "Forget it", move |key| {
        let Some(ui) = weak.upgrade() else { return };
        key.set_sensitive(false);
        glib::spawn_future_local(async move {
            match ui.call("gym.reset", json!({})).await {
                Ok(_) => toast(&ui, "Forgotten. Import an export to bring it back.", None),
                Err(e) => ui.show_error(&e.to_string()),
            }
            ui.refresh_page();
        });
    });

    let here = card(&pair[1], "What is here", None);
    if !imported {
        quiet_row(&here, "Nothing yet.");
        return;
    }
    let i = &s["imported"];
    let t = &s["totals"];
    let mut lines = vec![
        ("calendar", String::from("Exported"), format!("{} · Avex {}", text(i, "exported_at"), text(i, "app_version"))),
        ("download", String::from("Imported"), crate::relative::ago(text(i, "imported_at"), crate::relative::Form::Long).unwrap_or_default()),
        ("data", String::from("File"), text(i, "file").to_string()),
        ("lift", String::from("Workouts"), format!("{} · {} sets", group(t["sessions"].as_i64().unwrap_or(0)), group(t["sets"].as_i64().unwrap_or(0)))),
        ("run", String::from("Cardio"), format!("{} · {}", plural(t["cardio"].as_i64().unwrap_or(0).max(0) as usize, "entry").replace("entrys", "entries"), minutes(t["cardio_minutes"].as_i64().unwrap_or(0)))),
        ("gauge", String::from("Units"), if text(s, "unit") == "kg" { String::from("Kilograms, as Avex shows them") } else { String::from("Pounds, as Avex shows them") }),
    ];
    if let (Some(first), Some(last)) = (t["first_day"].as_str(), t["last_day"].as_str()) {
        lines.insert(4, ("clock", String::from("Spanning"), format!("{} to {}", human_date(first), human_date(last))));
    }
    for (icon_key, title, detail) in lines {
        let lead = if glyph(icon_key).is_some() { mark(icon_key) } else { tile(icon_key) };
        here.append(&icon_row(&lead, &title, &detail, "", ""));
    }
}

/// Choose Avex's export and import it, replacing what is here.
pub(super) fn import(ui: &Rc<Ui>) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Avex training history (.json)"));
        filter.add_pattern("*.json");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title("Import Avex's training history").filters(&filters).default_filter(&filter).build();
        let Ok(file) = dialog.open_future(Some(&ui.window)).await else { return };
        let Some(path) = file.path() else {
            ui.show_error("That file is not on this PC's disk. Copy it here first.");
            return;
        };
        match ui.call("gym.import", json!({"path": path.to_string_lossy()})).await {
            Ok(v) => toast(
                &ui,
                &format!(
                    "Imported {} and {} from Avex, exported {}.",
                    plural(v["sessions"].as_u64().unwrap_or(0) as usize, "workout"),
                    plural(v["sets"].as_u64().unwrap_or(0) as usize, "set"),
                    text(&v, "exported_at")
                ),
                None,
            ),
            Err(e) => ui.show_error(&said(&e)),
        }
        ui.refresh_page();
    });
}

// ---- The thread panel ----------------------------------------------------------------------

pub(super) async fn panel_read(ui: &Rc<Ui>, tab: &str) -> Result<Value, Error> {
    match tab {
        "avex-workouts" => ui.call("gym.sessions", json!({"limit": 20})).await,
        "avex-lifts" => ui.call("gym.lifts", json!({})).await,
        _ => ui.call("gym.summary", json!({})).await,
    }
}

pub(super) fn panel_draw(ui: &Rc<Ui>, body: &gtk::Box, tab: &str, v: &Value) {
    match tab {
        "avex-workouts" => panel_workouts(ui, body, v),
        "avex-lifts" => panel_lifts(ui, body, v),
        _ => panel_overview(ui, body, v),
    }
}

/// A titled block of the thread panel, as Tally's panel draws them.
fn panel_block(body: &gtk::Box, name: &str, aside: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("threads-block");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = label(name, "threads-caption");
    title.set_hexpand(true);
    head.append(&title);
    if !aside.is_empty() {
        head.append(&label(aside, "threads-panel-aside"));
    }
    b.append(&head);
    body.append(&b);
    b
}

/// A panel row: a name over a detail, a figure at the end, the whole of it a key.
fn panel_row(name: &str, detail: &str, figure: &str, figure_class: &str, about: &str, run: Action) -> gtk::Button {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let words = gtk::Box::new(gtk::Orientation::Vertical, 1);
    words.set_hexpand(true);
    let n = label(name, "threads-panel-name");
    n.set_ellipsize(gtk::pango::EllipsizeMode::End);
    words.append(&n);
    if !detail.is_empty() {
        let d = label(detail, "threads-panel-detail");
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        words.append(&d);
    }
    row.append(&words);
    if !figure.is_empty() {
        let f = label(figure, "threads-panel-figures");
        if !figure_class.is_empty() {
            f.add_css_class(figure_class);
        }
        f.set_valign(gtk::Align::Center);
        row.append(&f);
    }
    let key = gtk::Button::new();
    key.add_css_class("arbiter-panel-row");
    key.set_child(Some(&row));
    key.set_tooltip_text(Some(about));
    key.connect_clicked(move |_| run());
    key
}

fn open_workout_action(ui: &Rc<Ui>, id: i64) -> Action {
    let weak = Rc::downgrade(ui);
    Rc::new(move || {
        if let Some(ui) = weak.upgrade() {
            open_workout(&ui, id);
        }
    })
}

fn open_lift_action(ui: &Rc<Ui>, name: &str) -> Action {
    let weak = Rc::downgrade(ui);
    let name = name.to_string();
    Rc::new(move || {
        if let Some(ui) = weak.upgrade() {
            open_lift(&ui, &name);
        }
    })
}

fn panel_open_key(ui: &Rc<Ui>, body: &gtk::Box) {
    let open = button("Open Avex", "");
    open.set_halign(gtk::Align::Start);
    let run = go(ui, "avex-home");
    open.connect_clicked(move |_| run());
    body.append(&open);
}

/// The panel's Overview: this week against the twelve before, recent workouts and records.
fn panel_overview(ui: &Rc<Ui>, body: &gtk::Box, s: &Value) {
    if s["imported"].is_null() {
        let note = label("Nothing from Avex yet. Export Training history in Avex (Settings, Export) and import it here.", "money-muted");
        note.set_wrap(true);
        body.append(&note);
        let key = import_key(ui, "Import Avex export…", "");
        key.set_halign(gtk::Align::Start);
        body.append(&key);
        return;
    }
    let unit = text(s, "unit").to_string();
    let head = gtk::Box::new(gtk::Orientation::Vertical, 6);
    head.add_css_class("threads-summary");
    let when = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    when.append(&icon("avex", 13));
    when.append(&label(&exported_line(&s["imported"]), "threads-panel-eyebrow"));
    head.append(&when);
    let week = &s["this_week"];
    let target = s["imported"]["days_per_week"].as_i64().unwrap_or(0);
    let sessions = week["sessions"].as_i64().unwrap_or(0);
    let of = if target > 0 { format!(" of {target}") } else { String::new() };
    head.append(&label(&format!("{sessions}{of} this week"), "threads-panel-figure"));
    head.append(&label(
        &format!("{} · {} streak", volume(&week["volume"], &unit), plural(s["streak_weeks"].as_i64().unwrap_or(0).max(0) as usize, "week")),
        "money-muted",
    ));
    let weeks = rows(s, "weeks");
    if !weeks.is_empty() {
        let values = weeks.iter().map(|w| num(&w["volume"]).unwrap_or(0.0)).collect();
        let labels = weeks.iter().map(|w| format!("Week of {}", human_date(text(w, "start")))).collect();
        let chart = bars(values, labels, if unit == "kg" { "kg" } else { "lb" }, 56);
        chart.set_margin_top(6);
        head.append(&chart);
    }
    body.append(&head);
    let recent = panel_block(body, "Recent", "");
    for w in rows(s, "recent").iter().take(4) {
        let lifts: Vec<&str> = w["lifts"].as_array().into_iter().flatten().filter_map(Value::as_str).take(3).collect();
        let detail = format!("{} · {}", human_date(text(w, "day")), lifts.join(", "));
        recent.append(&panel_row(text(w, "title"), &detail, &volume(&w["volume"], &unit), "", "Open this workout", open_workout_action(ui, w["id"].as_i64().unwrap_or(0))));
    }
    let records = rows(s, "records");
    if !records.is_empty() {
        let list = panel_block(body, "Records", "");
        for r in records.iter().take(4) {
            list.append(&panel_row(
                text(r, "lift"),
                &human_date(text(r, "day")),
                &set_words(&r["weight"], &r["reps"], &unit),
                "money-in",
                "Open this lift",
                open_lift_action(ui, text(r, "lift")),
            ));
        }
    }
    panel_open_key(ui, body);
}

fn panel_workouts(ui: &Rc<Ui>, body: &gtk::Box, l: &Value) {
    let list = rows(l, "sessions");
    let block = panel_block(body, "Workouts", &l["total"].as_u64().map(|t| t.to_string()).unwrap_or_default());
    if list.is_empty() {
        block.append(&label("Nothing from Avex yet.", "money-muted"));
    }
    for w in &list {
        let lifts: Vec<&str> = w["lifts"].as_array().into_iter().flatten().filter_map(Value::as_str).take(3).collect();
        let detail = format!("{} · {}", human_date(text(w, "day")), lifts.join(", "));
        let figure = format!("{} sets", w["sets"].as_i64().unwrap_or(0));
        block.append(&panel_row(text(w, "title"), &detail, &figure, "", "Open this workout", open_workout_action(ui, w["id"].as_i64().unwrap_or(0))));
    }
    panel_open_key(ui, body);
}

fn panel_lifts(ui: &Rc<Ui>, body: &gtk::Box, l: &Value) {
    let unit = text(l, "unit").to_string();
    let list = rows(l, "lifts");
    let block = panel_block(body, "Lifts", "Best set");
    if list.is_empty() {
        block.append(&label("Nothing from Avex yet.", "money-muted"));
    }
    for lift in list.iter().take(16) {
        let detail = match num(&lift["change_pct"]) {
            Some(c) => format!("{} · {c:+.1}% in four weeks", plural(lift["sessions"].as_i64().unwrap_or(0).max(0) as usize, "workout")),
            None => plural(lift["sessions"].as_i64().unwrap_or(0).max(0) as usize, "workout"),
        };
        let best = &lift["best"];
        let figure = if best.is_object() { set_words(&best["weight"], &best["reps"], &unit) } else { String::new() };
        block.append(&panel_row(text(lift, "name"), &detail, &figure, "", "Open this lift", open_lift_action(ui, text(lift, "name"))));
    }
    panel_open_key(ui, body);
}
