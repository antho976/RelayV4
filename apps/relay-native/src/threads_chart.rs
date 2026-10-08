//! Charts in a thread: what an agent's ```` ```chart ```` block draws (docs/THREADS.md).
//!
//! A chart keeps its question (a `money.series` payload), never its numbers: it reads them when it
//! is drawn and again whenever the ledger moves (`refresh_all`, on `money.changed`). Bars and lines
//! are drawn with cairo; their axis and labels are plain labels around the drawing, in Dev's type.
//! Colour is the data's: the newest period in Relay's signal, the ones before it in a fixed
//! palette, and categories in their own hue.
use super::pages::HUES;
use crate::app::{clear, label};
use gtk::prelude::*;
use gtk4 as gtk;
use relay_client::thread_view::{ticks, ChartKind, ChartSpec};
use relay_money::money::{Locale, MoneyFormatter};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::f64::consts::{FRAC_PI_2, PI};

/// Series colours, the newest first: Relay's lime, then blue, orange, violet, teal, gold.
const PALETTE: [&str; 6] = ["#C6F24E", "#6A9FD8", "#E08A5F", "#C27BC0", "#4FA9A0", "#D9A441"];
const PLOT_H: i32 = 184;
const PAD_T: f64 = 10.0;
const PAD_B: f64 = 8.0;
/// Room on the left for the axis labels.
const AXIS_W: i32 = 56;

/// A chart drawn from a question: its body, its legend and the question.
type Live = (glib::WeakRef<gtk::Box>, glib::WeakRef<gtk::Box>, ChartSpec);

thread_local! {
    /// Charts drawn from a question, to read again when the ledger moves.
    static LIVE: RefCell<Vec<Live>> = const { RefCell::new(Vec::new()) };
}

fn rgb(hex: &str) -> (f64, f64, f64) {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xEDE9E2);
    (f64::from((v >> 16) & 0xFF) / 255.0, f64::from((v >> 8) & 0xFF) / 255.0, f64::from(v & 0xFF) / 255.0)
}

fn series_color(index: usize, count: usize) -> &'static str {
    PALETTE[(count - 1 - index.min(count - 1)) % PALETTE.len()]
}

fn hue(color: Option<i64>, index: usize) -> &'static str {
    match color {
        Some(c) => HUES[c.rem_euclid(HUES.len() as i64) as usize],
        None => PALETTE[index % PALETTE.len()],
    }
}

fn dot(hex: &str) -> gtk::DrawingArea {
    let (r, g, b) = rgb(hex);
    let area = gtk::DrawingArea::new();
    area.set_content_width(9);
    area.set_content_height(9);
    area.set_valign(gtk::Align::Center);
    area.set_draw_func(move |_, cr, w, h| {
        cr.set_source_rgb(r, g, b);
        cr.arc(f64::from(w) / 2.0, f64::from(h) / 2.0, 4.0, 0.0, 2.0 * PI);
        let _ = cr.fill();
    });
    area
}

/// A chart card for `spec`, reading its numbers now.
pub fn card(spec: ChartSpec) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("threads-chart");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let title = label(if spec.title.is_empty() { "Chart" } else { &spec.title }, "threads-chart-title");
    title.set_wrap(true);
    head.append(&title);
    let legend = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    legend.set_hexpand(true);
    legend.set_halign(gtk::Align::End);
    head.append(&legend);
    card.append(&head);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&label("Reading the ledger…", "threads-chart-note"));
    card.append(&body);
    let foot = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    foot.add_css_class("threads-chart-foot");
    if spec.query.is_some() {
        let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        lamp.add_css_class("threads-dot");
        lamp.set_valign(gtk::Align::Center);
        foot.append(&lamp);
        foot.append(&label("Live from Tally, redraws when the ledger changes", "threads-chart-note"));
        LIVE.with(|l| l.borrow_mut().push((body.downgrade(), legend.downgrade(), spec.clone())));
    } else {
        foot.append(&label("Numbers as written in the reply", "threads-chart-note"));
    }
    card.append(&foot);
    load(&body, &legend, &spec);
    card
}

/// Read every live chart still on screen again.
pub fn refresh_all() {
    let live: Vec<_> = LIVE.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(body, _, _)| body.upgrade().is_some());
        l.iter().filter_map(|(b, g, s)| Some((b.upgrade()?, g.upgrade()?, s.clone()))).collect()
    });
    for (body, legend, spec) in live {
        load(&body, &legend, &spec);
    }
}

fn load(body: &gtk::Box, legend: &gtk::Box, spec: &ChartSpec) {
    if let Some(data) = &spec.data {
        let mut data = data.clone();
        if data["currency"].is_null() {
            data["currency"] = json!(super::formatter().currency);
        }
        draw(body, legend, spec.kind, &data);
        return;
    }
    let Some(query) = spec.query.clone() else { return };
    let Some(ui) = super::threads::the_ui_pub() else { return };
    let (body, legend, kind) = (body.downgrade(), legend.downgrade(), spec.kind);
    glib::spawn_future_local(async move {
        let read = ui.call("money.series", query).await;
        let (Some(body), Some(legend)) = (body.upgrade(), legend.upgrade()) else { return };
        match read {
            Ok(series) => draw(&body, &legend, kind, &series),
            Err(e) => {
                clear(&body);
                let note = label(&format!("This chart could not be read: {e}"), "threads-error");
                note.set_wrap(true);
                body.append(&note);
            }
        }
    });
}

struct Data {
    labels: Vec<String>,
    colors: Vec<Option<i64>>,
    series: Vec<(String, Vec<i64>)>,
    /// How many labels each series has reached (the period still running), else all of them.
    known: Vec<usize>,
    fmt: MoneyFormatter,
}

fn read(v: &Value) -> Data {
    let labels: Vec<String> = v["labels"].as_array().into_iter().flatten().map(|l| l.as_str().unwrap_or("").to_string()).collect();
    let colors = (0..labels.len()).map(|i| v["label_colors"][i].as_i64()).collect();
    let series = v["series"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            let values = (0..labels.len()).map(|i| s["values"][i].as_i64().unwrap_or(0)).collect();
            (s["name"].as_str().unwrap_or("").to_string(), values)
        })
        .collect();
    let known = v["series"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| s["known"].as_u64().map_or(labels.len(), |k| (k as usize).min(labels.len())))
        .collect();
    let currency = v["currency"].as_str().filter(|c| !c.is_empty()).unwrap_or("CAD");
    Data { labels, colors, series, known, fmt: MoneyFormatter::new(currency, Locale::from_env()) }
}

fn draw(body: &gtk::Box, legend: &gtk::Box, kind: ChartKind, v: &Value) {
    clear(body);
    clear(legend);
    let data = read(v);
    if data.labels.is_empty() || data.series.iter().all(|(_, values)| values.iter().all(|x| *x == 0)) {
        body.append(&label("Nothing to draw yet: no entries match.", "threads-chart-note"));
        return;
    }
    match kind {
        ChartKind::Donut => donut(body, &data),
        _ => {
            let count = data.series.len();
            if count > 1 || kind == ChartKind::Line {
                for (i, (name, _)) in data.series.iter().enumerate() {
                    let item = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    item.append(&dot(series_color(i, count)));
                    item.append(&label(name, "threads-chart-legend"));
                    legend.append(&item);
                }
            }
            plot(body, &data, kind);
        }
    }
}

/// Bars or lines over a value axis, labels under each group.
fn plot(body: &gtk::Box, data: &Data, kind: ChartKind) {
    let all = data.series.iter().flat_map(|(_, v)| v.iter().copied());
    let (lo, hi) = all.fold((0_i64, 0_i64), |(lo, hi), x| (lo.min(x), hi.max(x)));
    let digits = data.fmt.fraction_digits;
    let (step, up) = ticks(hi.max(-lo), digits);
    let down = if lo < 0 { ((-lo + step - 1) / step) as u32 } else { 0 };
    let (top, bottom) = (step * i64::from(up), -step * i64::from(down));
    let span = (top - bottom).max(1) as f64;
    let y_of = move |value: i64, h: f64| PAD_T + (top - value) as f64 / span * (h - PAD_T - PAD_B);

    let overlay = gtk::Overlay::new();
    let area = gtk::DrawingArea::new();
    area.set_content_height(PLOT_H);
    area.set_hexpand(true);
    overlay.set_child(Some(&area));
    for k in -(down as i64)..=i64::from(up) {
        let value = k * step;
        let tick = label(&data.fmt.format_whole(value), "threads-chart-axis");
        tick.set_halign(gtk::Align::Start);
        tick.set_valign(gtk::Align::Start);
        tick.set_margin_top((y_of(value, f64::from(PLOT_H)) - 7.0).max(0.0) as i32);
        tick.set_can_target(false);
        overlay.add_overlay(&tick);
    }
    body.append(&overlay);

    let groups = data.labels.len();
    let count = data.series.len();
    let series = data.series.clone();
    let known = data.known.clone();
    area.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        let left = f64::from(AXIS_W);
        let plot_w = (w - left - 4.0).max(10.0);
        let gw = plot_w / groups as f64;
        // Grid: one line per tick, the zero line a step brighter.
        for k in -(down as i64)..=i64::from(up) {
            let y = y_of(k * step, h).round() + 0.5;
            let (r, g, b) = rgb(if k == 0 { "#33302D" } else { "#242220" });
            cr.set_source_rgb(r, g, b);
            cr.set_line_width(1.0);
            cr.move_to(left, y);
            cr.line_to(w - 4.0, y);
            let _ = cr.stroke();
        }
        let zero = y_of(0, h);
        match kind {
            ChartKind::Line => {
                for (i, (_, values)) in series.iter().enumerate() {
                    // A period still running stops at today: the days after it have not happened.
                    let values = &values[..known[i].max(1).min(values.len())];
                    let (r, g, b) = rgb(series_color(i, count));
                    cr.set_source_rgb(r, g, b);
                    cr.set_line_width(2.5);
                    cr.set_line_join(gtk::cairo::LineJoin::Round);
                    for (j, v) in values.iter().enumerate() {
                        let (x, y) = (left + (j as f64 + 0.5) * gw, y_of(*v, h));
                        if j == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
                    }
                    let _ = cr.stroke();
                    if groups <= 16 {
                        for (j, v) in values.iter().enumerate() {
                            cr.arc(left + (j as f64 + 0.5) * gw, y_of(*v, h), 3.0, 0.0, 2.0 * PI);
                            let _ = cr.fill();
                        }
                    }
                }
            }
            _ => {
                let bw = (gw * 0.72 / count as f64 - 3.0).clamp(3.0, 26.0);
                let inner = bw * count as f64 + 3.0 * (count as f64 - 1.0);
                for j in 0..groups {
                    let x0 = left + j as f64 * gw + (gw - inner) / 2.0;
                    for (i, (_, values)) in series.iter().enumerate() {
                        let v = values[j];
                        if v == 0 {
                            continue;
                        }
                        let (r, g, b) = rgb(series_color(i, count));
                        cr.set_source_rgb(r, g, b);
                        let x = x0 + i as f64 * (bw + 3.0);
                        let (y1, y2) = (y_of(v, h).min(zero), y_of(v, h).max(zero));
                        let rad = (bw / 2.0).min(3.0).min(y2 - y1);
                        // Rounded at the end away from zero.
                        if v > 0 {
                            cr.move_to(x, y2);
                            cr.line_to(x, y1 + rad);
                            cr.arc(x + rad, y1 + rad, rad, PI, 1.5 * PI);
                            cr.arc(x + bw - rad, y1 + rad, rad, 1.5 * PI, 0.0);
                            cr.line_to(x + bw, y2);
                        } else {
                            cr.move_to(x, y1);
                            cr.line_to(x, y2 - rad);
                            cr.arc_negative(x + rad, y2 - rad, rad, PI, FRAC_PI_2);
                            cr.arc_negative(x + bw - rad, y2 - rad, rad, FRAC_PI_2, 0.0);
                            cr.line_to(x + bw, y1);
                        }
                        cr.close_path();
                        let _ = cr.fill();
                    }
                }
            }
        }
    });
    // Hovering a group says its numbers.
    area.set_has_tooltip(true);
    let (labels, series, fmt) = (data.labels.clone(), data.series.clone(), data.fmt.clone());
    area.connect_query_tooltip(move |area, x, _, _, tooltip| {
        let left = f64::from(AXIS_W);
        let gw = (f64::from(area.width()) - left - 4.0).max(10.0) / labels.len() as f64;
        let j = ((f64::from(x) - left) / gw).floor();
        if j < 0.0 || j as usize >= labels.len() {
            return false;
        }
        let j = j as usize;
        let mut lines = vec![labels[j].clone()];
        for (name, values) in series.iter().rev() {
            let name = if name.is_empty() { String::new() } else { format!("{name}  ") };
            lines.push(format!("{name}{}", fmt.format(values[j])));
        }
        tooltip.set_text(Some(&lines.join("\n")));
        true
    });

    // The group labels, thinned to about ten.
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_homogeneous(true);
    row.set_margin_start(AXIS_W);
    row.set_margin_end(4);
    let every = groups.div_ceil(10).max(1);
    for (j, name) in data.labels.iter().enumerate() {
        let shown = if j % every == 0 { name.as_str() } else { "" };
        let l = label(shown, "threads-chart-x");
        l.set_xalign(0.5);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_tooltip_text(Some(name));
        row.append(&l);
    }
    body.append(&row);
}

/// Where the money went: a ring of the first series, and a list beside it.
fn donut(body: &gtk::Box, data: &Data) {
    let (_, values) = &data.series[data.series.len() - 1];
    let parts: Vec<(String, i64, &'static str)> = data
        .labels
        .iter()
        .zip(values)
        .enumerate()
        .filter(|(_, (_, v))| **v > 0)
        .map(|(i, (l, v))| (l.clone(), *v, hue(data.colors[i], i)))
        .collect();
    let total: i64 = parts.iter().map(|(_, v, _)| v).sum();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    let overlay = gtk::Overlay::new();
    let ring = gtk::DrawingArea::new();
    ring.set_content_width(156);
    ring.set_content_height(156);
    ring.set_valign(gtk::Align::Center);
    let slices: Vec<(f64, &'static str)> = parts.iter().map(|(_, v, c)| (*v as f64 / total.max(1) as f64, *c)).collect();
    ring.set_draw_func(move |_, cr, w, h| {
        let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
        let radius = cx.min(cy) - 12.0;
        cr.set_line_width(22.0);
        let mut at = -FRAC_PI_2;
        for (fraction, color) in &slices {
            let sweep = fraction * 2.0 * PI;
            let (r, g, b) = rgb(color);
            cr.set_source_rgb(r, g, b);
            // A hairline gap between slices.
            let gap = if slices.len() > 1 { 0.012 } else { 0.0 };
            cr.arc(cx, cy, radius, at + gap, at + sweep - gap);
            let _ = cr.stroke();
            at += sweep;
        }
    });
    overlay.set_child(Some(&ring));
    let centre = gtk::Box::new(gtk::Orientation::Vertical, 0);
    centre.set_halign(gtk::Align::Center);
    centre.set_valign(gtk::Align::Center);
    let sum = label(&data.fmt.format_whole(total), "threads-chart-total");
    sum.set_xalign(0.5);
    centre.append(&sum);
    let caption = label(&data.series[data.series.len() - 1].0, "threads-chart-note");
    caption.set_xalign(0.5);
    centre.append(&caption);
    overlay.add_overlay(&centre);
    row.append(&overlay);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    list.set_valign(gtk::Align::Center);
    list.set_hexpand(true);
    for (name, value, color) in &parts {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        item.append(&dot(color));
        let n = label(name, "threads-chart-legend");
        n.set_hexpand(true);
        item.append(&n);
        item.append(&label(&data.fmt.format_whole(*value), "threads-chart-value"));
        let share = label(&format!("{:.0}%", *value as f64 * 100.0 / total.max(1) as f64), "threads-chart-share");
        share.set_width_chars(4);
        share.set_xalign(1.0);
        item.append(&share);
        list.append(&item);
    }
    row.append(&list);
    body.append(&row);
}
