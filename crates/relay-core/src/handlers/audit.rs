//! `audit.*` (BUS.md §10.3): list, get. `audit.undo` lands with the board (phase 7).

use crate::audit as log;
use crate::engine::{Ctx, Engine, IntoBus};
use relay_bus::error::BusError;
use relay_bus::ops::audit::*;
use rusqlite::OptionalExtension;
use serde_json::Value;

fn target_updated_at(ctx: &Ctx, op: &str, payload: &Value) -> Result<Option<String>, BusError> {
    let (table, key) = if op.starts_with("task.") {
        ("tasks", "task_id")
    } else if op.starts_with("module.") {
        ("modules", "module_id")
    } else if op.starts_with("notes.") {
        ("notes", "note_id")
    } else if op == "project.update" {
        ("projects", "project_id")
    } else if op.starts_with("workspace.") {
        ("workspaces", "workspace_id")
    } else if op.starts_with("skill.") {
        ("skills", "skill_id")
    } else if op == "session.update" {
        // Sessions are addressed by name, which a closed session hands on to a later one.
        let Some(name) = payload.get("session").and_then(Value::as_str) else { return Ok(None) };
        return ctx.tx().query_row(
            "SELECT updated_at FROM sessions WHERE name=?1 AND state!='closed' ORDER BY id DESC LIMIT 1",
            [name], |row| row.get(0),
        ).optional().bus();
    } else {
        return Err(BusError::conflict("audit.stale", format!("cannot verify that the target of {op} is unchanged; pass force to undo anyway")));
    };
    let Some(id) = payload.get(key).and_then(Value::as_i64) else { return Ok(None) };
    let sql = format!("SELECT updated_at FROM {table} WHERE id=?1");
    ctx.tx().query_row(&sql, [id], |row| row.get(0)).optional().bus()
}

/// The settings subtree an op writes: `""` is the whole tree.
fn settings_path(op: &str, payload: &Value) -> Option<String> {
    match op {
        "settings.set" | "settings.reset" => Some(payload.get("path").and_then(Value::as_str).unwrap_or("").to_string()),
        "notify.settings.set" => Some("notifications".into()),
        _ => None,
    }
}

fn paths_overlap(a: &str, b: &str) -> bool {
    a.is_empty() || b.is_empty() || a == b || a.starts_with(&format!("{b}.")) || b.starts_with(&format!("{a}."))
}

/// For ops that record no `expect` — settings and guardrail config have no `updated_at` per
/// value — the staleness test is whether a later successful op wrote the same thing.
fn written_since(ctx: &Ctx, row: &relay_bus::types::AuditRow) -> Result<bool, BusError> {
    let payload = row.payload.clone().unwrap_or(Value::Null);
    let later = |ops: &[&str]| -> Result<Vec<(String, Value)>, BusError> {
        let mut stmt = ctx.tx().prepare_cached(
            "SELECT op, payload FROM audit WHERE id > ?1 AND kind='ok' AND op IN (SELECT value FROM json_each(?2))",
        ).bus()?;
        let rows = stmt.query_map(rusqlite::params![row.id, serde_json::to_string(ops).unwrap_or_default()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        }).bus()?.collect::<rusqlite::Result<Vec<_>>>().bus()?;
        Ok(rows.into_iter().map(|(op, p)| (op, p.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or(Value::Null))).collect())
    };
    if let Some(path) = settings_path(&row.op, &payload) {
        return Ok(later(&["settings.set", "settings.reset", "notify.settings.set"])?.iter()
            .any(|(op, p)| settings_path(op, p).is_none_or(|other| paths_overlap(&path, &other))));
    }
    if row.op == "guardrail.config.set" {
        let scope = |p: &Value| (p.get("workspace_id").cloned(), p.get("project_id").cloned());
        return Ok(later(&["guardrail.config.set"])?.iter().any(|(_, p)| scope(p) == scope(&payload)));
    }
    Ok(false)
}

pub fn register(e: &mut Engine) {
    e.register::<List>(|ctx, p| {
        let since = p.since.as_deref().map(|s| crate::time::bound("since", s)).transpose()?;
        let until = p.until.as_deref().map(|s| crate::time::bound("until", s)).transpose()?;
        let f = log::ListFilter {
            project_id: p.project_id,
            actor: p.actor.as_ref(),
            session_id: p.session_id,
            op_prefix: p.op_prefix.as_deref(),
            parent_req: p.parent_req.as_deref(),
            since: since.as_deref(),
            until: until.as_deref(),
            limit: p.limit.unwrap_or(200),
        };
        let rows = log::list(ctx.tx(), &f).bus()?;
        Ok(ListOut { rows })
    });
    e.register::<Get>(|ctx, p| {
        log::get(ctx.tx(), p.audit_id).bus()?
            .ok_or_else(|| BusError::not_found("audit.not_found", format!("no audit row {}", p.audit_id)))
    });
    e.register::<Undo>(|ctx: &mut Ctx, p| {
        let row = log::get(ctx.tx(), p.audit_id).bus()?
            .ok_or_else(|| BusError::not_found("audit.not_found", format!("no audit row {}", p.audit_id)))?;
        if row.kind != relay_bus::types::AuditKind::Ok || row.undo_op.is_none() {
            return Err(BusError::conflict("audit.not_undoable", format!("audit row {} has no successful inverse", row.id)));
        }
        if row.undone_by.is_some() {
            return Err(BusError::conflict("audit.not_undoable", format!("audit row {} was already undone", row.id)));
        }
        let inverse = row.undo_op.clone().expect("checked above");
        if !p.force.unwrap_or(false) {
            if let Some(expected) = inverse.expect.as_ref().and_then(|v| v.get("updated_at")).and_then(Value::as_str) {
                let actual = target_updated_at(ctx, &inverse.op, &inverse.payload)?;
                if actual.as_deref() != Some(expected) {
                    return Err(BusError::conflict("audit.stale", "the entity changed after the audited operation")
                        .with_details(serde_json::json!({"expected": expected, "actual": actual})));
                }
            } else if written_since(ctx, &row)? {
                return Err(BusError::conflict("audit.stale", "a later operation changed the same setting; pass force to undo anyway"));
            }
        }
        let by: i64 = ctx.tx().query_row("SELECT COALESCE(MAX(id),0)+1 FROM audit", [], |r| r.get(0)).bus()?;
        ctx.invoke_registered(&inverse.op, inverse.payload)?;
        ctx.set_undo_of(row.id);
        Ok(UndoOut { undone: row.id, by })
    });
}
