//! `task.*` — the Phase 7 board, task lifecycle, attachments, and dispatch spine.

use crate::engine::{Ctx, Engine, IntoBus};
use crate::sessions;
use base64::Engine as _;
use relay_bus::error::BusError;
use relay_bus::ops::task::*;
use relay_bus::types::{
    Attachment, Column, Id, Label, Priority, Size, Task, TaskCommit, TaskRelation, TaskRollup,
    TaskState, TaskType, TASK_CHILDREN_MAX, TASK_DEPTH_MAX,
};
use relay_bus::Empty;
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde_json::json;
use std::path::{Path, PathBuf};

pub(crate) fn column_str(value: Column) -> &'static str {
    match value {
        Column::Backlog => "backlog",
        Column::InReview => "in_review",
        Column::Ready => "ready",
        Column::Active => "active",
        Column::Done => "done",
    }
}
pub(crate) fn parse_column(value: &str) -> Column {
    match value {
        "in_review" => Column::InReview,
        "ready" => Column::Ready,
        "active" => Column::Active,
        "done" => Column::Done,
        _ => Column::Backlog,
    }
}
pub(crate) fn priority_str(value: Priority) -> &'static str {
    match value {
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
        Priority::Urgent => "urgent",
    }
}
pub(crate) fn parse_priority(value: &str) -> Priority {
    match value {
        "low" => Priority::Low,
        "high" => Priority::High,
        "urgent" => Priority::Urgent,
        _ => Priority::Medium,
    }
}
fn state_str(value: TaskState) -> &'static str {
    match value {
        TaskState::None => "none",
        TaskState::Dispatched => "dispatched",
        TaskState::Running => "running",
        TaskState::Blocked => "blocked",
        TaskState::Failed => "failed",
        TaskState::AwaitingReview => "awaiting_review",
    }
}
fn parse_task_state(value: &str) -> TaskState {
    match value {
        "dispatched" => TaskState::Dispatched,
        "running" => TaskState::Running,
        "blocked" => TaskState::Blocked,
        "failed" => TaskState::Failed,
        "awaiting_review" => TaskState::AwaitingReview,
        _ => TaskState::None,
    }
}
fn size_str(value: Size) -> &'static str {
    match value {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}
fn parse_size(value: Option<String>) -> Option<Size> {
    match value.as_deref() {
        Some("S") => Some(Size::S),
        Some("M") => Some(Size::M),
        Some("L") => Some(Size::L),
        _ => None,
    }
}

pub(crate) fn type_str(value: TaskType) -> &'static str {
    match value {
        TaskType::Task => "task",
        TaskType::Feature => "feature",
        TaskType::Bug => "bug",
        TaskType::Chore => "chore",
        TaskType::Spike => "spike",
    }
}
fn parse_type(value: &str) -> TaskType {
    match value {
        "feature" => TaskType::Feature,
        "bug" => TaskType::Bug,
        "chore" => TaskType::Chore,
        "spike" => TaskType::Spike,
        _ => TaskType::Task,
    }
}
fn relation_str(value: TaskRelation) -> &'static str {
    match value {
        TaskRelation::BlockedBy => "blocked_by",
        TaskRelation::DuplicateOf => "duplicate_of",
    }
}

/// Labels are stored once per project and referenced; the wire form is the trimmed name.
fn normalise_label(value: &str) -> Result<String, BusError> {
    let name = value.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(BusError::invalid(
            "task.label",
            "a label is 1-40 characters of non-blank text",
        ));
    }
    Ok(name.to_string())
}

fn label_id(tx: &Transaction, project_id: Id, name: &str, now: &str) -> Result<Id, BusError> {
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM labels WHERE project_id=?1 AND name=?2 COLLATE NOCASE",
            params![project_id, name],
            |r| r.get(0),
        )
        .optional()
        .bus()?
    {
        return Ok(id);
    }
    tx.execute(
        "INSERT INTO labels(project_id,name,created_at) VALUES (?1,?2,?3)",
        params![project_id, name, now],
    )
    .bus()?;
    Ok(tx.last_insert_rowid())
}

/// Distance from the root, walking parents. Bounded by `TASK_DEPTH_MAX` so a cycle that
/// somehow reached the store cannot spin here.
fn depth_of(tx: &Transaction, task_id: Id) -> rusqlite::Result<i64> {
    let mut depth = 0;
    let mut cursor = task_id;
    while let Some(parent) = tx
        .prepare_cached("SELECT parent_id FROM tasks WHERE id=?1")?
        .query_row([cursor], |r| r.get::<_, Option<Id>>(0))
        .optional()?
        .flatten()
    {
        depth += 1;
        cursor = parent;
        if depth > TASK_DEPTH_MAX {
            break;
        }
    }
    Ok(depth)
}

fn child_ids(tx: &Transaction, task_id: Id) -> Result<Vec<Id>, BusError> {
    let mut stmt = tx
        .prepare_cached(
            "SELECT id FROM tasks WHERE parent_id=?1 AND deleted_at IS NULL ORDER BY position,id",
        )
        .bus()?;
    let values = stmt
        .query_map([task_id], |r| r.get(0))
        .bus()?
        .collect::<rusqlite::Result<Vec<Id>>>()
        .bus()?;
    Ok(values)
}

/// The deepest level below `task_id`, so re-parenting a whole subtree can be checked in one go.
/// `assert_parent` refuses cycles on every write path; the visited set is what stops a
/// hand-edited store from blowing the stack inside a request transaction.
fn subtree_height(tx: &Transaction, task_id: Id) -> Result<i64, BusError> {
    fn walk(
        tx: &Transaction,
        id: Id,
        seen: &mut std::collections::HashSet<Id>,
    ) -> Result<i64, BusError> {
        if !seen.insert(id) {
            return Ok(0);
        }
        let mut height = 0;
        for child in child_ids(tx, id)? {
            height = height.max(1 + walk(tx, child, seen)?);
        }
        Ok(height)
    }
    walk(tx, task_id, &mut std::collections::HashSet::new())
}

/// Every descendant id, parents before their own children. Used for roll-up and fan-out.
fn descendants(tx: &Transaction, task_id: Id) -> Result<Vec<Id>, BusError> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::from([task_id]);
    let mut frontier = std::collections::VecDeque::from([task_id]);
    while let Some(current) = frontier.pop_front() {
        for child in child_ids(tx, current)? {
            if seen.insert(child) {
                out.push(child);
                frontier.push_back(child);
            }
        }
    }
    Ok(out)
}

fn attachment_row(row: &Row) -> rusqlite::Result<Attachment> {
    Ok(Attachment {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        name: row.get("name")?,
        mime: row.get("mime")?,
        bytes: row.get("bytes")?,
        path: row.get("path")?,
        created_at: row.get("created_at")?,
    })
}

/// Every statement here is `prepare_cached`: a task list hydrates each row with seven of them,
/// and compiling five of those per row was 61 % of `task.list` (PERF §1.2).
pub(crate) fn row_task(tx: &Transaction, row: &Row) -> Result<Task, rusqlite::Error> {
    let id: Id = row.get("id")?;
    let sessions = {
        let mut stmt = tx.prepare_cached("SELECT s.name FROM task_sessions ts JOIN sessions s ON s.id=ts.session_id WHERE ts.task_id=?1 ORDER BY ts.ord,ts.session_id")?;
        let values = stmt
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        values
    };
    let commits = {
        let mut stmt = tx.prepare_cached(
            "SELECT sha,branch,linked_at FROM task_commits WHERE task_id=?1 ORDER BY id",
        )?;
        let values = stmt
            .query_map([id], |r| {
                Ok(TaskCommit {
                    sha: r.get(0)?,
                    branch: r.get(1)?,
                    linked_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        values
    };
    let attachments = {
        let mut stmt =
            tx.prepare_cached("SELECT * FROM attachments WHERE task_id=?1 ORDER BY id")?;
        let values = stmt
            .query_map([id], attachment_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        values
    };
    let labels = {
        let mut stmt = tx.prepare_cached("SELECT l.name FROM task_labels tl JOIN labels l ON l.id=tl.label_id WHERE tl.task_id=?1 ORDER BY l.name COLLATE NOCASE")?;
        let values = stmt
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        values
    };
    let edges = |sql: &str| -> rusqlite::Result<Vec<Id>> {
        let mut stmt = tx.prepare_cached(sql)?;
        let values = stmt.query_map([id], |r| r.get(0))?.collect();
        values
    };
    let blocked_by = edges("SELECT r.to_task FROM task_relations r JOIN tasks t ON t.id=r.to_task WHERE r.from_task=?1 AND r.rel='blocked_by' AND t.deleted_at IS NULL ORDER BY r.to_task")?;
    let blocks = edges("SELECT r.from_task FROM task_relations r JOIN tasks t ON t.id=r.from_task WHERE r.to_task=?1 AND r.rel='blocked_by' AND t.deleted_at IS NULL ORDER BY r.from_task")?;
    let duplicate_of = edges("SELECT r.to_task FROM task_relations r JOIN tasks t ON t.id=r.to_task WHERE r.from_task=?1 AND r.rel='duplicate_of' AND t.deleted_at IS NULL ORDER BY r.to_task")?
        .into_iter()
        .next();
    let children = {
        let mut stmt = tx.prepare_cached(
            "SELECT id FROM tasks WHERE parent_id=?1 AND deleted_at IS NULL ORDER BY position,id",
        )?;
        let values = stmt
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<Id>>>()?;
        values
    };
    // Roll-up walks the subtree here rather than in a recursive CTE so it counts exactly the
    // rows `descendants` would fan out to; depth is capped at 3, so the walk is shallow.
    let mut rollup = TaskRollup::default();
    let mut counted: std::collections::HashSet<Id> = std::collections::HashSet::from([id]);
    let mut frontier = children.clone();
    while let Some(current) = frontier.pop() {
        if !counted.insert(current) {
            continue;
        }
        let (col, kids) = {
            let column: String = tx
                .prepare_cached("SELECT col FROM tasks WHERE id=?1")?
                .query_row([current], |r| r.get(0))?;
            let mut stmt = tx.prepare_cached(
                "SELECT id FROM tasks WHERE parent_id=?1 AND deleted_at IS NULL ORDER BY position,id",
            )?;
            let kids = stmt
                .query_map([current], |r| r.get::<_, Id>(0))?
                .collect::<rusqlite::Result<Vec<Id>>>()?;
            (column, kids)
        };
        rollup.total += 1;
        if col == "done" {
            rollup.done += 1;
        }
        frontier.extend(kids);
    }
    Ok(Task {
        id,
        project_id: row.get("project_id")?,
        module_id: row.get("module_id")?,
        title: row.get("title")?,
        body: row.get("body")?,
        changelog: row.get("changelog")?,
        column: parse_column(&row.get::<_, String>("col")?),
        position: row.get("position")?,
        state: parse_task_state(&row.get::<_, String>("state")?),
        priority: parse_priority(&row.get::<_, String>("priority")?),
        size: parse_size(row.get("size")?),
        task_type: parse_type(&row.get::<_, String>("kind")?),
        parent_id: row.get("parent_id")?,
        depth: depth_of(tx, id)?,
        children,
        rollup,
        labels,
        blocked_by,
        blocks,
        duplicate_of,
        sessions,
        commits,
        attachments,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: row.get("deleted_at")?,
    })
}

pub(crate) fn get_task(tx: &Transaction, id: Id, include_deleted: bool) -> Result<Task, BusError> {
    let sql = if include_deleted {
        "SELECT * FROM tasks WHERE id=?1"
    } else {
        "SELECT * FROM tasks WHERE id=?1 AND deleted_at IS NULL"
    };
    tx.query_row(sql, [id], |row| row_task(tx, row))
        .optional()
        .bus()?
        .ok_or_else(|| BusError::not_found("task.not_found", format!("no task {id}")))
}

fn assert_module(tx: &Transaction, project_id: Id, module_id: Option<Id>) -> Result<(), BusError> {
    let Some(module_id) = module_id else {
        return Ok(());
    };
    let found: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM modules WHERE id=?1 AND project_id=?2 AND deleted_at IS NULL)", params![module_id, project_id], |r| r.get(0)).bus()?;
    if !found {
        return Err(BusError::not_found(
            "module.not_found",
            format!("no module {module_id} in project {project_id}"),
        ));
    }
    Ok(())
}

/// A parent must exist, live in the same project, and leave room under the depth cap for the
/// subtree being hung off it. Cycles are refused by walking up from the candidate parent.
fn assert_parent(
    tx: &Transaction,
    project_id: Id,
    child: Option<Id>,
    parent_id: Id,
) -> Result<(), BusError> {
    let parent = get_task(tx, parent_id, false)?;
    if parent.project_id != project_id {
        return Err(BusError::conflict(
            "task.parent_project",
            "a sub-task must stay in its parent's project",
        ));
    }
    if Some(parent.id) == child {
        return Err(BusError::conflict(
            "task.parent_cycle",
            "a task cannot be its own parent",
        ));
    }
    if let Some(child) = child {
        let mut cursor = parent.parent_id;
        while let Some(id) = cursor {
            if id == child {
                return Err(BusError::conflict(
                    "task.parent_cycle",
                    "that parent is already a descendant of this task",
                ));
            }
            cursor = get_task(tx, id, false)?.parent_id;
        }
    }
    let below = match child {
        Some(child) => subtree_height(tx, child)?,
        None => 0,
    };
    if parent.depth + 1 + below >= TASK_DEPTH_MAX {
        return Err(BusError::conflict(
            "task.depth",
            format!("sub-tasks nest {TASK_DEPTH_MAX} levels deep at most"),
        ));
    }
    let held: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE parent_id=?1 AND deleted_at IS NULL AND id IS NOT ?2",
            params![parent.id, child],
            |r| r.get(0),
        )
        .bus()?;
    if held >= TASK_CHILDREN_MAX {
        return Err(BusError::conflict(
            "task.children_full",
            format!("a task holds {TASK_CHILDREN_MAX} sub-tasks at most"),
        ));
    }
    Ok(())
}

fn set_labels(ctx: &mut Ctx, task: &Task, labels: &[String]) -> Result<(), BusError> {
    ctx.tx()
        .execute("DELETE FROM task_labels WHERE task_id=?1", [task.id])
        .bus()?;
    for raw in labels {
        let name = normalise_label(raw)?;
        let id = label_id(ctx.tx(), task.project_id, &name, &ctx.now)?;
        ctx.tx()
            .execute(
                "INSERT OR IGNORE INTO task_labels(task_id,label_id) VALUES (?1,?2)",
                params![task.id, id],
            )
            .bus()?;
    }
    Ok(())
}

fn emit_task(ctx: &mut Ctx, task: &Task) -> Result<(), BusError> {
    ctx.set_project(task.project_id);
    ctx.emit("task.changed", serde_json::to_value(task).bus()?);
    Ok(())
}

fn next_position(tx: &Transaction, project_id: Id, column: Column) -> Result<i64, BusError> {
    tx.query_row("SELECT COALESCE(MAX(position),-1)+1 FROM tasks WHERE project_id=?1 AND col=?2 AND deleted_at IS NULL", params![project_id, column_str(column)], |r| r.get(0)).bus()
}

/// Where `task` sits among the other live tasks of its column, counting from 0 in board order
/// (`position, id`). Undo records this rather than the raw `position`, which can hold gaps
/// and ties, so moving the task back to that index restores the exact order it left.
fn column_index(tx: &Transaction, task: &Task) -> Result<i64, BusError> {
    tx.prepare_cached("SELECT COUNT(*) FROM tasks WHERE project_id=?1 AND col=?2 AND deleted_at IS NULL AND id<>?3 AND (position<?4 OR (position=?4 AND id<?3))")
        .and_then(|mut stmt| stmt.query_row(params![task.project_id, column_str(task.column), task.id, task.position], |r| r.get(0)))
        .bus()
}

/// Opens slot `index` in `column` for `task_id`: the column's other tasks are renumbered
/// `0..` in board order, skipping `index`, which is returned clamped to the column's length.
fn open_slot(tx: &Transaction, project_id: Id, column: Column, task_id: Id, index: i64) -> Result<i64, BusError> {
    let ids: Vec<Id> = tx
        .prepare_cached("SELECT id FROM tasks WHERE project_id=?1 AND col=?2 AND deleted_at IS NULL AND id<>?3 ORDER BY position,id")
        .and_then(|mut stmt| stmt.query_map(params![project_id, column_str(column), task_id], |r| r.get(0))?.collect())
        .bus()?;
    let index = index.clamp(0, ids.len() as i64);
    let mut renumber = tx.prepare_cached("UPDATE tasks SET position=?1 WHERE id=?2 AND position<>?1").bus()?;
    for (at, id) in ids.iter().enumerate() {
        let at = at as i64;
        renumber.execute(params![if at < index { at } else { at + 1 }, id]).bus()?;
    }
    Ok(index)
}

fn attachment_root(ctx: &Ctx) -> PathBuf {
    ctx.engine()
        .store
        .path()
        .parent()
        .map(|p| p.join("attachments"))
        .unwrap_or_else(|| PathBuf::from("attachments"))
}

fn safe_name(name: &str) -> Result<String, BusError> {
    let name = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err(BusError::invalid(
            "task.attachment_name",
            "attachment name is invalid",
        ));
    }
    Ok(name.to_string())
}

fn attach_bytes(
    ctx: &mut Ctx,
    task: &Task,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<Attachment, BusError> {
    let name = safe_name(name)?;
    if bytes.is_empty() {
        return Err(BusError::invalid(
            "task.attachment_empty",
            "attachment cannot be empty",
        ));
    }
    ctx.tx().execute("INSERT INTO attachments(task_id,name,mime,bytes,path,created_at) VALUES (?1,?2,?3,?4,'',?5)", params![task.id, name, mime, bytes.len() as i64, ctx.now]).bus()?;
    let id = ctx.tx().last_insert_rowid();
    let dir = attachment_root(ctx).join(task.id.to_string());
    std::fs::create_dir_all(&dir).bus()?;
    let path = dir.join(format!("{id}-{name}"));
    std::fs::write(&path, bytes).bus()?;
    ctx.tx()
        .execute(
            "UPDATE attachments SET path=?1 WHERE id=?2",
            params![path.display().to_string(), id],
        )
        .bus()?;
    ctx.tx()
        .query_row(
            "SELECT * FROM attachments WHERE id=?1",
            [id],
            attachment_row,
        )
        .bus()
}

fn link_commit(
    tx: &Transaction,
    task_id: Id,
    sha: &str,
    branch: Option<&str>,
    now: &str,
) -> Result<(), BusError> {
    let sha = sha.trim();
    if sha.is_empty() {
        return Err(BusError::invalid(
            "task.commit",
            "commit sha cannot be empty",
        ));
    }
    tx.execute(
        "INSERT OR IGNORE INTO task_commits(task_id,sha,branch,linked_at) VALUES (?1,?2,?3,?4)",
        params![task_id, sha, branch, now],
    )
    .bus()?;
    Ok(())
}

fn assign_session(ctx: &mut Ctx, name: &str, task: &Task) -> Result<Vec<Id>, BusError> {
    let row = sessions::by_name(ctx.tx(), name)?;
    if row.session.project_id != task.project_id {
        return Err(BusError::conflict(
            "task.session_project",
            "task and session belong to different projects",
        ));
    }
    let mut stmt = ctx
        .tx()
        .prepare_cached("SELECT id FROM sessions WHERE worktree=?1 AND state!='closed' ORDER BY id")
        .bus()?;
    let ids = stmt
        .query_map([&row.session.worktree], |record| record.get::<_, Id>(0))
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    let mut newly_current = Vec::new();
    for id in ids {
        let ord: i64 = ctx
            .tx()
            .query_row(
                "SELECT COALESCE(MAX(ord),-1)+1 FROM task_sessions WHERE task_id=?1",
                [task.id],
                |r| r.get(0),
            )
            .bus()?;
        let queue_ord: i64 = ctx
            .tx()
            .query_row(
                "SELECT COALESCE(MAX(queue_ord),-1)+1 FROM task_sessions WHERE session_id=?1",
                [id],
                |r| r.get(0),
            )
            .bus()?;
        ctx.tx()
            .execute(
                "INSERT OR IGNORE INTO task_sessions(task_id,session_id,ord,queue_ord) VALUES (?1,?2,?3,?4)",
                params![task.id, id, ord, queue_ord],
            )
            .bus()?;
        let next: Option<(Id,Option<Id>)> = ctx.tx().query_row(
            "SELECT t.id,t.module_id FROM task_sessions ts JOIN tasks t ON t.id=ts.task_id WHERE ts.session_id=?1 AND ts.completed_at IS NULL AND t.deleted_at IS NULL AND t.col='active' ORDER BY ts.queue_ord,t.id LIMIT 1",
            [id],|row|Ok((row.get(0)?,row.get(1)?))).optional().bus()?;
        if let Some((task_id,module_id)) = next {
            let changed = ctx.tx().execute(
                "UPDATE sessions SET task_id=?1,module_id=?2,updated_at=?3 WHERE id=?4
                 AND (task_id IS NULL OR NOT EXISTS(SELECT 1 FROM tasks WHERE id=sessions.task_id AND deleted_at IS NULL AND col!='done'))",
                params![task_id,module_id,ctx.now,id]).bus()?;
            if changed > 0 { newly_current.push(id); }
        }
    }
    Ok(newly_current)
}

/// Bump `updated_at` for edits that live in a side table (labels, relations) so the row the
/// UI re-reads is genuinely newer and `set_undo`'s guard has something to compare.
fn touch(ctx: &mut Ctx, task_id: Id) -> Result<(), BusError> {
    ctx.tx()
        .execute(
            "UPDATE tasks SET updated_at=?1 WHERE id=?2",
            params![ctx.now, task_id],
        )
        .bus()?;
    Ok(())
}

/// The other end of a relation must be a live task in the same project, and not the task itself.
fn assert_relatable(tx: &Transaction, task: &Task, other_id: Id) -> Result<Id, BusError> {
    if other_id == task.id {
        return Err(BusError::conflict(
            "task.relation_self",
            "a task cannot relate to itself",
        ));
    }
    let other = get_task(tx, other_id, false)?;
    if other.project_id != task.project_id {
        return Err(BusError::conflict(
            "task.relation_project",
            "related tasks must share a project",
        ));
    }
    Ok(other.id)
}

/// One task → one session (BUS.md §10.5). `task.dispatch` calls this once for the task it was
/// given and once more per not-done descendant when `fanout` is set.
fn dispatch_task(
    ctx: &mut Ctx,
    task_id: Id,
    session: Option<String>,
    create: Option<relay_bus::ops::session::CreateIn>,
    start: bool,
) -> Result<(Task, relay_bus::types::Session), BusError> {
    let (mut session, mut create) = (session, create);
    let before = get_task(ctx.tx(), task_id, false)?;
    if before.column == Column::Done {
        return Err(BusError::conflict(
            "task.column_transition",
            "done tasks cannot be dispatched",
        ));
    }
    if before.column == Column::InReview {
        ctx.tx().execute("UPDATE task_sessions SET completed_at=NULL WHERE task_id=?1",[before.id]).bus()?;
    }
    ctx.tx().execute("UPDATE tasks SET col='active',position=?1,state='dispatched',updated_at=?2 WHERE id=?3",params![next_position(ctx.tx(),before.project_id,Column::Active)?,ctx.now,before.id]).bus()?;
    let active = get_task(ctx.tx(), before.id, false)?;
    let mut newly_current = Vec::new();
    let name = if let Some(name) = session.take() {
        newly_current = assign_session(ctx, &name, &active)?;
        name
    } else {
        let mut create = create.take().expect("validated");
        if create.project_id != active.project_id {
            return Err(BusError::invalid(
                "task.dispatch_project",
                "created session must use the task project",
            ));
        }
        create.task_id = Some(active.id);
        create.module_id = active.module_id;
        let value = ctx.invoke_registered("session.create", serde_json::to_value(create).bus()?)?;
        let name = serde_json::from_value::<relay_bus::types::Session>(value)
            .bus()?
            .name;
        // A create payload may add the second half of a PAIR. Assign both identities to
        // the task so own-task authorization and injected peer context agree.
        newly_current.extend(assign_session(ctx, &name, &active)?);
        name
    };
    let row = sessions::by_name(ctx.tx(), &name)?;
    let launch_op = match row.session.state {
        relay_bus::types::SessionState::Created => Some("session.spawn"),
        relay_bus::types::SessionState::Parked => Some("session.wake"),
        relay_bus::types::SessionState::Restorable => Some("session.resume"),
        relay_bus::types::SessionState::Running
        | relay_bus::types::SessionState::Idle
        | relay_bus::types::SessionState::Blocked => None,
        _ => {
            return Err(BusError::conflict(
                "task.session_state",
                format!(
                    "session {name} cannot be dispatched while {}",
                    sessions::state_str(row.session.state)
                ),
            ))
        }
    };
    if start {
        if let Some(op) = launch_op {
            ctx.invoke_registered(op, json!({"session":name}))?;
        }
        for id in newly_current {
            if let Some(assigned) = sessions::by_id(ctx.tx(), id)? {
                if matches!(assigned.session.state, relay_bus::types::SessionState::Idle | relay_bus::types::SessionState::Running | relay_bus::types::SessionState::Blocked) {
                    if let Some(task_id) = assigned.session.task_id { crate::handlers::session::announce_assignment(ctx, &assigned.session, task_id, false)?; }
                }
            }
        }
    }
    let task = get_task(ctx.tx(), active.id, false)?;
    let launched = sessions::by_name(ctx.tx(), &name)?.session;
    ctx.set_project(task.project_id);
    ctx.emit("session.changed", serde_json::to_value(&launched).bus()?);
    emit_task(ctx, &task)?;
    Ok((task, launched))
}

pub fn register(e: &mut Engine) {
    e.register::<Create>(|ctx: &mut Ctx, p| {
        crate::handlers::workspace::get_project(ctx.tx(), p.project_id)?;
        let title = p.title.trim();
        if title.is_empty() { return Err(BusError::invalid("task.title", "task title cannot be empty")) }
        assert_module(ctx.tx(), p.project_id, p.module_id)?;
        if let Some(parent_id) = p.parent_id { assert_parent(ctx.tx(), p.project_id, None, parent_id)?; }
        let column = p.column.unwrap_or(Column::Backlog);
        let position = next_position(ctx.tx(), p.project_id, column)?;
        ctx.tx().execute(
            "INSERT INTO tasks(project_id,module_id,title,body,changelog,col,position,state,priority,size,kind,parent_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",
            params![p.project_id,p.module_id,title,p.body.unwrap_or_default(),p.changelog.unwrap_or_default(),column_str(column),position,state_str(p.state.unwrap_or(TaskState::None)),priority_str(p.priority.unwrap_or(Priority::Medium)),p.size.map(size_str),type_str(p.task_type.unwrap_or(TaskType::Task)),p.parent_id,ctx.now],
        ).bus()?;
        let id = ctx.tx().last_insert_rowid();
        let mut task = get_task(ctx.tx(), id, false)?;
        if let Some(labels) = p.labels.as_ref() { set_labels(ctx, &task, labels)?; }
        for input in p.attachments.unwrap_or_default() {
            let bytes = base64::engine::general_purpose::STANDARD.decode(&input.bytes_b64)
                .map_err(|_| BusError::invalid("task.attachment_base64", "attachment bytes_b64 is invalid"))?;
            attach_bytes(ctx, &task, &input.name, &input.mime, &bytes)?;
        }
        task = get_task(ctx.tx(), id, false)?;
        ctx.set_undo("task.delete", json!({"task_id": id}), Some(json!({"updated_at": task.updated_at})));
        emit_task(ctx, &task)?;
        Ok(task)
    });

    e.register::<Activity>(|ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let limit = p.limit.unwrap_or(100).clamp(1, 500) as usize;
        let mut stmt = ctx
            .tx()
            .prepare_cached(
                "SELECT id FROM audit WHERE project_id = ?1 AND (?3 IS NULL OR id < ?3)
             AND ((op LIKE 'task.%' AND json_extract(payload, '$.task_id') = ?2)
               OR (op = 'task.create' AND json_extract(result_summary, '$.id') = ?2)
               OR json_extract(result_summary, '$.task.id') = ?2)
             ORDER BY id DESC LIMIT ?4",
            )
            .bus()?;
        let ids = stmt
            .query_map(
                params![task.project_id, task.id, p.before_audit, (limit + 1) as i64],
                |row| row.get::<_, i64>(0),
            )
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        let next_audit = (ids.len() > limit).then(|| ids[limit - 1]);
        let history = ids
            .into_iter()
            .take(limit)
            .map(|id| {
                crate::audit::get(ctx.tx(), id).bus().and_then(|row| {
                    row.ok_or_else(|| BusError::internal("task audit row disappeared"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut stmt = ctx
            .tx()
            .prepare_cached(
                "SELECT * FROM messages WHERE project_id = ?1 AND re_task = ?2
             AND (?3 IS NULL OR id < ?3) ORDER BY id DESC LIMIT ?4",
            )
            .bus()?;
        let mut messages = stmt
            .query_map(
                params![
                    task.project_id,
                    task.id,
                    p.before_message,
                    (limit + 1) as i64
                ],
                |row| {
                    Ok(relay_bus::types::Message {
                        id: row.get("id")?,
                        project_id: row.get("project_id")?,
                        from: row.get("from_session")?,
                        to: row.get("to_spec")?,
                        text: row.get("text")?,
                        re_task: row.get("re_task")?,
                        priority: row.get::<_, i64>("priority")? != 0,
                        sent_at: row.get("sent_at")?,
                        acked_at: None,
                    })
                },
            )
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        let next_message = (messages.len() > limit).then(|| messages[limit - 1].id);
        messages.truncate(limit);
        Ok(ActivityOut {
            history,
            messages,
            next_audit,
            next_message,
        })
    });
    e.register::<Get>(|ctx, p| get_task(ctx.tx(), p.task_id, false));

    e.register::<List>(|ctx, p| {
        if let Some(project_id) = p.project_id { crate::handlers::workspace::get_project(ctx.tx(), project_id)?; }
        if let Some(sort) = p.sort.as_deref() {
            if !matches!(sort, "column" | "priority" | "updated") { return Err(BusError::invalid("task.sort", "sort must be column, priority, or updated")) }
        }
        let mut sql = String::from("SELECT * FROM tasks WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !p.include_deleted.unwrap_or(false) { sql.push_str(" AND deleted_at IS NULL"); }
        if let Some(v)=p.project_id { sql.push_str(" AND project_id=?"); args.push(Box::new(v)); }
        if let Some(v)=p.column { sql.push_str(" AND col=?"); args.push(Box::new(column_str(v).to_string())); }
        if let Some(v)=p.state { sql.push_str(" AND state=?"); args.push(Box::new(state_str(v).to_string())); }
        if let Some(v)=p.module_id { sql.push_str(" AND module_id=?"); args.push(Box::new(v)); }
        if let Some(v)=p.priority { sql.push_str(" AND priority=?"); args.push(Box::new(priority_str(v).to_string())); }
        if let Some(v)=p.task_type { sql.push_str(" AND kind=?"); args.push(Box::new(type_str(v).to_string())); }
        match p.parent_id {
            Some(Some(v)) => { sql.push_str(" AND parent_id=?"); args.push(Box::new(v)); }
            Some(None) => sql.push_str(" AND parent_id IS NULL"),
            None => {}
        }
        if let Some(v)=p.label { sql.push_str(" AND EXISTS(SELECT 1 FROM task_labels tl JOIN labels l ON l.id=tl.label_id WHERE tl.task_id=tasks.id AND l.name=? COLLATE NOCASE)"); args.push(Box::new(normalise_label(&v)?)); }
        if let Some(v)=p.session { sql.push_str(" AND EXISTS(SELECT 1 FROM task_sessions ts JOIN sessions s ON s.id=ts.session_id WHERE ts.task_id=tasks.id AND s.name=?)"); args.push(Box::new(v)); }
        match p.sort.as_deref().unwrap_or("column") {
            "priority" => sql.push_str(" ORDER BY CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 ELSE 3 END,updated_at DESC,id"),
            "updated" => sql.push_str(" ORDER BY updated_at DESC,id DESC"),
            _ => sql.push_str(" ORDER BY CASE col WHEN 'backlog' THEN 0 WHEN 'in_review' THEN 1 WHEN 'ready' THEN 2 WHEN 'active' THEN 3 ELSE 4 END,position,id"),
        }
        // A dozen filter shapes at most, and the store's statement cache holds 128: the built
        // statement is compiled once per shape, not once per call.
        let mut stmt=ctx.tx().prepare_cached(&sql).bus()?;
        let mut rows=stmt.query(rusqlite::params_from_iter(args.iter().map(|v| v.as_ref()))).bus()?;
        let mut tasks=Vec::new();
        while let Some(row)=rows.next().bus()? { tasks.push(row_task(ctx.tx(), row).bus()?); }
        Ok(ListOut { tasks })
    });

    e.register::<Update>(|ctx: &mut Ctx, p| {
        let before=get_task(ctx.tx(),p.task_id,false)?;
        if ctx.actor.is_agent() && (p.title.is_some() || p.priority.is_some() || p.size.is_some() || p.module_id.is_some() || p.state.is_some() || p.task_type.is_some()) {
            return Err(BusError::refused("actor.allowlist", "agents may update only their task body and changelog"));
        }
        if let Some(expected) = &p.expected {
            let current = serde_json::to_value(&before).bus()?;
            for (field, value) in expected {
                if !["title", "body", "priority", "size", "module_id", "state", "changelog", "type"].contains(&field.as_str()) {
                    return Err(BusError::invalid("task.expected_field", format!("{field} is not an editable task field")));
                }
                if current.get(field) != Some(value) {
                    return Err(BusError::conflict("task.edit_conflict", format!("Task {field} changed elsewhere; your draft was not saved")));
                }
            }
        }
        let title=p.title.as_deref().unwrap_or(&before.title).trim();
        if title.is_empty() { return Err(BusError::invalid("task.title", "task title cannot be empty")) }
        let module_id=p.module_id.unwrap_or(before.module_id);
        assert_module(ctx.tx(),before.project_id,module_id)?;
        let size=p.size.unwrap_or(before.size);
        ctx.tx().execute("UPDATE tasks SET title=?1,body=?2,priority=?3,size=?4,module_id=?5,state=?6,changelog=?7,kind=?8,updated_at=?9 WHERE id=?10",
            params![title,p.body.as_deref().unwrap_or(&before.body),priority_str(p.priority.unwrap_or(before.priority)),size.map(size_str),module_id,state_str(p.state.unwrap_or(before.state)),p.changelog.as_deref().unwrap_or(&before.changelog),type_str(p.task_type.unwrap_or(before.task_type)),ctx.now,before.id]).bus()?;
        let task=get_task(ctx.tx(),before.id,false)?;
        ctx.set_undo("task.update",json!({"task_id":before.id,"title":before.title,"body":before.body,"priority":priority_str(before.priority),"size":before.size.map(size_str),"module_id":before.module_id,"state":state_str(before.state),"changelog":before.changelog,"type":type_str(before.task_type)}),Some(json!({"updated_at":task.updated_at})));
        emit_task(ctx,&task)?; Ok(task)
    });

    e.register::<Move>(|ctx: &mut Ctx,p| {
        let before=get_task(ctx.tx(),p.task_id,false)?;
        if ctx.actor.is_agent() && !(before.column==Column::Active && p.column==Column::InReview) { return Err(BusError::conflict("task.column_transition","agents may only move their active task to in_review")) }
        if p.column==Column::Done { return Err(BusError::conflict("task.column_transition","move to done through task.approve")) }
        // An explicit position is the index the task ends at in its column; the others shift
        // around it, so a card can be dropped between two others. Without one it goes last.
        let index=column_index(ctx.tx(),&before)?;
        let position=match p.position { Some(at) => open_slot(ctx.tx(),before.project_id,p.column,before.id,at)?, None => next_position(ctx.tx(),before.project_id,p.column)? };
        let moved_state = if before.column == Column::Done && p.column == Column::InReview { TaskState::AwaitingReview } else { before.state };
        ctx.tx().execute("UPDATE tasks SET col=?1,position=?2,state=?3,updated_at=?4 WHERE id=?5",params![column_str(p.column),position,state_str(moved_state),ctx.now,before.id]).bus()?;
        let task=get_task(ctx.tx(),before.id,false)?;
        ctx.set_undo("task.move",json!({"task_id":before.id,"column":column_str(before.column),"position":index}),Some(json!({"updated_at":task.updated_at})));
        emit_task(ctx,&task)?; Ok(task)
    });

    e.register::<Delete>(|ctx: &mut Ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        ctx.tx()
            .execute(
                "UPDATE tasks SET deleted_at=?1,updated_at=?1 WHERE id=?2",
                params![ctx.now, task.id],
            )
            .bus()?;
        ctx.set_project(task.project_id);
        ctx.set_undo(
            "task.restore",
            json!({"task_id":task.id}),
            Some(json!({"updated_at":ctx.now})),
        );
        ctx.emit(
            "task.deleted",
            json!({"id":task.id,"project_id":task.project_id}),
        );
        Ok(Empty {})
    });
    e.register::<Restore>(|ctx: &mut Ctx, p| {
        let before = get_task(ctx.tx(), p.task_id, true)?;
        if before.deleted_at.is_none() {
            return Err(BusError::conflict(
                "task.not_deleted",
                format!("task {} is not deleted", before.id),
            ));
        }
        ctx.tx()
            .execute(
                "UPDATE tasks SET deleted_at=NULL,updated_at=?1 WHERE id=?2",
                params![ctx.now, before.id],
            )
            .bus()?;
        let task = get_task(ctx.tx(), before.id, false)?;
        ctx.set_undo(
            "task.delete",
            json!({"task_id":task.id}),
            Some(json!({"updated_at":task.updated_at})),
        );
        emit_task(ctx, &task)?;
        Ok(task)
    });
    e.register::<LinkCommit>(|ctx: &mut Ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        link_commit(ctx.tx(), task.id, &p.sha, p.branch.as_deref(), &ctx.now)?;
        ctx.tx()
            .execute(
                "UPDATE tasks SET updated_at=?1 WHERE id=?2",
                params![ctx.now, task.id],
            )
            .bus()?;
        let task = get_task(ctx.tx(), task.id, false)?;
        emit_task(ctx, &task)?;
        Ok(task)
    });
    e.register::<ChangelogWrite>(|ctx: &mut Ctx, p| {
        let before = get_task(ctx.tx(), p.task_id, false)?;
        ctx.tx()
            .execute(
                "UPDATE tasks SET changelog=?1,updated_at=?2 WHERE id=?3",
                params![p.text, ctx.now, before.id],
            )
            .bus()?;
        let task = get_task(ctx.tx(), before.id, false)?;
        ctx.set_undo(
            "task.changelog.write",
            json!({"task_id":before.id,"text":before.changelog}),
            Some(json!({"updated_at":task.updated_at})),
        );
        emit_task(ctx, &task)?;
        Ok(task)
    });
    e.register::<Attach>(|ctx: &mut Ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let attachment = match (p.path, p.name, p.mime, p.bytes_b64) {
            (Some(path), None, None, None) => {
                let src = PathBuf::from(&path);
                if !src.is_file() {
                    return Err(BusError::not_found(
                        "task.attachment_path",
                        format!("no file {path}"),
                    ));
                }
                let bytes = std::fs::read(&src).bus()?;
                let name = src
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment");
                attach_bytes(ctx, &task, name, "application/octet-stream", &bytes)?
            }
            (None, Some(name), Some(mime), Some(encoded)) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| {
                        BusError::invalid(
                            "task.attachment_base64",
                            "attachment bytes_b64 is invalid",
                        )
                    })?;
                attach_bytes(ctx, &task, &name, &mime, &bytes)?
            }
            _ => {
                return Err(BusError::invalid(
                    "task.attachment_input",
                    "provide either path or name + mime + bytes_b64",
                ))
            }
        };
        emit_task(ctx, &get_task(ctx.tx(), task.id, false)?)?;
        Ok(attachment)
    });
    e.register::<Detach>(|ctx: &mut Ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let attachment = ctx
            .tx()
            .query_row(
                "SELECT * FROM attachments WHERE id=?1 AND task_id=?2",
                params![p.attachment_id, task.id],
                attachment_row,
            )
            .optional()
            .bus()?
            .ok_or_else(|| {
                BusError::not_found(
                    "task.attachment_not_found",
                    format!("no attachment {} on task {}", p.attachment_id, task.id),
                )
            })?;
        ctx.tx()
            .execute("DELETE FROM attachments WHERE id=?1", [attachment.id])
            .bus()?;
        ctx.set_undo(
            "task.attach",
            json!({"task_id":task.id,"path":attachment.path}),
            None,
        );
        emit_task(ctx, &get_task(ctx.tx(), task.id, false)?)?;
        Ok(Empty {})
    });
    e.register::<Dispatch>(|ctx: &mut Ctx, p| {
        if p.session.is_some() == p.create.is_some() {
            return Err(BusError::invalid(
                "task.dispatch_target",
                "provide exactly one of session or create",
            ));
        }
        let fanout = p.fanout.unwrap_or(false);
        let start = p.start.unwrap_or(true);
        if fanout && p.create.is_none() {
            return Err(BusError::invalid(
                "task.fanout_target",
                "fanout needs create: one session cannot hold several tasks",
            ));
        }
        let template = p.create.clone();
        let (task, session) = dispatch_task(ctx, p.task_id, p.session, p.create, start)?;
        let mut fanned = Vec::new();
        if fanout {
            for id in descendants(ctx.tx(), task.id)? {
                let child = get_task(ctx.tx(), id, false)?;
                if child.column == Column::Done {
                    continue;
                }
                let (task, session) = dispatch_task(ctx, id, None, template.clone(), start)?;
                fanned.push(Dispatched { task, session });
            }
        }
        // Re-read: fanning out to a child does not change the parent row, but its roll-up and
        // its children's states did move, and the caller reads them off this Task.
        let task = get_task(ctx.tx(), task.id, false)?;
        Ok(DispatchOut {
            task,
            session,
            fanned,
        })
    });
    e.register::<Approve>(|ctx: &mut Ctx, p| {
        let before = get_task(ctx.tx(), p.task_id, false)?;
        if before.column == Column::Done {
            return Err(BusError::conflict(
                "task.column_transition",
                "task is already done",
            ));
        }
        let (sha, branch) = if let Some(sha) = p.sha {
            (sha, None)
        } else {
            let session_id = ctx.tx().query_row(
                "SELECT session_id FROM task_sessions WHERE task_id=?1 ORDER BY ord DESC,session_id DESC LIMIT 1",
                [before.id],
                |row| row.get::<_, Id>(0),
            ).optional().bus()?;
            let project = crate::handlers::workspace::get_project(ctx.tx(), before.project_id)?;
            let repo = gix::open(&project.path)
                .map_err(|e| BusError::unavailable("git.head", e.to_string()))?;
            // Approval is a historical task operation. The assigned session may already be
            // closed and its worktree removed, but session.close deliberately keeps its branch.
            // Resolve that recorded branch from the project repository instead of requiring a
            // currently live session (or accidentally finding a newer session with the same name).
            if let Some(session_id) = session_id {
                let session = sessions::by_id(ctx.tx(), session_id)?
                    .ok_or_else(|| BusError::internal("task session vanished"))?;
                let sha = repo
                    .rev_parse_single(format!("refs/heads/{}", session.session.branch).as_str())
                    .map_err(|e| BusError::unavailable("git.head", e.to_string()))?
                    .detach()
                    .to_string();
                (sha, Some(session.session.branch))
            } else {
                // Work completed directly in the project checkout still gets the same Done
                // invariant: link the checkout HEAD even though Relay never dispatched it.
                let branch = repo.head_name().ok().flatten().map(|name| name.shorten().to_string());
                let sha = repo.head_id()
                    .map_err(|e| BusError::unavailable("git.head", e.to_string()))?
                    .to_string();
                (sha, branch)
            }
        };
        link_commit(ctx.tx(), before.id, &sha, branch.as_deref(), &ctx.now)?;
        let index = column_index(ctx.tx(), &before)?;
        ctx.tx()
            .execute(
                "UPDATE tasks SET col='done',position=?1,state='none',updated_at=?2 WHERE id=?3",
                params![
                    next_position(ctx.tx(), before.project_id, Column::Done)?,
                    ctx.now,
                    before.id
                ],
            )
            .bus()?;
        let task = get_task(ctx.tx(), before.id, false)?;
        ctx.set_undo(
            "task.move",
            json!({"task_id":before.id,"column":column_str(before.column),"position":index}),
            Some(json!({"updated_at":task.updated_at})),
        );
        emit_task(ctx, &task)?;
        Ok(task)
    });
    e.register::<ParentSet>(|ctx: &mut Ctx, p| {
        let before = get_task(ctx.tx(), p.task_id, false)?;
        if let Some(parent_id) = p.parent_id {
            assert_parent(ctx.tx(), before.project_id, Some(before.id), parent_id)?;
        }
        let position = p.position.unwrap_or(before.position).max(0);
        ctx.tx()
            .execute(
                "UPDATE tasks SET parent_id=?1,position=?2,updated_at=?3 WHERE id=?4",
                params![p.parent_id, position, ctx.now, before.id],
            )
            .bus()?;
        let task = get_task(ctx.tx(), before.id, false)?;
        ctx.set_undo(
            "task.parent.set",
            json!({"task_id":before.id,"parent_id":before.parent_id,"position":before.position}),
            Some(json!({"updated_at":task.updated_at})),
        );
        emit_task(ctx, &task)?;
        // The old and the new parent both changed their roll-up; the board reads roll-ups off
        // the parent row, so both have to be re-emitted or a stale n/m survives on screen.
        for parent in [before.parent_id, task.parent_id].into_iter().flatten() {
            let parent = get_task(ctx.tx(), parent, false)?;
            emit_task(ctx, &parent)?;
        }
        Ok(task)
    });

    e.register::<Children>(|ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let ids = if p.recursive.unwrap_or(false) {
            descendants(ctx.tx(), task.id)?
        } else {
            task.children.clone()
        };
        let mut tasks = Vec::with_capacity(ids.len());
        for id in ids {
            tasks.push(get_task(ctx.tx(), id, false)?);
        }
        Ok(ListOut { tasks })
    });

    e.register::<LabelAdd>(|ctx: &mut Ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let name = normalise_label(&p.label)?;
        let id = label_id(ctx.tx(), task.project_id, &name, &ctx.now)?;
        let added = ctx
            .tx()
            .execute(
                "INSERT OR IGNORE INTO task_labels(task_id,label_id) VALUES (?1,?2)",
                params![task.id, id],
            )
            .bus()?;
        touch(ctx, task.id)?;
        let task = get_task(ctx.tx(), task.id, false)?;
        if added > 0 {
            ctx.set_undo(
                "task.label.remove",
                json!({"task_id":task.id,"label":name}),
                Some(json!({"updated_at":task.updated_at})),
            );
        }
        emit_task(ctx, &task)?;
        Ok(task)
    });

    e.register::<LabelRemove>(|ctx: &mut Ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let name = normalise_label(&p.label)?;
        let removed = ctx.tx().execute(
            "DELETE FROM task_labels WHERE task_id=?1 AND label_id IN (SELECT id FROM labels WHERE project_id=?2 AND name=?3 COLLATE NOCASE)",
            params![task.id, task.project_id, name],
        ).bus()?;
        touch(ctx, task.id)?;
        let task = get_task(ctx.tx(), task.id, false)?;
        if removed > 0 {
            ctx.set_undo(
                "task.label.add",
                json!({"task_id":task.id,"label":name}),
                Some(json!({"updated_at":task.updated_at})),
            );
        }
        emit_task(ctx, &task)?;
        Ok(task)
    });

    e.register::<LabelList>(|ctx, p| {
        crate::handlers::workspace::get_project(ctx.tx(), p.project_id)?;
        let mut stmt = ctx
            .tx()
            .prepare_cached("SELECT id,project_id,name,created_at FROM labels WHERE project_id=?1 ORDER BY name COLLATE NOCASE")
            .bus()?;
        let labels = stmt
            .query_map([p.project_id], |r| {
                Ok(Label { id: r.get(0)?, project_id: r.get(1)?, name: r.get(2)?, created_at: r.get(3)? })
            })
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        Ok(LabelListOut { labels })
    });

    e.register::<Relate>(|ctx: &mut Ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let other = assert_relatable(ctx.tx(), &task, p.other_id)?;
        let rel = relation_str(p.relation);
        // duplicate_of is single-valued: a task duplicates one other task, so a second call
        // replaces the first rather than stacking edges.
        let replaced = if p.relation == TaskRelation::DuplicateOf { task.duplicate_of } else { None };
        if replaced.is_some() {
            ctx.tx().execute("DELETE FROM task_relations WHERE from_task=?1 AND rel='duplicate_of'", [task.id]).bus()?;
        }
        ctx.tx()
            .execute(
                "INSERT OR IGNORE INTO task_relations(from_task,to_task,rel,created_at) VALUES (?1,?2,?3,?4)",
                params![task.id, other, rel, ctx.now],
            )
            .bus()?;
        touch(ctx, task.id)?;
        let task = get_task(ctx.tx(), task.id, false)?;
        let undo = match replaced {
            Some(prior) => json!({"task_id":task.id,"relation":rel,"other_id":prior}),
            None => json!({"task_id":task.id,"relation":rel,"other_id":other}),
        };
        ctx.set_undo(
            if replaced.is_some() { "task.relate" } else { "task.unrelate" },
            undo,
            Some(json!({"updated_at":task.updated_at})),
        );
        emit_task(ctx, &task)?;
        emit_task(ctx, &get_task(ctx.tx(), other, false)?)?;
        Ok(task)
    });

    e.register::<Unrelate>(|ctx: &mut Ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let other = assert_relatable(ctx.tx(), &task, p.other_id)?;
        let rel = relation_str(p.relation);
        let removed = ctx
            .tx()
            .execute(
                "DELETE FROM task_relations WHERE from_task=?1 AND to_task=?2 AND rel=?3",
                params![task.id, other, rel],
            )
            .bus()?;
        touch(ctx, task.id)?;
        let task = get_task(ctx.tx(), task.id, false)?;
        if removed > 0 {
            ctx.set_undo(
                "task.relate",
                json!({"task_id":task.id,"relation":rel,"other_id":other}),
                Some(json!({"updated_at":task.updated_at})),
            );
        }
        emit_task(ctx, &task)?;
        emit_task(ctx, &get_task(ctx.tx(), other, false)?)?;
        Ok(task)
    });

    e.register::<CopyText>(|ctx, p| {
        let task = get_task(ctx.tx(), p.task_id, false)?;
        let mut text = format!("#{} {}", task.id, task.title);
        if !task.body.trim().is_empty() {
            text.push_str("\n\n");
            text.push_str(&task.body)
        }
        Ok(CopyTextOut { text })
    });
}
