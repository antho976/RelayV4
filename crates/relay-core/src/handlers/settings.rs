//! `settings.*` (BUS.md §10.16): a dotted-path tree stored as leaf rows, overlaid on defaults.

use crate::engine::{Ctx, Engine, IntoBus};
use relay_bus::error::BusError;
use relay_bus::ops::notify::{SettingsGet, SettingsReset, SettingsSet, ValueOut};
use rusqlite::{params, Transaction};
use serde_json::{json, Map, Value};

/// Defaults for the top-level keys BUS.md §10.16 names. Each feature fills its subtree in
/// when it lands; unknown paths are allowed (settings are a tree, not a schema).
///
/// Guardrail evaluation reads this for every audited agent mutation, so the tree is built
/// once and handed out by reference; callers clone only the branch they overlay.
pub fn defaults() -> &'static Value {
    static DEFAULTS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
        json!({
            "appearance": { "mode": "dark", "panel_alpha": 1.0, "wallpaper": null, "wallpaper_preview": null },
            "notifications": { "sound": "chime", "volume": 0.7, "categories": {} },
            "providers": { "claude": { "path": null }, "codex": { "path": null } },
            "device": { "sdk_path": null, "adb_path": null, "emulator_path": null, "avdmanager_path": null },
            "guardrails": {
                "caps": { "files": 40, "lines": 2000 },
                "destructive_write": { "min_removed_lines": 50, "min_removed_pct": 40, "min_file_lines": 30, "allow_if_recoverable": true },
                "protected_paths": [],
                "shape_gates": [],
                "denied_commands": ["rm -rf", "git reset --hard", "git clean -fd", "git push --force"],
                "allowed_write_roots": [],
                "roles": {
                    "builder": ["task.move", "task.link_commit", "task.changelog.write", "task.update", "mailbox.*", "notes.append", "overlap.flag", "overlap.ack", "integration.request", "session.done", "session.report", "session.intent", "session.claim", "session.release", "usage.report", "guardrail.gate"],
                    "reviewer": ["mailbox.*", "notes.append", "overlap.flag", "task.changelog.write", "session.done", "session.report", "session.intent", "session.claim", "session.release", "usage.report"],
                    "docs": ["task.move", "task.changelog.write", "task.update", "mailbox.*", "notes.append", "notes.create", "notes.update", "overlap.flag", "overlap.ack", "session.done", "session.report", "session.intent", "session.claim", "session.release", "usage.report", "guardrail.gate"]
                },
                "projects": {},
            },
            "undo": { "grace_days": 7 },
            "audit": { "retention_days": 180 },
            "parking": { "idle_minutes": 30 },
            "layout": {},
            "theme": {},
            "keybindings": {
                "palette": "Ctrl+K", "agents": "Ctrl+1", "code": "Ctrl+2", "board": "Ctrl+3",
                "new_session": "Ctrl+N", "settings": "Ctrl+,", "sidebar": "Ctrl+Shift+B"
            },
            "roles": {},
        })
    });
    &DEFAULTS
}

/// `""` is the root (whole tree); otherwise dotted segments of `[A-Za-z0-9_-]`.
fn valid_path(p: &str) -> Result<(), BusError> {
    if p.is_empty() {
        return Ok(());
    }
    if p.split('.').any(|s| s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        return Err(BusError::invalid("settings.path", format!("bad settings path {p:?}")));
    }
    Ok(())
}

fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(m) if !m.is_empty() => {
            for (k, v) in m {
                let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(&p, v, out);
            }
        }
        _ => out.push((prefix.to_string(), v.clone())),
    }
}

fn set_at(root: &mut Value, path: &str, v: Value) {
    if path.is_empty() {
        *root = v;
        return;
    }
    let mut cur = root;
    let segs: Vec<&str> = path.split('.').collect();
    for (i, s) in segs.iter().enumerate() {
        if i == segs.len() - 1 {
            if !cur.is_object() {
                *cur = Value::Object(Map::new());
            }
            cur.as_object_mut().unwrap().insert(s.to_string(), v);
            return;
        }
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        cur = cur.as_object_mut().unwrap().entry(s.to_string()).or_insert(Value::Object(Map::new()));
    }
}

/// RFC-7396-style object merge used by guardrail global/project configuration. Objects
/// recurse; arrays and scalars replace. `null` removes a key from the destination.
pub fn merge_value(dst: &mut Value, patch: &Value) {
    match (dst, patch) {
        (Value::Object(dst), Value::Object(patch)) => {
            for (key, value) in patch {
                if value.is_null() {
                    dst.remove(key);
                } else {
                    merge_value(dst.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
        }
        (dst, patch) => *dst = patch.clone(),
    }
}

fn get_at<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for s in path.split('.') {
        cur = cur.get(s)?;
    }
    Some(cur)
}

/// The full effective tree: defaults overlaid with stored leaves.
pub fn tree(tx: &Transaction) -> Result<Value, BusError> {
    let mut root = defaults().clone();
    let mut stmt = tx.prepare_cached("SELECT path, value FROM settings ORDER BY path").bus()?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .bus()?;
    for row in rows {
        let (p, v) = row.bus()?;
        let v: Value = serde_json::from_str(&v).unwrap_or(Value::Null);
        set_at(&mut root, &p, v);
    }
    Ok(root)
}

/// The rows that can affect the value at `path`: the subtree under it, plus its ancestors —
/// a stored scalar at `appearance` replaces the whole default object below it, so it has to be
/// applied before `appearance.mode` is read back out.
///
/// Everything else in the table is irrelevant to `path`, and reading it is not free: a leaf read
/// of `appearance.mode` used to decode the megabytes parked in `appearance.wallpaper*` on its way
/// past. Ordering by path keeps the ancestor-then-descendant application order [`tree`] relies on
/// (a prefix always sorts before the paths it prefixes).
fn subtree(tx: &Transaction, path: &str) -> Result<Value, BusError> {
    let mut root = defaults().clone();
    let ancestors: Vec<String> = path
        .match_indices('.')
        .map(|(i, _)| path[..i].to_string())
        .collect();
    let like = format!("{}.%", path.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let mut stmt = tx
        .prepare_cached(
            "SELECT path, value FROM settings
             WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'
                OR path IN (SELECT value FROM json_each(?3))
             ORDER BY path",
        )
        .bus()?;
    let rows = stmt
        .query_map(
            params![path, like, serde_json::to_string(&ancestors).bus()?],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .bus()?;
    for row in rows {
        let (p, v) = row.bus()?;
        let v: Value = serde_json::from_str(&v).unwrap_or(Value::Null);
        set_at(&mut root, &p, v);
    }
    Ok(get_at(&root, path).cloned().unwrap_or(Value::Null))
}

pub fn get(tx: &Transaction, path: Option<&str>) -> Result<Value, BusError> {
    match path {
        None => tree(tx),
        Some("") => tree(tx),
        Some(p) => {
            valid_path(p)?;
            subtree(tx, p)
        }
    }
}

fn delete_under(tx: &Transaction, path: &str) -> Result<(), BusError> {
    if path.is_empty() {
        tx.execute("DELETE FROM settings", []).bus()?;
        return Ok(());
    }
    tx.execute("DELETE FROM settings WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'",
        params![path, format!("{}.%", path.replace('%', "\\%").replace('_', "\\_"))]).bus()?;
    Ok(())
}

pub fn set(tx: &Transaction, path: &str, value: &Value, now: &str) -> Result<(), BusError> {
    valid_path(path)?;
    delete_under(tx, path)?;
    let mut leaves = Vec::new();
    flatten(path, value, &mut leaves);
    for (p, v) in leaves {
        tx.execute("INSERT INTO settings(path, value, updated_at) VALUES (?1, ?2, ?3)",
            params![p, serde_json::to_string(&v).bus()?, now]).bus()?;
    }
    Ok(())
}

pub fn register(e: &mut Engine) {
    e.register::<SettingsGet>(|ctx, p| Ok(ValueOut { value: get(ctx.tx(), p.path.as_deref())? }));
    e.register::<SettingsSet>(|ctx: &mut Ctx, p| {
        let before = get(ctx.tx(), Some(&p.path))?;
        set(ctx.tx(), &p.path, &p.value, &ctx.now.clone())?;
        ctx.set_undo("settings.set", json!({ "path": p.path, "value": before }), None);
        let value = get(ctx.tx(), Some(&p.path))?;
        ctx.emit("settings.changed", json!({ "path": p.path, "value": value }));
        Ok(ValueOut { value })
    });
    e.register::<SettingsReset>(|ctx: &mut Ctx, p| {
        let path = p.path.unwrap_or_default();
        valid_path(&path)?;
        let before = get(ctx.tx(), Some(&path))?;
        delete_under(ctx.tx(), &path)?;
        ctx.set_undo("settings.set", json!({ "path": path, "value": before }), None);
        let value = get(ctx.tx(), Some(&path))?;
        ctx.emit("settings.changed", json!({ "path": path, "value": value }));
        Ok(ValueOut { value })
    });
}
