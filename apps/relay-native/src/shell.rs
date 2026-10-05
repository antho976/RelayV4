use super::*;
use vte4::prelude::*;

impl Ui {
    pub fn open_project(self: &Rc<Self>, project: i64, page: &str) {
        if project != self.project.get() {
            if !self.dismiss_panels() {
                return;
            }
            if self.editor.is_dirty() {
                self.show_error("Save or discard editor changes before switching projects.");
                return;
            }
            if self.launch_busy.get() {
                self.show_error("Wait for agent creation to finish before switching projects.");
                return;
            }
            self.launch.set_reveal_child(false);
            self.project.set(project);
            // Switch the visible wall synchronously. The registry already contains
            // all projects' sessions; a slow refresh must never leave the old CLI here.
            *self.sessions.borrow_mut() = self.sidebar_sessions.borrow().iter()
                .filter(|session| session["project_id"].as_i64() == Some(project))
                .cloned().collect();
            self.ordered.borrow_mut().clear();
            self.reconcile();
            self.editor.reset();
            self.restored_project.set(0);
            self.refresh();
        }
        self.navigate(page);
    }
    pub(super) fn render_projects(self: &Rc<Self>) {
        // Only registry or visible session state changes rebuild the project tree.
        let signature = format!(
            "{}:{}:{}:{:?}",
            self.project.get(),
            serde_json::to_string(&*self.projects.borrow()).unwrap_or_default(),
            serde_json::to_string(&*self.workspaces.borrow()).unwrap_or_default(),
            self.sidebar_sessions
                .borrow()
                .iter()
                .map(|s| (s["project_id"].as_i64(), text(s, "state").to_owned()))
                .collect::<Vec<_>>()
        );
        if self.projects_box.widget_name() == signature {
            return;
        }
        self.projects_box.set_widget_name(&signature);
        clear(&self.projects_box);
        let projects = self.projects.borrow().clone();
        for workspace in self.workspaces.borrow().iter() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            row.add_css_class("workspace-row");
            self.registry_drag(&row, workspace, true);
            let caption = label(text(workspace, "name"), "dim");
            caption.set_hexpand(true);
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&crate::icons::image("chevron-down", 12));
            row.append(&caption);
            let add = icon_button("plus", "Add project to this workspace");
            add.add_css_class("small-key");
            add.add_css_class("add-project");
            add.set_child(Some(&crate::icons::image("plus", 12)));
            add.set_widget_name(&format!("workspace-add-{}", workspace["id"]));
            row.append(&add);
            let weak = Rc::downgrade(self);
            let target = workspace.clone();
            add.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.open_repository_in(Some(target.clone()));
                }
            });
            let manage = self.registry_menu(workspace, true);
            manage.add_css_class("small-key");
            manage.add_css_class("manage");
            row.append(&manage);
            self.projects_box.append(&row);
            let mut children: Vec<_> = projects
                .iter()
                .filter(|p| p["workspace_id"] == workspace["id"])
                .collect();
            children.sort_by_key(|p| {
                (
                    !p["pinned"].as_bool().unwrap_or(false),
                    p["order"].as_i64().unwrap_or(0),
                    p["id"].as_i64().unwrap_or(0),
                )
            });
            for project in children {
                let row = gtk::Overlay::new();
                row.add_css_class("project-row");
                self.registry_drag(&row, project, false);
                let name = text(project, "name");
                let id = project["id"].as_i64().unwrap_or(0);
                let b = button("", "project");
                b.set_hexpand(true);
                let content = gtk::Grid::builder()
                    .column_spacing(6)
                    .row_spacing(2)
                    .build();
                content.add_css_class("project-content");
                let sessions = self.sidebar_sessions.borrow();
                let mine: Vec<_> = sessions
                    .iter()
                    .filter(|s| s["project_id"].as_i64() == Some(id))
                    .collect();
                let live = mine
                    .iter()
                    .filter(|s| {
                        matches!(
                            text(s, "state"),
                            "spawning" | "running" | "idle" | "blocked"
                        )
                    })
                    .count();
                let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                lamp.add_css_class("lamp");
                lamp.add_css_class("small-lamp");
                lamp.set_valign(gtk::Align::Center);
                if mine.iter().any(|s| text(s, "state") == "blocked") {
                    lamp.add_css_class("held");
                } else if mine
                    .iter()
                    .any(|s| matches!(text(s, "state"), "running" | "spawning"))
                {
                    lamp.add_css_class("live");
                }
                content.attach(&lamp, 0, 0, 1, 1);
                let title = label(name, "project-name");
                title.set_hexpand(true);
                title.set_width_chars(1);
                title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                content.attach(&title, 1, 0, 1, 1);
                if live > 0 {
                    content.attach(&label(&live.to_string(), "project-count"), 2, 0, 1, 1);
                }
                if id == self.project.get() {
                    b.add_css_class("selected");
                    row.add_css_class("selected");
                    let branch = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                    branch.add_css_class("project-branch");
                    branch.set_tooltip_text(Some(
                        "Base for new agent branches. Change it in Project settings.",
                    ));
                    branch.append(&crate::icons::image("branch", 11));
                    let branch_name =
                        label(&format!("Base: {}", text(project, "base_branch")), "mono");
                    branch_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    branch_name.set_width_chars(1);
                    branch_name.set_hexpand(true);
                    branch.append(&branch_name);
                    content.attach(&branch, 1, 1, 2, 1);
                }
                if project["pinned"] == true {
                    b.add_css_class("pinned");
                }
                b.set_child(Some(&content));
                let weak = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.open_project(id, "agents");
                    }
                });
                row.set_child(Some(&b));
                let controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                controls.add_css_class("project-tools");
                controls.set_halign(gtk::Align::End);
                controls.set_valign(gtk::Align::Start);
                let pin = icon_button(
                    "pin",
                    if project["pinned"] == true {
                        "Unpin project"
                    } else {
                        "Pin project"
                    },
                );
                pin.set_child(Some(&crate::icons::image("pin", 12)));
                pin.add_css_class("small-key");
                pin.add_css_class("manage");
                if project["pinned"] == true {
                    pin.add_css_class("pinned");
                }
                controls.append(&pin);
                let weak = Rc::downgrade(self);
                let pinned = project["pinned"] != true;
                pin.connect_clicked(move |key| {
                    if let Some(ui) = weak.upgrade() {
                        ui.mutate(
                            "project.update",
                            json!({"project_id":id,"pinned":pinned}),
                            key,
                        );
                    }
                });
                let menu = self.registry_menu(project, false);
                menu.add_css_class("small-key");
                menu.add_css_class("manage");
                controls.append(&menu);
                row.add_overlay(&controls);
                self.projects_box.append(&row);
            }
        }
    }
    fn registry_menu(self: &Rc<Self>, value: &Value, workspace: bool) -> gtk::MenuButton {
        let menu = gtk::MenuButton::new();
        menu.set_child(Some(&crate::icons::image("more", 13)));
        menu.set_tooltip_text(Some(if workspace {
            "Workspace menu"
        } else {
            "Project menu"
        }));
        let popover = gtk::Popover::new();
        popover.add_css_class("registry-menu");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 3);
        body.set_size_request(230, -1);
        let title = label(text(value, "name"), "title");
        body.append(&title);
        let path = label(text(value, "path"), "dim");
        path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        path.set_max_width_chars(32);
        path.set_tooltip_text(Some(text(value, "path")));
        body.append(&path);
        body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let choices = if workspace {
            vec![
                ("plus", "Add project", "add"),
                ("settings", "Workspace settings", "settings"),
            ]
        } else {
            vec![
                ("terminal", "Open agents", "agents"),
                ("files", "Files and Git", "code"),
                ("settings", "Project settings", "settings"),
            ]
        };
        for (icon, caption, action) in choices {
            let key = button("", "registry-action");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            line.append(&crate::icons::image(icon, 15));
            line.append(&label(caption, ""));
            key.set_child(Some(&line));
            body.append(&key);
            let weak = Rc::downgrade(self);
            let value = value.clone();
            let pop = popover.clone();
            key.connect_clicked(move |_| {
                pop.popdown();
                if let Some(ui) = weak.upgrade() {
                    match action {
                        "add" => ui.open_repository_in(Some(value.clone())),
                        "settings" => ui.registry_editor(value.clone(), workspace),
                        page => ui.open_project(value["id"].as_i64().unwrap_or(0), page),
                    }
                }
            });
        }
        popover.set_child(Some(&body));
        menu.set_popover(Some(&popover));
        menu
    }
    fn registry_drag(
        self: &Rc<Self>,
        widget: &impl IsA<gtk::Widget>,
        value: &Value,
        workspace: bool,
    ) {
        let id = value["id"].as_i64().unwrap_or(0);
        let kind = if workspace { "workspace" } else { "project" };
        widget.set_widget_name(&format!("registry-{kind}-{id}"));
        let token = format!("relay-{kind}:{id}");
        let source = gtk::DragSource::new();
        source.set_actions(gtk::gdk::DragAction::MOVE);
        source.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(&token.to_value()))
        });
        widget.add_controller(source);
        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        let target = value.clone();
        drop.connect_drop(move |drop, v, _, y| {
            let Ok(token) = v.get::<String>() else {
                return false;
            };
            let Some(from) = token
                .strip_prefix(&format!("relay-{kind}:"))
                .and_then(|s| s.parse::<i64>().ok())
            else {
                return false;
            };
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            if from == id {
                return false;
            }
            let mut items = if workspace {
                ui.workspaces.borrow().clone()
            } else {
                ui.projects
                    .borrow()
                    .iter()
                    .filter(|p| {
                        p["workspace_id"] == target["workspace_id"]
                            && p["pinned"] == target["pinned"]
                    })
                    .cloned()
                    .collect()
            };
            items.sort_by_key(|p| {
                (
                    p["order"].as_i64().unwrap_or(0),
                    p["id"].as_i64().unwrap_or(0),
                )
            });
            let Some(index) = items.iter().position(|p| p["id"] == from) else {
                ui.show_error("Reorder projects within the same workspace and pinned group.");
                return false;
            };
            let item = items.remove(index);
            let Some(index) = items.iter().position(|p| p["id"] == id) else {
                return false;
            };
            let after = drop
                .widget()
                .is_some_and(|w| y > f64::from(w.height()) / 2.0);
            items.insert(index + usize::from(after), item);
            ui.projects_box.set_sensitive(false);
            glib::spawn_future_local(async move {
                for (order, item) in items.into_iter().enumerate() {
                    let payload = if workspace {
                        json!({"workspace_id":item["id"],"order":order})
                    } else {
                        json!({"project_id":item["id"],"order":order})
                    };
                    if let Err(e) = ui
                        .call(
                            if workspace {
                                "workspace.update"
                            } else {
                                "project.update"
                            },
                            payload,
                        )
                        .await
                    {
                        ui.show_error(&e.to_string());
                        break;
                    }
                }
                ui.projects_box.set_sensitive(true);
                ui.registry_dirty.set(true);
                ui.refresh();
            });
            true
        });
        widget.add_controller(drop);
    }
    fn registry_editor(self: &Rc<Self>, value: Value, workspace: bool) {
        let Some((window, body)) =
            self.sheet(if workspace { "Workspace" } else { "Project" }, 420, 480)
        else {
            return;
        };
        let name = gtk::Entry::builder().text(text(&value, "name")).build();
        field("Name", &name, &body);
        let path = label(text(&value, "path"), "dim");
        path.set_wrap(true);
        path.set_selectable(true);
        body.append(&path);
        let branch = gtk::Entry::builder()
            .text(text(&value, "base_branch"))
            .build();
        let build = gtk::Entry::builder()
            .text(text(&value, "build_cmd"))
            .build();
        let run = gtk::Entry::builder().text(text(&value, "run_cmd")).build();
        let pinned = gtk::CheckButton::with_label("Pin project");
        pinned.set_active(value["pinned"] == true);
        if !workspace {
            field("Base branch", &branch, &body);
            let help = label(
                "New agents branch from this base. Existing agent branches stay where they are.",
                "dim",
            );
            help.set_wrap(true);
            body.append(&help);
            field("Build command", &build, &body);
            field("Run command", &run, &body);
            body.append(&pinned);
        }
        let save = button("Save", "primary");
        body.append(&save);
        let weak = Rc::downgrade(self);
        let win = window.clone();
        let v = value.clone();
        save.connect_clicked(move|key|{let Some(ui)=weak.upgrade()else{return;};let name=name.text().trim().to_string();if name.is_empty(){ui.show_error("Enter a name.");return;}
            let payload=if workspace{json!({"workspace_id":v["id"],"name":name})}else{json!({"project_id":v["id"],"name":name,"base_branch":branch.text().as_str(),"build_cmd":build.text().as_str(),"run_cmd":run.text().as_str(),"pinned":pinned.is_active()})};
            let win=win.clone();let key=key.clone();key.set_sensitive(false);
            glib::spawn_future_local(async move{match ui.call(if workspace{"workspace.update"}else{"project.update"},payload).await{Ok(_)=>{ui.registry_dirty.set(true);ui.refresh();win.close();},Err(e)=>ui.show_error(&e.to_string())}key.set_sensitive(true);});
        });
        let forget = button(
            if workspace {
                "Remove empty workspace"
            } else {
                "Forget project"
            },
            "quiet",
        );
        body.append(&forget);
        let weak = Rc::downgrade(self);
        let win = window.clone();
        forget.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let payload = if workspace {
                    json!({"workspace_id":value["id"]})
                } else {
                    json!({"project_id":value["id"]})
                };
                ui.confirm_mutation(
                    if workspace {
                        "Remove this workspace from Relay? It must be empty first. Projects, terminals, and files will not be removed automatically."
                    } else {
                        "Remove this project from Relay? Close its sessions and stop its active runs first. Files and branches stay on disk; Relay will not close terminals for you."
                    },
                    if workspace {
                        "workspace.remove"
                    } else {
                        "project.remove"
                    },
                    payload,
                    Some(win.clone()),
                );
            }
        });
        window.present();
    }
    pub(super) fn sheet(
        &self,
        title: &str,
        width: i32,
        _height: i32,
    ) -> Option<(Rc<crate::panel::Panel>, gtk::Box)> {
        let panel = crate::panel::Panel::toggle(self, title, width)?;
        let body = panel.body.clone();
        Some((panel, body))
    }
    fn confirm_mutation(
        self: &Rc<Self>,
        message: &str,
        op: &'static str,
        payload: Value,
        parent: Option<Rc<crate::panel::Panel>>,
    ) {
        let dialog = crate::panel::Panel::new(self, "Confirm action", 520);
        let copy = label(message, "body");
        copy.set_wrap(true);
        dialog.body.append(&copy);
        let ui = self.clone();
        glib::spawn_future_local(async move {
            if dialog.response("Confirm").await {
                match ui.call(op, payload).await {
                    Ok(_) => {
                        if let Some(w) = parent {
                            w.close();
                        }
                        ui.registry_dirty.set(true);
                        ui.refresh();
                    }
                    Err(e) => ui.show_error(&e.to_string()),
                }
            }
        });
    }
    pub(super) fn layout(self: &Rc<Self>) {
        while let Some(w) = self.wall.first_child() {
            self.wall.remove(&w);
        }
        while let Some(w) = self.wall_right.first_child() {
            self.wall_right.remove(&w);
        }
        let names: Vec<_> = self.ordered.borrow().iter().cloned().collect();
        if self
            .focused
            .borrow()
            .as_ref()
            .is_none_or(|n| !names.contains(n))
        {
            *self.focused.borrow_mut() = names.first().cloned();
        }
        let focus = self.focused.borrow().clone();
        let mode = self.mode.borrow().clone();
        self.wall_right.set_visible(
            matches!(mode.as_str(), "review" | "mosaic")
                || (mode == "grid" && self.columns.get() == 2 && names.len() > 1),
        );
        clear(&self.focus_tabs);
        self.focus_tabs
            .set_visible(mode == "focus" && names.len() > 1);
        for (i, name) in names.iter().enumerate() {
            if mode == "focus" {
                let b = button("", "focus-tab");
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
                let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                lamp.add_css_class("lamp");
                lamp.add_css_class("small-lamp");
                lamp.set_valign(gtk::Align::Center);
                if let Some(session) = self
                    .sessions
                    .borrow()
                    .iter()
                    .find(|s| text(s, "name") == name)
                {
                    match text(session, "state") {
                        "running" | "spawning" => lamp.add_css_class("live"),
                        "blocked" => lamp.add_css_class("held"),
                        "restorable" => lamp.add_css_class("waiting"),
                        _ => (),
                    }
                }
                row.append(&lamp);
                row.append(&label(name, ""));
                b.set_child(Some(&row));
                if focus.as_ref() == Some(name) {
                    b.add_css_class("selected");
                }
                let weak = Rc::downgrade(self);
                let name = name.clone();
                b.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        *ui.focused.borrow_mut() = Some(name.clone());
                        ui.layout();
                        ui.save_layout();
                    }
                });
                self.focus_tabs.append(&b);
            }
            if let Some(p) = self.panes.borrow().get(name) {
                if mode == "focus" && focus.as_ref() != Some(name) {
                    continue;
                }
                if matches!(mode.as_str(), "review" | "mosaic") && names.len() > 1 {
                    if focus.as_ref() == Some(name) {
                        self.wall.attach(&p.root, 0, 0, 1, 1);
                    } else {
                        let offset = names[..i]
                            .iter()
                            .filter(|n| Some(*n) != focus.as_ref())
                            .count() as i32;
                        let columns = if mode == "mosaic" { 2 } else { 1 };
                        self.wall_right
                            .attach(&p.root, offset % columns, offset / columns, 1, 1);
                    }
                } else if mode == "grid" && self.columns.get() == 2 && names.len() > 1 {
                    let grid = if i % 2 == 0 {
                        &self.wall
                    } else {
                        &self.wall_right
                    };
                    grid.attach(&p.root, 0, (i / 2) as i32, 1, 1);
                } else {
                    let cols = if mode == "focus" {
                        1
                    } else {
                        self.columns.get()
                    };
                    self.wall.attach(
                        &p.root,
                        i as i32 % cols,
                        if mode == "focus" { 0 } else { i as i32 / cols },
                        1,
                        1,
                    );
                }
                p.schedule_resize();
            }
        }
        self.update_attachments();
    }
    pub(super) fn set_mode(self: &Rc<Self>, mode: &str) {
        *self.mode.borrow_mut() = mode.into();
        self.layout();
        self.save_layout();
    }
    pub(super) fn save_layout(self: &Rc<Self>) {
        self.layout_revision
            .set(self.layout_revision.get().wrapping_add(1));
        let project = self.project.get();
        if project == 0 || self.client.borrow().is_none() || self.restored_project.get() != project
        {
            return;
        }
        self.layout_saves
            .borrow_mut()
            .insert(project, self.layout_state());
        if self.layout_saving.replace(true) {
            return;
        }
        let ui = self.clone();
        glib::spawn_future_local(async move {
            loop {
                let next = ui.layout_saves.borrow_mut().pop_first();
                let Some((project, state)) = next else { break };
                if let Err(e) = ui
                    .call(
                        "settings.set",
                        json!({"path":format!("native.layout.current.{project}"),"value":state}),
                    )
                    .await
                {
                    ui.show_error(&e.to_string());
                }
            }
            ui.layout_saving.set(false);
        });
    }
    fn layout_state(&self) -> Value {
        json!({"page":*self.page.borrow(),"agent_layout":*self.mode.borrow(),"columns":self.columns.get(),"focused":*self.focused.borrow(),"order":*self.ordered.borrow(),"sidebar":if self.page.borrow().as_str() == "settings" { self.settings_sidebar.get() } else { self.sidebar.is_visible() },"split":self.wall_split.position(),"width":self.window.width(),"height":self.window.height(),"project_tools":self.editor.layout_state()})
    }
    pub(super) async fn restore_layout(self: &Rc<Self>, revision: u64) {
        let project = self.project.get();
        if self.restored_project.get() == project {
            return;
        }
        self.restored_project.set(project);
        if self.layout_revision.get() != revision {
            self.save_layout();
            return;
        }
        let result = self
            .call(
                "settings.get",
                json!({"path":format!("native.layout.current.{project}")}),
            )
            .await;
        // A pending startup read must not undo navigation or a layout edit.
        if self.project.get() != project || self.layout_revision.get() != revision {
            return;
        }
        if let Ok(v) = result {
            if !v["value"].is_null() {
                let applying = self.applying_ui.replace(true);
                self.apply_layout(&v["value"]);
                self.applying_ui.set(applying);
            }
        }
        self.refresh_page();
    }
    pub(super) fn apply_layout(self: &Rc<Self>, state: &Value) {
        self.editor.apply_layout(&state["project_tools"]);
        if let Some(mode) = state["agent_layout"]
            .as_str()
            .filter(|s| matches!(*s, "grid" | "focus" | "review" | "mosaic"))
        {
            *self.mode.borrow_mut() = mode.into();
        }
        if let Some(n) = state["columns"].as_i64() {
            self.columns.set(n.clamp(1, 3) as i32);
        }
        if let Some(n) = state["focused"].as_str() {
            *self.focused.borrow_mut() = Some(n.into());
        }
        if let Some(order) = state["order"].as_array() {
            let current = self.ordered.borrow().clone();
            let mut order: Vec<String> = order
                .iter()
                .filter_map(|n| n.as_str().map(str::to_owned))
                .filter(|n| current.contains(n))
                .collect();
            order.dedup();
            for n in current {
                if !order.contains(&n) {
                    order.push(n);
                }
            }
            *self.ordered.borrow_mut() = order;
        }
        if let Some(split) = state["split"].as_i64() {
            let saved_width = state["width"].as_i64().unwrap_or(1440) as f64;
            let current_width = self.window.width().max(self.window.default_width());
            let sidebar = if self.sidebar.is_visible() {
                200.0
            } else {
                0.0
            };
            let available = (current_width as f64 - sidebar - 32.0).max(560.0);
            let ratio =
                (split as f64 / (saved_width - sidebar - 32.0).max(560.0)).clamp(0.25, 0.75);
            self.wall_split.set_position((available * ratio) as i32);
        }
        if let Some(visible) = state["sidebar"].as_bool() {
            self.settings_sidebar.set(visible);
            if self.page.borrow().as_str() != "settings" {
                self.sidebar.set_visible(visible);
            }
        }
        if let Some(page) = state["page"]
            .as_str()
            .filter(|p| *p == "agents" || *p == "code" || self.pages.contains_key(*p))
        {
            self.navigate(page);
        } else {
            self.layout();
        }
    }
    pub(super) fn sync_navigation(self: &Rc<Self>, page: &str) {
        *self.navigation_pending.borrow_mut() = Some((self.project.get(), page.into()));
        if self.navigation_sending.replace(true) {
            return;
        }
        let ui = self.clone();
        glib::spawn_future_local(async move {
            // Coalesce rapid clicks and keep the engine's final page in click order.
            loop {
                let next = ui.navigation_pending.borrow_mut().take();
                let Some((project, page)) = next else { break };
                let client = ui.client.borrow().clone();
                let Some(client) = client else { break };
                let id = uuid::Uuid::new_v4();
                ui.navigation_echoes.borrow_mut().insert(id);
                let result = client
                    .request_with_id(
                        &ui.rt,
                        "ui.page.switch",
                        json!({"page":page,"project_id":if project>0 {Some(project)} else {None}}),
                        id,
                    )
                    .await;
                if let Err(error) = result {
                    ui.navigation_echoes.borrow_mut().remove(&id);
                    ui.show_error(&error.to_string());
                }
            }
            ui.navigation_sending.set(false);
        });
    }
    pub(super) fn apply_ui_event(self: &Rc<Self>, state: &Value) {
        self.applying_ui.set(true);
        if let Some(page) = state["page"].as_str() {
            let project = state["project_id"].as_i64().unwrap_or(self.project.get());
            if *self.page.borrow() != page || self.project.get() != project {
                self.open_project(project, page);
            }
        }
        if let Some(panes) = state["panes"].as_array() {
            for pane in panes {
                if pane["focused"] == true {
                    if let Some(session) = pane["target"]["session"].as_str() {
                        let changed = self.focused.borrow().as_deref() != Some(session);
                        *self.focused.borrow_mut() = Some(session.into());
                        // ui.changed carries all panes even when switching pages.
                        // Only a new terminal focus on Agents should change its layout.
                        if changed && state["page"].as_str().is_none_or(|page| page == "agents") {
                            self.set_mode("focus");
                            self.navigate("agents");
                        }
                    }
                }
            }
        }
        self.applying_ui.set(false);
    }
    pub(super) fn layout_menu(self: &Rc<Self>) {
        let Some((window, body)) = self.sheet("Window presets", 390, 420) else {
            return;
        };
        window.compact(false, 620);
        body.append(&label(
            "Choose a starting layout, then save your arrangement.",
            "dim",
        ));
        let modes = gtk::Grid::builder()
            .column_spacing(8)
            .row_spacing(8)
            .column_homogeneous(true)
            .build();
        body.append(&modes);
        for (i, (mode, caption, columns)) in [
            ("grid", "Two columns", 2),
            ("focus", "Focus + tabs", 1),
            ("review", "Focus + stack", 2),
            ("mosaic", "Focus + grid", 2),
            ("grid", "Three columns", 3),
            ("grid", "Vertical stack", 1),
        ]
        .into_iter()
        .enumerate()
        {
            let b = button("", "layout-preset");
            let card = gtk::Box::new(gtk::Orientation::Vertical, 7);
            let preview = gtk::DrawingArea::new();
            preview.set_content_height(50);
            preview.set_draw_func(move |_, cr, width, height| {
                let w = f64::from(width);
                let h = f64::from(height);
                cr.set_source_rgb(0.35, 0.37, 0.39);
                let rect = |x: f64, y: f64, rw: f64, rh: f64| {
                    cr.rectangle(x * w + 2., y * h + 2., rw * w - 4., rh * h - 4.);
                    let _ = cr.fill();
                };
                match mode {
                    "focus" => rect(0., 0., 1., 1.),
                    "review" => {
                        rect(0., 0., 0.6, 1.);
                        rect(0.6, 0., 0.4, 0.5);
                        rect(0.6, 0.5, 0.4, 0.5);
                    }
                    "mosaic" => {
                        rect(0., 0., 0.5, 1.);
                        for j in 0..4 {
                            rect(
                                0.5 + f64::from(j % 2) * 0.25,
                                f64::from(j / 2) * 0.5,
                                0.25,
                                0.5,
                            );
                        }
                    }
                    _ => {
                        for j in 0..columns * 2 {
                            rect(
                                f64::from(j % columns) / f64::from(columns),
                                f64::from(j / columns) * 0.5,
                                1. / f64::from(columns),
                                0.5,
                            );
                        }
                    }
                }
            });
            card.append(&preview);
            card.append(&label(caption, ""));
            b.set_child(Some(&card));
            if *self.mode.borrow() == mode && (mode != "grid" || self.columns.get() == columns) {
                b.add_css_class("selected");
            }
            modes.attach(&b, (i % 2) as i32, (i / 2) as i32, 1, 1);
            let weak = Rc::downgrade(self);
            let grid = modes.downgrade();
            b.connect_clicked(move |key| {
                if let Some(ui) = weak.upgrade() {
                    ui.columns.set(columns);
                    ui.set_mode(mode);
                    if let Some(grid) = grid.upgrade() {
                        let mut child = grid.first_child();
                        while let Some(item) = child {
                            item.remove_css_class("selected");
                            child = item.next_sibling();
                        }
                    }
                    key.add_css_class("selected");
                }
            });
        }
        let sidebar = gtk::CheckButton::with_label("Show sidebar");
        sidebar.set_active(self.sidebar.is_visible());
        body.append(&sidebar);
        let weak = Rc::downgrade(self);
        sidebar.connect_toggled(move |b| {
            if let Some(ui) = weak.upgrade() {
                ui.sidebar.set_visible(b.is_active());
                ui.save_layout();
            }
        });
        body.append(&label("Saved presets", "title"));
        let saved = gtk::Box::new(gtk::Orientation::Vertical, 4);
        body.append(&saved);
        let name = gtk::Entry::builder()
            .placeholder_text("Preset name")
            .max_length(64)
            .build();
        body.append(&name);
        let save = button("Save current", "primary");
        body.append(&save);
        let weak = Rc::downgrade(self);
        let win = window.clone();
        let project = self.project.get();
        save.connect_clicked(move |key| {
            if let Some(ui) = weak.upgrade() {
                let name = name.text().trim().to_string();
                if name.is_empty() {
                    return;
                }
                let state = ui.layout_state();
                let win = win.clone();
                let key = key.clone();
                key.set_sensitive(false);
                glib::spawn_future_local(async move {
                    match ui
                        .call(
                            "ui.layout.save",
                            json!({"project_id":project,"name":name,"state":state}),
                        )
                        .await
                    {
                        Ok(_) => win.close(),
                        Err(e) => ui.show_error(&e.to_string()),
                    }
                    key.set_sensitive(true);
                });
            }
        });
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match ui
                .call("ui.layout.list", json!({"project_id":project}))
                .await
            {
                Ok(v) => {
                    for name in rows(&v, "layouts").iter().filter_map(Value::as_str) {
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                        let apply = button(name, "quiet");
                        apply.set_hexpand(true);
                        row.append(&apply);
                        let delete = icon_button("edit-delete-symbolic", "Delete preset");
                        row.append(&delete);
                        saved.append(&row);
                        for (key, op) in [(apply, "ui.layout.apply"), (delete, "ui.layout.delete")]
                        {
                            let weak = Rc::downgrade(&ui);
                            let name = name.to_string();
                            key.connect_clicked(move |b| {
                                if let Some(ui) = weak.upgrade() {
                                    ui.mutate(op, json!({"project_id":project,"name":name}), b);
                                }
                            });
                        }
                    }
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
        });
        window.present();
    }
    pub(super) fn install_pane_controls(self: &Rc<Self>, pane: &Rc<Pane>, name: &str) {
        let drag = gtk::DragSource::new();
        drag.set_actions(gtk::gdk::DragAction::MOVE);
        let name = name.to_string();
        drag.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(&name.to_value()))
        });
        pane.header.add_controller(drag);
        let drop_target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        let target = pane.name().to_string();
        drop_target.connect_drop(move |_, v, _, y| {
            let Ok(name) = v.get::<String>() else {
                return false;
            };
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            let mut order = ui.ordered.borrow_mut();
            let Some(from) = order.iter().position(|n| *n == name) else {
                return false;
            };
            if name == target {
                return false;
            }
            order.remove(from);
            let Some(to) = order.iter().position(|n| *n == target) else {
                return false;
            };
            order.insert(to + usize::from(y > 100.0), name);
            drop(order);
            ui.layout();
            ui.save_layout();
            true
        });
        pane.root.add_controller(drop_target);
    }
    pub(super) fn session_actions(self: &Rc<Self>, pane: &Rc<Pane>, session: &Value) {
        clear(&pane.actions);
        clear(&pane.slate_actions);
        let state = text(session, "state");
        let name = text(session, "name");
        let (caption, icon, op) = match state {
            "created" => ("Start", "media-playback-start-symbolic", "session.spawn"),
            "parked" => ("Wake", "media-playback-start-symbolic", "session.wake"),
            "restorable" | "exited" => {
                ("Resume", "media-playback-start-symbolic", "session.resume")
            }
            _ => ("Park", "media-playback-pause-symbolic", "session.park"),
        };
        let action = icon_button(icon, caption);
        action.set_child(Some(&crate::icons::image(icon, 13)));
        if matches!(state, "created" | "parked" | "restorable" | "exited") {
            let slate_action = button(
                if state == "created" {
                    "Start session"
                } else {
                    caption
                },
                "",
            );
            let weak = Rc::downgrade(self);
            let n = name.to_string();
            slate_action.connect_clicked(move |key| {
                if let Some(ui) = weak.upgrade() {
                    ui.mutate(op, json!({"session":n}), key);
                }
            });
            pane.slate_actions.append(&slate_action);
        }
        let weak = Rc::downgrade(self);
        let n = name.to_string();
        action.connect_clicked(move |b| {
            if let Some(ui) = weak.upgrade() {
                ui.mutate(op, json!({"session":n}), b);
            }
        });
        pane.actions.append(&action);
        if matches!(state, "restorable" | "exited") {
            // Confirmed in place: the key itself turns into "Confirm …" for a second click.
            for (caption, icon, op, armed, tip) in [
                ("Clear context", "refresh", "session.clear_restorable", "Confirm clear", "Start fresh in this session and worktree; saved provider conversation context is cleared"),
                ("Discard session", "close", "session.close", "Confirm discard", "Remove this session from the wall; its worktree and branch are kept"),
            ] {
                if op == "session.clear_restorable" && state != "restorable" { continue; }
                for key in [button(caption, "quiet"), icon_button(icon, caption)] {
                    let slate = key.label().is_some();
                    key.set_tooltip_text(Some(if slate { tip } else { caption }));
                    let weak = Rc::downgrade(self);
                    let n = name.to_string();
                    crate::app::confirm_inline(&key, if slate { armed } else { "Confirm" }, move |key| {
                        if let Some(ui) = weak.upgrade() {
                            let payload = if op == "session.close" { json!({"session":n,"remove_worktree":false,"purge_build":false}) } else { json!({"session":n}) };
                            ui.mutate(op, payload, key);
                        }
                    });
                    if slate { pane.slate_actions.append(&key); } else { pane.actions.append(&key); }
                }
            }
        }
        if matches!(state, "created" | "parked" | "restorable" | "exited") {
            self.describe_stopped_session(pane, session);
        } else {
            pane.begin_context();
        }
        let zoom = icon_button("view-fullscreen-symbolic", "Focus terminal");
        zoom.set_child(Some(&crate::icons::image("maximize", 13)));
        let weak = Rc::downgrade(self);
        let n = name.to_string();
        zoom.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let already =
                    *ui.mode.borrow() == "focus" && ui.focused.borrow().as_ref() == Some(&n);
                *ui.focused.borrow_mut() = Some(n.clone());
                ui.set_mode(if already { "grid" } else { "focus" });
            }
        });
        pane.actions.append(&zoom);
        let menu = icon_button("view-more-symbolic", "Session menu");
        menu.set_child(Some(&crate::icons::image("more", 13)));
        let weak = Rc::downgrade(self);
        let session = session.clone();
        menu.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.session_menu(session.clone());
            }
        });
        pane.actions.append(&menu);
    }
    fn session_menu(self: &Rc<Self>, session: Value) {
        let name = text(&session, "name").to_string();
        let Some((window, body)) = self.sheet(&name, 440, 620) else {
            return;
        };
        body.append(&label(
            &format!(
                "{} · {} · {}",
                text(&session, "provider"),
                text(&session, "role"),
                text(&session, "state")
            ),
            "dim",
        ));
        for key in ["worktree", "branch", "intent", "pair_with"] {
            let l = label(text(&session, key), "dim");
            l.set_wrap(true);
            l.set_selectable(true);
            body.append(&l);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        body.append(&row);
        for (caption, delta) in [("Move earlier", -1), ("Move later", 1)] {
            let b = button(caption, "quiet");
            row.append(&b);
            let weak = Rc::downgrade(self);
            let n = name.clone();
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    let mut order = ui.ordered.borrow_mut();
                    if let Some(i) = order.iter().position(|s| *s == n) {
                        let next = (i as i32 + delta).clamp(0, order.len() as i32 - 1) as usize;
                        order.swap(i, next);
                    }
                    drop(order);
                    ui.layout();
                    ui.save_layout();
                }
            });
        }
        let brief = button("Inspect launch brief", "quiet");
        body.append(&brief);
        let weak = Rc::downgrade(self);
        let n = name.clone();
        brief.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let n = n.clone();
                glib::spawn_future_local(async move {
                    match ui.call("session.brief", json!({"session":n})).await {
                        Ok(v) => {
                            let Some((w, b)) = ui.sheet("Session brief", 760, 640) else {
                                return;
                            };
                            let t = gtk::TextView::new();
                            t.set_editable(false);
                            t.set_monospace(true);
                            t.buffer().set_text(text(&v, "text"));
                            b.append(&t);
                            w.present();
                        }
                        Err(e) => ui.show_error(&e.to_string()),
                    }
                });
            }
        });
        let model = gtk::Entry::builder().text(text(&session, "model")).build();
        field("Model", &model, &body);
        let effort = gtk::Entry::builder().text(text(&session, "effort")).build();
        field("Effort", &effort, &body);
        let writes = gtk::CheckButton::with_label("Allow agent bus writes");
        writes.set_active(session["bus_writes"] == true);
        body.append(&writes);
        let ui_access = gtk::CheckButton::with_label("Allow UI control");
        ui_access.set_active(session["allow_ui"] == true);
        body.append(&ui_access);
        let spawned = !session["spawned_at"].is_null();
        model.set_sensitive(!spawned);
        effort.set_sensitive(!spawned);
        let save = button("Save session settings", "primary");
        body.append(&save);
        let weak = Rc::downgrade(self);
        let n = name.clone();
        save.connect_clicked(move|b|{if let Some(ui)=weak.upgrade(){let mut payload=json!({"session":n,"bus_writes":writes.is_active(),"allow_ui":ui_access.is_active()});if !spawned{payload["model"]=model.text().as_str().into();payload["effort"]=effort.text().as_str().into();}ui.mutate("session.update",payload,b);}});
        if text(&session, "state") == "restorable" {
            let fresh = button("Start fresh without saved provider context…", "quiet");
            body.append(&fresh);
            let weak = Rc::downgrade(self);
            let n = name.clone();
            let w = window.clone();
            fresh.connect_clicked(move |_|{if let Some(ui)=weak.upgrade(){ui.confirm_mutation("Start fresh in this same session and worktree? Saved provider conversation context will be cleared.","session.clear_restorable",json!({"session":n}),Some(w.clone()));}});
        }
        let cleanup = gtk::Expander::new(Some("Worktree cleanup"));
        body.append(&cleanup);
        let options = gtk::Box::new(gtk::Orientation::Vertical, 6);
        cleanup.set_child(Some(&options));
        let remove = gtk::CheckButton::with_label("Remove worktree after closing");
        options.append(&remove);
        let purge = gtk::CheckButton::with_label("Purge build output");
        options.append(&purge);
        let key = button("Close with selected cleanup…", "quiet");
        options.append(&key);
        let weak = Rc::downgrade(self);
        let n = name.clone();
        let w = window.clone();
        let path = text(&session, "worktree").to_string();
        key.connect_clicked(move |_|{if let Some(ui)=weak.upgrade(){ui.confirm_mutation(&format!("Close {n} at {path}? Remove worktree: {}. Purge build output: {}. The branch is retained.",remove.is_active(),purge.is_active()),"session.close",json!({"session":n,"remove_worktree":remove.is_active(),"purge_build":purge.is_active()}),Some(w.clone()));}});
        let close = button("Close session…", "quiet");
        body.append(&close);
        let weak = Rc::downgrade(self);
        let w = window.clone();
        close.connect_clicked(move |_|{if let Some(ui)=weak.upgrade(){ui.confirm_mutation("Close this session and stop its process? The worktree and branch will be kept.","session.close",json!({"session":name,"remove_worktree":false,"purge_build":false}),Some(w.clone()));}});
        window.present();
    }
    pub(super) fn refresh_notification_count(self: &Rc<Self>) {
        let revision = self.notification_revision.get().wrapping_add(1);
        self.notification_revision.set(revision);
        let generation = self.generation.get();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = ui
                .call("notify.list", json!({"unread_only":true,"limit":100}))
                .await
            {
                if ui.generation.get() != generation || ui.notification_revision.get() != revision {
                    return;
                }
                let unread = rows(&result, "notifications");
                ui.notification_count.set_visible(!unread.is_empty());
                ui.notification_count.set_text(&if unread.len() > 99 {
                    String::from("99+")
                } else {
                    unread.len().to_string()
                });
                if unread
                    .iter()
                    .any(|n| matches!(text(n, "category"), "agent_blocked" | "guardrail"))
                {
                    ui.notification_count.add_css_class("held");
                } else {
                    ui.notification_count.remove_css_class("held");
                }
            }
        });
    }
    pub(super) fn project_skills(self: &Rc<Self>) {
        let project = self.project.get();
        let Some((panel, body)) = self.sheet("Agent skills", 390, 560) else {
            return;
        };
        panel.top(560);
        panel.add_css_class("agent-skills-popover");
        let name = self
            .projects
            .borrow()
            .iter()
            .find(|p| p["id"].as_i64() == Some(project))
            .map(|p| text(p, "name").to_owned())
            .unwrap_or_else(|| "No project".into());
        body.set_spacing(0);
        let scope = label(&name.to_uppercase(), "section-label");
        body.append(&scope);
        let intro = label(
            "Change the project instructions used when agents start or resume. Running agents keep the context already loaded.",
            "agent-skills-intro",
        );
        intro.set_wrap(true);
        body.append(&intro);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.append(&label("Loading skills…", "dim"));
        body.append(&list);
        let feedback = label("", "dim");
        feedback.set_wrap(true);
        feedback.set_visible(false);
        body.append(&feedback);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("agent-skills-footer");
        let scope = label("PROJECT-WIDE", "section-label");
        scope.set_hexpand(true);
        footer.append(&scope);
        let manage = button("Open skill library", "quiet");
        footer.append(&manage);
        body.append(&footer);
        let weak = Rc::downgrade(self);
        let panel_weak = Rc::downgrade(&panel);
        manage.connect_clicked(move |_| {
            if let Some(panel) = panel_weak.upgrade() {
                panel.close();
            }
            if let Some(ui) = weak.upgrade() {
                ui.navigate("skills");
            }
        });
        panel.present();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match ui.call("skill.list", json!({"project_id":project})).await {
                Ok(result) => {
                    clear(&list);
                    let skills = rows(&result, "skills");
                    if skills.is_empty() {
                        list.append(&label("No skills installed.", "dim"));
                    }
                    for skill in skills {
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                        row.add_css_class("agent-skill-row");
                        let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
                        words.set_hexpand(true);
                        let title = label(text(&skill, "name"), "skill-title");
                        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        words.append(&title);
                        let source = skill["source_path"].as_str().unwrap_or(
                            if skill["source_url"].is_string() {
                                "Installed from GitHub"
                            } else {
                                "Local instruction"
                            },
                        );
                        let source = label(source, "skill-source");
                        source.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        words.append(&source);
                        row.append(&words);
                        let toggle = gtk::Switch::new();
                        toggle.set_valign(gtk::Align::Center);
                        let enabled = skill["enabled_in"]
                            .as_array()
                            .is_some_and(|ids| ids.iter().any(|id| id.as_i64() == Some(project)));
                        toggle.set_active(enabled);
                        toggle.set_sensitive(project > 0);
                        toggle.set_tooltip_text(Some(&format!(
                            "Enable {} for {name}",
                            text(&skill, "name")
                        )));
                        row.append(&toggle);
                        list.append(&row);
                        let weak = Rc::downgrade(&ui);
                        let feedback = feedback.clone();
                        toggle.connect_state_set(move |key,on| {
                            if let Some(ui) = weak.upgrade() {
                                let key = key.clone();
                                let feedback = feedback.clone();
                                let id = skill["id"].clone();
                                key.set_sensitive(false);
                                glib::spawn_future_local(async move {
                                    match ui.call("skill.enable", json!({"skill_id":id,"project_id":project,"enabled":on})).await {
                                        Ok(_) => feedback.set_visible(false),
                                        Err(error) => { key.set_active(!on); feedback.set_text(&error.to_string()); feedback.set_visible(true); }
                                    }
                                    key.set_sensitive(true);
                                });
                            }
                            glib::Propagation::Proceed
                        });
                    }
                }
                Err(error) => {
                    clear(&list);
                    feedback.set_text(&error.to_string());
                    feedback.set_visible(true);
                }
            }
        });
    }
    /// The plugin switches of one project, opened from its row in the sidebar.
    pub(super) fn project_plugins(self: &Rc<Self>, project: i64) {
        let Some((panel, body)) = self.sheet("Plugins", 440, 640) else {
            return;
        };
        panel.top(640);
        // Filled asynchronously; without a floor the panel keeps its "Loading" height.
        panel.min_height(460);
        panel.add_css_class("agent-skills-popover");
        let name = self
            .projects
            .borrow()
            .iter()
            .find(|p| p["id"].as_i64() == Some(project))
            .map(|p| text(p, "name").to_owned())
            .unwrap_or_else(|| "No project".into());
        body.set_spacing(0);
        body.append(&label(&name.to_uppercase(), "section-label"));
        let intro = label(
            "A plugin that is on gives every agent here its skills, standing rules and MCP tools. Skills reach running agents now; rules and tools apply when an agent starts or resumes.",
            "agent-skills-intro",
        );
        intro.set_wrap(true);
        body.append(&intro);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.append(&label("Loading plugins…", "dim"));
        body.append(&list);
        let feedback = label("", "dim");
        feedback.set_wrap(true);
        feedback.set_visible(false);
        body.append(&feedback);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("agent-skills-footer");
        let scope = label("THIS PROJECT", "section-label");
        scope.set_hexpand(true);
        footer.append(&scope);
        let manage = button("All plugins", "quiet");
        footer.append(&manage);
        body.append(&footer);
        let weak = Rc::downgrade(self);
        let panel_weak = Rc::downgrade(&panel);
        manage.connect_clicked(move |_| {
            if let Some(panel) = panel_weak.upgrade() {
                panel.close();
            }
            if let Some(ui) = weak.upgrade() {
                ui.navigate("plugins");
            }
        });
        panel.present();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match ui.call("plugin.list", json!({"project_id":project})).await {
                Ok(result) => {
                    clear(&list);
                    let plugins = rows(&result, "plugins");
                    if plugins.is_empty() {
                        let stale = label(crate::tools::plugins::STALE_ENGINE, "dim");
                        stale.set_wrap(true);
                        list.append(&stale);
                    }
                    for plugin in plugins {
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                        row.add_css_class("agent-skill-row");
                        let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
                        words.set_hexpand(true);
                        let title = label(text(&plugin, "name"), "skill-title");
                        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        words.append(&title);
                        let suggested = plugin["suggested_for"]
                            .as_array()
                            .is_some_and(|ids| ids.iter().any(|id| id.as_i64() == Some(project)));
                        let detail = if suggested {
                            format!("Suggested for this project · {}", text(&plugin, "summary"))
                        } else {
                            text(&plugin, "summary").to_string()
                        };
                        let detail = label(&detail, "skill-source");
                        detail.set_wrap(true);
                        detail.set_xalign(0.);
                        words.append(&detail);
                        row.append(&words);
                        let toggle = crate::tools::plugins::switch(
                            &ui,
                            &plugin,
                            project,
                            Some(feedback.clone()),
                        );
                        toggle.set_sensitive(project > 0);
                        row.append(&toggle);
                        list.append(&row);
                    }
                }
                Err(error) => {
                    clear(&list);
                    feedback.set_text(&crate::tools::plugins::explain(&error.to_string()));
                    feedback.set_visible(true);
                }
            }
        });
    }
    pub(super) fn command_palette(self: &Rc<Self>) {
        let Some((window, body)) = self.sheet("Command palette", 560, 480) else {
            return;
        };
        window.compact(true, 400);
        let search = gtk::SearchEntry::new();
        body.append(&search);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        body.append(&list);
        let mut entries = Vec::new();
        for (page, name) in [
            ("agents", "Agents"),
            ("code", "Code"),
            ("board", "Board"),
            ("modules", "Modules"),
            ("notes", "Notes"),
            ("dashboard", "Dashboard"),
            ("skills", "Skills"),
            ("plugins", "Plugins"),
            ("settings", "Settings"),
            ("mailbox", "Mailbox"),
            ("guardrails", "Guardrails"),
            ("devices", "Devices"),
            ("notifications", "Notifications"),
        ] {
            let b = button(name, "nav");
            list.append(&b);
            entries.push((name.to_lowercase(), b.clone()));
            let weak = Rc::downgrade(self);
            let w = window.clone();
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.navigate(page);
                    w.close();
                }
            });
        }
        for s in self.sessions.borrow().iter() {
            let name = text(s, "name").to_string();
            let b = button(&format!("Focus {name}"), "nav");
            list.append(&b);
            entries.push((name.to_lowercase(), b.clone()));
            let weak = Rc::downgrade(self);
            let w = window.clone();
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    *ui.focused.borrow_mut() = Some(name.clone());
                    ui.set_mode("focus");
                    ui.navigate("agents");
                    w.close();
                }
            });
        }
        search.connect_search_changed(move |s| {
            let q = s.text().to_lowercase();
            for (name, b) in &entries {
                b.set_visible(name.contains(&q));
            }
        });
        window.present();
        search.grab_focus();
    }
    pub(super) fn install_shortcuts(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(ui) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !ui.panels.borrow().is_empty() {
                return glib::Propagation::Proceed;
            }
            for (action, _, fallback) in crate::shortcuts::DEFAULTS {
                let bindings = ui.keybindings.borrow();
                let chord = bindings[action].as_str().unwrap_or(fallback);
                if !crate::shortcuts::matches(key, mods, chord) {
                    continue;
                }
                drop(bindings);
                match action {
                    "palette" => ui.command_palette(),
                    "agents" | "code" | "board" | "settings" => ui.navigate(action),
                    "new_session" => ui.show_launch(None),
                    "sidebar" => {
                        ui.sidebar.set_visible(!ui.sidebar.is_visible());
                        ui.save_layout();
                    }
                    _ => continue,
                }
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Escape && ui.launch.reveals_child() && !ui.launch_busy.get() {
                ui.launch.set_reveal_child(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
    }
    pub(super) fn load_keybindings(self: &Rc<Self>) {
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let generation = ui.generation.get();
            match ui.call("settings.get", json!({"path":"keybindings"})).await {
                Ok(v) if generation == ui.generation.get() => {
                    *ui.keybindings.borrow_mut() = v["value"].clone()
                }
                Err(e) => ui.show_error(&e.to_string()),
                _ => {}
            }
        });
    }
    pub(super) fn usage(self: &Rc<Self>) {
        let Some((window, body)) = self.sheet("Provider usage", 400, 420) else {
            return;
        };
        window.bottom(380);
        let refresh = button("Refresh", "quiet");
        body.append(&refresh);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.append(&list);
        let pending = Rc::new(RefCell::new(None::<glib::JoinHandle<()>>));
        let closing = pending.clone();
        window.on_closed(move || {
            if let Some(task) = closing.borrow_mut().take() {
                task.abort();
            }
        });
        let weak = Rc::downgrade(self);
        refresh.connect_clicked(move |key| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let key = key.clone();
            let list = list.clone();
            key.set_sensitive(false);
            *pending.borrow_mut() = Some(glib::spawn_future_local(async move {
                clear(&list);
                match ui.call("usage.get", json!({})).await {
                    Ok(v) => {
                        let entries = rows(&v, "usage");
                        if entries.is_empty() {
                            list.append(&label("No provider usage has been reported yet.", "dim"));
                        }
                        for item in entries {
                            list.append(&label(text(&item, "provider"), "title"));
                            if let Some(windows) = item["windows"].as_object() {
                                for (name, value) in windows {
                                    if let Some(pct) =
                                        value["used_pct"].as_f64().or(value["pct"].as_f64())
                                    {
                                        list.append(&label(
                                            &format!(
                                                "{} · {:.0}% used",
                                                name.replace('_', " "),
                                                pct.clamp(0., 100.)
                                            ),
                                            "body",
                                        ));
                                        let bar = gtk::ProgressBar::new();
                                        bar.set_fraction(pct.clamp(0., 100.) / 100.);
                                        list.append(&bar);
                                        if let Some(reset) = value["resets_in"].as_str() {
                                            list.append(&label(
                                                &format!("Resets in {reset}"),
                                                "dim",
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => ui.show_error(&e.to_string()),
                }
                key.set_sensitive(true);
            }));
        });
        refresh.emit_clicked();
        window.present();
    }
    pub(super) fn load_appearance(self: &Rc<Self>) {
        crate::wallpaper_rotation::refresh(self);
        let ui = self.clone();
        glib::spawn_future_local(async move {
            let generation = ui.generation.get();
            let (mode, size, alpha, contrast, image, dim) = tokio::join!(
                ui.call("settings.get", json!({"path":"appearance.mode"})),
                ui.call("settings.get", json!({"path":"terminal.font_size"})),
                ui.call("settings.get", json!({"path":"appearance.panel_alpha"})),
                ui.call(
                    "settings.get",
                    json!({"path":"appearance.content_contrast"})
                ),
                ui.call("settings.get", json!({"path":"appearance.wallpaper"})),
                ui.call("settings.get", json!({"path":"appearance.wallpaper_dim"}))
            );
            if generation != ui.generation.get() {
                return;
            }
            if let Ok(value) = mode {
                *ui.palette.borrow_mut() = value["value"].as_str().unwrap_or("matte").into();
            }
            if let Ok(value) = size {
                ui.font_size
                    .set(value["value"].as_f64().unwrap_or(9.75).clamp(8.0, 24.0));
            }
            let colors = match ui.palette.borrow().as_str() {
                "dark" => [
                    "#0a0b0d", "#101114", "#16171b", "#1e1f24", "#08090a", "#eef0f2", "#a3a7ad",
                    "#202228", "#70747b", "#33363d",
                ],
                "oled" => [
                    "#000000", "#000000", "#0d0d0e", "#161618", "#000000", "#ececea", "#a5a5a3",
                    "#1f1f22", "#77777a", "#333336",
                ],
                _ => [
                    "#0e0e10", "#141416", "#1b1b1e", "#232327", "#0a0a0b", "#ececea", "#a5a5a3",
                    "#252529", "#77777a", "#37373c",
                ],
            };
            let css: [String; 10] = std::array::from_fn(|i| {
                format!(
                    "@define-color {} {};",
                    [
                        "wall",
                        "console",
                        "slab",
                        "wash",
                        "screen",
                        "ink",
                        "secondary",
                        "edge",
                        "faint",
                        "strong"
                    ][i],
                    colors[i]
                )
            });
            let mut css = css.join("\n");
            let alpha = alpha
                .ok()
                .and_then(|v| v["value"].as_f64())
                .unwrap_or(1.)
                .clamp(0.5, 1.);
            let contrast = contrast
                .ok()
                .and_then(|v| v["value"].as_f64())
                .unwrap_or(0.)
                .clamp(0., 1.);
            let alpha = alpha + (1. - alpha) * contrast;
            for ((name, color), offset) in ["wall", "console", "slab", "wash"]
                .iter()
                .zip(colors.iter())
                .zip([0.02_f64, 0.0, 0.04, 0.08])
            {
                css += &format!(
                    "\n@define-color {name} alpha({color},{});",
                    (alpha + offset).min(1.)
                );
            }
            css += &format!(
                "\n@define-color backbox alpha({},{});\n@define-color backbox_chrome alpha({},{});",
                colors[0],
                alpha.max(0.84),
                colors[1],
                alpha.max(0.84)
            );
            ui.appearance.load_from_string(&css);
            ui.wallpaper_dim.set_opacity(
                dim.ok()
                    .and_then(|v| v["value"].as_f64())
                    .unwrap_or(0.28)
                    .clamp(0., 0.85),
            );
            if let Ok(v) = image {
                crate::tools::settings::sync_wallpaper(&ui, &v["value"]);
                use base64::Engine;
                if let Some(data) = v["value"].as_str().and_then(|s| {
                    s.strip_prefix("data:image/jpeg;base64,")
                        .or_else(|| s.strip_prefix("data:image/png;base64,"))
                }) {
                    match base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .ok()
                        .and_then(|bytes| {
                            gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
                        }) {
                        Some(texture) => ui.wallpaper.set_paintable(Some(&texture)),
                        None => ui.show_error("The saved wallpaper could not be decoded."),
                    }
                } else {
                    ui.wallpaper.set_paintable(gtk::gdk::Paintable::NONE);
                }
            }

            ui.editor.set_palette(&ui.palette.borrow());
            for p in ui.panes.borrow().values() {
                p.apply_appearance(&ui.palette.borrow(), ui.font_size.get());
                p.schedule_resize();
            }
        });
    }
}
