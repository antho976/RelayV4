//! App-owned surfaces share the main window; only Notes and the device mirror detach.
use crate::app::{button, clear, icon_button, label, scrolled, Ui};
use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub struct Panel {
    pub body: gtk::Box,
    layer: gtk::Overlay,
    frame: gtk::Box,
    host: gtk::Overlay,
    panels: Weak<RefCell<Vec<Rc<Panel>>>>,
    guard: RefCell<Option<Box<dyn Fn() -> bool>>>,
    closed: RefCell<Vec<Box<dyn Fn()>>>,
    focus: Option<gtk::Widget>,
}

impl Panel {
    pub fn new(ui: &Ui, title: &str, width: i32) -> Rc<Self> {
        let layer = gtk::Overlay::new();
        let scrim = button("", "panel-scrim");
        scrim.set_focusable(false);
        scrim.update_property(&[gtk::accessible::Property::Label("Close panel")]);
        layer.set_child(Some(&scrim));
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.add_css_class("app-panel");
        frame.set_widget_name("app-panel");
        frame.set_halign(gtk::Align::End);
        frame.set_size_request(width.min(780), -1);
        frame.set_focusable(true);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        heading.add_css_class("panel-heading");
        let title = label(title, "section-title");
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        heading.append(&title);
        let close = icon_button("close", "Close panel");
        close.set_widget_name("panel-close");
        heading.append(&close);
        frame.append(&heading);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.add_css_class("panel-body");
        frame.append(&scrolled(&body));
        layer.add_overlay(&frame);
        let panel = Rc::new(Self {
            body,
            layer,
            frame,
            host: ui.overlay.clone(),
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
        panel
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
            if let Some(content) = self.host.child() {
                content.set_sensitive(false);
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
            } else if let Some(content) = self.host.child() {
                content.set_sensitive(true);
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
        self.present();
        cancel.grab_focus();
        let accepted = recv.recv().await.unwrap_or(false);
        self.close();
        accepted
    }
}
