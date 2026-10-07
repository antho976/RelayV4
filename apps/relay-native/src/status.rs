use super::*;
use std::collections::VecDeque;

#[path = "status_usage.rs"]
mod usage;
pub(super) use usage::UsageState;

thread_local! {
    /// A device count is being read, and whether another event asked for one meanwhile: a
    /// burst of device and run events reads it at most twice.
    static DEVICES_BUSY: Cell<bool> = const { Cell::new(false) };
    static DEVICES_QUEUED: Cell<bool> = const { Cell::new(false) };
}

impl Ui {
    pub(super) fn render_status_counts(&self) {
        clear(&self.resource_status);
        let sessions = self.sessions.borrow();
        let blocked = sessions
            .iter()
            .filter(|s| text(s, "state") == "blocked")
            .count();
        let live = sessions
            .iter()
            .filter(|s| matches!(text(s, "state"), "spawning" | "running" | "idle"))
            .count();
        let parked = sessions
            .iter()
            .filter(|s| text(s, "state") == "parked")
            .count();
        for (count, caption, color) in [
            (blocked, "needs you", (0.898, 0.22, 0.18)),
            (
                live,
                "live",
                if live > 0 {
                    (0.18, 0.77, 0.41)
                } else {
                    (0.35, 0.35, 0.37)
                },
            ),
            (parked, "parked", (0.47, 0.47, 0.48)),
        ] {
            if count == 0 && caption != "live" {
                continue;
            }
            if self.resource_status.first_child().is_some() {
                self.resource_status.append(&label("·", "faint"));
            }
            if caption != "parked" {
                let lamp = gtk::DrawingArea::new();
                lamp.set_content_width(6);
                lamp.set_content_height(6);
                lamp.set_valign(gtk::Align::Center);
                lamp.set_draw_func(move |_, cr, _, _| {
                    cr.set_source_rgb(color.0, color.1, color.2);
                    cr.arc(3., 3., 3., 0., std::f64::consts::TAU);
                    let _ = cr.fill();
                });
                self.resource_status.append(&lamp);
            }
            self.resource_status.append(&label(
                &format!("{count} {caption}"),
                if caption == "parked" {
                    "faint"
                } else {
                    "status-count"
                },
            ));
        }
    }

    pub(super) fn refresh_status(self: &Rc<Self>) {
        self.render_status_counts();
        self.refresh_usage(false);
        self.refresh_devices();
    }

    fn refresh_devices(self: &Rc<Self>) {
        if self.client.borrow().is_none() {
            return;
        }
        if DEVICES_BUSY.with(|b| b.replace(true)) {
            DEVICES_QUEUED.with(|q| q.set(true));
            return;
        }
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let generation = ui.generation.get();
            // A connection of its own: `adb devices` may take its full 2 s while it starts the
            // adb server, and on the control connection that wait would sit in front of the
            // first project and session reads after connect.
            let list = || Client::lifecycle_request(&ui.rt, ui.path.clone(), "device.list", json!({}));
            let mut devices = list().await;
            if matches!(&devices, Err(Error::Bus(e)) if e.code == "device.adb_timeout") {
                // The server was starting; it answers once it is up, rather than leaving
                // "No device" until the next device event.
                glib::timeout_future_seconds(1).await;
                devices = list().await;
            }
            DEVICES_BUSY.with(|b| b.set(false));
            if DEVICES_QUEUED.with(|q| q.replace(false)) {
                ui.refresh_devices();
            }
            if generation != ui.generation.get() {
                return;
            }
            if let Ok(v) = devices {
                let n = rows(&v, "devices").len();
                ui.device_status.set_text(&match n {
                    0 => "No device".into(),
                    1 => "1 device".into(),
                    _ => format!("{n} devices"),
                });
            }
        });
    }

    pub fn resources(self: &Rc<Self>) {
        let Some(panel) = crate::panel::Panel::toggle(self, "Resources", 460) else {
            return;
        };
        panel.bottom(260);
        panel.add_css_class("resources-panel");
        let body = panel.body.clone();
        body.append(&label("Connecting…", "dim"));
        let ui = self.clone();
        let task = glib::spawn_future_local(async move {
            let result = Client::connect(&ui.rt, ui.path.clone()).await;
            let (client, notices) = match result {
                Ok(v) => v,
                Err(e) => {
                    clear(&body);
                    body.append(&label(&e.to_string(), "dim"));
                    return;
                }
            };
            let history = Rc::new(RefCell::new(VecDeque::new()));
            let mut view = None;
            let result = async {
                client
                    .request(
                        &ui.rt,
                        "bus.subscribe",
                        json!({"events":["resource.sample"]}),
                    )
                    .await?;
                client
                    .request(&ui.rt, "app.resources.watch", json!({"on":true}))
                    .await?;
                client.request(&ui.rt, "app.resources.get", json!({})).await
            }
            .await;
            match result {
                Ok(v) => render_resources(&body, &v, &history, &mut view),
                Err(e) => {
                    clear(&body);
                    body.append(&label(&e.to_string(), "dim"));
                    return;
                }
            }
            while let Ok(notice) = notices.recv().await {
                match notice {
                    Notice::Event(e) if e.ev == "resource.sample" => {
                        render_resources(&body, &e.payload, &history, &mut view)
                    }
                    Notice::Disconnected(e) => {
                        body.append(&label(&e.to_string(), "dim"));
                        break;
                    }
                    _ => {}
                }
            }
        });
        // Closing the socket releases its sampling lease, including app shutdown.
        panel.on_closed(move || task.abort());
        panel.present();
    }
}

/// D66: the panel keeps the last 30 samples.
const HISTORY: usize = 30;

/// The open panel's widgets. A sample updates them in place; they are rebuilt only when the
/// set of sessions or worktrees changes, so a hovered path keeps its tooltip and a sample
/// every 2 s does not recreate every row.
struct ResourceView {
    keys: Vec<String>,
    summary: gtk::Label,
    charts: Vec<(gtk::Label, gtk::DrawingArea)>,
    rows: Vec<ResourceRow>,
}

struct ResourceRow {
    detail: gtk::Label,
    bar: gtk::ProgressBar,
    value: gtk::Label,
    size: gtk::Label,
}

struct RowData {
    key: String,
    name: String,
    detail: String,
    fraction: f64,
    value: String,
    size: f64,
}

/// Unchanged text is left alone: resetting a label's tooltip hides one being read.
fn set_text(label: &gtk::Label, s: &str) {
    if label.text() != s {
        label.set_text(s);
        label.set_tooltip_text(Some(s));
    }
}

fn resource_row(body: &gtk::Box, name: &str) -> ResourceRow {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("resource-row");
    let title = gtk::Box::new(gtk::Orientation::Vertical, 2);
    title.set_hexpand(true);
    let name = label(name, "body");
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(23);
    name.set_tooltip_text(Some(&name.text()));
    title.append(&name);
    // A worktree path differs from its neighbours at the end, not in the shared prefix.
    let detail = label("", "dim");
    detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    detail.set_max_width_chars(23);
    title.append(&detail);
    row.append(&title);
    let bar = gtk::ProgressBar::new();
    bar.set_valign(gtk::Align::Center);
    bar.add_css_class("resource-meter");
    bar.set_size_request(80, -1);
    row.append(&bar);
    let value = label("", "mono");
    value.set_size_request(56, -1);
    value.set_xalign(1.);
    row.append(&value);
    let size = label("", "mono");
    size.set_size_request(76, -1);
    size.set_xalign(1.);
    row.append(&size);
    body.append(&row);
    ResourceRow { detail, bar, value, size }
}

fn build_resources(body: &gtk::Box, history: &Rc<RefCell<VecDeque<[f64; 3]>>>, items: &[RowData]) -> ResourceView {
    clear(body);
    let summary = label("", "mono");
    body.append(&summary);
    let charts_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let mut charts = Vec::new();
    for i in 0..3 {
        let chart = gtk::Box::new(gtk::Orientation::Vertical, 4);
        chart.add_css_class("resource-chart");
        chart.set_hexpand(true);
        let caption = label("", "dim");
        chart.append(&caption);
        let graph = gtk::DrawingArea::new();
        graph.set_content_height(48);
        let history = history.clone();
        graph.set_draw_func(move |_, cr, w, h| {
            let history = history.borrow();
            let max = history.iter().map(|p| p[i]).fold(1., f64::max);
            cr.set_source_rgb(0.55, 0.56, 0.58);
            cr.set_line_width(1.5);
            for (n, val) in history.iter().map(|p| p[i]).enumerate() {
                let x = 2. + (w - 4) as f64 * n as f64 / (history.len() - 1).max(1) as f64;
                let y = h as f64 - 3. - val / max * (h - 6) as f64;
                if n == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            let _ = cr.stroke();
        });
        chart.append(&graph);
        charts_box.append(&chart);
        charts.push((caption, graph));
    }
    body.append(&charts_box);
    let count = |prefix: &str| items.iter().filter(|r| r.key.starts_with(prefix)).count();
    let sessions = count("session:");
    body.append(&label("RELAY", "section-label"));
    let mut rows = Vec::new();
    for item in items {
        rows.push(resource_row(body, &item.name));
        // The relay row comes first, then the sessions, then the worktrees.
        if rows.len() == 1 {
            body.append(&label(&format!("SESSIONS {sessions}"), "section-label"));
        }
        if rows.len() == 1 + sessions {
            body.append(&label(&format!("WORKTREES {}", count("worktree:")), "section-label"));
        }
    }
    ResourceView { keys: items.iter().map(|r| r.key.clone()).collect(), summary, charts, rows }
}

fn render_resources(
    body: &gtk::Box,
    v: &Value,
    history: &Rc<RefCell<VecDeque<[f64; 3]>>>,
    view: &mut Option<ResourceView>,
) {
    let panes = rows(v, "panes");
    let trees = rows(v, "worktrees");
    let cpu = panes
        .iter()
        .map(|p| p["cpu_pct"].as_f64().unwrap_or(0.))
        .sum::<f64>()
        + v["relay"]["cpu_pct"].as_f64().unwrap_or(0.);
    let memory = v["total_rss_mb"].as_f64().unwrap_or(0.);
    let max_rss = panes
        .iter()
        .map(|p| p["rss_mb"].as_f64().unwrap_or(0.))
        .fold(1., f64::max);
    let max_disk = trees
        .iter()
        .map(|p| p["disk_mb"].as_f64().unwrap_or(0.))
        .fold(1., f64::max);
    let disk = trees
        .iter()
        .map(|t| t["disk_mb"].as_f64().unwrap_or(0.))
        .sum::<f64>();
    {
        let mut history = history.borrow_mut();
        history.push_back([cpu, memory, disk]);
        while history.len() > HISTORY {
            history.pop_front();
        }
    }
    let relay = &v["relay"];
    let relay_rss = relay["rss_mb"].as_f64().unwrap_or(0.);
    let mut items = vec![RowData {
        key: "relay".into(),
        name: "Relay engine".into(),
        detail: format!("pid {}", relay["pid"]),
        fraction: relay_rss / max_rss.max(relay_rss),
        value: format!("{:.1}%", relay["cpu_pct"].as_f64().unwrap_or(0.)),
        size: relay_rss,
    }];
    for p in &panes {
        let rss = p["rss_mb"].as_f64().unwrap_or(0.);
        items.push(RowData {
            key: format!("session:{}", text(p, "session")),
            name: text(p, "session").to_owned(),
            detail: p["pid"]
                .as_i64()
                .map(|pid| format!("pid {pid}"))
                .unwrap_or_else(|| "parked".into()),
            fraction: rss / max_rss,
            value: format!("{:.1}%", p["cpu_pct"].as_f64().unwrap_or(0.)),
            size: rss,
        });
    }
    for t in &trees {
        let path = text(t, "path");
        items.push(RowData {
            key: format!("worktree:{path}"),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            detail: path.to_owned(),
            fraction: t["disk_mb"].as_f64().unwrap_or(0.) / max_disk,
            value: t["build_mb"]
                .as_f64()
                .map(|n| format!("{n:.0} build"))
                .unwrap_or_else(|| "clean".into()),
            size: t["disk_mb"].as_f64().unwrap_or(0.),
        });
    }
    if view.as_ref().is_none_or(|view| view.keys.iter().ne(items.iter().map(|r| &r.key))) {
        *view = Some(build_resources(body, history, &items));
    }
    let Some(view) = view.as_ref() else { return };
    view.summary.set_text(&format!(
        "{:.0} MB Relay   {memory:.0} MB agents   {:.0} MB store",
        relay_rss,
        v["store_mb"].as_f64().unwrap_or(0.)
    ));
    for ((caption, graph), text) in view.charts.iter().zip([
        format!("CPU  {cpu:.1}%"),
        format!("Memory  {memory:.0} MB"),
        format!("Disk  {:.1} GB", disk.max(0.) / 1024.),
    ]) {
        caption.set_text(&text);
        graph.queue_draw();
    }
    for (row, item) in view.rows.iter().zip(&items) {
        set_text(&row.detail, &item.detail);
        row.bar.set_fraction(item.fraction.clamp(0., 1.));
        row.value.set_text(&item.value);
        row.size.set_text(&format!("{:.0} MB", item.size));
    }
}
