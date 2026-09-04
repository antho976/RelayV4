use super::*;
use vte4::prelude::*;

impl Ui {
    pub fn open_project(self: &Rc<Self>, project: i64, page: &str) {
        if project != self.project.get() {
            if self.editor.is_dirty() {
                self.show_error("Save or discard editor changes before switching projects.");
                return;
            }
            self.project.set(project);
            self.editor.reset();
            self.restored_project.set(0);
            self.refresh();
        }
        self.navigate(page);
    }
    pub(super) fn render_projects(self: &Rc<Self>) {
        // Session traffic must not replace sidebar widgets or keyboard focus.
        let signature = format!(
            "{}:{}:{}",
            self.project.get(),
            serde_json::to_string(&*self.projects.borrow()).unwrap_or_default(),
            serde_json::to_string(&*self.workspaces.borrow()).unwrap_or_default()
        );
        if self.projects_box.widget_name() == signature {
            return;
        }
        self.projects_box.set_widget_name(&signature);
        clear(&self.projects_box);
        let projects = self.projects.borrow().clone();
        for workspace in self.workspaces.borrow().iter() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            row.add_css_class("workspace-row");
            let caption = label(text(workspace, "name"), "dim");
            caption.set_hexpand(true);
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&crate::icons::image("chevron-down", 14));
            row.append(&caption);
            let manage = icon_button("view-more-symbolic", "Workspace settings");
            row.append(&manage);
            let weak = Rc::downgrade(self);
            let workspace_menu = workspace.clone();
            manage.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.registry_editor(workspace_menu.clone(), true);
                }
            });
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
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let name = text(project, "name");
                let id = project["id"].as_i64().unwrap_or(0);
                let b = button(
                    &format!(
                        "{name}{}",
                        if id == self.project.get() {
                            format!("\n{}", text(project, "base_branch"))
                        } else {
                            String::new()
                        }
                    ),
                    "project",
                );
                b.set_hexpand(true);
                if id == self.project.get() {
                    b.add_css_class("selected");
                }
                if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                }
                let weak = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.open_project(id, "agents");
                    }
                });
                row.append(&b);
                let menu = icon_button("view-more-symbolic", "Project settings");
                row.append(&menu);
                let weak = Rc::downgrade(self);
                let project = project.clone();
                menu.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.registry_editor(project.clone(), false);
                    }
                });
                self.projects_box.append(&row);
            }
        }
    }
    fn registry_editor(self: &Rc<Self>, value: Value, workspace: bool) {
        let (window, body) = self.sheet(if workspace { "Workspace" } else { "Project" }, 420, 480);
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
                    "Forget this registration? Files on disk remain in place.",
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
    pub(super) fn sheet(&self, title: &str, width: i32, height: i32) -> (gtk::Window, gtk::Box) {
        let window = gtk::Window::builder()
            .title(title)
            .transient_for(&self.window)
            .application(&self.window.application().unwrap())
            .default_width(width)
            .default_height(height)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.add_css_class("page");
        window.set_child(Some(&scrolled(&body)));
        (window, body)
    }
    fn confirm_mutation(
        self: &Rc<Self>,
        message: &str,
        op: &'static str,
        payload: Value,
        parent: Option<gtk::Window>,
    ) {
        let dialog = gtk::AlertDialog::builder()
            .message(message)
            .buttons(["Cancel", "Confirm"])
            .cancel_button(0)
            .default_button(0)
            .build();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&ui.window)).await == Ok(1) {
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
        let names: Vec<_> = self
            .ordered
            .borrow()
            .iter()
            .filter(|n| !self.satellites.borrow().contains_key(*n))
            .cloned()
            .collect();
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
            mode == "review" || (mode == "grid" && self.columns.get() == 2 && names.len() > 1),
        );
        clear(&self.focus_tabs);
        self.focus_tabs
            .set_visible(mode == "focus" && names.len() > 1);
        for (i, name) in names.iter().enumerate() {
            if mode == "focus" {
                let b = button(name, "quiet");
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
                if mode == "review" && names.len() > 1 {
                    if focus.as_ref() == Some(name) {
                        self.wall.attach(&p.root, 0, 0, 1, 1);
                    } else {
                        let offset = names[..i]
                            .iter()
                            .filter(|n| Some(*n) != focus.as_ref())
                            .count() as i32;
                        self.wall_right.attach(&p.root, 0, offset, 1, 1);
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
        let project = self.project.get();
        if project == 0 || self.client.borrow().is_none() || self.restored_project.get() != project
        {
            return;
        }
        let state = self.layout_state();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = ui
                .call(
                    "settings.set",
                    json!({"path":format!("native.layout.current.{project}"),"value":state}),
                )
                .await
            {
                ui.show_error(&e.to_string());
            }
        });
    }
    fn layout_state(&self) -> Value {
        json!({"page":*self.page.borrow(),"agent_layout":*self.mode.borrow(),"columns":self.columns.get(),"focused":*self.focused.borrow(),"order":*self.ordered.borrow(),"sidebar":self.sidebar.is_visible(),"split":self.wall_split.position(),"width":self.window.width(),"height":self.window.height()})
    }
    pub(super) async fn restore_layout(self: &Rc<Self>) {
        let project = self.project.get();
        if self.restored_project.get() == project {
            return;
        }
        self.restored_project.set(project);
        let result = self
            .call(
                "settings.get",
                json!({"path":format!("native.layout.current.{project}")}),
            )
            .await;
        if self.project.get() != project {
            return;
        }
        if let Ok(v) = result {
            if !v["value"].is_null() {
                self.apply_layout(&v["value"]);
            }
        }
        self.refresh_page();
    }
    pub(super) fn apply_layout(self: &Rc<Self>, state: &Value) {
        if let Some(mode) = state["agent_layout"]
            .as_str()
            .filter(|s| matches!(*s, "grid" | "focus" | "review"))
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
            self.sidebar.set_visible(visible);
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
                        *self.focused.borrow_mut() = Some(session.into());
                        self.set_mode("focus");
                        self.navigate("agents");
                    }
                }
            }
        }
        self.applying_ui.set(false);
    }
    pub(super) fn layout_menu(self: &Rc<Self>) {
        let (window, body) = self.sheet("Window presets", 390, 420);
        let modes = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        body.append(&modes);
        for (mode, caption) in [("grid", "Grid"), ("focus", "Focus"), ("review", "Review")] {
            let b = button(caption, "quiet");
            if *self.mode.borrow() == mode {
                b.add_css_class("selected");
            }
            modes.append(&b);
            let weak = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_mode(mode);
                }
            });
        }
        let columns = gtk::DropDown::from_strings(&["One column", "Two columns", "Three columns"]);
        columns.set_selected((self.columns.get() - 1) as u32);
        body.append(&columns);
        let weak = Rc::downgrade(self);
        columns.connect_selected_notify(move |d| {
            if let Some(ui) = weak.upgrade() {
                ui.columns.set(d.selected() as i32 + 1);
                ui.set_mode("grid");
            }
        });
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
        let weak = Rc::downgrade(self);
        let n = name.to_string();
        action.connect_clicked(move |b| {
            if let Some(ui) = weak.upgrade() {
                ui.mutate(op, json!({"session":n}), b);
            }
        });
        pane.actions.append(&action);
        let zoom = icon_button("view-fullscreen-symbolic", "Focus terminal");
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
        let (window, body) = self.sheet(&name, 440, 620);
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
        let pop = button("Open in separate window", "quiet");
        body.append(&pop);
        let weak = Rc::downgrade(self);
        let n = name.clone();
        pop.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.popout(&n);
            }
        });
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
                            let (w, b) = ui.sheet("Session brief", 760, 640);
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
        if matches!(text(&session, "state"), "restorable" | "exited") {
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
    fn popout(self: &Rc<Self>, name: &str) {
        if let Some(w) = self.satellites.borrow().get(name) {
            w.present();
            return;
        }
        let Some(p) = self.panes.borrow().get(name).cloned() else {
            return;
        };
        if let Some(grid) = p.root.parent().and_downcast::<gtk::Grid>() {
            grid.remove(&p.root);
        }
        let window = gtk::Window::builder()
            .application(&self.window.application().unwrap())
            .title(name)
            .default_width(900)
            .default_height(650)
            .build();
        window.set_child(Some(&p.root));
        self.satellites
            .borrow_mut()
            .insert(name.into(), window.clone());
        let weak = Rc::downgrade(self);
        let n = name.to_string();
        window.connect_close_request(move |w| {
            w.set_child(gtk::Widget::NONE);
            if let Some(ui) = weak.upgrade() {
                ui.satellites.borrow_mut().remove(&n);
                ui.layout();
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&p);
        window.connect_realize(move |w| {
            if let Some(surface) = w.surface() {
                let weak = weak.clone();
                surface.connect_layout(move |_, _, _| {
                    if let Some(p) = weak.upgrade() {
                        p.schedule_resize();
                    }
                });
            }
        });
        window.present();
        self.layout();
    }
    pub(super) fn command_palette(self: &Rc<Self>) {
        let (window, body) = self.sheet("Command palette", 520, 480);
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
            ("plan", "Plan"),
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
            if key == gtk::gdk::Key::Escape && ui.launch.reveals_child() {
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
        let (window, body) = self.sheet("Provider usage", 400, 420);
        let refresh = button("Refresh", "quiet");
        body.append(&refresh);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.append(&list);
        let weak = Rc::downgrade(self);
        refresh.connect_clicked(move |key| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let key = key.clone();
            let list = list.clone();
            key.set_sensitive(false);
            glib::spawn_future_local(async move {
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
            });
        });
        refresh.emit_clicked();
        window.present();
    }
    pub(super) fn resources(self: &Rc<Self>) {
        let (w, b) = self.sheet("Resources", 440, 360);
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match ui.call("app.resources.get", json!({})).await {
                Ok(v) => {
                    b.append(&label(
                        &format!(
                            "{:.0} MB agents · {:.0} MB Relay",
                            v["total_rss_mb"].as_f64().unwrap_or(0.),
                            v["relay"]["rss_mb"].as_f64().unwrap_or(0.)
                        ),
                        "title",
                    ));
                    for p in rows(&v, "panes") {
                        b.append(&label(
                            &format!(
                                "{} · {:.0} MB · {:.1}% CPU",
                                text(&p, "session"),
                                p["rss_mb"].as_f64().unwrap_or(0.),
                                p["cpu_pct"].as_f64().unwrap_or(0.)
                            ),
                            "body",
                        ));
                    }
                    b.append(&label(
                        &format!("{:.1} MB Relay store", v["store_mb"].as_f64().unwrap_or(0.)),
                        "dim",
                    ));
                    for wt in rows(&v, "worktrees") {
                        b.append(&label(
                            &format!(
                                "{} · {:.0} MB",
                                text(&wt, "path"),
                                wt["disk_mb"].as_f64().unwrap_or(0.)
                            ),
                            "dim",
                        ));
                    }
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
        });
        w.present();
    }
    pub(super) fn load_wall_files(self: &Rc<Self>, directory: String) {
        self.file_tree_revision
            .set(self.file_tree_revision.get() + 1);
        let revision = self.file_tree_revision.get();
        let project = self.project.get();
        let ui = self.clone();
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "file.tree",
                    json!({"project_id":project,"path":directory,"depth":1,"git_badges":true}),
                )
                .await
            {
                Ok(v) if ui.project.get() == project && ui.file_tree_revision.get() == revision => {
                    clear(&ui.wall_files);
                    if !directory.is_empty() {
                        let up = button("Parent folder", "quiet");
                        let weak = Rc::downgrade(&ui);
                        let parent = std::path::Path::new(&directory)
                            .parent()
                            .unwrap_or(std::path::Path::new(""))
                            .to_string_lossy()
                            .to_string();
                        up.connect_clicked(move |_| {
                            if let Some(ui) = weak.upgrade() {
                                ui.load_wall_files(parent.clone());
                            }
                        });
                        ui.wall_files.append(&up);
                    }
                    for entry in rows(&v, "entries") {
                        let dir = text(&entry, "kind") == "dir";
                        let path = text(&entry, "path").to_string();
                        let b = button(
                            &format!("{}{}", text(&entry, "name"), if dir { "/" } else { "" }),
                            "file",
                        );
                        let weak = Rc::downgrade(&ui);
                        b.connect_clicked(move |_| {
                            if let Some(ui) = weak.upgrade() {
                                if dir {
                                    ui.load_wall_files(path.clone());
                                } else {
                                    ui.navigate("code");
                                    ui.editor.open_path(&ui, path.clone(), Some(String::new()));
                                }
                            }
                        });
                        ui.wall_files.append(&b);
                    }
                }
                Ok(_) => {}
                Err(e) => ui.show_error(&e.to_string()),
            }
        });
    }
    pub(super) fn load_appearance(self: &Rc<Self>) {
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
                    .set(value["value"].as_f64().unwrap_or(10.0).clamp(8.0, 24.0));
            }
            let colors = match ui.palette.borrow().as_str() {
                "dark" => [
                    "#0a0b0d", "#101114", "#16171b", "#1e1f24", "#08090a", "#eef0f2", "#a3a7ad",
                    "#202228",
                ],
                "oled" => [
                    "#000000", "#000000", "#0d0d0e", "#161618", "#000000", "#ececea", "#a5a5a3",
                    "#1f1f22",
                ],
                _ => [
                    "#0e0e10", "#141416", "#1b1b1e", "#232327", "#0a0a0b", "#ececea", "#a5a5a3",
                    "#252529",
                ],
            };
            let css: [String; 8] = std::array::from_fn(|i| {
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
                        "edge"
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
            for (name, color) in ["wall", "console", "slab", "wash"]
                .iter()
                .zip(colors.iter())
            {
                css += &format!("\n@define-color {name} alpha({color},{alpha});");
            }
            ui.appearance.load_from_string(&css);
            ui.wallpaper_dim.set_opacity(
                dim.ok()
                    .and_then(|v| v["value"].as_f64())
                    .unwrap_or(0.28)
                    .clamp(0., 0.85),
            );
            if let Ok(v) = image {
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

            for p in ui.panes.borrow().values() {
                p.apply_appearance(&ui.palette.borrow(), ui.font_size.get());
                p.schedule_resize();
            }
        });
    }
}
