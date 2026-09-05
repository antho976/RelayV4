//! App-owned surfaces share the main window; only Notes and the device mirror detach.
use crate::app::{button, clear, icon_button, label, scrolled, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub struct Panel {
    pub body: gtk::Box,
    title: String,
    modal: Cell<bool>,
    layer: gtk::Overlay,
    frame: gtk::Box,
    scroll: gtk::ScrolledWindow,
    host: gtk::Overlay,
    panels: Weak<RefCell<Vec<Rc<Panel>>>>,
    guard: RefCell<Option<Box<dyn Fn() -> bool>>>,
    closed: RefCell<Vec<Box<dyn Fn()>>>,
    focus: Option<gtk::Widget>,
}

impl Panel {
    pub fn new(ui: &Ui, title: &str, width: i32) -> Rc<Self> {
        Self::build(ui, title, width, &ui.overlay, false)
    }

    /// Repeated activation closes the current surface before any work is started.
    pub fn toggle(ui: &Ui, title: &str, width: i32) -> Option<Rc<Self>> {
        let open = ui.panels.borrow().iter().any(|panel| panel.title == title);
        if !ui.dismiss_panels() || open {
            return None;
        }
        Some(Self::new(ui, title, width))
    }

    pub fn page(ui: &Ui, title: &str) -> Rc<Self> {
        Self::build(ui, title, -1, &ui.page_overlay, true)
    }

    fn build(ui: &Ui, title: &str, width: i32, host: &gtk::Overlay, page: bool) -> Rc<Self> {
        let layer = gtk::Overlay::new();
        let scrim = button("", "panel-scrim");
        scrim.set_focusable(false);
        scrim.update_property(&[gtk::accessible::Property::Label("Close panel")]);
        layer.set_child(Some(&scrim));
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.add_css_class("app-panel");
        frame.set_widget_name("app-panel");
        frame.set_halign(if page {
            gtk::Align::Fill
        } else {
            gtk::Align::End
        });
        frame.set_size_request(width, -1);
        frame.set_focusable(true);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        heading.add_css_class("panel-heading");
        let title_text = title.to_string();
        let title = label(title, "section-title");
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let close = icon_button(
            if page { "arrow-left" } else { "close" },
            if page { "Back" } else { "Close panel" },
        );
        if page {
            heading.append(&close);
            frame.add_css_class("detail-page");
        }
        heading.append(&title);
        close.set_widget_name("panel-close");
        if !page {
            heading.append(&close);
        }
        frame.append(&heading);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.add_css_class("panel-body");
        let scroll = scrolled(&body);
        frame.append(&scroll);
        layer.add_overlay(&frame);
        let panel = Rc::new(Self {
            body,
            title: title_text,
            modal: Cell::new(true),
            layer,
            frame,
            scroll,
            host: host.clone(),
            panels: Rc::downgrade(&ui.panels),
            guard: RefCell::default(),
            closed: RefCell::default(),
            focus: gtk::prelude::GtkWindowExt::focus(&ui.window),
        });
        for key in [close, scrim] {
            let weak = Rc::downgrade(&panel);
            key.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.close();
                }
            });
        }
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&panel);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if matches!(key, gtk::gdk::Key::Tab | gtk::gdk::Key::ISO_Left_Tab) {
                if let Some(panel) = weak.upgrade() {
                    let direction = if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                        gtk::DirectionType::TabBackward
                    } else {
                        gtk::DirectionType::TabForward
                    };
                    if !panel.frame.child_focus(direction) {
                        panel.frame.grab_focus();
                        panel.frame.child_focus(direction);
                    }
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        panel.layer.add_controller(keys);
        panel
    }

    pub fn compact(&self, centered: bool, height: i32) {
        self.frame.add_css_class("compact-panel");
        self.frame.set_valign(gtk::Align::Start);
        self.frame.set_halign(if centered {
            gtk::Align::Center
        } else {
            gtk::Align::End
        });
        self.frame.set_margin_top(if centered {
            self.host.height() * 12 / 100
        } else {
            0
        });
        self.frame.set_margin_end(if centered { 0 } else { 126 });
        self.scroll
            .set_min_content_height(height.min((self.host.height() - 160).max(200)));
    }

    pub fn centered(&self, height: i32) {
        self.frame.set_halign(gtk::Align::Center);
        self.frame.set_valign(gtk::Align::Center);
        self.scroll.set_vexpand(false);
        self.scroll
            .set_max_content_height(height.min((self.host.height() - 48).max(200)));
        self.scroll.set_propagate_natural_height(true);
    }

    pub fn header_action(&self, widget: &impl IsA<gtk::Widget>) {
        if let Some(heading) = self.frame.first_child().and_downcast::<gtk::Box>() {
            let previous = heading.last_child().and_then(|close| close.prev_sibling());
            heading.insert_child_after(widget, previous.as_ref());
        }
    }

    pub fn top(&self, height: i32) {
        self.modal.set(false);
        self.layer.add_css_class("utility-layer");
        self.frame.add_css_class("utility-panel");
        self.frame.set_valign(gtk::Align::Start);
        self.frame.set_margin_end(8);
        self.scroll.set_vexpand(false);
        self.scroll
            .set_max_content_height(height.min((self.host.height() - 16).max(200)));
        self.scroll.set_propagate_natural_height(true);
    }

    pub fn bottom(&self, height: i32) {
        self.modal.set(false);
        self.layer.add_css_class("utility-layer");
        self.frame.add_css_class("utility-panel");
        self.frame.set_valign(gtk::Align::End);
        self.frame.set_margin_end(8);
        let footer = self
            .host
            .next_sibling()
            .filter(|widget| widget.is_visible())
            .map(|widget| widget.height())
            .unwrap_or(0);
        self.frame.set_margin_bottom((30 - footer).max(0));
        self.scroll.set_vexpand(false);
        self.scroll
            .set_min_content_height(height.min((self.host.height() - 100).max(200)));
        self.scroll
            .set_max_content_height((self.host.height() - 70).max(200));
        self.scroll.set_propagate_natural_height(true);
    }

    pub fn add_css_class(&self, name: &str) {
        self.frame.add_css_class(name);
    }

    pub fn set_guard(&self, guard: impl Fn() -> bool + 'static) {
        *self.guard.borrow_mut() = Some(Box::new(guard));
    }

    pub fn can_close(&self) -> bool {
        self.guard.borrow().as_ref().is_none_or(|guard| guard())
    }

    pub fn on_closed(&self, callback: impl Fn() + 'static) {
        self.closed.borrow_mut().push(Box::new(callback));
    }

    pub fn present(self: &Rc<Self>) {
        if self.layer.parent().is_none() {
            let Some(panels) = self.panels.upgrade() else {
                return;
            };
            if let Some(previous) = panels.borrow().last() {
                previous.layer.set_sensitive(false);
            }
            if self.modal.get() {
                if let Some(content) = self.host.child() {
                    content.set_sensitive(false);
                }
            }
            self.host.add_overlay(&self.layer);
            panels.borrow_mut().push(self.clone());
        }
        self.frame.child_focus(gtk::DirectionType::TabForward);
    }

    pub fn close(&self) {
        if !self.can_close() || self.layer.parent().is_none() {
            return;
        }
        self.host.remove_overlay(&self.layer);
        if let Some(panels) = self.panels.upgrade() {
            panels.borrow_mut().retain(|p| p.layer != self.layer);
            if let Some(previous) = panels.borrow().last() {
                previous.layer.set_sensitive(true);
            }
            if !panels
                .borrow()
                .iter()
                .any(|panel| panel.host == self.host && panel.modal.get())
            {
                if let Some(content) = self.host.child() {
                    content.set_sensitive(true);
                }
            }
        }
        for callback in self.closed.borrow_mut().drain(..) {
            callback();
        }
        clear(&self.body);
        if let Some(focus) = &self.focus {
            if focus.is_sensitive() && focus.is_mapped() {
                focus.grab_focus();
            }
        }
    }

    pub async fn response(self: &Rc<Self>, caption: &str) -> bool {
        let (send, recv) = async_channel::bounded(1);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        let cancel = button("Cancel", "quiet");
        let accept = button(caption, "primary");
        accept.set_widget_name("panel-accept");
        actions.append(&cancel);
        actions.append(&accept);
        self.body.append(&actions);
        let weak = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.close();
            }
        });
        let accepted = send.clone();
        accept.connect_clicked(move |_| {
            let _ = accepted.try_send(true);
        });
        self.on_closed(move || {
            let _ = send.try_send(false);
        });
        self.compact(true, 280);
        self.present();
        cancel.grab_focus();
        let accepted = recv.recv().await.unwrap_or(false);
        self.close();
        accepted
    }
}
