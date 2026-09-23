use super::*;
use std::collections::VecDeque;

fn percent(item: &Value) -> Option<f64> {
    item["windows"].as_object()?.values().find_map(|v| {
        v["used_pct"]
            .as_f64()
            .or(v["pct"].as_f64())
            .map(|n| n.clamp(0., 100.))
    })
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

    /// Coalesced like `refresh`: one pass at a time, and events that arrive meanwhile ask for
    /// exactly one more. Passes run in order, so an older reply never lands after a newer one.
    /// `devices` also lists devices, which starts an `adb` process; only device events and the
    /// first load need that.
    pub(super) fn refresh_status(self: &Rc<Self>, devices: bool) {
        self.render_status_counts();
        if devices {
            self.status_devices.set(true);
        }
        self.status_dirty.set(true);
        if self.status_pending.replace(true) {
            return;
        }
        let ui = self.clone();
        glib::spawn_future_local(async move {
            while ui.status_dirty.replace(false) {
                ui.refresh_status_once().await;
            }
            ui.status_pending.set(false);
        });
    }

    async fn refresh_status_once(self: &Rc<Self>) {
        let ui = self;
        let generation = ui.generation.get();
        let list_devices = ui.status_devices.replace(false);
        let (usage, devices, providers) = tokio::join!(
            ui.call("usage.get", json!({})),
            async {
                if list_devices {
                    Some(ui.call("device.list", json!({})).await)
                } else {
                    None
                }
            },
            ui.call("provider.list", json!({}))
        );
        if generation != ui.generation.get() {
            return;
        }
        {
            if let Ok(v) = usage {
                clear(&ui.usage_meters);
                let mut usage = rows(&v, "usage");
                if let Ok(providers) = providers {
                    for provider in rows(&providers, "providers") {
                        if provider["installed"] == true
                            && !usage.iter().any(|u| u["provider"] == provider["provider"])
                        {
                            usage.push(json!({"provider":provider["provider"],"windows":{}}));
                        }
                    }
                }
                for item in usage {
                    let provider = text(&item, "provider");
                    let pct = percent(&item);
                    let meter = gtk::Box::new(gtk::Orientation::Horizontal, 5);
                    meter.append(&label(provider, "mono"));
                    let bar = gtk::ProgressBar::new();
                    bar.add_css_class("usage-meter");
                    bar.set_valign(gtk::Align::Center);
                    bar.set_fraction(pct.unwrap_or(0.) / 100.);
                    if pct.is_some_and(|pct| pct >= 85.) {
                        bar.add_css_class("hot");
                    }
                    meter.append(&bar);
                    meter.append(&label(
                        &pct.map(|n| format!("{n:.0}%"))
                            .unwrap_or_else(|| "?".into()),
                        "mono",
                    ));
                    meter.set_tooltip_text(Some(
                        &pct.map(|n| format!("{provider}: {n:.0}% used"))
                            .unwrap_or_else(|| format!("{provider}: no usage window reported")),
                    ));
                    ui.usage_meters.append(&meter);
                }
                if ui.usage_meters.first_child().is_none() {
                    ui.usage_meters.append(&label("Usage", "mono"));
                }
                ui.usage_meters
                    .append(&crate::icons::image("chevron-down", 10));
            }
            match devices {
                Some(Ok(v)) => {
                    let n = rows(&v, "devices").len();
                    ui.device_status.set_text(&match n {
                        0 => "No device".into(),
                        1 => "1 device".into(),
                        _ => format!("{n} devices"),
                    });
                }
                // Try again on the next pass rather than leaving a stale count.
                Some(Err(_)) => ui.status_devices.set(true),
                None => {}
            }
        }
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
            let mut history = VecDeque::new();
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
                Ok(v) => render_resources(&body, &v, &mut history),
                Err(e) => {
                    clear(&body);
                    body.append(&label(&e.to_string(), "dim"));
                    return;
                }
            }
            while let Ok(notice) = notices.recv().await {
                match notice {
                    Notice::Event(e) if e.ev == "resource.sample" => {
                        render_resources(&body, &e.payload, &mut history)
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

fn resource_row(body: &gtk::Box, name: &str, detail: &str, fraction: f64, value: &str, size: f64) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("resource-row");
    let title = gtk::Box::new(gtk::Orientation::Vertical, 2);
    title.set_hexpand(true);
    for (s, class) in [(name, "body"), (detail, "dim")] {
        let l = label(s, class);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(23);
        l.set_tooltip_text(Some(s));
        title.append(&l);
    }
    row.append(&title);
    let bar = gtk::ProgressBar::new();
    bar.set_valign(gtk::Align::Center);
    bar.set_fraction(fraction.clamp(0., 1.));
    bar.add_css_class("resource-meter");
    bar.set_size_request(80, -1);
    row.append(&bar);
    let value = label(value, "mono");
    value.set_size_request(56, -1);
    value.set_xalign(1.);
    row.append(&value);
    let size = label(&format!("{size:.0} MB"), "mono");
    size.set_size_request(76, -1);
    size.set_xalign(1.);
    row.append(&size);
    body.append(&row);
}

fn render_resources(body: &gtk::Box, v: &Value, history: &mut VecDeque<[f64; 3]>) {
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
    history.push_back([cpu, memory, disk]);
    if history.len() > 40 {
        history.pop_front();
    }
    clear(body);
    body.append(&label(
        &format!(
            "{:.0} MB Relay   {memory:.0} MB agents   {:.0} MB store",
            v["relay"]["rss_mb"].as_f64().unwrap_or(0.),
            v["store_mb"].as_f64().unwrap_or(0.)
        ),
        "mono",
    ));
    let charts = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for (i, name, value) in [
        (0, "CPU", format!("{cpu:.1}%")),
        (1, "Memory", format!("{memory:.0} MB")),
        (2, "Disk", format!("{:.1} GB", disk.max(0.) / 1024.)),
    ] {
        let chart = gtk::Box::new(gtk::Orientation::Vertical, 4);
        chart.add_css_class("resource-chart");
        chart.set_hexpand(true);
        chart.append(&label(&format!("{name}  {value}"), "dim"));
        let graph = gtk::DrawingArea::new();
        graph.set_content_height(48);
        let values: Vec<f64> = history.iter().map(|p| p[i]).collect();
        graph.set_draw_func(move |_, cr, w, h| {
            let max = values.iter().copied().fold(1., f64::max);
            cr.set_source_rgb(0.55, 0.56, 0.58);
            cr.set_line_width(1.5);
            for (n, val) in values.iter().enumerate() {
                let x = 2. + (w - 4) as f64 * n as f64 / (values.len() - 1).max(1) as f64;
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
        charts.append(&chart);
    }
    body.append(&charts);
    body.append(&label("RELAY", "section-label"));
    let relay = &v["relay"];
    resource_row(
        body,
        "Relay engine",
        &format!("pid {}", relay["pid"]),
        relay["rss_mb"].as_f64().unwrap_or(0.)
            / max_rss.max(relay["rss_mb"].as_f64().unwrap_or(0.)),
        &format!("{:.1}%", relay["cpu_pct"].as_f64().unwrap_or(0.)),
        relay["rss_mb"].as_f64().unwrap_or(0.),
    );
    body.append(&label(
        &format!("SESSIONS {}", panes.len()),
        "section-label",
    ));
    for p in &panes {
        let rss = p["rss_mb"].as_f64().unwrap_or(0.);
        resource_row(
            body,
            text(p, "session"),
            &p["pid"]
                .as_i64()
                .map(|pid| format!("pid {pid}"))
                .unwrap_or_else(|| "parked".into()),
            rss / max_rss,
            &format!("{:.1}%", p["cpu_pct"].as_f64().unwrap_or(0.)),
            rss,
        );
    }
    body.append(&label(
        &format!("WORKTREES {}", trees.len()),
        "section-label",
    ));
    for t in &trees {
        let path = text(t, "path");
        resource_row(
            body,
            path.rsplit('/').next().unwrap_or(path),
            path,
            t["disk_mb"].as_f64().unwrap_or(0.) / max_disk,
            &t["build_mb"]
                .as_f64()
                .map(|n| format!("{n:.0} build"))
                .unwrap_or_else(|| "clean".into()),
            t["disk_mb"].as_f64().unwrap_or(0.),
        );
    }
}
