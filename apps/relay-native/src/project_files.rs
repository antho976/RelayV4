use super::*;

impl Editor {
    pub fn mount_agents(&self, agents: &impl IsA<gtk::Widget>) {
        self.content_stack.add_named(agents, Some("agents"));
        self.show_agents();
    }
    pub fn show_agents(&self) {
        if self.content_stack.child_by_name("agents").is_some() {
            self.content_stack.set_visible_child_name("agents");
        }
    }
    pub fn agents_visible(&self) -> bool {
        self.content_stack.visible_child_name().as_deref() == Some("agents")
    }
    pub fn connect_view_changed(&self, callback: impl Fn() + 'static) {
        self.content_stack
            .connect_visible_child_name_notify(move |_| callback());
    }
    pub fn show_files(&self) {
        self.file_sidebar.set_visible(true);
        self.content_stack
            .set_visible_child_name(if self.path.borrow().is_empty() {
                "empty"
            } else if self.image_mode.get() {
                "image"
            } else {
                "source"
            });
    }
    pub fn toggle_files(&self) {
        self.file_sidebar
            .set_visible(!self.file_sidebar.is_visible());
    }
    pub fn toggle_git(&self) {
        self.git.set_visible(!self.git.is_visible());
    }
    pub fn layout_state(&self) -> Value {
        json!({"files":self.file_sidebar.is_visible(),"git":self.git.is_visible(),
            "files_width":self.files_split.position().max(260),"git_width":self.git.width().max(320)})
    }
    pub fn apply_layout(&self, state: &Value) {
        if let Some(visible) = state["files"].as_bool() {
            self.file_sidebar.set_visible(visible);
        }
        if let Some(visible) = state["git"].as_bool() {
            self.git.set_visible(visible);
        }
        let files_width = state["files_width"].as_i64().unwrap_or(260).clamp(180, 440) as i32;
        let git_width = state["git_width"].as_i64().unwrap_or(320).clamp(260, 520) as i32;
        self.files_split.set_position(files_width);
        self.git_split.set_position(
            (self.root.width().max(900)
                - if self.file_sidebar.is_visible() {
                    files_width
                } else {
                    0
                }
                - git_width)
                .max(200),
        );
    }
    pub fn prepare_project(self: &Rc<Self>, ui: &Rc<Ui>) {
        self.load_tree(ui, None);
        self.refresh_scopes(ui);
        self.refresh_git(ui);
        self.update_scope_label(ui);
    }
    pub(super) fn update_scope_label(&self, ui: &Ui) {
        let projects = ui.projects.borrow();
        let project = projects.iter().find(|p| p["id"] == ui.project.get());
        let project_name = project.map(|p| text(p, "name")).unwrap_or("Project");
        let spaces = ui.workspaces.borrow();
        let space = project.and_then(|p| spaces.iter().find(|w| w["id"] == p["workspace_id"]));
        let workspace_name = space.map(|w| text(w, "name")).unwrap_or("Workspace");
        let worktrees = self.worktrees.borrow();
        let selected = worktrees
            .iter()
            .find(|w| text(w, "path") == self.worktree.borrow().as_str())
            .or_else(|| worktrees.first());
        let branch = selected
            .map(|w| text(w, "branch"))
            .unwrap_or("Primary checkout");
        self.scope_label
            .set_text(&format!("{workspace_name} / {project_name} · {branch}"));
    }
    fn select_checkout(self: &Rc<Self>, ui: &Rc<Ui>, path: &str) {
        if self.worktree.borrow().as_str() == path {
            return;
        }
        if self.is_dirty() {
            ui.show_info("Save or discard your changes before switching checkouts.");
            return;
        }
        let agents = self.agents_visible();
        self.clear_document();
        self.commit_message.buffer().set_text("");
        self.branch_name.set_text("");
        self.branch_start.set_text("");
        self.selected_path.borrow_mut().clear();
        self.expanded.borrow_mut().clear();
        *self.worktree.borrow_mut() = path.into();
        self.search.set_text("");
        self.load_tree(ui, None);
        self.refresh_git(ui);
        self.update_scope_label(ui);
        if agents {
            self.show_agents();
        }
    }
    pub(super) fn scope_picker(self: &Rc<Self>, ui: &Rc<Ui>, button: &gtk::MenuButton) {
        let popover = gtk::Popover::new();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.set_size_request(350, -1);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search projects, branches, or agents"));
        body.append(&search);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let mut searchable: Vec<(gtk::Widget, String)> = Vec::new();
        list.append(&label("PROJECTS", "section-label"));
        for project in ui.projects.borrow().iter() {
            let spaces = ui.workspaces.borrow();
            let space = spaces.iter().find(|w| w["id"] == project["workspace_id"]);
            let title = format!(
                "{} / {}",
                space.map(|w| text(w, "name")).unwrap_or("Workspace"),
                text(project, "name")
            );
            let key = button_row(&title, "folder");
            let id = project["id"].as_i64().unwrap_or(0);
            if id == ui.project.get() {
                key.add_css_class("selected");
            }
            let weak = Rc::downgrade(ui);
            let pop = popover.downgrade();
            key.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.open_project(id, "agents");
                }
                if let Some(pop) = pop.upgrade() {
                    pop.popdown();
                }
            });
            searchable.push((key.clone().upcast(), title.to_lowercase()));
            list.append(&key);
        }
        list.append(&label("CHECKOUTS & BRANCHES", "section-label"));
        let mut choices = vec![(String::new(), String::from("Primary checkout"))];
        choices.extend(self.worktrees.borrow().iter().map(|w| {
            (
                text(w, "path").to_string(),
                format!(
                    "{} · {}",
                    text(w, "branch"),
                    w["session"].as_str().unwrap_or("Checkout")
                ),
            )
        }));
        for (path, title) in choices {
            let key = button_row(&title, "branch");
            key.set_tooltip_text(Some(&path));
            if path == *self.worktree.borrow() {
                key.add_css_class("selected");
            }
            let ed = self.clone();
            let weak = Rc::downgrade(ui);
            let pop = popover.downgrade();
            key.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ed.select_checkout(&ui, &path);
                }
                if let Some(pop) = pop.upgrade() {
                    pop.popdown();
                }
            });
            searchable.push((key.clone().upcast(), title.to_lowercase()));
            list.append(&key);
        }
        let branch_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        branch_list.append(&label("LOCAL BRANCHES", "section-label"));
        list.append(&branch_list);
        let ed = self.clone();
        let weak = Rc::downgrade(ui);
        let pop = popover.downgrade();
        let query = search.clone();
        let revision = self.scope_revision.get();
        let project = ui.project.get();
        let worktree = self.worktree.borrow().clone();
        let payload = self.payload(ui, json!({}));
        glib::spawn_future_local(async move {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let result = ui.call("git.branches", payload).await;
            if !ed.matches(&ui, project, &worktree) || ed.scope_revision.get() != revision {
                return;
            }
            match result {
                Ok(value) => {
                    for branch in rows(&value, "branches") {
                        let name = text(&branch, "name").to_owned();
                        let key = button_row(&name, "branch");
                        key.set_sensitive(branch["current"] != true && branch["session"].is_null());
                        let needle = name.to_lowercase();
                        key.set_visible(needle.contains(query.text().to_lowercase().as_str()));
                        let row = key.downgrade();
                        query.connect_search_changed(move |search| {
                            if let Some(row) = row.upgrade() {
                                row.set_visible(
                                    needle.contains(search.text().to_lowercase().as_str()),
                                );
                            }
                        });
                        let editor = ed.clone();
                        let weak = Rc::downgrade(&ui);
                        let pop = pop.clone();
                        key.connect_clicked(move |_| {
                            if let Some(pop) = pop.upgrade() {
                                pop.popdown();
                            }
                            if let Some(ui) = weak.upgrade() {
                                editor.git_action(
                                    &ui,
                                    "git.branch.switch",
                                    json!({"name":name}),
                                    None,
                                );
                            }
                        });
                        branch_list.append(&key);
                    }
                }
                Err(error) => branch_list.append(&label(&error.to_string(), "dim")),
            }
        });
        search.connect_search_changed(move |search| {
            let query = search.text().to_lowercase();
            for (row, title) in &searchable {
                row.set_visible(title.contains(&query));
            }
        });
        let scroll = scrolled(&list);
        scroll.set_max_content_height(400);
        scroll.set_propagate_natural_height(true);
        body.append(&scroll);
        popover.set_child(Some(&body));
        button.set_popover(Some(&popover));
        popover.connect_show(move |_| {
            search.grab_focus();
        });
    }
    pub(super) fn bind_tree_row(
        self: &Rc<Self>,
        ui: &Rc<Ui>,
        row: &impl IsA<gtk::Widget>,
        path: &str,
        directory: bool,
    ) {
        row.set_widget_name(&format!("project-file:{path}"));
        let mark = gtk::GestureClick::new();
        mark.set_button(1);
        let weak = Rc::downgrade(self);
        let selected = path.to_owned();
        mark.connect_pressed(move |_, _, _, _| {
            if let Some(editor) = weak.upgrade() {
                *editor.selected_path.borrow_mut() = selected.clone();
                editor.selected_directory.set(directory);
            }
        });
        row.add_controller(mark);
        let select = gtk::GestureClick::new();
        select.set_button(3);
        let ed = self.clone();
        let weak = Rc::downgrade(ui);
        let path_owned = path.to_owned();
        let target = row.as_ref().downgrade();
        select.connect_pressed(move |_, _, x, y| {
            let (Some(ui), Some(target)) = (weak.upgrade(), target.upgrade()) else {
                return;
            };
            *ed.selected_path.borrow_mut() = path_owned.clone();
            ed.selected_directory.set(directory);
            let pop = gtk::Popover::new();
            pop.set_parent(&target);
            pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            let actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
            for (title, op) in [
                ("New file or folder", "file.create"),
                ("Rename", "file.rename"),
                ("Move to trash", "file.delete"),
            ] {
                let key = button(title, "quiet");
                let ed = ed.clone();
                let ui = Rc::downgrade(&ui);
                let close = pop.downgrade();
                key.connect_clicked(move |_| {
                    if let Some(pop) = close.upgrade() {
                        pop.popdown();
                    }
                    if let Some(ui) = ui.upgrade() {
                        ed.file_action(&ui, op);
                    }
                });
                actions.append(&key);
            }
            pop.set_child(Some(&actions));
            pop.connect_closed(|pop| pop.unparent());
            pop.popup();
        });
        row.add_controller(select);
        let source = gtk::DragSource::new();
        source.set_actions(gtk::gdk::DragAction::MOVE);
        let weak = Rc::downgrade(ui);
        let ed = self.clone();
        if directory {
            self.bind_tree_drop(ui, row, path);
        }
        let path = path.to_owned();
        source.connect_prepare(move |_, _, _| {
            let ui = weak.upgrade()?;
            let data =
                json!({"project":ui.project.get(),"worktree":*ed.worktree.borrow(),"path":path})
                    .to_string();
            Some(gtk::gdk::ContentProvider::for_value(&data.to_value()))
        });
        row.add_controller(source);
    }
    pub(super) fn bind_tree_drop(
        self: &Rc<Self>,
        ui: &Rc<Ui>,
        row: &impl IsA<gtk::Widget>,
        into: &str,
    ) {
        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        let ed = self.clone();
        let weak = Rc::downgrade(ui);
        let into = into.to_owned();
        drop.connect_drop(move |_, value, _, _| {
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            let Ok(data) = value.get::<String>() else {
                return false;
            };
            let Ok(data) = serde_json::from_str::<Value>(&data) else {
                return false;
            };
            if data["project"] != ui.project.get()
                || data["worktree"] != *ed.worktree.borrow()
                || ed.is_dirty()
            {
                return false;
            }
            let Some(path) = data["path"].as_str().filter(|path| !path.is_empty()) else {
                return false;
            };
            if into == path || into.starts_with(&format!("{path}/")) {
                return false;
            }
            let payload = ed.payload(&ui, json!({"path":path,"into":into}));
            let moved = path.to_owned();
            let document = ed.path.borrow().clone();
            let agents = ed.agents_visible();
            let ed = ed.clone();
            ed.busy.set(true);
            glib::spawn_future_local(async move {
                let result = ui.call("file.move", payload).await;
                ed.busy.set(false);
                match result {
                    Ok(value) => {
                        let suffix = document
                            .strip_prefix(&moved)
                            .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'));
                        if let Some(suffix) = suffix {
                            let new_path = format!("{}{suffix}", text(&value, "path"));
                            ed.clear_document();
                            ed.open(&ui, new_path);
                        }
                        ed.load_tree(&ui, None);
                        ed.refresh_git(&ui);
                        if agents {
                            ed.show_agents();
                        }
                    }
                    Err(error) => ui.show_error(&error.to_string()),
                }
            });
            true
        });
        row.add_controller(drop);
    }
}
fn button_row(title: &str, icon: &str) -> gtk::Button {
    let row = button("", "quiet");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&crate::icons::image(icon, 13));
    let name = label(title, "body");
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name.set_max_width_chars(42);
    content.append(&name);
    row.set_child(Some(&content));
    row
}
pub(super) fn file_icon(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
    {
        "rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "kt" | "java" | "svelte" | "html" | "css"
        | "sh" => "code",
        "toml" | "json" | "yaml" | "yml" | "xml" | "ini" => "settings",
        "md" | "txt" | "rst" => "notes",
        "zip" | "gz" | "tar" => "modules",
        _ => "file",
    }
}
