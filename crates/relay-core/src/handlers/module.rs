//! `module.*` — Phase 7 release tracking over the board's single task source of truth.

use crate::engine::{Ctx, Engine, IntoBus};
use crate::handlers::notes::{actor_project, assert_actor_project};
use crate::handlers::task::{get_task, parse_column, parse_priority, priority_str};
use relay_bus::error::BusError;
use relay_bus::ops::module::*;
use relay_bus::types::{Column, Id, Module, ModuleHeader, ModuleSummary, Priority, Task};
use relay_bus::Empty;
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde_json::json;
use std::collections::BTreeMap;

fn module_row(row: &Row) -> rusqlite::Result<Module> {
    Ok(Module {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        name: row.get("name")?,
        icon: row.get("icon")?,
        priority: parse_priority(&row.get::<_, String>("priority")?),
        order: row.get("ord")?,
        completed_at: row.get("completed_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: row.get("deleted_at")?,
    })
}

fn get_module(tx: &Transaction, id: Id, include_deleted: bool) -> Result<Module, BusError> {
    let sql = if include_deleted {
        "SELECT * FROM modules WHERE id=?1"
    } else {
        "SELECT * FROM modules WHERE id=?1 AND deleted_at IS NULL"
    };
    tx.query_row(sql, [id], module_row)
        .optional()
        .bus()?
        .ok_or_else(|| BusError::not_found("module.not_found", format!("no module {id}")))
}

fn emit_module(ctx: &mut Ctx, module: &Module) -> Result<(), BusError> {
    ctx.set_project(module.project_id);
    ctx.emit("module.changed", serde_json::to_value(module).bus()?);
    Ok(())
}

fn counts(tx: &Transaction, module_id: Id) -> Result<BTreeMap<Column, i64>, BusError> {
    let mut out = BTreeMap::new();
    for col in [
        Column::Backlog,
        Column::InReview,
        Column::Ready,
        Column::Active,
        Column::Done,
    ] {
        out.insert(col, 0);
    }
    let mut stmt = tx
        .prepare_cached(
            "SELECT col,COUNT(*) FROM tasks WHERE module_id=?1 AND deleted_at IS NULL GROUP BY col",
        )
        .bus()?;
    let rows = stmt
        .query_map([module_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    for (col, n) in rows {
        out.insert(parse_column(&col), n);
    }
    Ok(out)
}

fn summary(tx: &Transaction, module: Module) -> Result<ModuleSummary, BusError> {
    let counts = counts(tx, module.id)?;
    let total: i64 = counts.values().sum();
    let done = *counts.get(&Column::Done).unwrap_or(&0);
    Ok(ModuleSummary {
        module,
        counts,
        progress_pct: if total == 0 {
            0.0
        } else {
            done as f64 * 100.0 / total as f64
        },
    })
}

fn header(tx: &Transaction, project_id: Id) -> Result<ModuleHeader, BusError> {
    crate::handlers::workspace::get_project(tx, project_id)?;
    let count=tx.query_row("SELECT COUNT(*) FROM modules WHERE project_id=?1 AND deleted_at IS NULL AND completed_at IS NULL",[project_id],|r|r.get(0)).bus()?;
    let in_flight=tx.query_row("SELECT COUNT(DISTINCT m.id) FROM modules m JOIN tasks t ON t.module_id=m.id AND t.deleted_at IS NULL WHERE m.project_id=?1 AND m.deleted_at IS NULL AND m.completed_at IS NULL AND t.col IN ('active','in_review')",[project_id],|r|r.get(0)).bus()?;
    let (issues,completed):(i64,i64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(CASE WHEN t.col='done' THEN 1 ELSE 0 END),0) FROM tasks t JOIN modules m ON m.id=t.module_id WHERE m.project_id=?1 AND m.deleted_at IS NULL AND t.deleted_at IS NULL",[project_id],|r|Ok((r.get(0)?,r.get(1)?))).bus()?;
    Ok(ModuleHeader {
        count,
        in_flight,
        issues,
        completed,
        completion_pct: if issues == 0 {
            0.0
        } else {
            completed as f64 * 100.0 / issues as f64
        },
    })
}

fn tasks_for(tx: &Transaction, module_id: Id) -> Result<Vec<Task>, BusError> {
    let mut stmt=tx.prepare_cached("SELECT id FROM tasks WHERE module_id=?1 AND deleted_at IS NULL ORDER BY CASE col WHEN 'backlog' THEN 0 WHEN 'in_review' THEN 1 WHEN 'ready' THEN 2 WHEN 'active' THEN 3 ELSE 4 END,position,id").bus()?;
    let ids = stmt
        .query_map([module_id], |r| r.get(0))
        .bus()?
        .collect::<rusqlite::Result<Vec<Id>>>()
        .bus()?;
    ids.into_iter().map(|id| get_task(tx, id, false)).collect()
}

/// The task ids a module delete or restore relinks, so its `task.changed` hint names them.
fn ids(tx: &Transaction, sql: &str, module_id: Id) -> Result<Vec<Id>, BusError> {
    tx.prepare_cached(sql).bus()?.query_map([module_id], |r| r.get(0)).bus()?.collect::<rusqlite::Result<Vec<Id>>>().bus()
}

pub fn register(e: &mut Engine) {
    e.register::<Create>(|ctx:&mut Ctx,p|{
        crate::handlers::workspace::get_project(ctx.tx(),p.project_id)?;let name=p.name.trim();if name.is_empty(){return Err(BusError::invalid("module.name","module name cannot be empty"))}
        let order:i64=ctx.tx().query_row("SELECT COALESCE(MAX(ord),-1)+1 FROM modules WHERE project_id=?1 AND deleted_at IS NULL",[p.project_id],|r|r.get(0)).bus()?;
        ctx.tx().execute("INSERT INTO modules(project_id,name,icon,priority,ord,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?6)",params![p.project_id,name,p.icon,priority_str(p.priority.unwrap_or(Priority::Medium)),order,ctx.now]).bus()?;
        let module=get_module(ctx.tx(),ctx.tx().last_insert_rowid(),false)?;ctx.set_undo("module.delete",json!({"module_id":module.id}),Some(json!({"updated_at":module.updated_at})));emit_module(ctx,&module)?;Ok(module)
    });
    e.register::<Get>(|ctx, p| {
        let module = get_module(ctx.tx(), p.module_id, false)?;
        assert_actor_project(ctx, module.project_id)?;
        let mut grouped: BTreeMap<Column, Vec<Task>> = BTreeMap::new();
        for col in [
            Column::Backlog,
            Column::InReview,
            Column::Ready,
            Column::Active,
            Column::Done,
        ] {
            grouped.insert(col, Vec::new());
        }
        for task in tasks_for(ctx.tx(), module.id)? {
            grouped.entry(task.column).or_default().push(task);
        }
        Ok(GetOut {
            module,
            tasks_by_state: grouped,
        })
    });
    e.register::<List>(|ctx,p|{
        // An agent reads its own project's modules, as it does its board (D106, RA-411).
        let project_id=match (actor_project(ctx)?,p.project_id){
            (Some(own),Some(asked)) if asked!=own=>return Err(BusError::not_own("project")),
            (Some(own),_)=>own,
            (None,Some(asked))=>asked,
            (None,None)=>return Err(BusError::invalid("module.project","project_id is required")),
        };
        crate::handlers::workspace::get_project(ctx.tx(),project_id)?;let sql=if p.include_archived.unwrap_or(false){"SELECT * FROM modules WHERE project_id=?1 AND deleted_at IS NULL ORDER BY completed_at IS NOT NULL,ord,id"}else{"SELECT * FROM modules WHERE project_id=?1 AND deleted_at IS NULL AND completed_at IS NULL ORDER BY ord,id"};let mut stmt=ctx.tx().prepare(sql).bus()?;let modules=stmt.query_map([project_id],module_row).bus()?.collect::<rusqlite::Result<Vec<_>>>().bus()?.into_iter().map(|m|summary(ctx.tx(),m)).collect::<Result<Vec<_>,_>>()?;Ok(ListOut{modules,header:header(ctx.tx(),project_id)?})});
    e.register::<Update>(|ctx:&mut Ctx,p|{let before=get_module(ctx.tx(),p.module_id,false)?;
        if let Some(expected) = &p.expected {
            let current = serde_json::to_value(&before).bus()?;
            for (field, value) in expected {
                if !["name", "icon", "priority", "order"].contains(&field.as_str()) {
                    return Err(BusError::invalid("module.expected_field", format!("{field} is not an editable module field")));
                }
                if current.get(field) != Some(value) {
                    return Err(BusError::conflict("module.edit_conflict", format!("Module {field} changed elsewhere; your draft was not saved")));
                }
            }
        }
        let name=p.name.as_deref().unwrap_or(&before.name).trim();if name.is_empty(){return Err(BusError::invalid("module.name","module name cannot be empty"))}let icon=p.icon.unwrap_or(before.icon.clone());ctx.tx().execute("UPDATE modules SET name=?1,icon=?2,priority=?3,ord=?4,updated_at=?5 WHERE id=?6",params![name,icon,priority_str(p.priority.unwrap_or(before.priority)),p.order.unwrap_or(before.order),ctx.now,before.id]).bus()?;let module=get_module(ctx.tx(),before.id,false)?;ctx.set_undo("module.update",json!({"module_id":before.id,"name":before.name,"icon":before.icon,"priority":priority_str(before.priority),"order":before.order}),Some(json!({"updated_at":module.updated_at})));emit_module(ctx,&module)?;Ok(module)});
    e.register::<Complete>(|ctx: &mut Ctx, p| {
        let before = get_module(ctx.tx(), p.module_id, false)?;
        if before.completed_at.is_some() {
            return Err(BusError::conflict(
                "module.completed",
                format!("module {} is already completed", before.id),
            ));
        }
        ctx.tx()
            .execute(
                "UPDATE modules SET completed_at=?1,updated_at=?1 WHERE id=?2",
                params![ctx.now, before.id],
            )
            .bus()?;
        let module = get_module(ctx.tx(), before.id, false)?;
        ctx.set_undo(
            "module.reopen",
            json!({"module_id":module.id}),
            Some(json!({"updated_at":module.updated_at})),
        );
        emit_module(ctx, &module)?;
        Ok(module)
    });
    e.register::<Reopen>(|ctx: &mut Ctx, p| {
        let before = get_module(ctx.tx(), p.module_id, false)?;
        if before.completed_at.is_none() {
            return Err(BusError::conflict(
                "module.not_completed",
                format!("module {} is not completed", before.id),
            ));
        }
        ctx.tx()
            .execute(
                "UPDATE modules SET completed_at=NULL,updated_at=?1 WHERE id=?2",
                params![ctx.now, before.id],
            )
            .bus()?;
        let module = get_module(ctx.tx(), before.id, false)?;
        ctx.set_undo(
            "module.complete",
            json!({"module_id":module.id}),
            Some(json!({"updated_at":module.updated_at})),
        );
        emit_module(ctx, &module)?;
        Ok(module)
    });
    e.register::<Delete>(|ctx:&mut Ctx,p|{let module=get_module(ctx.tx(),p.module_id,false)?;let task_ids=ids(ctx.tx(),"SELECT id FROM tasks WHERE module_id=?1 ORDER BY id",module.id)?;ctx.tx().execute("INSERT OR IGNORE INTO module_unlinked_tasks(module_id,task_id) SELECT ?1,id FROM tasks WHERE module_id=?1",[module.id]).bus()?;ctx.tx().execute("UPDATE tasks SET module_id=NULL,updated_at=?1 WHERE module_id=?2",params![ctx.now,module.id]).bus()?;ctx.tx().execute("UPDATE modules SET deleted_at=?1,updated_at=?1 WHERE id=?2",params![ctx.now,module.id]).bus()?;ctx.set_project(module.project_id);ctx.set_undo("module.restore",json!({"module_id":module.id}),Some(json!({"updated_at":ctx.now})));ctx.emit("module.deleted",json!({"id":module.id,"project_id":module.project_id}));ctx.emit("task.changed",json!({"module_id":module.id,"unlinked":true,"task_ids":task_ids}));Ok(Empty{})});
    e.register::<Restore>(|ctx:&mut Ctx,p|{let before=get_module(ctx.tx(),p.module_id,true)?;if before.deleted_at.is_none(){return Err(BusError::conflict("module.not_deleted",format!("module {} is not deleted",before.id)))}ctx.tx().execute("UPDATE modules SET deleted_at=NULL,updated_at=?1 WHERE id=?2",params![ctx.now,before.id]).bus()?;let task_ids=ids(ctx.tx(),"SELECT id FROM tasks WHERE module_id IS NULL AND id IN (SELECT task_id FROM module_unlinked_tasks WHERE module_id=?1) ORDER BY id",before.id)?;ctx.tx().execute("UPDATE tasks SET module_id=?1,updated_at=?2 WHERE module_id IS NULL AND id IN (SELECT task_id FROM module_unlinked_tasks WHERE module_id=?1)",params![before.id,ctx.now]).bus()?;ctx.tx().execute("DELETE FROM module_unlinked_tasks WHERE module_id=?1",[before.id]).bus()?;let module=get_module(ctx.tx(),before.id,false)?;ctx.set_undo("module.delete",json!({"module_id":module.id}),Some(json!({"updated_at":module.updated_at})));emit_module(ctx,&module)?;ctx.emit("task.changed",json!({"module_id":module.id,"restored":true,"task_ids":task_ids}));Ok(module)});
    e.register::<Stats>(|ctx, p| {
        assert_actor_project(ctx, p.project_id)?;
        header(ctx.tx(), p.project_id)
    });
    e.register::<ChangelogDraft>(|ctx, p| {
        if p.group_by.as_deref().is_some_and(|v| v != "priority") {
            return Err(BusError::invalid(
                "module.changelog_group",
                "group_by must be priority",
            ));
        }
        let module = get_module(ctx.tx(), p.module_id, false)?;
        assert_actor_project(ctx, module.project_id)?;
        let mut done = tasks_for(ctx.tx(), module.id)?
            .into_iter()
            .filter(|t| t.column == Column::Done && !t.changelog.trim().is_empty())
            .collect::<Vec<_>>();
        done.sort_by_key(|t| match t.priority {
            Priority::Urgent => 0,
            Priority::High => 1,
            Priority::Medium => 2,
            Priority::Low => 3,
        });
        let mut markdown = format!("# {}\n", module.name);
        let mut last = None;
        let mut ids = Vec::new();
        for task in done {
            if last != Some(task.priority) {
                markdown.push_str(&format!(
                    "\n## {}\n",
                    match task.priority {
                        Priority::Urgent => "Urgent",
                        Priority::High => "High",
                        Priority::Medium => "Medium",
                        Priority::Low => "Low",
                    }
                ));
                last = Some(task.priority);
            }
            markdown.push_str(&format!("- {}\n", task.changelog.trim()));
            ids.push(task.id);
        }
        Ok(ChangelogDraftOut {
            markdown: markdown.trim_end().to_string() + "\n",
            tasks: ids,
        })
    });
}
