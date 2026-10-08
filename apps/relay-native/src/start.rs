//! The start screen's look: the Relay lockup, a greeting, and one card per space, opened with
//! one short animation. What a card does and what its live line says is `money.rs`'s.
//!
//! The mark follows the brand sheet on a 120 grid: a ring of r36, stroke 12 with round caps, cut
//! into an outgoing upper arc (190° to 330°, ink) and a receiving lower arc (10° to 150°,
//! dimmed), with the signal, r8 at (102, 54), in the gap between them. The wordmark is Sora 600.
//!
//! The animation runs once when the screen opens, about 1.2 s, all from one frame-clock tick:
//! the two arcs draw in, the signal lands in the gap, then the wordmark, the greeting and the
//! cards fade up. With animations off (GTK's setting) it opens at its last frame.
//! `RELAY_NATIVE_START_T=<0..1>` holds it at one point of the timeline, for screenshots.
use crate::app::{button, label};
use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

/// The whole opening, in microseconds of frame-clock time.
const DURATION_US: f64 = 1_200_000.0;

const INK: (f64, f64, f64) = (0xED as f64 / 255.0, 0xEE as f64 / 255.0, 0xF0 as f64 / 255.0);
const DIM: (f64, f64, f64) = (0x63 as f64 / 255.0, 0x66 as f64 / 255.0, 0x6B as f64 / 255.0);
const SIGNAL: (f64, f64, f64) = (0xC6 as f64 / 255.0, 0xF2 as f64 / 255.0, 0x4E as f64 / 255.0);

/// Where each part's motion sits on the 0..1 timeline: (from, to).
const ARC_A: (f64, f64) = (0.0, 0.42);
const ARC_B: (f64, f64) = (0.10, 0.52);
const SIGNAL_IN: (f64, f64) = (0.40, 0.64);
const WORDMARK: (f64, f64) = (0.44, 0.75);
const GREETING: (f64, f64) = (0.58, 0.88);
const CARDS: (f64, f64) = (0.68, 1.0);
const HINT: (f64, f64) = (0.80, 1.0);

/// What `build` hands back: the screen, each card's live line, and the cards (Dev, Money).
pub struct Built {
    pub screen: gtk::Box,
    pub lines: [gtk::Label; 2],
    pub keys: [gtk::Button; 2],
}

/// `t` (0..1) within `span`, eased out: fast start, soft landing.
fn phase(t: f64, (from, to): (f64, f64)) -> f64 {
    let x = ((t - from) / (to - from)).clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// Like `phase`, with a small overshoot: the signal arrives and settles.
fn phase_back(t: f64, (from, to): (f64, f64)) -> f64 {
    let x = ((t - from) / (to - from)).clamp(0.0, 1.0);
    let c = 1.70158;
    1.0 + (c + 1.0) * (x - 1.0).powi(3) + c * (x - 1.0).powi(2)
}

/// The mark at timeline point `t`, in a `size` square. The title bar draws it settled (`t` = 1).
pub(crate) fn draw_mark(cr: &gtk::cairo::Context, size: f64, t: f64) {
    let s = size / 120.0;
    cr.scale(s, s);
    cr.set_line_width(12.0);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    let arc = |cr: &gtk::cairo::Context, from: f64, sweep: f64, (r, g, b): (f64, f64, f64)| {
        if sweep <= 0.5 {
            return;
        }
        cr.set_source_rgb(r, g, b);
        cr.new_sub_path();
        cr.arc(60.0, 60.0, 36.0, from * PI / 180.0, (from + sweep) * PI / 180.0);
        let _ = cr.stroke();
    };
    // Outgoing leg first, then the receiving leg, each drawn along its own direction.
    arc(cr, 190.0, 140.0 * phase(t, ARC_A), INK);
    arc(cr, 10.0, 140.0 * phase(t, ARC_B), DIM);
    let r = 8.0 * phase_back(t, SIGNAL_IN);
    if r > 0.1 {
        let (red, green, blue) = SIGNAL;
        cr.set_source_rgb(red, green, blue);
        cr.arc(102.0, 54.0, r, 0.0, 2.0 * PI);
        let _ = cr.fill();
    }
}

/// A widget's place on the timeline: it fades in over `span` and rises `rise` pixels into place.
/// The rise moves margin from above it to below it, so nothing around it shifts while it runs.
fn reveal(widget: &impl IsA<gtk::Widget>, t: f64, span: (f64, f64), rise: i32) {
    let p = phase(t, span);
    let above = ((1.0 - p) * f64::from(rise)).round() as i32;
    widget.set_opacity(p);
    widget.set_margin_top(above);
    widget.set_margin_bottom(rise - above);
}

/// Where the opening stands: held by `RELAY_NATIVE_START_T`, finished when animations are off,
/// else `None` (run it).
fn fixed_point() -> Option<f64> {
    if let Some(t) = std::env::var("RELAY_NATIVE_START_T").ok().and_then(|v| v.parse::<f64>().ok()) {
        return Some(t.clamp(0.0, 1.0));
    }
    let animate = gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations());
    (!animate).then_some(1.0)
}

/// The screen for `greeting`, with `hint` under the cards and the Dev and Money cards titled,
/// described and keyed as given. `pick(true)` is Money.
pub fn build(greeting: &str, hint: &str, pick: Rc<dyn Fn(bool)>) -> Built {
    let screen = gtk::Box::new(gtk::Orientation::Vertical, 0);
    screen.add_css_class("start-screen");
    screen.set_widget_name("start-screen");
    screen.set_focusable(true);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::Center);
    column.set_vexpand(true);

    let lockup = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    lockup.add_css_class("start-lockup");
    lockup.set_halign(gtk::Align::Center);
    // The wordmark ends its slide with 12px after it; the same before keeps the lockup centred.
    lockup.set_margin_start(12);
    let t = Rc::new(Cell::new(fixed_point().unwrap_or(0.0)));
    let mark = gtk::DrawingArea::new();
    // On the brand's lockup the ring stands a little taller than the wordmark's ascenders.
    mark.set_content_width(84);
    mark.set_content_height(84);
    mark.set_valign(gtk::Align::Center);
    mark.set_accessible_role(gtk::AccessibleRole::Img);
    mark.update_property(&[gtk::accessible::Property::Label("Relay")]);
    let at = t.clone();
    mark.set_draw_func(move |_, cr, w, h| draw_mark(cr, f64::from(w.min(h)), at.get()));
    lockup.append(&mark);
    let wordmark = label("relay", "start-wordmark");
    wordmark.set_valign(gtk::Align::Center);
    lockup.append(&wordmark);
    column.append(&lockup);

    let words = gtk::Box::new(gtk::Orientation::Vertical, 6);
    words.add_css_class("start-words");
    let hello = label(greeting, "start-greeting");
    hello.set_xalign(0.5);
    hello.set_wrap(true);
    words.append(&hello);
    let ask = label("What are we working on today?", "start-question");
    ask.set_xalign(0.5);
    words.append(&ask);
    column.append(&words);

    let cards = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    cards.add_css_class("start-cards");
    cards.set_halign(gtk::Align::Center);
    cards.set_homogeneous(true);
    let mut lines = Vec::new();
    let mut keys = Vec::new();
    for (index, (to_money, title, about)) in
        [(false, "Dev", "Agents, tasks and code"), (true, "Threads", "Ask about your data, with Tally")].into_iter().enumerate()
    {
        let card = button("", "start-card");
        card.set_widget_name(if to_money { "start-money" } else { "start-dev" });
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let eyebrow = label(&format!("0{} · {}", index + 1, title.to_lowercase()), "start-eyebrow");
        eyebrow.set_hexpand(true);
        eyebrow.set_xalign(0.0);
        top.append(&eyebrow);
        // The brand's signal: lit on the card under the pointer or the keyboard.
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("start-dot");
        dot.set_valign(gtk::Align::Center);
        top.append(&dot);
        inner.append(&top);
        let name = label(title, "start-title");
        name.set_xalign(0.0);
        inner.append(&name);
        let what = label(about, "start-about");
        what.set_xalign(0.0);
        inner.append(&what);
        let line = label("Reading…", "start-line");
        line.set_xalign(0.0);
        line.set_wrap(true);
        line.set_max_width_chars(34);
        inner.append(&line);
        card.set_child(Some(&inner));
        let pick = pick.clone();
        card.connect_clicked(move |_| pick(to_money));
        cards.append(&card);
        lines.push(line);
        keys.push(card);
    }
    column.append(&cards);
    let hint = label(hint, "start-hint");
    hint.set_xalign(0.5);
    column.append(&hint);
    screen.append(&column);

    let apply = {
        let (mark, wordmark, words, cards, hint) = (mark.clone(), wordmark.clone(), words.clone(), cards.clone(), hint.clone());
        move |t: f64| {
            mark.queue_draw();
            let w = phase(t, WORDMARK);
            let before = ((1.0 - w) * 12.0).round() as i32;
            wordmark.set_opacity(w);
            wordmark.set_margin_start(before);
            wordmark.set_margin_end(12 - before);
            reveal(&words, t, GREETING, 10);
            reveal(&cards, t, CARDS, 14);
            reveal(&hint, t, HINT, 0);
        }
    };
    apply(t.get());
    if fixed_point().is_none() {
        let start = Cell::new(None::<i64>);
        screen.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let began = start.get().unwrap_or(now);
            start.set(Some(began));
            let p = ((now - began) as f64 / DURATION_US).clamp(0.0, 1.0);
            t.set(p);
            apply(p);
            if p >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
        });
    }
    let [dev, money]: [gtk::Label; 2] = lines.try_into().expect("two cards");
    let [dev_key, money_key]: [gtk::Button; 2] = keys.try_into().expect("two cards");
    Built { screen, lines: [dev, money], keys: [dev_key, money_key] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_starts_hidden_and_ends_in_place() {
        for span in [ARC_A, ARC_B, WORDMARK, GREETING, CARDS, HINT] {
            assert_eq!(phase(0.0, span), 0.0);
            assert!((phase(1.0, span) - 1.0).abs() < 1e-9);
        }
        assert!(phase_back(0.0, SIGNAL_IN).abs() < 1e-9);
        assert!((phase_back(1.0, SIGNAL_IN) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_signal_lands_after_the_outgoing_arc_and_before_the_words() {
        assert!(SIGNAL_IN.0 < ARC_A.1 + 0.01 && SIGNAL_IN.1 <= WORDMARK.1);
        assert!(GREETING.0 > WORDMARK.0 && CARDS.0 > GREETING.0);
        assert!(CARDS.1 <= 1.0 && HINT.1 <= 1.0);
    }
}
