use crate::client::{Client, Error, Notice};
use base64::Engine;
use gtk4 as gtk;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use tokio::runtime::Handle;
use vte4::prelude::*;

#[derive(Default, Debug)]
pub struct Sequence {
    pub last: Option<(u64, u64)>,
    pub catch_up: bool,
}
#[derive(Debug, PartialEq)]
pub enum Step {
    Feed,
    Reset,
    Duplicate,
    Gap,
}
impl Sequence {
    pub fn observe(&mut self, epoch: u64, seq: u64) -> Step {
        let catch_up = std::mem::take(&mut self.catch_up);
        let step = match self.last {
            None => Step::Feed,
            Some((e, _)) if epoch > e => Step::Reset,
            Some((e, _)) if epoch < e => Step::Duplicate,
            Some((_, s)) if seq <= s => Step::Duplicate,
            Some((_, s)) if catch_up || seq == s + 1 => Step::Feed,
            _ => Step::Gap,
        };
        if matches!(step, Step::Feed | Step::Reset) {
            self.last = Some((epoch, seq));
        }
        step
    }
}

pub struct Pane {
    pub root: gtk::Box,
    pub terminal: vte4::Terminal,
    pub caption: gtk::Label,
    pub actions: gtk::Box,
    pub header: gtk::Box,
    metadata: gtk::Label,
    state: gtk::Label,
    lamp: gtk::Box,
    branch: gtk::Label,
    branch_row: gtk::Box,
    lane: gtk::DrawingArea,
    pub slate_actions: gtk::Box,
    slate: gtk::Box,
    slate_state: gtk::Label,
    slate_hint: gtk::Label,
    /// "Where it left off" on a stopped session's slate (`session_context.rs`).
    slate_context: gtk::Box,
    context_serial: Cell<u64>,
    status: gtk::Label,
    name: String,
    path: PathBuf,
    rt: Handle,
    client: RefCell<Option<Client>>,
    stream: RefCell<Option<glib::JoinHandle<()>>>,
    input: RefCell<Option<glib::JoinHandle<()>>>,
    sequence: RefCell<Sequence>,
    size: Cell<(u16, u16)>,
    resize_pending: Cell<bool>,
    active: Cell<bool>,
}

impl Pane {
    pub fn new(name: &str, path: PathBuf, rt: Handle) -> Rc<Self> {
        let terminal = vte4::Terminal::new();
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);
        terminal.set_size(40, 12);
        terminal.set_scrollback_lines(10_000);
        terminal.set_scroll_on_output(false);
        terminal.set_scroll_on_keystroke(true);
        let mut font = gtk::pango::FontDescription::from_string("Fira Mono");
        font.set_absolute_size(13.0 * gtk::pango::SCALE as f64);
        terminal.set_font(Some(&font));
        terminal.set_cell_height_scale(1.25);
        terminal.set_color_foreground(&gtk::gdk::RGBA::parse("#ececea").unwrap());
        terminal.set_color_background(&gtk::gdk::RGBA::parse("#0a0a0b").unwrap());
        terminal.set_cursor_blink_mode(vte4::CursorBlinkMode::Off);
        terminal.set_margin_start(10);
        terminal.set_margin_end(2);
        terminal.set_margin_top(8);
        terminal.set_margin_bottom(4);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("terminal-plate");
        root.set_size_request(280, 280);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("umd");
        let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        lamp.set_valign(gtk::Align::Center);
        lamp.add_css_class("lamp");
        footer.append(&lamp);
        let caption = gtk::Label::new(Some(name));
        caption.set_xalign(0.0);
        caption.set_max_width_chars(24);
        caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
        caption.add_css_class("session-name");
        let status = gtk::Label::new(Some("Connecting"));
        status.add_css_class("dim");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        footer.append(&caption);
        let metadata = gtk::Label::new(None);
        metadata.add_css_class("terminal-identity");
        metadata.set_ellipsize(gtk::pango::EllipsizeMode::End);

        metadata.set_xalign(0.0);
        footer.append(&metadata);
        let branch = gtk::Label::new(None);
        branch.add_css_class("session-branch");
        branch.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        branch.set_max_width_chars(24);
        let branch_row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        branch_row.add_css_class("session-branch");
        let lane = gtk::DrawingArea::new();
        lane.set_content_width(6);
        lane.set_content_height(6);
        lane.set_valign(gtk::Align::Center);
        branch_row.append(&lane);
        branch_row.append(&branch);
        footer.append(&branch_row);
        let state = gtk::Label::new(None);
        state.add_css_class("session-state");
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);
        footer.append(&state);
        footer.append(&actions);
        root.append(&footer);
        let screen = gtk::Overlay::new();
        screen.set_vexpand(true);
        let terminal_body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        terminal_body.append(&terminal);
        let scroll =
            gtk::Scrollbar::new(gtk::Orientation::Vertical, terminal.vadjustment().as_ref());
        scroll.add_css_class("terminal-scrollbar");
        scroll.set_widget_name("terminal-scrollback");
        terminal_body.append(&scroll);
        screen.set_child(Some(&terminal_body));
        let slate = gtk::Box::new(gtk::Orientation::Vertical, 8);
        slate.add_css_class("session-slate");
        slate.set_halign(gtk::Align::Fill);
        slate.set_valign(gtk::Align::Fill);
        let slate_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        slate_content.set_halign(gtk::Align::Center);
        slate_content.set_valign(gtk::Align::Center);
        slate_content.set_vexpand(true);
        let slate_state = gtk::Label::new(None);
        slate_state.add_css_class("slate-state");
        slate_content.append(&slate_state);
        let slate_hint = gtk::Label::new(None);
        slate_hint.add_css_class("slate-hint");
        slate_hint.set_wrap(true);
        slate_hint.set_max_width_chars(44);
        slate_hint.set_justify(gtk::Justification::Center);
        slate_content.append(&slate_hint);
        let slate_context = gtk::Box::new(gtk::Orientation::Vertical, 8);
        slate_context.add_css_class("slate-context");
        slate_context.set_visible(false);
        slate_content.append(&slate_context);
        let slate_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        slate_actions.set_halign(gtk::Align::Center);
        slate_content.append(&slate_actions);
        // A plate is only 280px square; a tall context card scrolls instead of clipping.
        let slate_scroll = gtk::ScrolledWindow::builder()
            .child(&slate_content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        slate.append(&slate_scroll);
        screen.add_overlay(&slate);
        root.append(&screen);
        status.set_wrap(true);
        status.set_max_width_chars(40);
        status.add_css_class("terminal-status");
        status.connect_label_notify(|label| label.set_visible(!label.text().is_empty()));
        root.append(&status);
        let pane = Rc::new(Self {
            root,
            terminal,
            caption,
            actions,
            header: footer,
            metadata,
            state,
            lamp,
            branch,
            branch_row,
            lane,
            slate_actions,
            slate,
            slate_state,
            slate_hint,
            slate_context,
            context_serial: Cell::new(0),
            status,
            name: name.into(),
            path,
            rt,
            client: RefCell::new(None),
            stream: RefCell::new(None),
            input: RefCell::new(None),
            sequence: RefCell::new(Sequence::default()),
            size: Cell::new((0, 0)),
            resize_pending: Cell::new(false),
            active: Cell::new(false),
        });

        let (tx, rx) = async_channel::bounded::<String>(64);
        let weak = Rc::downgrade(&pane);
        pane.terminal.connect_commit(move |_, text, _| {
            if let Some(p) = weak.upgrade() {
                if p.client.borrow().is_none() {
                    p.status.set_text("Input unavailable: reconnect");
                } else if tx.try_send(text.into()).is_err() {
                    p.status.set_text("Input queue full: keystroke not sent");
                }
            }
        });
        let weak = Rc::downgrade(&pane);
        *pane.input.borrow_mut() = Some(glib::spawn_future_local(async move {
            while let Ok(data) = rx.recv().await {
                let Some(p) = weak.upgrade() else {
                    break;
                };
                let client = p.client.borrow().clone();
                if let Some(client) = client {
                    if let Err(e) = client
                        .request(
                            &p.rt,
                            "session.input",
                            json!({"session":p.name,"data":data}),
                        )
                        .await
                    {
                        p.status.set_text(&e.to_string());
                    }
                }
            }
        }));
        let weak = Rc::downgrade(&pane);
        pane.terminal.connect_char_size_changed(move |_, _, _| {
            if let Some(p) = weak.upgrade() {
                p.schedule_resize();
            }
        });
        let weak = Rc::downgrade(&pane);
        pane.terminal.connect_map(move |_| {
            if let Some(p) = weak.upgrade() {
                p.schedule_resize();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&pane);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if modifiers
                .contains(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
            {
                if let Some(p) = weak.upgrade() {
                    if key == gtk::gdk::Key::C || key == gtk::gdk::Key::c {
                        p.terminal.copy_clipboard_format(vte4::Format::Text);
                        return glib::Propagation::Stop;
                    }
                    if key == gtk::gdk::Key::V || key == gtk::gdk::Key::v {
                        p.terminal.paste_clipboard();
                        return glib::Propagation::Stop;
                    }
                }
            }
            glib::Propagation::Proceed
        });
        pane.terminal.add_controller(keys);
        pane
    }

    pub fn apply_appearance(&self, mode: &str, points: f64) {
        let background = match mode {
            "oled" => "#000000",
            "dark" => "#08090a",
            _ => "#0a0a0b",
        };
        let color = |value: &str| gtk::gdk::RGBA::parse(value).expect("Relay terminal color");
        let palette = [
            "#1a1a1d", "#e5382e", "#3fbf74", "#e0b04a", "#5b9cf6", "#c979d6", "#3ec5cf", "#c8c8c6",
            "#5c5c60", "#ff6b5f", "#5fe08c", "#f2cf6b", "#8ab8ff", "#e19bea", "#68dfe8", "#f2f2f0",
        ]
        .map(color);
        self.terminal.set_colors(
            Some(&color("#dcdcda")),
            Some(&color(background)),
            &palette.iter().collect::<Vec<_>>(),
        );
        self.terminal.set_color_cursor(Some(&color("#ececea")));
        self.terminal
            .set_color_cursor_foreground(Some(&color(background)));
        self.terminal
            .set_color_highlight(Some(&color("rgba(236,236,234,0.22)")));
        let mut font = gtk::pango::FontDescription::from_string("Fira Mono");
        font.set_absolute_size(points.clamp(8.0, 24.0) * 96.0 / 72.0 * gtk::pango::SCALE as f64);
        self.terminal.set_font(Some(&font));
    }
    pub fn verify_ready(&self) {
        assert!(
            self.active.get() && self.client.borrow().is_some(),
            "{} has no ready terminal attachment (active={})",
            self.name,
            self.active.get()
        );
    }
    pub fn verify_session_rendered(&self, session: &serde_json::Value) {
        let state = crate::app::text(session, "state");
        assert!(!self.state.text().is_empty(), "{} has an uninitialized header", self.name);
        assert!(!self.metadata.text().is_empty(), "{} has no provider metadata", self.name);
        assert_eq!(self.slate.is_visible(), !matches!(state, "spawning" | "running" | "idle" | "blocked"),
            "{} has an incorrect terminal overlay", self.name);
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Empty and hide the slate's context card; the returned serial is the only one
    /// [`Pane::context_card`] still answers to, so an older, slower read cannot fill it.
    pub fn begin_context(&self) -> u64 {
        let serial = self.context_serial.get().wrapping_add(1);
        self.context_serial.set(serial);
        crate::app::clear(&self.slate_context);
        self.slate_context.set_visible(false);
        serial
    }
    pub fn context_card(&self, serial: u64) -> Option<&gtk::Box> {
        (self.context_serial.get() == serial).then_some(&self.slate_context)
    }
    pub fn update_session(&self, session: &serde_json::Value) {
        use crate::app::text;
        self.caption.set_text(text(session, "name"));
        let pair = text(session, "pair_with");
        let intent = text(session, "intent").trim();
        self.metadata.set_text(
            &format!(
                "{} · {}{}",
                text(session, "provider"),
                text(session, "role"),
                if !intent.is_empty() {
                    format!(" · {intent}")
                } else if pair.is_empty() {
                    String::new()
                } else {
                    format!(" · pair {pair}")
                }
            )
            .to_uppercase(),
        );
        self.branch.set_text(text(session, "branch"));
        let lane_key = if text(session, "worktree").is_empty() {
            text(session, "branch")
        } else {
            text(session, "worktree")
        };
        let hash = lane_key.encode_utf16().fold(0_i32, |hash, unit| {
            hash.wrapping_mul(31).wrapping_add(unit as i32)
        });
        let color = gtk::gdk::RGBA::parse(
            ["#4f8fd9", "#c9a13b", "#a56cd6", "#3fb0a5", "#d97a4f"]
                [hash.unsigned_abs() as usize % 5],
        )
        .unwrap();
        self.lane.set_draw_func(move |_, cr, width, height| {
            cr.set_source_rgb(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
            );
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.fill();
        });
        self.header.set_tooltip_text(Some(&format!(
            "{}\n{}\n{}",
            text(session, "branch"),
            text(session, "worktree"),
            text(session, "intent")
        )));
        let state = text(session, "state");
        let state_label = match state {
            "running" => String::from("LIVE"),
            "blocked" => String::from("NEEDS YOU"),
            _ => state.to_uppercase(),
        };
        self.state.set_text(&state_label);
        let live = matches!(state, "spawning" | "running" | "idle" | "blocked");
        self.slate.set_visible(!live);
        self.slate_state.set_text(&state_label);
        self.slate_hint.set_text(match state {
            "parked" => "Process released. Scrollback and worktree kept; wake respawns with provider resume.",
            "restorable" => "The previous process ended. Resume restores scrollback and the worktree.",
            "created" => "The launch was interrupted before the provider started.",
            _ => "No signal from this session.",
        });
        for class in ["live", "held", "waiting", "off"] {
            self.root.remove_css_class(class);
            self.lamp.remove_css_class(class);
        }
        let class = match state {
            "running" | "spawning" => "live",
            "blocked" => "held",
            "restorable" => "waiting",
            _ => "off",
        };
        self.root.add_css_class(class);
        self.lamp.add_css_class(class);
    }

    pub fn set_active(self: &Rc<Self>, active: bool) {
        if self.active.replace(active) == active {
            return;
        }
        if !active {
            if let Some(task) = self.stream.borrow_mut().take() {
                task.abort();
            }
            self.client.borrow_mut().take(); // socket EOF releases only this attachment
            self.status.set_text("");
            return;
        }
        let weak = Rc::downgrade(self);
        *self.stream.borrow_mut() = Some(glib::spawn_future_local(async move {
            let Some(p) = weak.upgrade() else {
                return;
            };
            loop {
                let result = p.attach_and_render().await;
                p.client.borrow_mut().take();
                match result {
                    Ok(()) => {
                        p.status.set_text("Recovering output");
                    }
                    Err(e) => {
                        p.status.set_text(&e.to_string());
                        break;
                    }
                }
            }
        }));
    }

    async fn attach_and_render(self: &Rc<Self>) -> Result<(), Error> {
        let (client, rx) = Client::connect(&self.rt, self.path.clone()).await?;
        let last = self.sequence.borrow().last;
        self.sequence.borrow_mut().catch_up = true;
        client
            .request(
                &self.rt,
                "session.attach",
                json!({"session": self.name,
            "epoch":last.map(|v|v.0), "from_seq":last.map(|v|v.1)}),
            )
            .await?;
        *self.client.borrow_mut() = Some(client);
        self.size.set((0, 0));
        self.schedule_resize();
        self.status.set_text("");
        let mut bytes = 0;
        while let Ok(notice) = rx.recv().await {
            match notice {
                Notice::Frame(frame)
                    if frame.stream == "pty" && frame.session.as_deref() == Some(&self.name) =>
                {
                    let epoch = frame
                        .epoch
                        .ok_or_else(|| Error::Protocol("PTY epoch missing".into()))?;
                    let data = base64::engine::general_purpose::STANDARD
                        .decode(
                            frame
                                .data
                                .as_str()
                                .ok_or_else(|| Error::Protocol("PTY data missing".into()))?,
                        )
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                    let step = self.sequence.borrow_mut().observe(epoch, frame.seq);
                    match step {
                        Step::Duplicate => continue,
                        Step::Gap => return Ok(()), // reconnect from the last byte range fed
                        Step::Reset => self.terminal.reset(true, true),
                        Step::Feed => {}
                    }
                    self.terminal.feed(&data);
                    bytes += data.len();
                    // Bound each GLib dispatch. This is output-driven, never an idle timer.
                    if bytes >= 64 * 1024 {
                        let (send, recv) = async_channel::bounded(1);
                        glib::idle_add_local_once(move || {
                            let _ = send.try_send(());
                        });
                        let _ = recv.recv().await;
                        bytes = 0;
                    }
                }
                Notice::Disconnected(e) => return Err(e),
                _ => {}
            }
        }
        Err(Error::Disconnected)
    }

    pub fn schedule_resize(self: &Rc<Self>) {
        if self.resize_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(p) = weak.upgrade() else {
                return;
            };
            p.resize_pending.set(false);
            p.metadata.set_visible(p.root.width() > 420);
            p.branch_row.set_visible(p.root.width() > 600);
            if !p.active.get() || !p.terminal.is_mapped() {
                return;
            }
            let size = (
                p.terminal.column_count().clamp(2, 500) as u16,
                p.terminal.row_count().clamp(2, 300) as u16,
            );
            if p.size.get() == size {
                return;
            }
            let client = p.client.borrow().clone();
            if let Some(client) = client {
                p.size.set(size);
                glib::spawn_future_local(async move {
                    if let Err(e) = client
                        .request(
                            &p.rt,
                            "session.resize",
                            json!({"session":p.name,"cols":size.0,"rows":size.1}),
                        )
                        .await
                    {
                        p.size.set((0, 0));
                        p.status.set_text(&e.to_string());
                    }
                });
            }
        });
    }

    pub fn stop(&self) {
        self.active.set(false);
        if let Some(task) = self.stream.borrow_mut().take() {
            task.abort();
        }
        if let Some(task) = self.input.borrow_mut().take() {
            task.abort();
        }
        self.client.borrow_mut().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catchup_duplicates_gaps_and_respawn_are_distinct() {
        let mut s = Sequence {
            catch_up: true,
            ..Default::default()
        };
        assert_eq!(s.observe(1, 12), Step::Feed);
        assert_eq!(s.observe(1, 12), Step::Duplicate);
        assert_eq!(s.observe(1, 14), Step::Gap);
        assert_eq!(s.last, Some((1, 12)));
        s.catch_up = true;
        assert_eq!(s.observe(1, 20), Step::Feed);
        assert_eq!(s.observe(1, 21), Step::Feed);
        assert_eq!(s.observe(2, 0), Step::Reset);
        assert_eq!(s.observe(1, 90), Step::Duplicate);
    }
}
