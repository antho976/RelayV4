use super::*;

/// The most agents one launch configures: the count keys offer 1 to 6.
const AGENTS: usize = 6;

/// The reasoning efforts the effort keys offer, in key order.
const EFFORTS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// One choice made with a row of grouped toggle keys (RA-690). The keys are the control: the
/// chosen index lives in a cell beside them, not in a hidden widget the form has to keep in
/// the tree. The smoke harness drives the row, named, through its keys as a person would.
/// A handler connected to keys should hold [`Choice::downgrade`], not a clone: a clone holds
/// the row, and a row whose key handler holds the row is never freed.
#[derive(Clone)]
struct Choice {
    row: gtk::Box,
    keys: Vec<gtk::ToggleButton>,
    index: Rc<Cell<u32>>,
}

#[derive(Clone)]
struct WeakChoice {
    row: glib::WeakRef<gtk::Box>,
    keys: Vec<glib::WeakRef<gtk::ToggleButton>>,
    index: Rc<Cell<u32>>,
}

impl Choice {
    /// Groups `keys`, already in `row`, with key `initial` chosen.
    fn new(row: gtk::Box, keys: Vec<gtk::ToggleButton>, initial: u32) -> Self {
        let index = Rc::new(Cell::new(initial));
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                key.set_group(keys.first());
            }
            key.set_active(i as u32 == initial);
            let index = index.clone();
            key.connect_toggled(move |key| {
                if key.is_active() {
                    index.set(i as u32);
                }
            });
        }
        Choice { row, keys, index }
    }
    fn selected(&self) -> u32 {
        self.index.get()
    }
    fn set_selected(&self, index: u32) {
        if let Some(key) = self.keys.get(index as usize) {
            key.set_active(true);
        }
    }
    /// Runs `changed` with the new index each time another key is chosen; `selected()`
    /// already returns it.
    fn connect_changed(&self, changed: impl Fn(u32) + 'static) {
        let changed = Rc::new(changed);
        for (i, key) in self.keys.iter().enumerate() {
            let changed = changed.clone();
            key.connect_toggled(move |key| {
                if key.is_active() {
                    changed(i as u32);
                }
            });
        }
    }
    fn set_sensitive(&self, sensitive: bool) {
        self.row.set_sensitive(sensitive);
    }
    fn downgrade(&self) -> WeakChoice {
        WeakChoice {
            row: self.row.downgrade(),
            keys: self.keys.iter().map(|key| key.downgrade()).collect(),
            index: self.index.clone(),
        }
    }
}

impl WeakChoice {
    /// The chosen index, which outlives the keys.
    fn selected(&self) -> u32 {
        self.index.get()
    }
    fn upgrade(&self) -> Option<Choice> {
        Some(Choice {
            row: self.row.upgrade()?,
            keys: self.keys.iter().map(|key| key.upgrade()).collect::<Option<_>>()?,
            index: self.index.clone(),
        })
    }
}

fn choice_cards(choices: &[(&str, &str, &str)]) -> Choice {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.set_homogeneous(true);
    let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (icon, title, copy) in choices {
        let key = gtk::ToggleButton::new();
        key.add_css_class("launch-choice");
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
        row.append(&key);
        buttons.push(key);
    }
    Choice::new(row, buttons, 0)
}

fn segments(choices: &[&str], initial: u32) -> Choice {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("launch-segmented");
    row.set_homogeneous(true);
    let mut keys: Vec<gtk::ToggleButton> = Vec::new();
    for title in choices {
        let key = gtk::ToggleButton::with_label(title);
        row.append(&key);
        keys.push(key);
    }
    Choice::new(row, keys, initial)
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

/// The order an agent takes its tasks in: the one the sheet was opened from, then the others
/// in the order they were ticked. `active` is every selected id in card (board-column) order;
/// one with no recorded tick keeps that order at the end.
fn task_order(active: &[i64], preselect: Option<i64>, ticks: &[i64]) -> Vec<i64> {
    let mut order: Vec<i64> = preselect.filter(|id| active.contains(id)).into_iter().collect();
    for id in ticks.iter().chain(active) {
        if active.contains(id) && !order.contains(id) {
            order.push(*id);
        }
    }
    order
}

/// Note a card being ticked (to the end of the sequence) or unticked (out of it).
fn record_tick(ticks: &mut Vec<i64>, id: i64, active: bool) {
    ticks.retain(|t| *t != id);
    if active {
        ticks.push(id);
    }
}

struct Profile {
    root: gtk::Box,
    provider: Choice,
    role: Choice,
    model: gtk::Entry,
    effort: Choice,
    worktree: gtk::Entry,
    prompt: gtk::TextView,
    writes: gtk::CheckButton,
    ui_access: gtk::CheckButton,
    tasks: RefCell<Vec<(i64, gtk::ToggleButton)>>,
    /// The task the sheet was opened from, and the other ids in the order they were ticked:
    /// the first selected task becomes the agent's current one (RA-558).
    preselect: Cell<Option<i64>>,
    ticks: RefCell<Vec<i64>>,
    task_box: gtk::Grid,
    task_search: gtk::SearchEntry,
    task_filter: Choice,
    task_count: gtk::Label,
    /// Whether the task cards were built: only once the agent is first shown.
    filled: Cell<bool>,
    provider_cards: Vec<(gtk::ToggleButton, gtk::Label, gtk::Label)>,
}
impl Profile {
    fn new(index: usize, compact: bool) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_widget_name(&format!("launch-profile-{index}"));
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
        providers.set_widget_name(&format!("launch-provider-{index}"));
        providers.set_homogeneous(true);
        providers.set_hexpand(true);
        let mut provider_cards: Vec<(gtk::ToggleButton, gtk::Label, gtk::Label)> = Vec::new();
        for (icon, title) in [("claude", "Claude Code"), ("codex", "Codex")] {
            let key = gtk::ToggleButton::new();
            key.add_css_class("launch-provider-card");
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
            providers.append(&key);
            provider_cards.push((key, account, facts));
        }
        let keys = provider_cards.iter().map(|(key, _, _)| key.clone()).collect();
        let provider = Choice::new(providers.clone(), keys, 0);
        configuration.append(&providers);
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 10);
        controls.set_size_request(260, -1);
        let role = segments(&["Builder", "Reviewer", "Docs"], 0);
        controls.append(&label("ROLE", "section-label"));
        controls.append(&role.row);
        let model = gtk::Entry::builder()
            .placeholder_text("Provider default, or enter a model ID")
            .build();
        model.set_widget_name(&format!("launch-model-{index}"));
        model.set_width_chars(1);
        model.set_hexpand(true);
        field("Model", &model, &controls);
        let effort = segments(&EFFORTS, 3);
        effort.row.set_widget_name(&format!("launch-effort-{index}"));
        controls.append(&label("REASONING EFFORT", "section-label"));
        // Minimal is Codex's alone and max Claude's alone.
        effort.keys[0].set_visible(false);
        let weak_effort = effort.downgrade();
        provider.connect_changed(move |provider| {
            let Some(effort) = weak_effort.upgrade() else {
                return;
            };
            let codex = provider == 1;
            effort.keys[0].set_visible(codex);
            effort.keys[5].set_visible(!codex);
            if (codex && effort.selected() == 5) || (!codex && effort.selected() == 0) {
                effort.set_selected(3);
            }
        });
        controls.append(&effort.row);
        configuration.append(&controls);
        root.append(&configuration);
        let prompt = gtk::TextView::new();
        prompt.set_wrap_mode(gtk::WrapMode::WordChar);
        prompt.set_size_request(-1, 100);

        let advanced = gtk::Expander::new(Some("Worktree and permissions"));
        let options = gtk::Box::new(gtk::Orientation::Vertical, 8);
        field("Additional instructions", &prompt, &options);
        // Blank means the project default: a new worktree, or the primary checkout when an
        // enabled plugin asks for it (D160). Only a typed value is sent.
        let worktree = gtk::Entry::builder()
            .placeholder_text("Project default — or new, primary, or an absolute path")
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
        let task_filter = segments(&["All open", "Backlog", "Ready", "Active", "In review"], 0);
        tools.append(&task_filter.row);
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
            preselect: Cell::new(None),
            ticks: RefCell::default(),
            task_box,
            task_search,
            task_filter,
            task_count,
            filled: Cell::new(false),
            provider_cards,
        })
    }
    fn selected_tasks(&self) -> Vec<i64> {
        let active: Vec<i64> = self
            .tasks
            .borrow()
            .iter()
            .filter(|(_, b)| b.is_active())
            .map(|(id, _)| *id)
            .collect();
        task_order(&active, self.preselect.get(), &self.ticks.borrow())
    }
    fn payload(&self, project: i64, role: Option<&str>) -> Value {
        // A review group keeps its own fresh worktree unless the user typed one: groups in the
        // primary checkout collide with solo agents already there.
        let group = role.is_some();
        let role = role.unwrap_or(match self.role.selected() {
            1 => "reviewer",
            2 => "docs",
            _ => "builder",
        });
        let model = self.model.text().trim().to_string();
        let effort = EFFORTS.get(self.effort.selected() as usize);
        let mut payload = json!({"project_id":project,"provider":if self.provider.selected()==0{"claude"}else{"codex"},"role":role,"model":if model.is_empty(){None}else{Some(model)},"effort":effort,"bus_writes":self.writes.is_active(),"allow_ui":self.ui_access.is_active(),"prompt":self.prompt()});
        let worktree = self.worktree.text().trim().to_string();
        if !worktree.is_empty() {
            payload["worktree"] = json!(worktree);
        } else if group {
            payload["worktree"] = json!("new");
        }
        payload
    }
    fn prompt(&self) -> String {
        let b = self.prompt.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).to_string()
    }
}

impl Profile {
    /// Build this agent's task cards from the open tasks. Each agent gets its own, so they are
    /// built only for an agent the user opens, once.
    fn fill_tasks(self: &Rc<Self>, tasks: &[Value], preselect: Option<i64>) {
        if self.filled.replace(true) {
            return;
        }
        let profile = self;
        profile.preselect.set(preselect);
        let mut filters: Vec<(gtk::ToggleButton, String, String)> = Vec::new();
        for (index, row) in tasks
            .iter()
            .filter(|t| text(t, "column") != "done")
            .enumerate()
        {
            let id = row["id"].as_i64().unwrap_or(0);
            let check = gtk::ToggleButton::new();
            check.set_active(preselect == Some(id));
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
            check.connect_toggled(move |check| {
                if let Some(p) = weak.upgrade() {
                    record_tick(&mut p.ticks.borrow_mut(), id, check.is_active());
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
        profile.task_filter.connect_changed(move |_| filter());
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
}

impl Ui {
    pub fn show_launch(self: &Rc<Self>, task: Option<i64>) {
        if self.project.get() == 0 {
            let workspace = self.workspaces.borrow().first().cloned();
            self.open_repository_in(workspace);
            return;
        }
        if self.launch_busy.get() {
            self.show_error("A launch is in progress. Allocated sessions will appear on the wall.");
            return;
        }
        // Already open (Ctrl+N again): keep what the user has filled in.
        if self.launch.reveals_child() {
            self.launch_box.child_focus(gtk::DirectionType::TabForward);
            return;
        }
        if !self.dismiss_panels() {
            return;
        }
        clear(&self.launch_box);
        self.launch.set_reveal_child(true);
        // The form holds a card per open task for every agent shown; once the sheet has
        // finished hiding, however it was closed, nothing keeps it.
        let hidden: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
        let id = {
            let hidden = hidden.clone();
            let launch_box = self.launch_box.downgrade();
            self.launch.connect_child_revealed_notify(move |launch| {
                if launch.is_child_revealed() || launch.reveals_child() {
                    return;
                }
                if let Some(launch_box) = launch_box.upgrade() {
                    clear(&launch_box);
                }
                if let Some(id) = hidden.take() {
                    launch.disconnect(id);
                }
            })
        };
        hidden.set(Some(id));
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

        if !compact {
            body.append(&launch_heading(
                "Session type",
                "Choose how the agents share a worktree.",
            ));
        }
        let mode = choice_cards(
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
        );
        mode.row.set_widget_name("launch-mode");
        body.append(&mode.row);
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
        // The solo agent count: key n launches n + 1 agents.
        let count = segments(&["1", "2", "3", "4", "5", "6"], 0);
        count.row.add_css_class("launch-count");
        count.row.set_widget_name("launch-count");
        members.append(&count.row);
        // A review group: one or two builders, plus the reviewer.
        let builders = segments(&["1 builder", "2 builders"], 0);
        builders.row.set_widget_name("launch-builders");
        members.append(&builders.row);
        // Six solo agents at most; a review group uses the first three.
        let profiles: Rc<Vec<_>> = Rc::new((0..AGENTS).map(|i| Profile::new(i, compact)).collect());
        // The open tasks, once task.list answers; each agent's cards are built from them when
        // the agent is first shown.
        let open_tasks: Rc<RefCell<Option<Vec<Value>>>> = Rc::default();
        let stack = gtk::Stack::new();
        let selector = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        selector.set_homogeneous(true);
        selector.add_css_class("member-rail");
        body.append(&selector);
        body.append(&stack);
        let mut member_keys: Vec<gtk::ToggleButton> = Vec::new();
        for (i, p) in profiles.iter().enumerate() {
            stack.add_named(&p.root, Some(&format!("agent-{i}")));
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
                    let Some(p) = p.upgrade() else {
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
                    let effort = EFFORTS.get(p.effort.selected() as usize).unwrap_or(&"");
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
            for choice in [&p.provider, &p.role, &p.effort, &mode] {
                let refresh = refresh.clone();
                choice.connect_changed(move |_| refresh());
            }
            refresh();
            let stack = stack.downgrade();
            let profile = Rc::downgrade(p);
            let open_tasks = open_tasks.clone();
            key.connect_toggled(move |key| {
                if key.is_active() {
                    if let Some(stack) = stack.upgrade() {
                        stack.set_visible_child_name(&format!("agent-{i}"));
                    }
                    if let (Some(profile), Some(tasks)) = (profile.upgrade(), open_tasks.borrow().as_ref()) {
                        profile.fill_tasks(tasks, None);
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
            let member_keys = member_keys.clone();
            move || {
                let (Some(count_keys), Some(builder_keys), Some(stack)) =
                    (count.row.upgrade(), builders.row.upgrade(), stack.upgrade())
                else {
                    return;
                };
                let group = mode.selected() == 1;
                count_keys.set_visible(!group);
                builder_keys.set_visible(group);
                for (i, p) in profiles.iter().enumerate() {
                    let visible = if group {
                        i == 0 || i == 2 || (i == 1 && builders.selected() == 1)
                    } else {
                        i <= count.selected() as usize
                    };
                    stack.page(&p.root).set_visible(visible);
                    member_keys[i].set_visible(visible);
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
        for choice in [&mode, &builders, &count] {
            let update = update.clone();
            choice.connect_changed(move |_| update());
        }
        update();
        let hint=label("Each solo agent gets the project's default checkout: its own worktree, or the primary checkout when a plugin asks for it. A review group shares one branch, with a separate reviewer and one or two builders. Choose the group's task queue on agent 1.","dim");
        hint.set_wrap(true);
        body.append(&hint);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.add_css_class("launch-footer");
        self.launch_box.append(&footer);
        let progress = label("Loading open tasks…", "dim");
        progress.set_wrap(true);
        progress.set_hexpand(true);
        footer.append(&progress);
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
        let loaded = open_tasks.clone();
        let shown = stack.downgrade();
        let status = progress.clone();
        let ready = start.clone();
        glib::spawn_future_local(async move {
            let (task_result, providers) = tokio::join!(
                ui.call("task.list", json!({"project_id":project})),
                ui.call("provider.list", json!({}))
            );
            if !launch_is_current(&ui, project, generation, &form) {
                if launch_is_shown(&ui, &form) {
                    status.set_text(STALE);
                }
                return;
            }
            // A failed check says so and leaves both providers open: session.create still
            // refuses one that is not installed.
            let (provider_data, provider_error) = match providers {
                Ok(v) => (rows(&v, "providers"), None),
                Err(e) => (Vec::new(), Some(e.to_string())),
            };
            match task_result {
                Ok(v) => {
                    let tasks = rows(&v, "tasks");
                    for profile in p.iter() {
                        for (i, (key, account, facts)) in profile.provider_cards.iter().enumerate()
                        {
                            let name = if i == 0 { "claude" } else { "codex" };
                            let info = provider_data.iter().find(|p| text(p, "provider") == name);
                            let installed = info.is_some_and(|p| p["installed"] == true);
                            key.set_sensitive(installed || provider_error.is_some());
                            account.set_text(
                                &info
                                    .and_then(|p| p["signed_in_as"].as_str())
                                    .map(|s| format!("Signed in as {s}"))
                                    .unwrap_or_else(|| {
                                        if provider_error.is_some() {
                                            "Could not check the CLI".into()
                                        } else if installed {
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
                        // Start on an installed provider: a Codex-only machine opens on Codex.
                        let cards = &profile.provider_cards;
                        if !cards[profile.provider.selected() as usize].0.is_sensitive() {
                            if let Some((key, _, _)) = cards.iter().find(|(key, _, _)| key.is_sensitive()) {
                                key.set_active(true);
                            }
                        }
                    }
                    // Agent 1 now, with the task the sheet was opened for; any other agent
                    // when its key is first pressed.
                    p[0].fill_tasks(&tasks, task);
                    if let Some(index) = shown.upgrade().and_then(|s| s.visible_child_name()).and_then(|n| n.strip_prefix("agent-").and_then(|i| i.parse::<usize>().ok())) {
                        p[index].fill_tasks(&tasks, None);
                    }
                    *loaded.borrow_mut() = Some(tasks);
                    match &provider_error {
                        Some(e) => status.set_text(&format!("Ready to launch · could not check the provider CLIs: {e}")),
                        None => status.set_text("Ready to launch"),
                    }
                    ready.set_sensitive(true);
                }
                Err(e) => status.set_text(&e.to_string()),
            }
        });
        let weak = Rc::downgrade(self);
        start.connect_clicked(move|key|{
            let Some(ui)=weak.upgrade()else{return;};
            if !launch_is_current(&ui,project,generation,&heading){
                // Still on screen after the engine reconnected: say so instead of doing nothing.
                if launch_is_shown(&ui,&heading){ui.show_error(STALE);}
                return;
            }
            if ui.launch_busy.replace(true){return;}
            let group=mode.selected()==1;let two=builders.selected()==1;let indexes=if group{if two{vec![0,2,1]}else{vec![0,2]}}else{(0..=count.selected()as usize).collect()};
            let mut profiles_data=Vec::new();
            for index in indexes{
                let p=&profiles[index];if !p.provider_cards[p.provider.selected() as usize].0.is_sensitive(){ui.launch_busy.set(false);ui.show_error("Choose an installed provider for every agent.");return;}let payload=p.payload(project,if group{Some(if index==2{"reviewer"}else{"builder"})}else{None});
                let tasks=if !group||index==0{p.selected_tasks()}else{Vec::new()};profiles_data.push((payload,tasks,p.prompt()));
            }
            key.set_sensitive(false);let key=key.clone();ui.launch_box.set_sensitive(false);
            // The sheet steps aside on the click and the new agents appear at once, each pane
            // saying which step it is on; the worktree fetch and checkout happen behind them.
            let clicked=std::time::Instant::now();
            ui.launch.set_reveal_child(false);ui.open_project(project,"agents");
            let payloads:Vec<Value>=profiles_data.iter().map(|(payload,_,_)|payload.clone()).collect();
            let placeholders=ui.launch_placeholders(project,&payloads);
            tracing::debug!(agents=placeholders.len(),elapsed_us=clicked.elapsed().as_micros() as u64,"launch: placeholders visible");
            glib::spawn_future_local(async move{
                let mut touched:Vec<String>=placeholders.clone();
                let agents=profiles_data.len();
                // What the failure message can truthfully say: sessions that exist, and tasks
                // that were meant for one of them but are not on its queue.
                let mut created:Vec<String>=Vec::new();let mut unstaged:Vec<i64>=Vec::new();
                let result=async{
                    // session.create refreshes new-branch refs before allocation.
                    let mut allocated: Vec<(Value, Vec<i64>, String)> = Vec::new();
                    for (index,(mut payload,tasks,prompt)) in profiles_data.into_iter().enumerate(){
                        ui.launch_progress(&placeholders[index],if group&&index>0{"Joining the shared worktree…"}else{"Preparing worktree…"});
                        if group&&index>0{payload["pair_with"]=json!(text(&allocated[index-1].0,"name"));payload.as_object_mut().unwrap().remove("worktree");}
                        if let Some(task)=tasks.first(){payload["task_id"]=json!(task);}
                        let session=ui.call("session.create",payload).await?;
                        ui.launch_adopt(&placeholders[index],&session,"Worktree ready · waiting to start…");touched.push(text(&session,"name").to_string());created.push(text(&session,"name").to_string());
                        tracing::debug!(session=text(&session,"name"),elapsed_ms=clicked.elapsed().as_millis() as u64,"launch: session created");
                        // Stage this agent's queue now, so a later agent's failure cannot cost it.
                        unstaged.extend(&tasks);
                        for task in &tasks{ui.launch_progress(text(&session,"name"),&format!("Staging task #{task}…"));ui.call("task.dispatch",json!({"task_id":task,"session":session["name"],"start":false})).await?;unstaged.retain(|t|t!=task);}
                        allocated.push((session,tasks,prompt));
                    }
                    // Start reviewers first so their mailbox is live before builders publish files.
                    allocated.sort_by_key(|(s,_,_)|text(s,"role")!="reviewer");
                    for(session,_,prompt)in allocated{
                        let name=text(&session,"name").to_string();
                        ui.launch_progress(&name,"Starting the provider…");
                        let started=ui.call("session.spawn",json!({"session":name,"prompt":prompt})).await;
                        ui.launch_done(&name);
                        let started=started?;ui.upsert_session(&started);
                        tracing::debug!(session=%name,elapsed_ms=clicked.elapsed().as_millis() as u64,"launch: provider started");
                    }
                    Ok::<(),Error>(())
                }.await;
                ui.launch_abort(&touched);
                if let Err(e)=result{
                    let mut message=if created.is_empty(){format!("Launch failed: {e}. No session was created.")}else{format!("Launch incomplete: {e}. Created {}; start them individually from the wall.",created.join(", "))};
                    if !created.is_empty()&&created.len()<agents{message.push_str(&format!(" {} of {agents} agents were not created.",agents-created.len()));}
                    if !unstaged.is_empty(){message.push_str(&format!(" Not queued: {}.",unstaged.iter().map(|t|format!("#{t}")).collect::<Vec<_>>().join(", ")));}
                    ui.show_error(&message);
                }
                ui.launch_busy.set(false);ui.launch_box.set_sensitive(true);key.set_sensitive(true);ui.refresh();
            });
        });
    }
}

const STALE: &str = "This form is out of date after a reconnect. Close it and open New session again.";

fn launch_is_current(ui: &Ui, project: i64, generation: u64, heading: &gtk::Box) -> bool {
    ui.project.get() == project && ui.generation.get() == generation && launch_is_shown(ui, heading)
}

/// This form is the one the sheet shows, whatever has changed under it.
fn launch_is_shown(ui: &Ui, heading: &gtk::Box) -> bool {
    ui.launch.reveals_child() && ui.launch_box.first_child().as_ref() == Some(heading.upcast_ref())
}

#[cfg(test)]
mod tests {
    use super::{record_tick, task_order};

    #[test]
    fn the_opening_task_comes_first_then_the_tick_order() {
        // Card order is backlog first: #3 (backlog), #7 (backlog), #12 (ready).
        let mut ticks = Vec::new();
        record_tick(&mut ticks, 7, true);
        record_tick(&mut ticks, 3, true);
        assert_eq!(task_order(&[3, 7, 12], Some(12), &ticks), [12, 7, 3]);
        // Unticking drops a task; ticking it again puts it last.
        record_tick(&mut ticks, 7, false);
        assert_eq!(task_order(&[3, 12], Some(12), &ticks), [12, 3]);
        record_tick(&mut ticks, 7, true);
        assert_eq!(task_order(&[3, 7, 12], Some(12), &ticks), [12, 3, 7]);
    }

    #[test]
    fn an_unticked_opening_task_is_left_out() {
        assert_eq!(task_order(&[3, 7], Some(12), &[7, 3]), [7, 3]);
        assert_eq!(task_order(&[3, 7], None, &[7]), [7, 3]);
        assert!(task_order(&[], Some(12), &[]).is_empty());
    }
}
