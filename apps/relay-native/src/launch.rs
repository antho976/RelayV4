use super::*;

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
    tasks: RefCell<Vec<(i64, gtk::CheckButton)>>,
    task_box: gtk::Box,
}
impl Profile {
    fn new(index: usize) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let provider = gtk::DropDown::from_strings(&["Claude", "Codex"]);
        if index == 2 {
            provider.set_selected(1);
        }
        field("Provider", &provider, &root);
        let role = gtk::DropDown::from_strings(&["Builder", "Reviewer", "Docs"]);
        field("Role", &role, &root);
        let model = gtk::Entry::builder()
            .placeholder_text("Provider default")
            .build();
        field("Model", &model, &root);
        let effort =
            gtk::DropDown::from_strings(&["Default", "low", "medium", "high", "xhigh", "max"]);
        field("Reasoning effort", &effort, &root);
        let worktree = gtk::Entry::builder()
            .text("new")
            .placeholder_text("new, primary, or absolute worktree path")
            .build();
        field("Worktree", &worktree, &root);
        let prompt = gtk::TextView::new();
        prompt.set_wrap_mode(gtk::WrapMode::WordChar);
        prompt.set_size_request(-1, 120);
        field("Assignment", &prompt, &root);
        let writes = gtk::CheckButton::with_label("Allow agent bus writes");
        writes.set_active(true);
        root.append(&writes);
        let ui_access = gtk::CheckButton::with_label("Allow UI control");
        root.append(&ui_access);
        let expander = gtk::Expander::new(Some("Task queue"));
        let task_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        expander.set_child(Some(&task_box));
        root.append(&expander);
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
            self.show_error("Open a repository before adding agents.");
            return;
        }
        if self.launch_busy.get() {
            self.show_error("A launch is in progress. Allocated sessions will appear on the wall.");
            return;
        }
        clear(&self.launch_box);
        self.launch.set_reveal_child(true);
        self.launch_box.append(&label("New session", "title"));
        let mode = gtk::DropDown::from_strings(&["Solo agents", "Review group"]);
        mode.set_widget_name("launch-mode");
        field("Launch", &mode, &self.launch_box);
        let count = gtk::SpinButton::with_range(1.0, 11.0, 1.0);
        count.set_value(1.0);
        field("Solo agents", &count, &self.launch_box);
        let builders =
            gtk::DropDown::from_strings(&["One builder + reviewer", "Two builders + reviewer"]);
        field("Review group", &builders, &self.launch_box);
        let profiles: Rc<Vec<_>> = Rc::new((0..11).map(Profile::new).collect());
        let stack = gtk::Stack::new();
        let selector = gtk::StackSwitcher::new();
        selector.set_stack(Some(&stack));
        self.launch_box.append(&selector);
        self.launch_box.append(&stack);
        for (i, p) in profiles.iter().enumerate() {
            stack.add_titled(&p.root, Some(&format!("agent-{i}")), &format!("{}", i + 1));
        }
        let update: Rc<dyn Fn()> = Rc::new({
            let profiles = profiles.clone();
            let mode = mode.clone();
            let count = count.clone();
            let builders = builders.clone();
            let stack = stack.clone();
            move || {
                let group = mode.selected() == 1;
                count.set_sensitive(!group);
                builders.set_sensitive(group);
                for (i, p) in profiles.iter().enumerate() {
                    let visible = if group {
                        i == 0 || i == 2 || (i == 1 && builders.selected() == 1)
                    } else {
                        i < count.value_as_int() as usize
                    };
                    stack.page(&p.root).set_visible(visible);
                    p.role.set_sensitive(!group);
                    p.worktree.set_sensitive(!group || i == 0);
                    if group {
                        p.role.set_selected(if i == 2 { 1 } else { 0 });
                    }
                }
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
        self.launch_box.append(&hint);
        let progress = label("Loading open tasks…", "dim");
        progress.set_wrap(true);
        self.launch_box.append(&progress);
        builders.set_widget_name("launch-builders");
        let start = button("Launch", "primary");
        start.set_widget_name("launch-start");
        start.set_sensitive(false);
        self.launch_box.append(&start);
        let cancel = button("Cancel", "quiet");
        self.launch_box.append(&cancel);
        let weak = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.launch.set_reveal_child(false);
            }
        });
        let project = self.project.get();
        let ui = self.clone();
        let p = profiles.clone();
        let status = progress.clone();
        let ready = start.clone();
        glib::spawn_future_local(async move {
            match ui.call("task.list", json!({"project_id":project})).await {
                Ok(v) => {
                    let tasks = rows(&v, "tasks");
                    for profile in p.iter() {
                        for row in tasks.iter().filter(|t| text(t, "column") != "done") {
                            let id = row["id"].as_i64().unwrap_or(0);
                            let check = gtk::CheckButton::with_label(&format!(
                                "#{} {}",
                                id,
                                text(row, "title")
                            ));
                            check.set_active(task == Some(id));
                            profile.task_box.append(&check);
                            profile.tasks.borrow_mut().push((id, check));
                        }
                    }
                    status.set_text(
                        "All identities and task queues are prepared before agents start.",
                    );
                    ready.set_sensitive(true);
                }
                Err(e) => status.set_text(&e.to_string()),
            }
        });
        let weak = Rc::downgrade(self);
        start.connect_clicked(move|key|{
            let Some(ui)=weak.upgrade()else{return;};if ui.launch_busy.replace(true){return;}
            let group=mode.selected()==1;let two=builders.selected()==1;let indexes=if group{if two{vec![0,2,1]}else{vec![0,2]}}else{(0..count.value_as_int()as usize).collect()};
            let mut profiles_data=Vec::new();
            for index in indexes{
                let p=&profiles[index];let payload=p.payload(project,if group{Some(if index==2{"reviewer"}else{"builder"})}else{None});
                if text(&payload,"worktree").is_empty(){ui.launch_busy.set(false);ui.show_error("Set a worktree for every agent.");return;}
                let tasks=if !group||index==0{p.selected_tasks()}else{Vec::new()};profiles_data.push((payload,tasks,p.prompt()));
            }
            key.set_sensitive(false);let key=key.clone();ui.launch_box.set_sensitive(false);let progress=progress.clone();
            glib::spawn_future_local(async move{
                let result=async{
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
