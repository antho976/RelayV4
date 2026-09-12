//! Guardrails and session authorization (SPEC §5, BUS.md §9). Evaluation is pure:
//! [`evaluate`] returns allow/refuse/hold. The `guardrail.gate` handler is the only caller
//! that persists a hold; `guardrail.check` never writes.

use crate::engine::IntoBus;
use crate::sessions;
use relay_bus::envelope::{Actor, Request};
use relay_bus::error::BusError;
use relay_bus::registry::{Callable, OpEntry, OpKind, Registry};
use relay_bus::types::{
    GateKind, GuardrailConfig, Hold, HoldState, Id, Role, Session, ShapeGate,
};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// Result of pure policy evaluation. A hold names the one policy confirmation may skip.
///
/// A rewrite whose size Relay merely *inferred* is a judgement call, so it holds and the
/// person who can judge it is one confirmation away. Everything the user stated outright — a
/// cap, a protected path, a denied command, a permitted write root — refuses: re-asking on
/// every occurrence of a rule you already wrote down is how a guardrail becomes noise. Both
/// kinds say what would unblock them.
#[derive(Debug, Clone)]
pub enum Decision {
    Allow,
    Refuse(BusError),
    Hold { policy: String, error: BusError, details: Value },
}

#[derive(Debug, Clone)]
pub struct GateRequest<'a> {
    pub actor: &'a Actor,
    pub project_id: Id,
    pub worktree: &'a Path,
    pub kind: GateKind,
    pub path: Option<&'a str>,
    pub new_text: Option<&'a str>,
    pub diff: Option<&'a str>,
    pub command: Option<&'a str>,
    /// Confirmation skips exactly this policy and no other (BUS.md §9.4).
    pub skip_policy: Option<&'a str>,
}

// ---------------------------------------------------------------- configuration

/// Read the effective config: defaults <- global settings <- project override <- the
/// project's legacy `protected_paths` / `critical_files` fields.
pub fn config(conn: &Connection, project_id: Option<Id>) -> Result<GuardrailConfig, BusError> {
    let mut root = crate::handlers::settings::defaults()["guardrails"].clone();
    // Settings are leaves. Overlay every global guardrail leaf except `projects.*`.
    let mut stmt = conn
        .prepare_cached("SELECT path, value FROM settings WHERE path LIKE 'guardrails.%' ORDER BY path")
        .bus()?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    for (path, raw) in rows {
        let Some(relative) = path.strip_prefix("guardrails.") else { continue };
        if relative.starts_with("projects.") {
            continue;
        }
        let value: Value = serde_json::from_str(&raw).map_err(crate::engine::internal)?;
        set_at(&mut root, relative, value);
    }
    if let Some(project_id) = project_id {
        let prefix = format!("guardrails.projects.{project_id}");
        let mut stmt = conn
            .prepare_cached("SELECT path, value FROM settings WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\' ORDER BY path")
            .bus()?;
        let rows = stmt
            .query_map(params![prefix, format!("{prefix}.%")], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        for (path, raw) in rows {
            let relative = path.strip_prefix(&prefix).unwrap_or("").trim_start_matches('.');
            let value: Value = serde_json::from_str(&raw).map_err(crate::engine::internal)?;
            if relative.is_empty() {
                crate::handlers::settings::merge_value(&mut root, &value);
            } else {
                set_at(&mut root, relative, value);
            }
        }
        let legacy: Option<(String, String)> = conn
            .query_row(
                "SELECT protected_paths, critical_files FROM projects WHERE id = ?1",
                [project_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .bus()?;
        let Some((protected, critical)) = legacy else {
            return Err(BusError::not_found("project.not_found", format!("no project {project_id}")));
        };
        let mut protected_paths: Vec<String> = serde_json::from_str(&protected).unwrap_or_default();
        protected_paths.extend(
            root["protected_paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
        protected_paths.sort();
        protected_paths.dedup();
        root["protected_paths"] = json!(protected_paths);

        let critical: Vec<String> = serde_json::from_str(&critical).unwrap_or_default();
        let gates = root["shape_gates"].as_array_mut().ok_or_else(|| {
            BusError::invalid("guardrail.config", "shape_gates must be an array")
        })?;
        for path in critical {
            if !gates.iter().any(|g| g["path"] == path) {
                gates.push(json!({"path": path, "validator": "non_empty"}));
            }
        }
    }
    root.as_object_mut().map(|m| m.remove("projects"));
    let cfg: GuardrailConfig = serde_json::from_value(root)
        .map_err(|e| BusError::invalid("guardrail.config", format!("invalid guardrail config: {e}")))?;
    validate_config(&cfg)?;
    Ok(cfg)
}

pub fn validate_config(cfg: &GuardrailConfig) -> Result<(), BusError> {
    if cfg.caps.files == 0 || cfg.caps.lines == 0 {
        return Err(BusError::invalid("guardrail.config", "caps.files and caps.lines must be > 0"));
    }
    if !(0.0..=100.0).contains(&cfg.destructive_write.min_removed_pct) {
        return Err(BusError::invalid("guardrail.config", "destructive_write.min_removed_pct must be 0..=100"));
    }
    for path in &cfg.protected_paths {
        validate_pattern(path)?;
    }
    for gate in &cfg.shape_gates {
        validate_pattern(&gate.path)?;
        if !matches!(gate.validator.as_str(), "non_empty" | "json" | "json_non_empty_array" | "json_non_empty_object") {
            return Err(BusError::invalid(
                "guardrail.config",
                format!("unknown shape validator {:?} for {}", gate.validator, gate.path),
            ));
        }
    }
    Ok(())
}

fn set_at(root: &mut Value, path: &str, value: Value) {
    if path.is_empty() {
        crate::handlers::settings::merge_value(root, &value);
        return;
    }
    let mut current = root;
    let parts: Vec<&str> = path.split('.').collect();
    for (index, part) in parts.iter().enumerate() {
        if index == parts.len() - 1 {
            if !current.is_object() {
                *current = json!({});
            }
            current.as_object_mut().unwrap().insert((*part).to_string(), value);
            return;
        }
        if !current.is_object() {
            *current = json!({});
        }
        current = current
            .as_object_mut()
            .unwrap()
            .entry((*part).to_string())
            .or_insert_with(|| json!({}));
    }
}

// ---------------------------------------------------------------- authorization

/// Layer 2 allowlist + own-session/task scope (BUS.md §9.1). Layer 1 is registry metadata
/// and runs immediately before this in the engine.
pub fn authorize(
    conn: &Connection,
    session_id: Id,
    entry: &OpEntry,
    payload: &Value,
) -> Result<(), BusError> {
    let row = sessions::by_id(conn, session_id)?
        .ok_or_else(|| BusError::actor(format!("unknown session id {session_id}")))?;
    let session = &row.session;

    if let Some(name) = payload.get("session").and_then(Value::as_str) {
        let own = name == session.name;
        // Reads reach any session in the caller's own project — the same boundary
        // `session.list` draws. Private surfaces (scrollback, brief) narrow it again in the
        // handler; mutations remain own-session, or the PAIR partner, only (D106).
        let project_read = entry.meta.kind == OpKind::Query
            && sessions::by_name(conn, name).is_ok_and(|peer| peer.session.project_id == session.project_id);
        let pair_read = entry.meta.kind == OpKind::Query && session.pair_with.as_deref() == Some(name);
        if !own && !pair_read && !project_read {
            return Err(BusError::not_own("session"));
        }
    }
    if let Some(task_id) = payload.get("task_id").and_then(Value::as_i64) {
        let assigned: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_sessions WHERE session_id=?1 AND task_id=?2)",
            rusqlite::params![session.id, task_id],
            |record| record.get(0),
        ).map_err(crate::engine::internal)?;
        if !assigned {
            return Err(BusError::not_own("task"));
        }
    }
    if entry.meta.kind == OpKind::Query {
        return Ok(());
    }

    let cfg = config(conn, Some(session.project_id))?;
    let allowed = role_allowlist(&cfg, session.role);
    if allowed.iter().any(|pattern| op_matches(pattern, entry.name)) {
        return Ok(());
    }
    let bus_write = entry.name.starts_with("file.")
        || matches!(entry.name, "git.stage" | "git.unstage" | "git.commit");
    if bus_write && session.bus_writes {
        return Ok(());
    }
    let ui = entry.name.starts_with("ui.") || entry.name.starts_with("os.");
    if ui && session.allow_ui {
        return Ok(());
    }
    let role = sessions::role_str(session.role);
    let mut err = BusError::allowlist(entry.name, role);
    if bus_write {
        err = err
            .with_details(json!({"op": entry.name, "role": role, "option": "bus_writes"}))
            .with_hint("enable bus_writes for this session deliberately");
    } else if ui {
        err = err
            .with_details(json!({"op": entry.name, "role": role, "option": "allow_ui"}))
            .with_hint("enable allow_ui for this session deliberately");
    }
    Err(err)
}

/// All three gating layers, answered for one op without calling it (BUS.md §9.1). This is
/// the single source `bus.ops`, `relay ops --mine`, the MCP tool list and `session.bootstrap`
/// all read, so discovery can never again promise more than execution delivers (D104).
pub fn callability(
    entry: &OpEntry,
    actor: &Actor,
    session: Option<&Session>,
    cfg: Option<&GuardrailConfig>,
) -> (Callable, String) {
    // Layer 1: registry metadata.
    if !entry.meta.actors.admits(actor) {
        return (Callable::No, "user_only".to_string());
    }
    let Some(session) = session else {
        return (Callable::Yes, if actor.is_agent() { "agent".into() } else { "user".into() });
    };
    let role = sessions::role_str(session.role);
    // Layer 2: row scope. An op that names a session or a task only ever acts on this one.
    let scoped = Registry::global().payload_has(entry.name, "session")
        || Registry::global().payload_has(entry.name, "task_id");
    let self_only = |why: String| {
        (if scoped { Callable::SelfOnly } else { Callable::Yes }, why)
    };
    if entry.meta.kind == OpKind::Query {
        return self_only(if scoped { "own session only".into() } else { "query".into() });
    }
    // Layer 3: the role allowlist, then the two deliberate per-session escape hatches.
    let Some(cfg) = cfg else {
        return (Callable::No, "guardrail config unavailable".to_string());
    };
    if role_admits(cfg, session.role, entry.name) {
        return self_only(format!("role:{role} allowlist"));
    }
    let bus_write = entry.name.starts_with("file.")
        || matches!(entry.name, "git.stage" | "git.unstage" | "git.commit");
    if bus_write && session.bus_writes {
        return self_only("session.bus_writes".to_string());
    }
    let ui = entry.name.starts_with("ui.") || entry.name.starts_with("os.");
    if ui && session.allow_ui {
        return self_only("session.allow_ui".to_string());
    }
    let mut why = format!("role:{role} not in allowlist");
    if bus_write {
        why.push_str(" (needs session.bus_writes)");
    } else if ui {
        why.push_str(" (needs session.allow_ui)");
    }
    (Callable::No, why)
}

/// The allow-set for one role, as configured (BUS.md §9.1 layer 3).
pub fn role_allowlist(cfg: &GuardrailConfig, role: Role) -> &Vec<String> {
    match role {
        Role::Builder => &cfg.roles.builder,
        Role::Reviewer => &cfg.roles.reviewer,
        Role::Docs => &cfg.roles.docs,
    }
}

/// Would layer 3 admit `op` for `role`? The escape hatches (`bus_writes`, `allow_ui`) are a
/// property of the session, not the role, so callers apply those separately.
pub fn role_admits(cfg: &GuardrailConfig, role: Role, op: &str) -> bool {
    role_allowlist(cfg, role).iter().any(|pattern| op_matches(pattern, op))
}

pub fn op_matches(pattern: &str, op: &str) -> bool {
    pattern == op
        || pattern == "*"
        || pattern
            .strip_suffix(".*")
            .map(|prefix| op.starts_with(prefix) && op.as_bytes().get(prefix.len()) == Some(&b'.'))
            .unwrap_or(false)
}

// ---------------------------------------------------------------- policy evaluation

pub fn evaluate(conn: &Connection, request: &GateRequest<'_>) -> Result<Decision, BusError> {
    let cfg = config(conn, Some(request.project_id))?;
    match request.kind {
        GateKind::Write => evaluate_write(&cfg, request),
        GateKind::Commit => evaluate_commit(&cfg, request),
        GateKind::Exec => evaluate_exec(&cfg, request),
    }
}

fn evaluate_write(cfg: &GuardrailConfig, request: &GateRequest<'_>) -> Result<Decision, BusError> {
    let raw_path = request.path.ok_or_else(|| {
        BusError::invalid("guardrail.path", "kind=write requires path")
    })?;
    let path = match write_target(cfg, request, raw_path)? {
        WriteTarget::InWorktree(path) => path,
        WriteTarget::Scratch => return Ok(Decision::Allow),
        WriteTarget::Outside(decision) => return Ok(*decision),
    };
    if let Some(pattern) = cfg.protected_paths.iter().find(|p| path_matches(p, &path)) {
        if request.skip_policy != Some("protected_path") {
            return Ok(refuse_or_user_hold(
                request.actor,
                "protected_path",
                "guardrail.protected_path",
                format!("{} is protected by {pattern:?}", path.display()),
                json!({"path": path, "pattern": pattern}),
            ));
        }
    }
    let Some(new_text) = request.new_text else {
        // Edit hooks may only provide a diff. Destructive counts still work from it; shape
        // gates need complete text, so a critical path without text is held conservatively.
        if let Some(gate) = cfg.shape_gates.iter().find(|g| path_matches(&g.path, &path)) {
            if request.skip_policy != Some("shape_gate") {
                return Ok(hold(
                    "shape_gate",
                    "guardrail.shape_gate",
                    format!("{} needs complete text for validator {}", path.display(), gate.validator),
                    json!({"path": path, "validator": gate.validator, "reason": "new_text missing"}),
                ));
            }
        }
        if let Some(diff) = request.diff {
            // The file itself is right there. Measuring it is strictly better than inferring
            // a size from the diff, which is what made every diff-only write read as 100%.
            let (removed, added) = diff_counts(diff);
            return destructive_decision(cfg, request, &path, removed, added, file_lines(request.worktree, &path));
        }
        return Err(BusError::invalid(
            "guardrail.write",
            "kind=write requires new_text or diff",
        ));
    };

    if let Some(gate) = cfg.shape_gates.iter().find(|g| path_matches(&g.path, &path)) {
        if request.skip_policy != Some("shape_gate") {
            if let Err(reason) = validate_shape(gate, new_text) {
                return Ok(refuse_or_user_hold(
                    request.actor,
                    "shape_gate",
                    "guardrail.shape_gate",
                    format!("{} failed {}: {reason}", path.display(), gate.validator),
                    json!({"path": path, "validator": gate.validator, "reason": reason}),
                ));
            }
        }
    }

    let absolute = request.worktree.join(&path);
    let old = match std::fs::metadata(&absolute) {
        Ok(meta) if meta.len() > 64 * 1024 * 1024 => {
            return Ok(hold(
                "destructive_write",
                "guardrail.destructive_write",
                format!("{} is larger than the 64 MiB comparison cap", path.display()),
                json!({"path": path, "bytes": meta.len(), "reason": "comparison_cap"}),
            ));
        }
        Ok(_) => std::fs::read_to_string(&absolute).unwrap_or_default(),
        Err(_) => String::new(),
    };
    let (removed, added) = changed_line_counts(&old, new_text);
    destructive_decision(
        cfg,
        request,
        &path,
        removed,
        added,
        Some(old.lines().count() as u32),
    )
}

/// Where a write is aimed. Scratch space outside the worktree is allowed outright: it is not
/// a repo-integrity concern, and refusing it puts an agent between two mandatory systems (D102).
enum WriteTarget {
    InWorktree(PathBuf),
    Scratch,
    Outside(Box<Decision>),
}

/// Absolute roots this session may write to, worktree first. The process temp directory is
/// always included, which is where every provider harness puts its own scratchpad.
pub fn write_roots(cfg: &GuardrailConfig, worktree: &Path) -> Vec<PathBuf> {
    let mut roots = vec![worktree.to_path_buf()];
    if let Some(home) = directories::BaseDirs::new() {
        let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from)
            .unwrap_or_else(|| home.home_dir().join(".codex"));
        roots.push(codex.join("memories"));
        let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)
            .unwrap_or_else(|| home.home_dir().join(".claude"));
        // Claude shares auto-memory across a repository's worktrees. Keep the
        // older per-checkout location usable too, without granting transcripts.
        // https://code.claude.com/docs/en/memory
        let primary = gix::open(worktree).ok().and_then(|repo|
            repo.common_dir().parent().map(Path::to_path_buf));
        for project in std::iter::once(worktree).chain(primary.as_deref()) {
            let project = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
            let project_key: String = project.to_string_lossy().chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
            roots.push(claude.join("projects").join(project_key).join("memory"));
        }
    }
    roots.extend(cfg.allowed_write_roots.iter().map(PathBuf::from).filter(|path| path.is_absolute()));
    roots.push(std::env::temp_dir());
    let mut seen = std::collections::HashSet::new();
    roots.retain(|root| seen.insert(root.clone()));
    roots
}

fn write_target(
    cfg: &GuardrailConfig,
    request: &GateRequest<'_>,
    raw_path: &str,
) -> Result<WriteTarget, BusError> {
    let candidate = Path::new(raw_path);
    if !candidate.is_absolute() {
        return relative_path(raw_path).map(WriteTarget::InWorktree);
    }
    if candidate.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(BusError::invalid(
            "guardrail.path",
            format!("{raw_path:?} must contain no .."),
        ));
    }
    if let Ok(relative) = candidate.strip_prefix(request.worktree) {
        if relative.as_os_str().is_empty() {
            return Err(BusError::invalid("guardrail.path", "write target is the worktree itself"));
        }
        return Ok(WriteTarget::InWorktree(relative.to_path_buf()));
    }
    let roots = write_roots(cfg, request.worktree);
    if roots.iter().skip(1).any(|root| candidate.starts_with(root)) {
        return Ok(WriteTarget::Scratch);
    }
    let listed: Vec<String> = roots.iter().map(|root| root.display().to_string()).collect();
    Ok(WriteTarget::Outside(Box::new(refuse_or_user_hold(
        request.actor,
        "write_root",
        "guardrail.write_root",
        format!(
            "{raw_path} is outside every root this session may write to ({})",
            listed.join(", ")
        ),
        json!({"path": raw_path, "write_roots": listed}),
    ))))
}

/// Can git put this file back exactly as it is now? Returns why, or `None` when the content
/// would be lost for good. Committed-and-unmodified is restorable with `git checkout --`;
/// an ignored file is build output or scratch, not repository content. Untracked or modified
/// files carry work that exists nowhere else — those are the ones worth stopping (D114).
fn recoverable(worktree: &Path, path: &Path) -> Option<&'static str> {
    let out = Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["status", "--porcelain", "--ignored=matching", "--untracked-files=all", "--"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let status = String::from_utf8_lossy(&out.stdout);
    match status.trim_end().lines().next() {
        // No status line at all: git is tracking it and it matches HEAD.
        None => Some("committed and unmodified"),
        Some(line) if line.starts_with("!!") => Some("ignored by git"),
        // Untracked, staged, or modified — the current bytes exist only here.
        Some(_) => None,
    }
}

/// How many lines the file on disk has, or `None` when it does not exist or cannot be read.
/// Files past the comparison cap are treated as unmeasured rather than read into memory.
fn file_lines(worktree: &Path, path: &Path) -> Option<u32> {
    let absolute = worktree.join(path);
    let metadata = std::fs::metadata(&absolute).ok()?;
    if metadata.len() > 64 * 1024 * 1024 {
        return None;
    }
    Some(std::fs::read_to_string(&absolute).ok()?.lines().count() as u32)
}

fn destructive_decision(
    cfg: &GuardrailConfig,
    request: &GateRequest<'_>,
    path: &Path,
    removed: u32,
    added: u32,
    old_lines: Option<u32>,
) -> Result<Decision, BusError> {
    let limits = &cfg.destructive_write;
    // A share of the old file is only meaningful when there was a file to take a share of.
    // With no known size — a hook that supplied a diff and nothing else — the old code
    // substituted the removed count, which made every such write exactly 100% and therefore
    // always destructive. The absolute rule still applies; the percentage one stays quiet.
    let removed_pct = old_lines
        .filter(|old| *old >= limits.min_file_lines.max(1))
        .map(|old| removed as f64 * 100.0 / old as f64);
    let over_lines = removed > limits.min_removed_lines;
    let over_pct = removed_pct.is_some_and(|pct| pct > limits.min_removed_pct);
    if (over_lines || over_pct) && request.skip_policy != Some("destructive_write") {
        // Volume is a proxy; what actually matters is whether the work can come back. Asked
        // only here, on the path that was about to block, so the common write pays nothing.
        if limits.allow_if_recoverable {
            if let Some(reason) = recoverable(request.worktree, path) {
                tracing::debug!(path = %path.display(), reason, "large rewrite allowed: git can restore it");
                return Ok(Decision::Allow);
            }
        }
        let share = match removed_pct {
            Some(pct) => format!(" ({pct:.1}% of the file)"),
            None => String::new(),
        };
        let breached = if over_lines {
            format!("more than {} lines", limits.min_removed_lines)
        } else {
            format!("more than {:.1}% of a file of {} lines or more", limits.min_removed_pct, limits.min_file_lines)
        };
        return Ok(refuse_or_user_hold(
            request.actor,
            "destructive_write",
            "guardrail.destructive_write",
            format!("{} removes {removed} lines{share}: {breached}", path.display()),
            json!({
                "path": path, "removed_lines": removed, "added_lines": added,
                "old_lines": old_lines, "removed_pct": removed_pct,
                "limit_lines": limits.min_removed_lines,
                "limit_pct": limits.min_removed_pct,
                "min_file_lines": limits.min_file_lines,
            }),
        ));
    }
    Ok(Decision::Allow)
}

fn evaluate_commit(cfg: &GuardrailConfig, request: &GateRequest<'_>) -> Result<Decision, BusError> {
    let numstat = match request.diff {
        Some(diff) => diff.to_string(),
        None => git_output(request.worktree, &["diff", "--cached", "--numstat"])? ,
    };
    let mut files = 0u32;
    let mut lines = 0u32;
    let mut touched = Vec::new();
    for line in numstat.lines() {
        let mut fields = line.splitn(3, '\t');
        let added = fields.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let removed = fields.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let Some(path) = fields.next() else { continue };
        files += 1;
        lines = lines.saturating_add(added).saturating_add(removed);
        touched.push(path.to_string());
    }
    if let Some((path, pattern)) = touched.iter().find_map(|path| {
        cfg.protected_paths
            .iter()
            .find(|pattern| path_matches(pattern, Path::new(path)))
            .map(|pattern| (path, pattern))
    }) {
        if request.skip_policy != Some("protected_path") {
            return Ok(refuse_or_user_hold(
                request.actor,
                "protected_path",
                "guardrail.protected_path",
                format!("commit touches protected path {path}"),
                json!({"path": path, "pattern": pattern, "files": touched}),
            ));
        }
    }
    if (files > cfg.caps.files || lines > cfg.caps.lines) && request.skip_policy != Some("cap") {
        return Ok(refuse_or_user_hold(
            request.actor,
            "cap",
            "guardrail.cap",
            format!(
                "commit changes {files} files / {lines} lines; caps are {} files / {} lines. \
                 Commit in smaller pieces, or ask for guardrails.caps to be raised for this project",
                cfg.caps.files, cfg.caps.lines
            ),
            json!({"files": files, "lines": lines, "cap_files": cfg.caps.files, "cap_lines": cfg.caps.lines, "paths": touched}),
        ));
    }
    Ok(Decision::Allow)
}

fn evaluate_exec(cfg: &GuardrailConfig, request: &GateRequest<'_>) -> Result<Decision, BusError> {
    let command = request.command.ok_or_else(|| {
        BusError::invalid("guardrail.command", "kind=exec requires command")
    })?;
    if let Some((pattern, argv)) = denied_match(&cfg.denied_commands, command) {
        if request.skip_policy != Some("denied_command") {
            return Ok(refuse_or_user_hold(
                request.actor,
                "denied_command",
                "guardrail.command",
                format!(
                    "{} runs the denied command {pattern:?}; test a command with guardrail.check first, or ask for a hold",
                    argv.join(" ")
                ),
                json!({"command": command, "pattern": pattern, "argv": argv}),
            ));
        }
    }
    Ok(Decision::Allow)
}

/// One shell word plus whether *every* character of it came from inside quotes. A partly
/// quoted word counts as unquoted: when in doubt, still inspect it.
struct Word {
    text: String,
    quoted: bool,
}

/// Split a command line into the individual commands it runs, each as words. A deliberately
/// small shell reader — enough to tell an argument from a quoted string, which is the whole
/// difference between running a denied command and merely naming one (D103).
#[allow(unused_assignments)] // the final end_command! clears state it will never re-read
fn shell_commands(line: &str) -> Vec<Vec<Word>> {
    let mut commands: Vec<Vec<Word>> = Vec::new();
    let mut words: Vec<Word> = Vec::new();
    let mut text = String::new();
    let mut any_bare = false;
    let mut started = false;
    let mut chars = line.chars().peekable();

    macro_rules! end_word {
        () => {
            if started {
                words.push(Word { text: std::mem::take(&mut text), quoted: !any_bare });
                any_bare = false;
                started = false;
            }
        };
    }
    macro_rules! end_command {
        () => {{
            end_word!();
            if !words.is_empty() {
                commands.push(std::mem::take(&mut words));
            }
        }};
    }

    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                started = true;
                let quote = c;
                for inner in chars.by_ref() {
                    if inner == quote {
                        break;
                    }
                    text.push(inner);
                }
            }
            '\\' => {
                started = true;
                any_bare = true;
                if let Some(escaped) = chars.next() {
                    text.push(escaped);
                }
            }
            ';' | '\n' | '|' | '&' => {
                // `&&` / `||` are two characters for one separator; a single one separates too.
                if chars.peek() == Some(&c) {
                    chars.next();
                }
                end_command!();
            }
            c if c.is_whitespace() => end_word!(),
            c => {
                started = true;
                any_bare = true;
                text.push(c);
            }
        }
    }
    end_command!();
    commands
}

/// Does any command in `line` actually *run* one of the denied patterns? Returns the pattern
/// and the offending argv. Quoted data that merely mentions a pattern is not a match.
fn denied_match(patterns: &[String], line: &str) -> Option<(String, Vec<String>)> {
    for command in shell_commands(line) {
        if is_guardrail_dry_run(&command) {
            continue;
        }
        let bare: Vec<String> = command
            .iter()
            .filter(|word| !word.quoted)
            .map(|word| word.text.to_ascii_lowercase())
            .collect();
        for pattern in patterns {
            let wanted: Vec<String> = pattern
                .split_whitespace()
                .map(|part| part.to_ascii_lowercase())
                .collect();
            if wanted.is_empty() || wanted.len() > bare.len() {
                continue;
            }
            if bare.windows(wanted.len()).any(|window| window == wanted.as_slice()) {
                let argv = command.iter().map(|word| word.text.clone()).collect();
                return Some((pattern.clone(), argv));
            }
        }
    }
    None
}

/// `relay q guardrail.check ...` is the sanctioned way to ask "would this be allowed?".
/// Blocking the question would make the safe path the unavailable one, so it stays open.
fn is_guardrail_dry_run(command: &[Word]) -> bool {
    let Some(first) = command.first() else { return false };
    let binary = Path::new(&first.text)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&first.text);
    binary == "relay" && command.iter().any(|word| word.text == "guardrail.check")
}

fn refuse_or_user_hold(
    actor: &Actor,
    policy: &str,
    code: &str,
    message: String,
    details: Value,
) -> Decision {
    if matches!(actor, Actor::User | Actor::Test) {
        hold(
            policy,
            "guardrail.user_bypass",
            format!("{message}; confirm to bypass this policy"),
            json!({"policy": policy, "original_code": code, "original": details}),
        )
    } else if matches!(policy, "destructive_write" | "shape_gate") {
        hold(policy, code, message, details)
    } else {
        Decision::Refuse(BusError::refused(code, message).with_details(details))
    }
}

fn hold(policy: &str, code: &str, message: String, details: Value) -> Decision {
    Decision::Hold {
        policy: policy.to_string(),
        error: BusError::held(code, message).with_details(details.clone()),
        details,
    }
}

fn validate_shape(gate: &ShapeGate, text: &str) -> Result<(), String> {
    match gate.validator.as_str() {
        "non_empty" if text.trim().is_empty() => Err("file is empty".into()),
        "non_empty" => Ok(()),
        "json" => serde_json::from_str::<Value>(text).map(|_| ()).map_err(|e| e.to_string()),
        "json_non_empty_array" => match serde_json::from_str::<Value>(text) {
            Ok(Value::Array(v)) if !v.is_empty() => Ok(()),
            Ok(Value::Array(_)) => Err("array is empty".into()),
            Ok(_) => Err("root is not an array".into()),
            Err(e) => Err(e.to_string()),
        },
        "json_non_empty_object" => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(v)) if !v.is_empty() => Ok(()),
            Ok(Value::Object(_)) => Err("object is empty".into()),
            Ok(_) => Err("root is not an object".into()),
            Err(e) => Err(e.to_string()),
        },
        other => Err(format!("unknown validator {other:?}")),
    }
}

fn diff_counts(diff: &str) -> (u32, u32) {
    let mut removed = 0;
    let mut added = 0;
    for line in diff.lines() {
        if line.starts_with("---") || line.starts_with("+++") {
            continue;
        }
        if line.starts_with('-') {
            removed += 1;
        } else if line.starts_with('+') {
            added += 1;
        }
    }
    (removed, added)
}

/// Linear multiset line comparison. Moving an unchanged line is not destructive; removing
/// one of N duplicate lines counts exactly once. This deliberately avoids quadratic LCS work
/// on large generated files (SPEC §15's performance doctrine).
fn changed_line_counts(old: &str, new: &str) -> (u32, u32) {
    let mut old_counts: HashMap<&str, u32> = HashMap::new();
    let mut new_counts: HashMap<&str, u32> = HashMap::new();
    for line in old.lines() {
        *old_counts.entry(line).or_default() += 1;
    }
    for line in new.lines() {
        *new_counts.entry(line).or_default() += 1;
    }
    let removed = old_counts
        .iter()
        .map(|(line, count)| count.saturating_sub(*new_counts.get(line).unwrap_or(&0)))
        .sum();
    let added = new_counts
        .iter()
        .map(|(line, count)| count.saturating_sub(*old_counts.get(line).unwrap_or(&0)))
        .sum();
    (removed, added)
}

fn git_output(worktree: &Path, args: &[&str]) -> Result<String, BusError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(args)
        .output()
        .map_err(|e| BusError::unavailable("git.unavailable", e.to_string()))?;
    if !out.status.success() {
        return Err(BusError::conflict(
            "git.failed",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn relative_path(path: &str) -> Result<PathBuf, BusError> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err(BusError::invalid(
            "guardrail.path",
            format!("{path:?} must be worktree-relative and contain no .."),
        ));
    }
    Ok(path.to_path_buf())
}

fn validate_pattern(pattern: &str) -> Result<(), BusError> {
    if pattern.is_empty() || Path::new(pattern).is_absolute() || pattern.split('/').any(|p| p == "..") {
        return Err(BusError::invalid(
            "guardrail.config",
            format!("bad relative path pattern {pattern:?}"),
        ));
    }
    Ok(())
}

/// Exact path, directory prefix, or a glob. `*` matches within one path segment; `**`
/// crosses separators. The distinction matters: `src/*` used to quietly protect the whole
/// subtree under `src`, so a pattern written for one directory locked an entire tree (D113).
pub fn path_matches(pattern: &str, path: &Path) -> bool {
    let pattern = pattern.trim_matches('/');
    let path = path.to_string_lossy().trim_matches('/').to_string();
    if pattern == path || path.starts_with(&format!("{pattern}/")) {
        return true;
    }
    glob_match(pattern.as_bytes(), path.as_bytes())
}

/// Backtracking matcher over one `*` (segment-local) or `**` (crosses `/`) at a time.
fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    // (pattern index to resume at, text index to retry from, whether that star may cross `/`)
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize, bool)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            let crosses = pattern.get(p + 1) == Some(&b'*');
            while p < pattern.len() && pattern[p] == b'*' {
                p += 1;
            }
            // A trailing `**` swallows the rest; a trailing `*` swallows the last segment.
            if p == pattern.len() {
                return crosses || !text[t..].contains(&b'/');
            }
            star = Some((p, t, crosses));
            continue;
        }
        if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
            continue;
        }
        match star {
            // Retry with the star consuming one more character — but a segment-local star
            // may never consume a separator.
            Some((after_star, star_text, crosses)) if crosses || text[star_text] != b'/' => {
                t = star_text + 1;
                p = after_star;
                star = Some((after_star, t, crosses));
            }
            _ => return false,
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

// ---------------------------------------------------------------- holds / notifications

#[allow(clippy::too_many_arguments)] // frozen request attribution is clearer as named scalar fields
pub fn insert_hold(
    tx: &Transaction,
    request: &Request,
    project_id: Id,
    session_id: Option<Id>,
    session: Option<&str>,
    policy: &str,
    details: &Value,
    now: &str,
) -> Result<Id, BusError> {
    let mut frozen = request.clone();
    frozen.token = None; // never persist session secrets
    tx.execute(
        "INSERT INTO holds(project_id, session_id, session, actor, op, envelope, policy, details, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', ?9)",
        params![
            project_id,
            session_id,
            session,
            request.actor.to_string(),
            request.op,
            serde_json::to_string(&frozen).bus()?,
            policy,
            serde_json::to_string(details).bus()?,
            now,
        ],
    )
    .bus()?;
    let hold_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO notifications(project_id, category, title, body, link, read, created_at)
         VALUES (?1, 'guardrail', 'Guardrail hold', ?2, ?3, 0, ?4)",
        params![
            project_id,
            format!("{} held {}", request.actor, request.op),
            // The hold is answered inside the session that tripped it, so the deep link
            // carries the session name the notification centre needs to get there (D101).
            json!({"op": "guardrail.confirm", "payload": {"hold_id": hold_id, "session": session}}).to_string(),
            now,
        ],
    )
    .bus()?;
    Ok(hold_id)
}

pub fn insert_refusal_notification(
    tx: &Transaction,
    project_id: Id,
    actor: &Actor,
    error: &BusError,
    now: &str,
) -> Result<(), BusError> {
    tx.execute(
        "INSERT INTO notifications(project_id, category, title, body, link, read, created_at)
         VALUES (?1, 'guardrail', 'Guardrail refusal', ?2, NULL, 0, ?3)",
        params![project_id, format!("{actor}: {} — {}", error.code, error.message), now],
    )
    .bus()?;
    Ok(())
}

pub fn hold_by_id(conn: &Connection, id: Id) -> Result<Hold, BusError> {
    conn.query_row("SELECT * FROM holds WHERE id = ?1", [id], hold_row)
        .optional()
        .bus()?
        .ok_or_else(|| BusError::not_found("guardrail.hold_not_found", format!("no hold {id}")))
}

pub fn frozen_request(conn: &Connection, id: Id) -> Result<Request, BusError> {
    let raw: String = conn
        .query_row("SELECT envelope FROM holds WHERE id = ?1", [id], |r| r.get(0))
        .optional()
        .bus()?
        .ok_or_else(|| BusError::not_found("guardrail.hold_not_found", format!("no hold {id}")))?;
    serde_json::from_str(&raw).map_err(crate::engine::internal)
}

pub fn hold_row(row: &Row) -> rusqlite::Result<Hold> {
    let envelope: String = row.get("envelope")?;
    let payload_hash = serde_json::from_str::<Request>(&envelope)
        .map(|request| crate::audit::payload_hash(&request.payload))
        .unwrap_or_else(|_| crate::audit::payload_hash(&Value::String(envelope)));
    let actor = Actor::parse(&row.get::<_, String>("actor")?).unwrap_or(Actor::System);
    let resolved_by = row
        .get::<_, Option<String>>("resolved_by")?
        .and_then(|v| Actor::parse(&v).ok());
    let state = match row.get::<_, String>("state")?.as_str() {
        "confirmed" => HoldState::Confirmed,
        "rejected" => HoldState::Rejected,
        "expired" => HoldState::Expired,
        _ => HoldState::Open,
    };
    Ok(Hold {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        session_id: row.get("session_id")?,
        session: row.get("session")?,
        actor,
        op: row.get("op")?,
        payload_hash,
        policy: row.get("policy")?,
        details: serde_json::from_str(&row.get::<_, String>("details")?).unwrap_or(Value::Null),
        state,
        created_at: row.get("created_at")?,
        resolved_at: row.get("resolved_at")?,
        resolved_by,
    })
}

#[cfg(test)]
mod glob_tests {
    use super::path_matches;
    use std::path::Path;

    fn m(pattern: &str, path: &str) -> bool {
        path_matches(pattern, Path::new(path))
    }

    #[test]
    fn a_single_star_stays_inside_one_segment() {
        assert!(m("src/*.rs", "src/main.rs"));
        assert!(!m("src/*.rs", "src/deep/main.rs"), "one star must not cross a separator");
        assert!(m("src/**/*.rs", "src/deep/main.rs"));
        assert!(m("**/secrets.toml", "a/b/secrets.toml"));
        assert!(m("*.env", ".env"));
        assert!(!m("*.env", "config/.env"));
    }

    #[test]
    fn directory_prefixes_and_exact_paths_still_match() {
        assert!(m("secret", "secret/key"));
        assert!(m("secret/key", "secret/key"));
        assert!(!m("secret", "secretly/key"), "a prefix must end on a separator");
        assert!(m("**", "anything/at/all"));
        assert!(m("src/*", "src/main.rs"));
        assert!(!m("src/*", "src/a/b.rs"));
    }
}
