//! `workspace.*` / `project.*` (BUS.md §10.4). All mutations user-only (registry).

use crate::engine::{Ctx, Engine, IntoBus};
use relay_bus::error::BusError;
use relay_bus::ops::workspace::*;
use relay_bus::types::{Id, Project, Workspace};
use relay_bus::Empty;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn ws_row(r: &Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: r.get("id")?, path: r.get("path")?, name: r.get("name")?, order: r.get("ord")?,
        created_at: r.get("created_at")?, updated_at: r.get("updated_at")?,
    })
}

fn project_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get("id")?, workspace_id: r.get("workspace_id")?, path: r.get("path")?, name: r.get("name")?,
        base_branch: r.get("base_branch")?, build_cmd: r.get("build_cmd")?, run_cmd: r.get("run_cmd")?,
        protected_paths: super::util::json_vec(&r.get::<_, String>("protected_paths")?),
        critical_files: super::util::json_vec(&r.get::<_, String>("critical_files")?),
        order: r.get("ord")?, pinned: r.get::<_, i64>("pinned")? != 0,
        created_at: r.get("created_at")?, updated_at: r.get("updated_at")?,
    })
}

pub fn get_workspace(tx: &Connection, id: Id) -> Result<Workspace, BusError> {
    tx.query_row("SELECT * FROM workspaces WHERE id = ?1", [id], ws_row).optional().bus()?
        .ok_or_else(|| BusError::not_found("workspace.not_found", format!("no workspace {id}")))
}

pub fn get_project(tx: &Connection, id: Id) -> Result<Project, BusError> {
    tx.query_row("SELECT * FROM projects WHERE id = ?1", [id], project_row).optional().bus()?
        .ok_or_else(|| BusError::not_found("project.not_found", format!("no project {id}")))
}

fn canon(path: &str, code: &str) -> Result<PathBuf, BusError> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(BusError::invalid(code, format!("{path:?} must be an absolute path")));
    }
    std::fs::canonicalize(p).map_err(|e| BusError::invalid(code, format!("{path:?}: {e}")))
}

fn canon_workspace(path: &str) -> Result<PathBuf, BusError> {
    let suggested;
    let p = if path.trim().is_empty() {
        suggested = suggested_workspace()?;
        suggested.as_path()
    } else {
        Path::new(path)
    };
    if !p.is_absolute() {
        return Err(BusError::invalid("workspace.path", format!("{path:?} must be an absolute path")));
    }
    if !p.exists() {
        std::fs::create_dir_all(p).map_err(|e| BusError::invalid("workspace.path", format!("cannot create {path:?}: {e}")))?;
    }
    std::fs::canonicalize(p).map_err(|e| BusError::invalid("workspace.path", format!("{path:?}: {e}")))
}

/// A desktop app may inherit a deep build directory as its process CWD. When that directory
/// lives inside a Git checkout, use the checkout's parent so discovery presents the repository
/// itself as a project instead of cloning another copy into its source tree.
fn suggested_workspace() -> Result<PathBuf, BusError> {
    let cwd = std::env::current_dir()
        .map_err(|error| BusError::unavailable("workspace.cwd", error.to_string()))?;
    let cwd = std::fs::canonicalize(&cwd)
        .map_err(|error| BusError::unavailable("workspace.cwd", error.to_string()))?;
    Ok(suggested_workspace_from(&cwd))
}

fn suggested_workspace_from(cwd: &Path) -> PathBuf {
    for ancestor in cwd.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.parent().unwrap_or(ancestor).to_path_buf();
        }
    }
    cwd.to_path_buf()
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

pub fn register(e: &mut Engine) {
    e.register::<WsCreate>(|ctx: &mut Ctx, p| {
        let path = canon_workspace(p.path.as_deref().unwrap_or(""))?;
        if !path.is_dir() {
            return Err(BusError::invalid("workspace.path", format!("{} is not a directory", path.display())));
        }
        let path_s = path.display().to_string();
        let dup: Option<Id> = ctx.tx().query_row("SELECT id FROM workspaces WHERE path = ?1", [&path_s], |r| r.get(0)).optional().bus()?;
        if let Some(id) = dup {
            return Err(BusError::conflict("workspace.exists", format!("{path_s} is already workspace {id}")).with_details(json!({"workspace_id": id})));
        }
        let name = p.name.unwrap_or_else(|| name_of(&path));
        let ord: i64 = ctx.tx().query_row("SELECT COALESCE(MAX(ord), -1) + 1 FROM workspaces", [], |r| r.get(0)).bus()?;
        ctx.tx().execute(
            "INSERT INTO workspaces(path, name, ord, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![path_s, name, ord, ctx.now],
        ).bus()?;
        let ws = get_workspace(ctx.tx(), ctx.tx().last_insert_rowid())?;
        ctx.emit("workspace.changed", serde_json::to_value(&ws).bus()?);
        Ok(ws)
    });
    e.register::<WsDiscover>(|_, p| {
        let path = if p.path.as_deref().is_none_or(|path| path.trim().is_empty()) {
            suggested_workspace()?
        } else {
            let candidate = PathBuf::from(p.path.unwrap());
            if !candidate.is_absolute() {
                return Err(BusError::invalid("workspace.path", "workspace path must be absolute"));
            }
            if !candidate.exists() {
                return Ok(WsDiscoverOut { path: candidate.display().to_string(), repositories: Vec::new() });
            }
            std::fs::canonicalize(&candidate).map_err(|error| BusError::invalid("workspace.path", error.to_string()))?
        };
        if !path.is_dir() {
            return Err(BusError::invalid("workspace.path", format!("{} is not a directory", path.display())));
        }
        Ok(WsDiscoverOut { path: path.display().to_string(), repositories: discover_repositories(&path)? })
    });
    e.register::<WsList>(|ctx, _| {
        let mut stmt = ctx.tx().prepare_cached("SELECT * FROM workspaces ORDER BY ord, id").bus()?;
        let rows = stmt.query_map([], ws_row).bus()?.collect::<rusqlite::Result<Vec<_>>>();
        Ok(WsListOut { workspaces: rows.bus()? })
    });
    e.register::<WsUpdate>(|ctx: &mut Ctx, p| {
        let before = get_workspace(ctx.tx(), p.workspace_id)?;
        let name = p.name.clone().unwrap_or_else(|| before.name.clone());
        let ord = p.order.unwrap_or(before.order);
        ctx.tx().execute("UPDATE workspaces SET name = ?1, ord = ?2, updated_at = ?3 WHERE id = ?4",
            params![name, ord, ctx.now, p.workspace_id]).bus()?;
        let ws = get_workspace(ctx.tx(), p.workspace_id)?;
        ctx.set_undo("workspace.update",
            json!({ "workspace_id": before.id, "name": before.name, "order": before.order }),
            Some(json!({ "updated_at": ws.updated_at })));
        ctx.emit("workspace.changed", serde_json::to_value(&ws).bus()?);
        Ok(ws)
    });
    e.register::<WsRemove>(|ctx: &mut Ctx, p| {
        get_workspace(ctx.tx(), p.workspace_id)?;
        let n: i64 = ctx.tx().query_row("SELECT COUNT(*) FROM projects WHERE workspace_id = ?1", [p.workspace_id], |r| r.get(0)).bus()?;
        if n > 0 {
            return Err(BusError::conflict("workspace.has_projects", format!("workspace {} still has {n} project(s)", p.workspace_id))
                .with_hint("remove its projects first (project.remove)"));
        }
        ctx.tx().execute("DELETE FROM workspaces WHERE id = ?1", [p.workspace_id]).bus()?;
        ctx.emit("workspace.deleted", json!({ "id": p.workspace_id }));
        Ok(Empty {})
    });

    e.register::<ProjectAdd>(|ctx: &mut Ctx, p| {
        let ws = get_workspace(ctx.tx(), p.workspace_id)?;
        let path = canon(&p.path, "project.path")?;
        if !path.join(".git").exists() {
            return Err(BusError::invalid("project.path", format!("{} is not a git repository root", path.display())));
        }
        if !path.starts_with(&ws.path) {
            return Err(BusError::invalid("project.outside_workspace",
                format!("{} is not inside workspace {}", path.display(), ws.path)));
        }
        let path_s = path.display().to_string();
        let dup: Option<Id> = ctx.tx().query_row("SELECT id FROM projects WHERE path = ?1", [&path_s], |r| r.get(0)).optional().bus()?;
        if let Some(id) = dup {
            return Err(BusError::conflict("project.exists", format!("{path_s} is already project {id}")).with_details(json!({"project_id": id})));
        }
        let name = p.name.unwrap_or_else(|| name_of(&path));
        let ord: i64 = ctx.tx().query_row("SELECT COALESCE(MAX(ord), -1) + 1 FROM projects WHERE workspace_id = ?1", [ws.id], |r| r.get(0)).bus()?;
        let base = detect_default_branch(&path);
        ctx.tx().execute(
            "INSERT INTO projects(workspace_id, path, name, base_branch, ord, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![ws.id, path_s, name, base, ord, ctx.now],
        ).bus()?;
        let pr = get_project(ctx.tx(), ctx.tx().last_insert_rowid())?;
        // Skills are app-wide (D147): a project added today starts with every installed skill
        // enabled, and gets the folders as soon as the transaction commits.
        ctx.tx().execute(
            "INSERT OR IGNORE INTO skill_projects(skill_id,project_id) SELECT id,?1 FROM skills WHERE deleted_at IS NULL",
            [pr.id],
        ).bus()?;
        let root = path.clone();
        ctx.after_commit(move |engine| {
            std::thread::Builder::new().name("skill-materialize".into()).spawn(move || {
                let plan = { crate::skills::plan(&engine.store.lock(), &root, pr.id) };
                match plan {
                    Ok(plan) => if let Err(error) = crate::skills::apply(&plan, &engine.store) {
                        tracing::warn!(error = %error, "materializing skills");
                    },
                    Err(error) => tracing::warn!(error = %error, "planning skills"),
                }
            }).ok();
        });
        ctx.set_project(pr.id);
        ctx.emit("project.changed", serde_json::to_value(&pr).bus()?);
        Ok(pr)
    });
    e.register::<ProjectClone>(|ctx: &mut Ctx, p| {
        let workspace = get_workspace(ctx.tx(), p.workspace_id)?;
        let destination = clone_destination(&workspace.path, &p.url, p.dest.as_deref())?;
        if destination.exists() {
            return Err(BusError::conflict("project.clone_destination", format!("{} already exists", destination.display())));
        }
        let output = Command::new("git").arg("clone").arg("--").arg(&p.url).arg(&destination).output()
            .map_err(|error| BusError::unavailable("project.git_missing", format!("cannot start git: {error}")))?;
        if !output.status.success() {
            return Err(BusError::unavailable("project.clone_failed", String::from_utf8_lossy(&output.stderr).trim().to_string()));
        }
        let added = ctx.invoke_registered("project.add", json!({"workspace_id":workspace.id,"path":destination}))
            .and_then(|value| serde_json::from_value::<Project>(value).map_err(|error| BusError::internal(error.to_string())));
        match added {
            Ok(project) => Ok(ProjectCloneOut { project }),
            Err(error) => {
                let _ = fs::remove_dir_all(&destination);
                Err(error)
            }
        }
    });
    e.register::<ProjectList>(|ctx, p| {
        let projects = match p.workspace_id {
            Some(w) => {
                let mut stmt = ctx.tx().prepare_cached("SELECT * FROM projects WHERE workspace_id = ?1 ORDER BY pinned DESC, ord, id").bus()?;
                let rows = stmt.query_map([w], project_row).bus()?.collect::<rusqlite::Result<Vec<_>>>();
                rows.bus()?
            }
            None => {
                let mut stmt = ctx.tx().prepare_cached("SELECT * FROM projects ORDER BY workspace_id, pinned DESC, ord, id").bus()?;
                let rows = stmt.query_map([], project_row).bus()?.collect::<rusqlite::Result<Vec<_>>>();
                rows.bus()?
            }
        };
        Ok(ProjectListOut { projects })
    });
    e.register::<ProjectGet>(|ctx, p| get_project(ctx.tx(), p.project_id));
    e.register::<ProjectUpdate>(|ctx: &mut Ctx, p| {
        let b = get_project(ctx.tx(), p.project_id)?;
        let name = p.name.clone().unwrap_or_else(|| b.name.clone());
        let build_cmd = p.build_cmd.clone().unwrap_or_else(|| b.build_cmd.clone());
        let run_cmd = p.run_cmd.clone().unwrap_or_else(|| b.run_cmd.clone());
        let base_branch = p.base_branch.clone().unwrap_or_else(|| b.base_branch.clone());
        let protected = p.protected_paths.clone().unwrap_or_else(|| b.protected_paths.clone());
        let critical = p.critical_files.clone().unwrap_or_else(|| b.critical_files.clone());
        let ord = p.order.unwrap_or(b.order);
        let pinned = p.pinned.unwrap_or(b.pinned);
        ctx.tx().execute(
            "UPDATE projects SET name=?1, build_cmd=?2, run_cmd=?3, base_branch=?4, protected_paths=?5, critical_files=?6, ord=?7, pinned=?8, updated_at=?9 WHERE id=?10",
            params![name, build_cmd, run_cmd, base_branch, serde_json::to_string(&protected).bus()?, serde_json::to_string(&critical).bus()?, ord, pinned as i64, ctx.now, b.id],
        ).bus()?;
        let pr = get_project(ctx.tx(), b.id)?;
        ctx.set_project(pr.id);
        ctx.set_undo("project.update", json!({
            "project_id": b.id, "name": b.name, "build_cmd": b.build_cmd, "run_cmd": b.run_cmd, "base_branch": b.base_branch,
            "protected_paths": b.protected_paths, "critical_files": b.critical_files, "order": b.order, "pinned": b.pinned,
        }), Some(json!({ "updated_at": pr.updated_at })));
        ctx.emit("project.changed", serde_json::to_value(&pr).bus()?);
        Ok(pr)
    });
    e.register::<ProjectRemove>(|ctx: &mut Ctx, p| {
        let pr = get_project(ctx.tx(), p.project_id)?;
        let live_sessions: i64 = ctx.tx().query_row(
            "SELECT COUNT(*) FROM sessions WHERE project_id=?1 AND state!='closed'",
            [pr.id], |row| row.get(0),
        ).bus()?;
        if live_sessions > 0 {
            return Err(BusError::conflict("project.sessions_live", format!("project {} still has {live_sessions} open session(s)", pr.id))
                .with_hint("close its sessions before removing the project"));
        }
        let live_integrations: i64 = ctx.tx().query_row(
            "SELECT COUNT(*) FROM integrations WHERE project_id=?1 AND state IN ('queued','merging','building','deploying')",
            [pr.id], |row| row.get(0),
        ).bus()?;
        let live_runs: i64 = ctx.tx().query_row(
            "SELECT COUNT(*) FROM device_runs WHERE project_id=?1 AND state IN ('building','running')",
            [pr.id], |row| row.get(0),
        ).bus()?;
        if live_integrations + live_runs > 0 {
            return Err(BusError::conflict("project.activity_live", "project still has an active integration or device run")
                .with_hint("stop active runs before removing the project"));
        }

        // `project.remove` forgets Relay metadata only. Audit rows deliberately remain as the
        // immutable history, while every FK-owned row is removed in dependency order.
        for sql in [
            "DELETE FROM message_recipients WHERE message_id IN (SELECT id FROM messages WHERE project_id=?1)",
            "DELETE FROM messages WHERE project_id=?1",
            "DELETE FROM session_scrollback WHERE session_id IN (SELECT id FROM sessions WHERE project_id=?1)",
            "DELETE FROM claims WHERE project_id=?1",
            "DELETE FROM task_sessions WHERE session_id IN (SELECT id FROM sessions WHERE project_id=?1) OR task_id IN (SELECT id FROM tasks WHERE project_id=?1)",
            "DELETE FROM sessions WHERE project_id=?1",
            "DELETE FROM task_commits WHERE task_id IN (SELECT id FROM tasks WHERE project_id=?1)",
            "DELETE FROM attachments WHERE task_id IN (SELECT id FROM tasks WHERE project_id=?1)",
            "DELETE FROM module_unlinked_tasks WHERE module_id IN (SELECT id FROM modules WHERE project_id=?1) OR task_id IN (SELECT id FROM tasks WHERE project_id=?1)",
            "DELETE FROM tasks WHERE project_id=?1",
            "DELETE FROM modules WHERE project_id=?1",
            "DELETE FROM notes WHERE project_id=?1",
            "DELETE FROM notifications WHERE project_id=?1",
            "DELETE FROM overlaps WHERE project_id=?1",
            "DELETE FROM file_trash WHERE project_id=?1",
            "DELETE FROM integrations WHERE project_id=?1",
            "DELETE FROM ui_layouts WHERE project_id=?1",
            "DELETE FROM device_runs WHERE project_id=?1",
            "DELETE FROM skill_projects WHERE project_id=?1",
        ] {
            ctx.tx().execute(sql, [pr.id]).bus()?;
        }
        ctx.tx().execute("DELETE FROM settings WHERE path=?1 OR path LIKE ?2",
            params![format!("layout.current.{}", pr.id), format!("guardrails.projects.{}.%", pr.id)]).bus()?;
        ctx.tx().execute("DELETE FROM projects WHERE id = ?1", [pr.id]).bus()?;
        ctx.set_project(pr.id);
        ctx.emit("project.deleted", json!({ "id": pr.id }));
        Ok(Empty {})
    });
}

fn discover_repositories(root: &Path) -> Result<Vec<relay_bus::types::LocalRepo>, BusError> {
    fn walk(path: &Path, depth: usize, out: &mut Vec<relay_bus::types::LocalRepo>) -> std::io::Result<()> {
        if path.join(".git").exists() {
            out.push(relay_bus::types::LocalRepo { path: path.display().to_string(), name: name_of(path) });
            return Ok(());
        }
        if depth >= 4 { return Ok(()); }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() || entry.file_type()?.is_symlink() { continue; }
            let name = entry.file_name();
            if matches!(name.to_str(), Some(".git" | ".relay" | "node_modules" | "target" | "build" | ".gradle")) { continue; }
            walk(&entry.path(), depth + 1, out)?;
        }
        Ok(())
    }
    let mut repositories = Vec::new();
    walk(root, 0, &mut repositories).map_err(|error| BusError::unavailable("workspace.discover_failed", error.to_string()))?;
    repositories.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(repositories)
}

fn clone_destination(workspace: &str, url: &str, requested: Option<&str>) -> Result<PathBuf, BusError> {
    let name = requested.filter(|value| !value.trim().is_empty()).map(str::trim).map(str::to_string).unwrap_or_else(|| {
        url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("repository").trim_end_matches(".git").to_string()
    });
    let relative = Path::new(&name);
    if name.is_empty() || relative.is_absolute() || relative.components().any(|component| !matches!(component, std::path::Component::Normal(_))) {
        return Err(BusError::invalid("project.clone_dest", "clone destination must be one directory name"));
    }
    Ok(Path::new(workspace).join(relative))
}

/// `main` unless the repo's HEAD says otherwise; no subprocess (SPEC §15: zero idle spawns,
/// and this is a one-off anyway — but a file read is cheaper and enough).
fn detect_default_branch(repo: &Path) -> String {
    let head = std::fs::read_to_string(repo.join(".git").join("HEAD")).unwrap_or_default();
    head.trim().strip_prefix("ref: refs/heads/").map(str::to_string).unwrap_or_else(|| "main".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_workspace_uses_the_parent_of_a_containing_repository() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("relay");
        let deep = repo.join("apps/relay-app/src-tauri");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(&deep).unwrap();
        assert_eq!(suggested_workspace_from(&deep), root.path());
    }

    #[test]
    fn suggested_workspace_keeps_a_non_repository_directory() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("projects");
        fs::create_dir_all(&directory).unwrap();
        assert_eq!(suggested_workspace_from(&directory), directory);
    }
}
