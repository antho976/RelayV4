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
            let branch = label(text(&status, "branch"), "mono");
            branch.set_hexpand(true);
            branch.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            header.add_css_class("code-bar");
            header.append(&crate::icons::image("branch", 13));
            header.append(&branch);
            let refresh = crate::app::icon_button("refresh", "Refresh Git");
            refresh.add_css_class("small-key");
            refresh.set_valign(gtk::Align::Center);
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
            let panes = gtk::Paned::new(gtk::Orientation::Vertical);
            panes.add_css_class("code-git-split");
            panes.set_vexpand(true);
            panes.set_resize_start_child(true);
            panes.set_resize_end_child(false);
            panes.set_position(
                (ui.window.height().max(ui.window.default_height()) - 42 - 24 - 36 - 280).max(120),
            );
            let upper = gtk::Box::new(gtk::Orientation::Vertical, 0);
            upper.add_css_class("code-git-upper");
            panes.set_start_child(Some(&scrolled(&upper)));
            e.git.append(&panes);
            let changes = rows(&status, "files");
            let publish = gtk::Box::new(gtk::Orientation::Vertical, 0);
            publish.add_css_class("code-git-section");
            publish.append(&label("PUBLISH WORKFLOW", "code-section-heading"));
            let commit_step = publish_step(
                &publish,
                "1",
                "Commit",
                &if changes.is_empty() {
                    String::from("Working tree clean")
                } else {
                    format!(
                        "{} changed file{}",
                        changes.len(),
                        if changes.len() == 1 { "" } else { "s" }
                    )
                },
            );
            let write = button(
                if changes.is_empty() {
                    "Done"
                } else {
                    "Write message"
                },
                "quiet",
            );
            write.add_css_class("small-key");
            write.set_valign(gtk::Align::Center);
            write.set_sensitive(!changes.is_empty());
            let message = e.commit_message.clone();
            write.connect_clicked(move |_| {
                message.grab_focus();
            });
            commit_step.append(&write);
            let ahead = status["ahead"].as_i64();
            let behind = status["behind"].as_i64();
            let upstream = status["upstream"].is_string();
            let can_push = !upstream || (ahead.is_some_and(|n| n > 0) && behind == Some(0));
            let push_state = if !upstream {
                String::from("Not on remote")
            } else if let (Some(ahead), Some(behind)) = (ahead, behind) {
                if ahead == 0 && behind == 0 {
                    String::from("Up to date")
                } else {
                    format!("{ahead} ahead · {behind} behind")
                }
            } else {
                String::from("Upstream status unknown · fetch to refresh")
            };
            let push_step = publish_step(&publish, "2", "Push", &push_state);
            let push = button(
                &if can_push {
                    if let Some(ahead) = ahead.filter(|n| *n > 0) {
                        format!("Push {ahead}")
                    } else {
                        String::from("Publish branch")
                    }
                } else if behind.is_some_and(|n| n > 0) {
                    String::from("Pull first")
                } else if ahead.is_none() || behind.is_none() {
                    String::from("Unknown")
                } else {
                    String::from("Done")
                },
                if can_push { "primary" } else { "quiet" },
            );
            push.add_css_class("small-key");
            push.set_valign(gtk::Align::Center);
            push.set_sensitive(can_push);
            let ed = e.clone();
            let weak = Rc::downgrade(&ui);
            push.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ed.git_action(&ui, "git.push", json!({}), None);
                }
            });
            push_step.append(&push);
            let pr_step = publish_step(&publish, "3", "Pull request", "Push the branch first");
            let pr = button("Open PR", "quiet");
            pr.add_css_class("small-key");
            pr.set_valign(gtk::Align::Center);
            pr.set_sensitive(false);
            pr_step.append(&pr);
            upper.append(&publish);
            let changes_section = gtk::Box::new(gtk::Orientation::Vertical, 0);
            changes_section.add_css_class("code-git-section");
            changes_section.append(&label(
                &format!("CHANGES  {}", changes.len()),
                "code-section-heading",
            ));
            upper.append(&changes_section);
            if !changes.is_empty() {
                let summary = label(&change_summary(&changes), "code-git-hint");
                summary.set_wrap(true);
                summary.set_max_width_chars(42);
                changes_section.append(&summary);
            }
            if changes.is_empty() {
                changes_section.append(&label("Working tree clean.", "code-git-hint"));
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
                row.add_css_class("code-change");
                changes_section.append(&row);
            }
            let message = e.commit_message.clone();
            if message.widget_name() != "commit-message" {
                message.set_widget_name("commit-message");
                let keys = gtk::EventControllerKey::new();
                let weak = Rc::downgrade(&ui);
                let editor = Rc::downgrade(&e);
                keys.connect_key_pressed(move |_, key, _, modifiers| {
                    if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
                        && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                    {
                        if let (Some(ui), Some(editor)) = (weak.upgrade(), editor.upgrade()) {
                            let message = commit_text(&editor.commit_message).trim().to_string();
                            if !message.is_empty() {
                                editor.git_action(
                                    &ui,
                                    "git.commit",
                                    json!({"message":message,"all":true}),
                                    None,
                                );
                            }
                        }
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                });
                message.add_controller(keys);
            }
            detach(&message);
            message.set_wrap_mode(gtk::WrapMode::WordChar);
            message.set_size_request(-1, 66);
            message.add_css_class("code-commit-message");
            message.update_property(&[gtk::accessible::Property::Label(
                "Commit message · Ctrl Enter commits all",
            )]);
            let commit_form = gtk::Box::new(gtk::Orientation::Vertical, 6);
            commit_form.add_css_class("code-commit-form");
            let input = gtk::Overlay::new();
            input.set_child(Some(&message));
            let placeholder = label(
                "Commit message · Ctrl Enter commits all",
                "code-commit-placeholder",
            );
            placeholder.set_halign(gtk::Align::Start);
            placeholder.set_valign(gtk::Align::Start);
            placeholder.set_wrap(true);
            placeholder.set_can_target(false);
            input.add_overlay(&placeholder);
            message
                .buffer()
                .bind_property("text", &placeholder, "visible")
                .transform_to(|_, value: String| Some(value.is_empty()))
                .sync_create()
                .build();
            commit_form.append(&input);
            changes_section.append(&commit_form);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for (title, all) in [("Commit staged", false), ("Commit all", true)] {
                let b = button(title, if all { "primary" } else { "quiet" });
                b.add_css_class("small-key");
                b.set_sensitive(!changes.is_empty());
                let ed = e.clone();
                let weak = Rc::downgrade(&ui);
                let message = message.clone();
                b.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        let content = commit_text(&message).trim().to_string();
                        if content.is_empty() {
                            ui.show_error("Enter a commit message first.");
                            return;
                        }
                        ed.git_action(
                            &ui,
                            "git.commit",
                            json!({"message":content,"all":all}),
                            None,
                        );
                    }
                });
                actions.append(&b);
            }
            commit_form.append(&actions);
            let suggest = button("Suggest", "quiet");
            suggest.add_css_class("small-key");
            let ed = e.clone();
            let weak = Rc::downgrade(&ui);
            let message = message.clone();
            suggest.connect_clicked(move |_| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let ed = ed.clone();
                let field = message.clone();
                let original = commit_text(&field);
                let project = ui.project.get();
                let worktree = ed.worktree.borrow().clone();
                glib::spawn_future_local(async move {
                    match ui
                        .call("git.suggest_message", ed.payload(&ui, json!({})))
                        .await
                    {
                        Ok(v)
                            if ed.matches(&ui, project, &worktree)
                                && commit_text(&field) == original =>
                        {
                            field.buffer().set_text(text(&v, "message"))
                        }
                        Ok(_) => {}
                        Err(err) => ui.show_error(&err.to_string()),
                    }
                });
            });
            actions.prepend(&suggest);
            let branch_tools = gtk::MenuButton::new();
            branch_tools.add_css_class("layout-menu");
            branch_tools.set_icon_name("view-more-symbolic");
            branch_tools.set_tooltip_text(Some("Branch tools"));
            header.append(&branch_tools);
            let branches_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let branch_popup = gtk::Popover::new();
            branch_popup.set_child(Some(&branches_box));
            branch_tools.set_popover(Some(&branch_popup));
            let fetch = button("Fetch", "quiet");
            let ed = e.clone();
            let weak = Rc::downgrade(&ui);
            fetch.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ed.git_action(&ui, "git.fetch", json!({}), None);
                }
            });
            branches_box.append(&fetch);
            let history_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
            history_box.add_css_class("code-history");
            let history = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let history_title = label("HISTORY", "code-section-heading");
            history.append(&history_title);
            history.set_size_request(-1, 220);
            let history_scroll = scrolled(&history_box);
            history_scroll.set_min_content_height(220);
            history.append(&history_scroll);
            panes.set_end_child(Some(&history));
            let results = tokio::join!(
                ui.call("git.branches", payload.clone()),
                ui.call("git.log", e.payload(&ui, json!({"limit":30}))),
                ui.call("integration.list", json!({"project_id":project})),
                ui.call("git.pr.list", json!({"project_id":project}))
            );
            if !e.matches(&ui, project, &worktree) || revision != e.git_revision.get() {
                return;
            }
            let branch_record = results.0.as_ref().ok().and_then(|value| {
                rows(value, "branches")
                    .into_iter()
                    .find(|branch| text(branch, "name") == text(&status, "branch"))
            });
            let pull_request = results.3.as_ref().ok().and_then(|value| {
                let matches: Vec<_> = rows(value, "pull_requests")
                    .into_iter()
                    .filter(|pr| {
                        pr["same_repository"] == true
                            && text(pr, "branch") == text(&status, "branch")
                    })
                    .collect();
                matches
                    .iter()
                    .find(|pr| text(pr, "state") == "open")
                    .or_else(|| matches.first())
                    .cloned()
            });
            let complete = results
                .3
                .as_ref()
                .ok()
                .is_some_and(|value| value["complete"] == true);
            let ready = complete
                && upstream
                && ahead == Some(0)
                && behind == Some(0)
                && branch_record
                    .as_ref()
                    .is_some_and(|branch| branch["merged"] != true);
            let hint = if let Some(pr) = &pull_request {
                let state = if text(pr, "state") == "open" && pr["draft"] == true {
                    "draft"
                } else {
                    text(pr, "state")
                };
                let badge = label(state, "tag");
                badge.set_valign(gtk::Align::Center);
                pr_step.append(&badge);
                format!("PR #{} · {state}", pr["number"])
            } else if results.3.is_err() {
                String::from("GitHub unavailable · PR status unknown")
            } else if !complete {
                String::from("PR lookup incomplete")
            } else if ready {
                String::from("No PR · pushed and ready for review")
            } else {
                String::from("No PR · push the branch first")
            };
            if let Some(copy) = pr_step
                .first_child()
                .and_then(|first| first.next_sibling())
                .and_downcast::<gtk::Box>()
            {
                if let Some(label) = copy.last_child().and_downcast::<gtk::Label>() {
                    label.set_text(&hint);
                }
            }
            pr.set_sensitive(pull_request.is_some() || ready);
            pr.set_label(if pull_request.is_some() {
                "Open"
            } else {
                "Open PR"
            });
            if ready && pull_request.is_none() {
                pr.add_css_class("primary");
            }
            let ed = e.clone();
            let weak = Rc::downgrade(&ui);
            pr.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if let Some(url) = pull_request.as_ref().and_then(|pr| pr["url"].as_str()) {
                        let uri = gtk::UriLauncher::new(url);
                        uri.launch(Some(&ui.window), gtk::gio::Cancellable::NONE, |_| {});
                    } else {
                        ed.git_action(&ui, "git.pr.open", json!({}), None);
                    }
                }
            });
            let merge = gtk::Box::new(gtk::Orientation::Vertical, 0);
            merge.add_css_class("code-git-section");
            merge.append(&label(
                &format!("MERGE TEST  {}", ui.sessions.borrow().len()),
                "code-section-heading",
            ));
            upper.append(&merge);
            let hint = label(
                "Safely test agent branches together in a disposable worktree. This does not merge them into your current branch.",
                "code-git-hint",
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
                let pick = gtk::CheckButton::new();
                pick.add_css_class("code-merge-pick");
                let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
                copy.append(&label(&name, "body"));
                copy.append(&label(text(session, "branch"), "mono"));
                pick.set_child(Some(&copy));
                merge.append(&pick);
                picks.push((name, pick));
            }
            let test = button("Select at least 2 branches", "quiet");
            test.add_css_class("code-merge-action");
            test.set_sensitive(false);
            let selected_count = Rc::new(Cell::new(0_usize));
            for (_, pick) in &picks {
                let count = selected_count.clone();
                let test = test.clone();
                pick.connect_toggled(move |pick| {
                    let total = if pick.is_active() {
                        count.get() + 1
                    } else {
                        count.get().saturating_sub(1)
                    };
                    count.set(total);
                    test.set_sensitive(total >= 2);
                    test.set_label(&if total >= 2 {
                        format!("Test merge + build · {total}")
                    } else {
                        String::from("Select at least 2 branches")
                    });
                });
            }
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
                    let commits = rows(&v, "commits");
                    history_title.set_text(&format!("HISTORY  {}", commits.len()));
                    let graph = commit_graph(&commits);
                    let graph_width = graph
                        .iter()
                        .flat_map(|row| {
                            row.links
                                .iter()
                                .map(|(_, lane)| *lane)
                                .chain(std::iter::once(row.lane))
                        })
                        .max()
                        .unwrap_or(0)
                        + 1;
                    let graph_key = gtk::ToggleButton::with_label("Graph");
                    graph_key.add_css_class("code-graph-toggle");
                    graph_key.set_active(true);
                    let history_head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                    history.remove(&history_title);
                    history_title.set_hexpand(true);
                    history_head.append(&history_title);
                    history_head.append(&graph_key);
                    history.prepend(&history_head);
                    let mut graphs = Vec::new();
                    for (commit, graph_row) in commits.into_iter().zip(graph) {
                        let sha = text(&commit, "sha").to_string();
                        let b = button(
                            &format!("{} {}", &sha[..sha.len().min(7)], text(&commit, "subject")),
                            "file",
                        );
                        b.add_css_class("code-commit");
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        let graph = graph_widget(graph_row, graph_width);
                        graphs.push(graph.clone());
                        row.append(&graph);
                        row.append(&label(&sha[..sha.len().min(7)], "mono"));
                        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
                        let subject = label(text(&commit, "subject"), "code-commit-subject");
                        subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        copy.append(&subject);
                        copy.append(&label(text(&commit, "author"), "dim"));
                        row.append(&copy);
                        b.set_child(Some(&row));
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
                    graph_key.connect_toggled(move |key| {
                        for graph in &graphs {
                            graph.set_visible(key.is_active());
                        }
                    });
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
            let counts = match (branch["ahead"].as_i64(), branch["behind"].as_i64()) {
                (Some(ahead), Some(behind)) => format!("{ahead} ahead · {behind} behind"),
                _ if branch["upstream"].is_string() => "Unknown upstream".into(),
                _ => "Not published".into(),
            };
            row.append(&label(&counts, "dim"));
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
    pub(super) fn git_action(
        self: &Rc<Self>,
        ui: &Rc<Ui>,
        op: &'static str,
        extra: Value,
        input: Option<gtk::Entry>,
    ) {
        if self.git_busy.get() {
            return;
        }
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
        let commit_snapshot = (op == "git.commit").then(|| commit_text(&self.commit_message));
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
                    if commit_snapshot
                        .as_ref()
                        .is_some_and(|snapshot| commit_text(&e.commit_message) == *snapshot)
                    {
                        e.commit_message.buffer().set_text("");
                    }
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
                    if matches!(op, "git.branch.create" | "git.branch.switch") {
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

fn detach(entry: &impl IsA<gtk::Widget>) {
    if let Some(parent) = entry.parent() {
        if let Some(parent) = parent.downcast_ref::<gtk::Box>() {
            parent.remove(entry);
        } else if let Some(parent) = parent.downcast_ref::<gtk::Overlay>() {
            parent.set_child(gtk::Widget::NONE);
        }
    }
}

fn change_summary(files: &[Value]) -> String {
    let mut groups: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for file in files {
        let statuses = format!("{}{}", text(file, "index"), text(file, "worktree"));
        let category = if statuses.contains('U') {
            "Conflicted"
        } else if statuses.contains('R') {
            "Renamed"
        } else if statuses.contains('D') {
            "Deleted"
        } else if statuses.contains('A') || statuses.contains('?') {
            "Added"
        } else {
            "Modified"
        };
        groups.entry(category).or_default().push(text(file, "path"));
    }
    groups
        .into_iter()
        .map(|(kind, paths)| {
            let visible = paths.iter().take(3).copied().collect::<Vec<_>>().join(", ");
            format!(
                "{kind} {}: {visible}{}",
                paths.len(),
                if paths.len() > 3 {
                    format!(" +{} more", paths.len() - 3)
                } else {
                    String::new()
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn commit_text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}
fn publish_step(parent: &gtk::Box, number: &str, title: &str, detail: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.add_css_class("code-publish-step");
    let index = label(number, "code-step-index");
    index.set_xalign(0.5);
    index.set_valign(gtk::Align::Center);
    row.append(&index);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 1);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    copy.append(&label(title, "code-step-title"));
    let hint = label(detail, "code-step-hint");
    hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
    copy.append(&hint);
    row.append(&copy);
    parent.append(&row);
    row
}

#[derive(Debug)]
struct GraphRow {
    lane: usize,
    links: Vec<(u8, usize)>,
    merge: bool,
}
// Relay-2 commitGraph.ts: reserve parent lanes until the corresponding commit arrives.
fn commit_graph(commits: &[Value]) -> Vec<GraphRow> {
    let mut active: Vec<Option<String>> = Vec::new();
    let mut result = Vec::new();
    for commit in commits {
        let before = active.clone();
        let waiting: Vec<_> = before
            .iter()
            .enumerate()
            .filter(|(_, sha)| sha.as_deref() == Some(text(commit, "sha")))
            .map(|(lane, _)| lane)
            .collect();
        let lane = waiting.first().copied().unwrap_or_else(|| {
            active
                .iter()
                .position(Option::is_none)
                .unwrap_or(active.len())
        });
        let mut links = Vec::new();
        for &index in &waiting {
            active[index] = None;
            links.push((1, index));
        }
        if lane >= active.len() {
            active.resize(lane + 1, None);
        } else {
            active[lane] = None;
        }
        let parents: Vec<_> = commit["parents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut outgoing = Vec::new();
        for (index, parent) in parents.iter().enumerate() {
            let target = if index == 0 {
                lane
            } else {
                active
                    .iter()
                    .position(|sha| sha.as_deref() == Some(parent))
                    .unwrap_or_else(|| {
                        active
                            .iter()
                            .position(Option::is_none)
                            .unwrap_or(active.len())
                    })
            };
            if target >= active.len() {
                active.resize(target + 1, None);
            }
            active[target] = Some((*parent).to_string());
            if !outgoing.contains(&target) {
                outgoing.push(target);
            }
        }
        for target in outgoing {
            links.push((2, target));
        }
        for index in 0..before.len().max(active.len()) {
            if before.get(index).is_some_and(Option::is_some)
                && active.get(index).is_some_and(Option::is_some)
                && !waiting.contains(&index)
            {
                links.push((0, index));
            }
        }
        while active.last() == Some(&None) {
            active.pop();
        }
        result.push(GraphRow {
            lane,
            links,
            merge: parents.len() > 1,
        });
    }
    result
}
fn graph_widget(row: GraphRow, lanes: usize) -> gtk::DrawingArea {
    let graph = gtk::DrawingArea::new();
    graph.set_content_width(lanes as i32 * 14);
    graph.set_content_height(44);
    graph.set_draw_func(move |widget, cr, _, _| {
        let colors = [
            (79.0 / 255.0, 143.0 / 255.0, 217.0 / 255.0),
            (201.0 / 255.0, 161.0 / 255.0, 59.0 / 255.0),
            (165.0 / 255.0, 108.0 / 255.0, 214.0 / 255.0),
            (63.0 / 255.0, 176.0 / 255.0, 165.0 / 255.0),
            (217.0 / 255.0, 122.0 / 255.0, 79.0 / 255.0),
        ];
        let node = row.lane as f64 * 14.0 + 7.0;
        cr.set_line_width(1.5);
        for &(kind, lane) in &row.links {
            let x = lane as f64 * 14.0 + 7.0;
            let (r, g, b) = colors[lane % colors.len()];
            cr.set_source_rgba(r, g, b, 0.8);
            match kind {
                0 => {
                    cr.move_to(x, 0.0);
                    cr.line_to(x, 44.0);
                }
                1 => {
                    cr.move_to(x, 0.0);
                    if x == node {
                        cr.line_to(node, 22.0);
                    } else {
                        cr.curve_to(x, 22.0, node, 11.0, node, 22.0);
                    }
                }
                _ => {
                    cr.move_to(node, 22.0);
                    if x == node {
                        cr.line_to(x, 44.0);
                    } else {
                        cr.curve_to(node, 33.0, x, 22.0, x, 44.0);
                    }
                }
            }
            let _ = cr.stroke();
        }
        cr.arc(node, 22.0, 3.5, 0.0, std::f64::consts::TAU);
        if let Some(color) =
            widget
                .style_context()
                .lookup_color(if row.merge { "ink" } else { "console" })
        {
            cr.set_source_rgba(
                color.red() as f64,
                color.green() as f64,
                color.blue() as f64,
                color.alpha() as f64,
            );
        }
        let _ = cr.fill_preserve();
        let (r, g, b) = colors[row.lane % colors.len()];
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(2.0);
        let _ = cr.stroke();
    });
    graph
}
#[cfg(test)]
mod graph_tests {
    use super::*;
    #[test]
    fn precommit_summary_names_changes_and_counts_without_a_model() {
        let rows = vec![
            json!({"path":"new.rs","index":"A"}),
            json!({"path":"old.rs","worktree":"D"}),
            json!({"path":"moved.rs","index":"R"}),
            json!({"path":"edit.rs","worktree":"M"}),
        ];
        let summary = change_summary(&rows);
        assert!(summary.contains("Added 1: new.rs"));
        assert!(summary.contains("Deleted 1: old.rs"));
        assert!(summary.contains("Renamed 1: moved.rs"));
        assert!(summary.contains("Modified 1: edit.rs"));
    }
    #[test]
    fn merge_lanes_rejoin_at_the_shared_parent() {
        let commits = vec![
            json!({"sha":"merge","parents":["left","right"]}),
            json!({"sha":"left","parents":["base"]}),
            json!({"sha":"right","parents":["base"]}),
            json!({"sha":"base","parents":[]}),
        ];
        let graph = commit_graph(&commits);
        assert!(graph[0].merge);
        assert_eq!(
            graph.iter().map(|row| row.lane).collect::<Vec<_>>(),
            vec![0, 0, 1, 0]
        );
        assert!(graph[3].links.contains(&(1, 0)) && graph[3].links.contains(&(1, 1)));
        assert!(!graph[3].links.iter().any(|(kind, _)| *kind == 2));
    }
}
