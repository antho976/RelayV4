use super::*;

fn choice_cards(control: &gtk::DropDown, choices: &[(&str, &str, &str)]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.set_homogeneous(true);
    let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (index, (icon, title, copy)) in choices.iter().enumerate() {
        let key = gtk::ToggleButton::new();
        key.add_css_class("launch-choice");
        if let Some(first) = buttons.first() {
            key.set_group(Some(first));
        }
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let mark = crate::icons::image(icon, 16);
        mark.add_css_class("launch-mode-icon");
        mark.set_valign(gtk::Align::Center);
        mark.set_halign(gtk::Align::Center);
        content.append(&mark);
        let words = gtk::Box::new(gtk::Orientation::Vertical, 3);
        words.append(&label(title, "choice-title"));
        let copy = label(copy, "dim");
        copy.set_wrap(true);
        words.append(&copy);
        words.set_hexpand(true);
        content.append(&words);
        let check = crate::icons::image("check", 12);
        check.add_css_class("launch-choice-check");
        check.set_valign(gtk::Align::Center);
        check.set_halign(gtk::Align::Center);
        content.append(&check);
        key.set_child(Some(&content));
        key.set_active(control.selected() == index as u32);
        let selected = control.downgrade();
        key.connect_toggled(move |key| {
            if key.is_active() {
                if let Some(selected) = selected.upgrade() {
                    selected.set_selected(index as u32);
                }
            }
        });
        row.append(&key);
        buttons.push(key);
    }
    control.connect_selected_notify(move |control| {
        for (index, key) in buttons.iter().enumerate() {
            key.set_active(control.selected() == index as u32);
        }
    });
    row
}

fn segments(control: &gtk::DropDown, choices: &[&str]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("launch-segmented");
    row.set_homogeneous(true);
    let mut keys: Vec<gtk::ToggleButton> = Vec::new();
    for (index, title) in choices.iter().enumerate() {
        let key = gtk::ToggleButton::with_label(title);
        if let Some(first) = keys.first() {
            key.set_group(Some(first));
        }
        key.set_active(control.selected() == index as u32);
        let weak = control.downgrade();
        key.connect_toggled(move |key| {
            if key.is_active() {
                if let Some(control) = weak.upgrade() {
                    control.set_selected(index as u32);
                }
            }
        });
        row.append(&key);
        keys.push(key);
    }
    control.connect_selected_notify(move |control| {
        for (i, key) in keys.iter().enumerate() {
            key.set_active(i as u32 == control.selected());
        }
    });
    let weak = row.downgrade();
    control.connect_sensitive_notify(move |control| {
        if let Some(row) = weak.upgrade() {
            row.set_sensitive(control.is_sensitive());
        }
    });
    row
}

fn launch_heading(title: &str, hint: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.append(&label(title, "body"));
    let copy = label(hint, "dim");
    copy.set_wrap(true);
    row.append(&copy);
    row
}

fn agent_glyph(index: usize) -> gtk::Image {
    let paths = [
        r#"<path d="M5 16.5 12 3l7 13.5-7 4.5-7-4.5Z"/><path d="m8.2 15.1 3.8-7.4 3.8 7.4L12 17.6l-3.8-2.5Z"/>"#,
        r#"<path d="M4 12c3.2-6.7 12.8-6.7 16 0-3.2 6.7-12.8 6.7-16 0Z"/><circle cx="12" cy="12" r="2.8"/>"#,
        r#"<path d="m12 3 2.3 6.7L21 12l-6.7 2.3L12 21l-2.3-6.7L3 12l6.7-2.3L12 3Z"/><circle cx="12" cy="12" r="1.5"/>"#,
        r#"<path d="M4 8.5 12 4l8 4.5v7L12 20l-8-4.5v-7Z"/><path d="m8 10 4-2.2 4 2.2v4l-4 2.2L8 14v-4Z"/>"#,
        r#"<circle cx="12" cy="12" r="4.2"/><path d="M12 2v4M12 18v4M2 12h4M18 12h4M4.9 4.9l2.8 2.8M16.3 16.3l2.8 2.8M19.1 4.9l-2.8 2.8M7.7 16.3l-2.8 2.8"/>"#,
        r#"<path d="M5 5h6v6H5zM13 13h6v6h-6z"/><path d="m14 4 6 6M4 14l6 6"/>"#,
    ];
    let colors = [
        "#70b7ff", "#9d8cff", "#efad5b", "#59c9a5", "#df7faa", "#87b45a",
    ];
    let document = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="{}" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
        colors[index % 6],
        paths[index % 6]
    );
    let svg = gtk::Svg::from_bytes(&glib::Bytes::from_owned(document.into_bytes()));
    let image = gtk::Image::from_paintable(Some(&svg));
    image.set_pixel_size(21);
    image.add_css_class("launch-agent-glyph");
    image.set_valign(gtk::Align::Center);
    image.set_halign(gtk::Align::Center);
    image
}

struct Profile {
    root: gtk::Box,
    provider: gtk::DropDown,
    role: gtk::DropDown,
    model: gtk::Entry,
    effort: gtk::DropDown,
    worktree: gtk::Entry,
    prompt: gtk::TextView,
    writes: gtk::CheckButton,
    ui_access: gtk::CheckButton,
    tasks: RefCell<Vec<(i64, gtk::ToggleButton)>>,
    task_box: gtk::Grid,
    task_search: gtk::SearchEntry,
    task_filter: gtk::DropDown,
    task_count: gtk::Label,
    provider_cards: Vec<(gtk::ToggleButton, gtk::Label, gtk::Label)>,
}
impl Profile {
    fn new(index: usize, compact: bool) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_widget_name(&format!("launch-profile-{index}"));
        let provider = gtk::DropDown::from_strings(&["Claude", "Codex"]);
        provider.set_widget_name(&format!("launch-provider-{index}"));
        root.add_css_class("launch-profile");
        let heading = launch_heading(
            &format!("Configure Agent {}", index + 1),
            "Provider, responsibility, and reasoning belong to this agent only.",
        );
        if compact {
            heading.last_child().unwrap().set_visible(false);
        }
        root.append(&heading);
        let configuration = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        configuration.add_css_class("launch-configuration");
        let providers = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        providers.set_homogeneous(true);
        providers.set_hexpand(true);
        let mut provider_cards: Vec<(gtk::ToggleButton, gtk::Label, gtk::Label)> = Vec::new();
        for (i, (icon, title)) in [("claude", "Claude Code"), ("codex", "Codex")]
            .iter()
            .enumerate()
        {
            let key = gtk::ToggleButton::new();
            key.add_css_class("launch-provider-card");
            if let Some((first, _, _)) = provider_cards.first() {
                key.set_group(Some(first));
            }
            let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            let mark = crate::icons::image(icon, 18);
            mark.add_css_class("launch-provider-mark");
            mark.set_valign(gtk::Align::Center);
            mark.set_halign(gtk::Align::Center);
            top.append(&mark);
            let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
            words.set_hexpand(true);
            words.append(&label(title, "choice-title"));
            let account = label("Checking provider…", "faint");
            account.set_ellipsize(gtk::pango::EllipsizeMode::End);
            words.append(&account);
            top.append(&words);
            let check = crate::icons::image("check", 12);
            check.add_css_class("launch-choice-check");
            check.set_valign(gtk::Align::Center);
            check.set_halign(gtk::Align::Center);
            top.append(&check);
            card.append(&top);
            let facts = label("Version unavailable", "faint");
            facts.add_css_class("launch-provider-facts");
            facts.set_wrap(true);
            card.append(&facts);
            key.set_child(Some(&card));
            key.set_active(provider.selected() == i as u32);
            let selected = provider.downgrade();
            key.connect_toggled(move |key| {
                if key.is_active() {
                    if let Some(selected) = selected.upgrade() {
                        selected.set_selected(i as u32);
                    }
                }
            });
            providers.append(&key);
            provider_cards.push((key, account, facts));
        }
        provider.set_visible(false);
        root.append(&provider);
        configuration.append(&providers);
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 10);
        controls.set_size_request(260, -1);
        let role = gtk::DropDown::from_strings(&["Builder", "Reviewer", "Docs"]);
        controls.append(&label("ROLE", "section-label"));
        controls.append(&segments(&role, &["Builder", "Reviewer", "Docs"]));
        role.set_visible(false);
        controls.append(&role);
        let model = gtk::Entry::builder()
            .placeholder_text("Provider default, or enter a model ID")
            .build();
        model.set_widget_name(&format!("launch-model-{index}"));
        model.set_width_chars(1);
        model.set_hexpand(true);
        field("Model", &model, &controls);
        let effort =
            gtk::DropDown::from_strings(&["minimal", "low", "medium", "high", "xhigh", "max"]);
        effort.set_widget_name(&format!("launch-effort-{index}"));
        effort.set_selected(3);
        controls.append(&label("REASONING EFFORT", "section-label"));
        let effort_keys = segments(
            &effort,
            &["minimal", "low", "medium", "high", "xhigh", "max"],
        );
        effort_keys.first_child().unwrap().set_visible(false);
        let weak_effort = effort.downgrade();
        let weak_keys = effort_keys.downgrade();
        provider.connect_selected_notify(move |provider| {
            let (Some(effort), Some(keys)) = (weak_effort.upgrade(), weak_keys.upgrade()) else {
                return;
            };
            let codex = provider.selected() == 1;
            keys.first_child().unwrap().set_visible(codex);
            keys.last_child().unwrap().set_visible(!codex);
            if (codex && effort.selected() == 5) || (!codex && effort.selected() == 0) {
                effort.set_selected(3);
            }
        });
        controls.append(&effort_keys);
        effort.set_visible(false);
        controls.append(&effort);
        configuration.append(&controls);
        root.append(&configuration);
        let prompt = gtk::TextView::new();
        prompt.set_wrap_mode(gtk::WrapMode::WordChar);
        prompt.set_size_request(-1, 100);

        let advanced = gtk::Expander::new(Some("Worktree and permissions"));
        let options = gtk::Box::new(gtk::Orientation::Vertical, 8);
        field("Additional instructions", &prompt, &options);
        let worktree = gtk::Entry::builder()
            .text("new")
            .placeholder_text("new, primary, or absolute worktree path")
            .build();
        field("Worktree", &worktree, &options);
        let writes = gtk::CheckButton::with_label("Allow agent bus writes");
        writes.set_active(true);
        options.append(&writes);
        let ui_access = gtk::CheckButton::with_label("Allow UI control");
        options.append(&ui_access);
        advanced.set_child(Some(&options));
        root.append(&advanced);
        let assignment = gtk::Box::new(gtk::Orientation::Vertical, 8);
        assignment.add_css_class("launch-assignment");
        assignment.append(&launch_heading(
            "Assign the work",
            "Select every task this agent should own.",
        ));
        let tools = gtk::Box::new(gtk::Orientation::Vertical, 8);
        tools.add_css_class("launch-task-tools");
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let task_search = gtk::SearchEntry::new();
        task_search.set_placeholder_text(Some("Search tasks"));
        task_search.set_hexpand(true);
        search_row.append(&task_search);
        let task_count = label("0 selected", "mono");
        search_row.append(&task_count);
        tools.append(&search_row);
        let task_filter =
            gtk::DropDown::from_strings(&["All open", "Backlog", "Ready", "Active", "In review"]);
        tools.append(&segments(
            &task_filter,
            &["All open", "Backlog", "Ready", "Active", "In review"],
        ));
        task_filter.set_visible(false);
        tools.append(&task_filter);
        assignment.append(&tools);
        let task_box = gtk::Grid::builder()
            .column_homogeneous(true)
            .column_spacing(2)
            .row_spacing(2)
            .build();
        task_box.add_css_class("launch-task-list");
        let scroll = scrolled(&task_box);
        scroll.set_min_content_height(240);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        assignment.append(&scroll);
        root.append(&assignment);
        Rc::new(Self {
            root,
            provider,
            role,
            model,
            effort,
            worktree,
            prompt,
            writes,
            ui_access,
            tasks: RefCell::default(),
            task_box,
            task_search,
            task_filter,
            task_count,
            provider_cards,
        })
    }
    fn selected_tasks(&self) -> Vec<i64> {
        self.tasks
            .borrow()
            .iter()
            .filter(|(_, b)| b.is_active())
            .map(|(id, _)| *id)
            .collect()
    }
    fn payload(&self, project: i64, role: Option<&str>) -> Value {
        let role = role.unwrap_or(match self.role.selected() {
            1 => "reviewer",
            2 => "docs",
            _ => "builder",
        });
        let model = self.model.text().trim().to_string();
        let effort = self
            .effort
            .selected_item()
            .and_downcast::<gtk::StringObject>()
            .map(|s| s.string().to_string())
            .filter(|s| s != "Default");
        json!({"project_id":project,"provider":if self.provider.selected()==0{"claude"}else{"codex"},"role":role,"model":if model.is_empty(){None}else{Some(model)},"effort":effort,"worktree":self.worktree.text().trim(),"bus_writes":self.writes.is_active(),"allow_ui":self.ui_access.is_active(),"prompt":self.prompt()})
    }
    fn prompt(&self) -> String {
        let b = self.prompt.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).to_string()
    }
}

impl Ui {
    pub fn show_launch(self: &Rc<Self>, task: Option<i64>) {
        if self.project.get() == 0 {
            let workspace = self.workspaces.borrow().first().cloned();
            self.open_repository_in(workspace);
            return;
        }
        if self.launch_busy.get() {
            self.show_info("A launch is in progress. Allocated sessions will appear on the wall.");
            return;
        }
        if !self.dismiss_panels() {
            return;
        }
        clear(&self.launch_box);
        self.launch.set_reveal_child(true);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        heading.add_css_class("launch-heading");
        let title = launch_heading(
            "New session",
            "Shape each agent, assign any amount of work, then launch.",
        );
        title.set_hexpand(true);
        heading.append(&title);
        let close = icon_button("close", "Close new session");
        heading.append(&close);
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                if !ui.launch_busy.get() {
                    ui.launch.set_reveal_child(false);
                }
            }
        });
        self.launch_box.append(&heading);
        let compact = self.window.height() <= 850;
        let body = gtk::Box::new(gtk::Orientation::Vertical, if compact { 8 } else { 14 });
        if compact {
            body.add_css_class("launch-compact");
        }
        body.add_css_class("launch-body");
        self.launch_box.append(&scrolled(&body));

        let mode = gtk::DropDown::from_strings(&["Solo agents", "Review group"]);
        mode.set_widget_name("launch-mode");
        if !compact {
            body.append(&launch_heading(
                "Session type",
                "Choose how the agents share a worktree.",
            ));
        }
        body.append(&choice_cards(
            &mode,
            &[
                (
                    "user",
                    "Solo sessions",
                    "Independent worktrees, roles, settings, and assignments.",
                ),
                (
                    "merge",
                    "Review group",
                    "Builders and one read-only reviewer share a branch.",
                ),
            ],
        ));
        mode.set_visible(false);
        body.append(&mode);
        let members = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let members_title = launch_heading(
            "Agents",
            "Switch here to configure the whole profile below.",
        );
        members_title.set_hexpand(true);
        if compact {
            members_title.last_child().unwrap().set_visible(false);
        }
        members.append(&members_title);
        body.append(&members);
        let count = gtk::SpinButton::with_range(1.0, 11.0, 1.0);
        count.set_value(1.0);
        count.set_visible(false);
        members.append(&count);
        let count_select = gtk::DropDown::from_strings(&["1", "2", "3", "4", "5", "6"]);
        // Segmented keys hold weak references to their backing control. Keep it owned
        // by the form, just like the mode and builder controls.
        count_select.set_visible(false);
        count_select.set_widget_name("launch-count-control");
        members.append(&count_select);
        let count_keys = segments(&count_select, &["1", "2", "3", "4", "5", "6"]);
        count_keys.add_css_class("launch-count");
        let spin = count.clone();
        count_select.connect_selected_notify(move |c| spin.set_value((c.selected() + 1) as f64));
        members.append(&count_keys);
        let builders =
            gtk::DropDown::from_strings(&["One builder + reviewer", "Two builders + reviewer"]);
        builders.set_visible(false);
        members.append(&builders);
        let builder_keys = segments(&builders, &["1 builder", "2 builders"]);
        members.append(&builder_keys);
        let profiles: Rc<Vec<_>> = Rc::new((0..11).map(|i| Profile::new(i, compact)).collect());
        let stack = gtk::Stack::new();
        let selector = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        selector.set_homogeneous(true);
        selector.add_css_class("member-rail");
        body.append(&selector);
        body.append(&stack);
        let mut member_keys: Vec<gtk::ToggleButton> = Vec::new();
        for (i, p) in profiles.iter().enumerate() {
            stack.add_titled(&p.root, Some(&format!("agent-{i}")), &format!("{}", i + 1));
            let key = gtk::ToggleButton::new();
            key.add_css_class("launch-member-card");
            if let Some(first) = member_keys.first() {
                key.set_group(Some(first));
            }
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            content.append(&agent_glyph(i));
            let words = gtk::Box::new(gtk::Orientation::Vertical, 3);
            words.set_hexpand(true);
            let title = label(&format!("Agent {}", i + 1), "body");
            words.append(&title);
            let facts = label("", "faint");
            facts.set_ellipsize(gtk::pango::EllipsizeMode::End);
            facts.set_max_width_chars(23);
            words.append(&facts);
            content.append(&words);
            key.set_child(Some(&content));
            let refresh: Rc<dyn Fn()> = Rc::new({
                let p = Rc::downgrade(p);
                let facts = facts.clone();
                let title = title.clone();
                let mode = mode.downgrade();
                move || {
                    let (Some(p), Some(mode)) = (p.upgrade(), mode.upgrade()) else {
                        return;
                    };
                    let role = match p.role.selected() {
                        1 => "Reviewer",
                        2 => "Docs",
                        _ => "Builder",
                    };
                    title.set_text(&if mode.selected() == 1 {
                        if i == 2 {
                            "Reviewer".into()
                        } else {
                            format!("Builder {}", i + 1)
                        }
                    } else {
                        format!("Agent {}", i + 1)
                    });
                    let effort = p
                        .effort
                        .selected_item()
                        .and_downcast::<gtk::StringObject>()
                        .map(|s| s.string().to_string())
                        .unwrap_or_default();
                    facts.set_text(&format!(
                        "{} · {} · {}",
                        if p.provider.selected() == 0 {
                            "Claude"
                        } else {
                            "Codex"
                        },
                        role,
                        effort
                    ));
                }
            });
            for d in [&p.provider, &p.role, &p.effort, &mode] {
                let refresh = refresh.clone();
                d.connect_selected_notify(move |_| refresh());
            }
            refresh();
            let stack = stack.downgrade();
            key.connect_toggled(move |key| {
                if key.is_active() {
                    if let Some(stack) = stack.upgrade() {
                        stack.set_visible_child_name(&format!("agent-{i}"));
                    }
                }
            });
            selector.append(&key);
            member_keys.push(key);
        }
        member_keys[0].set_active(true);
        let update: Rc<dyn Fn()> = Rc::new({
            let profiles = profiles.clone();
            let mode = mode.downgrade();
            let count = count.downgrade();
            let builders = builders.downgrade();
            let stack = stack.downgrade();
            let count_keys = count_keys.clone();
            let builder_keys = builder_keys.clone();
            let member_keys = member_keys.clone();
            move || {
                let (Some(mode), Some(count), Some(builders), Some(stack)) = (
                    mode.upgrade(),
                    count.upgrade(),
                    builders.upgrade(),
                    stack.upgrade(),
                ) else {
                    return;
                };
                let group = mode.selected() == 1;
                count_keys.set_visible(!group);
                builder_keys.set_visible(group);
                for (i, p) in profiles.iter().enumerate() {
                    let visible = if group {
                        i == 0 || i == 2 || (i == 1 && builders.selected() == 1)
                    } else {
                        i < count.value_as_int() as usize
                    };
                    stack.page(&p.root).set_visible(visible);
                    member_keys[i].set_visible(visible);
                    stack.page(&p.root).set_title(&format!(
                        "{}\n{}",
                        i + 1,
                        if group && i == 2 { "Reviewer" } else { "Agent" }
                    ));
                    p.role.set_sensitive(!group);
                    p.worktree.set_sensitive(!group || i == 0);
                    if group {
                        p.role.set_selected(if i == 2 { 1 } else { 0 });
                    }
                }
                member_keys[0].set_active(true);
                stack.set_visible_child_name("agent-0");
            }
        });
        for d in [&mode, &builders] {
            let update = update.clone();
            d.connect_selected_notify(move |_| update());
        }
        let u = update.clone();
        count.connect_value_changed(move |_| u());
        update();
        let hint=label("Each solo agent has its own worktree. A review group shares one branch, with a separate reviewer and one or two builders. Choose the group's task queue on agent 1.","dim");
        hint.set_wrap(true);
        body.append(&hint);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.add_css_class("launch-footer");
        self.launch_box.append(&footer);
        let progress = label("Loading open tasks…", "dim");
        progress.set_wrap(true);
        progress.set_hexpand(true);
        footer.append(&progress);
        builders.set_widget_name("launch-builders");
        let start = button("Launch session", "primary");
        start.set_valign(gtk::Align::Center);
        start.set_widget_name("launch-start");
        start.set_sensitive(false);
        footer.append(&start);
        let project = self.project.get();
        let generation = self.generation.get();
        let form = heading.clone();
        let ui = self.clone();
        let p = profiles.clone();
        let status = progress.clone();
        let ready = start.clone();
        glib::spawn_future_local(async move {
            let (task_result, providers) = tokio::join!(
                ui.call("task.list", json!({"project_id":project})),
                ui.call("provider.list", json!({}))
            );
            if !launch_is_current(&ui, project, generation, &form) {
                return;
            }
            let provider_data = providers
                .ok()
                .map(|v| rows(&v, "providers"))
                .unwrap_or_default();
            match task_result {
                Ok(v) => {
                    let tasks = rows(&v, "tasks");
                    for (profile_index, profile) in p.iter().enumerate() {
                        for (i, (key, account, facts)) in profile.provider_cards.iter().enumerate()
                        {
                            let name = if i == 0 { "claude" } else { "codex" };
                            let info = provider_data.iter().find(|p| text(p, "provider") == name);
                            let installed = info.is_some_and(|p| p["installed"] == true);
                            key.set_sensitive(installed);
                            account.set_text(
                                &info
                                    .and_then(|p| p["signed_in_as"].as_str())
                                    .map(|s| format!("Signed in as {s}"))
                                    .unwrap_or_else(|| {
                                        if installed {
                                            "Not signed in".into()
                                        } else {
                                            "CLI not installed".into()
                                        }
                                    }),
                            );
                            facts.set_text(&format!(
                                "{} · {}",
                                info.and_then(|p| p["version"].as_str())
                                    .unwrap_or("Version unavailable"),
                                if info.is_some_and(|p| p["guarded"] == true) {
                                    "Guarded"
                                } else {
                                    "Unguarded"
                                }
                            ));
                        }
                        let mut filters: Vec<(gtk::ToggleButton, String, String)> = Vec::new();
                        for (index, row) in tasks
                            .iter()
                            .filter(|t| text(t, "column") != "done")
                            .enumerate()
                        {
                            let id = row["id"].as_i64().unwrap_or(0);
                            let check = gtk::ToggleButton::new();
                            check.set_active(profile_index == 0 && task == Some(id));
                            check.add_css_class("launch-task");
                            let card = gtk::Box::new(gtk::Orientation::Vertical, 5);
                            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                            line.append(&label(&format!("#{id}"), "mono"));
                            line.append(&label(&text(row, "column").replace('_', " "), "faint"));
                            card.append(&line);
                            let title = label(text(row, "title"), "body");
                            title.set_wrap(true);
                            title.set_max_width_chars(30);
                            card.append(&title);
                            let desc = label(text(row, "body"), "dim");
                            desc.set_wrap(true);
                            desc.set_lines(2);
                            desc.set_ellipsize(gtk::pango::EllipsizeMode::End);
                            desc.set_max_width_chars(35);
                            card.append(&desc);
                            let overlay = gtk::Overlay::new();
                            overlay.set_child(Some(&card));
                            let mark = crate::icons::image("check", 12);
                            mark.add_css_class("launch-task-check");
                            mark.set_halign(gtk::Align::End);
                            mark.set_valign(gtk::Align::Start);
                            overlay.add_overlay(&mark);
                            check.set_child(Some(&overlay));
                            check.update_property(&[gtk::accessible::Property::Label(text(
                                row, "title",
                            ))]);
                            profile.task_box.attach(
                                &check,
                                (index % 2) as i32,
                                (index / 2) as i32,
                                1,
                                1,
                            );
                            filters.push((
                                check.clone(),
                                text(row, "column").to_owned(),
                                format!("#{id} {} {}", text(row, "title"), text(row, "body"))
                                    .to_lowercase(),
                            ));
                            profile.tasks.borrow_mut().push((id, check.clone()));
                            let weak = Rc::downgrade(profile);
                            check.connect_toggled(move |_| {
                                if let Some(p) = weak.upgrade() {
                                    p.task_count.set_text(&format!(
                                        "{} selected",
                                        p.selected_tasks().len()
                                    ));
                                }
                            });
                        }
                        profile
                            .task_count
                            .set_text(&format!("{} selected", profile.selected_tasks().len()));
                        let filter: Rc<dyn Fn()> = Rc::new({
                            let filters = filters.clone();
                            let search = profile.task_search.downgrade();
                            let column = profile.task_filter.downgrade();
                            let grid = profile.task_box.downgrade();
                            move || {
                                let (Some(search), Some(column), Some(grid)) =
                                    (search.upgrade(), column.upgrade(), grid.upgrade())
                                else {
                                    return;
                                };
                                for (check, _, _) in &filters {
                                    if check.parent().is_some() {
                                        grid.remove(check);
                                    }
                                }
                                let query = search.text().trim().to_lowercase();
                                let columns = ["", "backlog", "ready", "active", "in_review"];
                                let selected = columns[column.selected() as usize];
                                let mut index = 0;
                                for (check, column, content) in &filters {
                                    if (selected.is_empty() || selected == column)
                                        && (query.is_empty() || content.contains(&query))
                                    {
                                        grid.attach(check, index % 2, index / 2, 1, 1);
                                        index += 1;
                                    }
                                }
                            }
                        });
                        let f = filter.clone();
                        profile.task_search.connect_search_changed(move |_| f());
                        profile
                            .task_filter
                            .connect_selected_notify(move |_| filter());
                        if tasks.iter().all(|t| text(t, "column") == "done") {
                            profile.task_box.attach(
                                &label(
                                    "No open tasks. You can launch without an assignment.",
                                    "dim",
                                ),
                                0,
                                0,
                                2,
                                1,
                            );
                        }
                    }
                    status.set_text("Ready to launch");
                    ready.set_sensitive(true);
                }
                Err(e) => status.set_text(&e.to_string()),
            }
        });
        let weak = Rc::downgrade(self);
        start.connect_clicked(move|key|{
            let Some(ui)=weak.upgrade()else{return;};
            if !launch_is_current(&ui,project,generation,&heading){return;}
            if ui.launch_busy.replace(true){return;}
            let group=mode.selected()==1;let two=builders.selected()==1;let indexes=if group{if two{vec![0,2,1]}else{vec![0,2]}}else{(0..count.value_as_int()as usize).collect()};
            let mut profiles_data=Vec::new();
            for index in indexes{
                let p=&profiles[index];if !p.provider_cards[p.provider.selected() as usize].0.is_sensitive(){ui.launch_busy.set(false);ui.show_info("Choose an installed provider for every agent.");return;}let payload=p.payload(project,if group{Some(if index==2{"reviewer"}else{"builder"})}else{None});
                if text(&payload,"worktree").is_empty(){ui.launch_busy.set(false);ui.show_info("Set a worktree for every agent.");return;}
                let tasks=if !group||index==0{p.selected_tasks()}else{Vec::new()};profiles_data.push((payload,tasks,p.prompt()));
            }
            key.set_sensitive(false);let key=key.clone();ui.launch_box.set_sensitive(false);let progress=progress.clone();
            glib::spawn_future_local(async move{
                let result=async{
                    // session.create refreshes new-branch refs before allocation.
                    let mut allocated: Vec<(Value, Vec<i64>, String)> = Vec::new();
                    for (index,(mut payload,tasks,prompt)) in profiles_data.into_iter().enumerate(){
                        progress.set_text(&format!("Allocating agent {}",index+1));
                        if group&&index>0{payload["pair_with"]=json!(text(&allocated[index-1].0,"name"));payload.as_object_mut().unwrap().remove("worktree");}
                        if let Some(task)=tasks.first(){payload["task_id"]=json!(task);}
                        let session=ui.call("session.create",payload).await?;allocated.push((session,tasks,prompt));ui.refresh();
                    }
                    for(session,tasks,_)in &allocated{for task in tasks{progress.set_text(&format!("Staging task #{task}"));ui.call("task.dispatch",json!({"task_id":task,"session":session["name"],"start":false})).await?;}}
                    // Start reviewers first so their mailbox is live before builders publish files.
                    allocated.sort_by_key(|(s,_,_)|text(s,"role")!="reviewer");
                    for(session,_,prompt)in allocated{progress.set_text(&format!("Starting {}",text(&session,"name")));ui.call("session.spawn",json!({"session":session["name"],"prompt":prompt})).await?;}
                    Ok::<(),Error>(())
                }.await;
                match result{Ok(())=>{ui.launch.set_reveal_child(false);ui.open_project(project,"agents");},Err(e)=>{ui.launch.set_reveal_child(false);ui.open_project(project,"agents");ui.show_error(&format!("Launch incomplete: {e}. Created sessions and queues are preserved. Start the remaining sessions individually from the wall."));}}
                ui.launch_busy.set(false);ui.launch_box.set_sensitive(true);key.set_sensitive(true);ui.refresh();
            });
        });
    }
}

fn launch_is_current(ui: &Ui, project: i64, generation: u64, heading: &gtk::Box) -> bool {
    ui.project.get() == project
        && ui.generation.get() == generation
        && ui.launch.reveals_child()
        && ui.launch_box.first_child().as_ref() == Some(heading.upcast_ref())
}
