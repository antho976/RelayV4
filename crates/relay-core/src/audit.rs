//! The audit log (BUS.md §0.4, §5.1a, §5.3, §13): one row per audited mutation outcome,
//! `req_id` UNIQUE forever, replay-able.

use anyhow::Result;
use relay_bus::envelope::{Actor, Response};
use relay_bus::error::BusError;
use relay_bus::types::{AuditKind, AuditRow, Id, UndoOp};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Payloads and results larger than this are hashed but not stored (BUS.md §11.4).
pub const STORE_LIMIT: usize = 64 * 1024;

/// The provider hooks call `session.report` on every tool use, and `usage.report` on every
/// window refresh. Their payloads are high-volume and near-identical, and 64 KB of each is what
/// grew the audit table past the rest of the store put together. The hash still identifies the
/// request and the result summary still replays it; only the readable copy is bounded.
pub const CHATTY_STORE_LIMIT: usize = 2 * 1024;

fn store_limit(op: &str) -> usize {
    match op {
        "session.report" | "usage.report" => CHATTY_STORE_LIMIT,
        _ => STORE_LIMIT,
    }
}

pub fn payload_hash(payload: &Value) -> String {
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// What the pipeline records for one request.
pub struct AuditEntry<'a> {
    pub ts: &'a str,
    pub req_id: Uuid,
    pub parent_req: Option<Uuid>,
    pub actor: &'a Actor,
    pub on_behalf_of: Option<&'a Actor>,
    pub session_id: Option<Id>,
    pub op: &'a str,
    pub project_id: Option<Id>,
    pub payload: &'a Value,
    /// `Ok(result)` or the error the request ended with.
    pub outcome: &'a Result<Value, BusError>,
    pub hold_id: Option<Id>,
    pub undo_op: Option<&'a UndoOp>,
    pub undo_of: Option<Id>,
}

pub fn kind_of(outcome: &Result<Value, BusError>) -> AuditKind {
    match outcome {
        Ok(_) => AuditKind::Ok,
        Err(e) => match e.kind.audit_kind() {
            "held" => AuditKind::Held,
            "refused" => AuditKind::Refused,
            _ => AuditKind::Error,
        },
    }
}

fn kind_str(k: AuditKind) -> &'static str {
    match k {
        AuditKind::Ok => "ok",
        AuditKind::Held => "held",
        AuditKind::Refused => "refused",
        AuditKind::Error => "error",
    }
}

/// Append one row inside the caller's transaction. Returns the row id.
pub fn append(tx: &Transaction, e: &AuditEntry) -> Result<Id> {
    let payload_json = serde_json::to_string(e.payload)?;
    // Over the limit the row keeps a marker rather than NULL: "we chose not to store this" and
    // "there was nothing to store" are different answers to `audit.get`.
    let oversize = serde_json::json!({"truncated": true, "bytes": payload_json.len()}).to_string();
    let payload_col = if payload_json.len() <= store_limit(e.op) { &payload_json } else { &oversize };
    let (kind, code, summary) = match e.outcome {
        Ok(v) => {
            let s = serde_json::to_string(v)?;
            let s = if s.len() <= STORE_LIMIT { s } else { serde_json::json!({"truncated": true, "bytes": s.len()}).to_string() };
            (AuditKind::Ok, None, Some(s))
        }
        Err(err) => (kind_of(e.outcome), Some(err.code.clone()), Some(serde_json::to_string(err)?)),
    };
    tx.execute(
        "INSERT INTO audit (ts, req_id, parent_req, actor, on_behalf_of, session_id, op, project_id, kind, code, hold_id,
                            payload_hash, payload, result_summary, undo_op, undo_of)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            e.ts,
            e.req_id.to_string(),
            e.parent_req.map(|u| u.to_string()),
            e.actor.to_string(),
            e.on_behalf_of.map(|a| a.to_string()),
            e.session_id,
            e.op,
            e.project_id,
            kind_str(kind),
            code,
            e.hold_id,
            payload_hash(e.payload),
            payload_col.as_str(),
            summary,
            e.undo_op.map(|u| serde_json::to_string(u).unwrap_or_default()),
            e.undo_of,
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

/// A previously recorded outcome for `req_id`, for idempotent replay (BUS.md §5.3).
pub struct Recorded {
    pub payload_hash: String,
    pub response: Response,
}

pub fn lookup(conn: &Connection, req_id: Uuid) -> Result<Option<Recorded>> {
    let row = conn
        .query_row(
            "SELECT payload_hash, kind, result_summary FROM audit WHERE req_id = ?1",
            [req_id.to_string()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)),
        )
        .optional()?;
    let Some((hash, kind, summary)) = row else { return Ok(None) };
    let response = match (kind.as_str(), summary) {
        ("ok", Some(s)) => {
            let v: Value = serde_json::from_str(&s)?;
            if v.get("truncated").is_some() && v.get("bytes").is_some() && v.as_object().map(|o| o.len() == 2).unwrap_or(false) {
                Response::err(req_id, BusError::conflict("bus.replay_unavailable", "the recorded result was too large to keep; re-issue with a new id"))
            } else {
                Response::ok(req_id, v)
            }
        }
        (_, Some(s)) => Response::err(req_id, serde_json::from_str(&s)?),
        (_, None) => Response::err(req_id, BusError::internal("audit row without a recorded outcome")),
    };
    Ok(Some(Recorded { payload_hash: hash, response }))
}

fn parse_actor(s: &str) -> Actor {
    Actor::parse(s).unwrap_or(Actor::System)
}

fn row_to_audit(r: &Row) -> rusqlite::Result<AuditRow> {
    let kind = match r.get::<_, String>("kind")?.as_str() {
        "ok" => AuditKind::Ok,
        "held" => AuditKind::Held,
        "refused" => AuditKind::Refused,
        _ => AuditKind::Error,
    };
    let payload: Option<String> = r.get("payload")?;
    let summary: Option<String> = r.get("result_summary")?;
    let undo: Option<String> = r.get("undo_op")?;
    Ok(AuditRow {
        id: r.get("id")?,
        ts: r.get("ts")?,
        req_id: r.get("req_id")?,
        parent_req: r.get("parent_req")?,
        actor: parse_actor(&r.get::<_, String>("actor")?),
        on_behalf_of: r.get::<_, Option<String>>("on_behalf_of")?.map(|s| parse_actor(&s)),
        session_id: r.get("session_id")?,
        op: r.get("op")?,
        project_id: r.get("project_id")?,
        kind,
        code: r.get("code")?,
        hold_id: r.get("hold_id")?,
        payload_hash: r.get("payload_hash")?,
        payload: payload.and_then(|s| serde_json::from_str(&s).ok()),
        result_summary: summary.and_then(|s| serde_json::from_str(&s).ok()),
        undo_op: undo.and_then(|s| serde_json::from_str(&s).ok()),
        undo_of: r.get("undo_of")?,
        undone_by: r.get("undone_by")?,
    })
}

pub struct ListFilter<'a> {
    pub project_id: Option<Id>,
    pub actor: Option<&'a Actor>,
    pub session_id: Option<Id>,
    pub op_prefix: Option<&'a str>,
    pub parent_req: Option<&'a str>,
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub limit: u32,
}

pub fn list(conn: &Connection, f: &ListFilter) -> Result<Vec<AuditRow>> {
    let mut sql = String::from("SELECT * FROM audit WHERE 1=1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(p) = f.project_id { sql.push_str(" AND project_id = ?"); args.push(Box::new(p)); }
    if let Some(a) = f.actor { sql.push_str(" AND actor = ?"); args.push(Box::new(a.to_string())); }
    if let Some(s) = f.session_id { sql.push_str(" AND session_id = ?"); args.push(Box::new(s)); }
    if let Some(p) = f.op_prefix { sql.push_str(" AND op LIKE ? ESCAPE '\\'"); args.push(Box::new(format!("{}%", p.replace('%', "\\%").replace('_', "\\_")))); }
    if let Some(p) = f.parent_req { sql.push_str(" AND parent_req = ?"); args.push(Box::new(p.to_string())); }
    if let Some(s) = f.since { sql.push_str(" AND ts >= ?"); args.push(Box::new(s.to_string())); }
    if let Some(u) = f.until { sql.push_str(" AND ts <= ?"); args.push(Box::new(u.to_string())); }
    sql.push_str(" ORDER BY id DESC LIMIT ?");
    args.push(Box::new(f.limit.min(1000)));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())), row_to_audit)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get(conn: &Connection, id: Id) -> Result<Option<AuditRow>> {
    Ok(conn.query_row("SELECT * FROM audit WHERE id = ?1", [id], row_to_audit).optional()?)
}
