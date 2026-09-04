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
        terminal.set_font(Some(&gtk::pango::FontDescription::from_string(
            "Fira Mono 10",
        )));
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
        caption.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        caption.add_css_class("session-name");
        let status = gtk::Label::new(Some("Connecting"));
        status.add_css_class("dim");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        footer.append(&caption);
        let metadata = gtk::Label::new(None);
        metadata.add_css_class("dim");
        metadata.set_ellipsize(gtk::pango::EllipsizeMode::End);

        metadata.set_xalign(0.0);
        footer.append(&metadata);
        let branch = gtk::Label::new(None);
        branch.add_css_class("session-branch");
        branch.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        branch.set_max_width_chars(24);
        footer.append(&branch);
        let state = gtk::Label::new(None);
        state.add_css_class("session-state");
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);
        footer.append(&state);
        footer.append(&actions);
        root.append(&footer);
        root.append(&terminal);
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
        self.terminal
            .set_color_background(&gtk::gdk::RGBA::parse(background).unwrap());
        self.terminal
            .set_font(Some(&gtk::pango::FontDescription::from_string(&format!(
                "Fira Mono {}",
                points.clamp(8.0, 24.0)
            ))));
    }
    pub fn verify_ready(&self) {
        assert!(
            self.active.get() && self.client.borrow().is_some(),
            "{} has no ready terminal attachment (active={})",
            self.name,
            self.active.get()
        );
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn update_session(&self, session: &serde_json::Value) {
        use crate::app::text;
        self.caption.set_text(text(session, "name"));
        self.metadata.set_text(&format!(
            "{} · {}",
            text(session, "provider"),
            text(session, "role")
        ));
        self.branch.set_text(text(session, "branch"));
        self.header.set_tooltip_text(Some(&format!(
            "{}\n{}\n{}",
            text(session, "branch"),
            text(session, "worktree"),
            text(session, "intent")
        )));
        let state = text(session, "state");
        self.state.set_text(&state.to_uppercase());
        for class in ["live", "held", "waiting"] {
            self.root.remove_css_class(class);
            self.lamp.remove_css_class(class);
        }
        let class = match state {
            "running" | "spawning" => "live",
            "blocked" => "held",
            _ => "waiting",
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
            p.branch.set_visible(p.root.width() > 600);
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
