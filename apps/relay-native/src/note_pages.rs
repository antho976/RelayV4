use super::task_pages::{buffer_text, choose, chosen, multiline, Draft};
use super::*;

fn find_note(view: &gtk::TextView, needle: &str, backward: bool) {
    if needle.is_empty() {
        return;
    }
    let buffer = view.buffer();
    let cursor = buffer
        .selection_bounds()
        .map(|(start, end)| if backward { start } else { end })
        .unwrap_or_else(|| buffer.iter_at_offset(buffer.cursor_position()));
    let flags = gtk::TextSearchFlags::CASE_INSENSITIVE;
    let found = if backward {
        cursor
            .backward_search(needle, flags, None)
            .or_else(|| buffer.end_iter().backward_search(needle, flags, None))
    } else {
        cursor
            .forward_search(needle, flags, None)
            .or_else(|| buffer.start_iter().forward_search(needle, flags, None))
    };
    if let Some((mut start, end)) = found {
        buffer.select_range(&end, &start);
        view.scroll_to_iter(&mut start, 0., false, 0., 0.);
    }
}

fn replace_note(view: &gtk::TextView, needle: &str, replacement: &str, all: bool) {
    if needle.is_empty() {
        return;
    }
    let buffer = view.buffer();
    buffer.begin_user_action();
    if all {
        let mut cursor = buffer.start_iter();
        while let Some((mut start, mut end)) =
            cursor.forward_search(needle, gtk::TextSearchFlags::CASE_INSENSITIVE, None)
        {
            buffer.delete(&mut start, &mut end);
            buffer.insert(&mut start, replacement);
            cursor = start;
        }
    } else if let Some((mut start, mut end)) = buffer.selection_bounds() {
        if buffer.text(&start, &end, false).to_lowercase() == needle.to_lowercase() {
            buffer.delete(&mut start, &mut end);
            buffer.insert(&mut start, replacement);
        }
    }
    buffer.end_user_action();
    if !all {
        find_note(view, needle, false);
    }
}

pub fn verify_tools() {
    let view = gtk::TextView::new();
    view.buffer().set_text("one ONE one");
    find_note(&view, "one", true);
    assert_eq!(
        view.buffer()
            .selection_bounds()
            .map(|(a, b)| (a.offset(), b.offset())),
        Some((8, 11))
    );
    replace_note(&view, "one", "two", false);
    assert_eq!(buffer_text(&view.buffer()), "one ONE two");
    replace_note(&view, "one", "one+", true);
    assert_eq!(buffer_text(&view.buffer()), "one+ one+ two");
}

fn editor(ui: &Rc<Ui>, note: Value) -> Rc<Draft> {
    let id = note["id"].as_i64().unwrap_or(0);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 0);
    form.set_vexpand(true);
    form.add_css_class("document-editor");
    let document_head = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    document_head.add_css_class("document-head");
    let title = gtk::Entry::builder().text(text(&note, "title")).build();
    title.set_widget_name("note-title");
    title.add_css_class("document-title");
    title.set_hexpand(true);
    title.set_valign(gtk::Align::Center);
    document_head.append(&title);
    let pin = gtk::CheckButton::with_label("Pin");
    pin.set_active(note["pinned"].as_bool().unwrap_or(false));
    pin.set_valign(gtk::Align::Center);
    document_head.append(&pin);
    form.append(&document_head);

    let body = multiline(text(&note, "body"), 100);
    body.set_vexpand(true);
    body.add_css_class("document-text");
    body.set_monospace(true);
    body.set_widget_name("note-body");
    let inset = move |width: i32| (width as f64 * 0.04).round().max(22.) as i32;
    let window = ui
        .notes_window
        .borrow()
        .as_ref()
        .map(|owner| owner.window.clone())
        .unwrap_or_else(|| ui.window.clone().upcast());
    body.set_left_margin(inset(window.width()));
    body.set_right_margin(inset(window.width()));
    if let Some(surface) = window.surface() {
        let target = body.downgrade();
        let handler = surface.connect_layout(move |_, width, _| {
            if let Some(view) = target.upgrade() {
                view.set_left_margin(inset(width));
                view.set_right_margin(inset(width));
            }
        });
        let handler = RefCell::new(Some(handler));
        body.connect_destroy(move |_| {
            if let Some(handler) = handler.borrow_mut().take() {
                surface.disconnect(handler);
            }
        });
    }
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    toolbar.add_css_class("document-toolbar");

    for (caption, hint, before, after) in [
        ("B", "Bold", "**", "**"),
        ("I", "Italic", "_", "_"),
        ("H2", "Heading", "\n## ", ""),
        ("•≡", "Bulleted list", "\n- ", ""),
        ("1≡", "Numbered list", "\n1. ", ""),
        ("☐", "Checklist", "\n- [ ] ", ""),
        ("❯", "Quote", "\n> ", ""),
        ("<>", "Inline code", "`", "`"),
        ("▣", "Code block", "\n```\n", "\n```\n"),
        ("↗", "Link", "[", "](url)"),
    ] {
        if hint == "Checklist" {
            continue;
        }
        let key = button(caption, "quiet");
        key.set_valign(gtk::Align::Center);
        key.set_tooltip_text(Some(hint));
        if hint == "Bulleted list" {
            let separator = gtk::Separator::new(gtk::Orientation::Vertical);
            separator.add_css_class("document-tool-separator");
            toolbar.append(&separator);
        }
        if hint == "Bold" {
            key.add_css_class("document-tool-bold");
        }
        if hint == "Italic" {
            key.add_css_class("document-tool-italic");
        }
        toolbar.append(&key);
        let buffer = body.buffer();
        key.connect_clicked(move |_| {
            let (mut start, mut end) = buffer.selection_bounds().unwrap_or_else(|| {
                let cursor = buffer.iter_at_offset(buffer.cursor_position());
                (cursor, cursor)
            });
            let selected = buffer.text(&start, &end, false);
            let replacement = format!("{before}{selected}{after}");
            buffer.begin_user_action();
            buffer.delete(&mut start, &mut end);
            buffer.insert(&mut start, &replacement);
            buffer.end_user_action();
        });
    }
    form.append(&toolbar);
    let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let query = gtk::Entry::builder()
        .placeholder_text("Find")
        .hexpand(true)
        .build();
    let replacement = gtk::Entry::builder()
        .placeholder_text("Replace with")
        .hexpand(true)
        .build();
    let matches = label("0 matches", "dim");
    find_row.append(&query);
    find_row.append(&matches);
    for (caption, backward) in [("Previous", true), ("Next", false)] {
        let key = button(caption, "quiet");
        find_row.append(&key);
        let view = body.clone();
        let query = query.clone();
        key.connect_clicked(move |_| find_note(&view, &query.text(), backward));
    }
    find_row.append(&replacement);
    for (caption, all) in [("Replace", false), ("All", true)] {
        let key = button(caption, "quiet");
        find_row.append(&key);
        let view = body.clone();
        let query = query.clone();
        let replacement = replacement.clone();
        key.connect_clicked(move |_| replace_note(&view, &query.text(), &replacement.text(), all));
    }
    let close = crate::app::icon_button("close", "Close find");
    find_row.append(&close);
    let target = find_row.downgrade();
    close.connect_clicked(move |_| {
        if let Some(row) = target.upgrade() {
            row.set_visible(false);
        }
    });
    find_row.add_css_class("document-find");
    find_row.set_visible(false);
    let find_toggle = crate::app::icon_button("search", "Find and replace");

    let separator = gtk::Separator::new(gtk::Orientation::Vertical);
    separator.add_css_class("document-tool-separator");
    toolbar.append(&separator);
    toolbar.append(&find_toggle);

    let find_box = find_row.clone();
    find_toggle.connect_clicked(move |_| {
        find_box.set_visible(!find_box.is_visible());
    });
    form.append(&find_row);
    let buffer = body.buffer();
    let q = query.clone();
    let count = matches.clone();
    let update: Rc<dyn Fn()> = Rc::new(move || {
        let needle = q.text().to_lowercase();
        let count_value = if needle.is_empty() {
            0
        } else {
            buffer_text(&buffer).to_lowercase().matches(&needle).count()
        };
        count.set_text(&format!("{count_value} matches"));
    });
    let changed = update.clone();
    query.connect_changed(move |_| changed());
    body.buffer().connect_changed(move |_| update());
    let view = body.clone();
    let q = query.clone();
    query.connect_activate(move |_| find_note(&view, &q.text(), false));
    let keyboard = gtk::EventControllerKey::new();
    let row = find_row.clone();
    let q = query.clone();
    keyboard.connect_key_pressed(move |_, key, _, mods| {
        if key == gtk::gdk::Key::f && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            row.set_visible(true);
            q.grab_focus();
            glib::Propagation::Stop
        } else if key == gtk::gdk::Key::Escape && row.is_visible() {
            row.set_visible(false);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    form.add_controller(keyboard);
    let wrap = gtk::ToggleButton::with_label("↩");
    wrap.add_css_class("quiet");
    wrap.set_valign(gtk::Align::Center);
    wrap.set_active(true);
    let view = body.clone();
    wrap.connect_toggled(move |key| {
        view.set_wrap_mode(if key.is_active() {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        })
    });
    wrap.set_tooltip_text(Some("Toggle line wrap"));

    let stamp = crate::app::icon_button("history", "Insert timestamp");
    let buffer = body.buffer();
    stamp.connect_clicked(move |_| {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(stamp) = now.format("%x %X") {
                buffer.insert_at_cursor(&stamp);
            }
        }
    });
    toolbar.append(&stamp);
    toolbar.append(&wrap);
    let size = Rc::new(std::cell::Cell::new(13_i32));
    let provider = gtk::CssProvider::new();
    body.style_context()
        .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    for (caption, delta) in [("A−", -1), ("A+", 1)] {
        let key = button(caption, "quiet");
        key.set_tooltip_text(Some(if delta < 0 {
            "Decrease text size"
        } else {
            "Increase text size"
        }));
        let size = size.clone();
        let provider = provider.clone();
        key.connect_clicked(move |_| {
            size.set((size.get() + delta).clamp(11, 17));
            provider.load_from_string(&format!(
                "textview.document-text {{ font-size: {}px; }}",
                size.get()
            ));
        });
        toolbar.append(&key);
    }

    let editor_scroll = crate::app::scrolled(&body);

    form.append(&editor_scroll);

    let count = label("", "dim");
    let buffer = body.buffer();
    let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    status_bar.add_css_class("document-status");
    count.remove_css_class("dim");
    status_bar.append(&count);

    form.append(&status_bar);
    let update = move |b: &gtk::TextBuffer| {
        let value = buffer_text(b);
        count.set_text(&format!(
            "{} lines · {} words · {} characters",
            value.lines().count().max(1),
            value.split_whitespace().count(),
            value.chars().count()
        ));
    };
    update(&buffer);
    buffer.connect_changed(update);
    let dirty_title = title.clone();
    let dirty_pin = pin.clone();
    let snapshot: Rc<dyn Fn() -> Value> = Rc::new(
        move || json!({"title":if title.text().trim().is_empty(){Value::Null}else{json!(title.text().trim())},"body":buffer_text(&body.buffer()),"pinned":pin.is_active()}),
    );
    let draft = Draft::new_note(ui, "Note", note, snapshot, form);
    draft.controls(ui, "notes.get", "notes.update", "note_id", id);
    draft.layout.add_css_class("note-draft");
    for child in super::widgets(&draft.footer) {
        if let Ok(key) = child.downcast::<gtk::Button>() {
            match key.label().as_deref() {
                Some("Close") => key.set_visible(false),
                Some("Discard and close") => {
                    key.set_label("Discard");
                    key.set_tooltip_text(Some("Discard changes and close this note"));
                }
                _ => {}
            }
        }
    }
    draft.layout.remove(&draft.footer);
    draft.footer.set_valign(gtk::Align::Center);

    document_head.append(&draft.footer);

    draft.layout.remove(&draft.status);
    draft.status.add_css_class("document-save-status");
    draft.status.set_text("Saved");

    status_bar.prepend(&draft.status);

    draft.layout.set_spacing(0);
    draft.layout.set_margin_top(0);
    draft.layout.set_margin_bottom(0);
    draft.layout.set_margin_start(0);
    draft.layout.set_margin_end(0);
    draft.form.set_valign(gtk::Align::Fill);
    if let Some(scroll) = draft
        .layout
        .first_child()
        .and_downcast::<gtk::ScrolledWindow>()
    {
        scroll.set_child(gtk::Widget::NONE);
        draft.layout.remove(&scroll);
        draft.layout.prepend(&draft.form);
    }

    super::task_pages::action(
        ui,
        &draft,
        &draft.footer,
        "Delete",
        "notes.delete",
        move || json!({"note_id":id}),
        None,
    );

    let sync: Rc<dyn Fn()> = Rc::new({
        let weak = Rc::downgrade(&draft);
        move || {
            if let Some(draft) = weak.upgrade() {
                if draft.busy.get() {
                    return;
                }
                let dirty = draft.dirty();
                draft
                    .status
                    .set_text(if dirty { "Modified" } else { "Saved" });
                for child in super::widgets(&draft.footer) {
                    if let Ok(key) = child.downcast::<gtk::Button>() {
                        if key.widget_name() == "draft-save" {
                            key.set_sensitive(dirty);
                        }
                        if key.label().as_deref() == Some("Discard") {
                            key.set_visible(dirty);
                        }
                    }
                }
            }
        }
    });
    let changed = sync.clone();
    dirty_title.connect_changed(move |_| changed());
    let changed = sync.clone();
    dirty_pin.connect_toggled(move |_| changed());
    let changed = sync.clone();
    buffer.connect_changed(move |_| changed());
    sync();

    draft
}
pub fn edit(ui: &Rc<Ui>, note: Value) {
    ui.note_tabs.set_visible(true);
    if let Some(host) = ui.note_tabs.parent() {
        for child in super::widgets(&host) {
            if child.has_css_class("notes-welcome") {
                child.set_visible(false);
            }
        }
    }
    let id = note["id"].as_i64().unwrap_or(0);
    if let Some(d) = ui.note_drafts.borrow().get(&id) {
        d.layout.set_visible(true);
        if let Some(page) = ui.note_tabs.page_num(&d.layout) {
            ui.note_tabs.set_current_page(Some(page));
        }
        return;
    }
    let d = editor(ui, note.clone());
    let title = if text(&note, "title").is_empty() {
        "Untitled"
    } else {
        text(&note, "title")
    };
    let tab = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let project = note["project_id"].as_i64().unwrap_or(ui.project.get());
    let project_name = ui
        .projects
        .borrow()
        .iter()
        .find(|p| p["id"].as_i64() == Some(project))
        .map(|p| text(p, "name").to_string())
        .unwrap_or_else(|| format!("Project {project}"));
    tab.add_css_class("document-tab");
    tab.set_tooltip_text(Some(&format!("{project_name} · {title}")));
    tab.append(&label(title, "dim"));
    let close = crate::app::icon_button("window-close-symbolic", "Close note");
    tab.append(&close);
    let page = ui.note_tabs.append_page(&d.layout, Some(&tab));
    ui.note_tabs.set_tab_reorderable(&d.layout, true);
    ui.note_tabs.set_current_page(Some(page));
    ui.note_tabs.set_scrollable(true);
    let weak = Rc::downgrade(ui);
    let draft = Rc::downgrade(&d);
    *d.on_close.borrow_mut() = Some(Box::new(move || {
        if let (Some(ui), Some(d)) = (weak.upgrade(), draft.upgrade()) {
            if let Some(page) = ui.note_tabs.page_num(&d.layout) {
                ui.note_tabs.remove_page(Some(page));
            }
            ui.note_drafts.borrow_mut().remove(&id);
            super::refresh_notes(&ui);
        }
    }));
    let draft = d.clone();
    close.connect_clicked(move |_| draft.close());
    ui.note_drafts.borrow_mut().insert(id, d);
}

pub fn workspace(ui: &Rc<Ui>, name: &str, project: i64, notes: &[Value]) {
    let page = &ui.pages[name];
    page.add_css_class("notes-page");
    page.set_spacing(0);
    ui.note_tabs.set_show_tabs(true);
    ui.note_tabs.set_visible(true);
    // Retain editors while the project library refreshes.
    if let Some(parent) = ui.note_tabs.parent() {
        if let Ok(paned) = parent.clone().downcast::<gtk::Paned>() {
            paned.set_end_child(gtk::Widget::NONE);
        } else if let Ok(container) = parent.downcast::<gtk::Box>() {
            container.remove(&ui.note_tabs);
        }
    }
    clear(page);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("notes-app-head");
    let icon = crate::icons::image("notes", 19);
    icon.add_css_class("notes-app-icon");
    icon.set_valign(gtk::Align::Center);
    header.append(&icon);
    let heading = gtk::Box::new(gtk::Orientation::Vertical, 2);
    heading.append(&label("Notes", "title"));
    heading.append(&label("Plain-text project context agents can read.", "dim"));
    heading.set_hexpand(true);
    heading.set_valign(gtk::Align::Center);
    header.append(&heading);
    header.append(&super::workspace_picker(ui, project, name));

    let add = button("New document", "primary");
    add.set_valign(gtk::Align::Center);
    header.append(&add);
    let weak = Rc::downgrade(ui);
    add.connect_clicked(move |key| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let key = key.clone();
        key.set_sensitive(false);
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "notes.create",
                    json!({"project_id":project,"title":"Untitled","body":"","pinned":false}),
                )
                .await
            {
                Ok(note) => {
                    edit(&ui, note);
                    super::refresh_notes(&ui);
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            key.set_sensitive(true);
        });
    });
    page.append(&header);
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_widget_name("notes-library-split");
    let owner = ui.notes_window.borrow().clone();
    split.set_position(owner.as_ref().map_or(270, |window| window.rail_width.get()));
    split.add_css_class("notes-workspace");
    split.set_vexpand(true);
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    sidebar.set_size_request(180, -1);
    sidebar.add_css_class("document-library");
    sidebar.set_visible(
        owner
            .as_ref()
            .is_none_or(|window| !window.rail_collapsed.get()),
    );
    let collapse = crate::app::icon_button("sidebar", "Toggle document library");
    collapse.set_widget_name("notes-library-toggle");
    collapse.set_valign(gtk::Align::Center);
    header.prepend(&collapse);
    let library = sidebar.clone();
    let weak = Rc::downgrade(ui);
    collapse.connect_clicked(move |_| {
        library.set_visible(!library.is_visible());
        if let Some(ui) = weak.upgrade() {
            if let Some(window) = ui.notes_window.borrow().as_ref() {
                window.rail_collapsed.set(!library.is_visible());
                window.persist(&ui);
            }
        }
    });
    let weak = Rc::downgrade(ui);
    split.connect_position_notify(move |split| {
        if let Some(ui) = weak.upgrade() {
            if let Some(window) = ui.notes_window.borrow().as_ref() {
                if !window.rail_collapsed.get() {
                    window.rail_width.set(split.position().max(180));
                }
                window.persist(&ui);
            }
        }
    });
    let library_head = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    library_head.add_css_class("document-library-head");
    let documents = label("Documents", "dim");
    documents.set_hexpand(true);
    library_head.append(&documents);
    library_head.append(&label(&notes.len().to_string(), "dim"));
    sidebar.append(&library_head);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search notes"));
    sidebar.append(&search);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    sidebar.append(&crate::app::scrolled(&list));
    let mut filters = Vec::new();
    for note in notes {
        let title = text(note, "title");
        let b = button("", "document-row");
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.append(&label(
            if title.is_empty() { "Untitled" } else { title },
            "document-row-title",
        ));
        let preview = text(note, "body")
            .lines()
            .next()
            .unwrap_or("Empty document");
        let preview = label(preview, "document-preview");
        preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
        preview.set_max_width_chars(32);
        row.append(&preview);
        let date = glib::DateTime::from_iso8601(text(note, "updated_at"), None)
            .and_then(|date| date.to_local())
            .and_then(|date| date.format("%b %-d"))
            .map(|date| date.to_string())
            .unwrap_or_else(|_| text(note, "updated_at").to_string());
        row.append(&label(&date, "document-date"));
        if note["pinned"] == true {
            b.add_css_class("pinned");
        }
        b.set_child(Some(&row));
        list.append(&b);
        filters.push((
            format!("{} {}", title, text(note, "body")).to_lowercase(),
            b.clone(),
        ));
        let weak = Rc::downgrade(ui);
        let note = note.clone();
        b.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                edit(&ui, note.clone());
            }
        });
    }
    search.connect_search_changed(move |s| {
        let query = s.text().to_lowercase();
        for (text, b) in &filters {
            b.set_visible(text.contains(&query));
        }
    });
    split.set_start_child(Some(&sidebar));
    let editor_host = gtk::Box::new(gtk::Orientation::Vertical, 0);
    editor_host.set_vexpand(true);
    editor_host.append(&ui.note_tabs);
    let welcome = gtk::Box::new(gtk::Orientation::Vertical, 9);
    welcome.add_css_class("notes-welcome");
    welcome.set_valign(gtk::Align::Center);
    welcome.set_halign(gtk::Align::Center);
    welcome.set_vexpand(true);
    welcome.append(&label("Open a document", "title"));
    welcome.append(&label(
        "Choose a note from the library or create a new document.",
        "dim",
    ));
    editor_host.append(&welcome);
    split.set_end_child(Some(&editor_host));
    page.append(&split);
    let changed_project = owner
        .as_ref()
        .is_some_and(|w| w.rendered_project.replace(project) != project);
    let mut selected = None;
    for draft in ui.note_drafts.borrow().values() {
        let belongs = draft.base.borrow()["project_id"].as_i64() == Some(project);
        draft.layout.set_visible(belongs);
        if belongs && selected.is_none() {
            selected = ui.note_tabs.page_num(&draft.layout);
        }
    }
    if changed_project {
        if let Some(page) = selected {
            ui.note_tabs.set_current_page(Some(page));
        } else if let Some(note) = notes.first() {
            edit(ui, note.clone());
        }
    }
    let has_document = ui
        .note_drafts
        .borrow()
        .values()
        .any(|draft| draft.base.borrow()["project_id"].as_i64() == Some(project));
    ui.note_tabs.set_visible(has_document);
    welcome.set_visible(!has_document);
}

pub fn note_row(ui: &Rc<Ui>, body: &gtk::Box, note: Value) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    row.add_css_class("record");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = label(
        if text(&note, "title").is_empty() {
            "Untitled"
        } else {
            text(&note, "title")
        },
        "title",
    );
    title.set_hexpand(true);
    heading.append(&title);
    let edit_key = button("Open editor", "quiet");
    let weak = Rc::downgrade(ui);
    let n = note.clone();
    edit_key.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, n.clone())
        }
    });
    heading.append(&edit_key);
    let pin = button(
        if note["pinned"] == true {
            "Unpin"
        } else {
            "Pin"
        },
        "quiet",
    );
    let weak = Rc::downgrade(ui);
    let id = note["id"].clone();
    let pinned = note["pinned"] != true;
    pin.connect_clicked(move |b| {
        if let Some(ui) = weak.upgrade() {
            ui.mutate("notes.pin", json!({"note_id":id,"pinned":pinned}), b)
        }
    });
    heading.append(&pin);
    row.append(&heading);
    let preview: String = text(&note, "body").chars().take(500).collect();
    row.append(&paragraph(&preview));
    row.append(&label(
        if note["pinned"] == true {
            "PINNED · shared with agents"
        } else {
            "Project note"
        },
        "dim",
    ));
    body.append(&row);
}

pub fn modules(ui: &Rc<Ui>, body: &gtk::Box, modules: &[Value]) {
    for module in modules {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("record");
        let name = label(text(module, "name"), "title");
        name.set_hexpand(true);
        row.append(&name);
        row.append(&label(
            &format!(
                "{} · {:.0}% done{}",
                text(module, "priority"),
                module["progress_pct"].as_f64().unwrap_or(0.),
                if module["completed_at"].is_string() {
                    " · archived"
                } else {
                    ""
                }
            ),
            "dim",
        ));
        let key = button("Open module", "quiet");
        let weak = Rc::downgrade(ui);
        let id = module["id"].as_i64().unwrap_or(0);
        key.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                open_module(&ui, id)
            }
        });
        row.append(&key);
        body.append(&row);
    }
}
pub fn module_composer(ui: &Rc<Ui>, page: &gtk::Box, project: i64) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Entry::builder()
        .placeholder_text("Release outcome")
        .hexpand(true)
        .build();
    let priority = choose(&["low", "medium", "high", "urgent"], "medium");
    let key = button("Create module", "primary");
    row.append(&name);
    row.append(&priority);
    row.append(&key);
    page.append(&row);
    let weak = Rc::downgrade(ui);
    key.connect_clicked(move |b| {
        let Some(ui) = weak.upgrade() else { return };
        let value = name.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        let priority = chosen(&priority);
        b.set_sensitive(false);
        let b = b.clone();
        let name = name.clone();
        glib::spawn_future_local(async move {
            match ui
                .call(
                    "module.create",
                    json!({"project_id":project,"name":value,"priority":priority}),
                )
                .await
            {
                Ok(m) => {
                    if name.text().trim() == value {
                        name.set_text("")
                    }
                    super::refresh_notes(&ui);
                    open_module(&ui, m["id"].as_i64().unwrap_or(0));
                }
                Err(e) => ui.show_error(&e.to_string()),
            }
            b.set_sensitive(true);
        });
    });
}
fn open_module(ui: &Rc<Ui>, id: i64) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match ui.call("module.get", json!({"module_id":id})).await {
            Ok(module) => module_detail(&ui, module),
            Err(e) => ui.show_error(&e.to_string()),
        }
    });
}
fn module_detail(ui: &Rc<Ui>, module: Value) {
    let id = module["id"].as_i64().unwrap_or(0);
    let project = module["project_id"].as_i64().unwrap_or(0);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let name = gtk::Entry::builder().text(text(&module, "name")).build();
    let priority = choose(
        &["low", "medium", "high", "urgent"],
        text(&module, "priority"),
    );
    let icon = gtk::Entry::builder()
        .text(text(&module, "icon"))
        .placeholder_text("Icon name")
        .build();
    field("Module name", &name, &form);
    field("Priority", &priority, &form);
    field("Icon", &icon, &form);
    let snapshot: Rc<dyn Fn() -> Value> = Rc::new(
        move || json!({"name":name.text().trim(),"priority":chosen(&priority),"icon":if icon.text().trim().is_empty(){Value::Null}else{json!(icon.text().trim())}}),
    );
    let d = Draft::new(ui, "Module", module.clone(), snapshot, form);
    d.controls(ui, "module.get", "module.update", "module_id", id);
    for column in super::task_pages::COLUMNS {
        d.form.append(&label(
            &column.replace('_', " ").to_uppercase(),
            "section-label",
        ));
        for task in rows(&module["tasks_by_state"], column) {
            let key = button(
                &format!("#{} {}", task["id"], text(&task, "title")),
                "quiet",
            );
            let weak = Rc::downgrade(ui);
            let task_id = task["id"].as_i64().unwrap_or(0);
            key.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    super::task_pages::open(&ui, task_id)
                }
            });
            d.form.append(&key);
        }
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Entry::builder()
        .placeholder_text("Add module task")
        .hexpand(true)
        .build();
    row.append(&title);
    super::task_pages::action(
        ui,
        &d,
        &row,
        "Add task",
        "task.create",
        move || json!({"project_id":project,"module_id":id,"title":title.text().trim(),"column":"backlog"}),
        None,
    );
    d.form.append(&row);
    let key = button("Draft patch notes", "quiet");
    d.form.append(&key);
    let preview = multiline("", 140);
    preview.set_editable(false);
    d.form.append(&preview);
    let copy = button("Copy patch notes", "quiet");
    let buffer = preview.buffer();
    copy.connect_clicked(move |b| b.clipboard().set_text(&buffer_text(&buffer)));
    d.form.append(&copy);
    let weak = Rc::downgrade(ui);
    let status = d.status.clone();
    key.connect_clicked(move |b| {
        let Some(ui) = weak.upgrade() else { return };
        let buffer = preview.buffer();
        let status = status.clone();
        b.set_sensitive(false);
        let b = b.clone();
        glib::spawn_future_local(async move {
            match ui
                .call("module.changelog.draft", json!({"module_id":id}))
                .await
            {
                Ok(v) => buffer.set_text(text(&v, "markdown")),
                Err(e) => status.set_text(&e.to_string()),
            }
            b.set_sensitive(true);
        });
    });
    let archived = module["completed_at"].is_string();
    super::task_pages::action(
        ui,
        &d,
        &d.footer,
        if archived {
            "Reopen"
        } else {
            "Complete module"
        },
        if archived {
            "module.reopen"
        } else {
            "module.complete"
        },
        move || json!({"module_id":id}),
        None,
    );
    super::task_pages::action(
        ui,
        &d,
        &d.footer,
        "Delete module",
        "module.delete",
        move || json!({"module_id":id}),
        None,
    );
    d.present();
}
