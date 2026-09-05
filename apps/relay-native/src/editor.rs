use crate::app::{Ui, button, clear, label, rows, scrolled, text};
use gtk4 as gtk;
use serde_json::{Value, json};
#[path = "code_git.rs"]
mod code_git;
#[path = "project_files.rs"]
mod project_files;
use sourceview5::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Editor {
    pub root: gtk::Box,
    tree: gtk::Box,
    expanded: RefCell<std::collections::BTreeSet<String>>,
    scope: gtk::MenuButton,
    scope_label: gtk::Label,
    file_sidebar: gtk::Box,
    files_split: gtk::Paned,
    git_split: gtk::Paned,
    document_tools: gtk::Box,
    selected_path: RefCell<String>,
    selected_directory: Cell<bool>,
    worktree: RefCell<String>,
    worktrees: RefCell<Vec<Value>>,

    scope_project: Cell<i64>,
    scope_revision: Cell<u64>,
    search: gtk::SearchEntry,
    git: gtk::Box,
    git_revision: Cell<u64>,
    git_busy: Cell<bool>,
    commit_message: gtk::TextView,
    branch_name: gtk::Entry,
    branch_start: gtk::Entry,
    invalidate_pending: Cell<bool>,
    diff: Cell<bool>,
    before: sourceview5::Buffer,
    before_scroll: gtk::ScrolledWindow,
    content_stack: gtk::Stack,
    position: gtk::Label,
    find_bar: gtk::Box,
    find: gtk::Entry,
    replacement: gtk::Entry,
    search_context: sourceview5::SearchContext,
    file_actions: gtk::Box,
    file_undo: gtk::Box,
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
    pub(crate) tree_revision: Cell<u64>,
}
impl Editor {
    pub fn set_palette(&self, mode: &str) {
        let manager = sourceview5::StyleSchemeManager::default();
        if let Some(scheme) = manager
            .scheme(&format!("relay-{mode}"))
            .or_else(|| manager.scheme("relay-matte"))
        {
            self.buffer.set_style_scheme(Some(&scheme));
            self.before.set_style_scheme(Some(&scheme));
        }
    }

    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tools.add_css_class("toolbar");
        tools.add_css_class("code-bar");
        let scope = gtk::MenuButton::new();
        scope.set_widget_name("project-scope");
        scope.set_valign(gtk::Align::Center);
        scope.set_tooltip_text(Some("Search projects and checkouts"));
        let scope_label = label("Select a project", "mono");
        scope_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        scope_label.set_max_width_chars(42);
        let scope_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        scope_content.append(&crate::icons::image("branch", 13));
        scope_content.append(&scope_label);
        scope_content.append(&crate::icons::image("chevron-down", 12));
        scope.set_child(Some(&scope_content));
        let project_tools = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        project_tools.add_css_class("code-bar");
        project_tools.append(&scope);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        project_tools.append(&spacer);
        let agents_button = button("Agents", "quiet");
        agents_button.set_widget_name("project-agents");
        project_tools.append(&agents_button);
        let document_button = button("Editor", "quiet");
        document_button.set_widget_name("project-editor");
        project_tools.append(&document_button);
        root.append(&project_tools);

        let caption = label("No file open", "title");
        caption.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        caption.set_hexpand(true);
        tools.append(&caption);
        let save = button("Save", "primary");
        save.set_widget_name("project-save");
        let discard = crate::app::icon_button("undo", "Discard changes");
        tools.append(&discard);
        tools.append(&save);

        let split = gtk::Paned::new(gtk::Orientation::Horizontal);
        split.set_position(260);
        split.set_resize_start_child(false);
        split.set_shrink_start_child(false);
        split.set_shrink_end_child(false);
        split.set_vexpand(true);
        let tree = gtk::Box::new(gtk::Orientation::Vertical, 2);
        tree.add_css_class("file-tree");
        let file_sidebar = gtk::Box::new(gtk::Orientation::Vertical, 6);
        file_sidebar.set_size_request(260, -1);
        file_sidebar.set_spacing(0);
        file_sidebar.add_css_class("code-files");
        let file_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        file_header.add_css_class("code-bar");
        file_header.append(&label("FILES", "section-label"));
        file_sidebar.append(&file_header);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search in worktree"));
        search.set_tooltip_text(Some(
            "Search file contents; press Enter to search, Escape to return to files",
        ));
        search.add_css_class("code-search");
        file_sidebar.append(&search);
        let file_actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        file_sidebar.append(&file_actions);
        let file_undo = gtk::Box::new(gtk::Orientation::Vertical, 2);
        file_sidebar.append(&file_undo);
        file_sidebar.append(&scrolled(&tree));
        split.set_start_child(Some(&file_sidebar));
        let buffer = sourceview5::Buffer::new(None);
        let view = sourceview5::View::with_buffer(&buffer);
        view.set_widget_name("project-source");
        view.set_monospace(true);
        view.add_css_class("code-source");
        view.set_show_line_numbers(true);
        view.set_highlight_current_line(true);
        view.set_tab_width(4);
        view.set_auto_indent(true);
        view.set_editable(false);
        if let Some(scheme) = sourceview5::StyleSchemeManager::default().scheme("relay-matte") {
            buffer.set_style_scheme(Some(&scheme));
        }
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&tools);
        let find_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        find_bar.add_css_class("toolbar");
        let find = gtk::Entry::new();
        find.set_placeholder_text(Some("Find in file"));
        find.set_hexpand(true);
        let replacement = gtk::Entry::new();
        replacement.set_placeholder_text(Some("Replace with"));
        replacement.set_hexpand(true);
        find_bar.append(&find);
        find_bar.append(&replacement);
        content.append(&find_bar);
        find_bar.set_visible(false);
        let search_settings = sourceview5::SearchSettings::new();
        search_settings.set_wrap_around(true);
        let search_context = sourceview5::SearchContext::new(&buffer, Some(&search_settings));
        let before = sourceview5::Buffer::new(None);
        before.set_style_scheme(buffer.style_scheme().as_ref());
        let before_view = sourceview5::View::with_buffer(&before);
        before_view.set_monospace(true);
        before_view.add_css_class("code-source");
        before_view.set_show_line_numbers(true);
        before_view.set_editable(false);
        let before_scroll = scrolled(&before_view);
        before_scroll.set_visible(false);
        let compare = gtk::Paned::new(gtk::Orientation::Horizontal);
        compare.set_start_child(Some(&before_scroll));
        compare.set_end_child(Some(&scrolled(&view)));
        compare.set_resize_start_child(true);
        compare.set_resize_end_child(true);
        compare.set_position(360);
        compare.set_vexpand(true);
        let content_stack = gtk::Stack::new();
        content_stack.set_hhomogeneous(false);
        content_stack.set_vhomogeneous(false);
        content_stack.set_vexpand(true);
        content_stack.add_named(&compare, Some("source"));
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 8);
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Start);
        empty.set_margin_top(38);
        empty.add_css_class("code-empty");
        let glyph = crate::icons::image("code", 28);
        glyph.set_halign(gtk::Align::Center);
        glyph.add_css_class("faint");
        empty.append(&glyph);
        let title = label("Code, review, integrate", "title");
        title.set_xalign(0.5);
        title.set_margin_top(12);
        empty.append(&title);
        let hint = label(
            "Open a file from the tree, a change from the git panel, or a commit from a task. Saves go through the bus; guardrails apply to your edits too.",
            "dim",
        );
        hint.set_wrap(true);
        hint.set_max_width_chars(58);
        hint.set_justify(gtk::Justification::Center);
        hint.set_xalign(0.5);
        empty.append(&hint);
        content_stack.add_named(&empty, Some("empty"));
        content_stack.set_visible_child_name("empty");
        content.append(&content_stack);
        let position = label("Select a file to edit", "dim");
        position.add_css_class("code-position");
        content.append(&position);
        let workspace = gtk::Paned::new(gtk::Orientation::Horizontal);
        workspace.set_start_child(Some(&content));
        workspace.set_resize_start_child(true);
        workspace.set_shrink_start_child(false);
        workspace.set_resize_end_child(false);
        workspace.set_position(760);
        let git = gtk::Box::new(gtk::Orientation::Vertical, 0);
        git.set_vexpand(true);
        git.set_size_request(320, -1);
        git.add_css_class("code-git");
        let git_panel = git.clone();
        workspace.set_shrink_end_child(false);
        workspace.set_end_child(Some(&git_panel));
        split.set_end_child(Some(&workspace));
        let find_button = crate::app::icon_button("search", "Find in file");
        tools.append(&find_button);
        let files_button = button("Files", "quiet");
        files_button.set_widget_name("project-files");
        project_tools.append(&files_button);
        let edit_button = crate::app::icon_button("edit", "Edit file");
        tools.append(&edit_button);
        let git_button = button("Git", "quiet");
        git_button.set_widget_name("project-git");
        project_tools.append(&git_button);
        let mut control = tools.first_child();
        while let Some(widget) = control {
            control = widget.next_sibling();
            widget.set_valign(gtk::Align::Center);
        }
        file_sidebar.set_visible(false);
        git.set_visible(false);
        root.append(&split);
        save.set_sensitive(false);
        discard.set_sensitive(false);
        let editor = Rc::new(Self {
            root,
            tree,
            expanded: RefCell::default(),
            scope,
            scope_label,
            file_sidebar,
            files_split: split,
            git_split: workspace,
            document_tools: tools,
            selected_path: RefCell::default(),
            selected_directory: Cell::new(false),
            worktree: RefCell::default(),
            worktrees: RefCell::default(),
            scope_project: Cell::new(0),
            scope_revision: Cell::new(0),
            search,
            git,
            git_revision: Cell::new(0),
            git_busy: Cell::new(false),
            commit_message: gtk::TextView::new(),
            branch_name: gtk::Entry::new(),
            branch_start: gtk::Entry::new(),
            invalidate_pending: Cell::new(false),
            diff: Cell::new(false),
            before,
            before_scroll,
            content_stack,
            position,
            find_bar,
            find,
            replacement,
            search_context,
            file_actions,
            file_undo,
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
        let weak = Rc::downgrade(&editor);
        find_button.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                e.find_bar.set_visible(!e.find_bar.is_visible());
                if e.find_bar.is_visible() {
                    e.find.grab_focus();
                }
            }
        });
        let weak = Rc::downgrade(&editor);
        editor.buffer.connect_cursor_position_notify(move |buffer| {
            if let Some(e) = weak.upgrade() {
                let iter = buffer.iter_at_offset(buffer.cursor_position());
                e.position.set_text(&format!(
                    "Ln {}, Col {}{}",
                    iter.line() + 1,
                    iter.line_offset() + 1,
                    if e.diff.get() {
                        " · HEAD / working tree · read only"
                    } else {
                        ""
                    }
                ));
            }
        });
        let weak = Rc::downgrade(&editor);
        edit_button.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                if e.diff.get() {
                    e.discard.emit_clicked();
                }
            }
        });
        let weak = Rc::downgrade(&editor);
        agents_button.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.show_agents();
            }
        });
        let weak = Rc::downgrade(&editor);
        document_button.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.show_files();
            }
        });
        let weak = Rc::downgrade(&editor);
        files_button.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.toggle_files();
            }
        });
        let weak = Rc::downgrade(&editor);
        git_button.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.toggle_git();
            }
        });
        let weak = Rc::downgrade(&editor);
        editor
            .content_stack
            .connect_visible_child_name_notify(move |_| {
                if let Some(editor) = weak.upgrade() {
                    let document = !editor.agents_visible();
                    editor.document_tools.set_visible(document);
                    editor.position.set_visible(document);
                    if !document {
                        editor.find_bar.set_visible(false);
                    }
                }
            });
        editor.setup_find();
        editor
    }
    pub fn is_dirty(&self) -> bool {
        self.buffer.is_modified() || self.busy.get() || self.git_busy.get()
    }
    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        if busy || !self.agents_visible() {
            self.content_stack
                .set_visible_child_name(if busy || !self.path.borrow().is_empty() {
                    "source"
                } else {
                    "empty"
                });
        }
        self.view
            .set_editable(!busy && !self.diff.get() && !self.path.borrow().is_empty());
        self.save.set_sensitive(!busy && self.buffer.is_modified());
        self.discard
            .set_sensitive(!busy && self.buffer.is_modified());
    }
    pub fn reset(&self) {
        self.scope_revision.set(self.scope_revision.get() + 1);
        self.git_revision.set(self.git_revision.get() + 1);
        clear(&self.file_undo);
        self.scope_project.set(0);
        self.selected_path.borrow_mut().clear();
        self.expanded.borrow_mut().clear();
        self.show_agents();
        self.commit_message.buffer().set_text("");
        self.branch_name.set_text("");
        self.branch_start.set_text("");
        self.worktree.borrow_mut().clear();
        self.worktrees.borrow_mut().clear();
        self.scope_label.set_text("Select a project");
        clear(&self.git);
        self.search.set_text("");
        self.clear_document();
    }
    fn clear_document(&self) {
        self.diff.set(false);
        self.before_scroll.set_visible(false);
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
        self.caption.set_text("Open a file");
    }
    pub fn invalidate(self: &Rc<Self>, ui: &Rc<Ui>) {
        if matches!(ui.page.borrow().as_str(), "code" | "agents")
            && !self.invalidate_pending.replace(true)
        {
            let weak = Rc::downgrade(ui);
            let e = self.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1000), move || {
                e.invalidate_pending.set(false);
                if let Some(ui) = weak.upgrade() {
                    if matches!(ui.page.borrow().as_str(), "code" | "agents") {
                        let directory = e.directory.borrow().clone();
                        if e.search.text().trim().is_empty() {
                            e.load_tree(&ui, Some(directory));
                        } else {
                            e.run_search(&ui);
                        }
                        e.refresh_git(&ui);
                        e.refresh_scopes(&ui);
                    }
                }
            });
        }
        if !self.path.borrow().is_empty() && !self.busy.get() && self.buffer.is_modified() {
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
            self.bind_controls(ui);
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
        if self.scope_project.get() != ui.project.get() {
            self.refresh_scopes(ui);
            self.refresh_git(ui);
        }
        let project = ui.project.get();
        let worktree = self.worktree.borrow().clone();
        self.tree_revision.set(self.tree_revision.get() + 1);
        let revision = self.tree_revision.get();
        let directory = directory.unwrap_or_default();
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = ui
                .call(
                    "file.tree",
                    json!({"project_id":project,"worktree":optional_scope(&worktree),"path":directory,"depth":1,"git_badges":true}),
                )
                .await;
            if !e.matches(&ui, project, &worktree) || e.tree_revision.get() != revision {
                return;
            }
            match result {
                Ok(v) => {
                    *e.directory.borrow_mut() = directory.clone();
                    clear(&e.tree);
                    e.render_tree(&ui, &e.tree, rows(&v, "entries"), revision);
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
        let worktree = self.worktree.borrow().clone();
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = ui
                .call(
                    "file.read",
                    json!({"project_id":project,"worktree":optional_scope(&worktree),"path":path,"max_bytes":1048576}),
                )
                .await;
            if e.revision.get() != revision {
                return;
            }
            e.set_busy(false);
            if !e.matches(&ui, project, &worktree) {
                return;
            }
            match result {
                Ok(v) => {
                    if v["truncated"] == true || v["text"].as_str().is_none() {
                        ui.show_error("This file is binary or exceeds the 1 MiB editor limit. It was not opened for editing.");
                        return;
                    }
                    e.diff.set(false);
                    e.before_scroll.set_visible(false);
                    let content = text(&v, "text");
                    e.buffer.set_text(content);
                    e.buffer.set_modified(false);
                    e.project.set(project);
                    *e.path.borrow_mut() = path.clone();
                    *e.selected_path.borrow_mut() = path.clone();
                    e.selected_directory.set(false);
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
        if project != ui.project.get()
            || !self.buffer.is_modified()
            || self.busy.get()
            || self.git_busy.get()
        {
            return;
        }
        let path = self.path.borrow().clone();
        let worktree = self.worktree.borrow().clone();
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
            use sha2::Digest;
            let expected = sha2::Sha256::digest(original.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let result = ui.call("file.write", json!({"project_id":project,"worktree":optional_scope(&worktree),"path":path,"text":content,"expected_sha256":expected})).await;
            if e.revision.get() != revision {
                return;
            }
            e.set_busy(false);
            if !e.matches(&ui, project, &worktree) {
                return;
            }
            match result {
                Ok(_) => {
                    *e.original.borrow_mut() = content;
                    e.buffer.set_modified(false);
                    e.refresh_git(&ui);
                }
                Err(err) => {
                    ui.show_error(&err.to_string());
                    e.save.set_sensitive(true);
                }
            }
        });
    }
}

fn optional_scope(worktree: &str) -> Option<&str> {
    if worktree.is_empty() {
        None
    } else {
        Some(worktree)
    }
}

impl Editor {
    fn render_tree(
        self: &Rc<Self>,
        ui: &Rc<Ui>,
        target: &gtk::Box,
        entries: Vec<Value>,
        revision: u64,
    ) {
        if entries.is_empty() {
            target.append(&label("Empty folder", "dim"));
        }
        for entry in entries {
            let path = text(&entry, "path").to_string();
            let title = format!("{} {}", text(&entry, "name"), text(&entry, "badge"));
            if text(&entry, "kind") == "dir" {
                let children = gtk::Box::new(gtk::Orientation::Vertical, 0);
                children.set_margin_start(14);
                let row = gtk::Expander::builder()
                    .label(&title)
                    .child(&children)
                    .build();
                row.add_css_class("code-folder");
                let heading = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                heading.append(&crate::icons::image("folder", 13));
                heading.append(&label(&title, "code-file-name"));
                row.set_label_widget(Some(&heading));
                let loaded = Rc::new(Cell::new(false));
                let e = self.clone();
                let weak = Rc::downgrade(ui);
                let child_path = path.clone();
                row.connect_expanded_notify(move |row| {
                    if !row.is_expanded() {
                        e.expanded.borrow_mut().remove(&child_path);
                        return;
                    }
                    e.expanded.borrow_mut().insert(child_path.clone());
                    if loaded.replace(true) {
                        return;
                    }
                    let Some(ui) = weak.upgrade() else {
                        return;
                    };
                    let project = ui.project.get();
                    let worktree = e.worktree.borrow().clone();
                    let payload =
                        e.payload(&ui, json!({"path":child_path,"depth":1,"git_badges":true}));
                    let e = e.clone();
                    let target = children.clone();
                    let loaded = loaded.clone();
                    glib::spawn_future_local(async move {
                        let result = ui.call("file.tree", payload).await;
                        if !e.matches(&ui, project, &worktree) || e.tree_revision.get() != revision
                        {
                            return;
                        }
                        match result {
                            Ok(v) => e.render_tree(&ui, &target, rows(&v, "entries"), revision),
                            Err(err) => {
                                loaded.set(false);
                                ui.show_error(&err.to_string());
                            }
                        }
                    });
                });
                self.bind_tree_row(ui, &row, &path, true);
                target.append(&row);
                let expand = self.expanded.borrow().contains(&path);
                row.set_expanded(expand);
            } else {
                let row = button("", "file");
                row.add_css_class("code-file");
                row.set_tooltip_text(Some(&path));
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                content.append(&crate::icons::image(project_files::file_icon(&path), 13));
                let name = label(text(&entry, "name"), "code-file-name");
                name.set_hexpand(true);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                content.append(&name);
                let badge = label(text(&entry, "badge"), "dim");
                content.append(&badge);
                row.set_child(Some(&content));
                self.bind_tree_row(ui, &row, &path, false);
                let e = self.clone();
                let weak = Rc::downgrade(ui);
                row.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        *e.selected_path.borrow_mut() = path.clone();
                        e.selected_directory.set(false);
                        e.open_path(&ui, path.clone(), None);
                    }
                });
                target.append(&row);
            }
        }
    }
    fn matches(&self, ui: &Ui, project: i64, worktree: &str) -> bool {
        ui.project.get() == project && self.worktree.borrow().as_str() == worktree
    }
    fn payload(&self, ui: &Ui, mut extra: Value) -> Value {
        extra["project_id"] = json!(ui.project.get());
        if !self.worktree.borrow().is_empty() {
            extra["worktree"] = json!(*self.worktree.borrow());
        }
        extra
    }
    /// Used by session file rails and task links. A dirty document always keeps its scope.
    pub fn open_path(self: &Rc<Self>, ui: &Rc<Ui>, path: String, worktree: Option<String>) {
        if self.is_dirty() {
            ui.show_error("Save or discard your changes before opening another file or worktree.");
            return;
        }
        if let Some(worktree) = worktree {
            if *self.worktree.borrow() != worktree {
                self.clear_document();
                *self.worktree.borrow_mut() = worktree;
                self.refresh_scopes(ui);
                self.refresh_git(ui);
                self.load_tree(ui, None);
            }
        }
        self.open(ui, path);
    }
    fn refresh_scopes(self: &Rc<Self>, ui: &Rc<Ui>) {
        self.scope_project.set(ui.project.get());
        self.scope_revision.set(self.scope_revision.get() + 1);
        let revision = self.scope_revision.get();
        let project = ui.project.get();
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = ui
                .call(
                    "worktree.list",
                    json!({"project_id":project,"include_dirty":false}),
                )
                .await;
            if project != ui.project.get() || revision != e.scope_revision.get() {
                return;
            }
            match result {
                Ok(v) => {
                    let items = rows(&v, "worktrees");
                    *e.worktrees.borrow_mut() = items;
                    e.update_scope_label(&ui);
                }
                Err(err) => ui.show_error(&err.to_string()),
            }
        });
    }
    fn bind_controls(self: &Rc<Self>, ui: &Rc<Ui>) {
        let e = self.clone();
        let weak = Rc::downgrade(ui);
        self.scope.set_create_popup_func(move |button| {
            if let Some(ui) = weak.upgrade() {
                e.scope_picker(&ui, button);
            }
        });
        self.bind_tree_drop(ui, &self.tree, "");
        let e = self.clone();
        let weak = Rc::downgrade(ui);
        self.search.connect_activate(move |_| {
            if let Some(ui) = weak.upgrade() {
                e.run_search(&ui);
            }
        });
        let e = self.clone();
        let weak = Rc::downgrade(ui);
        self.search.connect_stop_search(move |search| {
            search.set_text("");
            if let Some(ui) = weak.upgrade() {
                e.load_tree(&ui, None);
            }
        });
        for (title, op) in [
            ("New", "file.create"),
            ("Rename", "file.rename"),
            ("Trash", "file.delete"),
        ] {
            let action = button(title, "quiet");
            action.set_widget_name(&format!("project-{op}"));
            let e = self.clone();
            let weak = Rc::downgrade(ui);
            action.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    e.file_action(&ui, op);
                }
            });
            self.file_actions.append(&action);
        }
        let keys = gtk::EventControllerKey::new();
        let e = self.clone();
        let weak = Rc::downgrade(ui);
        keys.connect_key_pressed(move |_, key, _, state| {
            if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                if key == gtk::gdk::Key::s {
                    if let Some(ui) = weak.upgrade() {
                        e.save_file(&ui);
                    }
                    return glib::Propagation::Stop;
                }
                if key == gtk::gdk::Key::f || key == gtk::gdk::Key::h {
                    e.find_bar.set_visible(true);
                    e.find.grab_focus();
                    return glib::Propagation::Stop;
                }
            }
            if key == gtk::gdk::Key::Escape && e.find_bar.is_visible() {
                e.find_bar.set_visible(false);
                e.view.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.root.add_controller(keys);
    }
    fn setup_find(self: &Rc<Self>) {
        let e = self.clone();
        self.find.connect_changed(move |entry| {
            e.search_context
                .settings()
                .set_search_text(Some(&entry.text()))
        });
        for (title, backwards) in [("Previous", true), ("Next", false)] {
            let b = button(title, "quiet");
            let e = self.clone();
            b.connect_clicked(move |_| e.find_match(backwards));
            self.find_bar.append(&b);
        }
        let e = self.clone();
        self.find.connect_activate(move |_| e.find_match(false));
        let replace = button("Replace", "quiet");
        let e = self.clone();
        replace.connect_clicked(move |_| {
            if !e.view.is_editable() {
                return;
            }
            if let Some((mut start, mut end)) = e.buffer.selection_bounds() {
                if e.search_context.occurrence_position(&start, &end) > 0 {
                    if let Err(err) =
                        e.search_context
                            .replace(&mut start, &mut end, &e.replacement.text())
                    {
                        e.position.set_text(&err.to_string());
                        return;
                    }
                }
            }
            e.find_match(false);
        });
        self.find_bar.append(&replace);
        let all = button("All", "quiet");
        let e = self.clone();
        all.connect_clicked(move |_| {
            if !e.view.is_editable() || e.find.text().is_empty() {
                return;
            }
            // Native search retains undo grouping and UTF-8 text positions.
            let result = e.search_context.replace_all(&e.replacement.text());
            match result {
                Ok(()) => e.position.set_text("Replaced all matches"),
                Err(err) => e.position.set_text(&err.to_string()),
            }
        });
        self.find_bar.append(&all);
    }
    fn find_match(&self, backwards: bool) {
        let (start, end) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let iter = self.buffer.iter_at_offset(self.buffer.cursor_position());
            (iter, iter)
        });
        let found = if backwards {
            self.search_context.backward(&start)
        } else {
            self.search_context.forward(&end)
        };
        if let Some((start, mut end, _)) = found {
            self.buffer.select_range(&start, &end);
            self.view.scroll_to_iter(&mut end, 0.1, false, 0.0, 0.0);
        } else {
            self.position.set_text("No matches in this file");
        }
    }
    fn run_search(self: &Rc<Self>, ui: &Rc<Ui>) {
        let query = self.search.text().to_string();
        if query.trim().is_empty() {
            self.load_tree(ui, None);
            return;
        }
        self.tree_revision.set(self.tree_revision.get() + 1);
        let revision = self.tree_revision.get();
        let project = ui.project.get();
        let worktree = self.worktree.borrow().clone();
        let payload = self.payload(ui, json!({"query":query,"limit":100,"regex":false}));
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = ui.call("file.search", payload).await;
            if !e.matches(&ui, project, &worktree) || e.tree_revision.get() != revision {
                return;
            }
            match result {
                Ok(v) => {
                    clear(&e.tree);
                    let hits = rows(&v, "hits");
                    e.tree.append(&label(
                        &format!(
                            "{} matches{}",
                            hits.len(),
                            if hits.len() == 100 {
                                " (limit reached)"
                            } else {
                                ""
                            }
                        ),
                        "dim",
                    ));
                    for hit in hits {
                        let path = text(&hit, "path").to_string();
                        let b = button(&format!("{}:{}", path, hit["line"]), "file");
                        b.set_tooltip_text(Some(text(&hit, "text")));
                        let ed = e.clone();
                        let weak = Rc::downgrade(&ui);
                        b.connect_clicked(move |_| {
                            if let Some(ui) = weak.upgrade() {
                                ed.open(&ui, path.clone());
                            }
                        });
                        e.tree.append(&b);
                    }
                }
                Err(err) => ui.show_error(&err.to_string()),
            }
        });
    }
    fn file_action(self: &Rc<Self>, ui: &Rc<Ui>, op: &'static str) {
        if self.is_dirty() {
            ui.show_error("Save or discard the current file before changing files.");
            return;
        }
        let path = if self.selected_path.borrow().is_empty() {
            self.path.borrow().clone()
        } else {
            self.selected_path.borrow().clone()
        };
        if op != "file.create" && path.is_empty() {
            ui.show_error("Select a file or folder first.");
            return;
        }
        let title = match op {
            "file.create" => "Create file",
            "file.rename" => "Rename file",
            _ => "Move file to Relay trash",
        };
        let dialog = crate::panel::Panel::new(ui, title, 480);
        let caption = if op == "file.delete" {
            "Move to trash"
        } else {
            "Apply"
        };
        let entry = gtk::Entry::new();
        entry.set_widget_name("project-file-path");
        entry.set_placeholder_text(Some(if op == "file.create" {
            "relative/path.rs"
        } else {
            "new-name.rs"
        }));
        if op == "file.create" && !path.is_empty() {
            let directory = if self.selected_directory.get() {
                path.clone()
            } else {
                std::path::Path::new(&path)
                    .parent()
                    .unwrap_or(std::path::Path::new(""))
                    .to_string_lossy()
                    .to_string()
            };
            if !directory.is_empty() {
                entry.set_text(&format!("{directory}/"));
            }
        }
        let folder = gtk::CheckButton::with_label("Create a folder");
        folder.set_widget_name("project-file-folder");
        if op == "file.delete" {
            let copy = label(
                &format!("{path}\nThe file can be restored from Relay trash."),
                "body",
            );
            copy.set_wrap(true);
            dialog.body.append(&copy);
        } else {
            if op == "file.rename" {
                entry.set_text(
                    std::path::Path::new(&path)
                        .file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or(""),
                );
            }
            dialog.body.append(&entry);
            if op == "file.create" {
                dialog.body.append(&folder);
            }
        }
        let project = ui.project.get();
        let worktree = self.worktree.borrow().clone();
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let accepted = dialog.response(caption).await;
            if !accepted || !e.matches(&ui, project, &worktree) || e.is_dirty() {
                return;
            }
            let value = entry.text().trim().to_string();
            if op != "file.delete" && value.is_empty() {
                ui.show_error("Enter a file name.");
                return;
            }
            let extra = match op {
                "file.create" => {
                    json!({"path":value,"kind":if folder.is_active(){"dir"}else{"file"},"text":""})
                }
                "file.rename" => json!({"path":path,"new_name":value}),
                _ => json!({"path":path}),
            };
            e.set_busy(true);
            let result = ui.call(op, e.payload(&ui, extra)).await;
            e.set_busy(false);
            if !e.matches(&ui, project, &worktree) {
                return;
            }
            match result {
                Ok(v) => {
                    if op == "file.rename" && text(&v, "kind") == "dir" {
                        let new_path = text(&v, "path");
                        let expanded = e
                            .expanded
                            .borrow()
                            .iter()
                            .map(|item| {
                                item.strip_prefix(&path)
                                    .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
                                    .map(|suffix| format!("{new_path}{suffix}"))
                                    .unwrap_or_else(|| item.clone())
                            })
                            .collect();
                        *e.expanded.borrow_mut() = expanded;
                    }
                    *e.selected_path.borrow_mut() = if op == "file.delete" {
                        String::new()
                    } else {
                        text(&v, "path").to_owned()
                    };
                    e.selected_directory.set(text(&v, "kind") == "dir");
                    e.clear_document();
                    e.load_tree(&ui, None);
                    e.refresh_git(&ui);
                    if op == "file.delete" {
                        // The ID is also kept by the engine; no permanent deletion is offered here.
                        let id = v["trash_id"].as_i64().unwrap_or(0);
                        let restore = button("Undo trash", "quiet");
                        restore.set_widget_name("project-file-undo");
                        let weak = Rc::downgrade(&ui);
                        let ed = e.clone();
                        restore.connect_clicked(move |button| {
                            let Some(ui) = weak.upgrade() else {
                                return;
                            };
                            if ui.project.get() != project {
                                return;
                            }
                            button.set_sensitive(false);
                            let button = button.clone();
                            let ed = ed.clone();
                            glib::spawn_future_local(async move {
                                match ui
                                    .call(
                                        "file.restore",
                                        json!({"project_id":project,"trash_id":id}),
                                    )
                                    .await
                                {
                                    Ok(_) => {
                                        button.set_visible(false);
                                        ed.load_tree(&ui, None);
                                        ed.refresh_git(&ui);
                                    }
                                    Err(err) => {
                                        button.set_sensitive(true);
                                        ui.show_error(&err.to_string());
                                    }
                                }
                            });
                        });
                        e.file_undo.append(&restore);
                    } else if text(&v, "kind") != "dir" {
                        e.open(&ui, text(&v, "path").to_string());
                    }
                }
                Err(err) => ui.show_error(&err.to_string()),
            }
        });
    }
}
