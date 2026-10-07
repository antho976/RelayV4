//! `file.*` (phase 8): worktree-relative IO, recoverable trash, search and git-badged trees.

use crate::engine::{Ctx, Engine, IntoBus, Unlocked};
use crate::handlers::guardrail;
use crate::handlers::workspace::get_project;
use base64::Engine as _;
use relay_bus::error::BusError;
use relay_bus::ops::file::*;
use relay_bus::types::{Entry, EntryKind, GateKind, Id};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub fn register(e: &mut Engine) {
    e.register_unlocked::<Tree>(|ctx, p| {
        let (project, root) = root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        crate::watch::ensure_after_commit(ctx, root.clone(), project.id);
        let rel = rel(p.path.as_deref().unwrap_or(""), true)?;
        let dir = safe_join(&root, &rel, true)?;
        if !dir.is_dir() {
            return Err(BusError::not_found(
                "file.not_found",
                format!("{} is not a directory", rel.display()),
            ));
        }
        let badges = if p.git_badges.unwrap_or(true) {
            super::git::status_badges(&root).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(TreeOut {
            entries: list_dir(&root, &dir, p.depth.unwrap_or(1).min(20), &badges)?,
        })
    });
    e.register_unlocked::<Read>(|ctx, p| {
        let (project, root) = root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        crate::watch::ensure_after_commit(ctx, root.clone(), project.id);
        let rel = rel(&p.path, false)?;
        let path = safe_join(&root, &rel, false)?;
        let md = fs::metadata(&path).map_err(|e| io_err("file.read_failed", &rel, e))?;
        if !md.is_file() {
            return Err(BusError::invalid("file.type", "path is not a file"));
        }
        let limit = p.max_bytes.unwrap_or(4 * 1024 * 1024).min(32 * 1024 * 1024);
        use std::io::Read;
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
            .map_err(|e| io_err("file.read_failed", &rel, e))?;
        let truncated = bytes.len() as u64 > limit;
        bytes.truncate(limit as usize);
        let mime = mime(&rel);
        let (text, bytes_b64) = match String::from_utf8(bytes.clone()) {
            Ok(text) if !text.contains('\0') => (Some(text), None),
            _ => (
                None,
                Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            ),
        };
        Ok(ReadOut {
            text,
            bytes_b64,
            mime,
            size: md.len(),
            truncated,
        })
    });
    e.register::<Write>(|ctx: &mut Ctx, p| {
        let (project, root) = root(ctx, p.project_id, p.worktree.as_deref())?;
        let rel = rel(&p.path, false)?;
        let path = safe_join(&root, &rel, true)?;
        let old = fs::read_to_string(&path);
        if let Some(expected) = &p.expected_sha256 {
            let matches = old.as_ref().is_ok_and(|text| {
                Sha256::digest(text.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
                    == *expected
            });
            if !matches {
                return Err(BusError::conflict(
                    "file.edit_conflict",
                    "File changed or was removed. Your draft has not been written.",
                ));
            }
        }
        let old = old.unwrap_or_default();
        guardrail::enforce(
            ctx,
            project.id,
            &root,
            GateKind::Write,
            Some(&p.path),
            Some(&p.text),
            None,
            None,
        )?;
        let (removed, added) = line_counts(&old, &p.text);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err("file.write_failed", &rel, e))?;
        }
        let temp_dir = root.join(".relay").join("tmp");
        fs::create_dir_all(&temp_dir).map_err(|e| io_err("file.write_failed", &rel, e))?;
        let temp = temp_dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&temp, p.text.as_bytes()).map_err(|e| io_err("file.write_failed", &rel, e))?;
        if let Ok(metadata) = fs::metadata(&path) {
            fs::set_permissions(&temp, metadata.permissions())
                .map_err(|e| io_err("file.write_failed", &rel, e))?;
        }
        fs::rename(&temp, &path).map_err(|e| io_err("file.write_failed", &rel, e))?;
        ctx.set_project(project.id);
        ctx.emit(
            "file.changed",
            json!({"project_id": project.id, "worktree": root, "path": p.path}),
        );
        Ok(WriteOut {
            bytes: p.text.len() as u64,
            removed_lines: removed,
            added_lines: added,
        })
    });
    e.register::<Create>(|ctx: &mut Ctx, p| {
        let (project, root) = root(ctx, p.project_id, p.worktree.as_deref())?;
        let rel = rel(&p.path, false)?;
        let path = safe_join(&root, &rel, true)?;
        if path.exists() {
            return Err(BusError::conflict(
                "file.exists",
                format!("{} already exists", p.path),
            ));
        }
        let text = p.text.clone().unwrap_or_default();
        guardrail::enforce(
            ctx,
            project.id,
            &root,
            GateKind::Write,
            Some(&p.path),
            Some(&text),
            None,
            None,
        )?;
        match p.kind.as_str() {
            "file" => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| io_err("file.create_failed", &rel, e))?;
                }
                fs::write(&path, text).map_err(|e| io_err("file.create_failed", &rel, e))?;
            }
            "dir" => {
                fs::create_dir_all(&path).map_err(|e| io_err("file.create_failed", &rel, e))?
            }
            _ => return Err(BusError::invalid("file.kind", "kind must be file or dir")),
        }
        changed(ctx, project.id, &root, &p.path);
        entry(&root, &path, &HashMap::new(), 0)
    });
    e.register::<Rename>(|ctx: &mut Ctx, p| {
        if p.new_name.is_empty()
            || Path::new(&p.new_name).components().count() != 1
            || p.new_name == "."
            || p.new_name == ".."
        {
            return Err(BusError::invalid(
                "file.path",
                "new_name must be one path component",
            ));
        }
        let (project, root) = root(ctx, p.project_id, p.worktree.as_deref())?;
        let from_rel = rel(&p.path, false)?;
        let from = safe_join(&root, &from_rel, false)?;
        let into_rel = from_rel.parent().unwrap_or(Path::new("")).join(&p.new_name);
        let into = safe_join(&root, &into_rel, true)?;
        guard_path_mutation(ctx, project.id, &root, &from_rel, None)?;
        guard_path_mutation(ctx, project.id, &root, &into_rel, Some(&from))?;
        if into.exists() {
            return Err(BusError::conflict(
                "file.exists",
                format!("{} already exists", into_rel.display()),
            ));
        }
        fs::rename(&from, &into).map_err(|e| io_err("file.rename_failed", &from_rel, e))?;
        changed(ctx, project.id, &root, &into_rel.to_string_lossy());
        entry(&root, &into, &HashMap::new(), 0)
    });
    e.register::<Move>(|ctx: &mut Ctx, p| {
        let (project, root) = root(ctx, p.project_id, p.worktree.as_deref())?;
        let from_rel = rel(&p.path, false)?;
        let from = safe_join(&root, &from_rel, false)?;
        let dir_rel = rel(&p.into, true)?;
        let dir = safe_join(&root, &dir_rel, false)?;
        if !dir.is_dir() {
            return Err(BusError::invalid(
                "file.into",
                "destination is not a directory",
            ));
        }
        let name = from
            .file_name()
            .ok_or_else(|| BusError::invalid("file.path", "path has no name"))?;
        let into_rel = dir_rel.join(name);
        let into = safe_join(&root, &into_rel, true)?;
        guard_path_mutation(ctx, project.id, &root, &from_rel, None)?;
        guard_path_mutation(ctx, project.id, &root, &into_rel, Some(&from))?;
        if into.exists() {
            return Err(BusError::conflict(
                "file.exists",
                format!("{} already exists", into_rel.display()),
            ));
        }
        fs::rename(&from, &into).map_err(|e| io_err("file.move_failed", &from_rel, e))?;
        changed(ctx, project.id, &root, &into_rel.to_string_lossy());
        entry(&root, &into, &HashMap::new(), 0)
    });
    e.register::<Delete>(|ctx: &mut Ctx, p| {
        let (project, root) = root(ctx, p.project_id, p.worktree.as_deref())?;
        let rel = rel(&p.path, false)?; let path = safe_join(&root, &rel, false)?;
        guard_path_mutation(ctx, project.id, &root, &rel, None)?;
        ctx.tx().execute(
            "INSERT INTO file_trash(project_id, worktree, original_path, trash_path, created_at) VALUES (?1,?2,?3,'',?4)",
            params![project.id, root.display().to_string(), p.path, ctx.now],
        ).bus()?;
        let id = ctx.tx().last_insert_rowid();
        let trash = root.join(".relay").join("trash").join(id.to_string()).join("payload");
        fs::create_dir_all(trash.parent().unwrap()).map_err(|e| io_err("file.delete_failed", &rel, e))?;
        fs::rename(&path, &trash).map_err(|e| io_err("file.delete_failed", &rel, e))?;
        ctx.tx().execute("UPDATE file_trash SET trash_path=?1 WHERE id=?2", params![trash.display().to_string(), id]).bus()?;
        ctx.set_undo("file.restore", json!({"project_id": project.id, "trash_id": id}), None);
        changed(ctx, project.id, &root, &p.path);
        Ok(DeleteOut { trash_id: id })
    });
    e.register::<Restore>(|ctx: &mut Ctx, p| {
        let project = get_project(ctx.tx(), p.project_id)?;
        let row: Option<(String,String,String)> = ctx.tx().query_row(
            "SELECT worktree, original_path, trash_path FROM file_trash WHERE id=?1 AND project_id=?2 AND restored_at IS NULL",
            params![p.trash_id, project.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().bus()?;
        let (root_s, rel_s, trash_s) = row.ok_or_else(|| BusError::not_found("file.trash_not_found", format!("no open trash {}", p.trash_id)))?;
        let root = PathBuf::from(root_s); let rel = rel(&rel_s, false)?; let path = safe_join(&root, &rel, true)?;
        if path.exists() { return Err(BusError::conflict("file.restore_conflict", format!("{} already exists", rel.display()))); }
        if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| io_err("file.restore_failed", &rel, e))?; }
        fs::rename(&trash_s, &path).map_err(|e| io_err("file.restore_failed", &rel, e))?;
        ctx.tx().execute("UPDATE file_trash SET restored_at=?1 WHERE id=?2", params![ctx.now, p.trash_id]).bus()?;
        changed(ctx, project.id, &root, &rel_s);
        entry(&root, &path, &HashMap::new(), 0)
    });
    e.register::<RestoreHead>(|ctx: &mut Ctx, p| {
        let (project, root) = root(ctx, p.project_id, p.worktree.as_deref())?;
        let rel = rel(&p.path, false)?;
        crate::worktree::git_mutate(&root, &["checkout", "--", &p.path])
            .map_err(|e| BusError::conflict("file.restore_head_failed", e.to_string()))?;
        let path = safe_join(&root, &rel, false)?;
        changed(ctx, project.id, &root, &p.path);
        entry(&root, &path, &HashMap::new(), 0)
    });
    e.register::<Import>(|ctx: &mut Ctx, p| {
        let (project, root) = root(ctx, p.project_id, p.worktree.as_deref())?;
        let into_rel = rel(&p.into, true)?;
        let into = safe_join(&root, &into_rel, false)?;
        if !into.is_dir() {
            return Err(BusError::invalid(
                "file.into",
                "destination is not a directory",
            ));
        }
        let mut entries = Vec::new();
        for source in &p.sources {
            let source = Path::new(source);
            if !source.is_absolute() || !source.exists() {
                return Err(BusError::invalid(
                    "file.source",
                    format!("{} is not an existing absolute path", source.display()),
                ));
            }
            let name = source
                .file_name()
                .ok_or_else(|| BusError::invalid("file.source", "source has no name"))?;
            let dest = into.join(name);
            let dest_rel = into_rel.join(name);
            guard_path_mutation(ctx, project.id, &root, &dest_rel, Some(source))?;
            if dest.exists() {
                return Err(BusError::conflict(
                    "file.exists",
                    format!("{} already exists", dest_rel.display()),
                ));
            }
            copy(source, &dest).map_err(|e| io_err("file.import_failed", &dest_rel, e))?;
            entries.push(entry(&root, &dest, &HashMap::new(), 0)?);
        }
        changed(ctx, project.id, &root, &p.into);
        Ok(ImportOut { entries })
    });
    e.register_unlocked::<Search>(|ctx, p| {
        let (project, root) = root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        crate::watch::ensure_after_commit(ctx, root.clone(), project.id);
        let regex = if p.regex.unwrap_or(false) {
            Some(
                regex::Regex::new(&p.query)
                    .map_err(|e| BusError::invalid("file.regex", e.to_string()))?,
            )
        } else {
            None
        };
        let limit = p.limit.unwrap_or(200).min(2000) as usize;
        let mut hits = Vec::new();
        let mut stack = vec![root.clone()];
        // One buffer for the whole walk: `read_to_string` allocated, grew and freed a fresh
        // `String` for every file in the tree, most of which contribute no hits at all.
        let mut buffer: Vec<u8> = Vec::new();
        while let Some(dir) = stack.pop() {
            // One unreadable subdirectory costs its own hits, not the whole search.
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) if dir == root => return Err(io_err("file.search_failed", &dir, e)),
                Err(_) => continue,
            };
            for e in entries.flatten() {
                let path = e.path();
                let relp = path.strip_prefix(&root).unwrap_or(&path);
                // `DirEntry::file_type` does not follow symlinks, so a linked directory is
                // never walked into. Generated trees are skipped by the same rule file.tree uses.
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    if !crate::watch::is_generated_path(&root, &path) {
                        stack.push(path);
                    }
                    continue;
                }
                if let Some(glob) = &p.glob {
                    if !crate::guardrail::path_matches(glob, relp) {
                        continue;
                    }
                }
                if !read_searchable(&root, &path, &mut buffer) {
                    continue;
                }
                // Same rule as `read_to_string`: what is not text is not searched.
                let Ok(text) = std::str::from_utf8(&buffer) else {
                    continue;
                };
                // The whole file is searched in one pass and hits are mapped back to lines,
                // one per line as before. Searching line by line built a fresh substring
                // searcher for every line of every file, a fifth of the op (PERF §1.7).
                let mut from = 0usize;
                let mut line = 1u32;
                let mut counted_to = 0usize;
                while from < text.len() {
                    let found = match &regex {
                        Some(r) => r.find_at(text, from).map(|m| m.start()),
                        None => text[from..].find(&p.query).map(|i| from + i),
                    };
                    let Some(at) = found else { break };
                    line += text[counted_to..at].bytes().filter(|b| *b == b'\n').count() as u32;
                    counted_to = at;
                    let start = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let end = text[at..].find('\n').map(|i| at + i).unwrap_or(text.len());
                    hits.push(Hit {
                        path: relp.to_string_lossy().to_string(),
                        line,
                        col: (at - start) as u32 + 1,
                        text: text[start..end].trim_end_matches('\r').to_string(),
                    });
                    if hits.len() >= limit {
                        return Ok(SearchOut { hits });
                    }
                    from = end + 1;
                }
            }
        }
        Ok(SearchOut { hits })
    });
}

/// Which tree an omitted `worktree` means. For an agent it is that session's own worktree,
/// never the project root: defaulting to the root answered a confident, well-formed, wrong
/// question — "your" diff, read from a tree the agent has never touched — and returned no
/// error either way (D111). `@project` asks for the root explicitly and loudly.
pub(crate) fn default_worktree(
    ctx: &Ctx,
    project: &relay_bus::types::Project,
    requested: Option<&str>,
) -> Result<PathBuf, BusError> {
    default_worktree_in(ctx.tx(), ctx.actor_session_id(), project, requested)
}

/// The store half of worktree resolution, split out so the unlocked query context can do it in
/// one short read and leave the `git worktree list` that validates the answer off the lock (D144).
pub(crate) fn default_worktree_in(
    conn: &Connection,
    session_id: Option<Id>,
    project: &relay_bus::types::Project,
    requested: Option<&str>,
) -> Result<PathBuf, BusError> {
    let repo = PathBuf::from(&project.path);
    match requested {
        Some(PROJECT_ROOT) => Ok(repo),
        Some(path) => Ok(PathBuf::from(path)),
        None => match session_id {
            Some(id) => {
                let row = crate::sessions::by_id(conn, id)?
                    .ok_or_else(|| BusError::actor("bound session vanished"))?;
                Ok(PathBuf::from(row.session.worktree))
            }
            None => Ok(repo),
        },
    }
}

/// The store half of [`root`]: project row plus the worktree the caller means, before any of it
/// is checked against the repository. Cheap, and the only part that needs the connection.
pub(crate) fn root_choice(
    conn: &Connection,
    session_id: Option<Id>,
    project_id: Id,
    requested: Option<&str>,
) -> Result<(relay_bus::types::Project, PathBuf), BusError> {
    let project = get_project(conn, project_id)?;
    let chosen = default_worktree_in(conn, session_id, &project, requested)?;
    Ok((project, chosen))
}

/// The external half: canonicalize and confirm the path really is a worktree of this repository.
/// `git worktree list` is a subprocess, so this must run with the store lock released.
pub(crate) fn root_verify(
    project: &relay_bus::types::Project,
    chosen: PathBuf,
    code: &'static str,
) -> Result<PathBuf, BusError> {
    let chosen = fs::canonicalize(&chosen).map_err(|e| BusError::invalid(code, e.to_string()))?;
    let valid = crate::worktree::contains(Path::new(&project.path), &chosen)
        .map_err(|e| BusError::unavailable("worktree.list_failed", e.to_string()))?;
    if !valid {
        return Err(BusError::invalid(
            code,
            format!("{} is not a worktree of this project", chosen.display()),
        ));
    }
    Ok(chosen)
}

/// The one spelling that means "the project root, deliberately".
pub(crate) const PROJECT_ROOT: &str = "@project";

fn root(
    ctx: &Ctx,
    project_id: Id,
    requested: Option<&str>,
) -> Result<(relay_bus::types::Project, PathBuf), BusError> {
    let (project, chosen) = root_choice(ctx.tx(), ctx.actor_session_id(), project_id, requested)?;
    let chosen = root_verify(&project, chosen, "file.worktree")?;
    Ok((project, chosen))
}

/// [`root`] for the unlocked query context: one short read, then the subprocess with nothing held.
pub(crate) fn root_unlocked(
    ctx: &Unlocked,
    project_id: Id,
    requested: Option<&str>,
) -> Result<(relay_bus::types::Project, PathBuf), BusError> {
    let session_id = ctx.actor_session_id();
    let (project, chosen) =
        ctx.read(|conn| root_choice(conn, session_id, project_id, requested))?;
    let chosen = root_verify(&project, chosen, "file.worktree")?;
    Ok((project, chosen))
}

fn rel(value: &str, allow_empty: bool) -> Result<PathBuf, BusError> {
    let path = Path::new(value);
    if (!allow_empty && value.is_empty())
        || path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(BusError::invalid(
            "file.path",
            format!("{value:?} must be worktree-relative and contain no .."),
        ));
    }
    Ok(path.to_path_buf())
}

fn safe_join(root: &Path, rel: &Path, may_not_exist: bool) -> Result<PathBuf, BusError> {
    let path = root.join(rel);
    let check = if path.exists() {
        fs::canonicalize(&path)
    } else if may_not_exist {
        fs::canonicalize(path.parent().unwrap_or(root))
    } else {
        fs::canonicalize(&path)
    };
    let check = check.map_err(|e| io_err("file.not_found", rel, e))?;
    if !check.starts_with(root) {
        return Err(BusError::invalid(
            "file.path",
            "path escapes the worktree through a symlink",
        ));
    }
    Ok(path)
}

fn list_dir(
    root: &Path,
    dir: &Path,
    depth: u32,
    badges: &HashMap<String, String>,
) -> Result<Vec<Entry>, BusError> {
    let mut out = fs::read_dir(dir)
        .map_err(|e| io_err("file.tree_failed", dir, e))?
        .flatten()
        .filter(|e| {
            !matches!(e.file_name().to_string_lossy().as_ref(), ".git" | ".relay")
                && !crate::watch::is_generated_path(root, &e.path())
        })
        .map(|e| entry(root, &e.path(), badges, depth.saturating_sub(1)))
        .collect::<Result<Vec<_>, _>>()?;
    out.sort_by_key(|e| (e.kind != EntryKind::Dir, e.name.to_lowercase()));
    Ok(out)
}

fn entry(
    root: &Path,
    path: &Path,
    badges: &HashMap<String, String>,
    child_depth: u32,
) -> Result<Entry, BusError> {
    let md = fs::symlink_metadata(path).map_err(|e| io_err("file.stat_failed", path, e))?;
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let kind = if md.file_type().is_symlink() {
        EntryKind::Symlink
    } else if md.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::File
    };
    let children = if kind == EntryKind::Dir && child_depth > 0 {
        Some(list_dir(root, path, child_depth, badges)?)
    } else {
        None
    };
    let modified_at = md
        .modified()
        .ok()
        .and_then(|time| jiff::Timestamp::try_from(time).ok())
        .map(|t| t.to_string());
    Ok(Entry {
        path: rel.clone(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        kind,
        size: (kind == EntryKind::File).then_some(md.len() as i64),
        modified_at,
        badge: badges.get(&rel).cloned(),
        children,
    })
}

/// Gate a rename, move, delete or import by path alone (BUS.md §9.2). `content` is the file
/// that would land at `rel`; it is read only when a shape gate covers `rel` and needs the text,
/// so moving a multi-GB asset never reads it, under the lock or otherwise.
fn guard_path_mutation(
    ctx: &mut Ctx,
    project_id: Id,
    root: &Path,
    rel: &Path,
    content: Option<&Path>,
) -> Result<(), BusError> {
    let shaped = content.is_some()
        && crate::guardrail::config(ctx.tx(), Some(project_id))?
            .shape_gates
            .iter()
            .any(|gate| crate::guardrail::path_matches(&gate.path, rel));
    let text = content
        .filter(|_| shaped)
        .filter(|path| fs::metadata(path).is_ok_and(|md| md.is_file() && md.len() <= SHAPE_TEXT_MAX))
        .and_then(|path| fs::read_to_string(path).ok());
    guardrail::enforce_path_mutation(ctx, project_id, root, &rel.to_string_lossy(), text.as_deref())
}

/// Shape gates validate configuration files; anything larger is not one, and is held unread.
const SHAPE_TEXT_MAX: u64 = 4 * 1024 * 1024;

fn changed(ctx: &mut Ctx, project_id: Id, root: &Path, path: &str) {
    ctx.set_project(project_id);
    ctx.emit(
        "file.changed",
        json!({"project_id": project_id, "worktree": root, "path": path}),
    );
}

fn line_counts(old: &str, new: &str) -> (i64, i64) {
    let mut removed = 0;
    let mut added = 0;
    for change in TextDiff::from_lines(old, new).iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => removed += 1,
            ChangeTag::Insert => added += 1,
            ChangeTag::Equal => {}
        }
    }
    (removed, added)
}

fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    let md = fs::symlink_metadata(from)?;
    if md.file_type().is_symlink() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(fs::read_link(from)?, to)?;
    } else if md.is_dir() {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            copy(&e.path(), &to.join(e.file_name()))?;
        }
    } else {
        fs::copy(from, to)?;
    }
    Ok(())
}

/// Files larger than this are not searched: at that size they are assets or generated output,
/// and reading them whole on every search is the cost the editor's re-search pays repeatedly.
const SEARCH_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// How much of a file is read first to decide whether it is text at all.
const SEARCH_SNIFF_BYTES: usize = 8 * 1024;

/// Read `path` into `buffer` if it is a searchable text file: a regular file (a symlink only
/// when its target stays inside `root`, the same rule as file.read), no larger than
/// [`SEARCH_MAX_BYTES`], whose first few KB hold no NUL and valid UTF-8. A FIFO, socket or
/// device is never opened, and a binary asset costs one small read rather than its full size.
fn read_searchable(root: &Path, path: &Path, buffer: &mut Vec<u8>) -> bool {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let Ok(link) = fs::symlink_metadata(path) else { return false };
    if link.file_type().is_symlink()
        && !fs::canonicalize(path).is_ok_and(|target| target.starts_with(root))
    {
        return false;
    }
    let Ok(md) = fs::metadata(path) else { return false };
    if !md.is_file() || md.len() > SEARCH_MAX_BYTES {
        return false;
    }
    // O_NONBLOCK: a file swapped for a FIFO after the check above cannot hang the open.
    let Ok(file) = fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path) else {
        return false;
    };
    if !file.metadata().is_ok_and(|md| md.is_file()) {
        return false;
    }
    buffer.clear();
    let mut file = file.take(SEARCH_MAX_BYTES);
    if (&mut file).take(SEARCH_SNIFF_BYTES as u64).read_to_end(buffer).is_err() {
        return false;
    }
    let sniff = &buffer[..];
    if sniff.contains(&0) {
        return false;
    }
    if let Err(error) = std::str::from_utf8(sniff) {
        // A multi-byte character cut by the sniff boundary is not evidence of binary.
        if error.error_len().is_some() {
            return false;
        }
    }
    if buffer.len() < SEARCH_SNIFF_BYTES {
        return true;
    }
    file.read_to_end(buffer).is_ok()
}

fn mime(path: &Path) -> String {
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" | "ts" | "js" | "svelte" | "kt" | "kts" | "md" | "toml" | "yaml" | "yml" | "css"
        | "html" => "text/plain",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn io_err(code: &str, path: impl AsRef<Path>, e: std::io::Error) -> BusError {
    BusError::unavailable(code, format!("{}: {e}", path.as_ref().display()))
}
