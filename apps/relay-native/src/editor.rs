use crate::app::{button, clear, label, rows, scrolled, text, Ui};
use gtk4 as gtk;
use serde_json::json;
use sourceview5::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Editor {
    pub root: gtk::Box,
    tree: gtk::Box,
    buffer: sourceview5::Buffer,
    view: sourceview5::View,
    caption: gtk::Label,
    path: RefCell<String>,
    original: RefCell<String>,
    project: Cell<i64>,
    revision: Cell<u64>,
    directory: RefCell<String>,
    save: gtk::Button,
    discard: gtk::Button,
    handlers: Cell<bool>,
    busy: Cell<bool>,
    tree_revision: Cell<u64>,
}
impl Editor {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        tools.add_css_class("toolbar");
        let caption = label("Code · project checkout", "title");
        caption.set_hexpand(true);
        tools.append(&caption);
        let save = button("Save", "primary");
        let discard = button("Discard changes", "quiet");
        tools.append(&discard);
        tools.append(&save);
        root.append(&tools);
        let split = gtk::Paned::new(gtk::Orientation::Horizontal);
        split.set_position(240);
        split.set_vexpand(true);
        let tree = gtk::Box::new(gtk::Orientation::Vertical, 2);
        tree.add_css_class("file-tree");
        split.set_start_child(Some(&scrolled(&tree)));
        let buffer = sourceview5::Buffer::new(None);
        let view = sourceview5::View::with_buffer(&buffer);
        view.set_monospace(true);
        view.set_show_line_numbers(true);
        view.set_highlight_current_line(true);
        view.set_tab_width(4);
        view.set_auto_indent(true);
        view.set_editable(false);
        if let Some(scheme) = sourceview5::StyleSchemeManager::default().scheme("Adwaita-dark") {
            buffer.set_style_scheme(Some(&scheme));
        }
        split.set_end_child(Some(&scrolled(&view)));
        root.append(&split);
        save.set_sensitive(false);
        discard.set_sensitive(false);
        let editor = Rc::new(Self {
            root,
            tree,
            buffer,
            view,
            caption,
            path: RefCell::default(),
            original: RefCell::default(),
            project: Cell::new(0),
            revision: Cell::new(0),
            directory: RefCell::default(),
            save,
            discard,
            handlers: Cell::new(false),
            busy: Cell::new(false),
            tree_revision: Cell::new(0),
        });
        let weak = Rc::downgrade(&editor);
        editor.buffer.connect_modified_changed(move |b| {
            if let Some(e) = weak.upgrade() {
                e.save
                    .set_sensitive(b.is_modified() && e.view.is_editable());
                e.discard.set_sensitive(b.is_modified() && !e.busy.get());
            }
        });
        editor
    }
    pub fn is_dirty(&self) -> bool {
        self.buffer.is_modified() || self.busy.get()
    }
    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.view
            .set_editable(!busy && !self.path.borrow().is_empty());
        self.save.set_sensitive(!busy && self.buffer.is_modified());
        self.discard
            .set_sensitive(!busy && self.buffer.is_modified());
    }
    pub fn reset(&self) {
        self.revision.set(self.revision.get() + 1);
        self.tree_revision.set(self.tree_revision.get() + 1);
        self.buffer.set_text("");
        self.buffer.set_modified(false);
        self.path.borrow_mut().clear();
        self.original.borrow_mut().clear();
        self.directory.borrow_mut().clear();
        self.view.set_editable(false);
        self.set_busy(false);
        clear(&self.tree);
        self.caption.set_text("Code · project checkout");
    }
    pub fn invalidate(self: &Rc<Self>, ui: &Rc<Ui>) {
        if *ui.page.borrow() == "code" {
            self.load_tree(ui, Some(self.directory.borrow().clone()));
        }
        if !self.path.borrow().is_empty() {
            ui.show_error("Files changed on disk. Save checks for conflicting edits; discard reloads the file.");
        }
    }
    pub fn load_tree(self: &Rc<Self>, ui: &Rc<Ui>, directory: Option<String>) {
        if ui.project.get() == 0 {
            return;
        }
        if !self.handlers.replace(true) {
            let weak = Rc::downgrade(ui);
            let editor = self.clone();
            self.save.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    editor.save_file(&ui);
                }
            });
            let weak = Rc::downgrade(ui);
            let editor = self.clone();
            self.discard.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if editor.busy.get() {
                        return;
                    }
                    let path = editor.path.borrow().clone();
                    editor.read_file(&ui, path, true);
                }
            });
        }
        let project = ui.project.get();
        self.tree_revision.set(self.tree_revision.get() + 1);
        let revision = self.tree_revision.get();
        let directory = directory.unwrap_or_default();
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = ui
                .call(
                    "file.tree",
                    json!({"project_id":project,"path":directory,"depth":1,"git_badges":false}),
                )
                .await;
            if ui.project.get() != project || e.tree_revision.get() != revision {
                return;
            }
            match result {
                Ok(v) => {
                    *e.directory.borrow_mut() = directory.clone();
                    clear(&e.tree);
                    if !directory.is_empty() {
                        let b = button("Parent folder", "quiet");
                        let parent = std::path::Path::new(&directory)
                            .parent()
                            .unwrap_or(std::path::Path::new(""))
                            .to_string_lossy()
                            .to_string();
                        let weak = Rc::downgrade(&ui);
                        let ed = e.clone();
                        b.connect_clicked(move |_| {
                            if let Some(ui) = weak.upgrade() {
                                ed.load_tree(&ui, Some(parent.clone()));
                            }
                        });
                        e.tree.append(&b);
                    }
                    for entry in rows(&v, "entries") {
                        let path = text(&entry, "path").to_string();
                        let dir = text(&entry, "kind") == "dir";
                        let b = button(
                            &format!("{}{}", text(&entry, "name"), if dir { "/" } else { "" }),
                            "file",
                        );
                        let weak = Rc::downgrade(&ui);
                        let ed = e.clone();
                        b.connect_clicked(move |_| {
                            if let Some(ui) = weak.upgrade() {
                                if dir {
                                    ed.load_tree(&ui, Some(path.clone()));
                                } else {
                                    ed.open(&ui, path.clone());
                                }
                            }
                        });
                        e.tree.append(&b);
                    }
                }
                Err(err) => ui.show_error(&err.to_string()),
            }
        });
    }
    fn open(self: &Rc<Self>, ui: &Rc<Ui>, path: String) {
        self.read_file(ui, path, false);
    }
    pub fn verify_open(self: &Rc<Self>, ui: &Rc<Ui>) {
        self.open(ui, "README.md".into());
        assert!(self.busy.get());
        assert!(
            !self.view.is_editable() && !self.save.is_sensitive() && !self.discard.is_sensitive()
        );
    }
    pub fn verify_save(self: &Rc<Self>, ui: &Rc<Ui>) {
        assert!(!self.busy.get());
        assert_eq!(self.path.borrow().as_str(), "README.md");
        self.buffer
            .insert_at_cursor("\nNative editor save verified.\n");
        self.save_file(ui);
        assert!(self.busy.get());
        assert!(
            !self.view.is_editable() && !self.save.is_sensitive() && !self.discard.is_sensitive()
        );
    }
    fn read_file(self: &Rc<Self>, ui: &Rc<Ui>, path: String, discard: bool) {
        if self.busy.get() || (self.buffer.is_modified() && !discard) {
            ui.show_error("Save or discard this file's changes before opening another file.");
            return;
        }
        self.revision.set(self.revision.get() + 1);
        self.set_busy(true);
        let revision = self.revision.get();
        let project = ui.project.get();
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = ui
                .call(
                    "file.read",
                    json!({"project_id":project,"path":path,"max_bytes":1048576}),
                )
                .await;
            if e.revision.get() != revision {
                return;
            }
            e.set_busy(false);
            if ui.project.get() != project {
                return;
            }
            match result {
                Ok(v) => {
                    if v["truncated"] == true || v["text"].as_str().is_none() {
                        ui.show_error("This file is binary or exceeds the 1 MiB editor limit. It was not opened for editing.");
                        return;
                    }
                    let content = text(&v, "text");
                    e.buffer.set_text(content);
                    e.buffer.set_modified(false);
                    e.project.set(project);
                    *e.path.borrow_mut() = path.clone();
                    *e.original.borrow_mut() = content.into();
                    e.set_busy(false);
                    let manager = sourceview5::LanguageManager::default();
                    e.buffer
                        .set_language(manager.guess_language(Some(&path), None).as_ref());
                    e.caption.set_text(&path);
                }
                Err(err) => ui.show_error(&err.to_string()),
            }
        });
    }
    fn save_file(self: &Rc<Self>, ui: &Rc<Ui>) {
        let project = self.project.get();
        if project != ui.project.get() || !self.buffer.is_modified() || self.busy.get() {
            return;
        }
        let path = self.path.borrow().clone();
        let original = self.original.borrow().clone();
        let content = self
            .buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), false)
            .to_string();
        let e = self.clone();
        let ui = ui.clone();
        self.revision.set(self.revision.get() + 1);
        let revision = self.revision.get();
        self.set_busy(true);
        glib::spawn_future_local(async move {
            let result = async {
                let disk = ui
                    .call(
                        "file.read",
                        json!({"project_id":project,"path":path,"max_bytes":1048576}),
                    )
                    .await?;
                if disk["truncated"] == true || disk["text"].as_str() != Some(original.as_str()) {
                    return Err(crate::client::Error::Io(
                        "File changed on disk. Copy your edits before discarding and reloading it."
                            .into(),
                    ));
                }
                ui.call(
                    "file.write",
                    json!({"project_id":project,"path":path,"text":content}),
                )
                .await
            }
            .await;
            if e.revision.get() != revision {
                return;
            }
            e.set_busy(false);
            if ui.project.get() != project {
                return;
            }
            match result {
                Ok(_) => {
                    *e.original.borrow_mut() = content;
                    e.buffer.set_modified(false);
                }
                Err(err) => {
                    ui.show_error(&err.to_string());
                    e.save.set_sensitive(true);
                }
            }
        });
    }
}
