use super::*;
use std::cell::Cell;

pub struct NotesWindow {
    pub window: gtk::Window,
    pub project: Cell<i64>,
    pub rendered_project: Cell<i64>,
    pub rail_width: Cell<i32>,
    pub rail_collapsed: Cell<bool>,
    loading: Cell<bool>,
    pending: Cell<bool>,
    restored: Cell<bool>,
    persist_timer: RefCell<Option<glib::SourceId>>,
}
impl NotesWindow {
    fn new(ui: &Rc<Ui>) -> Rc<Self> {
        let window = gtk::Window::builder()
            .application(&ui.window.application().unwrap())
            .title("Notes · Relay")
            .default_width(1080)
            .default_height(760)
            .build();
        window.add_css_class("notes-window");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let chrome = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        chrome.add_css_class("notes-window-chrome");
        let title = label("NOTES", "brand");
        title.set_hexpand(true);
        title.set_margin_start(12);
        chrome.append(&title);
        for (icon, hint) in [
            ("minimize", "Minimize"),
            ("maximize", "Maximize"),
            ("close", "Close Notes"),
        ] {
            let key = crate::app::icon_button(icon, hint);
            let target = window.downgrade();
            key.connect_clicked(move |_| {
                if let Some(window) = target.upgrade() {
                    match icon {
                        "minimize" => window.minimize(),
                        "maximize" => {
                            if window.is_maximized() {
                                window.unmaximize();
                            } else {
                                window.maximize();
                            }
                        }
                        _ => window.close(),
                    }
                }
            });
            chrome.append(&key);
        }
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&chrome));
        // Keep GTK's resize edges while drawing our own compact titlebar.
        window.set_titlebar(Some(&handle));
        let content = &ui.pages["notes"];
        content.set_vexpand(true);
        content.append(&label("Opening Notes…", "notes-window-loading"));
        root.append(content);
        window.set_child(Some(&root));
        let owned = Rc::new(Self {
            window,
            project: Cell::new(0),
            rendered_project: Cell::new(0),
            rail_width: Cell::new(270),
            rail_collapsed: Cell::new(false),
            loading: Cell::new(false),
            pending: Cell::new(false),
            restored: Cell::new(false),
            persist_timer: RefCell::new(None),
        });
        let weak = Rc::downgrade(ui);
        owned.window.connect_close_request(move |window| {
            window.set_visible(false);
            if let Some(ui) = weak.upgrade() {
                if let Some(owner) = ui.notes_window.borrow().as_ref() {
                    owner.persist(&ui);
                }
            }
            glib::Propagation::Stop
        });
        owned
    }
    pub fn persist(&self, ui: &Rc<Ui>) {
        if let Some(timer) = self.persist_timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = Rc::downgrade(ui);
        *self.persist_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(300),
            move || {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let value = {
                    let owner = ui.notes_window.borrow();
                    let Some(owner) = owner.as_ref() else {
                        return;
                    };
                    owner.persist_timer.borrow_mut().take();
                    json!({"rail_width":owner.rail_width.get(),"rail_collapsed":owner.rail_collapsed.get()})
                };
                glib::spawn_future_local(async move {
                    let _ = ui
                        .call(
                            "settings.set",
                            json!({"path":"native.notes.window","value":value}),
                        )
                        .await;
                });
            },
        ));
    }
}
pub fn show_notes(ui: &Rc<Ui>) {
    show_project(ui, ui.project.get());
}
pub fn show_project(ui: &Rc<Ui>, project: i64) {
    if ui.notes_window.borrow().is_none() {
        let window = NotesWindow::new(ui);
        *ui.notes_window.borrow_mut() = Some(window);
    }
    let window = ui.notes_window.borrow().as_ref().unwrap().clone();
    if project != 0 {
        window.project.set(project);
    }
    window.window.present();
    refresh_notes(ui);
}
pub fn refresh_notes(ui: &Rc<Ui>) {
    let Some(window) = ui.notes_window.borrow().clone() else {
        return;
    };
    if window.project.get() == 0 {
        window.project.set(ui.project.get());
    }
    if window.project.get() == 0 {
        return;
    }
    window.pending.set(true);
    if window.loading.replace(true) {
        return;
    }
    let weak = Rc::downgrade(ui);
    glib::spawn_future_local(async move {
        let Some(ui) = weak.upgrade() else {
            window.loading.set(false);
            return;
        };
        if !window.restored.replace(true) {
            if let Ok(value) = ui
                .call("settings.get", json!({"path":"native.notes.window"}))
                .await
            {
                if let Some(width) = value["value"]["rail_width"].as_i64() {
                    window.rail_width.set(width.clamp(180, 600) as i32);
                }
                window
                    .rail_collapsed
                    .set(value["value"]["rail_collapsed"].as_bool().unwrap_or(false));
            }
        }
        while window.pending.replace(false) {
            let project = window.project.get();
            match ui.call("notes.list", json!({"project_id":project})).await {
                Ok(value) if project == window.project.get() => {
                    super::note_pages::workspace(&ui, "notes", project, &rows(&value, "notes"))
                }
                Ok(_) => window.pending.set(true),
                Err(error) => {
                    if window.rendered_project.get() == 0 {
                        clear(&ui.pages["notes"]);
                        ui.pages["notes"].append(&label(&error.to_string(), "error"));
                    } else {
                        ui.show_error(&error.to_string());
                    }
                }
            }
        }
        window.loading.set(false);
    });
}
