//! Native lifecycle regressions. Invoked only by the isolated fixture harness.
use crate::app::Ui;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::json;
use std::{rc::Rc, time::Duration};

fn named(root: &impl IsA<gtk::Widget>, name: &str) -> Option<gtk::Widget> {
    let root = root.as_ref();
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = named(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}
async fn wait_for(mut predicate: impl FnMut() -> bool, reason: &str) -> Result<(), String> {
    for _ in 0..160 {
        if predicate() {
            return Ok(());
        }
        glib::timeout_future(Duration::from_millis(25)).await;
    }
    Err(format!("Timed out: {reason}"))
}
fn require(condition: bool, reason: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(reason.to_string())
    }
}
pub async fn run(ui: &Rc<Ui>) -> Result<(), String> {
    let project = ui.project.get();
    require(project != 0, "Fixture project must be ready")?;
    let note = ui.call("notes.create",json!({"project_id":project,"title":"Plan","body":"Ordinary Plan fixture","pinned":true})).await.map_err(|e|e.to_string())?;
    let id = note["id"].as_i64().ok_or("Missing fixture note id")?;
    crate::pages::open_note(ui, note);
    let owner = ui
        .notes_window
        .borrow()
        .as_ref()
        .ok_or("Notes window missing")?
        .clone();
    wait_for(
        || {
            owner.rendered_project.get() == project
                && named(&owner.window, "notes-library-split").is_some()
        },
        "Notes library loaded",
    )
    .await?;
    require(
        owner.window.is_decorated()
            && owner.window.is_resizable()
            && owner
                .window
                .titlebar()
                .is_some_and(|bar| bar.is::<gtk::WindowHandle>()),
        "Notes must retain native resize edges around its custom titlebar",
    )?;
    wait_for(
        || owner.window.is_mapped() && owner.window.width() > 120 && owner.window.height() > 80,
        "Notes window allocated",
    )
    .await?;
    let initial_size = (owner.window.width(), owner.window.height());
    println!("NOTES_RESIZE_INITIAL={}x{}", initial_size.0, initial_size.1);
    owner
        .window
        .set_default_size(initial_size.0 - 120, initial_size.1 - 80);
    wait_for(
        || owner.window.width() < initial_size.0 && owner.window.height() < initial_size.1,
        "Notes window shrank",
    )
    .await?;
    owner
        .window
        .set_default_size(initial_size.0, initial_size.1);
    wait_for(
        || owner.window.width() == initial_size.0 && owner.window.height() == initial_size.1,
        "Notes window grew back",
    )
    .await?;
    let draft = ui
        .note_drafts
        .borrow()
        .get(&id)
        .ok_or("Fixture draft missing")?
        .clone();
    let title = named(&draft.layout, "note-title")
        .ok_or("Plan must have a title editor")?
        .downcast::<gtk::Entry>()
        .map_err(|_| "Note title type")?;
    require(
        title.is_editable(),
        "An existing Plan document must remain an ordinary editable note",
    )?;
    let body = named(&draft.layout, "note-body")
        .ok_or("Note body missing")?
        .downcast::<gtk::TextView>()
        .map_err(|_| "Note body type")?;
    let source = body
        .clone()
        .downcast::<sourceview5::View>()
        .map_err(|_| "Note body must be a GtkSourceView")?;
    require(
        source
            .buffer()
            .downcast::<sourceview5::Buffer>()
            .ok()
            .and_then(|buffer| sourceview5::prelude::BufferExt::language(&buffer))
            .is_some_and(|language| language.id() == "markdown"),
        "Note body must use the Markdown language",
    )?;
    owner
        .window
        .activate_action("notes.find", None)
        .map_err(|_| "Find action missing")?;
    let query = named(&draft.layout, "notes-find-entry")
        .ok_or("Find bar missing")?
        .downcast::<gtk::Entry>()
        .map_err(|_| "Find entry type")?;
    require(query.is_mapped(), "Ctrl+F did not show the find bar")?;
    query.set_text("plan");
    let count = named(&draft.layout, "notes-find-count")
        .ok_or("Find count missing")?
        .downcast::<gtk::Label>()
        .map_err(|_| "Find count type")?;
    wait_for(|| count.text() == "1 of 1", "Find shows a match count").await?;
    owner
        .window
        .activate_action("notes.close-find", None)
        .map_err(|_| "Close-find action missing")?;
    require(!query.is_mapped(), "Escape did not close the find bar")?;
    owner
        .window
        .activate_action("notes.goto", None)
        .map_err(|_| "Go-to-line action missing")?;
    let goto = named(&draft.layout, "notes-goto-entry")
        .ok_or("Go-to-line field missing")?
        .downcast::<gtk::Entry>()
        .map_err(|_| "Go-to-line field type")?;
    goto.set_text("1:10");
    goto.emit_activate();
    require(
        body.buffer().cursor_position() == 9,
        "Go to line 1:10 did not move the cursor",
    )?;
    body.buffer()
        .set_text("Unsaved text survives the window closing.");
    owner.window.close();
    require(
        !owner.window.is_visible(),
        "Close must hide Notes even with a dirty draft",
    )?;
    require(
        ui.note_drafts.borrow().contains_key(&id),
        "Window close discarded a draft",
    )?;
    glib::timeout_future(Duration::from_millis(850)).await;
    let stored = ui
        .call("notes.get", json!({"note_id":id}))
        .await
        .map_err(|e| e.to_string())?;
    require(
        stored["body"] == "Ordinary Plan fixture",
        "Plan was silently autosaved",
    )?;
    crate::pages::show_notes(ui);
    require(
        Rc::ptr_eq(&owner, ui.notes_window.borrow().as_ref().unwrap()),
        "Reopen created a second Notes owner",
    )?;
    require(owner.window.is_visible(), "Reopen did not present Notes")?;
    require(
        body.buffer()
            .text(
                &body.buffer().start_iter(),
                &body.buffer().end_iter(),
                false,
            )
            .as_str()
            == "Unsaved text survives the window closing.",
        "Reopen replaced dirty editor contents",
    )?;
    let split = named(&owner.window, "notes-library-split")
        .unwrap()
        .downcast::<gtk::Paned>()
        .map_err(|_| "Library split type")?;
    split.set_position(331);
    let toggle = named(&owner.window, "notes-library-toggle")
        .ok_or("Library collapse control missing")?
        .downcast::<gtk::Button>()
        .map_err(|_| "Library toggle type")?;
    toggle.emit_clicked();
    require(owner.rail_collapsed.get(), "Library did not collapse")?;
    let renders = owner.renders.get();
    crate::pages::refresh_notes(ui);
    wait_for(|| owner.renders.get() > renders, "Notes refresh completed").await?;
    require(
        named(&owner.window, "notes-library-split")
            .is_some_and(|same| same == split.clone().upcast::<gtk::Widget>()),
        "Refresh rebuilt the Notes shell instead of keeping it",
    )?;
    require(
        owner.rail_collapsed.get(),
        "Refresh reset collapsed library",
    )?;
    require(owner.rail_width.get() == 331, "Refresh reset library width")?;
    owner.window.close();
    crate::pages::show_notes(ui);
    require(
        owner.rail_collapsed.get() && owner.rail_width.get() == 331,
        "Reopen reset rail state",
    )?;
    glib::timeout_future(Duration::from_millis(500)).await;
    let settings = ui
        .call("settings.get", json!({"path":"native.notes.window"}))
        .await
        .map_err(|e| e.to_string())?;
    require(
        settings["value"]["rail_width"] == 331 && settings["value"]["rail_collapsed"] == true,
        "Rail state was not persisted",
    )?;
    body.buffer().set_text("Ordinary Plan fixture");
    draft.close();
    require(
        !ui.note_drafts.borrow().contains_key(&id),
        "Clean tab close retained stale draft",
    )?;
    crate::pages::refresh_notes(ui);
    glib::timeout_future(Duration::from_millis(200)).await;
    require(
        !ui.note_drafts.borrow().contains_key(&id),
        "Refresh reopened a deliberately closed tab",
    )?;
    let task = ui
        .call(
            "task.create",
            json!({"project_id":project,"title":"Task message close guard"}),
        )
        .await
        .map_err(|e| e.to_string())?;
    let task_id = task["id"].as_i64().ok_or("Task id missing")?;
    ui.navigate("board");
    crate::pages::open_task(ui, task_id);
    wait_for(
        || named(&ui.window, "task-message").is_some(),
        "Task full-page details loaded",
    )
    .await?;
    let panel = ui
        .panels
        .borrow()
        .last()
        .ok_or("Task did not open a page")?
        .clone();
    let message = named(&ui.window, "task-message")
        .unwrap()
        .downcast::<gtk::TextView>()
        .map_err(|_| "Task message type")?;
    message
        .buffer()
        .set_text("Unsent fixture message. Do not transmit.");
    panel.close();
    require(
        ui.panels
            .borrow()
            .iter()
            .any(|candidate| Rc::ptr_eq(candidate, &panel)),
        "Closing task lost unsent message text",
    )?;
    message.buffer().set_text("");
    panel.close();
    require(
        !ui.panels
            .borrow()
            .iter()
            .any(|candidate| Rc::ptr_eq(candidate, &panel)),
        "Cleared message kept task close blocked",
    )?;
    ui.call("task.delete", json!({"task_id":task_id}))
        .await
        .map_err(|e| e.to_string())?;
    ui.call("notes.delete", json!({"note_id":id}))
        .await
        .map_err(|e| e.to_string())?;
    println!("NOTES_TASK_LIFECYCLE_OK: custom titlebar, resize, retained window, sourceview markdown, find count, go to line, dirty reopen, persistent shell, rail persistence, ordinary Plan, task message guard");
    Ok(())
}
