//! `app.import.v3` (SPEC §14, BUS.md §10.2): one-time import of a v3 per-repo `.relay/`
//! (relay.db + notes.json) into a v4 project, plus the project's guardrail caps from v3's
//! `sessions.json` (v3 "sessions" are v4 projects).
//!
//! v3 → v4 mapping (BUS.md §14 for the dropped bits):
//! - columns were free-form (`Backlog / Queue / Ready / Done / In review`) → the five fixed
//!   ones by name; unknown names land in `backlog` with a warning.
//! - `description` + `acceptance_criteria` + `target_files` + comments → one `body` with
//!   markdown sections; comments under a `--- comments ---` marker.
//! - `size_hint` small/medium/large → S/M/L; `priority` medium stays medium (confirmed 2026-08-17).
//! - `runs` are not imported (v4 has sessions, not runs); their count is a warning.
//! - attachments are copied into the store's `attachments/<task_id>/` if the file still exists.

use crate::engine::{Ctx, IntoBus};
use crate::handlers::workspace::get_project;
use relay_bus::error::BusError;
use relay_bus::ops::app::{ImportCounts, ImportIdMap, ImportV3In, ImportV3Out};
use relay_bus::types::{Column, Id};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn epoch_to_ts(secs: i64) -> String {
    jiff::Timestamp::from_second(secs).map(|t| t.to_string()).unwrap_or_else(|_| crate::time::now())
}

fn map_column(name: &str) -> Option<Column> {
    match name.trim().to_ascii_lowercase().replace('_', " ").as_str() {
        "backlog" => Some(Column::Backlog),
        "queue" | "queued" | "ready" | "todo" => Some(Column::Ready),
        "in review" | "review" | "reviewing" => Some(Column::InReview),
        "active" | "running" | "in progress" | "doing" => Some(Column::Active),
        "done" | "approved" | "complete" | "completed" => Some(Column::Done),
        _ => None,
    }
}

fn column_str(c: Column) -> &'static str {
    match c {
        Column::Backlog => "backlog",
        Column::InReview => "in_review",
        Column::Ready => "ready",
        Column::Active => "active",
        Column::Done => "done",
    }
}

fn map_priority(p: &str) -> &'static str {
    match p.trim().to_ascii_lowercase().as_str() {
        "low" => "low",
        "high" => "high",
        "urgent" | "critical" => "urgent",
        _ => "medium",
    }
}

fn map_size(s: Option<&str>) -> Option<&'static str> {
    match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("small") | Some("s") | Some("xs") => Some("S"),
        Some("medium") | Some("m") => Some("M"),
        Some("large") | Some("l") | Some("xl") => Some("L"),
        _ => None,
    }
}

fn map_state(status: &str) -> &'static str {
    match status.trim().to_ascii_lowercase().as_str() {
        "running" | "dispatched" => "running",
        "review" | "awaiting_review" | "in_review" => "awaiting_review",
        "failed" | "error" => "failed",
        "blocked" => "blocked",
        _ => "none",
    }
}

fn mime_for(name: &str) -> &'static str {
    match Path::new(name).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("txt") | Some("md") => "text/plain",
        Some("json") => "application/json",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Where the source points: `<dir>/.relay`, `<dir>/.relay/relay.db`, or `<repo>`.
fn resolve_source(source: &str) -> Result<(PathBuf, PathBuf), BusError> {
    let p = Path::new(source);
    let dir = if p.is_file() {
        p.parent().map(Path::to_path_buf).unwrap_or_default()
    } else if p.join("relay.db").is_file() {
        p.to_path_buf()
    } else if p.join(".relay").join("relay.db").is_file() {
        p.join(".relay")
    } else {
        return Err(BusError::not_found("import.source", format!("{source} is not a v3 .relay dir, relay.db, or a repo containing .relay/relay.db")));
    };
    let db = dir.join("relay.db");
    if !db.is_file() {
        return Err(BusError::not_found("import.source", format!("{} has no relay.db", dir.display())));
    }
    Ok((dir, db))
}

fn open_v3(db: &Path) -> Result<Connection, BusError> {
    let c = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| BusError::invalid("import.source", format!("cannot open {}: {e}", db.display())))?;
    let has_tasks: bool = c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tasks'", [], |r| r.get::<_, i64>(0)).map(|n| n > 0).bus()?;
    if !has_tasks {
        return Err(BusError::invalid("import.source", format!("{} has no tasks table — not a v3 store", db.display())));
    }
    Ok(c)
}

struct V3Task {
    id: i64,
    title: String,
    description: String,
    acceptance: String,
    target_files: String,
    size_hint: Option<String>,
    status: String,
    column_id: i64,
    module_id: Option<i64>,
    priority: String,
    created_at: i64,
    updated_at: i64,
}

pub fn import(ctx: &mut Ctx, p: ImportV3In) -> Result<ImportV3Out, BusError> {
    let project = get_project(ctx.tx(), p.project_id)?;
    ctx.set_project(project.id);
    let dry = p.dry_run.unwrap_or(false);
    let (dir, db) = resolve_source(&p.source)?;
    let source_key = format!("import_v3:{}", std::fs::canonicalize(&dir).unwrap_or(dir.clone()).display());
    let done: Option<String> = ctx.tx().query_row("SELECT value FROM meta WHERE key = ?1", [&source_key], |r| r.get(0)).optional().bus()?;
    if let Some(when) = done {
        return Err(BusError::conflict("import.already_done", format!("{} was already imported at {when}", dir.display()))
            .with_hint("v3 import is one-time by design (SPEC §14); restore a backup if you need to redo it"));
    }
    let v3 = open_v3(&db)?;
    let mut warnings = Vec::new();
    let mut id_map = ImportIdMap { tasks: BTreeMap::new(), modules: BTreeMap::new(), notes: BTreeMap::new() };
    let mut counts = ImportCounts { tasks: 0, modules: 0, notes: 0, sessions: 0 };
    let tx = ctx.tx();
    let now = ctx.now.clone();

    // ---- columns
    let mut columns: BTreeMap<i64, Column> = BTreeMap::new();
    {
        let mut st = v3.prepare_cached("SELECT id, name FROM columns").bus()?;
        let rows: Vec<(i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).bus()?.collect::<Result<_, _>>().bus()?;
        for (id, name) in rows {
            match map_column(&name) {
                Some(c) => { columns.insert(id, c); }
                None => { warnings.push(format!("column {name:?} has no v4 equivalent; its tasks go to backlog")); columns.insert(id, Column::Backlog); }
            }
        }
    }

    // ---- modules
    {
        let mut st = v3.prepare_cached("SELECT id, name, created_at FROM modules ORDER BY id").bus()?;
        let rows: Vec<(i64, String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).bus()?.collect::<Result<_, _>>().bus()?;
        for (i, (old, name, created)) in rows.into_iter().enumerate() {
            let created = epoch_to_ts(created);
            let new_id: Id = if dry { -(old) } else {
                tx.execute("INSERT INTO modules(project_id, name, icon, priority, ord, created_at, updated_at) VALUES (?1, ?2, NULL, 'medium', ?3, ?4, ?4)",
                    params![project.id, name, i as i64, created]).bus()?;
                tx.last_insert_rowid()
            };
            id_map.modules.insert(old.to_string(), new_id);
            counts.modules += 1;
        }
    }

    // ---- tasks (+ comments, attachments)
    let tasks: Vec<V3Task> = {
        let mut st = v3.prepare_cached("SELECT id, title, description, acceptance_criteria, target_files, size_hint, status, column_id, module_id, priority, created_at, updated_at FROM tasks ORDER BY id").bus()?;
        let rows = st.query_map([], |r| Ok(V3Task {
            id: r.get(0)?, title: r.get(1)?, description: r.get(2)?, acceptance: r.get(3)?, target_files: r.get(4)?,
            size_hint: r.get(5)?, status: r.get(6)?, column_id: r.get(7)?, module_id: r.get(8)?, priority: r.get(9)?,
            created_at: r.get(10)?, updated_at: r.get(11)?,
        })).bus()?.collect::<Result<_, _>>();
        rows.bus()?
    };
    let attach_root = ctx.engine().store.path().parent().map(|d| d.join("attachments")).unwrap_or_else(|| PathBuf::from("attachments"));
    let mut positions: BTreeMap<&'static str, i64> = BTreeMap::new();
    for t in &tasks {
        let col = columns.get(&t.column_id).copied().unwrap_or(Column::Backlog);
        let col_s = column_str(col);
        let pos = positions.entry(col_s).or_insert(0);
        let position = *pos;
        *pos += 1;
        // body
        let mut body = t.description.trim_end().to_string();
        if !t.acceptance.trim().is_empty() {
            body.push_str("\n\n## Acceptance criteria\n\n");
            body.push_str(t.acceptance.trim());
        }
        if !t.target_files.trim().is_empty() {
            body.push_str("\n\n## Target files\n\n");
            body.push_str(t.target_files.trim());
        }
        let comments: Vec<(String, i64)> = {
            let mut st = v3.prepare_cached("SELECT body, created_at FROM comments WHERE task_id = ?1 ORDER BY id").bus()?;
            let rows = st.query_map([t.id], |r| Ok((r.get(0)?, r.get(1)?))).bus()?.collect::<Result<_, _>>();
            rows.bus()?
        };
        if !comments.is_empty() {
            body.push_str("\n\n--- comments ---\n");
            for (c, at) in &comments {
                body.push_str(&format!("\n[{}] {}\n", epoch_to_ts(*at), c.trim()));
            }
        }
        let module_new = t.module_id.and_then(|m| id_map.modules.get(&m.to_string()).copied());
        if t.module_id.is_some() && module_new.is_none() {
            warnings.push(format!("task {} referenced missing module {:?}", t.id, t.module_id));
        }
        let new_id: Id = if dry { -(t.id) } else {
            tx.execute(
                "INSERT INTO tasks(project_id, module_id, title, body, changelog, col, position, state, priority, size, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![project.id, module_new, t.title, body, col_s, position, map_state(&t.status), map_priority(&t.priority),
                        map_size(t.size_hint.as_deref()), epoch_to_ts(t.created_at), epoch_to_ts(t.updated_at)],
            ).bus()?;
            tx.last_insert_rowid()
        };
        id_map.tasks.insert(t.id.to_string(), new_id);
        counts.tasks += 1;

        // attachments
        let atts: Vec<(String, String, i64)> = {
            let mut st = v3.prepare_cached("SELECT path, original_name, created_at FROM attachments WHERE task_id = ?1 ORDER BY id").bus()?;
            let rows = st.query_map([t.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).bus()?.collect::<Result<_, _>>();
            rows.bus()?
        };
        for (path, name, at) in atts {
            // v3 wrote paths relative to the repo root (`.relay/attachments/t<id>/...`); older
            // rows were relative to `.relay/`; be liberal.
            let candidates: Vec<PathBuf> = if Path::new(&path).is_absolute() {
                vec![PathBuf::from(&path)]
            } else {
                let mut v = vec![dir.join(&path)];
                if let Some(repo) = dir.parent() { v.push(repo.join(&path)); }
                v
            };
            let Some(src) = candidates.iter().find(|c| c.is_file()).cloned() else {
                warnings.push(format!("attachment {name:?} of task {} not found (tried {})", t.id,
                    candidates.iter().map(|c| c.display().to_string()).collect::<Vec<_>>().join(", ")));
                continue;
            };
            let bytes = std::fs::metadata(&src).map(|m| m.len() as i64).unwrap_or(0);
            if !dry {
                let dest_dir = attach_root.join(new_id.to_string());
                std::fs::create_dir_all(&dest_dir).bus()?;
                let dest = dest_dir.join(&name);
                std::fs::copy(&src, &dest).bus()?;
                tx.execute("INSERT INTO attachments(task_id, name, mime, bytes, path, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![new_id, name, mime_for(&name), bytes, dest.display().to_string(), epoch_to_ts(at)]).bus()?;
            }
        }
    }
    let runs: i64 = v3.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='runs'", [], |r| r.get(0)).unwrap_or(0);
    if runs > 0 {
        let n: i64 = v3.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0)).unwrap_or(0);
        if n > 0 {
            warnings.push(format!("{n} v3 runs not imported (v4 has sessions, not runs; commit links come from your own git history)"));
        }
    }

    // ---- notes.json
    let notes_path = dir.join("notes.json");
    if notes_path.is_file() {
        let raw = std::fs::read_to_string(&notes_path).bus()?;
        match serde_json::from_str::<Value>(&raw) {
            Ok(Value::Array(items)) => {
                for n in items {
                    let old = n.get("id").and_then(Value::as_i64).unwrap_or(0);
                    let title = n.get("title").and_then(Value::as_str).map(str::to_string);
                    let body = n.get("body").and_then(Value::as_str).unwrap_or("").to_string();
                    let pinned = n.get("pinned").and_then(Value::as_bool).unwrap_or(false);
                    let created = n.get("created_at").and_then(Value::as_i64).map(epoch_to_ts).unwrap_or_else(|| now.clone());
                    let updated = n.get("updated_at").and_then(Value::as_i64).map(epoch_to_ts).unwrap_or_else(|| created.clone());
                    let new_id: Id = if dry { -old } else {
                        tx.execute("INSERT INTO notes(project_id, title, body, pinned, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![project.id, title, body, pinned as i64, created, updated]).bus()?;
                        tx.last_insert_rowid()
                    };
                    id_map.notes.insert(old.to_string(), new_id);
                    counts.notes += 1;
                }
            }
            Ok(_) => warnings.push("notes.json is not a list; skipped".into()),
            Err(e) => warnings.push(format!("notes.json unreadable: {e}")),
        }
    }

    // ---- v3 sessions.json → guardrail caps for this project (v3 "sessions" are v4 projects)
    if let Some(cfg) = v3_sessions_json() {
        if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&std::fs::read_to_string(&cfg).unwrap_or_default()) {
            let repo_canon = std::fs::canonicalize(&project.path).map(|p| p.display().to_string()).unwrap_or(project.path.clone());
            for s in items {
                let repo = s.get("repo").and_then(Value::as_str).unwrap_or("");
                let repo_c = std::fs::canonicalize(repo).map(|p| p.display().to_string()).unwrap_or(repo.to_string());
                if repo_c == repo_canon {
                    counts.sessions += 1;
                    let files = s.get("max_files").and_then(Value::as_i64);
                    let lines = s.get("max_lines").and_then(Value::as_i64);
                    if !dry {
                        let path = format!("guardrails.projects.{}.caps", project.id);
                        crate::handlers::settings::set(tx, &path, &json!({ "files": files, "lines": lines }), &now)?;
                        if let Some(name) = s.get("name").and_then(Value::as_str) {
                            if !name.is_empty() && name != project.name {
                                tx.execute("UPDATE projects SET name = ?1, updated_at = ?2 WHERE id = ?3", params![name, now, project.id]).bus()?;
                            }
                        }
                    }
                }
            }
        }
    }

    if !dry {
        tx.execute("INSERT INTO meta(key, value) VALUES (?1, ?2)", params![source_key, now]).bus()?;
        // Bulk import: one re-query hint per entity kind rather than one event per row.
        for ev in ["module.changed", "task.changed", "notes.changed", "project.changed"] {
            ctx.emit(ev, json!({ "project_id": project.id, "bulk": true }));
        }
    }
    Ok(ImportV3Out { counts, id_map, warnings })
}

/// v3 kept its project list at `~/.config/dev.antho.relay/sessions.json`.
fn v3_sessions_json() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RELAY_V3_SESSIONS_JSON") {
        return Some(PathBuf::from(p));
    }
    let base = directories::BaseDirs::new()?;
    let p = base.config_dir().join("dev.antho.relay").join("sessions.json");
    p.is_file().then_some(p)
}
