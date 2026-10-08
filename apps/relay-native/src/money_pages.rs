//! Money's pages: Home, Transactions, Plan and Data, drawn after Tally's screens.
use super::{formatter, message, page, remember_currency, Action, Page};
use crate::app::{button, clear, confirm_inline, label, rows, text, Ui};
use crate::client::Error;
use gtk::prelude::*;
use gtk4 as gtk;
use relay_money::copy;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// Tally's twelve category hues (apps/tally/DESIGN.md `hue-*`), by stored colour index.
pub const HUES: [&str; 12] = [
    "#7FB27A", "#4FA9A0", "#6A9FD8", "#8C87D9", "#C27BC0", "#D9768E", "#E08A5F", "#D9A441", "#A3A84E", "#C2A585",
    "#8D99A6", "#B8664F",
];

/// Strokes on the 16-unit grid for Money's own glyphs: its page keys and the category icon
/// keys of `relay_money::model::ICON_KEYS` that have an obvious drawing. The rest take a dot.
fn glyph(key: &str) -> Option<&'static str> {
    Some(match key {
        "home" | "apartment" | "hotel" => r##"<path d="M2.5 7.5L8 3l5.5 4.5M4 6.5V13h8V6.5" /><path d="M6.5 13V9.5h3V13" />"##,
        "list" => r##"<path d="M5.5 4h8M5.5 8h8M5.5 12h8" /><circle cx="2.8" cy="4" r=".6" /><circle cx="2.8" cy="8" r=".6" /><circle cx="2.8" cy="12" r=".6" />"##,
        "plan" => r##"<path d="M8 2.5a5.5 5.5 0 1 0 5.5 5.5H8z" /><path d="M10 1.8a5 5 0 0 1 4.2 4.2H10z" />"##,
        "data" => r##"<ellipse cx="8" cy="4" rx="5" ry="1.8" /><path d="M3 4v8c0 1 2.2 1.8 5 1.8s5-.8 5-1.8V4M3 8c0 1 2.2 1.8 5 1.8S13 9 13 8" />"##,
        "coins" | "savings" | "cash" | "invest" | "crypto" => {
            r##"<ellipse cx="6.5" cy="5" rx="4" ry="1.7" /><path d="M2.5 5v3c0 .9 1.8 1.7 4 1.7M2.5 8v3c0 .9 1.8 1.7 4 1.7" /><ellipse cx="10" cy="9.3" rx="3.5" ry="1.5" /><path d="M6.5 9.3v2.9c0 .8 1.6 1.5 3.5 1.5s3.5-.7 3.5-1.5V9.3" />"##
        }
        "cart" | "store" => r##"<path d="M2 3h2l1.5 7h7L14 5H4.5" /><circle cx="6.5" cy="12.8" r="1" /><circle cx="11.5" cy="12.8" r="1" />"##,
        "dining" | "lunch" | "kitchen" => r##"<path d="M5 2v12M3.5 2v3.5a1.5 1.5 0 0 0 3 0V2M11.5 14V2c-1.6.8-2.5 2.6-2.5 5h2.5" />"##,
        "coffee" | "tea" => r##"<path d="M3 6h8v4a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z" /><path d="M11 7h1a1.5 1.5 0 0 1 0 3h-1M5.5 2.5V4M8.5 2.5V4" />"##,
        "transport" | "car" | "taxi" | "ev" | "carrepair" | "parking" => {
            r##"<path d="M3 11V8l1.5-3.5h7L13 8v3z" /><path d="M3 11v1.5M13 11v1.5M3 8h10" /><circle cx="5.5" cy="9.5" r=".5" /><circle cx="10.5" cy="9.5" r=".5" />"##
        }
        "train" | "subway" => r##"<rect x="4" y="2" width="8" height="9.5" rx="2" /><path d="M4 7h8M6 14l1-2.5M10 14l-1-2.5" />"##,
        "bike" | "scooter" => r##"<circle cx="4" cy="10.5" r="2.5" /><circle cx="12" cy="10.5" r="2.5" /><path d="M4 10.5l2.5-5H10l2 5M6.5 5.5L8.5 10.5h-4" />"##,
        "fuel" => r##"<path d="M3 14V3.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1V14M2 14h9M3 7h7M10 6l2 1.5V12a1 1 0 0 0 2 0V6.5L12.5 5" />"##,
        "bolt" => r##"<path d="M9 2L4 9h4l-1 5 5-7H8z" />"##,
        "wifi" | "online" | "cloud" => r##"<path d="M2 6.5a9 9 0 0 1 12 0M4 9a5.5 5.5 0 0 1 8 0M6 11.3a2.5 2.5 0 0 1 4 0" /><circle cx="8" cy="13.2" r=".6" />"##,
        "phone" => r##"<rect x="4.5" y="2" width="7" height="12" rx="1.2" /><path d="M7 12h2" />"##,
        "bag" | "clothes" | "sale" => r##"<path d="M3 5.5h10l-1 8.5H4z" /><path d="M5.5 5.5V5a2.5 2.5 0 0 1 5 0v.5" />"##,
        "health" | "medical" | "hospital" | "wellness" | "therapy" => {
            r##"<path d="M8 13.5S2.5 10.2 2.5 6.3A2.8 2.8 0 0 1 8 5a2.8 2.8 0 0 1 5.5 1.3C13.5 10.2 8 13.5 8 13.5z" />"##
        }
        "medication" | "pharmacy" => r##"<rect x="2" y="5.5" width="12" height="5" rx="2.5" /><path d="M8 5.5v5" />"##,
        "ticket" | "movie" | "theatre" | "party" | "music" | "games" => {
            r##"<path d="M2.5 5h11v2a1 1 0 0 0 0 2v2h-11V9a1 1 0 0 0 0-2z" /><path d="M10 5.5v1M10 7.5v1M10 9.5v1" />"##
        }
        "repeat" => r##"<path d="M3 7V6a2 2 0 0 1 2-2h7.5M10.5 2l2 2-2 2M13 9v1a2 2 0 0 1-2 2H3.5M5.5 14l-2-2 2-2" />"##,
        "flight" | "luggage" | "beach" => r##"<path d="M14 2L2 6.5l4.5 2 2 4.5z" /><path d="M6.5 8.5L14 2" />"##,
        "gift" | "birthday" => {
            r##"<path d="M2.5 6h11v3h-11zM3.5 9v5h9V9M8 6v8" /><path d="M8 6C6.5 6 5 5.5 5 4.2S6.8 2.8 8 6c1.2-3.2 3-2.6 3-1.8S9.5 6 8 6" />"##
        }
        "dots" => {
            r##"<circle cx="3.5" cy="8" r=".9" fill="currentColor" stroke="none" /><circle cx="8" cy="8" r=".9" fill="currentColor" stroke="none" /><circle cx="12.5" cy="8" r=".9" fill="currentColor" stroke="none" />"##
        }
        "work" | "business" => r##"<path d="M2.5 5.5h11v7.5h-11z" /><path d="M6 5.5V4h4v1.5M2.5 9h11" />"##,
        "spark" | "star" | "idea" => r##"<path d="M8 2l1.5 4.5L14 8l-4.5 1.5L8 14l-1.5-4.5L2 8l4.5-1.5z" />"##,
        "refund" => r##"<path d="M5 4L2.5 6.5 5 9" /><path d="M2.5 6.5H10a3.5 3.5 0 0 1 0 7H6" />"##,
        "bank" | "loan" | "tax" | "legal" => r##"<path d="M2 6l6-3.5L14 6zM3.5 6v6M6.5 6v6M9.5 6v6M12.5 6v6M2 13.5h12" />"##,
        "card" | "wallet" | "atm" | "fees" => r##"<rect x="2" y="4" width="12" height="8.5" rx="1" /><path d="M2 7h12M4.5 10.5h2" />"##,
        "school" | "book" => {
            r##"<path d="M2.5 3.5H7a1 1 0 0 1 1 1V13a1.5 1.5 0 0 0-1.5-1.5h-4zM13.5 3.5H9a1 1 0 0 0-1 1V13a1.5 1.5 0 0 1 1.5-1.5h4z" />"##
        }
        "tv" | "computer" => r##"<rect x="2" y="3" width="12" height="8" rx="1" /><path d="M5.5 14h5M8 11v3" />"##,
        "water" => r##"<path d="M8 2.5S4 7 4 9.8a4 4 0 0 0 8 0C12 7 8 2.5 8 2.5z" />"##,
        "heat" => r##"<path d="M8 14a4 4 0 0 1-4-4c0-3 4-4 3-8 3 1.5 5 4.5 5 8a4 4 0 0 1-4 4z" />"##,
        "pets" => {
            r##"<ellipse cx="8" cy="11" rx="3" ry="2.5" /><circle cx="4" cy="7" r="1.2" /><circle cx="6.5" cy="4.3" r="1.2" /><circle cx="9.5" cy="4.3" r="1.2" /><circle cx="12" cy="7" r="1.2" />"##
        }
        "mail" | "delivery" => r##"<rect x="2" y="3.5" width="12" height="9" rx="1" /><path d="M2.5 4.5L8 9l5.5-4.5" />"##,
        "family" | "child" | "baby" | "elderly" => {
            r##"<circle cx="5.5" cy="5" r="2" /><circle cx="11" cy="6" r="1.5" /><path d="M2 13.5a3.5 3.5 0 0 1 7 0M8.5 13.5a2.5 2.5 0 0 1 5 0" />"##
        }
        "charity" | "church" => r##"<path d="M8 13.5S3 10.5 3 7a2.5 2.5 0 0 1 5-1 2.5 2.5 0 0 1 5 1c0 3.5-5 6.5-5 6.5z" /><path d="M2 13.5h12" />"##,
        "insurance" => r##"<path d="M8 2l5 2v3.5c0 3.3-2 5.5-5 6.8-3-1.3-5-3.5-5-6.8V4z" />"##,
        "sport" | "soccer" | "hiking" | "outdoors" | "pool" => r##"<circle cx="8" cy="8" r="5.5" /><path d="M8 5.5l2.4 1.7-.9 2.8h-3l-.9-2.8z" />"##,
        "haircut" | "beauty" => r##"<circle cx="4.5" cy="11.5" r="2" /><circle cx="11.5" cy="11.5" r="2" /><path d="M6 10L12 2.5M10 10L4 2.5" />"##,
        _ => return None,
    })
}

/// A Money glyph, or a dot for an icon key with no drawing.
pub fn glyph_image(key: &str, size: i32) -> gtk::Image {
    crate::icons::from_geometry(
        glyph(key).unwrap_or(r##"<circle cx="8" cy="8" r="3.2" fill="currentColor" stroke="none" />"##),
        size,
        1.5,
    )
}

/// A category's glyph in its hue, on a wash of that hue.
pub fn badge(icon: &str, color: i64) -> gtk::Box {
    let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tile.add_css_class("money-badge");
    tile.add_css_class(&format!("hue-{}", color.rem_euclid(HUES.len() as i64)));
    tile.set_valign(gtk::Align::Center);
    tile.set_halign(gtk::Align::Start);
    tile.set_hexpand(false);
    tile.set_size_request(34, 34);
    let image = glyph_image(icon, 18);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image.set_hexpand(true);
    tile.append(&image);
    tile
}

/// What a failed read says on the page. An engine without the ledger yet says so plainly.
pub fn unavailable(error: &Error) -> String {
    match error {
        Error::Bus(e) if matches!(e.code.as_str(), "bus.unknown_op" | "bus.not_implemented") => {
            format!("This engine has no money ledger yet ({}). Update Relay's engine to use Money.", e.message)
        }
        e => e.to_string(),
    }
}

fn n(v: &Value, key: &str) -> i64 {
    v[key].as_i64().unwrap_or(0)
}

fn date(value: &str) -> Option<glib::DateTime> {
    let mut parts = value.split('-').map(|p| p.parse::<i32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    glib::DateTime::from_local(y, m, d, 0, 0, 0.0).ok()
}

fn ymd(day: &glib::DateTime) -> String {
    day.format("%Y-%m-%d").map(|s| s.to_string()).unwrap_or_default()
}

pub fn today() -> String {
    glib::DateTime::now_local().map(|now| ymd(&now)).unwrap_or_default()
}

/// "Today", "Yesterday", "Tue 6 Oct"; another year adds it.
pub fn human_date(value: &str) -> String {
    let now = glib::DateTime::now_local().ok();
    if value == today() {
        return String::from("Today");
    }
    if now.as_ref().and_then(|n| n.add_days(-1).ok()).is_some_and(|y| ymd(&y) == value) {
        return String::from("Yesterday");
    }
    let Some(day) = date(value) else { return value.to_string() };
    let same_year = now.is_some_and(|n| n.year() == day.year());
    day.format(if same_year { "%a %-d %b" } else { "%a %-d %b %Y" }).map(|s| s.to_string()).unwrap_or_else(|_| value.to_string())
}

/// "October", or "15 Oct to 14 Nov" for a period that does not start on the 1st.
fn period_name(period: &Value) -> String {
    let (Some(start), Some(end)) = (date(text(period, "start")), date(text(period, "end_exclusive"))) else {
        return String::from("This period");
    };
    let last = end.add_days(-1).unwrap_or(end);
    let fmt = |d: &glib::DateTime, f: &str| d.format(f).map(|s| s.to_string()).unwrap_or_default();
    if start.day_of_month() == 1 && last.month() == start.month() {
        let year = glib::DateTime::now_local().map(|n| n.year()).unwrap_or(start.year());
        fmt(&start, if start.year() == year { "%B" } else { "%B %Y" })
    } else {
        format!("{} to {}", fmt(&start, "%-d %b"), fmt(&last, "%-d %b"))
    }
}

/// An enum name as a word: `CHEQUING` reads "Chequing".
fn word(value: &str) -> String {
    let lower = value.replace('_', " ").to_lowercase();
    let mut chars = lower.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

fn plural(n: usize, one: &str) -> String {
    copy::plural(n as i64, one)
}

/// How a meter's fill reads: Dev's ink, a category's hue, ahead of pace (amber, Dev's waiting),
/// or over budget (red).
#[derive(Clone, Copy)]
pub enum Tone {
    Plain,
    Hue(i64),
    Ahead,
    Over,
}

/// A budget's tone from its pace status, in its category's hue when it has one.
pub fn tone(status: &str, hue: Option<i64>) -> Tone {
    match status {
        "OVER_BUDGET" => Tone::Over,
        "OVER_PACE" => Tone::Ahead,
        _ => hue.map_or(Tone::Plain, Tone::Hue),
    }
}

/// A bar filled to `fraction` with the pace tick at `tick`, red when `over`.
pub fn meter(fraction: f64, tick: Option<f64>, over: bool, height: i32) -> gtk::DrawingArea {
    meter_in(fraction, tick, if over { Tone::Over } else { Tone::Plain }, height)
}

fn rgb(hex: &str) -> (f64, f64, f64) {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xEDE9E2);
    (f64::from((v >> 16) & 0xFF) / 255.0, f64::from((v >> 8) & 0xFF) / 255.0, f64::from(v & 0xFF) / 255.0)
}

/// [`meter`] filled in `tone`.
pub fn meter_in(fraction: f64, tick: Option<f64>, tone: Tone, height: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(height + 6);
    area.set_hexpand(true);
    area.add_css_class("money-meter");
    let fill = match tone {
        Tone::Plain => rgb("#EDE9E2"),
        Tone::Hue(h) => rgb(HUES[h.rem_euclid(HUES.len() as i64) as usize]),
        Tone::Ahead => rgb("#f0a828"),
        Tone::Over => rgb("#f0786f"),
    };
    area.set_draw_func(move |_, cr, width, _| {
        let (w, h, top) = (width as f64, height as f64, 3.0);
        let rounded = |x: f64, w: f64| {
            let r = (h / 2.0).min(w / 2.0);
            cr.new_sub_path();
            cr.arc(x + w - r, top + r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
            cr.arc(x + r, top + r, r, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();
        };
        let (r, g, b) = rgb("#2A2826");
        cr.set_source_rgb(r, g, b);
        rounded(0.0, w);
        let _ = cr.fill();
        let filled = (fraction.clamp(0.0, 1.0) * w).max(if fraction > 0.0 { h } else { 0.0 });
        if filled > 0.0 {
            cr.set_source_rgb(fill.0, fill.1, fill.2);
            rounded(0.0, filled);
            let _ = cr.fill();
        }
        if let Some(tick) = tick.filter(|t| (0.0..=1.0).contains(t)) {
            let x = (tick * w).clamp(1.0, w - 1.0);
            let (r, g, b) = rgb("#8C877F");
            cr.set_source_rgb(r, g, b);
            cr.set_line_width(2.0);
            cr.move_to(x, 0.0);
            cr.line_to(x, h + 6.0);
            let _ = cr.stroke();
        }
    });
    area
}

/// Over budget: red. Ahead of pace is amber ([`tone`]), not red.
fn over(status: &str) -> bool {
    status == "OVER_BUDGET"
}

/// The figure's class for a pace status: red over budget, amber ahead of pace.
pub fn tone_class(status: &str) -> Option<&'static str> {
    match status {
        "OVER_BUDGET" => Some("money-over"),
        "OVER_PACE" => Some("money-ahead"),
        _ => None,
    }
}

/// The page title in the serif voice, its context line under it.
fn title(head: &gtk::Box, name: &str, context: &str) {
    clear(head);
    head.append(&label(name, "money-title"));
    if !context.is_empty() {
        let line = label(context, "money-context");
        line.set_wrap(true);
        head.append(&line);
    }
}

/// A section: its mono name, an optional text action, and the box its rows go in.
fn section(parent: &gtk::Box, name: &str, action: Option<(&str, Action)>) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 10);
    block.add_css_class("money-section");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let caption = label(&name.to_uppercase(), "money-label");
    caption.set_hexpand(true);
    header.append(&caption);
    if let Some((text, run)) = action {
        let key = button(&format!("{text} →"), "money-text-action");
        key.connect_clicked(move |_| run());
        header.append(&key);
    }
    block.append(&header);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 2);
    rows.add_css_class("money-group");
    block.append(&rows);
    parent.append(&block);
    rows
}

fn go(ui: &Rc<Ui>, page: &'static str) -> Action {
    let weak = Rc::downgrade(ui);
    Rc::new(move || {
        if let Some(ui) = weak.upgrade() {
            if page == "money-transactions" {
                super::set_period_offset(0);
            }
            ui.navigate(page);
        }
    })
}

/// A plain reading row: start text over an optional detail, a figure at the end.
fn line_row(title: &str, detail: &str, figure: &str, figure_class: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("money-line");
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    let name = label(title, "money-row-title");
    name.set_wrap(true);
    words.append(&name);
    if !detail.is_empty() {
        let d = label(detail, "money-row-detail");
        d.set_wrap(true);
        words.append(&d);
    }
    row.append(&words);
    if !figure.is_empty() {
        let f = label(figure, "money-amount");
        if !figure_class.is_empty() {
            f.add_css_class(figure_class);
        }
        f.set_valign(gtk::Align::Center);
        row.append(&f);
    }
    row
}

/// Deletes `tx`, with Undo on a toast rather than a confirmation first (Tally's rule). Undo is
/// `money.tx.restore`, which brings back the same row, its sync id included.
pub fn delete(ui: &Rc<Ui>, tx: Value) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match ui.call("money.tx.delete", json!({"id":tx["id"]})).await {
            Ok(_) => {
                let what = match text(&tx, "category") {
                    "" if text(&tx, "type") == "TRANSFER" => String::from("Transfer"),
                    "" => String::from("Entry"),
                    name => name.to_string(),
                };
                let weak = Rc::downgrade(&ui);
                let id = tx["id"].clone();
                let undo: Action = Rc::new(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let id = id.clone();
                    glib::spawn_future_local(async move {
                        if let Err(e) = ui.call("money.tx.restore", json!({"id":id})).await {
                            ui.show_error(&e.to_string());
                        }
                        ui.refresh_page();
                    });
                });
                super::toast(&ui, &format!("{what} {} deleted", formatter().format(n(&tx, "amount"))), Some(undo));
                ui.refresh_page();
            }
            Err(e) => ui.show_error(&e.to_string()),
        }
    });
}

/// One entry as a tappable slab: its category glyph, what and where, and the amount. Clicking
/// it edits; the trash key (where `deletable`) deletes with Undo.
fn tx_row(ui: &Rc<Ui>, tx: &Value, deletable: bool) -> gtk::Box {
    let fmt = formatter();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("money-row");
    let main = button("", "money-row-key");
    main.set_hexpand(true);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let kind = text(tx, "type");
    let transfer = kind == "TRANSFER";
    inner.append(&badge(if transfer { "repeat" } else { text(tx, "icon") }, if transfer { 10 } else { n(tx, "color") }));
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    let what = if transfer {
        String::from("Transfer")
    } else {
        match text(tx, "category") {
            "" => word(kind),
            name => name.to_string(),
        }
    };
    let name = label(&what, "money-row-title");
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    words.append(&name);
    let mut detail = if transfer {
        format!("{} → {}", text(tx, "account"), text(tx, "to_account"))
    } else {
        text(tx, "account").to_string()
    };
    if !text(tx, "note").is_empty() {
        detail = format!("{detail} · {}", text(tx, "note"));
    }
    let d = label(&detail, "money-row-detail");
    d.set_ellipsize(gtk::pango::EllipsizeMode::End);
    words.append(&d);
    inner.append(&words);
    let amount = n(tx, "amount");
    let figure = match kind {
        "INCOME" => label(&fmt.format_signed(amount), "money-amount"),
        _ => label(&fmt.format(amount), "money-amount"),
    };
    match kind {
        "INCOME" => figure.add_css_class("money-in"),
        "TRANSFER" => figure.add_css_class("money-quiet"),
        _ => {}
    }
    figure.set_valign(gtk::Align::Center);
    inner.append(&figure);
    main.set_child(Some(&inner));
    main.set_tooltip_text(Some("Edit this entry"));
    let weak = Rc::downgrade(ui);
    let edit = tx.clone();
    main.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            super::edit_entry(&ui, edit.clone());
        }
    });
    row.append(&main);
    if deletable {
        let trash = crate::app::icon_button("trash", "Delete entry");
        trash.add_css_class("money-row-delete");
        trash.set_valign(gtk::Align::Center);
        let weak = Rc::downgrade(ui);
        let gone = tx.clone();
        trash.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                delete(&ui, gone.clone());
            }
        });
        row.append(&trash);
    }
    row
}

thread_local! {
    /// The Transactions header's live parts: context line, period caption, next key.
    static TX_HEAD: RefCell<Option<(gtk::Label, gtk::Label, gtk::Button)>> = const { RefCell::new(None) };
}

/// Builds the headers that keep their widgets across reads (Transactions' period keys and
/// search), and shows a first message on every page.
pub fn install(ui: &Rc<Ui>) {
    for (name, caption, _) in super::PAGES {
        if let Some(page) = page(name) {
            title(&page.head, caption, "");
            message(&page.body, "Reading the ledger…");
        }
    }
    let Some(page) = page("money-transactions") else { return };
    clear(&page.head);
    page.head.append(&label("Transactions", "money-title"));
    let context = label("", "money-context");
    page.head.append(&context);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.add_css_class("money-controls");
    let previous = crate::app::icon_button("chevron-left", "Previous period");
    previous.add_css_class("money-chrome");
    previous.set_widget_name("money-period-previous");
    let period = label("", "money-period");
    period.set_width_chars(16);
    period.set_xalign(0.5);
    let next = crate::app::icon_button("chevron-right", "Next period");
    next.add_css_class("money-chrome");
    next.set_widget_name("money-period-next");
    controls.append(&previous);
    controls.append(&period);
    controls.append(&next);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    controls.append(&spacer);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search notes, categories, accounts"));
    search.set_width_chars(30);
    search.add_css_class("money-search");
    search.set_widget_name("money-search");
    controls.append(&search);
    page.head.append(&controls);
    for (key, step) in [(&previous, -1_i64), (&next, 1)] {
        let weak = Rc::downgrade(ui);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                super::set_period_offset((super::period_offset() + step).min(0));
                ui.refresh_page();
            }
        });
    }
    let weak = Rc::downgrade(ui);
    search.connect_search_changed(move |entry| {
        if let Some(ui) = weak.upgrade() {
            super::set_query(entry.text().trim());
            ui.refresh_page();
        }
    });
    TX_HEAD.with(|h| *h.borrow_mut() = Some((context, period, next)));
}

pub async fn refresh(ui: &Rc<Ui>, name: &str) {
    let Some(page) = page(name) else { return };
    match name {
        "money-home" => {
            let result = ui.call("money.summary", json!({})).await;
            if *ui.page.borrow() != name {
                return;
            }
            match result {
                Ok(s) => home(ui, &page, &s),
                Err(e) => failed(ui, &page, "Home", &e),
            }
        }
        "money-transactions" => {
            let (offset, query) = (super::period_offset(), super::query());
            let mut payload = json!({"period_offset":offset});
            if !query.is_empty() {
                payload["query"] = json!(query);
            }
            let (lists, list) = tokio::join!(ui.call("money.lists", json!({})), ui.call("money.tx.list", payload));
            if *ui.page.borrow() != name || super::period_offset() != offset || super::query() != query {
                return;
            }
            if let Ok(lists) = &lists {
                remember_currency(lists);
            }
            match list {
                Ok(list) => transactions(ui, &page, &list, offset, &query),
                Err(e) => failed(ui, &page, "", &e),
            }
        }
        "money-plan" => {
            let (summary, lists) = tokio::join!(ui.call("money.summary", json!({})), ui.call("money.lists", json!({})));
            if *ui.page.borrow() != name {
                return;
            }
            match (summary, lists) {
                (Ok(s), Ok(l)) => plan(ui, &page, &s, &l),
                (Err(e), _) | (_, Err(e)) => failed(ui, &page, "Plan", &e),
            }
        }
        "money-data" => {
            let (summary, lists) = tokio::join!(ui.call("money.summary", json!({})), ui.call("money.lists", json!({})));
            if *ui.page.borrow() != name {
                return;
            }
            match &summary {
                Ok(_) => data(ui, &page, summary.as_ref().ok(), lists.as_ref().ok()),
                Err(e) => failed(ui, &page, "Data", e),
            }
        }
        _ => {}
    }
}

fn failed(ui: &Rc<Ui>, page: &Page, name: &str, error: &Error) {
    if !name.is_empty() {
        title(&page.head, name, "");
    }
    message(&page.body, &unavailable(error));
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

/// Home: the hero reading, In and Out, budgets furthest over pace, then recent entries, bills,
/// accounts and goals.
fn home(ui: &Rc<Ui>, page: &Page, s: &Value) {
    remember_currency(s);
    let fmt = formatter();
    let period = &s["period"];
    let days_left = n(period, "days_left");
    let resets = date(text(period, "end_exclusive"))
        .and_then(|d| d.format("%-d %b").ok())
        .map(|d| format!("resets {d}"))
        .unwrap_or_default();
    title(
        &page.head,
        &period_name(period),
        &[copy::plural(days_left, "day") + " left", resets].iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" · "),
    );
    clear(&page.body);
    if s["empty"] == true {
        empty_state(ui, &page.body);
        return;
    }
    let columns = gtk::Box::new(gtk::Orientation::Horizontal, 48);
    columns.set_homogeneous(true);
    let start = gtk::Box::new(gtk::Orientation::Vertical, 28);
    let end = gtk::Box::new(gtk::Orientation::Vertical, 28);
    columns.append(&start);
    columns.append(&end);
    page.body.append(&columns);

    // The hero: what is left, lit by the accent, or by the red when the period is over.
    let pace = &s["pace"];
    let status = text(pace, "status");
    let budgeted = status != "NO_BUDGET" && n(pace, "budget") > 0;
    let hero = gtk::Box::new(gtk::Orientation::Vertical, 6);
    hero.add_css_class("money-hero");
    if over(status) {
        hero.add_css_class("money-hero-over");
    } else if status == "OVER_PACE" {
        hero.add_css_class("money-hero-ahead");
    }
    let figure = if budgeted { fmt.format(n(pace, "remaining")) } else { fmt.format(n(s, "spent")) };
    let figure = label(&figure, "money-hero-figure");
    figure.set_widget_name("money-hero-figure");
    hero.append(&figure);
    hero.append(&label(
        &if budgeted { format!("LEFT OF {}", fmt.format_whole(n(pace, "budget"))) } else { String::from("SPENT THIS PERIOD") },
        "money-label",
    ));
    let lines = &s["lines"];
    for (key, class) in [("margin", "money-hero-line"), ("pace", "money-pace-line")] {
        let value = text(lines, key);
        if !value.is_empty() {
            let line = label(value, class);
            line.set_wrap(true);
            hero.append(&line);
        }
    }
    if budgeted {
        let bar = meter_in(pace["spent_fraction"].as_f64().unwrap_or(0.0), pace["pace_fraction"].as_f64(), tone(status, None), 10);
        bar.set_margin_top(10);
        bar.set_tooltip_text(Some("The tick is where an even spend would be today"));
        hero.append(&bar);
    } else {
        let set = button("set a budget →", "money-text-action");
        set.set_halign(gtk::Align::Start);
        let run = go(ui, "money-plan");
        set.connect_clicked(move |_| run());
        hero.append(&set);
    }
    start.append(&hero);

    // In and Out, then how this compares with the last period.
    let tiles = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    tiles.set_homogeneous(true);
    for (value, caption, class) in [(n(s, "income"), "IN", "money-in"), (n(s, "spent"), "OUT", "")] {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
        tile.add_css_class("money-tile");
        let figure = label(&fmt.format(value), "money-tile-figure");
        if !class.is_empty() && value > 0 {
            figure.add_css_class(class);
        }
        tile.append(&figure);
        tile.append(&label(caption, "money-label"));
        tiles.append(&tile);
    }
    start.append(&tiles);
    let versus = text(lines, "versus_last");
    if !versus.is_empty() {
        let line = label(versus, "money-muted");
        line.set_wrap(true);
        start.append(&line);
    }

    // Budgets, the furthest over pace first.
    let pace_fraction = pace["pace_fraction"].as_f64().unwrap_or(0.0);
    let mut budgets: Vec<Value> = rows(s, "budgets").into_iter().filter(|b| n(b, "budget") > 0).collect();
    let ahead = |b: &Value| n(b, "spent") as f64 / n(b, "budget").max(1) as f64 - pace_fraction;
    budgets.sort_by(|a, b| ahead(b).total_cmp(&ahead(a)));
    if !budgets.is_empty() {
        let list = section(&start, "Budgets", Some(("plan", go(ui, "money-plan"))));
        list.add_css_class("money-open");
        for b in budgets.iter().take(6) {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
            row.add_css_class("money-budget");
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            top.append(&badge(text(b, "icon"), n(b, "color")));
            let name = label(text(b, "name"), "money-row-title");
            name.set_hexpand(true);
            top.append(&name);
            let reading = label(&copy::of_budget(n(b, "spent"), n(b, "budget"), &fmt), "money-row-detail");
            if let Some(class) = tone_class(text(b, "status")) {
                reading.add_css_class(class);
            }
            top.append(&reading);
            row.append(&top);
            let fraction = n(b, "spent") as f64 / n(b, "budget").max(1) as f64;
            row.append(&meter_in(fraction, Some(pace_fraction), tone(text(b, "status"), b["color"].as_i64()), 6));
            list.append(&row);
        }
    }

    // Recent entries, bills, accounts and goals.
    let recent = rows(s, "recent");
    if !recent.is_empty() {
        let list = section(&end, "Recent", Some(("all", go(ui, "money-transactions"))));
        for tx in &recent {
            list.append(&tx_row(ui, tx, false));
        }
    }
    let bills = rows(s, "bills");
    if !bills.is_empty() {
        let list = section(&end, "Bills", None);
        list.add_css_class("money-open");
        for bill in &bills {
            let amount = n(bill, "amount");
            let (figure, class) = if text(bill, "type") == "INCOME" {
                (fmt.format_signed(amount), "money-in")
            } else {
                (fmt.format(amount), "")
            };
            let detail = format!("{} · {}", text(bill, "due_line"), human_date(text(bill, "next_date")));
            list.append(&line_row(text(bill, "name"), &detail, &figure, class));
        }
    }
    let accounts = rows(s, "accounts");
    if !accounts.is_empty() {
        let list = section(&end, "Accounts", None);
        list.add_css_class("money-open");
        for account in &accounts {
            let balance = n(account, "balance");
            list.append(&line_row(
                text(account, "name"),
                &word(text(account, "type")),
                &fmt.format(balance),
                if balance < 0 { "money-over" } else { "" },
            ));
        }
        let worth = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        worth.add_css_class("money-worth");
        let caption = label("NET WORTH", "money-label");
        caption.set_hexpand(true);
        caption.set_valign(gtk::Align::Center);
        worth.append(&caption);
        worth.append(&label(&fmt.format(n(s, "net_worth")), "money-worth-figure"));
        list.append(&worth);
    }
    let goals = rows(s, "goals");
    if !goals.is_empty() {
        let list = section(&end, "Goals", None);
        list.add_css_class("money-open");
        for goal in &goals {
            let row = line_row(text(goal, "name"), text(goal, "line"), "", "");
            let block = gtk::Box::new(gtk::Orientation::Vertical, 6);
            block.append(&row);
            if n(goal, "target") > 0 {
                block.append(&meter(n(goal, "saved") as f64 / n(goal, "target") as f64, None, false, 6));
            }
            list.append(&block);
        }
    }
    if recent.is_empty() && bills.is_empty() && accounts.is_empty() && goals.is_empty() {
        let quiet = label("Nothing logged this period yet.", "money-muted");
        end.append(&quiet);
    }
}

/// An empty ledger says so, and offers the two ways to fill it.
fn empty_state(ui: &Rc<Ui>, body: &gtk::Box) {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 14);
    block.add_css_class("money-empty");
    block.append(&label("Nothing here yet", "money-empty-title"));
    let about = label(
        "This ledger is empty. Bring over what Tally holds on your phone with its backup file, start fresh with an account to log against, or load a sample household to look around first.",
        "money-muted",
    );
    about.set_wrap(true);
    about.set_max_width_chars(60);
    block.append(&about);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let import_key = button("Import a Tally backup", "primary");
    import_key.add_css_class("money-hero-action");
    import_key.set_widget_name("money-empty-import");
    let account = button("Add an account", "money-secondary");
    account.set_widget_name("money-empty-account");
    let sample = button("Load sample data", "money-secondary");
    sample.set_widget_name("money-empty-sample");
    actions.append(&import_key);
    actions.append(&account);
    actions.append(&sample);
    let weak = Rc::downgrade(ui);
    account.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            super::add_account(&ui);
        }
    });
    block.append(&actions);
    body.append(&block);
    let weak = Rc::downgrade(ui);
    import_key.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            import(&ui, true);
        }
    });
    let weak = Rc::downgrade(ui);
    sample.connect_clicked(move |key| {
        if let Some(ui) = weak.upgrade() {
            load_sample(&ui, key);
        }
    });
}

/// Transactions: the period's entries grouped by day.
fn transactions(ui: &Rc<Ui>, page: &Page, list: &Value, offset: i64, query: &str) {
    let fmt = formatter();
    let entries = rows(list, "transactions");
    let name = period_name(&list["period"]);
    TX_HEAD.with(|h| {
        if let Some((context, period, next)) = h.borrow().as_ref() {
            let count = plural(entries.len(), "entry").replace("entrys", "entries");
            context.set_text(&format!(
                "{count} · {} in · {} out",
                fmt.format_whole(n(list, "income")),
                fmt.format_whole(n(list, "spent"))
            ));
            period.set_text(&name);
            next.set_sensitive(offset < 0);
        }
    });
    clear(&page.body);
    if entries.is_empty() {
        let text = if query.is_empty() {
            format!("Nothing logged in {name}.")
        } else {
            format!("Nothing in {name} matches \u{201c}{query}\u{201d}.")
        };
        message(&page.body, &text);
        return;
    }
    let mut day = String::new();
    let mut group: Option<gtk::Box> = None;
    for tx in &entries {
        let date = text(tx, "date");
        if date != day || group.is_none() {
            day = date.to_string();
            let block = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let header = label(&human_date(date), "money-day");
            block.append(&header);
            let rows = gtk::Box::new(gtk::Orientation::Vertical, 2);
            rows.add_css_class("money-group");
            block.append(&rows);
            page.body.append(&block);
            group = Some(rows);
        }
        if let Some(rows) = &group {
            rows.append(&tx_row(ui, tx, true));
        }
    }
}

/// Plan: the monthly budget and one per expense category, edited in place.
fn plan(ui: &Rc<Ui>, page: &Page, s: &Value, lists: &Value) {
    remember_currency(s);
    let fmt = formatter();
    // A read that lands while a budget is being typed keeps the typing.
    let typing = gtk::prelude::GtkWindowExt::focus(&ui.window)
        .and_then(|w| w.ancestor(gtk::Entry::static_type()))
        .and_downcast::<gtk::Entry>()
        .filter(|e| e.widget_name().starts_with("money-budget"))
        .map(|e| (e.widget_name().to_string(), e.text().to_string(), e.position()));
    title(&page.head, "Plan", &format!("Budgets for {}. An empty field means no budget.", period_name(&s["period"])));
    // Removing a focused field ends its focus: that is a rebuild, not the person leaving it.
    REBUILDING.with(|r| r.set(true));
    clear(&page.body);
    REBUILDING.with(|r| r.set(false));
    let pace = &s["pace"];
    let overall = section(&page.body, "Monthly budget", None);
    overall.add_css_class("money-open");
    let status = text(pace, "status");
    let budget = if status == "NO_BUDGET" { 0 } else { n(pace, "budget") };
    let detail = if budget > 0 { copy::of_budget(n(pace, "spent"), budget, &fmt) } else { String::from("No budget: Home shows what you spent") };
    overall.append(&budget_row(ui, None, "Everything", &detail, budget, over(status), None));
    let spent: std::collections::BTreeMap<i64, Value> = rows(s, "budgets").into_iter().map(|b| (n(&b, "category_id"), b)).collect();
    let list = section(&page.body, "By category", None);
    list.add_css_class("money-open");
    let categories: Vec<Value> = rows(lists, "categories")
        .into_iter()
        .filter(|c| text(c, "kind") == "EXPENSE" && c["archived"] != true)
        .collect();
    if categories.is_empty() {
        list.append(&label("No expense categories yet. Import a backup or load sample data.", "money-muted"));
    }
    for category in &categories {
        let id = n(category, "id");
        let reading = spent.get(&id);
        let amount = reading.map_or(0, |b| n(b, "budget"));
        let detail = match reading {
            Some(b) if amount > 0 => copy::of_budget(n(b, "spent"), amount, &fmt),
            Some(b) if n(b, "spent") > 0 => format!("{} spent, no budget", fmt.format_whole(n(b, "spent"))),
            _ => String::new(),
        };
        let badge = badge(text(category, "icon"), n(category, "color"));
        list.append(&budget_row(
            ui,
            Some(id),
            text(category, "name"),
            &detail,
            amount,
            reading.is_some_and(|b| over(text(b, "status"))),
            Some(badge),
        ));
    }
    if let Some((name, text, position)) = typing {
        if let Some(entry) = named(page.body.upcast_ref(), &name).and_downcast::<gtk::Entry>() {
            entry.set_text(&text);
            entry.grab_focus();
            entry.set_position(position);
        }
    }
}

thread_local! {
    static REBUILDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn named(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == name {
            return Some(widget);
        }
        if let Some(found) = named(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

/// A budget line: name, reading, and the amount field that saves on Enter or on leaving it.
fn budget_row(
    ui: &Rc<Ui>,
    category: Option<i64>,
    name: &str,
    detail: &str,
    amount: i64,
    is_over: bool,
    badge: Option<gtk::Box>,
) -> gtk::Box {
    let fmt = formatter();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("money-budget");
    if let Some(badge) = badge {
        row.append(&badge);
    }
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    words.set_valign(gtk::Align::Center);
    words.append(&label(name, "money-row-title"));
    if !detail.is_empty() {
        let d = label(detail, "money-row-detail");
        if is_over {
            d.add_css_class("money-over");
        }
        words.append(&d);
    }
    row.append(&words);
    let field = gtk::Entry::new();
    field.add_css_class("money-field");
    field.set_widget_name(&match category {
        Some(id) => format!("money-budget-{id}"),
        None => String::from("money-budget-overall"),
    });
    field.set_width_chars(10);
    gtk::prelude::EntryExt::set_alignment(&field, 1.0);
    field.set_valign(gtk::Align::Center);
    field.set_placeholder_text(Some("No budget"));
    if amount > 0 {
        field.set_text(&fmt.format_input(amount));
    }
    field.update_property(&[gtk::accessible::Property::Label(&format!("Budget for {name}"))]);
    let symbol = label(&fmt.symbol, "money-suffix");
    symbol.set_valign(gtk::Align::Center);
    row.append(&symbol);
    row.append(&field);
    let saved = Rc::new(std::cell::Cell::new(amount));
    let commit = {
        let weak = Rc::downgrade(ui);
        let saved = saved.clone();
        move |field: &gtk::Entry| {
            let Some(ui) = weak.upgrade() else { return };
            if REBUILDING.with(|r| r.get()) {
                return;
            }
            let typed = field.text();
            let value = if typed.trim().is_empty() { Some(0) } else { formatter().parse(&typed) };
            let Some(value) = value else {
                field.add_css_class("error");
                super::toast(&ui, "Type an amount, like 250 or 250.00. Leave it empty for no budget.", None);
                return;
            };
            field.remove_css_class("error");
            if value == saved.get() {
                return;
            }
            saved.set(value);
            let mut payload = json!({"amount":value});
            if let Some(id) = category {
                payload["category_id"] = json!(id);
            }
            glib::spawn_future_local(async move {
                if let Err(e) = ui.call("money.budget.set", payload).await {
                    ui.show_error(&e.to_string());
                }
                ui.refresh_page();
            });
        }
    };
    let on_enter = commit.clone();
    field.connect_activate(move |field| on_enter(field));
    let focus = gtk::EventControllerFocus::new();
    let weak = field.downgrade();
    focus.connect_leave(move |_| {
        if let Some(field) = weak.upgrade() {
            commit(&field);
        }
    });
    field.add_controller(focus);
    row
}

/// Data: import, export, sample data, erase.
fn data(ui: &Rc<Ui>, page: &Page, summary: Option<&Value>, lists: Option<&Value>) {
    title(&page.head, "Data", "Your ledger lives on this PC, in money.db beside Relay's own store. Nothing here leaves it.");
    clear(&page.body);
    let empty = summary.is_none_or(|s| s["empty"] == true);
    if let Some(lists) = lists {
        phones(&page.body, lists);
    }
    let list = section(&page.body, "Your ledger", None);
    list.add_css_class("money-open");
    let action = |title: &str, about: &str, caption: &str, name: &str, class: &str| {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        row.add_css_class("money-data-row");
        let words = gtk::Box::new(gtk::Orientation::Vertical, 3);
        words.set_hexpand(true);
        words.append(&label(title, "money-row-title"));
        let d = label(about, "money-row-detail");
        d.set_wrap(true);
        d.set_max_width_chars(70);
        words.append(&d);
        row.append(&words);
        let key = button(caption, class);
        key.set_widget_name(name);
        key.set_valign(gtk::Align::Center);
        row.append(&key);
        list.append(&row);
        key
    };
    let import_key = action(
        "Import a Tally backup",
        "The .json file Tally writes under Settings, Backup. It replaces everything on this PC, as a restore does on the phone.",
        "Choose a backup…",
        "money-import",
        "money-secondary",
    );
    let export_key = action(
        "Export a backup",
        "The same file Tally reads, to keep somewhere safe or to restore on the phone.",
        "Export…",
        "money-export",
        "money-secondary",
    );
    let account = action(
        "Add an account",
        "Cash, chequing, savings, credit or investment, with what it holds today. The first one also brings Tally's usual categories.",
        "Add an account…",
        "money-account",
        "money-secondary",
    );
    let weak = Rc::downgrade(ui);
    account.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            super::add_account(&ui);
        }
    });
    let sample = action(
        "Load sample data",
        "A sample household to look around with. Only goes into an empty ledger.",
        "Load sample",
        "money-sample",
        "money-secondary",
    );
    sample.set_sensitive(empty);
    if !empty {
        sample.set_tooltip_text(Some("Only into an empty ledger: erase everything first"));
    }
    export_key.set_sensitive(!empty);
    let erase = action(
        "Erase everything",
        "Every entry, account, budget, bill and goal on this PC. Tally on the phone keeps its own.",
        "Erase everything",
        "money-reset",
        "money-destructive",
    );
    erase.set_sensitive(!empty);
    let weak = Rc::downgrade(ui);
    import_key.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            import(&ui, empty);
        }
    });
    let weak = Rc::downgrade(ui);
    export_key.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            export(&ui);
        }
    });
    let weak = Rc::downgrade(ui);
    sample.connect_clicked(move |key| {
        if let Some(ui) = weak.upgrade() {
            load_sample(&ui, key);
        }
    });
    if let Some(summary) = summary {
        ledger_settings(ui, &page.body, summary);
    }
    let weak = Rc::downgrade(ui);
    confirm_inline(&erase, "Erase all of it", move |key| {
        let Some(ui) = weak.upgrade() else { return };
        key.set_sensitive(false);
        glib::spawn_future_local(async move {
            match ui.call("money.reset", json!({})).await {
                Ok(_) => super::toast(&ui, "Erased. The ledger on this PC is empty.", None),
                Err(e) => ui.show_error(&e.to_string()),
            }
            ui.refresh_page();
        });
    });
}

/// The phones that sync with this ledger (`money.lists` `devices`, latest first).
fn phones(body: &gtk::Box, lists: &Value) {
    let list = section(body, "Your phone", None);
    list.add_css_class("money-open");
    let devices = rows(lists, "devices");
    if devices.is_empty() {
        list.append(&line_row("No phone syncs with this PC yet", "", "", ""));
    }
    for device in &devices {
        let when = crate::relative::ago(text(device, "last_sync"), crate::relative::Form::Long);
        let name = match text(device, "name") {
            "" => "A phone",
            name => name,
        };
        let line = match when {
            Some(when) => format!("{name} synced {when}"),
            None => format!("{name} has not synced yet"),
        };
        list.append(&line_row(&line, "", "", ""));
    }
    let hint = label(
        "Pair a phone from Tally's Settings, with the pairing link that \u{201c}relay remote pair\u{201d} prints on this PC. Both then keep the whole ledger and catch each other up.",
        "money-row-detail",
    );
    hint.set_wrap(true);
    hint.set_max_width_chars(80);
    list.append(&hint);
}

/// The ledger's currency and the day its budget period starts (`money.settings.set`).
fn ledger_settings(ui: &Rc<Ui>, body: &gtk::Box, summary: &Value) {
    let list = section(body, "Money settings", None);
    list.add_css_class("money-open");
    let row = |title: &str, about: &str, control: &gtk::Widget| {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        row.add_css_class("money-data-row");
        let words = gtk::Box::new(gtk::Orientation::Vertical, 3);
        words.set_hexpand(true);
        words.append(&label(title, "money-row-title"));
        let d = label(about, "money-row-detail");
        d.set_wrap(true);
        d.set_max_width_chars(70);
        words.append(&d);
        row.append(&words);
        control.set_valign(gtk::Align::Center);
        row.append(control);
        list.append(&row);
    };
    let currency = gtk::Entry::new();
    currency.add_css_class("money-field");
    currency.set_widget_name("money-currency");
    currency.set_width_chars(6);
    currency.set_max_length(3);
    currency.set_text(text(summary, "currency"));
    currency.update_property(&[gtk::accessible::Property::Label("Currency code")]);
    row(
        "Currency",
        "The ISO code amounts are kept in, such as CAD, USD or EUR. Changing it relabels amounts; it converts nothing.",
        currency.upcast_ref(),
    );
    let start = gtk::SpinButton::with_range(1.0, 28.0, 1.0);
    start.add_css_class("money-field");
    start.set_widget_name("money-month-start");
    start.set_value(summary["month_start_day"].as_f64().unwrap_or(1.0));
    start.update_property(&[gtk::accessible::Property::Label("Month start day")]);
    row(
        "Month starts on day",
        "For a budget that follows your pay: 15 runs each period from the 15th to the 14th.",
        start.upcast_ref(),
    );
    let save = button("Save settings", "money-secondary");
    save.set_widget_name("money-settings-save");
    save.set_halign(gtk::Align::End);
    list.append(&save);
    let weak = Rc::downgrade(ui);
    save.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else { return };
        let code = currency.text().trim().to_uppercase();
        if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
            super::toast(&ui, "A currency is three letters, such as CAD.", None);
            return;
        }
        let payload = json!({"currency":code,"month_start_day":start.value_as_int()});
        key.set_sensitive(false);
        let key = key.clone();
        glib::spawn_future_local(async move {
            match ui.call("money.settings.set", payload).await {
                Ok(v) => {
                    super::remember_currency(&v);
                    super::toast(&ui, "Saved. Every amount and period reads with the new settings.", None);
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            key.set_sensitive(true);
            ui.refresh_page();
        });
    });
}

fn load_sample(ui: &Rc<Ui>, key: &gtk::Button) {
    key.set_sensitive(false);
    let ui = ui.clone();
    let key = key.clone();
    glib::spawn_future_local(async move {
        match ui.call("money.sample", json!({})).await {
            Ok(v) => super::toast(&ui, &format!("Loaded a sample household: {}.", plural(n(&v, "transactions").max(0) as usize, "entry").replace("entrys", "entries")), None),
            Err(e) => ui.show_error(&e.to_string()),
        }
        key.set_sensitive(true);
        ui.refresh_page();
    });
}

/// Choose a Tally backup and restore it; over a ledger with anything in it, only after a yes.
fn import(ui: &Rc<Ui>, empty: bool) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Tally backups (.json)"));
        filter.add_pattern("*.json");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title("Import a Tally backup").filters(&filters).default_filter(&filter).build();
        let Ok(file) = dialog.open_future(Some(&ui.window)).await else { return };
        let Some(path) = file.path() else {
            ui.show_error("That file is not on this PC's disk. Copy it here first.");
            return;
        };
        if !empty {
            let panel = crate::panel::Panel::new(&ui, "Replace everything?", 440);
            panel.add_css_class("money-sheet");
            let about = label(
                &format!(
                    "{} replaces every entry, account, budget, bill and goal on this PC. Export a backup first if you might want them back.",
                    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                ),
                "money-muted",
            );
            about.set_wrap(true);
            panel.body.append(&about);
            if !panel.response("Replace everything").await {
                return;
            }
        }
        match ui.call("money.import", json!({"path":path.to_string_lossy()})).await {
            Ok(v) => super::toast(
                &ui,
                &format!(
                    "Imported {} and {}.",
                    plural(n(&v, "transactions").max(0) as usize, "entry").replace("entrys", "entries"),
                    plural(n(&v, "accounts").max(0) as usize, "account")
                ),
                None,
            ),
            Err(e) => ui.show_error(&e.to_string()),
        }
        ui.refresh_page();
    });
}

fn export(ui: &Rc<Ui>) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let name = format!("tally-backup-{}.json", today());
        let dialog = gtk::FileDialog::builder().title("Export a backup").initial_name(name.as_str()).build();
        let Ok(file) = dialog.save_future(Some(&ui.window)).await else { return };
        let Some(path) = file.path() else { return };
        match ui.call("money.export", json!({"path":path.to_string_lossy()})).await {
            Ok(v) => super::toast(
                &ui,
                &format!(
                    "Saved {} to {}.",
                    plural(n(&v, "transactions").max(0) as usize, "entry").replace("entrys", "entries"),
                    text(&v, "path")
                ),
                None,
            ),
            Err(e) => ui.show_error(&e.to_string()),
        }
    });
}
