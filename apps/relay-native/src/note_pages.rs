//! The Notes window: a persistent shell (toolbar, library sidebar, tabbed editors) that
//! survives refreshes, built around one `doc::Doc` per open note.
#[path = "note_pages/doc.rs"]
mod doc;
#[path = "note_pages/glyphs.rs"]
mod glyphs;
#[path = "note_pages/menu.rs"]
mod menu;
#[path = "note_pages/text.rs"]
mod text;

use super::task_pages::{buffer_text, choose, chosen, multiline, Draft};
use super::*;
use doc::Doc;
use glyphs::glyph;
use std::cell::{Cell, OnceCell};
use std::collections::{BTreeMap, BTreeSet};

/// View and editing preferences, persisted with the window in `native.notes.window`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Prefs {
    pub autosave: bool,
    pub wrap: bool,
    pub line_numbers: bool,
    pub current_line: bool,
    pub toolbar: bool,
    pub monospace: bool,
    pub markdown: bool,
    pub zoom: u16,
    pub sort_title: bool,
}
impl Default for Prefs {
    fn default() -> Self {
        // Autosave is opt-in: D72 notes save explicitly unless the person turns it on.
        Self {
            autosave: false,
            wrap: true,
            line_numbers: true,
            current_line: true,
            toolbar: true,
            monospace: true,
            markdown: true,
            zoom: 100,
            sort_title: false,
        }
    }
}
impl Prefs {
    fn to_json(self) -> Value {
        json!({"autosave":self.autosave,"wrap":self.wrap,"line_numbers":self.line_numbers,
            "current_line":self.current_line,"toolbar":self.toolbar,"monospace":self.monospace,
            "markdown":self.markdown,"zoom":self.zoom,"sort":if self.sort_title {"title"} else {"modified"}})
    }
    fn from_json(value: &Value) -> Self {
        let d = Self::default();
        let flag = |key: &str, fallback: bool| value[key].as_bool().unwrap_or(fallback);
        Self {
            autosave: flag("autosave", d.autosave),
            wrap: flag("wrap", d.wrap),
            line_numbers: flag("line_numbers", d.line_numbers),
            current_line: flag("current_line", d.current_line),
            toolbar: flag("toolbar", d.toolbar),
            monospace: flag("monospace", d.monospace),
            markdown: flag("markdown", d.markdown),
            zoom: value["zoom"].as_u64().map_or(d.zoom, |z| z.clamp(50, 300) as u16),
            sort_title: value["sort"] == "title",
        }
    }
}

struct Row {
    row: gtk::ListBoxRow,
    note: Value,
    haystack: String,
    dot: gtk::Box,
}

struct Shell {
    root: gtk::Box,
    toolbar: gtk::Box,
    picker: gtk::Box,
    split: gtk::Paned,
    library: gtk::Box,
    count: gtk::Label,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    list_scroll: gtk::ScrolledWindow,
    placeholder: gtk::Label,
    row_menu: gtk::PopoverMenu,
    banner: gtk::Box,
    banner_text: gtk::Label,
    welcome: gtk::Box,
    rows: RefCell<Vec<Row>>,
    notes: RefCell<Vec<Value>>,
    project: Cell<i64>,
    rail_applied: Cell<bool>,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
    static DOCS: RefCell<BTreeMap<i64, Rc<Doc>>> = const { RefCell::new(BTreeMap::new()) };
    static PREFS: Cell<Prefs> = Cell::new(Prefs::default());
    static SESSION: RefCell<serde_json::Map<String, Value>> = RefCell::new(serde_json::Map::new());
    static RESTORED: RefCell<BTreeSet<i64>> = const { RefCell::new(BTreeSet::new()) };
    static LAST_QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static FONT: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static SCHEME: OnceCell<Option<sourceview5::StyleScheme>> = const { OnceCell::new() };
}

const NOTES_SCHEME: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<!-- Relay Notes: the Code editor's matte palette, with quieter Markdown. -->
<style-scheme id="relay-notes" name="Relay Notes" version="1.0" parent-scheme="Adwaita-dark">
  <style name="text" foreground="#dcdcda" background="#0c0c0e"/>
  <style name="selection" background="#3a3a3f"/>
  <style name="cursor" foreground="#ececea"/>
  <style name="current-line" background="#131316"/>
  <style name="line-numbers" foreground="#47474c" background="#0c0c0e"/>
  <style name="current-line-number" foreground="#a5a5a3" background="#0c0c0e"/>
  <style name="search-match" foreground="#0c0c0e" background="#e0b04a"/>
  <style name="def:heading" foreground="#f4f4f2" bold="true"/>
  <style name="def:emphasis" italic="true"/>
  <style name="def:strong-emphasis" bold="true"/>
  <style name="def:list-marker" foreground="#e0b04a" bold="true"/>
  <style name="def:inline-code" foreground="#d9b46a" background="#17171a"/>
  <style name="def:preformatted-section" foreground="#c9c4b4"/>
  <style name="def:link-text" foreground="#7aa7f0" underline="single"/>
  <style name="def:link-destination" foreground="#6e6e72"/>
  <style name="def:link-symbol" foreground="#6e6e72"/>
  <style name="def:shebang" foreground="#a5a5a3" italic="true"/>
  <style name="def:thematic-break" foreground="#5a5a60"/>
  <style name="def:special-char" foreground="#77777a"/>
  <style name="def:comment" foreground="#6e6e70"/>
  <style name="def:string" foreground="#e0b04a"/>
  <style name="def:keyword" foreground="#c979d6"/>
</style-scheme>
"##;

pub(super) fn scheme() -> Option<sourceview5::StyleScheme> {
    SCHEME.with(|cell| {
        cell.get_or_init(|| {
            let manager = sourceview5::StyleSchemeManager::default();
            let directory = glib::user_cache_dir().join("relay-v4/notes-styles");
            let path = directory.join("relay-notes.xml");
            let written = std::fs::create_dir_all(&directory).and_then(|_| {
                if std::fs::read_to_string(&path).ok().as_deref() == Some(NOTES_SCHEME) {
                    Ok(())
                } else {
                    std::fs::write(&path, NOTES_SCHEME)
                }
            });
            match written {
                Ok(()) => manager.append_search_path(&directory.to_string_lossy()),
                Err(error) => tracing::warn!(%error, "Could not install the Notes colour scheme"),
            }
            manager
                .scheme("relay-notes")
                .or_else(|| manager.scheme("relay-matte"))
                .or_else(|| manager.scheme("Adwaita-dark"))
        })
        .clone()
    })
}

pub(super) fn prefs() -> Prefs {
    PREFS.with(Cell::get)
}

fn docs() -> Vec<Rc<Doc>> {
    DOCS.with(|docs| docs.borrow().values().cloned().collect())
}

fn doc_by_id(id: i64) -> Option<Rc<Doc>> {
    DOCS.with(|docs| docs.borrow().get(&id).cloned())
}

fn owner(ui: &Rc<Ui>) -> Option<Rc<NotesWindow>> {
    ui.notes_window.borrow().clone()
}

pub(super) fn window_of(ui: &Rc<Ui>) -> Option<gtk::Window> {
    owner(ui).map(|owner| owner.window.clone())
}

fn persist(ui: &Rc<Ui>) {
    if let Some(owner) = owner(ui) {
        owner.persist(ui);
    }
}

pub(super) fn last_query() -> String {
    LAST_QUERY.with(|q| q.borrow().clone())
}

pub(super) fn remember_query(value: &str) {
    LAST_QUERY.with(|q| value.clone_into(&mut q.borrow_mut()));
}

pub(super) fn project_name(ui: &Rc<Ui>, project: i64) -> String {
    ui.projects
        .borrow()
        .iter()
        .find(|p| p["id"].as_i64() == Some(project))
        .map(|p| text(p, "name").to_string())
        .unwrap_or_else(|| format!("Project {project}"))
}

/// "14:02" today, "Oct 5" this year, "Oct 5, 2025" before.
pub(super) fn fmt_date(iso: &str) -> String {
    let Some(date) = glib::DateTime::from_iso8601(iso, None).ok().and_then(|d| d.to_local().ok()) else {
        return String::new();
    };
    let now = glib::DateTime::now_local().ok();
    let format = match &now {
        Some(now) if now.year() == date.year() && now.day_of_year() == date.day_of_year() => "%H:%M",
        Some(now) if now.year() == date.year() => "%b %-d",
        _ => "%b %-d, %Y",
    };
    date.format(format).map(|s| s.to_string()).unwrap_or_default()
}

fn apply_font(prefs: &Prefs) {
    let css = format!(
        "textview.notes-source {{ font-family: {}; font-size: {:.1}px; }}",
        if prefs.monospace { "'Fira Mono', monospace" } else { "'Fira Sans', sans-serif" },
        13.0 * f64::from(prefs.zoom) / 100.0
    );
    FONT.with(|font| {
        let mut font = font.borrow_mut();
        if font.is_none() {
            if let Some(display) = gtk::gdk::Display::default() {
                let provider = gtk::CssProvider::new();
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                );
                *font = Some(provider);
            }
        }
        if let Some(provider) = font.as_ref() {
            provider.load_from_string(&css);
        }
    });
}

fn apply_prefs(ui: &Rc<Ui>, prefs: &Prefs, rows: bool) {
    apply_font(prefs);
    for doc in docs() {
        doc.apply_prefs(prefs);
    }
    if let Some(shell) = shell_if_built() {
        shell.toolbar.set_visible(prefs.toolbar);
        if rows {
            render_rows(ui, &shell);
        }
    }
}

pub(super) fn set_prefs(ui: &Rc<Ui>, next: Prefs) {
    let before = prefs();
    if before == next {
        return;
    }
    PREFS.with(|p| p.set(next));
    apply_prefs(ui, &next, before.sort_title != next.sort_title);
    if next.autosave && !before.autosave {
        for doc in docs() {
            doc.schedule_autosave(ui);
        }
    }
    persist(ui);
}

pub(super) fn zoom(ui: &Rc<Ui>, direction: i32) {
    let mut next = prefs();
    next.zoom = if direction == 0 { 100 } else { text::zoom_step(next.zoom, direction) };
    set_prefs(ui, next);
}

/// What the window persists besides its rail: preferences and open tabs per project.
pub(super) fn saved_state() -> Value {
    json!({"prefs": prefs().to_json(), "session": SESSION.with(|s| Value::Object(s.borrow().clone()))})
}

/// Settings arrived: adopt preferences and the remembered tabs, before the first render.
pub(super) fn restore_state(ui: &Rc<Ui>, value: &Value) {
    let restored = Prefs::from_json(&value["prefs"]);
    PREFS.with(|p| p.set(restored));
    if let Some(session) = value["session"].as_object() {
        SESSION.with(|s| *s.borrow_mut() = session.clone());
    }
    let sidebar = owner(ui).is_none_or(|o| !o.rail_collapsed.get());
    menu::sync_prefs(&restored, sidebar);
    apply_prefs(ui, &restored, true);
}

/// The window is hiding: with autosave on, nothing waits for the timer.
pub(super) fn flush(ui: &Rc<Ui>) {
    if !prefs().autosave {
        return;
    }
    for doc in docs() {
        if doc.dirty() && !doc.deleted.get() && !doc.conflict.get() {
            doc::save(ui, &doc, None);
        }
    }
}

/// Install the actions, shortcuts and menu bar on the Notes window.
pub(super) fn window_menu(ui: &Rc<Ui>, window: &gtk::Window) -> gtk::Widget {
    menu::install(ui, window).upcast()
}

fn shell_if_built() -> Option<Rc<Shell>> {
    SHELL.with(|s| s.borrow().clone())
}

fn current_project(ui: &Rc<Ui>) -> i64 {
    let shown = shell_if_built().map_or(0, |s| s.project.get());
    if shown != 0 {
        shown
    } else {
        owner(ui).map_or(ui.project.get(), |o| o.project.get())
    }
}

fn tool(icon: &str, tip: &str, action: &str) -> gtk::Button {
    let key = gtk::Button::new();
    key.set_child(Some(&glyph(icon, 16)));
    key.add_css_class("notes-tool");
    key.set_focus_on_click(false);
    key.set_tooltip_text(Some(tip));
    key.update_property(&[gtk::accessible::Property::Label(tip)]);
    key.set_action_name(Some(&format!("notes.{action}")));
    key
}

fn text_tool(caption: &str, class: &str, tip: &str, action: &str) -> gtk::Button {
    let key = gtk::Button::with_label(caption);
    key.add_css_class("notes-tool");
    key.add_css_class("notes-tool-text");
    key.add_css_class(class);
    key.set_focus_on_click(false);
    key.set_tooltip_text(Some(tip));
    key.set_action_name(Some(&format!("notes.{action}")));
    key
}

fn tool_separator() -> gtk::Separator {
    let line = gtk::Separator::new(gtk::Orientation::Vertical);
    line.add_css_class("notes-tool-sep");
    line
}

fn shell(ui: &Rc<Ui>) -> Rc<Shell> {
    if let Some(shell) = shell_if_built() {
        return shell;
    }
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("notes-shell");
    root.set_vexpand(true);

    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 1);
    toolbar.add_css_class("notes-toolbar");
    let collapse = gtk::Button::new();
    collapse.set_child(Some(&glyph("sidebar", 16)));
    collapse.add_css_class("notes-tool");
    collapse.set_focus_on_click(false);
    collapse.set_widget_name("notes-library-toggle");
    collapse.set_tooltip_text(Some("Show or hide the library (F9)"));
    collapse.update_property(&[gtk::accessible::Property::Label("Toggle library")]);
    toolbar.append(&collapse);
    toolbar.append(&tool_separator());
    toolbar.append(&tool("note-new", "New note (Ctrl+N)", "new"));
    toolbar.append(&tool("save", "Save (Ctrl+S)", "save"));
    toolbar.append(&tool_separator());
    toolbar.append(&tool("undo", "Undo (Ctrl+Z)", "undo"));
    toolbar.append(&tool("redo", "Redo (Ctrl+Shift+Z)", "redo"));
    toolbar.append(&tool_separator());
    toolbar.append(&tool("cut", "Cut (Ctrl+X)", "cut"));
    toolbar.append(&tool("copy", "Copy (Ctrl+C)", "copy"));
    toolbar.append(&tool("paste", "Paste (Ctrl+V)", "paste"));
    toolbar.append(&tool_separator());
    toolbar.append(&tool("search", "Find (Ctrl+F)", "find"));
    toolbar.append(&tool("replace", "Replace (Ctrl+H)", "replace"));
    toolbar.append(&tool_separator());
    toolbar.append(&text_tool("B", "notes-tool-bold", "Bold (Ctrl+B)", "bold"));
    toolbar.append(&text_tool("I", "notes-tool-italic", "Italic (Ctrl+I)", "italic"));
    let headings = gtk::MenuButton::new();
    headings.set_label("H");
    headings.set_always_show_arrow(false);
    headings.add_css_class("notes-tool-menu");
    headings.set_tooltip_text(Some("Heading"));
    headings.set_menu_model(Some(&menu::headings_menu()));
    toolbar.append(&headings);
    toolbar.append(&tool("list", "Bulleted list", "bullets"));
    toolbar.append(&tool("list-numbered", "Numbered list", "numbers"));
    toolbar.append(&tool("checklist", "Checklist", "checklist"));
    toolbar.append(&tool("quote", "Quote", "quote"));
    toolbar.append(&tool("code-inline", "Inline code", "code"));
    toolbar.append(&tool("code-block", "Code block", "code-block"));
    toolbar.append(&tool("link", "Link (Ctrl+K)", "link"));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    toolbar.append(&spacer);
    let picker = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    picker.add_css_class("notes-picker");
    toolbar.append(&picker);
    root.append(&toolbar);

    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_widget_name("notes-library-split");
    split.add_css_class("notes-split");
    split.set_vexpand(true);
    split.set_resize_start_child(false);
    split.set_shrink_start_child(false);
    split.set_shrink_end_child(false);
    split.set_position(owner(ui).map_or(270, |o| o.rail_width.get()));

    let library = gtk::Box::new(gtk::Orientation::Vertical, 0);
    library.add_css_class("notes-library");
    library.set_size_request(180, -1);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.add_css_class("notes-library-head");
    let heading = label("NOTES", "notes-library-title");
    head.append(&heading);
    let count = label("", "notes-library-count");
    count.set_hexpand(true);
    head.append(&count);
    let sort = gtk::MenuButton::new();
    sort.set_child(Some(&glyph("sort", 14)));
    sort.add_css_class("notes-library-key");
    sort.set_tooltip_text(Some("Sort notes"));
    sort.set_menu_model(Some(&menu::sort_menu()));
    head.append(&sort);
    let add = tool("plus", "New note (Ctrl+N)", "new");
    add.add_css_class("notes-library-key");
    head.append(&add);
    library.append(&head);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search notes"));
    search.add_css_class("notes-library-search");
    library.append(&search);
    let list = gtk::ListBox::new();
    list.add_css_class("notes-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    let placeholder = label("", "notes-list-empty");
    placeholder.set_wrap(true);
    placeholder.set_justify(gtk::Justification::Center);
    placeholder.set_xalign(0.5);
    list.set_placeholder(Some(&placeholder));
    let list_scroll = crate::app::scrolled(&list);
    list_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    library.append(&list_scroll);
    split.set_start_child(Some(&library));

    let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
    host.add_css_class("notes-host");
    host.set_vexpand(true);
    host.set_hexpand(true);
    let banner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    banner.add_css_class("notes-notice");
    banner.set_visible(false);
    let banner_text = label("", "notes-notice-text");
    banner_text.set_wrap(true);
    banner_text.set_hexpand(true);
    banner.append(&banner_text);
    let dismiss = gtk::Button::new();
    dismiss.set_child(Some(&glyph("close", 12)));
    dismiss.add_css_class("notes-notice-close");
    let hide = banner.downgrade();
    dismiss.connect_clicked(move |_| {
        if let Some(banner) = hide.upgrade() {
            banner.set_visible(false)
        }
    });
    banner.append(&dismiss);
    host.append(&banner);
    let tabs = &ui.note_tabs;
    if let Some(parent) = tabs.parent() {
        if let Ok(parent) = parent.downcast::<gtk::Box>() {
            parent.remove(tabs);
        }
    }
    tabs.add_css_class("notes-tabs");
    tabs.set_scrollable(true);
    tabs.set_show_border(false);
    tabs.set_show_tabs(true);
    tabs.set_vexpand(true);
    tabs.set_visible(false);
    host.append(tabs);
    let welcome = gtk::Box::new(gtk::Orientation::Vertical, 10);
    welcome.add_css_class("notes-welcome");
    welcome.set_valign(gtk::Align::Center);
    welcome.set_halign(gtk::Align::Center);
    welcome.set_vexpand(true);
    let mark = glyph("notes", 36);
    mark.add_css_class("notes-welcome-mark");
    welcome.append(&mark);
    let title = label("No note open", "notes-welcome-title");
    title.set_xalign(0.5);
    welcome.append(&title);
    let hint = label("Pick a note from the library, or start a new one.", "notes-welcome-text");
    hint.set_xalign(0.5);
    welcome.append(&hint);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::Center);
    let create = button("New note", "primary");
    create.set_action_name(Some("notes.new"));
    let find = button("Search the library", "");
    find.set_action_name(Some("notes.open"));
    actions.append(&create);
    actions.append(&find);
    welcome.append(&actions);
    let keys = label("Ctrl+N new note  ·  Ctrl+O search  ·  F9 library", "notes-welcome-keys");
    keys.set_xalign(0.5);
    welcome.append(&keys);
    host.append(&welcome);
    split.set_end_child(Some(&host));
    root.append(&split);

    let row_menu = gtk::PopoverMenu::from_model(None::<&gtk::gio::MenuModel>);
    row_menu.set_parent(&list);
    row_menu.set_has_arrow(false);
    row_menu.set_halign(gtk::Align::Start);
    let menu_owner = row_menu.downgrade();
    list.connect_destroy(move |_| {
        if let Some(menu) = menu_owner.upgrade() {
            menu.unparent();
        }
    });

    let shell = Rc::new(Shell {
        root,
        toolbar,
        picker,
        split,
        library,
        count,
        search,
        list,
        list_scroll,
        placeholder,
        row_menu,
        banner,
        banner_text,
        welcome,
        rows: RefCell::new(Vec::new()),
        notes: RefCell::new(Vec::new()),
        project: Cell::new(0),
        rail_applied: Cell::new(false),
    });
    SHELL.with(|s| *s.borrow_mut() = Some(shell.clone()));
    let weak_ui = Rc::downgrade(ui);
    let weak = Rc::downgrade(&shell);

    let target = weak_ui.clone();
    collapse.connect_clicked(move |_| {
        if let Some(ui) = target.upgrade() {
            let visible = shell_if_built().is_some_and(|s| s.library.is_visible());
            set_sidebar(&ui, !visible);
        }
    });
    let target = weak_ui.clone();
    shell.split.connect_position_notify(move |split| {
        if let Some(ui) = target.upgrade() {
            if let Some(owner) = owner(&ui) {
                if !owner.rail_collapsed.get() {
                    owner.rail_width.set(split.position().max(180));
                }
                owner.persist(&ui);
            }
        }
    });
    let filter = weak.clone();
    shell.list.set_filter_func(move |row| {
        let Some(shell) = filter.upgrade() else { return true };
        let query = shell.search.text().trim().to_lowercase();
        query.is_empty()
            || shell
                .rows
                .borrow()
                .get(row.index() as usize)
                .is_some_and(|r| r.haystack.contains(&query))
    });
    let headers = weak.clone();
    shell.list.set_header_func(move |row, before| {
        let Some(shell) = headers.upgrade() else { return };
        let rows = shell.rows.borrow();
        let pinned = |row: &gtk::ListBoxRow| {
            rows.get(row.index() as usize).is_some_and(|r| r.note["pinned"] == true)
        };
        let this = pinned(row);
        let show = rows.iter().any(|r| r.note["pinned"] == true)
            && before.is_none_or(|before| pinned(before) != this);
        if show {
            row.set_header(Some(&label(if this { "PINNED" } else { "NOTES" }, "notes-list-header")));
        } else {
            row.set_header(gtk::Widget::NONE);
        }
    });
    let target = weak_ui.clone();
    let rows = weak.clone();
    shell.list.connect_row_activated(move |_, row| {
        let (Some(ui), Some(shell)) = (target.upgrade(), rows.upgrade()) else { return };
        let note = shell.rows.borrow().get(row.index() as usize).map(|r| r.note.clone());
        if let Some(note) = note {
            edit(&ui, note);
            if let Some(doc) = current_doc(&ui) {
                doc.view.grab_focus();
            }
        }
    });
    let context = gtk::GestureClick::new();
    context.set_button(3);
    let menu_shell = weak.clone();
    context.connect_pressed(move |gesture, _, x, y| {
        let Some(shell) = menu_shell.upgrade() else { return };
        let Some(row) = shell.list.row_at_y(y as i32) else { return };
        let target = shell
            .rows
            .borrow()
            .get(row.index() as usize)
            .map(|r| (r.note["id"].as_i64().unwrap_or(0), r.note["pinned"] == true));
        if let Some((id, pinned)) = target {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            shell.row_menu.set_menu_model(Some(&menu::row_menu(id, pinned)));
            shell
                .row_menu
                .set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            shell.row_menu.popup();
        }
    });
    shell.list.add_controller(context);
    let searching = weak.clone();
    shell.search.connect_search_changed(move |_| {
        if let Some(shell) = searching.upgrade() {
            shell.list.invalidate_filter();
            shell.list.invalidate_headers();
            update_placeholder(&shell);
        }
    });
    let target = weak_ui.clone();
    let first = weak.clone();
    shell.search.connect_activate(move |_| {
        let (Some(ui), Some(shell)) = (target.upgrade(), first.upgrade()) else { return };
        if let Some(row) = first_visible(&shell) {
            shell.list.select_row(Some(&row));
            row.activate();
        }
        if let Some(doc) = current_doc(&ui) {
            doc.view.grab_focus();
        }
    });
    let target = weak_ui.clone();
    shell.search.connect_stop_search(move |search| {
        search.set_text("");
        if let Some(doc) = target.upgrade().and_then(|ui| current_doc(&ui)) {
            doc.view.grab_focus();
        }
    });
    let down = gtk::EventControllerKey::new();
    let first = weak.clone();
    down.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Down {
            if let Some(row) = first.upgrade().and_then(|shell| first_visible(&shell)) {
                row.grab_focus();
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    shell.search.add_controller(down);
    let target = weak_ui.clone();
    ui.note_tabs.connect_page_notify(move |_| {
        if let Some(ui) = target.upgrade() {
            tabs_changed(&ui);
        }
    });
    let target = weak_ui.clone();
    ui.note_tabs.connect_page_reordered(move |_, _, _| {
        if let Some(ui) = target.upgrade() {
            record_session(&ui);
        }
    });
    let prefs = prefs();
    apply_font(&prefs);
    shell.toolbar.set_visible(prefs.toolbar);
    update_placeholder(&shell);
    shell
}

fn first_visible(shell: &Shell) -> Option<gtk::ListBoxRow> {
    let query = shell.search.text().trim().to_lowercase();
    shell
        .rows
        .borrow()
        .iter()
        .find(|r| query.is_empty() || r.haystack.contains(&query))
        .map(|r| r.row.clone())
}

fn update_placeholder(shell: &Shell) {
    shell.placeholder.set_text(if shell.rows.borrow().is_empty() {
        "No notes in this project yet.\nPress Ctrl+N to start one."
    } else {
        "No notes match your search."
    });
}

fn mount(ui: &Rc<Ui>, shell: &Shell) {
    let page = &ui.pages["notes"];
    page.add_css_class("notes-page");
    page.set_spacing(0);
    if shell.root.parent().as_ref() != Some(page.upcast_ref::<gtk::Widget>()) {
        clear(page);
        page.append(&shell.root);
    }
}

fn shell_error(ui: &Rc<Ui>, message: &str) {
    let shell = shell(ui);
    shell.banner_text.set_text(message);
    shell.banner.set_visible(true);
}

fn build_row(note: &Value) -> Row {
    let title = text(note, "title");
    let body = text(note, "body");
    let id = note["id"].as_i64().unwrap_or(0);
    let row = gtk::ListBoxRow::new();
    row.add_css_class("notes-row");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 3);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("notes-dirty-dot");
    dot.set_valign(gtk::Align::Center);
    dot.set_visible(doc_by_id(id).is_some_and(|d| d.dirty()));
    dot.set_tooltip_text(Some("Unsaved changes"));
    top.append(&dot);
    let name = text::display_title(title, body);
    let heading = label(&name, "notes-row-title");
    heading.set_hexpand(true);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&heading);
    if note["pinned"] == true {
        let pin = glyph("pin", 12);
        pin.add_css_class("notes-row-pin");
        pin.set_tooltip_text(Some("Pinned: shared with agents"));
        top.append(&pin);
    }
    content.append(&top);
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    bottom.append(&label(&fmt_date(text(note, "updated_at")), "notes-row-date"));
    let preview = text::preview(title, body);
    let preview = label(if preview.is_empty() { "No additional text" } else { &preview }, "notes-row-preview");
    preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
    preview.set_hexpand(true);
    bottom.append(&preview);
    content.append(&bottom);
    row.set_child(Some(&content));
    row.update_property(&[gtk::accessible::Property::Label(&name)]);
    Row { row, note: note.clone(), haystack: format!("{title}\n{body}").to_lowercase(), dot }
}

fn render_rows(ui: &Rc<Ui>, shell: &Shell) {
    let adjustment = shell.list_scroll.vadjustment();
    let keep = adjustment.value();
    // Rows only: `remove_all` would also try the row menu parented to the list and, in GTK
    // 4.22, retry it forever ("Tried to remove non-child").
    while let Some(row) = shell.list.row_at_index(0) {
        shell.list.remove(&row);
    }
    let mut notes = shell.notes.borrow().clone();
    if prefs().sort_title {
        notes.sort_by_cached_key(|n| {
            (n["pinned"] != true, text::display_title(text(n, "title"), text(n, "body")).to_lowercase())
        });
    }
    let rows: Vec<Row> = notes.iter().map(build_row).collect();
    let widgets: Vec<gtk::ListBoxRow> = rows.iter().map(|r| r.row.clone()).collect();
    *shell.rows.borrow_mut() = rows;
    for row in &widgets {
        shell.list.append(row);
    }
    shell.count.set_text(&notes.len().to_string());
    update_placeholder(shell);
    shell.list.invalidate_filter();
    shell.list.invalidate_headers();
    select_active_row(ui, shell);
    glib::idle_add_local_once(move || adjustment.set_value(keep));
}

fn select_active_row(ui: &Rc<Ui>, shell: &Shell) {
    let active = current_doc(ui).map(|d| d.id);
    let row = shell
        .rows
        .borrow()
        .iter()
        .find(|r| active.is_some() && r.note["id"].as_i64() == active)
        .map(|r| r.row.clone());
    match row {
        Some(row) => shell.list.select_row(Some(&row)),
        None => shell.list.unselect_all(),
    }
}

/// The note in the visible tab, if any.
pub(super) fn current_doc(ui: &Rc<Ui>) -> Option<Rc<Doc>> {
    let tabs = &ui.note_tabs;
    let widget = tabs.nth_page(tabs.current_page())?;
    if !widget.is_visible() {
        return None;
    }
    docs().into_iter().find(|d| d.draft.layout.upcast_ref::<gtk::Widget>() == &widget)
}

pub(super) fn update_chrome(ui: &Rc<Ui>) {
    let Some(owner) = owner(ui) else { return };
    let (heading, title) = match current_doc(ui) {
        Some(doc) => {
            let name = doc.name();
            if doc.dirty() {
                (format!("{name}  •"), format!("{name} • — Notes · Relay"))
            } else {
                (name.clone(), format!("{name} — Notes · Relay"))
            }
        }
        None => ("Notes".to_string(), "Notes · Relay".to_string()),
    };
    if owner.heading.text() != heading {
        owner.heading.set_text(&heading);
    }
    if owner.window.title().as_deref() != Some(title.as_str()) {
        owner.window.set_title(Some(&title));
    }
}

pub(super) fn dirty_changed(ui: &Rc<Ui>, id: i64, dirty: bool) {
    if let Some(shell) = shell_if_built() {
        for row in shell.rows.borrow().iter() {
            if row.note["id"].as_i64() == Some(id) {
                row.dot.set_visible(dirty);
            }
        }
    }
    menu::sync(current_doc(ui).as_ref());
}

fn update_welcome(ui: &Rc<Ui>) {
    let Some(shell) = shell_if_built() else { return };
    let project = current_project(ui);
    let open = docs().iter().any(|d| {
        (project == 0 || d.project == project) && ui.note_tabs.page_num(&d.draft.layout).is_some()
    });
    ui.note_tabs.set_visible(open);
    shell.welcome.set_visible(!open);
}

fn tabs_changed(ui: &Rc<Ui>) {
    update_welcome(ui);
    if let Some(shell) = shell_if_built() {
        select_active_row(ui, &shell);
    }
    let current = current_doc(ui);
    menu::sync(current.as_ref());
    update_chrome(ui);
    record_session(ui);
}

/// Remember the open tabs and the active one for the project on screen.
fn record_session(ui: &Rc<Ui>) {
    let project = shell_if_built().map_or(0, |s| s.project.get());
    if project == 0 || !RESTORED.with(|r| r.borrow().contains(&project)) {
        return;
    }
    let all = docs();
    let tabs = &ui.note_tabs;
    let open: Vec<i64> = (0..tabs.n_pages())
        .filter_map(|i| tabs.nth_page(Some(i)))
        .filter_map(|w| {
            all.iter()
                .find(|d| d.project == project && d.draft.layout.upcast_ref::<gtk::Widget>() == &w)
                .map(|d| d.id)
        })
        .collect();
    let active = current_doc(ui).filter(|d| d.project == project).map(|d| d.id);
    let entry = json!({"open":open,"active":active});
    let changed = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let changed = s.get(&project.to_string()) != Some(&entry);
        s.insert(project.to_string(), entry);
        changed
    });
    if changed {
        persist(ui);
    }
}

/// Build a tab for `note` (at `position`, else last) and show it.
pub(super) fn open_doc(ui: &Rc<Ui>, note: Value, position: Option<u32>) -> Rc<Doc> {
    let shell = shell(ui);
    mount(ui, &shell);
    let doc = doc::build(ui, &note);
    let id = doc.id;
    let tabs = &ui.note_tabs;
    let page = match position {
        Some(position) => tabs.insert_page(&doc.draft.layout, Some(&doc.tab), Some(position)),
        None => tabs.append_page(&doc.draft.layout, Some(&doc.tab)),
    };
    tabs.set_tab_reorderable(&doc.draft.layout, true);
    let (weak_ui, weak) = (Rc::downgrade(ui), Rc::downgrade(&doc));
    *doc.draft.on_close.borrow_mut() = Some(Box::new(move || {
        let Some(ui) = weak_ui.upgrade() else { return };
        if let Some(doc) = weak.upgrade() {
            doc.stop_timers();
            if let Some(page) = ui.note_tabs.page_num(&doc.draft.layout) {
                ui.note_tabs.remove_page(Some(page));
            }
        }
        DOCS.with(|docs| docs.borrow_mut().remove(&id));
        ui.note_drafts.borrow_mut().remove(&id);
        if let Some(shell) = shell_if_built() {
            for row in shell.rows.borrow().iter() {
                if row.note["id"].as_i64() == Some(id) {
                    row.dot.set_visible(false);
                }
            }
        }
        tabs_changed(&ui);
    }));
    DOCS.with(|docs| docs.borrow_mut().insert(id, doc.clone()));
    ui.note_drafts.borrow_mut().insert(id, doc.draft.clone());
    tabs.set_current_page(Some(page));
    tabs_changed(ui);
    doc
}

pub fn edit(ui: &Rc<Ui>, note: Value) {
    let shell = shell(ui);
    mount(ui, &shell);
    let id = note["id"].as_i64().unwrap_or(0);
    if let Some(doc) = doc_by_id(id) {
        doc.draft.layout.set_visible(true);
        if let Some(page) = ui.note_tabs.page_num(&doc.draft.layout) {
            ui.note_tabs.set_current_page(Some(page));
        }
        tabs_changed(ui);
        return;
    }
    open_doc(ui, note, None);
}

pub(super) fn new_note(ui: &Rc<Ui>, title: Option<String>, body: String) {
    let project = current_project(ui);
    if project == 0 {
        return;
    }
    let mut payload = json!({"project_id":project,"body":body,"pinned":false});
    if let Some(title) = title {
        payload["title"] = json!(title);
    }
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match ui.call("notes.create", payload).await {
            Ok(note) => {
                open_doc(&ui, note, None).view.grab_focus();
                refresh_notes(&ui);
            }
            Err(error) => shell_error(&ui, &format!("Could not create a note: {error}")),
        }
    });
}

pub(super) fn set_sidebar(ui: &Rc<Ui>, visible: bool) {
    let shell = shell(ui);
    if let Some(owner) = owner(ui) {
        owner.rail_collapsed.set(!visible);
        owner.persist(ui);
    }
    shell.library.set_visible(visible);
    if let Some(action) = menu::lookup("sidebar") {
        action.set_state(&visible.to_variant());
    }
}

pub(super) fn focus_library(ui: &Rc<Ui>) {
    let shell = shell(ui);
    if !shell.library.is_visible() {
        set_sidebar(ui, true);
    }
    shell.search.grab_focus();
    shell.search.select_region(0, -1);
}

pub(super) fn toggle_pin(doc: &Rc<Doc>) {
    doc.pin.set_active(!doc.pin.is_active());
}

pub(super) fn cycle_tab(ui: &Rc<Ui>, direction: i32) {
    let tabs = &ui.note_tabs;
    let visible: Vec<u32> = (0..tabs.n_pages())
        .filter(|i| tabs.nth_page(Some(*i)).is_some_and(|w| w.is_visible()))
        .collect();
    if visible.len() < 2 {
        return;
    }
    let at = visible.iter().position(|i| Some(*i) == tabs.current_page()).unwrap_or(0) as i32;
    let next = (at + direction).rem_euclid(visible.len() as i32) as usize;
    tabs.set_current_page(Some(visible[next]));
}

fn listed(id: i64) -> Option<Value> {
    shell_if_built()?
        .notes
        .borrow()
        .iter()
        .find(|n| n["id"].as_i64() == Some(id))
        .cloned()
}

pub(super) fn open_id(ui: &Rc<Ui>, id: i64, rename: bool) {
    let Some(note) = doc_by_id(id).map(|d| d.draft.base.borrow().clone()).or_else(|| listed(id)) else {
        return;
    };
    edit(ui, note);
    if let Some(doc) = doc_by_id(id) {
        if rename {
            doc.title.grab_focus();
            doc.title.select_region(0, -1);
        } else {
            doc.view.grab_focus();
        }
    }
}

pub(super) fn pin_id(ui: &Rc<Ui>, id: i64) {
    if let Some(doc) = doc_by_id(id) {
        toggle_pin(&doc);
        return;
    }
    let Some(note) = listed(id) else { return };
    let pinned = note["pinned"] != true;
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        match ui.call("notes.pin", json!({"note_id":id,"pinned":pinned})).await {
            Ok(_) => refresh_notes(&ui),
            Err(error) => shell_error(&ui, &error.to_string()),
        }
    });
}

pub(super) fn duplicate_id(ui: &Rc<Ui>, id: i64) {
    let (title, body) = match doc_by_id(id) {
        Some(doc) => (doc.name(), doc.body()),
        None => match listed(id) {
            Some(note) => (text::display_title(text(&note, "title"), text(&note, "body")), text(&note, "body").to_string()),
            None => return,
        },
    };
    new_note(ui, Some(format!("{title} copy")), body);
}

pub(super) fn delete_id(ui: &Rc<Ui>, id: i64) {
    let name = match doc_by_id(id) {
        Some(doc) => doc.name(),
        None => listed(id).map_or_else(|| "this note".into(), |n| text::display_title(text(&n, "title"), text(&n, "body"))),
    };
    confirm_delete(ui, id, name);
}

pub(super) fn confirm_delete(ui: &Rc<Ui>, id: i64, name: String) {
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let dialog = gtk::AlertDialog::builder()
            .message(format!("Delete “{name}”?"))
            .detail("It leaves this project's library, and agents stop reading it.")
            .buttons(["Cancel", "Delete"])
            .cancel_button(0)
            .default_button(0)
            .modal(true)
            .build();
        if dialog.choose_future(window_of(&ui).as_ref()).await.ok() != Some(1) {
            return;
        }
        match ui.call("notes.delete", json!({"note_id":id})).await {
            Ok(_) => {
                if let Some(doc) = doc_by_id(id) {
                    doc::discard_close(&ui, &doc);
                }
                refresh_notes(&ui);
            }
            Err(error) => match doc_by_id(id) {
                Some(doc) => doc.show_notice(&ui, &format!("Not deleted: {error}"), Vec::new()),
                None => shell_error(&ui, &format!("Not deleted: {error}")),
            },
        }
    });
}

/// Render the library for `project`, reconcile open notes with the stored copies, and
/// restore the project's remembered tabs the first time it is shown. The shell itself is
/// built once, so the search text, scroll position and editors survive every refresh.
pub fn workspace(ui: &Rc<Ui>, _name: &str, project: i64, notes: &[Value]) {
    let shell = shell(ui);
    mount(ui, &shell);
    // Read the remembered tabs before hiding other projects' tabs records anything.
    let saved = SESSION.with(|s| s.borrow().get(&project.to_string()).cloned());
    let changed = shell.project.replace(project) != project;
    let owner = owner(ui);
    if let Some(owner) = &owner {
        owner.rendered_project.set(project);
        if !shell.rail_applied.replace(true) {
            shell.split.set_position(owner.rail_width.get());
            shell.library.set_visible(!owner.rail_collapsed.get());
            if let Some(action) = menu::lookup("sidebar") {
                action.set_state(&(!owner.rail_collapsed.get()).to_variant());
            }
        }
    }
    clear(&shell.picker);
    shell.picker.append(&super::workspace_picker(ui, project, "notes"));
    *shell.notes.borrow_mut() = notes.to_vec();
    for doc in docs() {
        let mine = doc.project == project;
        doc.draft.layout.set_visible(mine);
        if mine {
            doc::reconcile(ui, &doc, notes.iter().find(|n| n["id"].as_i64() == Some(doc.id)));
        }
    }
    render_rows(ui, &shell);
    let first_time = RESTORED.with(|r| r.borrow_mut().insert(project));
    if first_time {
        match &saved {
            Some(entry) => {
                for id in rows(entry, "open").iter().filter_map(Value::as_i64) {
                    if doc_by_id(id).is_none() {
                        if let Some(note) = notes.iter().find(|n| n["id"].as_i64() == Some(id)) {
                            open_doc(ui, note.clone(), None);
                        }
                    }
                }
            }
            None => {
                if !docs().iter().any(|d| d.project == project) {
                    if let Some(note) = notes.first() {
                        open_doc(ui, note.clone(), None);
                    }
                }
            }
        }
    }
    if first_time || changed {
        let wanted = saved
            .as_ref()
            .and_then(|s| s["active"].as_i64())
            .and_then(doc_by_id)
            .filter(|d| d.project == project)
            .or_else(|| {
                (0..ui.note_tabs.n_pages())
                    .filter_map(|i| ui.note_tabs.nth_page(Some(i)))
                    .find(|w| w.is_visible())
                    .and_then(|w| docs().into_iter().find(|d| d.draft.layout.upcast_ref::<gtk::Widget>() == &w))
            });
        if let Some(doc) = wanted {
            if let Some(page) = ui.note_tabs.page_num(&doc.draft.layout) {
                ui.note_tabs.set_current_page(Some(page));
            }
        }
    }
    tabs_changed(ui);
    if let Some(owner) = owner {
        owner.renders.set(owner.renders.get() + 1);
    }
}

/// Fixture checks for the editor's pure helpers and GtkSourceView search, run by smoke.
pub fn verify_tools() {
    use sourceview5::prelude::*;
    assert_eq!(text::list_enter("- milk", ""), Some(text::Enter::Continue("- ".into())));
    assert_eq!(text::list_enter("3. three", ""), Some(text::Enter::Continue("4. ".into())));
    assert_eq!(text::list_enter("- [ ] ", ""), Some(text::Enter::End));
    assert_eq!(text::toggle_lines(&["a", "b"], text::Prefix::Bullet), ["- a", "- b"]);
    assert_eq!(text::toggle_lines(&["## a"], text::Prefix::Heading(2)), ["a"]);
    assert_eq!(text::parse_goto("12:4"), Some((12, Some(4))));
    assert!(sourceview5::LanguageManager::default().language("markdown").is_some());
    assert!(scheme().is_some(), "Notes colour scheme");
    let buffer = sourceview5::Buffer::new(None);
    buffer.set_text("one ONE one");
    let settings = sourceview5::SearchSettings::new();
    settings.set_wrap_around(true);
    settings.set_search_text(Some("one"));
    let search = sourceview5::SearchContext::new(&buffer, Some(&settings));
    let (start, end, _) = search.forward(&buffer.start_iter()).expect("first match");
    assert_eq!((start.offset(), end.offset()), (0, 3));
    let (mut start, mut end, _) = search.backward(&buffer.end_iter()).expect("last match");
    assert_eq!((start.offset(), end.offset()), (8, 11));
    search.replace(&mut start, &mut end, "two").expect("replace one");
    assert_eq!(buffer_text(buffer.upcast_ref()), "one ONE two");
    settings.set_case_sensitive(true);
    assert_eq!(crate::editor::replace_all(&search, "one+").expect("replace all"), 1);
    assert_eq!(buffer_text(buffer.upcast_ref()), "one+ ONE two");
    // No match left: the binding's own replace_all aborts here; the helper reports 0.
    settings.set_search_text(Some("absent"));
    assert_eq!(crate::editor::replace_all(&search, "x").expect("replace none"), 0);
    assert_eq!(buffer_text(buffer.upcast_ref()), "one+ ONE two");
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
    d.controls(ui, "module.update", "module_id", id);
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
