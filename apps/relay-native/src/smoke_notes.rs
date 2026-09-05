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
    require(
        std::env::var("RELAY_NATIVE_FIXTURE").as_deref() == Ok("1")
            && std::env::var("RELAY_INSTANCE").as_deref() == Ok("test"),
        "Notes/task lifecycle smoke requires the isolated test instance and fixture flag",
    )?;
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
    crate::pages::refresh_notes(ui);
    wait_for(
        || {
            named(&owner.window, "notes-library-split")
                .is_some_and(|new| new != split.clone().upcast::<gtk::Widget>())
        },
        "Notes refresh completed",
    )
    .await?;
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
    println!("NOTES_TASK_LIFECYCLE_OK: custom titlebar, resize, retained window, dirty reopen, rail persistence, ordinary Plan, task message guard");
    Ok(())
}
