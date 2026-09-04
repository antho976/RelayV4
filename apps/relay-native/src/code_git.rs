use super::*;
use crate::client::Error;

impl Editor {
    pub(super) fn refresh_git(self: &Rc<Self>, ui: &Rc<Ui>) {
        if ui.project.get() == 0 {
            return;
        }
        self.git_revision.set(self.git_revision.get() + 1);
        let revision = self.git_revision.get();
        let project = ui.project.get();
        let worktree = self.worktree.borrow().clone();
        let payload = self.payload(ui, json!({}));
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = ui.call("git.status", payload.clone()).await;
            if !e.matches(&ui, project, &worktree) || revision != e.git_revision.get() {
                return;
            }
            let status = match result {
                Ok(v) => v,
                Err(err) => {
                    ui.show_error(&err.to_string());
                    return;
                }
            };
            clear(&e.git);
            let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let branch = label(text(&status, "branch"), "title");
            branch.set_hexpand(true);
            branch.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            header.append(&branch);
            let refresh = button("Refresh", "quiet");
            let ed = e.clone();
            let weak = Rc::downgrade(&ui);
            refresh.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ed.refresh_git(&ui);
                    ed.refresh_scopes(&ui);
                }
            });
            header.append(&refresh);
            e.git.append(&header);
            e.git.append(&label(
                &format!("{} ahead · {} behind", status["ahead"], status["behind"]),
                "dim",
            ));
            let changes = rows(&status, "files");
            e.git
                .append(&label(&format!("CHANGES · {}", changes.len()), "dim"));
            if changes.is_empty() {
                e.git.append(&label("Working tree clean", "dim"));
            }
            for file in &changes {
                let path = text(file, "path").to_string();
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                let open = button(
                    &format!(
                        "{}{}  {}",
                        text(file, "index"),
                        text(file, "worktree"),
                        path
                    ),
                    "file",
                );
                open.set_hexpand(true);
                open.set_tooltip_text(Some(&path));
                if let Some(label) = open.child().and_downcast::<gtk::Label>() {
                    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                    label.set_xalign(0.0);
                }
                let ed = e.clone();
                let weak = Rc::downgrade(&ui);
                let open_path = path.clone();
                open.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ed.open_diff(&ui, open_path.clone());
                    }
                });
                row.append(&open);
                // Partially staged files expose both actions; neither half is hidden.
                for (title, op, enabled) in [
                    ("+", "git.stage", !text(file, "worktree").trim().is_empty()),
                    (
                        "−",
                        "git.unstage",
                        !text(file, "index").trim().is_empty() && text(file, "index") != "?",
                    ),
                ] {
                    if !enabled {
                        continue;
                    }
                    let b = button(title, "quiet");
                    b.set_tooltip_text(Some(if op == "git.stage" {
                        "Stage file"
                    } else {
                        "Unstage file"
                    }));
                    let ed = e.clone();
                    let weak = Rc::downgrade(&ui);
                    let path = path.clone();
                    b.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            ed.git_action(&ui, op, json!({"paths":[path]}), None);
                        }
                    });
                    row.append(&b);
                }
                e.git.append(&row);
            }
            let message = e.commit_message.clone();
            detach(&message);
            message.set_placeholder_text(Some("Commit message"));
            e.git.append(&message);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for (title, all) in [("Commit staged", false), ("Commit all", true)] {
                let b = button(title, if all { "quiet" } else { "primary" });
                b.set_sensitive(!changes.is_empty());
                let ed = e.clone();
                let weak = Rc::downgrade(&ui);
                let message = message.clone();
                b.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        let content = message.text().trim().to_string();
                        if content.is_empty() {
                            ui.show_error("Enter a commit message first.");
                            return;
                        }
                        ed.git_action(
                            &ui,
                            "git.commit",
                            json!({"message":content,"all":all}),
                            Some(message.clone()),
                        );
                    }
                });
                actions.append(&b);
            }
            e.git.append(&actions);
            let suggest = button("Suggest message", "quiet");
            let ed = e.clone();
            let weak = Rc::downgrade(&ui);
            let message = message.clone();
            suggest.connect_clicked(move |_| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let ed = ed.clone();
                let field = message.clone();
                let original = field.text();
                let project = ui.project.get();
                let worktree = ed.worktree.borrow().clone();
                glib::spawn_future_local(async move {
                    match ui
                        .call("git.suggest_message", ed.payload(&ui, json!({})))
                        .await
                    {
                        Ok(v)
                            if ed.matches(&ui, project, &worktree) && field.text() == original =>
                        {
                            field.set_text(text(&v, "message"))
                        }
                        Ok(_) => {}
                        Err(err) => ui.show_error(&err.to_string()),
                    }
                });
            });
            e.git.append(&suggest);
            let remote = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for (title, op) in [
                ("Fetch", "git.fetch"),
                ("Push", "git.push"),
                ("Open PR", "git.pr.open"),
            ] {
                let b = button(title, "quiet");
                let ed = e.clone();
                let weak = Rc::downgrade(&ui);
                b.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ed.git_action(&ui, op, json!({}), None);
                    }
                });
                remote.append(&b);
            }
            e.git.append(&remote);
            let branches_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let branches = gtk::Expander::builder()
                .label("Branches")
                .child(&branches_box)
                .build();
            e.git.append(&branches);
            let history_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let history = gtk::Expander::builder()
                .label("History")
                .child(&history_box)
                .build();
            e.git.append(&history);
            let results = tokio::join!(
                ui.call("git.branches", payload.clone()),
                ui.call("git.log", e.payload(&ui, json!({"limit":30}))),
                ui.call("integration.list", json!({"project_id":project}))
            );
            if !e.matches(&ui, project, &worktree) || revision != e.git_revision.get() {
                return;
            }
            let merge = gtk::Box::new(gtk::Orientation::Vertical, 6);
            e.git.append(
                &gtk::Expander::builder()
                    .label("Merge test")
                    .child(&merge)
                    .build(),
            );
            let hint = label(
                "Test agent branches together in a disposable worktree.",
                "dim",
            );
            hint.set_wrap(true);
            merge.append(&hint);
            let mut picks = Vec::new();
            for session in ui
                .sessions
                .borrow()
                .iter()
                .filter(|s| text(s, "state") != "closed")
            {
                let name = text(session, "name").to_string();
                let pick = gtk::CheckButton::with_label(&name);
                merge.append(&pick);
                picks.push((name, pick));
            }
            let test = button("Test merge + build", "quiet");
            merge.append(&test);
            let weak = Rc::downgrade(&ui);
            let ed = e.clone();
            test.connect_clicked(move |key| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let selected: Vec<_> = picks
                    .iter()
                    .filter(|(_, p)| p.is_active())
                    .map(|(n, _)| n.clone())
                    .collect();
                if selected.len() < 2 {
                    ui.show_error("Select at least two agents to test together.");
                    return;
                }
                key.set_sensitive(false);
                let key = key.clone();
                let ed = ed.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = ui
                        .call(
                            "integration.request",
                            json!({"project_id":project,"sessions":selected,"build":true}),
                        )
                        .await
                    {
                        ui.show_error(&e.to_string());
                    }
                    key.set_sensitive(true);
                    if ui.project.get() == project {
                        ed.refresh_git(&ui);
                    }
                });
            });
            match results.2 {
                Ok(v) => {
                    for run in rows(&v, "integrations").iter().take(3) {
                        let l = label(
                            &format!(
                                "#{} · {}{}",
                                run["id"],
                                text(run, "state"),
                                run["conflict"]
                                    .as_array()
                                    .map(|v| format!(
                                        " · {}",
                                        v.iter()
                                            .filter_map(Value::as_str)
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    ))
                                    .unwrap_or_default()
                            ),
                            "dim",
                        );
                        l.set_wrap(true);
                        merge.append(&l);
                    }
                }
                Err(err) => merge.append(&label(&err.to_string(), "dim")),
            }
            match results.0 {
                Ok(v) => e.render_branches(&ui, &branches_box, &v),
                Err(err) => branches_box.append(&label(&err.to_string(), "dim")),
            }
            match results.1 {
                Ok(v) => {
                    for commit in rows(&v, "commits") {
                        let sha = text(&commit, "sha").to_string();
                        let b = button(
                            &format!("{} {}", &sha[..sha.len().min(7)], text(&commit, "subject")),
                            "file",
                        );
                        b.set_tooltip_text(Some(&format!(
                            "{}\n{}\n{}",
                            text(&commit, "author"),
                            text(&commit, "at"),
                            sha
                        )));
                        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                            l.set_xalign(0.0);
                        }
                        let ed = e.clone();
                        let weak = Rc::downgrade(&ui);
                        b.connect_clicked(move |_| {
                            if let Some(ui) = weak.upgrade() {
                                ed.show_commit(&ui, sha.clone());
                            }
                        });
                        history_box.append(&b);
                    }
                }
                Err(err) => history_box.append(&label(&err.to_string(), "dim")),
            }
        });
    }
    fn render_branches(self: &Rc<Self>, ui: &Rc<Ui>, container: &gtk::Box, v: &Value) {
        for branch in rows(v, "branches") {
            let name = text(&branch, "name").to_string();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let title = label(
                &format!(
                    "{}{}",
                    if branch["current"] == true {
                        "• "
                    } else {
                        ""
                    },
                    name
                ),
                "body",
            );
            title.set_hexpand(true);
            title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            row.append(&title);
            let deletable = branch["merged"] == true
                && branch["session"].is_null()
                && branch["current"] != true
                && !self
                    .worktrees
                    .borrow()
                    .iter()
                    .any(|w| text(w, "branch") == name)
                && !ui
                    .projects
                    .borrow()
                    .iter()
                    .any(|p| p["id"] == ui.project.get() && text(p, "base_branch") == name);
            if deletable {
                let b = button("Delete", "quiet");
                let e = self.clone();
                let weak = Rc::downgrade(ui);
                b.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        e.git_action(&ui, "git.branch.delete", json!({"name":name}), None);
                    }
                });
                row.append(&b);
            }
            container.append(&row);
        }
        let name = self.branch_name.clone();
        detach(&name);
        name.set_placeholder_text(Some("New branch name"));
        container.append(&name);
        let start = self.branch_start.clone();
        detach(&start);
        start.set_placeholder_text(Some("Start point (optional)"));
        container.append(&start);
        let create = button("Create and switch", "quiet");
        let e = self.clone();
        let weak = Rc::downgrade(ui);
        create.connect_clicked(move |_|{
            let Some(ui)=weak.upgrade()else{return;};
            if name.text().trim().is_empty(){ui.show_error("Enter a branch name.");return;}
            e.git_action(&ui,"git.branch.create",json!({"name":name.text().trim(),"start_point":optional_scope(start.text().trim()),"checkout":true}),Some(name.clone()));
        });
        container.append(&create);
    }
    fn git_action(
        self: &Rc<Self>,
        ui: &Rc<Ui>,
        op: &'static str,
        extra: Value,
        input: Option<gtk::Entry>,
    ) {
        if self.is_dirty() {
            ui.show_error("Save or discard your editor changes before changing Git state.");
            return;
        }
        let mut payload = self.payload(ui, extra);
        if matches!(op, "git.fetch" | "git.branch.delete") {
            payload.as_object_mut().unwrap().remove("worktree");
        }
        let project = ui.project.get();
        let worktree = self.worktree.borrow().clone();
        let revision = self.revision.get();
        let input_snapshot = input.as_ref().map(|e| e.text());
        let e = self.clone();
        let ui = ui.clone();
        self.git_busy.set(true);
        self.git.set_sensitive(false);
        self.view.set_editable(false);
        glib::spawn_future_local(async move {
            let result=async {
                if matches!(op,"git.push"|"git.pr.open"|"git.branch.delete"){
                    let explanation=match op{"git.push"=>"Push this worktree's branch to its remote?","git.pr.open"=>"Publish a pull request for this worktree?",_=>"Delete this merged local branch? The engine refuses protected, checked-out and session-owned branches."};
                    if !confirm(&ui,op,&format!("{explanation}\n{}",serde_json::to_string_pretty(&payload).unwrap_or_default())).await {return Ok(None);}
                }
                guarded(&ui,op,payload).await
            }.await;
            e.git_busy.set(false);
            e.git.set_sensitive(true);
            if !e.matches(&ui, project, &worktree) || revision != e.revision.get() {
                return;
            }
            e.set_busy(false);
            match result {
                Ok(Some(v)) => {
                    if let (Some(field), Some(snapshot)) = (input, input_snapshot) {
                        if field.text() == snapshot {
                            field.set_text("");
                        }
                    }
                    if let Some(url) = v["url"].as_str() {
                        ui.show_error(url);
                    } else if let Some(sha) = v["sha"].as_str() {
                        ui.show_error(&format!("Committed {}", &sha[..sha.len().min(7)]));
                    }
                    if op == "git.branch.create" {
                        e.clear_document();
                        e.load_tree(&ui, None);
                    }
                    e.refresh_git(&ui);
                    e.refresh_scopes(&ui);
                }
                Ok(None) => {}
                Err(err) => ui.show_error(&err.to_string()),
            }
        });
    }
    fn open_diff(self: &Rc<Self>, ui: &Rc<Ui>, path: String) {
        if self.is_dirty() {
            ui.show_error("Save or discard your edits before opening a diff.");
            return;
        }
        self.revision.set(self.revision.get() + 1);
        let revision = self.revision.get();
        let project = ui.project.get();
        let worktree = self.worktree.borrow().clone();
        let payload = self.payload(ui, json!({"path":path}));
        self.set_busy(true);
        let e = self.clone();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            let result = ui.call("git.diff.file", payload).await;
            if revision != e.revision.get() {
                return;
            }
            e.set_busy(false);
            if !e.matches(&ui, project, &worktree) {
                return;
            }
            match result {
                Ok(v) => {
                    let old = text(&v, "old");
                    let new = text(&v, "new");
                    if old.len() + new.len() > 1048576 {
                        ui.show_error("Diff exceeds the 1 MiB editor limit.");
                        return;
                    }
                    e.diff.set(true);
                    e.before.set_text(old);
                    e.buffer.set_text(new);
                    e.buffer.set_modified(false);
                    e.before_scroll.set_visible(true);
                    *e.path.borrow_mut() = path.clone();
                    e.project.set(project);
                    e.set_busy(false);
                    let language =
                        sourceview5::LanguageManager::default().guess_language(Some(&path), None);
                    e.buffer.set_language(language.as_ref());
                    e.before.set_language(language.as_ref());
                    e.caption.set_text(&format!("{path} · HEAD / working tree"));
                    e.position
                        .set_text("HEAD on left · working tree on right · read only");
                }
                Err(err) => ui.show_error(&err.to_string()),
            }
        });
    }
    fn show_commit(self: &Rc<Self>, ui: &Rc<Ui>, sha: String) {
        let project = ui.project.get();
        let ui = ui.clone();
        glib::spawn_future_local(async move {
            match ui
                .call("git.show", json!({"project_id":project,"sha":sha}))
                .await
            {
                Ok(v) if ui.project.get() == project => {
                    let dialog = crate::panel::Panel::new(&ui, "Commit details", 720);
                    let view = gtk::TextView::new();
                    view.set_editable(false);
                    view.set_monospace(true);
                    view.set_wrap_mode(gtk::WrapMode::WordChar);
                    let commit = &v["commit"];
                    let mut copy = format!(
                        "{}\n{}\n{} · {}\n\n{}\n\n",
                        text(commit, "sha"),
                        text(commit, "subject"),
                        text(commit, "author"),
                        text(commit, "at"),
                        text(commit, "body")
                    );
                    for file in rows(&v, "files") {
                        copy.push_str(&format!(
                            "{}  +{} −{}  {}\n",
                            text(&file, "status"),
                            file["added"],
                            file["removed"],
                            text(&file, "path")
                        ));
                    }
                    view.buffer().set_text(&copy);
                    dialog.body.append(&scrolled(&view));
                    dialog.present();
                }
                Ok(_) => {}
                Err(err) => ui.show_error(&err.to_string()),
            }
        });
    }
}

async fn confirm(ui: &Ui, title: &str, copy: &str) -> bool {
    let dialog = crate::panel::Panel::new(ui, title, 620);
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.buffer().set_text(copy);
    dialog.body.append(&scrolled(&view));
    dialog.response("Continue").await
}

async fn guarded(ui: &Ui, op: &str, payload: Value) -> Result<Option<Value>, Error> {
    match ui.call(op, payload.clone()).await {
        Ok(v) => Ok(Some(v)),
        Err(Error::Bus(err)) if err.kind == relay_bus::ErrorKind::Held => {
            let Some(confirmation) = err.confirm.as_ref().filter(|c| c.op == "guardrail.confirm")
            else {
                return Err(Error::Bus(err));
            };
            let inspection = ui
                .call("guardrail.hold.get", confirmation.payload.clone())
                .await?;
            if text(&inspection["request"], "op") != op
                || inspection["request"]["payload"] != payload
            {
                return Err(Error::Protocol(
                    "Held action does not match the requested operation. Inspect it in Guardrails."
                        .into(),
                ));
            }
            let copy = format!(
                "{}\n\n{}",
                err.message,
                serde_json::to_string_pretty(&inspection["request"]).unwrap_or_default()
            );
            if !confirm(ui, "Allow this held action once?", &copy).await {
                return Ok(None);
            }
            let result = ui
                .call("guardrail.confirm", confirmation.payload.clone())
                .await?;
            if result["outcome"]["ok"] == true {
                Ok(Some(result["outcome"]["result"].clone()))
            } else {
                Err(Error::Protocol(format!(
                    "Held action was not completed: {}",
                    result["outcome"]["error"]
                )))
            }
        }
        Err(err) => Err(err),
    }
}

fn detach(entry: &gtk::Entry) {
    if let Some(parent) = entry.parent().and_downcast::<gtk::Box>() {
        parent.remove(entry);
    }
}
