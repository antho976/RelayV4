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
    } else if op.starts_with("skill.") {
        ("skills", "skill_id")
    } else {
        return Ok(None);
    };
    let Some(id) = payload.get(key).and_then(Value::as_i64) else { return Ok(None) };
    let sql = format!("SELECT updated_at FROM {table} WHERE id=?1");
    ctx.tx().query_row(&sql, [id], |row| row.get(0)).optional().bus()
}

pub fn register(e: &mut Engine) {
    e.register::<List>(|ctx, p| {
        let f = log::ListFilter {
            project_id: p.project_id,
            actor: p.actor.as_ref(),
            session_id: p.session_id,
            op_prefix: p.op_prefix.as_deref(),
            parent_req: p.parent_req.as_deref(),
            since: p.since.as_deref(),
            until: p.until.as_deref(),
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
        let inverse = row.undo_op.expect("checked above");
        if !p.force.unwrap_or(false) {
            if let Some(expected) = inverse.expect.as_ref().and_then(|v| v.get("updated_at")).and_then(Value::as_str) {
                let actual = target_updated_at(ctx, &inverse.op, &inverse.payload)?;
                if actual.as_deref() != Some(expected) {
                    return Err(BusError::conflict("audit.stale", "the entity changed after the audited operation")
                        .with_details(serde_json::json!({"expected": expected, "actual": actual})));
                }
            }
        }
        let by: i64 = ctx.tx().query_row("SELECT COALESCE(MAX(id),0)+1 FROM audit", [], |r| r.get(0)).bus()?;
        ctx.invoke_registered(&inverse.op, inverse.payload)?;
        ctx.set_undo_of(row.id);
        Ok(UndoOut { undone: row.id, by })
    });
}
