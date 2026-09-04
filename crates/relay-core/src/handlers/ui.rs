//! Phase 9 shell state and durable named layouts.

use crate::engine::{Ctx, Engine, IntoBus};
use relay_bus::ops::ui::*;
use relay_bus::types::{PaneInfo, PaneTarget, WindowInfo};
use relay_bus::Empty;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

fn state(engine: &Engine) -> StateOut {
    let ui = engine.ui.lock().unwrap();
    StateOut {
        project_id: ui.project_id,
        page: ui.page,
        panes: ui.panes.clone(),
        focused: ui.focused.clone(),
        windows: ui.windows.clone(),
    }
}

fn valid_name(name: &str) -> Result<&str, relay_bus::BusError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(relay_bus::BusError::invalid(
            "ui.layout_name",
            "layout name must be 1 to 64 characters",
        ));
    }
    Ok(name)
}

fn fallback_state(ctx: &Ctx, project_id: i64) -> Result<Value, relay_bus::BusError> {
    let current =
        crate::handlers::settings::get(ctx.tx(), Some(&format!("layout.current.{project_id}")))?;
    Ok(if current.is_null() {
        json!({"page":"agents","agent_layout":"grid"})
    } else {
        current
    })
}

pub fn register(engine: &mut Engine) {
    engine.register::<State>(|ctx, _| Ok(state(ctx.engine())));
    engine.register::<PageSwitch>(|ctx: &mut Ctx, payload| {
        if let Some(project_id) = payload.project_id {
            crate::handlers::workspace::get_project(ctx.tx(), project_id)?;
        }
        let before = state(ctx.engine());
        {
            let mut ui = ctx.engine().ui.lock().unwrap();
            ui.page = payload.page;
            if payload.project_id.is_some() {
                ui.project_id = payload.project_id;
            }
        }
        ctx.set_undo(
            "ui.page.switch",
            json!({"page":before.page,"project_id":before.project_id}),
            None,
        );
        ctx.emit(
            "ui.changed",
            serde_json::to_value(state(ctx.engine())).bus()?,
        );
        Ok(Empty {})
    });
    engine.register::<PaneOpen>(|ctx: &mut Ctx, payload| {
        let pane = {
            let mut ui = ctx.engine().ui.lock().unwrap();
            let pane = format!("pane-{}", ui.next_pane);
            ui.next_pane += 1;
            for item in &mut ui.panes {
                item.focused = false;
            }
            ui.panes.push(PaneInfo {
                pane: pane.clone(),
                kind: payload.kind,
                target: payload.target.unwrap_or(PaneTarget {
                    session: None,
                    path: None,
                    sha: None,
                    note_id: None,
                    run_id: None,
                    mirror_id: None,
                }),
                window_id: "main".into(),
                focused: true,
            });
            ui.focused = Some(pane.clone());
            if let Some(main) = ui.windows.iter_mut().find(|window| window.main) {
                main.panes.push(pane.clone());
            }
            pane
        };
        ctx.emit(
            "ui.changed",
            serde_json::to_value(state(ctx.engine())).bus()?,
        );
        Ok(PaneOpenOut { pane })
    });
    engine.register::<PaneClose>(|ctx: &mut Ctx, payload| {
        let mut ui = ctx.engine().ui.lock().unwrap();
        let before = ui.panes.len();
        ui.panes.retain(|pane| pane.pane != payload.pane);
        if before == ui.panes.len() {
            return Err(relay_bus::BusError::not_found(
                "ui.pane_not_found",
                format!("no pane {}", payload.pane),
            ));
        }
        for window in &mut ui.windows {
            window.panes.retain(|pane| pane != &payload.pane);
        }
        if ui.focused.as_deref() == Some(payload.pane.as_str()) {
            ui.focused = ui.panes.last().map(|pane| pane.pane.clone());
        }
        drop(ui);
        ctx.emit(
            "ui.changed",
            serde_json::to_value(state(ctx.engine())).bus()?,
        );
        Ok(Empty {})
    });
    engine.register::<PaneFocus>(|ctx: &mut Ctx, payload| {
        let mut ui = ctx.engine().ui.lock().unwrap();
        if !ui.panes.iter().any(|pane| pane.pane == payload.pane) {
            return Err(relay_bus::BusError::not_found(
                "ui.pane_not_found",
                format!("no pane {}", payload.pane),
            ));
        }
        for pane in &mut ui.panes {
            pane.focused = pane.pane == payload.pane;
        }
        ui.focused = Some(payload.pane);
        drop(ui);
        ctx.emit(
            "ui.changed",
            serde_json::to_value(state(ctx.engine())).bus()?,
        );
        Ok(Empty {})
    });
    engine.register::<PaneMove>(|ctx: &mut Ctx, payload| {
        if !matches!(
            payload.edge.as_str(),
            "top" | "bottom" | "left" | "right" | "center"
        ) {
            return Err(relay_bus::BusError::invalid(
                "ui.pane_edge",
                "edge must be top, bottom, left, right, or center",
            ));
        }
        let mut ui = ctx.engine().ui.lock().unwrap();
        let from = ui
            .panes
            .iter()
            .position(|pane| pane.pane == payload.pane)
            .ok_or_else(|| {
                relay_bus::BusError::not_found(
                    "ui.pane_not_found",
                    format!("no pane {}", payload.pane),
                )
            })?;
        let to = ui
            .panes
            .iter()
            .position(|pane| pane.pane == payload.to)
            .ok_or_else(|| {
                relay_bus::BusError::not_found(
                    "ui.pane_not_found",
                    format!("no pane {}", payload.to),
                )
            })?;
        let pane = ui.panes.remove(from);
        let at = to.min(ui.panes.len());
        ui.panes.insert(at, pane);
        drop(ui);
        ctx.emit(
            "ui.changed",
            json!({"move":{"pane":payload.pane,"to":payload.to,"edge":payload.edge}}),
        );
        Ok(Empty {})
    });
    engine.register::<LayoutList>(|ctx, payload| {
        crate::handlers::workspace::get_project(ctx.tx(), payload.project_id)?;
        let mut stmt = ctx
            .tx()
            .prepare_cached("SELECT name FROM ui_layouts WHERE project_id=?1 ORDER BY name COLLATE NOCASE")
            .bus()?;
        let layouts = stmt
            .query_map([payload.project_id], |row| row.get(0))
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        Ok(LayoutListOut { layouts })
    });
    engine.register::<LayoutSave>(|ctx: &mut Ctx, payload| {
        crate::handlers::workspace::get_project(ctx.tx(), payload.project_id)?;
        let name = valid_name(&payload.name)?.to_string();
        let layout = match payload.state {
            Some(value) => value,
            None => fallback_state(ctx, payload.project_id)?,
        };
        let before = ctx
            .tx()
            .query_row(
                "SELECT state FROM ui_layouts WHERE project_id=?1 AND name=?2",
                params![payload.project_id, name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .bus()?;
        ctx.tx().execute(
            "INSERT INTO ui_layouts(project_id,name,state,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)
             ON CONFLICT(project_id,name) DO UPDATE SET state=excluded.state,updated_at=excluded.updated_at",
            params![payload.project_id,name,serde_json::to_string(&layout).bus()?,ctx.now],
        ).bus()?;
        if let Some(before) = before {
            ctx.set_undo("ui.layout.save", json!({"project_id":payload.project_id,"name":name,"state":serde_json::from_str::<Value>(&before).unwrap_or(Value::Null)}), None);
        } else {
            ctx.set_undo("ui.layout.delete", json!({"project_id":payload.project_id,"name":name}), None);
        }
        ctx.set_project(payload.project_id);
        ctx.emit("layout.changed", json!({"action":"saved","project_id":payload.project_id,"name":name,"state":layout}));
        Ok(Empty {})
    });
    engine.register::<LayoutApply>(|ctx: &mut Ctx, payload| {
        crate::handlers::workspace::get_project(ctx.tx(), payload.project_id)?;
        let name = valid_name(&payload.name)?;
        let raw = ctx
            .tx()
            .query_row(
                "SELECT state FROM ui_layouts WHERE project_id=?1 AND name=?2",
                params![payload.project_id, name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .bus()?
            .ok_or_else(|| {
                relay_bus::BusError::not_found("ui.layout_not_found", format!("no layout {name:?}"))
            })?;
        let layout: Value = serde_json::from_str(&raw).bus()?;
        ctx.set_project(payload.project_id);
        ctx.emit(
            "layout.changed",
            json!({"action":"applied","project_id":payload.project_id,"name":name,"state":layout}),
        );
        Ok(Empty {})
    });
    engine.register::<LayoutDelete>(|ctx: &mut Ctx, payload| {
        let name = valid_name(&payload.name)?.to_string();
        let before = ctx.tx().query_row(
            "SELECT state FROM ui_layouts WHERE project_id=?1 AND name=?2",
            params![payload.project_id,name], |row| row.get::<_, String>(0),
        ).optional().bus()?.ok_or_else(|| relay_bus::BusError::not_found("ui.layout_not_found", format!("no layout {name:?}")))?;
        ctx.tx().execute("DELETE FROM ui_layouts WHERE project_id=?1 AND name=?2", params![payload.project_id,name]).bus()?;
        ctx.set_undo("ui.layout.save", json!({"project_id":payload.project_id,"name":name,"state":serde_json::from_str::<Value>(&before).unwrap_or(Value::Null)}), None);
        ctx.set_project(payload.project_id);
        ctx.emit("layout.changed", json!({"action":"deleted","project_id":payload.project_id,"name":name}));
        Ok(Empty {})
    });
    engine.register::<WindowPopout>(|ctx: &mut Ctx, payload| {
        let mut ui = ctx.engine().ui.lock().unwrap();
        let position = ui
            .panes
            .iter()
            .position(|pane| pane.pane == payload.pane)
            .ok_or_else(|| {
                relay_bus::BusError::not_found(
                    "ui.pane_not_found",
                    format!("no pane {}", payload.pane),
                )
            })?;
        let window_id = format!("popout-{}", ui.next_window);
        ui.next_window += 1;
        ui.panes[position].window_id = window_id.clone();
        for window in &mut ui.windows {
            window.panes.retain(|pane| pane != &payload.pane);
        }
        ui.windows.push(WindowInfo {
            window_id: window_id.clone(),
            main: false,
            panes: vec![payload.pane],
        });
        drop(ui);
        ctx.emit(
            "ui.changed",
            serde_json::to_value(state(ctx.engine())).bus()?,
        );
        Ok(PopoutOut { window_id })
    });
    engine.register::<WindowClose>(|ctx: &mut Ctx, payload| {
        if payload.window_id == "main" {
            return Err(relay_bus::BusError::conflict(
                "ui.main_window",
                "the main window cannot be closed through ui.window.close",
            ));
        }
        let mut ui = ctx.engine().ui.lock().unwrap();
        let index = ui
            .windows
            .iter()
            .position(|window| window.window_id == payload.window_id)
            .ok_or_else(|| {
                relay_bus::BusError::not_found(
                    "ui.window_not_found",
                    format!("no window {}", payload.window_id),
                )
            })?;
        let panes = ui.windows.remove(index).panes;
        for pane in &mut ui.panes {
            if panes.contains(&pane.pane) {
                pane.window_id = "main".into();
            }
        }
        if let Some(main) = ui.windows.iter_mut().find(|window| window.main) {
            main.panes.extend(panes);
        }
        drop(ui);
        ctx.emit(
            "ui.changed",
            serde_json::to_value(state(ctx.engine())).bus()?,
        );
        Ok(Empty {})
    });
    engine.register::<WindowList>(|ctx, _| {
        Ok(WindowListOut {
            windows: ctx.engine().ui.lock().unwrap().windows.clone(),
        })
    });
    engine.register::<Toast>(|ctx: &mut Ctx, payload| {
        ctx.emit("ui.toast", json!({"text":payload.text,"level":payload.level.unwrap_or_else(||"info".into()),"ttl_ms":payload.ttl_ms.unwrap_or(4000)}));
        Ok(Empty {})
    });
    engine.register::<OsReveal>(|ctx: &mut Ctx, payload| {
        let path = std::path::PathBuf::from(payload.path);
        if !path.is_absolute() {
            return Err(relay_bus::BusError::invalid(
                "os.path",
                "path must be absolute",
            ));
        }
        ctx.after_commit(move |_| {
            let target = path.parent().unwrap_or(&path);
            let _ = std::process::Command::new("xdg-open").arg(target).spawn();
        });
        Ok(Empty {})
    });
    engine.register::<OsOpenUrl>(|ctx: &mut Ctx, payload| {
        if !(payload.url.starts_with("https://") || payload.url.starts_with("http://")) {
            return Err(relay_bus::BusError::invalid(
                "os.url",
                "only http and https URLs are supported",
            ));
        }
        ctx.after_commit(move |_| {
            let _ = std::process::Command::new("xdg-open")
                .arg(payload.url)
                .spawn();
        });
        Ok(Empty {})
    });
}
