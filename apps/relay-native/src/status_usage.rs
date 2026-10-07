//! Provider limit meters: the status-bar strip, its popup and the opt-in refresh interval.
//!
//! `usage.get` only re-reads what Claude Code and Codex already saved on this machine, so a
//! refresh never contacts a provider or spends a token. What shows, and whether the window
//! re-reads on a timer, lives under the `usage` settings subtree.
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

const PROVIDERS: [(&str, &str); 2] = [("claude", "Claude"), ("codex", "Codex")];
/// Minutes; 0 leaves refreshes to finished turns, engine events and the refresh keys.
const INTERVALS: [(u64, &str); 6] = [(0, "Off"), (1, "1m"), (5, "5m"), (15, "15m"), (30, "30m"), (60, "1h")];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Meter {
    FiveHour,
    Weekly,
    Fable,
}

impl Meter {
    fn key(self) -> &'static str {
        match self {
            Meter::FiveHour => "five_hour",
            Meter::Weekly => "weekly",
            Meter::Fable => "fable",
        }
    }
    fn caption(self) -> &'static str {
        match self {
            Meter::FiveHour => "5h",
            Meter::Weekly => "wk",
            Meter::Fable => "fable",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Meter::FiveHour => "5-hour",
            Meter::Weekly => "Weekly",
            Meter::Fable => "Fable",
        }
    }
}

fn meters(provider: &str) -> &'static [Meter] {
    if provider == "claude" {
        &[Meter::FiveHour, Meter::Weekly, Meter::Fable]
    } else {
        &[Meter::FiveHour, Meter::Weekly]
    }
}

/// Window names differ by provider and version: Claude reports `five_hour`/`seven_day`, the
/// engine names Codex windows by length, older reports use `primary`/`secondary`.
fn classify(name: &str) -> Option<Meter> {
    let name = name.to_ascii_lowercase();
    if name.contains("fable") {
        Some(Meter::Fable)
    } else if matches!(name.as_str(), "five_hour" | "primary" | "5h" | "300_minute") {
        Some(Meter::FiveHour)
    } else if matches!(name.as_str(), "seven_day" | "weekly" | "secondary" | "10080_minute") {
        Some(Meter::Weekly)
    } else {
        None
    }
}

#[derive(Debug)]
struct Window {
    name: String,
    pct: f64,
    resets_at: Option<u64>,
    resets_in: Option<String>,
}

impl Window {
    /// The reset passed after the CLI saved this report; the next report shows the new window.
    fn stale(&self, now: u64) -> bool {
        match self.resets_at {
            Some(at) => at <= now,
            None => self.resets_in.as_deref() == Some("0m"),
        }
    }
}

fn windows(item: &Value) -> Vec<Window> {
    let Some(map) = item["windows"].as_object() else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(name, value)| {
            let pct = value["used_pct"].as_f64().or(value["pct"].as_f64())?;
            Some(Window {
                name: name.clone(),
                pct: pct.clamp(0., 100.),
                resets_at: value["resets_at"].as_u64(),
                resets_in: value["resets_in"].as_str().map(str::to_string),
            })
        })
        .collect()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn parse_time(value: &str) -> Option<u64> {
    glib::DateTime::from_iso8601(value, None).ok().map(|at| at.to_unix().max(0) as u64)
}

fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..172_800 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

fn span(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds % 86_400 / 3600, seconds % 3600 / 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        "under a minute".into()
    }
}

/// "21:30" today, "Tue 21:30" on another day.
fn wall_clock(at: u64) -> Option<String> {
    let when = glib::DateTime::from_unix_local(at as i64).ok()?;
    let today = glib::DateTime::now_local().ok()?;
    let same_day = (when.year(), when.day_of_year()) == (today.year(), today.day_of_year());
    when.format(if same_day { "%H:%M" } else { "%a %H:%M" }).ok().map(|s| s.to_string())
}

fn reset_text(window: &Window, now: u64) -> String {
    if window.stale(now) {
        return "Reset since the last report".into();
    }
    match (window.resets_at, &window.resets_in) {
        (Some(at), _) => match wall_clock(at) {
            Some(clock) => format!("Resets in {} · {clock}", span(at - now)),
            None => format!("Resets in {}", span(at - now)),
        },
        (None, Some(label)) => format!("Resets in {label}"),
        (None, None) => "Reset time not reported".into(),
    }
}

fn level(pct: f64) -> Option<&'static str> {
    if pct >= 90. {
        Some("hot")
    } else if pct >= 70. {
        Some("warn")
    } else {
        None
    }
}

fn enabled(prefs: &Value, provider: &str, key: &str) -> bool {
    prefs[provider][key].as_bool().unwrap_or(true)
}

fn interval(prefs: &Value) -> u64 {
    prefs["refresh_minutes"].as_u64().unwrap_or(0).min(24 * 60)
}

fn interval_text(minutes: u64) -> String {
    match minutes {
        0 => "refreshes when an agent finishes a turn and on demand".into(),
        60 => "refreshes every hour".into(),
        m => format!("refreshes every {m} min"),
    }
}

/// Writes one dotted leaf into the local copy, creating objects along the way.
fn put(prefs: &mut Value, path: &str, value: Value) {
    let mut cursor = prefs;
    let parts: Vec<&str> = path.split('.').collect();
    for (index, part) in parts.iter().enumerate() {
        if !cursor.is_object() {
            *cursor = json!({});
        }
        let map = cursor.as_object_mut().expect("object");
        if index == parts.len() - 1 {
            map.insert(part.to_string(), value);
            return;
        }
        cursor = map.entry(part.to_string()).or_insert_with(|| json!({}));
    }
}

fn bar(pct: f64, stale: bool, class: &str) -> gtk::ProgressBar {
    let bar = gtk::ProgressBar::new();
    bar.add_css_class(class);
    bar.set_valign(gtk::Align::Center);
    bar.set_fraction(pct / 100.);
    if stale {
        bar.add_css_class("stale");
    } else if let Some(level) = level(pct) {
        bar.add_css_class(level);
    }
    bar
}

struct Popup {
    checked: glib::WeakRef<gtk::Label>,
    providers: glib::WeakRef<gtk::Box>,
    refresh: glib::WeakRef<gtk::Button>,
}

pub(crate) struct UsageState {
    pub(crate) strip: gtk::Box,
    age: gtk::Label,
    refresh: gtk::Button,
    /// Unix seconds of the last successful `usage.get`.
    checked: Cell<Option<u64>>,
    failed: RefCell<Option<String>>,
    prefs: RefCell<Value>,
    rows: RefCell<Vec<Value>>,
    /// Installed providers, and the connection generation that discovered them.
    installed: RefCell<Option<(u64, Vec<String>)>>,
    busy: Cell<bool>,
    queued: Cell<Option<bool>>,
    stale_windows: Cell<usize>,
    /// Idle sessions as (name, updated_at) at the last clock tick: a new pair is a finished
    /// turn, after which the providers have saved fresh limits.
    turns: RefCell<Option<std::collections::HashSet<(String, String)>>>,
    timer: RefCell<Option<glib::SourceId>>,
    popup: RefCell<Option<Popup>>,
}

impl UsageState {
    pub(crate) fn new() -> Self {
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        strip.add_css_class("usage-strip");
        let age = label("", "usage-age");
        age.set_widget_name("status-usage-age");
        strip.append(&age);
        let refresh = icon_button("refresh", "Re-read usage limits");
        refresh.set_child(Some(&crate::icons::image("refresh", 12)));
        refresh.add_css_class("usage-refresh");
        refresh.set_widget_name("status-usage-refresh");
        strip.append(&refresh);
        Self {
            strip,
            age,
            refresh,
            checked: Cell::new(None),
            failed: RefCell::default(),
            prefs: RefCell::new(Value::Null),
            rows: RefCell::default(),
            installed: RefCell::default(),
            busy: Cell::new(false),
            queued: Cell::new(None),
            stale_windows: Cell::new(0),
            turns: RefCell::default(),
            timer: RefCell::default(),
            popup: RefCell::default(),
        }
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.refresh.set_sensitive(!busy);
        if let Some(key) = self.popup.borrow().as_ref().and_then(|p| p.refresh.upgrade()) {
            key.set_sensitive(!busy);
        }
    }

    /// Providers worth a row: every enabled one that reported, plus enabled installed ones.
    fn visible(&self) -> Vec<(&'static str, &'static str, Option<Value>)> {
        let prefs = self.prefs.borrow();
        let rows = self.rows.borrow();
        let installed = self.installed.borrow();
        PROVIDERS
            .iter()
            .filter(|(id, _)| enabled(&prefs, id, "enabled"))
            .filter_map(|&(id, name)| {
                let item = rows.iter().find(|row| text(row, "provider") == id).cloned();
                let present = installed.as_ref().is_some_and(|(_, list)| list.iter().any(|p| p == id));
                (item.is_some() || present).then_some((id, name, item))
            })
            .collect()
    }
}

impl Ui {
    /// Wires the strip's refresh key and the clock that keeps its "updated" label honest.
    /// The clock rewrites labels, and re-reads usage only after an agent finished a turn:
    /// nothing reports usage to the engine, so that is when the saved limits move.
    pub(crate) fn install_usage(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.usage.refresh.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.refresh_usage(true);
            }
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(30, move || {
            let Some(ui) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if ui.usage_turn_ended() {
                ui.refresh_usage(false);
            }
            let stale = ui.stale_count();
            if stale != ui.usage.stale_windows.get() {
                ui.render_usage();
            } else {
                ui.render_usage_age();
                ui.render_usage_popup();
            }
            glib::ControlFlow::Continue
        });
        self.render_usage();
    }

    /// Re-reads `usage.get`. `discover` also re-lists installed providers and display
    /// preferences, which happens anyway on the first refresh of each connection.
    pub(crate) fn refresh_usage(self: &Rc<Self>, discover: bool) {
        let state = &self.usage;
        let generation = self.generation.get();
        let discover = discover
            || state.installed.borrow().as_ref().is_none_or(|(seen, _)| *seen != generation);
        if state.busy.get() {
            state.queued.set(Some(state.queued.get().unwrap_or(false) || discover));
            return;
        }
        state.set_busy(true);
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let usage = ui.call("usage.get", json!({}));
            let providers = async {
                if discover {
                    Some(ui.call("provider.list", json!({})).await)
                } else {
                    None
                }
            };
            let prefs = async {
                if discover {
                    Some(ui.call("settings.get", json!({"path":"usage"})).await)
                } else {
                    None
                }
            };
            let (usage, providers, prefs) = tokio::join!(usage, providers, prefs);
            let state = &ui.usage;
            state.set_busy(false);
            if generation == ui.generation.get() {
                if let Some(Ok(v)) = providers {
                    let list = rows(&v, "providers")
                        .iter()
                        .filter(|p| p["installed"] == true)
                        .map(|p| text(p, "provider").to_string())
                        .collect();
                    *state.installed.borrow_mut() = Some((generation, list));
                }
                if let Some(Ok(v)) = prefs {
                    *state.prefs.borrow_mut() = v["value"].clone();
                }
                match usage {
                    Ok(v) => {
                        *state.rows.borrow_mut() = rows(&v, "usage");
                        state.checked.set(Some(now()));
                        state.failed.borrow_mut().take();
                    }
                    Err(e) => *state.failed.borrow_mut() = Some(e.to_string()),
                }
                ui.render_usage();
                ui.schedule_usage();
            }
            if let Some(discover) = state.queued.take() {
                ui.refresh_usage(discover);
            }
        });
    }

    /// `settings.changed` under `usage`: re-render from the cached rows, no new read.
    pub(crate) fn reload_usage_prefs(self: &Rc<Self>) {
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let generation = ui.generation.get();
            if let Ok(v) = ui.call("settings.get", json!({"path":"usage"})).await {
                if generation == ui.generation.get() && *ui.usage.prefs.borrow() != v["value"] {
                    *ui.usage.prefs.borrow_mut() = v["value"].clone();
                    ui.render_usage();
                    ui.schedule_usage();
                }
            }
        });
    }

    /// One pending timeout at most, counted from the last refresh.
    fn schedule_usage(self: &Rc<Self>) {
        if let Some(source) = self.usage.timer.borrow_mut().take() {
            source.remove();
        }
        let minutes = interval(&self.usage.prefs.borrow());
        if minutes == 0 {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_seconds_local((minutes * 60) as u32, move || {
            if let Some(ui) = weak.upgrade() {
                // Returning Break destroys this source; forget its id so nobody removes it twice.
                ui.usage.timer.borrow_mut().take();
                ui.refresh_usage(false);
            }
            glib::ControlFlow::Break
        });
        *self.usage.timer.borrow_mut() = Some(source);
    }

    fn set_usage_pref(self: &Rc<Self>, path: &'static str, value: Value) {
        put(&mut self.usage.prefs.borrow_mut(), path, value.clone());
        self.render_usage();
        if path == "refresh_minutes" {
            self.schedule_usage();
        }
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let payload = json!({"path": format!("usage.{path}"), "value": value});
            if let Err(e) = ui.call("settings.set", payload).await {
                ui.show_error(&e.to_string());
                ui.reload_usage_prefs();
            }
        });
    }

    /// Whether a session went idle since the last tick. The first tick only takes stock: the
    /// connection's own refresh already read usage.
    fn usage_turn_ended(&self) -> bool {
        if !self.connected.get() {
            self.usage.turns.borrow_mut().take();
            return false;
        }
        let idle: std::collections::HashSet<(String, String)> = self
            .sidebar_sessions
            .borrow()
            .iter()
            .filter(|s| text(s, "state") == "idle")
            .map(|s| (text(s, "name").to_string(), text(s, "updated_at").to_string()))
            .collect();
        let before = self.usage.turns.replace(Some(idle.clone()));
        before.is_some_and(|before| !idle.is_subset(&before))
    }

    fn stale_count(&self) -> usize {
        let now = now();
        self.usage
            .rows
            .borrow()
            .iter()
            .flat_map(windows)
            .filter(|w| w.stale(now))
            .count()
    }

    fn render_usage(&self) {
        self.usage.stale_windows.set(self.stale_count());
        self.render_usage_meters();
        self.render_usage_age();
        self.render_usage_popup();
    }

    fn render_usage_meters(&self) {
        let now = now();
        let prefs = self.usage.prefs.borrow().clone();
        clear(&self.usage_meters);
        let mut tooltip = Vec::new();
        for (provider, name, item) in self.usage.visible() {
            if self.usage_meters.first_child().is_some() {
                self.usage_meters.append(&label("·", "faint"));
            }
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            group.add_css_class("usage-group");
            group.append(&label(name, "usage-name"));
            let found = item.as_ref().map(windows).unwrap_or_default();
            let mut shown = 0;
            for &meter in meters(provider) {
                if !enabled(&prefs, provider, meter.key()) {
                    continue;
                }
                let Some(window) = found.iter().find(|w| classify(&w.name) == Some(meter)) else {
                    continue;
                };
                let stale = window.stale(now);
                group.append(&label(meter.caption(), "usage-caption"));
                group.append(&bar(window.pct, stale, "usage-meter"));
                let pct = label(&format!("{:.0}%", window.pct), "usage-value");
                if stale {
                    pct.add_css_class("stale");
                } else if let Some(level) = level(window.pct) {
                    pct.add_css_class(level);
                }
                group.append(&pct);
                tooltip.push(format!("{name} {}  {:.0}% · {}", meter.title(), window.pct, reset_text(window, now)));
                shown += 1;
            }
            if shown == 0 {
                group.append(&label("–", "usage-caption"));
                tooltip.push(format!(
                    "{name}: {}",
                    if item.is_some() { "no shown meter reported" } else { "no limits reported yet" }
                ));
            }
            self.usage_meters.append(&group);
        }
        if self.usage_meters.first_child().is_none() {
            self.usage_meters.append(&label("Usage", "usage-name"));
            tooltip.push("Usage limits".into());
        }
        self.usage_meters.append(&crate::icons::image("chevron-down", 10));
        tooltip.push("Click for details and display options".into());
        self.usage_meters.set_tooltip_text(Some(&tooltip.join("\n")));
    }

    fn render_usage_age(&self) {
        let state = &self.usage;
        let minutes = interval(&state.prefs.borrow());
        let (text, tip) = match (state.checked.get(), state.failed.borrow().as_ref()) {
            (_, Some(error)) => ("update failed".to_string(), format!("The last refresh failed: {error}")),
            (Some(at), None) => (
                format!("updated {}", ago(now().saturating_sub(at))),
                format!(
                    "Limits checked at {} · {}",
                    glib::DateTime::from_unix_local(at as i64)
                        .ok()
                        .and_then(|d| d.format("%H:%M:%S").ok())
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                    interval_text(minutes)
                ),
            ),
            (None, None) => ("not checked".to_string(), "Limits have not been read yet".to_string()),
        };
        state.age.set_text(&text);
        state.age.set_tooltip_text(Some(&tip));
        if state.failed.borrow().is_some() {
            state.age.add_css_class("failed");
        } else {
            state.age.remove_css_class("failed");
        }
    }

    fn render_usage_popup(&self) {
        let (checked, list) = {
            let popup = self.usage.popup.borrow();
            let Some(popup) = popup.as_ref() else {
                return;
            };
            let (Some(checked), Some(list)) = (popup.checked.upgrade(), popup.providers.upgrade()) else {
                return;
            };
            (checked, list)
        };
        let now = now();
        let state = &self.usage;
        let prefs = state.prefs.borrow().clone();
        checked.set_text(&match (state.checked.get(), state.failed.borrow().as_ref()) {
            (_, Some(error)) => format!("The last refresh failed: {error}"),
            (Some(at), None) => format!("Checked {} · {}", ago(now.saturating_sub(at)), interval_text(interval(&prefs))),
            (None, None) => "Checking…".into(),
        });
        clear(&list);
        let visible = state.visible();
        if visible.is_empty() {
            let note = label(
                if PROVIDERS.iter().any(|(id, _)| enabled(&prefs, id, "enabled")) {
                    "Neither Claude Code nor Codex has reported limits on this machine yet."
                } else {
                    "Claude and Codex are both hidden. Turn one on below."
                },
                "usage-note",
            );
            note.set_wrap(true);
            list.append(&note);
        }
        for (provider, name, item) in visible {
            let section = gtk::Box::new(gtk::Orientation::Vertical, 10);
            section.add_css_class("usage-provider");
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            head.add_css_class("usage-provider-head");
            head.append(&crate::icons::image(provider, 14));
            let title = label(name, "usage-provider-name");
            title.set_hexpand(true);
            head.append(&title);
            let reported = item.as_ref().and_then(|i| parse_time(text(i, "taken_at")));
            let reported = label(
                &match (&item, reported) {
                    (None, _) => "nothing reported".into(),
                    (Some(_), Some(at)) => format!("reported {}", ago(now.saturating_sub(at))),
                    (Some(_), None) => String::new(),
                },
                "usage-reported",
            );
            reported.set_tooltip_text(Some(match provider {
                "claude" => "Claude Code saves its limits through Relay's status line on every reply",
                _ => "Codex records its limits in its session log while it works",
            }));
            head.append(&reported);
            section.append(&head);
            let Some(item) = item else {
                let note = label(
                    match provider {
                        "claude" => "Limits appear after Claude Code's next reply in a Relay session.",
                        _ => "Limits appear after Codex's next turn.",
                    },
                    "usage-note",
                );
                note.set_wrap(true);
                section.append(&note);
                list.append(&section);
                continue;
            };
            let found = windows(&item);
            let mut shown = 0;
            for &meter in meters(provider) {
                if !enabled(&prefs, provider, meter.key()) {
                    continue;
                }
                shown += 1;
                match found.iter().find(|w| classify(&w.name) == Some(meter)) {
                    Some(window) => section.append(&meter_row(meter.title(), window, now)),
                    None => section.append(&missing_row(
                        meter.title(),
                        if provider == "claude" { "Not reported by Claude Code" } else { "Not reported by Codex" },
                    )),
                }
            }
            for window in found.iter().filter(|w| classify(&w.name).is_none()) {
                section.append(&meter_row(&humanize(&window.name), window, now));
                shown += 1;
            }
            if shown == 0 {
                section.append(&label("Every meter is hidden for this provider.", "usage-note"));
            }
            list.append(&section);
        }
    }

    pub(crate) fn usage_panel(self: &Rc<Self>) {
        let Some(panel) = crate::panel::Panel::toggle(self, "Usage limits", 420) else {
            return;
        };
        panel.bottom(480);
        panel.add_css_class("usage-panel");
        let refresh = icon_button("refresh", "Re-read usage limits");
        refresh.set_widget_name("usage-panel-refresh");
        refresh.set_sensitive(!self.usage.busy.get());
        let weak = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.refresh_usage(true);
            }
        });
        panel.header_action(&refresh);
        let body = panel.body.clone();
        body.add_css_class("usage-body");
        body.set_spacing(0);
        let checked = label("", "usage-checked");
        body.append(&checked);
        let providers = gtk::Box::new(gtk::Orientation::Vertical, 0);
        providers.add_css_class("usage-providers");
        body.append(&providers);
        body.append(&self.usage_options());
        let footer = label(
            "Refreshing re-reads what each CLI last saved on this machine. It never contacts a provider or spends tokens.",
            "usage-footnote",
        );
        footer.set_wrap(true);
        body.append(&footer);
        *self.usage.popup.borrow_mut() = Some(Popup {
            checked: checked.downgrade(),
            providers: providers.downgrade(),
            refresh: refresh.downgrade(),
        });
        let weak = Rc::downgrade(self);
        panel.on_closed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.usage.popup.borrow_mut().take();
            }
        });
        self.render_usage_popup();
        panel.present();
        self.refresh_usage(false);
    }

    /// Built once per open: rebuilding would steal focus from the control being used.
    fn usage_options(self: &Rc<Self>) -> gtk::Box {
        let prefs = self.usage.prefs.borrow().clone();
        let options = gtk::Box::new(gtk::Orientation::Vertical, 8);
        options.add_css_class("usage-options");
        options.append(&label("Show in the status bar", "usage-options-title"));
        for (provider, name) in PROVIDERS {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("usage-option-row");
            let switch = gtk::Switch::new();
            switch.set_valign(gtk::Align::Center);
            switch.set_active(enabled(&prefs, provider, "enabled"));
            switch.update_property(&[gtk::accessible::Property::Label(&format!("Show {name}"))]);
            row.append(&switch);
            let title = label(name, "usage-option-name");
            title.set_hexpand(true);
            row.append(&title);
            let chips = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            chips.add_css_class("linked");
            chips.add_css_class("usage-chips");
            chips.set_sensitive(switch.is_active());
            for &meter in meters(provider) {
                let chip = gtk::ToggleButton::with_label(meter.title());
                chip.add_css_class("usage-chip");
                chip.set_active(enabled(&prefs, provider, meter.key()));
                chip.set_tooltip_text(Some(&format!("Show {name}'s {} limit", meter.title().to_lowercase())));
                let weak = Rc::downgrade(self);
                let path: &'static str = match (provider, meter) {
                    ("claude", Meter::FiveHour) => "claude.five_hour",
                    ("claude", Meter::Weekly) => "claude.weekly",
                    ("claude", Meter::Fable) => "claude.fable",
                    (_, Meter::FiveHour) => "codex.five_hour",
                    _ => "codex.weekly",
                };
                chip.connect_toggled(move |chip| {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_usage_pref(path, json!(chip.is_active()));
                    }
                });
                chips.append(&chip);
            }
            let weak = Rc::downgrade(self);
            let toggled = chips.clone();
            let path = if provider == "claude" { "claude.enabled" } else { "codex.enabled" };
            switch.connect_active_notify(move |switch| {
                toggled.set_sensitive(switch.is_active());
                if let Some(ui) = weak.upgrade() {
                    ui.set_usage_pref(path, json!(switch.is_active()));
                }
            });
            row.append(&chips);
            options.append(&row);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("usage-option-row");
        let title = label("Auto refresh", "usage-option-name");
        title.set_hexpand(true);
        title.set_tooltip_text(Some("Re-read the saved limits on a timer. Off still refreshes when an agent finishes a turn."));
        row.append(&title);
        let choices = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        choices.add_css_class("linked");
        choices.add_css_class("usage-chips");
        let current = interval(&prefs);
        let mut first: Option<gtk::ToggleButton> = None;
        for (minutes, caption) in INTERVALS {
            let choice = gtk::ToggleButton::with_label(caption);
            choice.add_css_class("usage-chip");
            choice.set_widget_name(&format!("usage-interval-{minutes}"));
            if let Some(first) = &first {
                choice.set_group(Some(first));
            }
            choice.set_active(minutes == current);
            let weak = Rc::downgrade(self);
            choice.connect_toggled(move |choice| {
                if choice.is_active() {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_usage_pref("refresh_minutes", json!(minutes));
                    }
                }
            });
            choices.append(&choice);
            first.get_or_insert(choice);
        }
        row.append(&choices);
        options.append(&row);
        options
    }
}

fn meter_row(title: &str, window: &Window, now: u64) -> gtk::Box {
    let stale = window.stale(now);
    let row = gtk::Box::new(gtk::Orientation::Vertical, 5);
    row.add_css_class("usage-row");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = label(title, "usage-row-title");
    name.set_hexpand(true);
    name.set_valign(gtk::Align::End);
    top.append(&name);
    let pct = label(&format!("{:.0}%", window.pct), "usage-pct");
    pct.set_valign(gtk::Align::End);
    if stale {
        pct.add_css_class("stale");
    } else if let Some(level) = level(window.pct) {
        pct.add_css_class(level);
    }
    top.append(&pct);
    row.append(&top);
    row.append(&bar(window.pct, stale, "usage-bar"));
    let reset = label(&reset_text(window, now), "usage-reset");
    if stale {
        reset.add_css_class("stale");
    }
    row.append(&reset);
    row
}

fn missing_row(title: &str, note: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("usage-row");
    row.add_css_class("missing");
    let name = label(title, "usage-row-title");
    name.set_hexpand(true);
    row.append(&name);
    row.append(&label(note, "usage-reset"));
    row
}

fn humanize(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_map_to_the_three_meters_by_name() {
        assert_eq!(classify("five_hour"), Some(Meter::FiveHour));
        assert_eq!(classify("primary"), Some(Meter::FiveHour));
        assert_eq!(classify("seven_day"), Some(Meter::Weekly));
        assert_eq!(classify("weekly"), Some(Meter::Weekly));
        assert_eq!(classify("secondary"), Some(Meter::Weekly));
        assert_eq!(classify("seven_day_fable"), Some(Meter::Fable));
        assert_eq!(classify("Fable"), Some(Meter::Fable));
        assert_eq!(classify("seven_day_opus"), None);
    }

    #[test]
    fn a_window_whose_reset_passed_is_stale() {
        let at = |resets_at, resets_in: Option<&str>| Window {
            name: "weekly".into(),
            pct: 63.,
            resets_at,
            resets_in: resets_in.map(str::to_string),
        };
        assert!(at(Some(100), None).stale(100));
        assert!(!at(Some(101), None).stale(100));
        assert!(at(None, Some("0m")).stale(100), "an older engine only sends the relative label");
        assert!(!at(None, Some("3h 2m")).stale(100));
        assert_eq!(reset_text(&at(Some(50), None), 100), "Reset since the last report");
        assert!(reset_text(&at(Some(100 + 4 * 3600 + 30 * 60), None), 100).starts_with("Resets in 4h 30m · "));
    }

    #[test]
    fn preferences_default_on_and_write_locally() {
        let mut prefs = Value::Null;
        assert!(enabled(&prefs, "claude", "fable"));
        assert_eq!(interval(&prefs), 0);
        put(&mut prefs, "codex.enabled", json!(false));
        put(&mut prefs, "refresh_minutes", json!(15));
        assert!(!enabled(&prefs, "codex", "enabled"));
        assert!(enabled(&prefs, "codex", "weekly"));
        assert_eq!(interval(&prefs), 15);
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(3 * 60 + 5), "3m ago");
        assert_eq!(span(2 * 86_400 + 5 * 3600), "2d 5h");
        assert_eq!(humanize("seven_day_opus"), "Seven day opus");
    }
}
