//! Guardrails and session authorization (SPEC §5, BUS.md §9). Evaluation is pure:
//! [`evaluate`] returns allow/refuse/hold. The `guardrail.gate` handler is the only caller
//! that persists a hold; `guardrail.check` never writes.

use crate::engine::IntoBus;
use crate::sessions;
use crate::shell::{self, Word};
use relay_bus::envelope::{Actor, Request};
use relay_bus::error::BusError;
use relay_bus::registry::{Callable, OpEntry, OpKind, Registry};
use relay_bus::types::{
    GateKind, GuardrailConfig, GuardrailLayer, Hold, HoldState, Id, Role, Session, ShapeGate,
};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub mod grants;
pub use grants::Grants;

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
    /// Exceptions a person granted this session. Each lifts only the rule it names.
    pub grants: Option<&'a Grants>,
    /// A rename, move or delete rather than a write of new content (BUS.md §9.2): only the
    /// path rules apply — where it lands, protected paths, and a shape gate on a destination
    /// (validated against `new_text` when the caller supplies it). Never reads the file.
    pub path_only: bool,
}

// ---------------------------------------------------------------- configuration

/// Where a guardrail config is read for. A project's layer sits on its workspace's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    Global,
    Workspace(Id),
    Project(Id),
}

/// Read the effective config: defaults <- global settings <- project override <- the
/// project's legacy `protected_paths` / `critical_files` fields.
pub fn config(conn: &Connection, project_id: Option<Id>) -> Result<GuardrailConfig, BusError> {
    config_for(conn, project_id.map_or(ConfigScope::Global, ConfigScope::Project))
}

/// The effective config at one scope: defaults <- global <- the workspace <- the project <- the
/// project's legacy `protected_paths` / `critical_files` columns.
pub fn config_for(conn: &Connection, scope: ConfigScope) -> Result<GuardrailConfig, BusError> {
    let layers = layers(conn, scope)?;
    let root = layers.stages.last().map(|(_, value)| value.clone()).unwrap_or_default();
    typed(root)
}

/// Every layer that applies at a scope: each one's raw stored overrides, and the effective
/// value after it was applied. Read with one pass over the stored `guardrails.*` leaves.
pub struct Layers {
    pub workspace_id: Option<Id>,
    /// `(layer, raw overrides)`, defaults excluded, in application order.
    pub raw: Vec<(GuardrailLayer, Value)>,
    /// `(layer, effective after it)`, defaults first.
    pub stages: Vec<(GuardrailLayer, Value)>,
    /// The project's legacy columns contributed something.
    pub legacy: bool,
}

pub fn layers(conn: &Connection, scope: ConfigScope) -> Result<Layers, BusError> {
    // The project row first: it names the workspace whose layer sits under it.
    let (workspace_id, project) = match scope {
        ConfigScope::Global => (None, None),
        ConfigScope::Workspace(id) => (Some(id), None),
        ConfigScope::Project(project_id) => {
            let row: Option<(Id, String, String)> = conn
                .prepare_cached("SELECT workspace_id, protected_paths, critical_files FROM projects WHERE id = ?1")
                .bus()?
                .query_row([project_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()
                .bus()?;
            let Some((workspace_id, protected, critical)) = row else {
                return Err(BusError::not_found("project.not_found", format!("no project {project_id}")));
            };
            (Some(workspace_id), Some((project_id, protected, critical)))
        }
    };
    let workspace_prefix = workspace_id.map(|id| format!("workspaces.{id}"));
    let project_prefix = project.as_ref().map(|(id, _, _)| format!("projects.{id}"));
    let mut global = json!({});
    let mut workspace = json!({});
    let mut own = json!({});
    let mut stmt = conn
        .prepare_cached("SELECT path, value FROM settings WHERE path LIKE 'guardrails.%' ORDER BY path")
        .bus()?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .bus()?;
    for row in rows {
        let (path, raw) = row.bus()?;
        let Some(relative) = path.strip_prefix("guardrails.") else { continue };
        let under = |prefix: &str| -> Option<String> {
            let rest = relative.strip_prefix(prefix)?;
            if rest.is_empty() {
                Some(String::new())
            } else {
                rest.strip_prefix('.').map(str::to_owned)
            }
        };
        let (target, at) = if let Some(at) = workspace_prefix.as_deref().and_then(under) {
            (&mut workspace, at)
        } else if let Some(at) = project_prefix.as_deref().and_then(under) {
            (&mut own, at)
        } else if under("projects").is_some() || under("workspaces").is_some() {
            continue;
        } else {
            (&mut global, relative.to_string())
        };
        let value: Value = serde_json::from_str(&raw).map_err(crate::engine::internal)?;
        put_raw(target, &at, value);
    }

    let mut current = crate::handlers::settings::defaults()["guardrails"].clone();
    if let Some(map) = current.as_object_mut() {
        map.remove("projects");
        map.remove("workspaces");
    }
    let mut stages = vec![(GuardrailLayer::Default, current.clone())];
    let mut raws = Vec::new();
    let mut apply = |layer: GuardrailLayer, raw: Value, current: &mut Value| {
        crate::handlers::settings::merge_value(current, &raw);
        stages.push((layer, current.clone()));
        raws.push((layer, raw));
    };
    apply(GuardrailLayer::Global, global, &mut current);
    if workspace_id.is_some() {
        apply(GuardrailLayer::Workspace, workspace, &mut current);
    }
    let mut legacy = false;
    if let Some((_, protected, critical)) = project {
        let mut next = current.clone();
        crate::handlers::settings::merge_value(&mut next, &own);
        let mut protected_paths: Vec<String> = serde_json::from_str(&protected).unwrap_or_default();
        legacy |= !protected_paths.is_empty();
        protected_paths.extend(
            next["protected_paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
        protected_paths.sort();
        protected_paths.dedup();
        next["protected_paths"] = json!(protected_paths);

        let critical: Vec<String> = serde_json::from_str(&critical).unwrap_or_default();
        legacy |= !critical.is_empty();
        let gates = next["shape_gates"].as_array_mut().ok_or_else(|| {
            BusError::invalid("guardrail.config", "shape_gates must be an array")
        })?;
        for path in critical {
            if !gates.iter().any(|g| g["path"] == path) {
                gates.push(json!({"path": path, "validator": "non_empty"}));
            }
        }
        stages.push((GuardrailLayer::Project, next));
        raws.push((GuardrailLayer::Project, own));
    }
    Ok(Layers { workspace_id, raw: raws, stages, legacy })
}

/// Deserialize and validate one effective tree.
pub fn typed(root: Value) -> Result<GuardrailConfig, BusError> {
    let cfg: GuardrailConfig = serde_json::from_value(root)
        .map_err(|e| BusError::invalid("guardrail.config", format!("invalid guardrail config: {e}")))?;
    validate_config(&cfg)?;
    Ok(cfg)
}

/// Settings rows are leaves. Rebuild one layer's overrides from them. A stored `null` means
/// nothing; a stored object (the `{}` a cleared subtree leaves behind) merges rather than
/// replacing the subtree, which used to wipe `caps` and fail every read after.
fn put_raw(root: &mut Value, path: &str, value: Value) {
    if value.is_null() {
        return;
    }
    if path.is_empty() {
        crate::handlers::settings::merge_value(root, &value);
        return;
    }
    let mut current = root;
    for part in path.split('.') {
        if !current.is_object() {
            *current = json!({});
        }
        current = current
            .as_object_mut()
            .unwrap()
            .entry(part.to_string())
            .or_insert(Value::Null);
    }
    if value.is_object() && current.is_object() {
        crate::handlers::settings::merge_value(current, &value);
    } else {
        *current = value;
    }
}

/// Drop every object that a `null` patch emptied, bottom up. An empty object stored as a leaf
/// carries no meaning, and the root itself stays.
pub fn prune_empty(value: &mut Value) {
    if let Value::Object(map) = value {
        for child in map.values_mut() {
            prune_empty(child);
        }
        map.retain(|_, child| !(child.is_null() || child.as_object().is_some_and(|m| m.is_empty())));
    }
}

/// The patch that undoes `before -> after` under [`merge_value`]: keys `after` added become
/// `null`, everything `before` held comes back.
pub fn inverse_patch(before: &Value, after: &Value) -> Value {
    let (Some(before_map), Some(after_map)) = (before.as_object(), after.as_object()) else {
        return before.clone();
    };
    let mut patch = serde_json::Map::new();
    for (key, value) in before_map {
        match after_map.get(key) {
            Some(next) if next == value => {}
            Some(next) if value.is_object() && next.is_object() => {
                patch.insert(key.clone(), inverse_patch(value, next));
            }
            _ => {
                patch.insert(key.clone(), value.clone());
            }
        }
    }
    for key in after_map.keys() {
        if !before_map.contains_key(key) {
            patch.insert(key.clone(), Value::Null);
        }
    }
    Value::Object(patch)
}

/// Leaf paths of an effective tree: objects recurse, arrays and scalars are leaves.
pub fn leaf_paths(value: &Value, prefix: &str, out: &mut Vec<String>) {
    match value.as_object() {
        Some(map) if !map.is_empty() => {
            for (key, child) in map {
                let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
                leaf_paths(child, &path, out);
            }
        }
        _ => out.push(prefix.to_string()),
    }
}

/// Does a raw override tree set `path` (or an ancestor of it, as a non-object)?
pub fn sets_path(raw: &Value, path: &str) -> bool {
    let mut current = raw;
    for part in path.split('.') {
        match current.get(part) {
            Some(next) if next.is_object() => current = next,
            Some(_) => return true,
            None => return false,
        }
    }
    true
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
        let assigned: bool = conn
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM task_sessions WHERE session_id=?1 AND task_id=?2)")
            .map_err(crate::engine::internal)?
            .query_row(rusqlite::params![session.id, task_id], |record| record.get(0))
            .map_err(crate::engine::internal)?;
        if !assigned {
            return Err(BusError::not_own("task"));
        }
    }
    if entry.meta.kind == OpKind::Query {
        return Ok(());
    }

    // Asking a person is always open: a role that may not act may still say it is stuck.
    if entry.name == grants::OP {
        return Ok(());
    }
    let cfg = config(conn, Some(session.project_id))?;
    let allowed = role_allowlist(&cfg, session.role);
    if allowed.iter().any(|pattern| op_matches(pattern, entry.name)) {
        return Ok(());
    }
    // A role whose shell is otherwise closed (the reviewer) may still run what its instructions
    // depend on: a Codex session reaches the bus only through `$RELAY_BIN q …` in its shell, and
    // refusing that left a Codex reviewer unable to bootstrap, mail or finish (RA-014). The
    // gate still judges the command as it judges every other.
    if entry.name == "guardrail.gate"
        && payload.get("kind").and_then(Value::as_str) == Some("exec")
        && payload.get("command").and_then(Value::as_str).is_some_and(read_only_exec)
    {
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
    if entry.name == grants::OP {
        return self_only("every agent may ask for an exception".to_string());
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

/// [`evaluate`], and when that would not allow an agent's action, once more with the grants
/// its session holds. The common path pays nothing for grants. Returns the decision and the
/// grants it leaned on — consume those only if the action then really happens.
pub fn evaluate_granted(
    conn: &Connection,
    request: &GateRequest<'_>,
    session_id: Option<Id>,
) -> Result<(Decision, Vec<Id>), BusError> {
    let first = evaluate(conn, request)?;
    let (Some(session_id), true) = (session_id, request.actor.is_agent()) else {
        return Ok((first, Vec::new()));
    };
    if matches!(first, Decision::Allow) {
        return Ok((first, Vec::new()));
    }
    let grants = Grants::load(conn, session_id)?;
    if grants.is_empty() {
        return Ok((first, Vec::new()));
    }
    let second = evaluate(conn, &GateRequest { grants: Some(&grants), ..request.clone() })?;
    let used = if matches!(second, Decision::Allow) { grants.used() } else { Vec::new() };
    Ok((second, used))
}

fn granted_path(request: &GateRequest<'_>, path: &Path) -> bool {
    request.grants.is_some_and(|grants| grants.covers_path(request.worktree, path))
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
        if request.skip_policy != Some("protected_path") && !granted_path(request, &path) {
            return Ok(refuse_or_user_hold(
                request.actor,
                "protected_path",
                "guardrail.protected_path",
                format!("{} is protected by {pattern:?}", path.display()),
                json!({"path": path, "pattern": pattern}),
            ));
        }
    }
    if request.path_only {
        if let Some(gate) = cfg.shape_gates.iter().find(|g| path_matches(&g.path, &path)) {
            if request.skip_policy != Some("shape_gate") {
                let Some(new_text) = request.new_text else {
                    return Ok(hold(
                        "shape_gate",
                        "guardrail.shape_gate",
                        format!("{} needs complete text for validator {}", path.display(), gate.validator),
                        json!({"path": path, "validator": gate.validator, "reason": "new_text missing"}),
                    ));
                };
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
        return Ok(Decision::Allow);
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
        // A confirmed hold or a path grant lifts this exactly as it lifts the line counts;
        // without that, a confirm replayed straight back into the same hold.
        Ok(meta) if meta.len() > 64 * 1024 * 1024 => {
            if request.skip_policy == Some("destructive_write") || granted_path(request, &path) {
                return Ok(Decision::Allow);
            }
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
    // A person let this session write here. It is outside the repository, so nothing below
    // (protected paths, shape gates, rewrite size) has anything left to judge.
    if request.grants.is_some_and(|grants| grants.covers_root(candidate)) {
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
    let out = crate::proc::output_with_timeout(
        Command::new("git")
            .arg("-C")
            .arg(worktree)
            .args(["status", "--porcelain", "--ignored=matching", "--untracked-files=all", "--"])
            .arg(path),
        GIT_TIMEOUT,
    )
    .ok()??;
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
    if (over_lines || over_pct) && request.skip_policy != Some("destructive_write") && !granted_path(request, path) {
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
    // Every protected path the commit touches needs its own grant.
    if let Some((path, pattern)) = touched.iter().find_map(|path| {
        cfg.protected_paths
            .iter()
            .find(|pattern| path_matches(pattern, Path::new(path)))
            .filter(|_| !granted_path(request, Path::new(path)))
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
    if (files > cfg.caps.files || lines > cfg.caps.lines)
        && request.skip_policy != Some("cap")
        && !request.grants.is_some_and(|grants| grants.covers_caps(files, lines, &cfg.caps))
    {
        return Ok(refuse_or_user_hold(
            request.actor,
            "cap",
            "guardrail.cap",
            format!(
                "commit changes {files} files / {lines} lines; caps are {} files / {} lines. \
                 Commit in smaller pieces; if the change cannot be split, ask the user to let it through",
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
    if request.actor.is_agent() {
        if let Some(argv) = self_approval(command) {
            // No grant lifts this one: an agent approving its own exception is no exception.
            return Ok(Decision::Refuse(
                BusError::refused(
                    "guardrail.self_approval",
                    format!("{} would answer a guardrail as the user; only the user can do that", argv.join(" ")),
                )
                .with_details(json!({"command": command, "argv": argv}))
                .with_hint("Ask with guardrail.request and wait for guardrail.request_resolved; the user answers it in Relay."),
            ));
        }
    }
    if request.skip_policy == Some("denied_command") {
        return Ok(Decision::Allow);
    }
    let uncovered = denied_matches(&cfg.denied_commands, command).into_iter().find(|hit| {
        !request.grants.is_some_and(|grants| grants.covers_command(&hit.bare))
    });
    if let Some(DeniedMatch { pattern, argv, .. }) = uncovered {
        return Ok(refuse_or_user_hold(
            request.actor,
            "denied_command",
            "guardrail.command",
            format!(
                "{} runs the denied command {pattern:?}; test a command with guardrail.check first",
                argv.join(" ")
            ),
            json!({"command": command, "pattern": pattern, "argv": argv}),
        ));
    }
    Ok(Decision::Allow)
}

/// The user-only answers to a guardrail, reached through the CLI as the user. Best effort: the
/// socket does not authenticate `user`, so this closes the obvious door, not every door.
const USER_ONLY_ANSWERS: &[&str] = &[
    "guardrail.confirm", "guardrail.reject", "guardrail.config.set", "guardrail.grant.revoke", "settings.set", "settings.reset",
];

/// Does a command line invoke Relay as the user, or answer a guardrail through it? Returns the
/// offending argv.
fn self_approval(line: &str) -> Option<Vec<String>> {
    for command in shell_commands(line) {
        let words: Vec<&str> = command.iter().map(|word| word.text.as_str()).collect();
        let relay = words.iter().any(|word| {
            let stem = Path::new(word).file_stem().and_then(|stem| stem.to_str()).unwrap_or(word);
            stem.to_ascii_lowercase().starts_with("relay") || word.contains("RELAY_BIN")
        });
        // Clearing the session identity makes the CLI fall back to the user actor.
        let sheds_identity = words.iter().enumerate().any(|(at, word)| {
            word.starts_with("RELAY_ACTOR=")
                || word.starts_with("RELAY_SESSION=")
                || (*word == "-u" || *word == "--unset") && words.get(at + 1) == Some(&"RELAY_SESSION")
                || *word == "unset" && words.get(at + 1) == Some(&"RELAY_SESSION")
        });
        let as_user = words.iter().enumerate().any(|(at, word)| {
            matches!(*word, "--actor=user" | "--actor=test")
                || *word == "--actor" && matches!(words.get(at + 1), Some(&"user") | Some(&"test"))
        });
        let answers = words.iter().any(|word| USER_ONLY_ANSWERS.contains(word));
        if (relay && (as_user || answers)) || sheds_identity {
            return Some(command.iter().map(|word| word.text.clone()).collect());
        }
    }
    None
}

/// Split a command line into the individual commands it runs, each as words (D103). The
/// reader is shared with the device-lease gate, so both see the same commands.
fn shell_commands(line: &str) -> Vec<Vec<Word>> {
    shell::commands(line)
}

/// One place a command line runs a denied pattern.
struct DeniedMatch {
    pattern: String,
    argv: Vec<String>,
    /// The command's unquoted words, lowercased.
    bare: Vec<String>,
}

/// Every command in `line` that actually *runs* one of the denied patterns. Quoted data that
/// merely mentions a pattern is not a match. All of them, not the first: a grant has to cover
/// each one for the line to pass.
fn denied_matches(patterns: &[String], line: &str) -> Vec<DeniedMatch> {
    let patterns: Vec<(&String, DeniedPattern)> = patterns
        .iter()
        .filter_map(|pattern| DeniedPattern::parse(pattern).map(|parsed| (pattern, parsed)))
        .collect();
    let mut hits = Vec::new();
    for command in shell_commands(line) {
        if is_guardrail_dry_run(&command) {
            continue;
        }
        let bare: Vec<String> = command
            .iter()
            .filter(|word| !word.quoted)
            .map(|word| word.text.to_ascii_lowercase())
            .collect();
        for (pattern, parsed) in &patterns {
            if parsed.runs_in(&command) {
                hits.push(DeniedMatch {
                    pattern: (*pattern).clone(),
                    argv: command.iter().map(|word| word.text.clone()).collect(),
                    bare: bare.clone(),
                });
            }
        }
    }
    hits
}

/// A denied pattern read the way its program reads it: `git reset --hard` is the program `git`,
/// the subcommand `reset` and the flag `--hard`. Matching on that, rather than on the pattern's
/// literal words in a row, is what makes `git -C app reset HEAD~1 --hard`, `rm -fr build` and
/// `git push origin main --force` the commands they are (RA-011).
struct DeniedPattern {
    program: String,
    /// The words right after the program that are not options, in order: `reset`, `push`.
    subcommands: Vec<String>,
    /// Every flag the pattern names, as a set, clusters expanded: `-rf` is `-r` and `-f`.
    flags: Vec<String>,
    /// Any other word the pattern names (`rm -rf /` names `/`), anywhere after the subcommand.
    operands: Vec<String>,
}

impl DeniedPattern {
    fn parse(pattern: &str) -> Option<Self> {
        let mut words = pattern.split_whitespace();
        let program = shell::program(words.next()?).to_ascii_lowercase();
        let mut parsed = Self { program, subcommands: Vec::new(), flags: Vec::new(), operands: Vec::new() };
        for word in words {
            if word.starts_with('-') && word.len() > 1 {
                parsed.flags.extend(expand_flags(&parsed.program, word));
            } else if parsed.flags.is_empty() && parsed.operands.is_empty() {
                parsed.subcommands.push(word.to_ascii_lowercase());
            } else {
                parsed.operands.push(word.to_ascii_lowercase());
            }
        }
        Some(parsed)
    }

    /// Whether `command` runs this pattern. The program may sit anywhere in the command, so
    /// `sudo rm -rf`, `timeout 60 git clean -fd` and `find . -exec rm -rf {} ;` are all seen.
    fn runs_in(&self, command: &[Word]) -> bool {
        (0..command.len()).any(|at| self.runs_at(command, at))
    }

    fn runs_at(&self, command: &[Word], at: usize) -> bool {
        let word = &command[at];
        // A quoted program still runs (`"rm" -rf`), but only as the command's own first word;
        // elsewhere a quoted word is an argument, which is data.
        if word.quoted && at != 0 || shell::program(&word.text).to_ascii_lowercase() != self.program {
            return false;
        }
        // Quoted text with a space in it is a string, never a flag or a subcommand (D103).
        let rest: Vec<&str> = command[at + 1..]
            .iter()
            .filter(|word| !(word.quoted && word.text.contains(char::is_whitespace)))
            .map(|word| word.text.as_str())
            .collect();
        let mut next = 0;
        for wanted in &self.subcommands {
            while let Some(word) = rest.get(next).filter(|word| word.starts_with('-')) {
                next += if takes_value(&self.program, word) { 2 } else { 1 };
            }
            match rest.get(next) {
                Some(word) if word.eq_ignore_ascii_case(wanted) => next += 1,
                _ => return false,
            }
        }
        let after = rest.get(next..).unwrap_or_default();
        let flags: Vec<String> = after
            .iter()
            .filter(|word| word.starts_with('-') && word.len() > 1)
            .flat_map(|word| expand_flags(&self.program, word))
            .collect();
        let operands: Vec<String> = after
            .iter()
            .filter(|word| !word.starts_with('-'))
            .map(|word| word.to_ascii_lowercase())
            .collect();
        // `git push origin +main` is a force push spelled as a refspec.
        let plus_refspec = self.program == "git"
            && self.subcommands.first().is_some_and(|sub| sub == "push")
            && operands.iter().any(|word| word.len() > 1 && word.starts_with('+'));
        self.flags.iter().all(|flag| flags.contains(flag) || flag == "-f" && plus_refspec)
            && self.operands.iter().all(|operand| operands.contains(operand))
    }
}

/// A flag word as the individual flags it sets, each spelled one way: `-rf` is `-r` and `-f`;
/// `--force=x` is `--force`; for `rm` and `git`, `--force` is `-f`; for `rm`, `-R` and
/// `--recursive` are `-r`. Short flags keep their case — `-X` and `-x` differ in most tools.
fn expand_flags(program: &str, word: &str) -> Vec<String> {
    let canonical = |flag: String| -> String {
        match (program, flag.as_str()) {
            ("rm" | "git", "--force") => "-f".into(),
            ("rm", "-R" | "--recursive") => "-r".into(),
            _ => flag,
        }
    };
    if let Some(long) = word.strip_prefix("--") {
        if long.is_empty() {
            return Vec::new();
        }
        let name = long.split('=').next().unwrap_or(long).to_ascii_lowercase();
        return vec![canonical(format!("--{name}"))];
    }
    word.chars().skip(1).map(|c| canonical(format!("-{c}"))).collect()
}

/// Global options that take the next word as their value, so it is not mistaken for the
/// subcommand: `git -C ../app reset --hard`, `git -c core.x=y push --force`.
fn takes_value(program: &str, word: &str) -> bool {
    program == "git" && matches!(word, "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path" | "--config-env")
}

/// Is `line` one command that changes nothing but what the bus itself authorizes: a Relay CLI
/// call through `$RELAY_BIN` (each op is then authorized on its own), or a read-only look at
/// the checkout (`git diff`, `cat`, `rg`, …)? Deliberately narrow: no chaining, pipes,
/// redirection, substitution or expansion outside single quotes, and no option that lets a
/// reader write or run something (`git -c`, `--output`, `--ext-diff`, `rg --pre`).
pub(crate) fn read_only_exec(line: &str) -> bool {
    read_only_exec_at(line, 0)
}

fn read_only_exec_at(line: &str, depth: u8) -> bool {
    let line = line.trim();
    let (relay, rest) = ["\"$RELAY_BIN\"", "\"${RELAY_BIN}\"", "$RELAY_BIN", "${RELAY_BIN}"]
        .iter()
        .find_map(|bin| line.strip_prefix(bin).filter(|rest| rest.starts_with(char::is_whitespace)))
        .map_or((false, line), |rest| (true, rest));
    if !plain_words(rest) {
        return false;
    }
    // The reader also lists what a `bash -lc '…'` runs as a command of its own; the wrapper
    // case below judges that inner line itself, so only the outer command is read here.
    let commands = shell_commands(rest);
    let Some(command) = commands.first() else { return false };
    let words: Vec<&str> = command.iter().map(|word| word.text.as_str()).collect();
    let wrapper = depth == 0 && matches!(words.first(), Some(&("bash" | "sh" | "zsh")));
    if commands.len() != 1 && !wrapper {
        return false;
    }
    if relay {
        // The session's own identity, never an override: `--actor user` is the user.
        return matches!(words.first(), Some(&("q" | "cmd" | "ops" | "schema" | "ping")))
            && !words.iter().any(|word| word.starts_with("--actor") || word.starts_with("--instance"));
    }
    let Some((&program, args)) = words.split_first() else { return false };
    match program {
        // Codex may hand over its own wrapper: judge the one command inside it.
        "bash" | "sh" | "zsh" if depth == 0 => {
            matches!(args, [flag, script] if matches!(*flag, "-c" | "-lc")) && read_only_exec_at(args[1], 1)
        }
        "ls" | "cat" | "head" | "tail" | "wc" | "pwd" | "grep" => true,
        "rg" => !args.iter().any(|arg| arg.starts_with("--pre")),
        "git" => {
            // The subcommand is the first word that is neither an option nor `-C`'s directory.
            let sub = args.iter().enumerate()
                .find(|(at, arg)| !arg.starts_with('-') && (*at == 0 || args[at - 1] != "-C"))
                .map(|(_, arg)| arg);
            matches!(sub, Some(&("diff" | "log" | "show" | "status" | "blame")))
                && !args.iter().any(|arg| {
                    *arg == "-c" || ["--output", "--ext-diff", "--exec-path", "--config-env"].iter().any(|bad| arg.starts_with(bad))
                })
        }
        _ => false,
    }
}

/// No shell syntax outside single quotes beyond words and spaces: nothing that chains, pipes,
/// redirects, substitutes or expands. Inside double quotes, no `$`, backtick or backslash.
fn plain_words(line: &str) -> bool {
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('\''), _) => {}
            (Some(_), '$' | '`' | '\\') => return false,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, ';' | '&' | '|' | '<' | '>' | '`' | '$' | '(' | ')' | '{' | '}' | '\\' | '\n' | '#') => return false,
            (None, _) => {}
        }
    }
    quote.is_none()
}

/// The unquoted words of each command in `line`, lowercased — how a command grant is compared.
pub(crate) fn command_words(line: &str) -> Vec<Vec<String>> {
    shell_commands(line)
        .into_iter()
        .map(|command| {
            command.iter().filter(|word| !word.quoted).map(|word| word.text.to_ascii_lowercase()).collect()
        })
        .collect()
}

/// `relay q guardrail.check ...` is the sanctioned way to ask "would this be allowed?".
/// Blocking the question would make the safe path the unavailable one, so it stays open.
fn is_guardrail_dry_run(command: &[Word]) -> bool {
    let Some(first) = command.first() else { return false };
    let binary = Path::new(&first.text)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&first.text);
    (binary == "relay" || first.text.contains("RELAY_BIN"))
        && command.iter().any(|word| word.text == "guardrail.check")
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
        // Held for a person; for a destructive write the agent may also ask for the path.
        let mut error = BusError::held(code, message).with_details(details.clone());
        if let Some((kind, value)) = grants::suggestion(policy, &details) {
            error = error.with_hint(grants::hint(kind, &value));
        }
        Decision::Hold { policy: policy.to_string(), error, details }
    } else {
        let mut error = BusError::refused(code, message);
        let mut details = details;
        // The agent is told it can ask, and exactly what to ask for.
        if let Some((kind, value)) = grants::suggestion(policy, &details) {
            error = error.with_hint(grants::hint(kind, &value));
            if let Some(map) = details.as_object_mut() {
                map.insert("exception".into(), json!({"kind": kind, "value": value}));
            }
        }
        Decision::Refuse(error.with_details(details))
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

/// Both git probes here run with the store lock held, so they get a deadline (D144).
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

fn git_output(worktree: &Path, args: &[&str]) -> Result<String, BusError> {
    let out = crate::proc::output_with_timeout(Command::new("git").arg("-C").arg(worktree).args(args), GIT_TIMEOUT)
        .map_err(|e| BusError::unavailable("git.unavailable", e.to_string()))?
        .ok_or_else(|| BusError::unavailable("git.timeout", format!("git {} took longer than 10s", args.join(" "))))?;
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
/// crosses separators, and a `**/` component matches zero or more whole directories, so
/// `**/.env` covers the root `.env` as well as `a/b/.env`. The distinction matters: `src/*`
/// used to quietly protect the whole subtree under `src`, so a pattern written for one
/// directory locked an entire tree (D113).
pub fn path_matches(pattern: &str, path: &Path) -> bool {
    let pattern = pattern.trim_matches('/');
    let path = path.to_string_lossy().trim_matches('/').to_string();
    if pattern == path || path.starts_with(&format!("{pattern}/")) {
        return true;
    }
    glob_match(pattern.as_bytes(), path.as_bytes())
}

/// Recursive matcher: every star gets its own backtrack point. One shared resume point used to
/// let a later `*` overwrite an earlier `**`'s, so `**/*.pem` matched only one directory deep.
fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    // Every (pattern, text) position is tried at most once, so an agent-supplied
    // `file.search` glob such as `*a*a*a*a*b` costs pattern x text, not an exponential walk.
    let mut failed = vec![false; (pattern.len() + 1) * (text.len() + 1)];
    Glob { pattern, text, failed: &mut failed }.from(0, 0)
}

struct Glob<'a> {
    pattern: &'a [u8],
    text: &'a [u8],
    failed: &'a mut [bool],
}

impl Glob<'_> {
    fn from(&mut self, p: usize, t: usize) -> bool {
        let slot = p * (self.text.len() + 1) + t;
        if self.failed[slot] {
            return false;
        }
        let matched = self.step(p, t);
        if !matched {
            self.failed[slot] = true;
        }
        matched
    }

    fn step(&mut self, p: usize, t: usize) -> bool {
        let (pattern, text) = (self.pattern, self.text);
        let Some(&c) = pattern.get(p) else {
            return t == text.len();
        };
        if c != b'*' {
            return text.get(t) == Some(&c) && self.from(p + 1, t + 1);
        }
        let mut after = p;
        while pattern.get(after) == Some(&b'*') {
            after += 1;
        }
        if after - p == 1 {
            // A segment-local star: it may consume anything up to, never across, a separator.
            let end = text[t..].iter().position(|&c| c == b'/').map_or(text.len(), |i| t + i);
            return (t..=end).any(|i| self.from(after, i));
        }
        // `**/` as a whole component matches zero or more directories: nothing at all, or any
        // run that ends on a separator.
        if pattern.get(after) == Some(&b'/') && (p == 0 || pattern[p - 1] == b'/') {
            return self.from(after + 1, t)
                || (t..text.len()).any(|i| text[i] == b'/' && self.from(after + 1, i + 1));
        }
        // Any other `**` (`**.pem`, a trailing `src/**`) crosses separators freely.
        (t..=text.len()).any(|i| self.from(after, i))
    }
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

    /// RA-012: one shared backtrack point let a later `*` overwrite the `**`'s, so these
    /// patterns matched exactly one directory deep and never at the top level.
    #[test]
    fn a_double_star_component_matches_zero_or_more_directories() {
        for path in ["c.pem", "x/c.pem", "x/y/c.pem", "x/y/z/c.pem"] {
            assert!(m("**/*.pem", path), "**/*.pem must match {path}");
        }
        assert!(!m("**/*.pem", "x/c.pem.bak"));
        for path in ["src/main.rs", "src/a/main.rs", "src/a/b/main.rs", "src/a/b/c/main.rs"] {
            assert!(m("src/**/*.rs", path), "src/**/*.rs must match {path}");
        }
        assert!(!m("src/**/*.rs", "lib/a/main.rs"));
        assert!(m("secrets/**/*.json", "secrets/a.json"));
        assert!(m("secrets/**/*.json", "secrets/x/y/a.json"));
        assert!(m("**/secrets.toml", "secrets.toml"));
        assert!(m("**/.env", ".env"));
        assert!(m("**/.env", "a/b/c/.env"));
        assert!(!m("**/.env", "a/b/c/x.env"), "`**/` matches whole directories only");
        assert!(m("**/*.key", "deploy/keys/prod.key"));
        assert!(m("a/**/b/*.txt", "a/x/b/y/b/z.txt"), "the inner star must not starve the outer");
        // A `**` that is not a whole component keeps crossing separators, as it always has.
        assert!(m("**.pem", "x/y/c.pem"));
        assert!(m("src/**", "src/a/b/c.rs"));
        assert!(!m("*.pem", "x/c.pem"));
        let long = "a".repeat(4000);
        assert!(!m("*a*a*a*a*a*a*a*a*b", &long), "a pathological glob must still finish");
    }
}

#[cfg(test)]
mod denied_tests {
    use super::{denied_matches, self_approval};

    fn denied(line: &str) -> bool {
        let defaults = ["rm -rf", "git reset --hard", "git clean -fd", "git push --force"].map(String::from);
        !denied_matches(&defaults, line).is_empty()
    }

    /// RA-010 / RA-011: everyday spellings of the default denied commands, and the same
    /// commands run inside subshells, substitutions and `sh -c`.
    #[test]
    fn every_spelling_of_a_denied_command_is_seen() {
        for line in [
            "rm -rf build", "rm -fr build", "rm -r -f build", "rm -rfv build", "rm -Rf build",
            "rm --recursive --force build", "/bin/rm -rf build", "\"rm\" -rf build", "rm '-rf' build",
            "\\rm -rf build", "sudo rm -rf build", "find . -name x -exec rm -rf {} ;",
            "git reset --hard", "git reset HEAD~1 --hard", "git -C ../app reset --hard",
            "git -c core.x=y reset --hard origin/main",
            "git clean -fd", "git clean -df", "git clean -fdx", "git clean -xfd", "git clean -f -d",
            "git -C sub clean -fd",
            "git push --force", "git push -f origin main", "git push origin main --force",
            "git push origin +main", "git push --force=true origin main",
            "(git reset --hard)", "cd sub && (git clean -fd)", "(cd frontend && rm -rf node_modules)",
            "echo $(rm -rf build)", "echo `rm -rf build`", "echo \"$(rm -rf build)\"",
            "sh -c 'rm -rf build'", "bash -lc \"git reset --hard\"", "eval 'git push -f'",
            "echo \"\\\"\" ; rm -rf build ; echo \"\\\"\"",
            "relay q guardrail.check $(rm -rf ~)",
        ] {
            assert!(denied(line), "{line:?} runs a denied command");
        }
    }

    /// D103 still holds: naming a command is not running it.
    #[test]
    fn quoted_mentions_and_near_misses_are_not_denied() {
        for line in [
            "echo \"rm -rf /\"", "git commit -m \"git reset --hard was a mistake\"",
            "relay q guardrail.check '{\"command\":\"rm -rf /\"}'", "rm -r build", "rm -f build",
            "git reset --soft HEAD~1", "git clean -n", "git push origin main", "git push -u origin main",
            "git log --grep reset", "cargo test", "grep -rf patterns.txt src",
        ] {
            assert!(!denied(line), "{line:?} does not run a denied command");
        }
        // A dry run is exempt, and only the dry run.
        assert!(denied("relay q guardrail.check && rm -rf /tmp/x"));
    }

    #[test]
    fn a_pattern_operand_must_be_present() {
        let patterns = ["rm -rf /".to_string(), "make deploy".to_string()];
        assert!(!denied_matches(&patterns, "rm -rf build").iter().any(|hit| hit.pattern == "rm -rf /"));
        assert!(denied_matches(&patterns, "rm -fr /").iter().any(|hit| hit.pattern == "rm -rf /"));
        assert!(!denied_matches(&patterns, "make -j8 deploy").is_empty());
        assert!(denied_matches(&patterns, "make test").is_empty());
    }

    #[test]
    fn self_approval_is_seen_inside_a_subshell() {
        assert!(self_approval("(relay --actor user q guardrail.confirm '{\"hold_id\":1}')").is_some());
        assert!(self_approval("echo $(relay q guardrail.confirm '{}')").is_some());
    }
}
