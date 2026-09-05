use super::*;

struct Setup {
    ui: Rc<Ui>,
    panel: std::rc::Weak<crate::panel::Panel>,
    body: gtk::Box,
    workspace: RefCell<Value>,
    busy: Cell<bool>,
    message: gtk::Label,
    repos: RefCell<Vec<Value>>,
    choices: gtk::ComboBoxText,
    search: gtk::SearchEntry,
    connect: gtk::Button,
    add: gtk::Button,
    github: Cell<bool>,
    local: gtk::Entry,
    destination: gtk::Entry,
}

pub fn open(ui: &Rc<Ui>, workspace: Option<Value>) {
    let first = ui.projects.borrow().is_empty();
    let panel = crate::panel::Panel::page(
        ui,
        if first {
            "Welcome to Relay"
        } else {
            "Add project"
        },
    );
    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    body.set_halign(gtk::Align::Center);
    body.set_hexpand(true);
    body.set_size_request(580, -1);
    body.add_css_class("setup-body");
    panel.body.append(&body);
    let setup = Rc::new(Setup {
        ui: ui.clone(),
        panel: Rc::downgrade(&panel),
        body,
        workspace: RefCell::new(Value::Null),
        busy: Cell::new(false),
        message: label("", "dim"),
        repos: RefCell::default(),
        choices: gtk::ComboBoxText::new(),
        search: gtk::SearchEntry::new(),
        connect: button("Connect GitHub", "quiet"),
        add: button("Add project", "primary"),
        github: Cell::new(false),
        local: gtk::Entry::new(),
        destination: gtk::Entry::new(),
    });
    setup.connect.set_widget_name("setup-connect");
    setup.message.set_wrap(true);
    setup.message.set_max_width_chars(65);
    let weak = Rc::downgrade(&setup);
    panel.set_guard(move || weak.upgrade().is_none_or(|s| !s.busy.get()));
    let keep = setup.clone();
    panel.on_closed(move || {
        clear(&keep.body);
    });
    if let Some(workspace) = workspace {
        *setup.workspace.borrow_mut() = workspace;
        setup.project_step();
    } else {
        setup.workspace_step(first);
    }
    panel.present();
}

impl Setup {
    fn workspace_step(self: &Rc<Self>, first: bool) {
        self.body.append(&label(
            if first {
                "Create your first workspace"
            } else {
                "Choose a workspace"
            },
            "title",
        ));
        let copy=label("A workspace is a folder for your projects. Add a local repository or clone one from GitHub next.", "body");
        copy.set_wrap(true);
        copy.set_max_width_chars(65);
        self.body.append(&copy);
        let existing = gtk::ComboBoxText::new();
        existing.append(Some("new"), "Create a workspace");
        for ws in self.ui.workspaces.borrow().iter() {
            existing.append(
                Some(&ws["id"].to_string()),
                &format!("{} · {}", text(ws, "name"), text(ws, "path")),
            );
        }
        existing.set_active(Some(if self.ui.workspaces.borrow().is_empty() {
            0
        } else {
            1
        }));
        field("Workspace", &existing, &self.body);
        let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("My projects"));
        name.set_widget_name("setup-name");
        field("Name (optional)", &name, &form);
        let path = gtk::Entry::new();
        path.set_widget_name("setup-path");
        path.set_text(
            &std::env::var("HOME")
                .map(|home| format!("{home}/Projects"))
                .unwrap_or_default(),
        );
        field("Folder", &path, &form);
        browse(&self.ui, &form, &path, "Choose workspace folder");
        self.body.append(&form);
        form.set_visible(existing.active_id().as_deref() == Some("new"));
        let f = form.clone();
        existing.connect_changed(move |e| f.set_visible(e.active_id().as_deref() == Some("new")));
        let next = button("Continue", "primary");
        next.set_widget_name("setup-continue");
        self.body.append(&next);
        self.body.append(&self.message);
        let weak = Rc::downgrade(self);
        next.connect_clicked(move |key| {
            let Some(setup) = weak.upgrade() else {
                return;
            };
            if setup.busy.replace(true) {
                return;
            }
            let key = key.clone();
            key.set_sensitive(false);
            setup.message.set_text("Preparing workspace…");
            let setup = setup.clone();
            let id = existing.active_id().unwrap_or_default().to_string();
            let path = path.text().trim().to_string();
            let name = name.text().trim().to_string();
            glib::spawn_future_local(async move {
                let known = setup
                    .ui
                    .workspaces
                    .borrow()
                    .iter()
                    .find(|ws| {
                        ws["id"].as_i64() == id.parse::<i64>().ok()
                            || (id == "new" && text(ws, "path") == path)
                    })
                    .cloned();
                let result = if let Some(ws) = known {
                    Ok(ws)
                } else if !std::path::Path::new(&path).is_absolute() {
                    Err(Error::Protocol(
                        "Choose an absolute workspace folder path.".into(),
                    ))
                } else {
                    setup
                        .ui
                        .call(
                            "workspace.create",
                            json!({"path":path,"name":if name.is_empty(){None}else{Some(name)}}),
                        )
                        .await
                };
                setup.busy.set(false);
                key.set_sensitive(true);
                match result {
                    Ok(ws) => {
                        *setup.workspace.borrow_mut() = ws;
                        setup.ui.registry_dirty.set(true);
                        setup.ui.refresh();
                        setup.project_step();
                    }
                    Err(e) => setup.message.set_text(&e.to_string()),
                }
            });
        });
    }

    fn project_step(self: &Rc<Self>) {
        clear(&self.body);
        self.body.append(&label(
            if self.ui.projects.borrow().is_empty() {
                "Add your first project"
            } else {
                "Add a project"
            },
            "title",
        ));
        self.body.append(&label(
            &format!(
                "{} · {}",
                text(&self.workspace.borrow(), "name"),
                text(&self.workspace.borrow(), "path")
            ),
            "dim",
        ));
        let tabs = gtk::Stack::new();
        tabs.set_widget_name("setup-source");
        let switch = gtk::StackSwitcher::new();
        switch.set_stack(Some(&tabs));
        self.body.append(&switch);
        let local = gtk::Box::new(gtk::Orientation::Vertical, 12);
        self.local.set_widget_name("setup-local-path");
        field("Repository folder", &self.local, &local);
        browse(&self.ui, &local, &self.local, "Choose repository");
        let found = gtk::ComboBoxText::new();
        found.set_widget_name("setup-local-repos");
        field("Repositories in this workspace", &found, &local);
        let entry = self.local.clone();
        found.connect_changed(move |c| {
            if let Some(path) = c.active_id() {
                entry.set_text(&path);
            }
        });
        let github = gtk::Box::new(gtk::Orientation::Vertical, 12);
        github.append(&self.connect);
        self.search
            .set_placeholder_text(Some("Search your repositories"));
        github.append(&self.search);
        self.choices.set_widget_name("setup-github-repos");
        field("Repository", &self.choices, &github);
        self.destination.set_widget_name("setup-destination");
        field("Clone folder name", &self.destination, &github);
        tabs.add_titled(&local, Some("local"), "Local repository");
        tabs.add_titled(&github, Some("github"), "GitHub");
        self.body.append(&tabs);
        self.message.set_text("");
        self.body.append(&self.message);
        self.body.append(&self.add);
        self.add.set_widget_name("setup-add");
        let weak = Rc::downgrade(self);
        tabs.connect_visible_child_name_notify(move |t| {
            let Some(setup) = weak.upgrade() else {
                return;
            };
            let gh = t.visible_child_name().as_deref() == Some("github");
            setup.github.set(gh);
            setup.update_action();
            if gh {
                setup.load_github();
            } else {
                setup.message.set_text("");
            }
        });
        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(setup) = weak.upgrade() {
                setup.filter_repos();
            }
        });
        let weak = Rc::downgrade(self);
        self.choices.connect_changed(move |c| {
            let Some(setup) = weak.upgrade() else {
                return;
            };
            let repo = c.active_id().and_then(|id| {
                setup
                    .repos
                    .borrow()
                    .iter()
                    .find(|r| text(r, "full_name") == id)
                    .cloned()
            });
            setup
                .destination
                .set_text(repo.as_ref().map(|r| text(r, "name")).unwrap_or(""));
            setup.update_action();
        });
        let weak = Rc::downgrade(self);
        self.local.connect_changed(move |_| {
            if let Some(setup) = weak.upgrade() {
                setup.update_action();
            }
        });
        let weak = Rc::downgrade(self);
        self.destination.connect_changed(move |_| {
            if let Some(setup) = weak.upgrade() {
                setup.update_action();
            }
        });
        let weak = Rc::downgrade(self);
        self.connect.connect_clicked(move |_| {
            if let Some(setup) = weak.upgrade() {
                setup.connect_github();
            }
        });
        let weak = Rc::downgrade(self);
        self.add.connect_clicked(move |_| {
            if let Some(setup) = weak.upgrade() {
                setup.submit();
            }
        });
        self.update_action();
        let setup = self.clone();
        glib::spawn_future_local(async move {
            let path = setup.workspace.borrow()["path"].clone();
            match setup
                .ui
                .call("workspace.discover", json!({"path":path}))
                .await
            {
                Ok(v) => {
                    for repo in rows(&v, "repositories").into_iter().filter(|r| {
                        !setup
                            .ui
                            .projects
                            .borrow()
                            .iter()
                            .any(|p| p["path"] == r["path"])
                    }) {
                        found.append(Some(text(&repo, "path")), text(&repo, "name"));
                    }
                    found.set_active(Some(0));
                }
                Err(e) => setup.message.set_text(&e.to_string()),
            }
        });
    }

    fn update_action(&self) {
        self.add.set_label(if self.github.get() {
            "Clone and open project"
        } else {
            "Open project"
        });
        self.add.set_sensitive(
            !self.busy.get()
                && if self.github.get() {
                    self.choices.active_id().is_some() && !self.destination.text().trim().is_empty()
                } else {
                    !self.local.text().trim().is_empty()
                },
        );
    }
    fn filter_repos(&self) {
        let previous = self.choices.active_id();
        self.choices.remove_all();
        let query = self.search.text().to_lowercase();
        for repo in self
            .repos
            .borrow()
            .iter()
            .filter(|r| text(r, "full_name").to_lowercase().contains(&query))
        {
            self.choices.append(
                Some(text(repo, "full_name")),
                &format!(
                    "{}{}{}",
                    text(repo, "full_name"),
                    if repo["private"] == true {
                        " · private"
                    } else {
                        ""
                    },
                    if repo["archived"] == true {
                        " · archived"
                    } else {
                        ""
                    }
                ),
            );
        }
        if !previous.is_some_and(|id| self.choices.set_active_id(Some(&id))) {
            self.choices.set_active(Some(0));
        }
        self.update_action();
    }
    fn load_github(self: &Rc<Self>) {
        self.connect.set_sensitive(false);
        self.message.set_text("Checking GitHub…");
        let setup = self.clone();
        glib::spawn_future_local(async move {
            let result=async {
                let status=setup.ui.call("github.status",json!({})).await?;
                if status["connected"]!=true {
                    setup.repos.borrow_mut().clear(); setup.filter_repos();
                    setup.message.set_text(if status["installed"]==true {"Connect GitHub to browse your repositories. Local repositories also work without an account."} else {"Install the GitHub CLI (gh), then connect your account. You can use a local repository now."});
                    setup.connect.set_label("Connect GitHub"); return Ok::<(),Error>(());
                }
                setup.connect.set_label("Refresh repositories");
                let repos=setup.ui.call("github.repo.list",json!({})).await?;
                *setup.repos.borrow_mut()=rows(&repos,"repositories"); setup.filter_repos();
                setup.message.set_text(&format!("Connected as {} · {} repositories",text(&status,"login"),setup.repos.borrow().len())); Ok(())
            }.await;
            if let Err(e) = result {
                setup.message.set_text(&e.to_string());
            }
            setup.connect.set_sensitive(true);
        });
    }
    fn connect_github(self: &Rc<Self>) {
        if self.connect.label().as_deref() == Some("Refresh repositories") {
            self.load_github();
            return;
        }
        self.connect.set_sensitive(false);
        self.message.set_text(
            "Complete GitHub sign-in in your browser. The device code is copied to your clipboard.",
        );
        let setup = self.clone();
        let task = glib::spawn_future_local(async move {
            let result = async {
                let (client, notices) =
                    Client::connect(&setup.ui.rt, setup.ui.path.clone()).await?;
                client
                    .request(
                        &setup.ui.rt,
                        "bus.subscribe",
                        json!({"events":["github.changed"]}),
                    )
                    .await?;
                client
                    .request(&setup.ui.rt, "github.connect", json!({}))
                    .await?;
                while let Ok(notice) = notices.recv().await {
                    match notice {
                        Notice::Event(e) if e.ev == "github.changed" => {
                            setup.load_github();
                            return Ok::<(), Error>(());
                        }
                        Notice::Disconnected(e) => return Err(e),
                        _ => {}
                    }
                }
                Err(Error::Disconnected)
            }
            .await;
            if let Err(e) = result {
                setup.message.set_text(&e.to_string());
                setup.connect.set_sensitive(true);
            }
        });
        if let Some(panel) = self.panel.upgrade() {
            panel.on_closed(move || task.abort());
        }
    }
    fn submit(self: &Rc<Self>) {
        if self.busy.replace(true) {
            return;
        }
        self.update_action();
        self.body.set_sensitive(false);
        let gh = self.github.get();
        self.message.set_text(if gh {
            "Cloning repository…"
        } else {
            "Opening repository…"
        });
        let setup = self.clone();
        let selected = self.choices.active_id().and_then(|id| {
            self.repos
                .borrow()
                .iter()
                .find(|r| text(r, "full_name") == id)
                .cloned()
        });
        let mut payload = json!({"workspace_id":self.workspace.borrow()["id"]});
        if gh {
            payload["url"] = selected.unwrap_or_default()["clone_url"].clone();
            payload["dest"] = json!(self.destination.text().trim());
        } else {
            payload["path"] = json!(self.local.text().trim());
        }
        glib::spawn_future_local(async move {
            let result = setup
                .ui
                .call(if gh { "project.clone" } else { "project.add" }, payload)
                .await;
            setup.busy.set(false);
            setup.body.set_sensitive(true);
            setup.update_action();
            match result {
                Ok(v) => {
                    let project = if gh { &v["project"] } else { &v };
                    if let Some(panel) = setup.panel.upgrade() {
                        panel.close();
                    }
                    setup.ui.registry_dirty.set(true);
                    setup
                        .ui
                        .open_project(project["id"].as_i64().unwrap_or(0), "agents");
                    setup.ui.refresh();
                }
                Err(e) => setup.message.set_text(&e.to_string()),
            }
        });
    }
}

fn browse(ui: &Rc<Ui>, body: &gtk::Box, entry: &gtk::Entry, title: &'static str) {
    let key = button("Browse…", "quiet");
    body.append(&key);
    let weak = Rc::downgrade(ui);
    let entry = entry.clone();
    key.connect_clicked(move |_| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let entry = entry.clone();
        glib::spawn_future_local(async move {
            let dialog = gtk::FileDialog::builder().title(title).build();
            if let Ok(file) = dialog.select_folder_future(Some(&ui.window)).await {
                if let Some(path) = file.path() {
                    entry.set_text(&path.to_string_lossy());
                }
            }
        });
    });
}
