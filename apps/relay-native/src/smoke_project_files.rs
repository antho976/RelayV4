//! Project files regression, limited to the disposable native fixture.
use crate::app::Ui;
use gtk4 as gtk;
use serde_json::json;
use sourceview5::prelude::*;
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
fn require(condition: bool, reason: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(reason.into())
    }
}
async fn wait_for(mut predicate: impl FnMut() -> bool, reason: &str) -> Result<(), String> {
    println!("PROJECT_WAIT={reason}");
    for _ in 0..160 {
        if predicate() {
            println!("PROJECT_READY={reason}");
            return Ok(());
        }
        glib::timeout_future(Duration::from_millis(25)).await;
    }
    Err(format!("Timed out: {reason}"))
}
fn click(ui: &Ui, name: &str) -> Result<(), String> {
    println!("PROJECT_CLICK={name}");
    let key = named(&ui.window, name)
        .ok_or_else(|| format!("Missing control: {name}"))?
        .downcast::<gtk::Button>()
        .map_err(|_| format!("Not a button: {name}"))?;
    require(key.is_sensitive(), name)?;
    key.emit_clicked();
    Ok(())
}
async fn file_action(
    ui: &Rc<Ui>,
    op: &str,
    path: Option<&str>,
    folder: bool,
) -> Result<(), String> {
    click(ui, &format!("project-{op}"))?;
    wait_for(
        || named(&ui.window, "panel-accept").is_some(),
        "File action panel",
    )
    .await?;
    if let Some(path) = path {
        named(&ui.window, "project-file-path")
            .ok_or("File path missing")?
            .downcast::<gtk::Entry>()
            .map_err(|_| "File path type")?
            .set_text(path);
    }
    if folder {
        named(&ui.window, "project-file-folder")
            .ok_or("Folder control missing")?
            .downcast::<gtk::CheckButton>()
            .map_err(|_| "Folder control type")?
            .set_active(true);
    }
    click(ui, "panel-accept")?;
    wait_for(
        || named(&ui.window, "panel-accept").is_none(),
        "File action closes",
    )
    .await
}
pub async fn run(ui: &Rc<Ui>) -> Result<(), String> {
    require(
        std::env::var("RELAY_NATIVE_FIXTURE").as_deref() == Ok("1")
            && std::env::var("RELAY_INSTANCE").as_deref() == Ok("test"),
        "Project smoke requires isolated test fixture",
    )?;
    let project = ui.project.get();
    require(project > 0, "Fixture project ready")?;
    let viewport = ui.window.width();
    ui.navigate("agents");
    ui.editor.prepare_project(ui);
    ui.editor.show_files();
    wait_for(
        || named(&ui.window, "project-file:README.md").is_some(),
        "Primary file tree",
    )
    .await?;
    click(ui, "project-file:README.md")?;
    let view = named(&ui.window, "project-source")
        .ok_or("Source view missing")?
        .downcast::<sourceview5::View>()
        .map_err(|_| "Source view type")?;
    wait_for(|| view.is_editable(), "File loaded").await?;
    let buffer = view.buffer();
    buffer.insert_at_cursor("\nProject view edit preserved.\n");
    let snapshot = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
    click(ui, "project-agents")?;
    require(ui.editor.agents_visible(), "Agents return action")?;
    require(
        buffer.is_modified(),
        "Agents return must retain unsaved edits",
    )?;
    click(ui, "project-editor")?;
    require(!ui.editor.agents_visible(), "Editor return action")?;
    require(
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) == snapshot,
        "Editor returns to same document",
    )?;
    click(ui, "project-git")?;
    glib::timeout_future(Duration::from_millis(80)).await;
    require(
        ui.window.width() == viewport,
        "Project rails expanded the window",
    )?;
    require(
        view.buffer() == buffer,
        "Git rail must not replace editor buffer",
    )?;
    click(ui, "project-save")?;
    wait_for(
        || !buffer.is_modified() && view.is_editable(),
        "Project save",
    )
    .await?;
    let saved = ui
        .call(
            "file.read",
            json!({"project_id":project,"path":"README.md"}),
        )
        .await
        .map_err(|e| e.to_string())?;
    require(
        saved["text"]
            .as_str()
            .is_some_and(|text| text.contains("Project view edit preserved.")),
        "Project save reaches engine",
    )?;
    // Let prior file events settle, then a burst should yield one tree reconstruction.
    glib::timeout_future(Duration::from_millis(1250)).await;
    let revision = ui.editor.tree_revision.get();
    for _ in 0..20 {
        ui.editor.invalidate(ui, None);
    }
    glib::timeout_future(Duration::from_millis(250)).await;
    require(
        ui.editor.tree_revision.get() == revision,
        "File events must not rebuild multiple times per second",
    )?;
    glib::timeout_future(Duration::from_millis(1000)).await;
    require(
        ui.editor.tree_revision.get() == revision + 1,
        "File events coalesce into one tree refresh",
    )?;
    glib::timeout_future(Duration::from_millis(1100)).await;
    require(
        ui.editor.tree_revision.get() == revision + 1,
        "File tree must not poll when idle",
    )?;
    file_action(ui, "file.create", Some("project-smoke.txt"), false).await?;
    wait_for(
        || named(&ui.window, "project-file:project-smoke.txt").is_some() && view.is_editable(),
        "File created and editable",
    )
    .await?;
    file_action(ui, "file.create", Some("project-smoke-folder"), true).await?;
    wait_for(
        || named(&ui.window, "project-file:project-smoke-folder").is_some(),
        "Folder created",
    )
    .await?;
    let folder =
        named(&ui.window, "project-file:project-smoke-folder").ok_or("Folder row missing")?;
    let controllers = folder.observe_controllers();
    let target = (0..controllers.n_items())
        .find_map(|i| {
            controllers
                .item(i)
                .and_then(|item| item.downcast::<gtk::DropTarget>().ok())
        })
        .ok_or("Folder drop target missing")?;
    let data = json!({"project":project,"worktree":"","path":"project-smoke.txt"}).to_string();
    let boxed = glib::BoxedValue(data.to_value());
    require(
        target.emit_by_name::<bool>("drop", &[&boxed, &0_f64, &0_f64]),
        "Folder accepts scoped file move",
    )?;
    wait_for(
        || named(&ui.window, "project-file:project-smoke.txt").is_none(),
        "Moved file removed from root",
    )
    .await?;
    let folder = named(&ui.window, "project-file:project-smoke-folder")
        .ok_or("Refreshed folder missing")?
        .downcast::<gtk::Button>()
        .map_err(|_| "Folder row type")?;
    // Explorer folders are rows that toggle their children, as in VS Code.
    folder.emit_clicked();
    wait_for(
        || {
            named(
                &ui.window,
                "project-file:project-smoke-folder/project-smoke.txt",
            )
            .is_some()
        },
        "Lazy child after move",
    )
    .await?;
    click(ui, "project-file:project-smoke-folder/project-smoke.txt")?;
    wait_for(|| view.is_editable(), "Moved file open").await?;
    file_action(ui, "file.rename", Some("renamed.txt"), false).await?;
    wait_for(
        || {
            named(&ui.window, "project-file:project-smoke-folder/renamed.txt").is_some()
                && view.is_editable()
        },
        "File rename",
    )
    .await?;
    file_action(ui, "file.delete", None, false).await?;
    wait_for(
        || named(&ui.window, "project-file-undo").is_some(),
        "Undo trash survives tree refresh",
    )
    .await?;
    click(ui, "project-file-undo")?;
    wait_for(
        || named(&ui.window, "project-file:project-smoke-folder/renamed.txt").is_some(),
        "Trash undo restores file",
    )
    .await?;
    ui.editor.show_agents();
    require(
        ui.page.borrow().as_str() == "agents",
        "File operations stay in project view",
    )?;
    image_preview(ui).await?;
    println!(
        "Project Files/Git/Agents, retained draft, save, create, folder, scoped move, rename, trash/undo, and refresh coalescing verified"
    );
    Ok(())
}

fn terminals(root: &impl IsA<gtk::Widget>, found: &mut Vec<vte4::Terminal>) {
    let root = root.as_ref();
    if let Some(terminal) = root.downcast_ref::<vte4::Terminal>() {
        found.push(terminal.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        terminals(&widget, found);
        child = widget.next_sibling();
    }
}
pub async fn profile_lifecycle(ui: &Rc<Ui>) -> Result<(), String> {
    use vte4::prelude::TerminalExt;
    require(
        std::env::var("RELAY_NATIVE_FIXTURE").as_deref() == Ok("1")
            && std::env::var("RELAY_INSTANCE").as_deref() == Ok("test"),
        "Terminal profile requires isolated fixture",
    )?;
    ui.navigate("agents");
    let mut results = Vec::new();
    for _ in 0..5 {
        let mut before = Vec::new();
        terminals(&ui.window, &mut before);
        let start = std::time::Instant::now();
        let session = ui
            .call(
                "session.create",
                json!({"project_id":ui.project.get(),"provider":"claude","role":"builder"}),
            )
            .await
            .map_err(|e| e.to_string())?;
        let created = start.elapsed().as_secs_f64() * 1000.0;
        let name = session["name"]
            .as_str()
            .ok_or("Missing session name")?
            .to_owned();
        let spawn = std::time::Instant::now();
        ui.call("session.spawn", json!({"session":name}))
            .await
            .map_err(|e| e.to_string())?;
        let spawned = spawn.elapsed().as_secs_f64() * 1000.0;
        wait_for(
            || {
                let mut widgets = Vec::new();
                terminals(&ui.window, &mut widgets);
                widgets.len() > before.len()
                    && widgets.iter().any(|terminal| {
                        terminal
                            .text_format(vte4::Format::Text)
                            .is_some_and(|text| text.contains(&format!("Session: {name}")))
                    })
            },
            "New VTE renders provider output",
        )
        .await?;
        let rendered = start.elapsed().as_secs_f64() * 1000.0;
        let close = std::time::Instant::now();
        ui.call("session.close", json!({"session":name}))
            .await
            .map_err(|e| e.to_string())?;
        let closed = close.elapsed().as_secs_f64() * 1000.0;
        wait_for(
            || {
                let mut widgets = Vec::new();
                terminals(&ui.window, &mut widgets);
                widgets.len() == before.len()
            },
            "Closed VTE removed",
        )
        .await?;
        results.push(json!({"create_ms":created,"spawn_ms":spawned,"create_to_output_ms":rendered,"close_ms":closed,"close_to_removed_ms":close.elapsed().as_secs_f64()*1000.0}));
    }
    let mut medians = serde_json::Map::new();
    for metric in [
        "create_ms",
        "spawn_ms",
        "create_to_output_ms",
        "close_ms",
        "close_to_removed_ms",
    ] {
        let mut values: Vec<_> = results
            .iter()
            .filter_map(|row| row[metric].as_f64())
            .collect();
        values.sort_by(f64::total_cmp);
        medians.insert(metric.into(), json!(values[values.len() / 2]));
    }
    println!(
        "TERMINAL_LIFECYCLE={}",
        json!({"samples":results,"median":medians,"fixture":"tiny local repository, fake provider, five warm app iterations"})
    );
    Ok(())
}


async fn image_preview(ui: &Rc<Ui>) -> Result<(), String> {
    let project = ui.call("project.get", json!({"project_id":ui.project.get()})).await.map_err(|e|e.to_string())?;
    let root = std::path::Path::new(project["path"].as_str().ok_or("Fixture project path")?);
    // An uncompressed PNG exceeds the ordinary 2 MiB bus frame limit.
    let pixels = (0..1024 * 768 * 3).map(|n| ((n * 73 + n / 29) % 256) as u8).collect::<Vec<_>>();
    let bytes = glib::Bytes::from_owned(pixels);
    let image = gtk::gdk_pixbuf::Pixbuf::from_bytes(&bytes,
        gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 1024, 768, 1024 * 3);
    let png = image.save_to_bufferv("png", &[("compression", "0")]).map_err(|e|e.to_string())?;
    require(png.len() > 2 * 1024 * 1024, "Image exercises the separate binary transport")?;
    std::fs::write(root.join("preview.PNG"), &png).map_err(|e|e.to_string())?;
    std::fs::write(root.join("broken.png"), b"not an image").map_err(|e|e.to_string())?;
    ui.editor.show_files();
    ui.editor.load_tree(ui, None);
    wait_for(|| named(&ui.window, "project-file:preview.PNG").is_some(), "Image file in tree").await?;
    click(ui, "project-file:preview.PNG")?;
    let picture = named(&ui.window, "project-image").ok_or("Preview widget")?.downcast::<gtk::Picture>().map_err(|_| "Picture type")?;
    wait_for(|| picture.is_mapped() && picture.paintable().is_some(), "Picture rendered").await?;
    require(!ui.editor.is_dirty(), "Image does not create an editable draft")?;
    let texture = picture.paintable().unwrap().downcast::<gtk::gdk::MemoryTexture>().map_err(|_| "Preview texture")?;
    require(texture.width() == 1024 && texture.height() == 768, "Image dimensions")?;
    require(!named(&ui.window, "project-save").unwrap().is_sensitive(), "Image cannot be saved as text")?;
    click(ui, "project-agents")?;
    click(ui, "project-editor")?;
    require(picture.is_mapped(), "Returning to editor restores image")?;
    // Capture the actual editor preview, including fit and surrounding chrome.
    glib::timeout_future(Duration::from_millis(150)).await;
    if let Ok(path) = std::env::var("RELAY_NATIVE_SCREENSHOT") {
        let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, ui.window.width() as f64, ui.window.height() as f64);
        let node = snapshot.to_node().ok_or("Preview snapshot")?;
        ui.window.renderer().ok_or("Preview renderer")?.render_texture(&node, None).save_to_png(path).map_err(|e| e.to_string())?;
    }
    click(ui, "project-file:broken.png")?;
    let message = named(&ui.window, "project-image-message").unwrap().downcast::<gtk::Label>().unwrap();
    wait_for(|| message.text().starts_with("Cannot preview image"), "Invalid image error").await?;
    require(picture.paintable().is_none(), "Invalid image clears previous picture")?;
    click(ui, "project-file:README.md")?;
    let view = named(&ui.window, "project-source").unwrap().downcast::<sourceview5::View>().unwrap();
    wait_for(|| view.is_mapped() && view.is_editable(), "Text editor after image").await?;
    view.buffer().insert_at_cursor("keep this draft");
    click(ui, "project-file:preview.PNG")?;
    require(view.buffer().is_modified() && view.is_mapped(), "Image click preserves unsaved text")?;
    click(ui, "project-discard")?;
    wait_for(|| !ui.editor.is_dirty(), "Discard fixture draft").await?;
    println!("IMAGE_PREVIEW_OK=large_png,fit,agents_roundtrip,invalid_image,source_return,dirty_draft");
    Ok(())
}
