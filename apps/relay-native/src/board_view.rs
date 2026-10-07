//! The board: a keyboard-first columns/list view over Relay's own tasks, in the spirit of
//! lific's issue board — status-icon columns, dense cards, a peek panel, quick create and
//! drag-to-reorder — built only on the engine's `task.*` ops.
//!
//! The page stays mounted while events re-render it, so its state (view, filters, hidden
//! columns, focus and the peeked task) lives in one `Board` per page, not in the widgets.
use super::*;
use gtk::cairo;
use gtk::gdk;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use task_pages::{COLUMNS, COLUMN_TITLES, OPEN_COLUMNS, PRIORITIES, SIZES, TYPES};

/// [`PRIORITIES`] most urgent first, the order the board groups and offers them in.
const URGENT_FIRST: [&str; 4] = {
    let mut urgent_first = PRIORITIES;
    let mut i = 0;
    while i < PRIORITIES.len() {
        urgent_first[i] = PRIORITIES[PRIORITIES.len() - 1 - i];
        i += 1;
    }
    urgent_first
};
const GROUPS: [(&str, &str); 7] = [
    ("", "None"),
    ("parent", "Parent"),
    ("type", "Type"),
    ("priority", "Priority"),
    ("size", "Size"),
    ("module", "Module"),
    ("session", "Agent"),
];

type Filters = BTreeMap<String, BTreeSet<String>>;
/// Rendered lanes (or list sections) in order, each with its visible cards in order.
type Lanes = Vec<(String, Vec<(i64, gtk::Widget)>)>;
type Action = Box<dyn Fn(&Rc<Board>)>;

thread_local! {
    static BOARD: RefCell<Option<Rc<Board>>> = const { RefCell::new(None) };
}
fn current() -> Option<Rc<Board>> {
    BOARD.with(|board| board.borrow().clone())
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Zone {
    Before,
    Into,
    After,
}
/// Where a drop lands on a card: its top and bottom 30% insert beside it, the middle nests.
fn zone(y: f64, height: f64) -> Zone {
    if height <= 0. || y < height * 0.3 {
        Zone::Before
    } else if y > height * 0.7 {
        Zone::After
    } else {
        Zone::Into
    }
}

/// The index `moving` should end at when dropped beside `onto`, in a column's board order
/// (`task.move` positions are final indices among the column's other tasks).
fn drop_index(order: &[i64], moving: i64, onto: i64, after: bool) -> usize {
    let others: Vec<_> = order.iter().filter(|id| **id != moving).collect();
    others
        .iter()
        .position(|id| **id == onto)
        .map(|at| at + after as usize)
        .unwrap_or(others.len())
}

/// Every task by id, built once per render: a card's links are looked up, not scanned for.
fn by_id(tasks: &[Value]) -> BTreeMap<i64, &Value> {
    tasks.iter().filter_map(|t| Some((t["id"].as_i64()?, t))).collect()
}

/// The tasks blocking `task` that are not done yet, looked up in [`by_id`]'s index.
fn open_blockers<'a>(task: &Value, all: &BTreeMap<i64, &'a Value>) -> Vec<&'a Value> {
    rows(task, "blocked_by")
        .iter()
        .filter_map(|other| other.as_i64().and_then(|other| all.get(&other)).copied())
        .filter(|t| text(t, "column") != "done")
        .collect()
}

fn caption(column: &str) -> &'static str {
    COLUMN_TITLES
        .iter()
        .find(|(name, _)| *name == column)
        .map(|(_, title)| *title)
        .unwrap_or("Backlog")
}

fn task_matches(task: &Value, filters: &Filters, query: &str) -> bool {
    filters.iter().all(|(key, values)| {
        values.is_empty()
            || values.iter().any(|value| match key.as_str() {
                "parent" if value == "roots" => task["parent_id"].is_null(),
                "parent" | "module" => task[if key == "parent" { "parent_id" } else { "module_id" }]
                    .as_i64()
                    .is_some_and(|id| id.to_string() == *value),
                "label" | "session" => task[if key == "label" { "labels" } else { "sessions" }]
                    .as_array()
                    .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(value))),
                key => text(task, key) == value,
            })
    }) && (query.is_empty() || haystack(task).contains(query))
}
fn haystack(task: &Value) -> String {
    let join = |key: &str| {
        rows(task, key)
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "#{} {} {} {} {} {}",
        task["id"],
        text(task, "title"),
        text(task, "body"),
        join("labels"),
        join("sessions"),
        text(task, "_module_name")
    )
    .to_lowercase()
}

/// A group's sort rank, a space, then its heading: priority and size keep their own order and
/// the fallback groups (`Unassigned`, `No module`…) sort after every named one.
fn group_of(task: &Value, grouping: &str) -> String {
    let named = |name: Option<&str>, none: &str| match name {
        Some(name) => format!("0 {name}"),
        None => format!("9 {none}"),
    };
    match grouping {
        "parent" => named(task["_parent_title"].as_str(), "Top level"),
        "module" => named(task["_module_name"].as_str(), "No module"),
        "session" => named(task["sessions"].as_array().and_then(|v| v.last()).and_then(Value::as_str), "Unassigned"),
        "" => String::new(),
        "priority" => {
            let p = text(task, "priority");
            format!("{} {p}", URGENT_FIRST.iter().position(|x| *x == p).unwrap_or(9))
        }
        "size" => match text(task, "size") {
            "" => "9 None".into(),
            s => format!("{} {s}", SIZES.iter().position(|x| *x == s).unwrap_or(8)),
        },
        key => named(task[key].as_str(), "None"),
    }
}
fn group_title(group: &str) -> String {
    group.split_once(' ').map(|(_, g)| g).unwrap_or(group).to_string()
}

/// `5m`, `3h`, `2d`: how long ago an RFC 3339 timestamp was, at a glance.
fn ago(ts: &str) -> String {
    crate::relative::ago(ts, crate::relative::Form::Compact).unwrap_or_default()
}
/// The peek's longer reading of an [`ago`]: `just now`, `5m ago`, `on Mar 1`, or a dash.
fn since(ts: &str) -> String {
    crate::relative::ago(ts, crate::relative::Form::CompactAgo).unwrap_or_else(|| "—".into())
}

fn hue(name: &str) -> usize {
    name.bytes()
        .fold(2_166_136_261u32, |h, b| (h ^ b as u32).wrapping_mul(16_777_619)) as usize
        % 8
}

fn paint(widget: &gtk::Widget, cr: &cairo::Context, alpha: f64) {
    let c = widget.color();
    cr.set_source_rgba(
        c.red() as f64,
        c.green() as f64,
        c.blue() as f64,
        c.alpha() as f64 * alpha,
    );
}

/// lific's status glyphs in Relay's state colours: dashed backlog, open ready, half-full
/// active, three-quarter review and a checked done.
fn status_icon(column: &str, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_valign(gtk::Align::Center);
    area.add_css_class("status-icon");
    area.add_css_class(&format!("status-{column}"));
    let column = column.to_string();
    area.set_draw_func(move |widget, cr, width, height| {
        let widget = widget.upcast_ref::<gtk::Widget>();
        paint(widget, cr, 1.);
        let (cx, cy) = (width as f64 / 2., height as f64 / 2.);
        let r = width.min(height) as f64 / 2. - 1.;
        cr.set_line_width(1.4);
        let pie = |cr: &cairo::Context, end: f64| {
            cr.move_to(cx, cy);
            cr.arc(cx, cy, r - 2.6, -std::f64::consts::FRAC_PI_2, end);
            cr.close_path();
            let _ = cr.fill();
        };
        match column.as_str() {
            "backlog" => {
                cr.set_dash(&[1.6, 1.9], 0.);
                cr.arc(cx, cy, r - 0.3, 0., std::f64::consts::TAU);
                let _ = cr.stroke();
            }
            "done" => {
                cr.arc(cx, cy, r, 0., std::f64::consts::TAU);
                let _ = cr.fill();
                cr.set_operator(cairo::Operator::Clear);
                cr.set_line_width(1.6);
                cr.move_to(cx - r * 0.45, cy + r * 0.02);
                cr.line_to(cx - r * 0.1, cy + r * 0.36);
                cr.line_to(cx + r * 0.48, cy - r * 0.32);
                let _ = cr.stroke();
            }
            other => {
                cr.arc(cx, cy, r - 0.3, 0., std::f64::consts::TAU);
                let _ = cr.stroke();
                match other {
                    "active" => pie(cr, std::f64::consts::FRAC_PI_2),
                    "in_review" => pie(cr, std::f64::consts::PI),
                    _ => {}
                }
            }
        }
    });
    area
}

/// Signal bars for low/medium/high, a red square with a cut-out `!` for urgent.
fn priority_icon(priority: &str) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(13);
    area.set_content_height(13);
    area.set_valign(gtk::Align::Center);
    area.add_css_class("priority-icon");
    area.add_css_class(&format!("priority-{priority}"));
    area.set_tooltip_text(Some(&format!("Priority: {priority}")));
    let priority = priority.to_string();
    area.set_draw_func(move |widget, cr, _, _| {
        let widget = widget.upcast_ref::<gtk::Widget>();
        if priority == "urgent" {
            paint(widget, cr, 1.);
            cr.rectangle(0.5, 0.5, 12., 12.);
            let _ = cr.fill();
            cr.set_operator(cairo::Operator::Clear);
            cr.rectangle(5.6, 2.8, 1.8, 5.2);
            cr.rectangle(5.6, 9.2, 1.8, 1.8);
            let _ = cr.fill();
            return;
        }
        let lit = match priority.as_str() {
            "high" => 3,
            "medium" => 2,
            _ => 1,
        };
        for i in 0..3 {
            paint(widget, cr, if i < lit { 1. } else { 0.25 });
            let h = [4.5, 8., 11.5][i];
            cr.rectangle(1. + i as f64 * 4., 12.5 - h, 2.6, h);
            let _ = cr.fill();
        }
    });
    area
}

fn lamp(class: &str) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(6);
    area.set_content_height(6);
    area.set_valign(gtk::Align::Center);
    area.add_css_class("board-lamp");
    if !class.is_empty() {
        area.add_css_class(class);
    }
    area.set_draw_func(|widget, cr, w, h| {
        paint(widget.upcast_ref(), cr, 1.);
        cr.arc(w as f64 / 2., h as f64 / 2., w.min(h) as f64 / 2., 0., std::f64::consts::TAU);
        let _ = cr.fill();
    });
    area
}

/// View-toggle glyphs: three lanes for Board, three rows for List.
fn view_glyph(list: bool) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(13);
    area.set_content_height(12);
    area.set_valign(gtk::Align::Center);
    area.set_draw_func(move |widget, cr, _, _| {
        paint(widget.upcast_ref(), cr, 1.);
        cr.set_line_width(1.4);
        for i in 0..3 {
            let at = 1.5 + i as f64 * 4.5;
            if list {
                cr.move_to(0.5, at);
                cr.line_to(12.5, at);
            } else {
                cr.rectangle(at - 1., 0.7, 2.6, 10.6);
            }
        }
        let _ = cr.stroke();
    });
    area
}

fn state_badge(state: &str) -> Option<gtk::Box> {
    let caption = match state {
        "dispatched" => "Dispatched",
        "running" => "Running",
        "blocked" => "Blocked",
        "failed" => "Failed",
        "awaiting_review" => "Review",
        _ => return None,
    };
    let badge = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    badge.add_css_class("state-badge");
    badge.add_css_class(&format!("state-{state}"));
    badge.set_valign(gtk::Align::Center);
    badge.append(&lamp(""));
    badge.append(&label(&caption.to_uppercase(), "state-caption"));
    badge.set_tooltip_text(Some(&format!("Execution state: {}", state.replace('_', " "))));
    Some(badge)
}

fn label_chip(name: &str) -> gtk::Box {
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    chip.add_css_class("label-chip");
    chip.set_valign(gtk::Align::Center);
    let swatch = gtk::DrawingArea::new();
    swatch.set_content_width(6);
    swatch.set_content_height(6);
    swatch.set_valign(gtk::Align::Center);
    swatch.add_css_class(&format!("label-hue-{}", hue(name)));
    swatch.set_draw_func(|widget, cr, w, h| {
        paint(widget.upcast_ref(), cr, 1.);
        cr.rectangle(0., 0., w as f64, h as f64);
        let _ = cr.fill();
    });
    chip.append(&swatch);
    let caption = label(name, "label-chip-name");
    caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
    caption.set_max_width_chars(14);
    chip.append(&caption);
    chip
}

fn labels_row(task: &Value, max: usize) -> Option<gtk::Box> {
    let names: Vec<String> = rows(task, "labels")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if names.is_empty() {
        return None;
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("card-labels");
    for name in names.iter().take(max) {
        row.append(&label_chip(name));
    }
    if names.len() > max {
        let more = label(&format!("+{}", names.len() - max), "label-more");
        more.set_tooltip_text(Some(&names[max..].join(", ")));
        row.append(&more);
    }
    Some(row)
}

fn spacer() -> gtk::Box {
    let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    space.set_hexpand(true);
    space
}

fn icon_text(icon: &gtk::Widget, caption: &str) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(icon);
    if !caption.is_empty() {
        content.append(&label(caption, ""));
    }
    content
}

/// A row that may be wider than a compact window: it clips instead of widening the page.
fn clipped(row: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(row)
        .build()
}

fn drop_id(value: &glib::Value) -> Option<i64> {
    value
        .get::<String>()
        .ok()
        .and_then(|s| s.strip_prefix("relay-task:").and_then(|id| id.parse().ok()))
}

/// The task editor's identity line, in the board's vocabulary: status, number, priority,
/// type, execution state and labels at a glance above the editable fields.
pub(super) fn identity_strip(task: &Value) -> gtk::Box {
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    strip.add_css_class("task-identity");
    let column = text(task, "column");
    let item = |icon: gtk::Widget, caption: &str| {
        let part = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        part.add_css_class("identity-part");
        part.append(&icon);
        part.append(&label(caption, "identity-caption"));
        part
    };
    strip.append(&item(status_icon(column, 13).upcast(), caption(column)));
    strip.append(&label(&format!("#{}", task["id"]), "card-id"));
    let priority = text(task, "priority");
    strip.append(&item(priority_icon(priority).upcast(), priority));
    let kind = text(task, "type");
    strip.append(&item(task_mark(kind, 9).upcast(), kind));
    if let Some(badge) = state_badge(text(task, "state")) {
        strip.append(&badge);
    }
    if let Some(tags) = labels_row(task, 6) {
        strip.append(&tags);
    }
    strip.append(&spacer());
    let when = label(&format!("updated {}", ago(text(task, "updated_at"))), "card-ago");
    when.set_tooltip_text(Some(text(task, "updated_at")));
    strip.append(&when);
    strip
}

struct Quick {
    bar: gtk::Box,
    column: gtk::DropDown,
    title: gtk::Entry,
    kind: gtk::DropDown,
    priority: gtk::DropDown,
    note: gtk::Label,
}

struct Board {
    ui: std::rc::Weak<Ui>,
    project: i64,
    page: gtk::Box,
    keys: gtk::EventControllerKey,
    /// Holds the workspace picker, rebuilt when the projects or workspaces it lists change.
    picker: gtk::Box,
    picker_key: RefCell<String>,
    count: gtk::Label,
    query: gtk::SearchEntry,
    filter: gtk::MenuButton,
    views: [gtk::ToggleButton; 2],
    strip: gtk::Box,
    quick: Quick,
    rail: gtk::Box,
    undo: gtk::Button,
    content: gtk::Box,
    peek: gtk::Box,
    tasks: RefCell<Vec<Value>>,
    filters: RefCell<Filters>,
    group: RefCell<String>,
    list: Cell<bool>,
    hidden: RefCell<BTreeSet<String>>,
    focused: Cell<Option<i64>>,
    peeked: Cell<Option<i64>>,
    dragged: Cell<Option<i64>>,
    layout: RefCell<Lanes>,
    /// Each scroller by role (`board`, `lane:<column>`, `list`), to keep places across renders.
    scrolls: RefCell<Vec<(String, gtk::Adjustment)>>,
    /// What the peek last drew (`peek_key`): an unchanged peek keeps its scroll and focus.
    peek_key: RefCell<Option<String>>,
    peek_shown: Cell<Option<i64>>,
    /// A render is waiting for an open card menu or picker to close.
    deferred: Cell<bool>,
    /// Audit rows Ctrl+Z found stale or not undoable: the next press reaches past them.
    skipped: RefCell<BTreeSet<i64>>,
}

/// Mounts the board on `page` for `project` the first time, then re-renders it with `tasks`.
pub fn show(ui: &Rc<Ui>, page: &gtk::Box, project: i64, tasks: Vec<Value>) {
    let owner = ui.page_projects.borrow().get("board").copied();
    let mut fresh = false;
    let board = match current() {
        Some(board) if owner == Some(project) && board.project == project => board,
        old => {
            fresh = true;
            if let Some(old) = &old {
                page.remove_controller(&old.keys);
            }
            clear(page);
            ui.page_projects.borrow_mut().insert("board".into(), project);
            let board = Board::mount(ui, page, project);
            // The view, hidden columns and grouping follow the person across projects; the
            // filters name one project's modules, labels and agents, so they start empty.
            if let Some(old) = old {
                board.list.set(old.list.get());
                board.views[old.list.get() as usize].set_active(true);
                *board.hidden.borrow_mut() = old.hidden.borrow().clone();
                *board.group.borrow_mut() = old.group.borrow().clone();
            }
            BOARD.with(|slot| *slot.borrow_mut() = Some(board.clone()));
            board
        }
    };
    board.refresh_picker(ui);
    let mut tasks = tasks;
    let titles: BTreeMap<i64, Value> = tasks
        .iter()
        .filter_map(|t| t["id"].as_i64().map(|id| (id, t["title"].clone())))
        .collect();
    for task in &mut tasks {
        if let Some(title) = task["parent_id"].as_i64().and_then(|id| titles.get(&id)) {
            task["_parent_title"] = title.clone();
        }
    }
    // Most events that refresh the page change no task: leave the board, and any open menu,
    // scroll position and selection on it, alone.
    if !fresh && *board.tasks.borrow() == tasks {
        return;
    }
    *board.tasks.borrow_mut() = tasks;
    board.render_when_closed();
}

impl Board {
    fn mount(ui: &Rc<Ui>, page: &gtk::Box, project: i64) -> Rc<Self> {
        page.add_css_class("board-page");
        page.set_spacing(0);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("board-head");
        header.append(&board_switcher(ui, "board"));
        let picker = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.append(&picker);
        let count = label("", "board-count");
        count.set_widget_name("board-count");
        count.set_hexpand(true);
        count.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header.append(&count);
        let query = gtk::SearchEntry::builder()
            .placeholder_text("Search tasks   /")
            .build();
        query.set_widget_name("board-query");
        query.add_css_class("board-search");
        query.set_size_request(200, -1);
        header.append(&query);
        let filter = gtk::MenuButton::new();
        filter.add_css_class("board-tool");
        filter.set_tooltip_text(Some("Filter and group (F)"));
        let views = [gtk::ToggleButton::new(), gtk::ToggleButton::new()];
        let toggle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toggle.add_css_class("board-view-toggle");
        for (index, key) in views.iter().enumerate() {
            let list = index == 1;
            key.set_child(Some(&view_glyph(list)));
            key.set_tooltip_text(Some(if list { "List view (V)" } else { "Board view (V)" }));
            key.update_property(&[gtk::accessible::Property::Label(if list { "List view" } else { "Board view" })]);
            if list {
                key.set_group(Some(&views[0]));
            }
            toggle.append(key);
        }
        views[0].set_active(true);
        header.append(&filter);
        header.append(&toggle);
        let undo = crate::app::icon_button("undo", "Undo the last board change (Ctrl+Z)");
        undo.add_css_class("board-tool");
        header.append(&undo);
        let add = button("", "primary");
        add.add_css_class("board-add");
        add.set_child(Some(&icon_text(
            crate::icons::image("plus", 12).upcast_ref(),
            "New task",
        )));
        add.set_tooltip_text(Some("Quick-create a task (C) · full editor (N)"));
        header.append(&add);
        for child in widgets(&header) {
            if child.parent().as_ref() == Some(header.upcast_ref()) {
                child.set_valign(gtk::Align::Center);
            }
        }
        page.append(&header);

        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        strip.add_css_class("board-strip");
        page.append(&clipped(&strip));

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("board-quick");
        let column = gtk::DropDown::from_strings(&OPEN_COLUMNS.iter().map(|c| caption(c)).collect::<Vec<_>>());
        column.set_tooltip_text(Some("Column"));
        let title = gtk::Entry::builder()
            .placeholder_text("Task title — Enter creates, Esc closes")
            .hexpand(true)
            .build();
        title.set_widget_name("board-quick-title");
        let kind = gtk::DropDown::from_strings(&TYPES);
        kind.set_tooltip_text(Some("Type"));
        let priority = gtk::DropDown::from_strings(&PRIORITIES);
        priority.set_selected(PRIORITIES.iter().position(|p| *p == "medium").unwrap_or(0) as u32);
        priority.set_tooltip_text(Some("Priority"));
        let create = button("Create", "primary");
        let more = button("More fields…", "quiet");
        let close = crate::app::icon_button("close", "Close (Esc)");
        let note = label("", "board-quick-note");
        note.set_ellipsize(gtk::pango::EllipsizeMode::End);
        note.set_max_width_chars(40);
        for widget in [
            column.upcast_ref::<gtk::Widget>(),
            title.upcast_ref(),
            kind.upcast_ref(),
            priority.upcast_ref(),
            note.upcast_ref(),
            create.upcast_ref(),
            more.upcast_ref(),
            close.upcast_ref(),
        ] {
            widget.set_valign(gtk::Align::Center);
            bar.append(widget);
        }
        bar.set_visible(false);
        page.append(&bar);

        let rail = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        rail.set_widget_name("board-drag-rail");
        rail.add_css_class("board-drag-rail");
        rail.append(&label("DROP TO SET", "board-rail-caption"));
        rail.set_visible(false);
        // Floats over the lanes: shown at drag start, it must not push the card under the pointer.
        rail.set_valign(gtk::Align::Start);
        rail.set_halign(gtk::Align::Start);

        let main = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        main.set_vexpand(true);
        main.add_css_class("board-main");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_hexpand(true);
        content.set_vexpand(true);
        main.append(&content);
        let peek = gtk::Box::new(gtk::Orientation::Vertical, 0);
        peek.add_css_class("board-peek");
        peek.set_size_request(380, -1);
        peek.set_visible(false);
        main.append(&peek);
        let stack = gtk::Overlay::new();
        stack.set_child(Some(&main));
        stack.add_overlay(&rail);
        page.append(&stack);

        let hints = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        hints.add_css_class("board-shortcuts");
        for (key, action) in [
            ("J K", "move"),
            ("H L", "column"),
            ("↵", "open"),
            ("Space", "peek"),
            ("C", "new"),
            ("[ ]", "change column"),
            ("⇧J ⇧K", "reorder"),
            ("P", "priority"),
            (".", "actions"),
            ("/", "search"),
            ("F", "filter"),
            ("V", "view"),
        ] {
            hints.append(&label(key, "board-kbd"));
            hints.append(&label(action, "board-hint"));
        }
        page.append(&clipped(&hints));

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let board = Rc::new(Self {
            ui: Rc::downgrade(ui),
            project,
            page: page.clone(),
            keys: keys.clone(),
            picker,
            picker_key: RefCell::new(String::new()),
            count,
            query: query.clone(),
            filter: filter.clone(),
            views: views.clone(),
            strip,
            quick: Quick {
                bar,
                column,
                title: title.clone(),
                kind,
                priority,
                note,
            },
            rail: rail.clone(),
            undo: undo.clone(),
            content,
            peek,
            tasks: RefCell::new(Vec::new()),
            filters: RefCell::new(Filters::new()),
            group: RefCell::new(String::new()),
            list: Cell::new(false),
            hidden: RefCell::new(BTreeSet::new()),
            focused: Cell::new(None),
            peeked: Cell::new(None),
            dragged: Cell::new(None),
            layout: RefCell::new(Vec::new()),
            scrolls: RefCell::new(Vec::new()),
            peek_key: RefCell::new(None),
            peek_shown: Cell::new(None),
            deferred: Cell::new(false),
            skipped: RefCell::new(BTreeSet::new()),
        });

        for (field, values) in [
            ("priority", &PRIORITIES[..]),
            ("size", &SIZES[1..]),
            ("type", &TYPES[..]),
        ] {
            for value in values {
                let key = button("", "quiet");
                let mark: gtk::Widget = match field {
                    "priority" => priority_icon(value).upcast(),
                    "type" => task_mark(value, 9).upcast(),
                    _ => label("", "").upcast(),
                };
                key.set_child(Some(&icon_text(&mark, value)));
                let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
                let weak = Rc::downgrade(&board);
                drop.connect_drop(move |_, token, _, _| {
                    let (Some(board), Some(id)) = (weak.upgrade(), drop_id(token)) else {
                        return false;
                    };
                    board.act("task.update", json!({"task_id":id, field:value}));
                    true
                });
                key.add_controller(drop);
                rail.append(&key);
            }
        }

        let weak = Rc::downgrade(&board);
        query.connect_search_changed(move |_| {
            if let Some(board) = weak.upgrade() {
                board.render();
            }
        });
        let weak = Rc::downgrade(&board);
        query.connect_stop_search(move |entry| {
            entry.set_text("");
            if let Some(board) = weak.upgrade() {
                board.focus_board();
            }
        });
        let popover = gtk::Popover::new();
        popover.add_css_class("board-menu");
        filter.set_popover(Some(&popover));
        let weak = Rc::downgrade(&board);
        filter.set_create_popup_func(move |_| {
            if let Some(board) = weak.upgrade() {
                board.filter_menu(&popover);
            }
        });
        let weak = Rc::downgrade(&board);
        views[1].connect_toggled(move |key| {
            if let Some(board) = weak.upgrade() {
                if board.list.replace(key.is_active()) != key.is_active() {
                    board.render();
                }
            }
        });
        let weak = Rc::downgrade(&board);
        undo.connect_clicked(move |key| {
            if let Some(board) = weak.upgrade() {
                board.undo_last(key);
            }
        });
        let weak = Rc::downgrade(&board);
        add.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                // The click took focus from the card, so start in the last focused card's column.
                let column = board.focused.get().and_then(|id| board.task(id)).map(|t| text(&t, "column").to_string());
                board.open_quick(column.as_deref().unwrap_or("backlog"));
            }
        });
        let weak = Rc::downgrade(&board);
        title.connect_activate(move |_| {
            if let Some(board) = weak.upgrade() {
                board.quick_create();
            }
        });
        let weak = Rc::downgrade(&board);
        create.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                board.quick_create();
            }
        });
        let weak = Rc::downgrade(&board);
        close.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                board.close_quick();
            }
        });
        let weak = Rc::downgrade(&board);
        more.connect_clicked(move |_| {
            if let (Some(board), Some(ui)) = (weak.upgrade(), weak.upgrade().and_then(|b| b.ui.upgrade())) {
                board.close_quick();
                task_pages::compose(&ui, board.project);
            }
        });
        let escape = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&board);
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(board) = weak.upgrade() {
                    board.close_quick();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        board.quick.bar.add_controller(escape);

        let weak = Rc::downgrade(&board);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(board) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            board.key(key, mods)
        });
        page.add_controller(keys);
        board
    }

    fn ui(&self) -> Option<Rc<Ui>> {
        self.ui.upgrade()
    }
    /// Rebuilds the workspace picker once a project or workspace it lists is added, renamed or
    /// removed, unless its menu is open.
    fn refresh_picker(&self, ui: &Rc<Ui>) {
        let key = {
            let pick = |rows: &[Value], keys: &[&str]| -> Vec<Value> {
                rows.iter().map(|row| Value::Array(keys.iter().map(|k| row[*k].clone()).collect())).collect()
            };
            json!([pick(&ui.projects.borrow(), &["id", "name", "workspace_id"]), pick(&ui.workspaces.borrow(), &["id", "name"])])
                .to_string()
        };
        if *self.picker_key.borrow() == key || crate::app::open_popover(self.picker.upcast_ref()).is_some() {
            return;
        }
        *self.picker_key.borrow_mut() = key;
        clear(&self.picker);
        self.picker.append(&workspace_picker(ui, self.project, "board"));
    }
    fn task(&self, id: i64) -> Option<Value> {
        self.tasks
            .borrow()
            .iter()
            .find(|t| t["id"].as_i64() == Some(id))
            .cloned()
    }
    /// Every live task of `column` in board order, filters ignored: the order positions index.
    fn column_order(&self, column: &str) -> Vec<i64> {
        let mut tasks: Vec<_> = self
            .tasks
            .borrow()
            .iter()
            .filter(|t| text(t, "column") == column)
            .map(|t| (t["position"].as_i64().unwrap_or(0), t["id"].as_i64().unwrap_or(0)))
            .collect();
        tasks.sort();
        tasks.into_iter().map(|(_, id)| id).collect()
    }

    fn act(self: &Rc<Self>, op: &'static str, payload: Value) {
        let Some(ui) = self.ui() else { return };
        glib::spawn_future_local(async move {
            if let Err(error) = ui.call(op, payload).await {
                ui.show_error(&error.to_string());
            }
            ui.refresh_page();
        });
    }
    /// Moves a task to `column` (at a final index when given). Done is reached by approval,
    /// exactly as the task editor's Approve: the engine links the last agent's branch tip, or
    /// the project HEAD for a task that was never dispatched.
    fn move_to(self: &Rc<Self>, task: &Value, column: &str, position: Option<usize>) {
        let from = text(task, "column");
        if column == "done" {
            if from != "done" {
                if let Some(ui) = self.ui() {
                    ui.show_error(&format!(
                        "Approved #{} into Done · Ctrl+Z moves it back; the linked commit stays",
                        task["id"]
                    ));
                }
                self.act("task.approve", json!({"task_id":task["id"]}));
            }
            return;
        }
        // Picking the column a task is already in would append it to the bottom.
        if column == from && position.is_none() {
            return;
        }
        // Moved here at once, so a key pressed again before the engine's refresh lands builds
        // on this move; that refresh confirms or corrects it.
        let id = task["id"].as_i64();
        let mut order = self.column_order(column);
        order.retain(|other| Some(*other) != id);
        if let Some(id) = id {
            order.insert(position.unwrap_or(order.len()).min(order.len()), id);
        }
        for t in self.tasks.borrow_mut().iter_mut() {
            if t["id"].as_i64() == id {
                t["column"] = column.into();
            }
            if let Some(at) = t["id"].as_i64().and_then(|i| order.iter().position(|o| *o == i)) {
                t["position"] = json!(at);
            }
        }
        self.render_when_closed();
        let mut payload = json!({"task_id":task["id"],"column":column});
        if let Some(at) = position {
            payload["position"] = json!(at);
        }
        self.act("task.move", payload);
    }
    /// Sets one field, applied locally and redrawn at once like [`Self::move_to`]. Picking the
    /// value a task already has writes nothing: a no-op update would only stale its undo rows.
    fn set_field(self: &Rc<Self>, id: i64, field: &'static str, value: Value) {
        {
            let mut tasks = self.tasks.borrow_mut();
            let Some(task) = tasks.iter_mut().find(|t| t["id"].as_i64() == Some(id)) else { return };
            if task[field] == value {
                return;
            }
            task[field] = value.clone();
        }
        self.render_when_closed();
        self.act("task.update", json!({"task_id":id, field:value}));
    }
    fn drop_on(self: &Rc<Self>, moving: i64, onto: i64, zone: Zone) -> bool {
        let (Some(task), Some(target)) = (self.task(moving), self.task(onto)) else {
            return false;
        };
        if moving == onto {
            return false;
        }
        if zone == Zone::Into {
            self.act("task.parent.set", json!({"task_id":moving,"parent_id":onto}));
            return true;
        }
        let column = text(&target, "column").to_string();
        if column == "done" && text(&task, "column") == "done" {
            return false;
        }
        let at = drop_index(&self.column_order(&column), moving, onto, zone == Zone::After);
        self.focused.set(Some(moving));
        self.move_to(&task, &column, Some(at));
        true
    }

    fn undo_last(self: &Rc<Self>, key: &gtk::Button) {
        let Some(ui) = self.ui() else { return };
        let project = self.project;
        key.set_sensitive(false);
        let key = key.clone();
        let board = self.clone();
        glib::spawn_future_local(async move {
            let result = async {
                let history = ui
                    .call(
                        "audit.list",
                        json!({"project_id":project,"actor":"user","op_prefix":"task.","limit":50}),
                    )
                    .await?;
                let skipped = board.skipped.borrow().clone();
                match rows(&history, "rows").iter().find(|r| {
                    !r["undo_op"].is_null()
                        && r["undone_by"].is_null()
                        && !r["id"].as_i64().is_some_and(|id| skipped.contains(&id))
                }) {
                    Some(row) => {
                        let what = format!("{} on #{}", text(row, "op"), row["payload"]["task_id"]);
                        match ui.call("audit.undo", json!({"audit_id":row["id"]})).await {
                            Ok(_) => ui.show_error(&format!("Undid {what}")),
                            // A row that cannot be undone must not block every older one.
                            Err(crate::client::Error::Bus(error))
                                if matches!(error.code.as_str(), "audit.stale" | "audit.not_undoable") =>
                            {
                                if let Some(id) = row["id"].as_i64() {
                                    board.skipped.borrow_mut().insert(id);
                                }
                                ui.show_error(&format!(
                                    "Cannot undo {what}: {}. Press Ctrl+Z again for the change before it.",
                                    error.message
                                ));
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    None => ui.show_error("Nothing on the board to undo."),
                }
                Ok::<(), crate::client::Error>(())
            }
            .await;
            if let Err(error) = result {
                ui.show_error(&error.to_string());
            }
            key.set_sensitive(true);
            ui.refresh_page();
        });
    }

    fn open_quick(self: &Rc<Self>, column: &str) {
        let at = OPEN_COLUMNS.iter().position(|c| *c == column).unwrap_or(0);
        self.quick.column.set_selected(at as u32);
        self.quick.note.set_text("");
        self.quick.bar.set_visible(true);
        self.quick.title.grab_focus();
    }
    fn close_quick(self: &Rc<Self>) {
        self.quick.bar.set_visible(false);
        self.quick.title.set_text("");
        self.focus_board();
    }
    fn quick_create(self: &Rc<Self>) {
        let Some(ui) = self.ui() else { return };
        let title = self.quick.title.text().trim().to_string();
        if title.is_empty() {
            self.quick.title.grab_focus();
            return;
        }
        let pick = |control: &gtk::DropDown, values: &[&'static str]| {
            values[(control.selected() as usize).min(values.len() - 1)]
        };
        let payload = json!({
            "project_id": self.project,
            "title": title,
            "column": pick(&self.quick.column, OPEN_COLUMNS),
            "type": pick(&self.quick.kind, &TYPES),
            "priority": pick(&self.quick.priority, &PRIORITIES),
        });
        // The whole bar, Create included, waits for the reply: a second click would create twice.
        self.quick.bar.set_sensitive(false);
        let board = self.clone();
        glib::spawn_future_local(async move {
            match ui.call("task.create", payload).await {
                Ok(task) => {
                    if board.quick.title.text().trim() == title {
                        board.quick.title.set_text("");
                    }
                    board.quick.note.set_text(&format!("Created #{}", task["id"]));
                    board.focused.set(task["id"].as_i64());
                }
                Err(error) => board.quick.note.set_text(&error.to_string()),
            }
            board.quick.bar.set_sensitive(true);
            board.quick.title.grab_focus();
            ui.refresh_page();
        });
    }

    fn focus_board(self: &Rc<Self>) {
        let layout = self.layout.borrow();
        let cards = layout.iter().flat_map(|(_, cards)| cards.iter());
        let mut cards: Vec<_> = cards.collect();
        if let Some(id) = self.focused.get() {
            cards.sort_by_key(|(card, _)| *card != id);
        }
        if let Some((_, card)) = cards.first() {
            card.grab_focus();
        } else {
            self.content.grab_focus();
        }
    }
    /// The focused card as (column, index in its lane), if focus is on one. A button inside the
    /// card, or its menu, is not the card: Enter and Space press it.
    fn current(&self) -> Option<(String, usize)> {
        let focus = self.page.root().and_then(|root| root.focus())?;
        self.layout.borrow().iter().find_map(|(column, cards)| {
            cards
                .iter()
                .position(|(_, card)| focus == *card)
                .map(|at| (column.clone(), at))
        })
    }
    fn current_id(&self) -> Option<i64> {
        let (column, at) = self.current()?;
        self.layout
            .borrow()
            .iter()
            .find(|(c, _)| *c == column)
            .and_then(|(_, cards)| cards.get(at).map(|(id, _)| *id))
    }
    fn focus_at(&self, lane: usize, at: usize) {
        if let Some((_, cards)) = self.layout.borrow().get(lane) {
            if let Some((id, card)) = cards.get(at.min(cards.len().saturating_sub(1))) {
                self.focused.set(Some(*id));
                card.grab_focus();
            }
        }
    }

    fn key(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use gdk::Key;
        let focus = self.page.root().and_then(|root| root.focus());
        let mut ancestor = focus.clone();
        while let Some(widget) = ancestor {
            // Menus and pickers keep their own keys: arrows, Enter and Escape act inside them.
            if widget.is::<gtk::Editable>() || widget.is::<gtk::TextView>() || widget.is::<gtk::Popover>() {
                return glib::Propagation::Proceed;
            }
            ancestor = widget.parent();
        }
        // Caps Lock sends uppercase letters without Shift: the intent to reorder is Shift alone.
        let key = key.to_lower();
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        if mods.contains(gdk::ModifierType::CONTROL_MASK) && !shift && key == Key::z {
            // emit_clicked runs even while an undo is in flight and the key is insensitive.
            if self.undo.is_sensitive() {
                self.undo.emit_clicked();
            }
            return glib::Propagation::Stop;
        }
        if mods.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        ) {
            return glib::Propagation::Proceed;
        }
        let lanes: Vec<(String, usize)> = self
            .layout
            .borrow()
            .iter()
            .map(|(c, cards)| (c.clone(), cards.len()))
            .collect();
        let here = self.current();
        let lane = here
            .as_ref()
            .and_then(|(c, _)| lanes.iter().position(|(name, _)| name == c));
        let id = self.current_id();
        let task = id.and_then(|id| self.task(id));
        // Card commands only claim their key while a card has focus, so Enter and Space
        // still press a focused button in the header, the strip or the peek panel.
        let card_command = matches!(
            key,
            Key::Return | Key::KP_Enter | Key::ISO_Enter | Key::space | Key::period | Key::Menu
                | Key::bracketleft | Key::bracketright | Key::p
        ) || (shift && matches!(key, Key::j | Key::k | Key::Up | Key::Down));
        if card_command && task.is_none() {
            return glib::Propagation::Proceed;
        }
        match key {
            Key::Escape => {
                if self.quick.bar.is_visible() {
                    self.close_quick();
                } else if self.peeked.get().is_some() {
                    self.peeked.set(None);
                    self.render_peek();
                    self.focus_board();
                } else {
                    return glib::Propagation::Proceed;
                }
            }
            Key::j | Key::k if shift => self.nudge(task.as_ref(), if key == Key::j { 1 } else { -1 }),
            Key::Down | Key::Up if shift => {
                self.nudge(task.as_ref(), if key == Key::Down { 1 } else { -1 })
            }
            Key::j | Key::k | Key::Down | Key::Up => {
                let delta: i64 = if matches!(key, Key::k | Key::Up) { -1 } else { 1 };
                match (lane, here.as_ref()) {
                    (Some(lane), Some((_, at))) => {
                        let next = *at as i64 + delta;
                        if self.list.get() && (next < 0 || next >= lanes[lane].1 as i64) {
                            // The list reads as one sequence: step into the next section.
                            let mut l = lane as i64 + delta;
                            while (0..lanes.len() as i64).contains(&l) {
                                let count = lanes[l as usize].1;
                                if count > 0 {
                                    self.focus_at(l as usize, if delta > 0 { 0 } else { count - 1 });
                                    break;
                                }
                                l += delta;
                            }
                        } else {
                            self.focus_at(lane, next.max(0) as usize);
                        }
                    }
                    _ => self.focus_board(),
                }
            }
            Key::h | Key::l | Key::Left | Key::Right => {
                let delta: i64 = if matches!(key, Key::h | Key::Left) { -1 } else { 1 };
                let at = here.as_ref().map(|(_, at)| *at).unwrap_or(0);
                let mut l = lane.map(|l| l as i64).unwrap_or(if delta > 0 { -1 } else { lanes.len() as i64 }) + delta;
                while (0..lanes.len() as i64).contains(&l) {
                    if lanes[l as usize].1 > 0 {
                        self.focus_at(l as usize, at);
                        break;
                    }
                    l += delta;
                }
            }
            Key::Return | Key::KP_Enter | Key::ISO_Enter => {
                if let (Some(id), Some(ui)) = (id, self.ui()) {
                    task_pages::open(&ui, id);
                }
            }
            Key::space => {
                if let Some(id) = id {
                    self.toggle_peek(id);
                }
            }
            Key::c => {
                let column = here.map(|(c, _)| c);
                self.open_quick(column.as_deref().unwrap_or("backlog"));
            }
            Key::n => {
                if let Some(ui) = self.ui() {
                    task_pages::compose(&ui, self.project);
                }
            }
            Key::slash => {
                self.query.grab_focus();
            }
            Key::f => self.filter.popup(),
            Key::v => {
                let list = !self.list.get();
                self.views[list as usize].set_active(true);
            }
            Key::g => {
                let now = self.group.borrow().clone();
                let at = GROUPS.iter().position(|(g, _)| *g == now).unwrap_or(0);
                *self.group.borrow_mut() = GROUPS[(at + 1) % GROUPS.len()].0.to_string();
                self.render();
            }
            Key::bracketleft | Key::bracketright => {
                if let Some(task) = task {
                    let at = COLUMNS.iter().position(|c| *c == text(&task, "column")).unwrap_or(0) as i64;
                    let next = (at + if key == Key::bracketleft { -1 } else { 1 }).clamp(0, COLUMNS.len() as i64 - 1);
                    if next != at {
                        self.focused.set(task["id"].as_i64());
                        self.move_to(&task, COLUMNS[next as usize], None);
                    }
                }
            }
            Key::p => {
                if let Some(task) = task {
                    let at = PRIORITIES.iter().position(|p| *p == text(&task, "priority")).unwrap_or(1);
                    if let Some(id) = task["id"].as_i64() {
                        self.focused.set(Some(id));
                        self.set_field(id, "priority", json!(PRIORITIES[(at + 1) % PRIORITIES.len()]));
                    }
                }
            }
            Key::period | Key::Menu => {
                if let (Some(id), Some(focus)) = (id, focus) {
                    let card = self.layout.borrow().iter().find_map(|(_, cards)| {
                        cards.iter().find(|(i, _)| *i == id).map(|(_, w)| w.clone())
                    });
                    self.menu(card.as_ref().unwrap_or(&focus), id, None);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }
    /// Shift+J/K: the focused card swaps places with its neighbour in the lane as shown, which
    /// leaves out filtered tasks and orders by group first.
    fn nudge(self: &Rc<Self>, task: Option<&Value>, delta: i64) {
        let Some(task) = task else { return };
        let Some(id) = task["id"].as_i64() else { return };
        let column = text(task, "column").to_string();
        if column == "done" {
            return;
        }
        let neighbour = self.layout.borrow().iter().find(|(lane, _)| *lane == column).and_then(|(_, cards)| {
            let at = cards.iter().position(|(card, _)| *card == id)? as i64 + delta;
            usize::try_from(at).ok().and_then(|at| cards.get(at)).map(|(card, _)| *card)
        });
        let Some(neighbour) = neighbour.and_then(|other| self.task(other)) else { return };
        let grouping = self.group.borrow().clone();
        if group_of(task, &grouping) != group_of(&neighbour, &grouping) {
            if let Some(ui) = self.ui() {
                ui.show_error("Cards keep to their group: change the task or the grouping to move it past one.");
            }
            return;
        }
        let Some(onto) = neighbour["id"].as_i64() else { return };
        self.focused.set(Some(id));
        self.move_to(task, &column, Some(drop_index(&self.column_order(&column), id, onto, delta > 0)));
    }

    fn toggle_peek(self: &Rc<Self>, id: i64) {
        self.peeked.set(if self.peeked.get() == Some(id) { None } else { Some(id) });
        self.focused.set(Some(id));
        self.render_peek();
    }

    /// Render now, or once an open card menu or peek picker closes: a render destroys the
    /// widgets those popovers hang from.
    fn render_when_closed(self: &Rc<Self>) {
        let Some(popover) = crate::app::open_popover(self.page.upcast_ref()) else {
            self.deferred.set(false);
            self.render();
            return;
        };
        if !self.deferred.replace(true) {
            let weak = Rc::downgrade(self);
            popover.connect_closed(move |_| {
                let Some(board) = weak.upgrade() else { return };
                if board.deferred.get() {
                    // The popover is still mapped while `closed` runs.
                    glib::idle_add_local_once(move || board.render_when_closed());
                }
            });
        }
    }

    fn render(self: &Rc<Self>) {
        let focus_inside = self
            .page
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| focus.is_ancestor(&self.content));
        let saved: BTreeMap<String, f64> = self.scrolls.borrow().iter().map(|(role, a)| (role.clone(), a.value())).collect();
        clear(&self.content);
        self.layout.borrow_mut().clear();
        self.scrolls.borrow_mut().clear();
        let tasks = self.tasks.borrow().clone();
        let filters = self.filters.borrow().clone();
        let query = self.query.text().trim().to_lowercase();
        let visible: Vec<&Value> = tasks
            .iter()
            .filter(|t| task_matches(t, &filters, &query))
            .collect();
        let open = tasks.iter().filter(|t| text(t, "column") != "done").count();
        let mut summary = format!("{open} open · {} done", tasks.len() - open);
        if visible.len() != tasks.len() {
            summary.push_str(&format!(" · {} shown", visible.len()));
        }
        self.count.set_text(&summary);
        let active = filters.values().map(BTreeSet::len).sum::<usize>();
        self.filter.set_child(Some(&icon_text(
            crate::icons::image("sliders", 12).upcast_ref(),
            &if active > 0 { format!("Filter · {active}") } else { "Filter".into() },
        )));
        if active > 0 {
            self.filter.add_css_class("active");
        } else {
            self.filter.remove_css_class("active");
        }
        self.render_strip(&tasks, &visible);
        if tasks.is_empty() {
            self.render_empty();
        } else {
            self.render_lanes(&tasks, &visible);
        }
        // Keep each scroller where it was: restore once the new content has its size. A scroller
        // the last render did not have (another view, a column shown again) starts at the top.
        for (role, adjustment) in self.scrolls.borrow().iter() {
            let Some(value) = saved.get(role).copied().filter(|v| *v > 0.) else {
                continue;
            };
            let done = Cell::new(false);
            adjustment.connect_changed(move |a| {
                if !done.get() && a.upper() - a.page_size() >= value {
                    done.set(true);
                    a.set_value(value);
                }
            });
        }
        if focus_inside {
            let id = self.focused.get();
            let card = self.layout.borrow().iter().find_map(|(_, cards)| {
                cards.iter().find(|(i, _)| Some(*i) == id).map(|(_, w)| w.clone())
            });
            match card {
                Some(card) => {
                    card.grab_focus();
                }
                None => self.focus_board(),
            }
        }
        self.render_peek();
    }

    fn render_empty(self: &Rc<Self>) {
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 8);
        empty.add_css_class("board-blank");
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        empty.set_vexpand(true);
        let icon = status_icon("backlog", 28);
        icon.set_halign(gtk::Align::Center);
        empty.append(&icon);
        let title = label("No tasks yet", "board-blank-title");
        title.set_halign(gtk::Align::Center);
        empty.append(&title);
        let hint = label(
            "Tasks you add here are what agents are dispatched to. Press C to add one.",
            "board-blank-hint",
        );
        hint.set_halign(gtk::Align::Center);
        empty.append(&hint);
        let add = button("New task", "primary");
        add.set_halign(gtk::Align::Center);
        let weak = Rc::downgrade(self);
        add.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                board.open_quick("backlog");
            }
        });
        empty.append(&add);
        self.content.append(&empty);
    }

    fn render_strip(self: &Rc<Self>, tasks: &[Value], visible: &[&Value]) {
        clear(&self.strip);
        self.strip.append(&label("COLUMNS", "board-strip-caption"));
        let hidden = self.hidden.borrow().clone();
        for (column, title) in COLUMN_TITLES {
            let shown = visible.iter().filter(|t| text(t, "column") == column).count();
            let chip = gtk::ToggleButton::new();
            chip.add_css_class("column-chip");
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.append(&status_icon(column, 12));
            content.append(&label(title, "column-chip-name"));
            content.append(&label(&shown.to_string(), "column-chip-count"));
            chip.set_child(Some(&content));
            chip.set_active(!hidden.contains(column));
            chip.set_tooltip_text(Some(&format!("Show or hide {title}")));
            let weak = Rc::downgrade(self);
            chip.connect_toggled(move |chip| {
                if let Some(board) = weak.upgrade() {
                    if chip.is_active() {
                        board.hidden.borrow_mut().remove(column);
                    } else {
                        board.hidden.borrow_mut().insert(column.to_string());
                    }
                    glib::idle_add_local_once(move || board.render());
                }
            });
            self.strip.append(&chip);
        }
        let filters = self.filters.borrow().clone();
        let group = self.group.borrow().clone();
        if filters.values().any(|v| !v.is_empty()) || !group.is_empty() {
            let rule = gtk::Separator::new(gtk::Orientation::Vertical);
            rule.add_css_class("board-strip-rule");
            self.strip.append(&rule);
        }
        for (key, values) in &filters {
            for value in values {
                let name = match key.as_str() {
                    "module" => tasks
                        .iter()
                        .find(|t| t["module_id"].as_i64() == value.parse().ok())
                        .and_then(|t| t["_module_name"].as_str())
                        .unwrap_or(value)
                        .to_string(),
                    "parent" if value == "roots" => "top level".into(),
                    "parent" => tasks
                        .iter()
                        .find(|t| t["id"].as_i64() == value.parse().ok())
                        .map(|t| format!("#{value} {}", text(t, "title")))
                        .unwrap_or_else(|| format!("#{value}")),
                    _ => value.clone(),
                };
                let key_name = if key == "session" { "agent" } else { key.as_str() };
                self.strip.append(&self.removable(&format!("{key_name}: {name}"), {
                    let (key, value) = (key.clone(), value.clone());
                    move |board| {
                        if let Some(set) = board.filters.borrow_mut().get_mut(&key) {
                            set.remove(&value);
                        }
                    }
                }));
            }
        }
        if !group.is_empty() {
            let name = GROUPS.iter().find(|(g, _)| *g == group).map(|(_, n)| *n).unwrap_or("");
            self.strip.append(&self.removable(&format!("grouped by {}", name.to_lowercase()), |board| {
                board.group.borrow_mut().clear();
            }));
        }
    }
    fn removable(self: &Rc<Self>, caption: &str, remove: impl Fn(&Rc<Board>) + 'static) -> gtk::Button {
        let chip = button("", "filter-chip");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let name = label(caption, "");
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_max_width_chars(28);
        content.append(&name);
        content.append(&crate::icons::image("close", 10));
        chip.set_child(Some(&content));
        chip.set_tooltip_text(Some("Remove"));
        let weak = Rc::downgrade(self);
        chip.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                remove(&board);
                glib::idle_add_local_once(move || board.render());
            }
        });
        chip
    }

    fn filter_menu(self: &Rc<Self>, popover: &gtk::Popover) {
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 8);
        menu.add_css_class("board-filter");
        menu.set_size_request(420, -1);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&label("FILTER", "board-menu-caption"));
        head.append(&spacer());
        let reset = button("Clear all", "quiet");
        let weak = Rc::downgrade(self);
        let pop = popover.downgrade();
        reset.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                board.filters.borrow_mut().clear();
                board.group.borrow_mut().clear();
                board.render();
                if let Some(pop) = pop.upgrade() {
                    board.filter_menu(&pop);
                }
            }
        });
        head.append(&reset);
        menu.append(&head);
        let tasks = self.tasks.borrow().clone();
        type Options = Vec<(&'static str, &'static str, Vec<(String, String)>)>;
        let mut options: Options = vec![
            ("type", "Type", TYPES.iter().map(|t| (t.to_string(), t.to_string())).collect()),
            (
                "priority",
                "Priority",
                URGENT_FIRST.iter().map(|t| (t.to_string(), t.to_string())).collect(),
            ),
            ("size", "Size", SIZES[1..].iter().map(|t| (t.to_string(), t.to_string())).collect()),
        ];
        let mut modules = BTreeMap::new();
        let mut labels = BTreeSet::new();
        let mut agents = BTreeSet::new();
        let mut parents = vec![("roots".to_string(), "Top level only".to_string())];
        for task in &tasks {
            if let Some(id) = task["module_id"].as_i64() {
                modules.insert(
                    id.to_string(),
                    task["_module_name"].as_str().map(str::to_string).unwrap_or_else(|| format!("Module #{id}")),
                );
            }
            labels.extend(rows(task, "labels").iter().filter_map(|v| v.as_str().map(str::to_string)));
            agents.extend(rows(task, "sessions").iter().filter_map(|v| v.as_str().map(str::to_string)));
            if tasks.iter().any(|t| t["parent_id"] == task["id"]) {
                parents.push((task["id"].to_string(), format!("#{} {}", task["id"], text(task, "title"))));
            }
        }
        for (key, title, values) in [
            ("module", "Module", modules.into_iter().collect::<Vec<_>>()),
            ("label", "Label", labels.into_iter().map(|l| (l.clone(), l)).collect()),
            ("session", "Agent", agents.into_iter().map(|a| (a.clone(), a)).collect()),
            ("parent", "Parent", parents),
        ] {
            options.push((key, title, values));
        }
        for (key, title, values) in options {
            if values.is_empty() {
                continue;
            }
            menu.append(&label(&title.to_uppercase(), "board-menu-caption"));
            let flow = gtk::FlowBox::new();
            flow.set_selection_mode(gtk::SelectionMode::None);
            flow.set_max_children_per_line(8);
            flow.set_column_spacing(4);
            flow.set_row_spacing(4);
            for (value, caption) in values {
                let chip = gtk::ToggleButton::new();
                chip.add_css_class("filter-option");
                let mark: Option<gtk::Widget> = match key {
                    "type" => Some(task_mark(&value, 9).upcast()),
                    "priority" => Some(priority_icon(&value).upcast()),
                    "label" => Some(label_chip(&value).upcast()),
                    _ => None,
                };
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                match mark {
                    Some(mark) if key == "label" => content.append(&mark),
                    Some(mark) => {
                        content.append(&mark);
                        content.append(&label(&caption, ""));
                    }
                    None => {
                        let name = label(&caption, "");
                        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        name.set_max_width_chars(26);
                        content.append(&name);
                    }
                }
                chip.set_child(Some(&content));
                chip.set_active(self.filters.borrow().get(key).is_some_and(|s| s.contains(&value)));
                let weak = Rc::downgrade(self);
                chip.connect_toggled(move |chip| {
                    if let Some(board) = weak.upgrade() {
                        let mut filters = board.filters.borrow_mut();
                        let set = filters.entry(key.to_string()).or_default();
                        if chip.is_active() {
                            set.insert(value.clone());
                        } else {
                            set.remove(&value);
                        }
                        if set.is_empty() {
                            filters.remove(key);
                        }
                        drop(filters);
                        board.render();
                    }
                });
                flow.insert(&chip, -1);
            }
            menu.append(&flow);
        }
        menu.append(&label("GROUP CARDS BY", "board-menu-caption"));
        let groups = gtk::FlowBox::new();
        groups.set_selection_mode(gtk::SelectionMode::None);
        groups.set_max_children_per_line(7);
        groups.set_column_spacing(4);
        let mut first: Option<gtk::ToggleButton> = None;
        for (group, title) in GROUPS {
            let chip = gtk::ToggleButton::with_label(title);
            chip.add_css_class("filter-option");
            match &first {
                Some(first) => chip.set_group(Some(first)),
                None => first = Some(chip.clone()),
            }
            chip.set_active(*self.group.borrow() == group);
            let weak = Rc::downgrade(self);
            chip.connect_toggled(move |chip| {
                if let (true, Some(board)) = (chip.is_active(), weak.upgrade()) {
                    *board.group.borrow_mut() = group.to_string();
                    board.render();
                }
            });
            groups.insert(&chip, -1);
        }
        menu.append(&groups);
        let scroll = crate::app::scrolled(&menu);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(560);
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        popover.set_child(Some(&scroll));
    }

    fn sorted<'a>(&self, visible: &[&'a Value], column: &str) -> Vec<&'a Value> {
        let grouping = self.group.borrow().clone();
        let mut tasks: Vec<&Value> = visible
            .iter()
            .copied()
            .filter(|t| text(t, "column") == column)
            .collect();
        tasks.sort_by(|a, b| {
            group_of(a, &grouping)
                .cmp(&group_of(b, &grouping))
                .then_with(|| a["position"].as_i64().cmp(&b["position"].as_i64()))
                .then_with(|| a["id"].as_i64().cmp(&b["id"].as_i64()))
        });
        tasks
    }

    fn lane_head(self: &Rc<Self>, column: &str, shown: usize, total: usize, class: &str) -> gtk::Box {
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        head.add_css_class(class);
        head.append(&status_icon(column, 13));
        head.append(&label(&caption(column).to_uppercase(), "lane-title"));
        head.append(&label(
            &if shown == total { total.to_string() } else { format!("{shown} / {total}") },
            "lane-count",
        ));
        head.append(&spacer());
        if column != "done" {
            let add = crate::app::icon_button("plus", &format!("New task in {}", caption(column)));
            add.add_css_class("lane-key");
            let weak = Rc::downgrade(self);
            let name = column.to_string();
            add.connect_clicked(move |_| {
                if let Some(board) = weak.upgrade() {
                    board.open_quick(&name);
                }
            });
            head.append(&add);
        }
        let hide = crate::app::icon_button("minimize", &format!("Hide {}", caption(column)));
        hide.add_css_class("lane-key");
        let weak = Rc::downgrade(self);
        let name = column.to_string();
        hide.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                board.hidden.borrow_mut().insert(name.clone());
                glib::idle_add_local_once(move || board.render());
            }
        });
        head.append(&hide);
        head
    }

    fn lane_drop(self: &Rc<Self>, widget: &impl IsA<gtk::Widget>, column: &'static str) {
        let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        drop.connect_enter(|target, _, _| {
            if let Some(w) = target.widget() {
                w.add_css_class("drop-lane");
            }
            gdk::DragAction::MOVE
        });
        drop.connect_leave(|target| {
            if let Some(w) = target.widget() {
                w.remove_css_class("drop-lane");
            }
        });
        drop.connect_drop(move |target, token, _, _| {
            if let Some(w) = target.widget() {
                w.remove_css_class("drop-lane");
            }
            let (Some(board), Some(id)) = (weak.upgrade(), drop_id(token)) else {
                return false;
            };
            let Some(task) = board.task(id) else { return false };
            board.focused.set(Some(id));
            if text(&task, "column") == column {
                // Below the last card of its own lane: to the bottom, unless it is already there.
                let order = board.column_order(column);
                if column == "done" || order.last() == Some(&id) {
                    return false;
                }
                board.move_to(&task, column, Some(order.len() - 1));
                return true;
            }
            board.move_to(&task, column, None);
            true
        });
        widget.add_controller(drop);
    }

    /// The columns view, or the list view when `self.list` is set: one lane (or list section)
    /// per shown column, each with its head, drop target, empty note, group headings and
    /// cards (or rows).
    fn render_lanes(self: &Rc<Self>, tasks: &[Value], visible: &[&Value]) {
        let list = self.list.get();
        let outer = if list {
            let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
            rows.add_css_class("board-list");
            let scroll = crate::app::scrolled(&rows);
            scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
            self.scrolls.borrow_mut().push(("list".into(), scroll.vadjustment()));
            self.content.append(&scroll);
            rows
        } else {
            let grid = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            grid.add_css_class("board-grid");
            grid.set_homogeneous(true);
            let scroller = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .hexpand(true)
                .child(&grid)
                .build();
            self.scrolls.borrow_mut().push(("board".into(), scroller.hadjustment()));
            self.content.append(&scroller);
            grid
        };
        let grouping = self.group.borrow().clone();
        let hidden = self.hidden.borrow().clone();
        let all = by_id(tasks);
        if hidden.len() == COLUMNS.len() {
            let all = label("Every column is hidden. Turn one back on in the COLUMNS strip.", "board-empty");
            all.set_halign(gtk::Align::Center);
            outer.append(&all);
        }
        for &column in COLUMNS {
            if hidden.contains(column) {
                continue;
            }
            let total = tasks.iter().filter(|t| text(t, "column") == column).count();
            let shown = self.sorted(visible, column);
            // A list section holds its rows itself; a lane scrolls its cards under its head.
            let (lane, items) = if list {
                let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
                section.add_css_class("list-section");
                section.append(&self.lane_head(column, shown.len(), total, "list-head"));
                (section.clone(), section)
            } else {
                let lane = gtk::Box::new(gtk::Orientation::Vertical, 0);
                lane.add_css_class("board-lane");
                lane.add_css_class(&format!("lane-{column}"));
                lane.set_size_request(232, -1);
                lane.append(&self.lane_head(column, shown.len(), total, "lane-head"));
                let cards = gtk::Box::new(gtk::Orientation::Vertical, 6);
                cards.add_css_class("board-cards");
                let scroll = crate::app::scrolled(&cards);
                scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
                self.scrolls.borrow_mut().push((format!("lane:{column}"), scroll.vadjustment()));
                lane.append(&scroll);
                (lane, cards)
            };
            self.lane_drop(&lane, column);
            if shown.is_empty() {
                if list {
                    items.append(&label(if total > 0 { "No matches" } else { "All quiet" }, "list-empty"));
                } else {
                    items.append(&self.empty_lane(column, total > 0));
                }
            }
            let heading = if list { "list-group-heading" } else { "board-group-heading" };
            let mut placed = Vec::new();
            let mut previous: Option<String> = None;
            for task in shown {
                if !grouping.is_empty() {
                    let group = group_of(task, &grouping);
                    if previous.as_ref() != Some(&group) {
                        items.append(&label(&group_title(&group), heading));
                        previous = Some(group);
                    }
                }
                let item = if list { self.row(task, &all) } else { self.card(task, &all) };
                items.append(&item);
                placed.push((task["id"].as_i64().unwrap_or(0), item.upcast()));
            }
            self.layout.borrow_mut().push((column.to_string(), placed));
            outer.append(&lane);
        }
    }

    fn empty_lane(&self, column: &str, filtered: bool) -> gtk::Box {
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 4);
        empty.add_css_class("board-empty");
        let (title, hint) = if filtered {
            ("No matches", "Nothing here fits the search or filters.")
        } else {
            match column {
                "backlog" => ("All quiet", "Press C to capture a task."),
                "ready" => ("All quiet", "Drop a card here when it is ready to dispatch."),
                "active" => ("No work in flight", "Dispatched tasks land here."),
                "in_review" => ("Nothing to review", "Agents move finished work here."),
                _ => ("Nothing approved yet", "Drop a card here to approve it."),
            }
        };
        let t = label(title, "board-empty-title");
        t.set_halign(gtk::Align::Center);
        let h = label(hint, "board-empty-hint");
        h.set_halign(gtk::Align::Center);
        h.set_wrap(true);
        h.set_justify(gtk::Justification::Center);
        empty.append(&t);
        empty.append(&h);
        empty
    }

    fn card(self: &Rc<Self>, task: &Value, all: &BTreeMap<i64, &Value>) -> gtk::Box {
        let id = task["id"].as_i64().unwrap_or(0);
        let done = text(task, "column") == "done";
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("task-card");
        card.set_widget_name("task-card");
        card.set_focusable(true);
        if done {
            card.add_css_class("done");
        }
        if !task["parent_id"].is_null() {
            card.add_css_class("task-child");
        }
        card.update_property(&[gtk::accessible::Property::Label(&format!(
            "#{id} {} · {} · {}",
            text(task, "title"),
            text(task, "type"),
            text(task, "priority")
        ))]);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        top.add_css_class("card-top");
        let mark = task_mark(text(task, "type"), 9);
        mark.set_tooltip_text(Some(text(task, "type")));
        top.append(&mark);
        top.append(&label(&format!("#{id}"), "card-id"));
        if let Some(badge) = state_badge(text(task, "state")) {
            top.append(&badge);
        }
        top.append(&spacer());
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tools.add_css_class("card-tools");
        let copy = crate::app::icon_button("copy", "Copy title, body and id");
        let contents = format!("{}\n\n{}\n\n#{id}", text(task, "title"), text(task, "body"));
        copy.connect_clicked(move |key| key.clipboard().set_text(&contents));
        let peek = crate::app::icon_button("sidebar", "Peek (Space)");
        let weak = Rc::downgrade(self);
        peek.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                board.toggle_peek(id);
            }
        });
        let open = crate::app::icon_button("external", "Open (Enter)");
        open.set_widget_name("board-open");
        let weak = self.ui.clone();
        open.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                task_pages::open(&ui, id);
            }
        });
        for key in [&copy, &peek, &open] {
            key.add_css_class("card-key");
            tools.append(key);
        }
        top.append(&tools);
        top.append(&priority_icon(text(task, "priority")));
        card.append(&top);

        let title = label(text(task, "title"), "card-title");
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.set_lines(3);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_xalign(0.);
        title.set_tooltip_text(Some(text(task, "title")));
        card.append(&title);

        if let Some(parent) = task["_parent_title"].as_str() {
            let lineage = gtk::Box::new(gtk::Orientation::Horizontal, 3);
            lineage.add_css_class("card-lineage");
            lineage.append(&crate::icons::image("chevron-right", 10));
            let caption = label(parent, "task-lineage");
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            lineage.append(&caption);
            card.append(&lineage);
        }
        let total = task["rollup"]["total"].as_i64().unwrap_or(0);
        if total > 0 {
            let finished = task["rollup"]["done"].as_i64().unwrap_or(0);
            let rollup = gtk::Box::new(gtk::Orientation::Horizontal, 7);
            rollup.add_css_class("task-rollup");
            let meter = gtk::ProgressBar::new();
            meter.set_fraction((finished as f64 / total as f64).clamp(0., 1.));
            meter.set_hexpand(true);
            meter.set_valign(gtk::Align::Center);
            rollup.append(&meter);
            rollup.append(&label(&format!("{finished}/{total} sub-tasks"), "task-lineage"));
            card.append(&rollup);
        }
        let blockers = open_blockers(task, all);
        if !blockers.is_empty() && !done {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            line.add_css_class("card-blocked");
            line.append(&lamp("blocked-lamp"));
            let caption = label(
                &if blockers.len() == 1 {
                    format!("Blocked by #{} {}", blockers[0]["id"], text(blockers[0], "title"))
                } else {
                    format!("Blocked by {} tasks", blockers.len())
                },
                "card-blocked-caption",
            );
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&caption);
            card.append(&line);
        }
        if let Some(duplicate) = task["duplicate_of"].as_i64().and_then(|other| all.get(&other)) {
            let caption = label(&format!("Duplicate of #{} {}", duplicate["id"], text(duplicate, "title")), "task-lineage");
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            card.append(&caption);
        }
        if let Some(tags) = labels_row(task, 3) {
            card.append(&tags);
        }

        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        foot.add_css_class("card-foot");
        if let Some(module) = task["_module_name"].as_str() {
            let caption = label(module, "card-module");
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            caption.set_max_width_chars(16);
            foot.append(&caption);
        }
        if !text(task, "size").is_empty() {
            foot.append(&label(text(task, "size"), "task-chip"));
        }
        if let Some(commit) = task["commits"]
            .as_array()
            .and_then(|list| list.last())
            .and_then(|c| c["sha"].as_str())
        {
            let sha = label(&commit.chars().take(7).collect::<String>(), "card-sha");
            sha.set_tooltip_text(Some(commit));
            foot.append(&sha);
        }
        foot.append(&spacer());
        let when = label(&ago(text(task, "updated_at")), "card-ago");
        when.set_tooltip_text(Some(&format!("Updated {}", text(task, "updated_at"))));
        foot.append(&when);
        card.append(&foot);

        let agents: Vec<String> = rows(task, "sessions")
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        if !agents.is_empty() {
            let strip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            strip.add_css_class("task-agent");
            strip.append(&lamp(if text(task, "state") == "running" { "live-lamp" } else { "" }));
            let names = agents.join(" · ");
            let caption = label(&names, "");
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            caption.set_tooltip_text(Some(&names));
            strip.append(&caption);
            card.append(&strip);
        }
        self.wire(&card, id);
        card
    }

    fn row(self: &Rc<Self>, task: &Value, all: &BTreeMap<i64, &Value>) -> gtk::Box {
        let id = task["id"].as_i64().unwrap_or(0);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        row.add_css_class("list-row");
        row.set_widget_name("task-card");
        row.set_focusable(true);
        if text(task, "column") == "done" {
            row.add_css_class("done");
        }
        row.append(&priority_icon(text(task, "priority")));
        let number = label(&format!("#{id}"), "card-id");
        number.set_width_chars(5);
        row.append(&number);
        let mark = task_mark(text(task, "type"), 9);
        mark.set_tooltip_text(Some(text(task, "type")));
        row.append(&mark);
        let title = label(text(task, "title"), "list-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        title.set_tooltip_text(Some(text(task, "title")));
        if task["parent_id"].is_null() {
            row.append(&title);
        } else {
            let both = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            both.set_hexpand(true);
            title.set_hexpand(false);
            both.append(&title);
            if let Some(parent) = task["_parent_title"].as_str() {
                let lineage = label(&format!("in {parent}"), "list-lineage");
                lineage.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lineage.set_hexpand(true);
                both.append(&lineage);
            }
            row.append(&both);
        }
        let total = task["rollup"]["total"].as_i64().unwrap_or(0);
        if total > 0 {
            row.append(&label(
                &format!("{}/{total}", task["rollup"]["done"].as_i64().unwrap_or(0)),
                "list-meta",
            ));
        }
        if !open_blockers(task, all).is_empty() && text(task, "column") != "done" {
            let lamp = lamp("blocked-lamp");
            lamp.set_tooltip_text(Some("Blocked by an open task"));
            row.append(&lamp);
        }
        if let Some(tags) = labels_row(task, 2) {
            row.append(&tags);
        }
        if let Some(badge) = state_badge(text(task, "state")) {
            row.append(&badge);
        }
        if let Some(module) = task["_module_name"].as_str() {
            let caption = label(module, "list-meta");
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            caption.set_max_width_chars(14);
            row.append(&caption);
        }
        if let Some(agent) = rows(task, "sessions").last().and_then(|v| v.as_str().map(str::to_string)) {
            let caption = label(&agent, "list-agent");
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            caption.set_max_width_chars(12);
            row.append(&caption);
        }
        let when = label(&ago(text(task, "updated_at")), "card-ago");
        when.set_width_chars(6);
        when.set_xalign(1.);
        row.append(&when);
        self.wire(&row, id);
        row
    }

    /// Click, keyboard focus, context menu, drag and drop — shared by cards and list rows.
    fn wire(self: &Rc<Self>, widget: &gtk::Box, id: i64) {
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_enter(move |_| {
            if let Some(board) = weak.upgrade() {
                board.focused.set(Some(id));
            }
        });
        widget.add_controller(focus);

        let click = gtk::GestureClick::new();
        click.set_button(0);
        let weak = Rc::downgrade(self);
        click.connect_pressed(|gesture, _, _, _| {
            if let Some(widget) = gesture.widget() {
                widget.grab_focus();
            }
        });
        click.connect_released(move |gesture, presses, x, y| {
            let (Some(board), Some(widget)) = (weak.upgrade(), gesture.widget()) else {
                return;
            };
            let state = gesture.current_event_state();
            match gesture.current_button() {
                3 => board.menu(&widget, id, Some((x, y))),
                2 => board.toggle_peek(id),
                1 if state.contains(gdk::ModifierType::CONTROL_MASK) => board.toggle_peek(id),
                // While the peek panel is open a click retargets it and a double click opens;
                // otherwise one click opens, and the second press of a double click is ignored.
                1 if presses == 1 && board.peeked.get().is_some() => {
                    board.peeked.set(Some(id));
                    board.render_peek();
                }
                1 if presses == 1 || board.peeked.get().is_some() => {
                    if let Some(ui) = board.ui() {
                        task_pages::open(&ui, id);
                    }
                }
                _ => {}
            }
        });
        widget.add_controller(click);

        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::MOVE);
        let grab = Rc::new(Cell::new((0., 0.)));
        let at = grab.clone();
        drag.connect_prepare(move |_, x, y| {
            at.set((x, y));
            Some(gdk::ContentProvider::for_value(&format!("relay-task:{id}").to_value()))
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_begin(move |source, _| {
            let Some(board) = weak.upgrade() else { return };
            board.dragged.set(Some(id));
            board.rail.set_visible(true);
            if let Some(widget) = source.widget() {
                let (x, y) = grab.get();
                // A still image of the card: the live one fades with the `dragging` class.
                let image = gtk::WidgetPaintable::new(Some(&widget)).current_image();
                source.set_icon(Some(&image), x as i32, y as i32);
                widget.add_css_class("dragging");
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_end(move |source, _, _| {
            if let Some(widget) = source.widget() {
                widget.remove_css_class("dragging");
            }
            if let Some(board) = weak.upgrade() {
                board.dragged.set(None);
                board.rail.set_visible(false);
            }
        });
        widget.add_controller(drag);

        let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        let classes = ["drop-before", "drop-into", "drop-after"];
        let weak = Rc::downgrade(self);
        drop.connect_motion(move |target, _, y| {
            let Some(widget) = target.widget() else {
                return gdk::DragAction::empty();
            };
            let dragging_self = weak.upgrade().and_then(|b| b.dragged.get()) == Some(id);
            for class in classes {
                widget.remove_css_class(class);
            }
            if dragging_self {
                return gdk::DragAction::empty();
            }
            widget.add_css_class(match zone(y, widget.height() as f64) {
                Zone::Before => "drop-before",
                Zone::Into => "drop-into",
                Zone::After => "drop-after",
            });
            gdk::DragAction::MOVE
        });
        drop.connect_leave(move |target| {
            if let Some(widget) = target.widget() {
                for class in classes {
                    widget.remove_css_class(class);
                }
            }
        });
        let weak = Rc::downgrade(self);
        drop.connect_drop(move |target, token, _, y| {
            let Some(widget) = target.widget() else { return false };
            for class in classes {
                widget.remove_css_class(class);
            }
            let (Some(board), Some(moving)) = (weak.upgrade(), drop_id(token)) else {
                return false;
            };
            board.drop_on(moving, id, zone(y, widget.height() as f64))
        });
        widget.add_controller(drop);
    }

    fn menu(self: &Rc<Self>, anchor: &gtk::Widget, id: i64, at: Option<(f64, f64)>) {
        let Some(task) = self.task(id) else { return };
        let pop = gtk::Popover::new();
        pop.add_css_class("board-menu");
        pop.set_has_arrow(false);
        pop.set_parent(anchor);
        if let Some((x, y)) = at {
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        }
        pop.connect_closed(|pop| {
            let pop = pop.clone();
            glib::idle_add_local_once(move || {
                if pop.parent().is_some() {
                    pop.unparent();
                }
            });
        });
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        menu.set_size_request(260, -1);
        let heading = label(&format!("#{id} {}", text(&task, "title")), "board-menu-title");
        heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
        heading.set_max_width_chars(34);
        menu.append(&heading);
        let item = |icon: &str, caption: &str, key: &str| {
            let b = button("", "board-menu-item");
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            content.append(&crate::icons::image(icon, 12));
            content.append(&label(caption, ""));
            content.append(&spacer());
            if !key.is_empty() {
                content.append(&label(key, "board-kbd"));
            }
            b.set_child(Some(&content));
            b
        };
        let weak = Rc::downgrade(self);
        let close = pop.downgrade();
        let run = move |f: Action| {
            let weak = weak.clone();
            let close = close.clone();
            move |_: &gtk::Button| {
                if let Some(pop) = close.upgrade() {
                    pop.popdown();
                }
                if let Some(board) = weak.upgrade() {
                    f(&board);
                }
            }
        };
        let open = item("external", "Open", "↵");
        open.connect_clicked(run(Box::new(move |board| {
            if let Some(ui) = board.ui() {
                task_pages::open(&ui, id);
            }
        })));
        menu.append(&open);
        let peek = item("sidebar", "Peek", "Space");
        peek.connect_clicked(run(Box::new(move |board| board.toggle_peek(id))));
        menu.append(&peek);

        let section = |title: &str| label(title, "board-menu-caption");
        menu.append(&section("MOVE TO"));
        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        columns.set_homogeneous(true);
        for (column, title) in COLUMN_TITLES {
            let key = button("", "board-menu-choice");
            key.set_child(Some(&status_icon(column, 13)));
            key.set_tooltip_text(Some(if column == "done" {
                "Done · approves the task and links the last agent's branch tip (the project HEAD if never dispatched)"
            } else {
                title
            }));
            if column == text(&task, "column") {
                key.add_css_class("selected");
                key.set_sensitive(false);
            }
            let moving = task.clone();
            key.connect_clicked(run(Box::new(move |board| {
                board.focused.set(Some(id));
                board.move_to(&moving, column, None);
            })));
            columns.append(&key);
        }
        menu.append(&columns);
        menu.append(&section("PRIORITY"));
        let priorities = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        priorities.set_homogeneous(true);
        for priority in URGENT_FIRST {
            let key = button("", "board-menu-choice");
            key.set_child(Some(&priority_icon(priority)));
            if priority == text(&task, "priority") {
                key.add_css_class("selected");
            }
            key.connect_clicked(run(Box::new(move |board| {
                board.set_field(id, "priority", json!(priority));
            })));
            priorities.append(&key);
        }
        menu.append(&priorities);
        menu.append(&section("TYPE"));
        let kinds = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        kinds.set_homogeneous(true);
        for kind in TYPES {
            let key = button("", "board-menu-choice");
            key.set_child(Some(&task_mark(kind, 9)));
            key.set_tooltip_text(Some(kind));
            if kind == text(&task, "type") {
                key.add_css_class("selected");
            }
            key.connect_clicked(run(Box::new(move |board| {
                board.set_field(id, "type", json!(kind));
            })));
            kinds.append(&key);
        }
        menu.append(&kinds);
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let copy = item("copy", "Copy title, body and id", "");
        let contents = format!("{}\n\n{}\n\n#{id}", text(&task, "title"), text(&task, "body"));
        copy.connect_clicked(move |key| {
            key.clipboard().set_text(&contents);
            if let Some(pop) = key.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>() {
                pop.popdown();
            }
        });
        menu.append(&copy);
        if !task["parent_id"].is_null() {
            let detach = item("arrow-left", "Detach from parent", "");
            detach.connect_clicked(run(Box::new(move |board| {
                board.act("task.parent.set", json!({"task_id":id,"parent_id":null}));
            })));
            menu.append(&detach);
        }
        let delete = item("trash", "Delete task", "");
        delete.add_css_class("destructive");
        delete.set_tooltip_text(Some("Ctrl+Z on the board restores it"));
        crate::app::confirm_inline(&delete, "Delete this task?", run(Box::new(move |board| {
            if board.peeked.get() == Some(id) {
                board.peeked.set(None);
            }
            board.act("task.delete", json!({"task_id":id}));
        })));
        menu.append(&delete);
        pop.set_child(Some(&menu));
        pop.popup();
    }

    /// A property row in the peek panel whose value opens a picker of `values`.
    fn picker(
        self: &Rc<Self>,
        values: &[&'static str],
        current: &str,
        icon: fn(&str) -> Option<gtk::Widget>,
        pick: impl Fn(&Rc<Board>, &'static str) + Clone + 'static,
    ) -> gtk::MenuButton {
        let name = |value: &str| -> String {
            if let Some((_, title)) = COLUMN_TITLES.iter().find(|(c, _)| *c == value) {
                title.to_string()
            } else if value.is_empty() {
                "None".into()
            } else {
                let mut chars = value.chars();
                chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
            }
        };
        let content = |value: &str| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
            if let Some(icon) = icon(value) {
                row.append(&icon);
            }
            row.append(&label(&name(value), ""));
            row
        };
        let menu = gtk::MenuButton::new();
        menu.add_css_class("peek-value");
        menu.set_child(Some(&content(current)));
        let pop = gtk::Popover::new();
        pop.add_css_class("board-menu");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 1);
        for &value in values {
            let key = button("", "board-menu-item");
            key.set_child(Some(&content(value)));
            if value == current {
                key.add_css_class("selected");
            }
            let weak = Rc::downgrade(self);
            let close = pop.downgrade();
            let pick = pick.clone();
            key.connect_clicked(move |_| {
                if let Some(pop) = close.upgrade() {
                    pop.popdown();
                }
                if let Some(board) = weak.upgrade() {
                    pick(&board, value);
                }
            });
            list.append(&key);
        }
        pop.set_child(Some(&list));
        menu.set_popover(Some(&pop));
        menu
    }

    /// The peeked task and the tasks it links to, which is everything the peek draws.
    fn peek_key(&self) -> Option<String> {
        let id = self.peeked.get()?;
        let tasks = self.tasks.borrow();
        let task = tasks.iter().find(|t| t["id"].as_i64() == Some(id))?;
        let listed = |key: &str, other: &Value| task[key].as_array().is_some_and(|ids| ids.contains(&other["id"]));
        let linked: Vec<&Value> = tasks
            .iter()
            .filter(|t| t["id"] == task["parent_id"] || t["parent_id"] == task["id"] || listed("blocked_by", t) || listed("blocks", t))
            .collect();
        Some(json!([task, linked]).to_string())
    }

    fn render_peek(self: &Rc<Self>) {
        let key = self.peek_key();
        if key.is_some() && *self.peek_key.borrow() == key && self.peek.first_child().is_some() {
            return;
        }
        // The same task redrawn keeps its place in a long description.
        let same = key.is_some() && self.peek_shown.replace(self.peeked.get()) == self.peeked.get();
        let kept = same
            .then(|| self.peek.last_child().and_downcast::<gtk::ScrolledWindow>())
            .flatten()
            .map(|scroll| scroll.vadjustment().value());
        *self.peek_key.borrow_mut() = key;
        clear(&self.peek);
        let Some(id) = self.peeked.get() else {
            self.peek.set_visible(false);
            return;
        };
        let Some(task) = self.task(id) else {
            self.peeked.set(None);
            self.peek.set_visible(false);
            return;
        };
        let tasks = self.tasks.borrow().clone();
        self.peek.set_visible(true);
        let column = text(&task, "column").to_string();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        head.add_css_class("peek-head");
        head.append(&status_icon(&column, 13));
        head.append(&label(&caption(&column).to_uppercase(), "lane-title"));
        head.append(&label(&format!("#{id}"), "card-id"));
        head.append(&spacer());
        let open = crate::app::icon_button("external", "Open the full editor (Enter)");
        let weak = self.ui.clone();
        open.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                task_pages::open(&ui, id);
            }
        });
        head.append(&open);
        let close = crate::app::icon_button("close", "Close (Esc)");
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                board.peeked.set(None);
                board.render_peek();
                // The focused key went with the peek.
                board.focus_board();
            }
        });
        head.append(&close);
        self.peek.append(&head);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.add_css_class("peek-body");
        let title = label(text(&task, "title"), "peek-title");
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.set_selectable(true);
        body.append(&title);

        let grid = gtk::Grid::new();
        grid.add_css_class("peek-props");
        grid.set_row_spacing(2);
        grid.set_column_spacing(12);
        let mut row = 0;
        let mut prop = |name: &str, value: &gtk::Widget| {
            let key = label(name, "peek-key");
            key.set_valign(gtk::Align::Center);
            grid.attach(&key, 0, row, 1, 1);
            value.set_halign(gtk::Align::Start);
            value.set_hexpand(true);
            grid.attach(value, 1, row, 1, 1);
            row += 1;
        };
        let moving = task.clone();
        prop(
            "Status",
            self.picker(
                COLUMNS,
                &column,
                |c| Some(status_icon(c, 13).upcast()),
                move |board, c| board.move_to(&moving, c, None),
            )
            .upcast_ref(),
        );
        prop(
            "Priority",
            self.picker(&URGENT_FIRST, text(&task, "priority"), |p| Some(priority_icon(p).upcast()), move |board, p| {
                board.set_field(id, "priority", json!(p));
            })
            .upcast_ref(),
        );
        prop(
            "Type",
            self.picker(&TYPES, text(&task, "type"), |t| Some(task_mark(t, 9).upcast()), move |board, t| {
                board.set_field(id, "type", json!(t));
            })
            .upcast_ref(),
        );
        prop(
            "Size",
            self.picker(&SIZES, text(&task, "size"), |_| None, move |board, s| {
                let size = if s.is_empty() { Value::Null } else { json!(s) };
                board.set_field(id, "size", size);
            })
            .upcast_ref(),
        );
        let value = |caption: &str| label(caption, "peek-text").upcast::<gtk::Widget>();
        prop(
            "State",
            &state_badge(text(&task, "state"))
                .map(|b| b.upcast::<gtk::Widget>())
                .unwrap_or_else(|| value("Idle")),
        );
        if let Some(module) = task["_module_name"].as_str() {
            prop("Module", &value(module));
        }
        let agents: Vec<String> = rows(&task, "sessions").iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
        prop("Agents", &value(&if agents.is_empty() { "Unassigned".into() } else { agents.join(" · ") }));
        if let Some(parent) = task["parent_id"].as_i64().and_then(|p| tasks.iter().find(|t| t["id"].as_i64() == Some(p))) {
            prop("Parent", self.task_link(parent).upcast_ref());
        }
        prop("Updated", &value(&since(text(&task, "updated_at"))));
        prop("Created", &value(&since(text(&task, "created_at"))));
        body.append(&grid);

        if let Some(tags) = labels_row(&task, 12) {
            tags.add_css_class("peek-labels");
            body.append(&tags);
        }
        body.append(&label("DESCRIPTION", "board-menu-caption"));
        let description = text(&task, "body").trim();
        let about = paragraph(if description.is_empty() { "No description." } else { description });
        about.add_css_class(if description.is_empty() { "peek-muted" } else { "peek-description" });
        about.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        body.append(&about);

        let children: Vec<&Value> = tasks.iter().filter(|t| t["parent_id"] == task["id"]).collect();
        if !children.is_empty() {
            let finished = children.iter().filter(|t| text(t, "column") == "done").count();
            body.append(&label(&format!("SUB-TASKS  {finished}/{}", children.len()), "board-menu-caption"));
            for child in children {
                body.append(&self.task_link(child));
            }
        }
        for (key, title) in [("blocked_by", "BLOCKED BY"), ("blocks", "BLOCKS")] {
            let linked: Vec<&Value> = rows(&task, key)
                .iter()
                .filter_map(|other| tasks.iter().find(|t| t["id"] == *other))
                .collect();
            if !linked.is_empty() {
                body.append(&label(title, "board-menu-caption"));
                for other in linked {
                    body.append(&self.task_link(other));
                }
            }
        }
        let commits = rows(&task, "commits");
        if !commits.is_empty() {
            body.append(&label("COMMITS", "board-menu-caption"));
            for commit in commits {
                let line = label(
                    &format!(
                        "{}  {}",
                        text(&commit, "sha").chars().take(10).collect::<String>(),
                        text(&commit, "branch")
                    ),
                    "peek-commit",
                );
                line.set_selectable(true);
                body.append(&line);
            }
        }
        let edit = button("Open full editor", "quiet");
        edit.add_css_class("peek-edit");
        let weak = self.ui.clone();
        edit.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                task_pages::open(&ui, id);
            }
        });
        body.append(&edit);
        let scroll = crate::app::scrolled(&body);
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        if let Some(value) = kept.filter(|v| *v > 0.) {
            let done = Cell::new(false);
            scroll.vadjustment().connect_changed(move |a| {
                if !done.get() && a.upper() - a.page_size() >= value {
                    done.set(true);
                    a.set_value(value);
                }
            });
        }
        self.peek.append(&scroll);
    }

    fn task_link(self: &Rc<Self>, task: &Value) -> gtk::Button {
        let id = task["id"].as_i64().unwrap_or(0);
        let key = button("", "peek-link");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        content.append(&status_icon(text(task, "column"), 12));
        content.append(&label(&format!("#{id}"), "card-id"));
        let title = label(text(task, "title"), "");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        content.append(&title);
        key.set_child(Some(&content));
        if text(task, "column") == "done" {
            key.add_css_class("done");
        }
        let weak = Rc::downgrade(self);
        key.connect_clicked(move |_| {
            if let Some(board) = weak.upgrade() {
                board.peeked.set(Some(id));
                board.focused.set(Some(id));
                board.render_peek();
                board.focus_board();
            }
        });
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn filters(pairs: &[(&str, &str)]) -> Filters {
        let mut map = Filters::new();
        for (k, v) in pairs {
            map.entry(k.to_string()).or_default().insert(v.to_string());
        }
        map
    }
    #[test]
    fn lens_combines_metadata_and_text_without_matching_unrelated_fields() {
        let task = json!({"id":9,"title":"Fix notes","body":"Keep drafts","type":"bug","priority":"high","size":"M","parent_id":2,"module_id":3,"labels":["native"],"sessions":["egret"]});
        assert!(task_matches(
            &task,
            &filters(&[("type", "bug"), ("label", "native"), ("session", "egret"), ("parent", "2")]),
            "drafts"
        ));
        assert!(!task_matches(&task, &filters(&[("parent", "roots")]), ""));
        assert!(!task_matches(&task, &filters(&[("module", "4")]), ""));
        assert!(!task_matches(&task, &Filters::new(), "high"));
        // Several values of one key are alternatives; keys still all have to match.
        assert!(task_matches(&task, &filters(&[("type", "bug"), ("type", "chore")]), ""));
        assert!(!task_matches(&task, &filters(&[("type", "bug"), ("priority", "low")]), ""));
        assert!(task_matches(&task, &Filters::new(), "#9"));
    }
    #[test]
    fn drops_beside_a_card_compute_its_final_index() {
        let order = [1, 2, 3, 4];
        // Down the column: the moving card is not counted among the others.
        assert_eq!(drop_index(&order, 1, 3, false), 1);
        assert_eq!(drop_index(&order, 1, 3, true), 2);
        // Up the column.
        assert_eq!(drop_index(&order, 4, 2, false), 1);
        // From another column.
        assert_eq!(drop_index(&order, 9, 4, true), 4);
        assert_eq!(drop_index(&order, 9, 7, false), 4);
        assert_eq!(zone(1., 100.), Zone::Before);
        assert_eq!(zone(50., 100.), Zone::Into);
        assert_eq!(zone(90., 100.), Zone::After);
    }
    #[test]
    fn ages_read_at_a_glance() {
        // The wording itself is relative.rs's; the board owns what an unreadable time shows.
        let minutes = |m: i32| glib::DateTime::now_utc().unwrap().add_minutes(-m).unwrap().format_iso8601().unwrap().to_string();
        assert_eq!(ago(&minutes(45)), "45m");
        assert_eq!(ago(""), "");
        assert_eq!(since(&minutes(0)), "just now");
        assert_eq!(since(&minutes(45)), "45m ago");
        assert_eq!(since(""), "—");
    }
    #[test]
    fn derived_orders_follow_the_one_vocabulary() {
        assert_eq!(URGENT_FIRST, ["urgent", "high", "medium", "low"]);
        assert_eq!(COLUMNS, COLUMN_TITLES.map(|(c, _)| c));
        assert_eq!(OPEN_COLUMNS, &COLUMNS[..COLUMNS.len() - 1]);
        assert!(!OPEN_COLUMNS.contains(&"done"));
    }
    #[test]
    fn groups_sort_in_their_own_order_with_fallbacks_last() {
        let sizes: Vec<String> = ["L", "", "S", "M"]
            .iter()
            .map(|s| group_of(&json!({"size": s}), "size"))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(sizes.iter().map(|g| group_title(g)).collect::<Vec<_>>(), ["S", "M", "L", "None"]);
        assert!(group_of(&json!({"sessions":[]}), "session") > group_of(&json!({"sessions":["zebra"]}), "session"));
        assert_eq!(group_title(&group_of(&json!({}), "module")), "No module");
    }
}
