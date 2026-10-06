//! Guardrail exceptions: an agent that cannot progress asks with `guardrail.request`, a person
//! answers with `guardrail.confirm {scope}` or `guardrail.reject`, and the grant then lifts that
//! one rule for that one session — once, or until the session ends.
//!
//! No migration: a request is a hold (`op = 'guardrail.request'`, `policy = 'exception'`), so it
//! shows up wherever holds already do and expires with the session the same way. The grant
//! lives in the hold's `details.grant`.

use crate::engine::IntoBus;
use relay_bus::envelope::Actor;
use relay_bus::error::BusError;
use relay_bus::types::{ExceptionKind, GrantScope, GuardrailCaps, GuardrailException, Hold, HoldState, Id};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::{Component, Path};

pub const OP: &str = "guardrail.request";
pub const POLICY: &str = "exception";
/// The event an asking agent waits for.
pub const RESOLVED_EVENT: &str = "guardrail.request_resolved";

/// `session.bootstrap`'s answer to "a guardrail refused me and I cannot go on".
pub const BOOTSTRAP_HINT: &str = "A refusal you cannot progress without is not the end: call guardrail.request \
{kind, value, reason} with the kind and value the refusal's hint names (command, path or cap), then bus.wait \
{events:[\"guardrail.request_resolved\"], matching:{request_id}} and retry once it is approved. The user may \
approve it once or for this session, or deny it with a reason. After a timeout, re-check with \
guardrail.request.get. Shape gates are not grantable, and only the user can approve.";

#[derive(Debug, Clone)]
pub struct Grant {
    pub id: Id,
    pub kind: ExceptionKind,
    pub value: String,
    pub scope: GrantScope,
}

/// The live grants of one session, plus which of them an evaluation leaned on. Only the ones
/// actually used are consumed, and only when the action they let through was allowed.
#[derive(Debug, Default)]
pub struct Grants {
    pub list: Vec<Grant>,
    used: RefCell<Vec<Id>>,
}

impl Grants {
    /// Approved, unrevoked grants of a session; a `once` grant only until it was used.
    pub fn load(conn: &Connection, session_id: Id) -> Result<Grants, BusError> {
        let mut stmt = conn
            .prepare_cached(
                "SELECT id, details FROM holds
                 WHERE session_id = ?1 AND op = 'guardrail.request' AND state = 'confirmed'
                 ORDER BY id",
            )
            .bus()?;
        let rows = stmt
            .query_map([session_id], |r| Ok((r.get::<_, Id>(0)?, r.get::<_, String>(1)?)))
            .bus()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .bus()?;
        let list = rows
            .into_iter()
            .filter_map(|(id, raw)| {
                let details: Value = serde_json::from_str(&raw).ok()?;
                let grant = Parsed::from(&details);
                if !grant.active() {
                    return None;
                }
                Some(Grant { id, kind: grant.kind?, value: grant.value, scope: grant.scope? })
            })
            .collect();
        Ok(Grants { list, used: RefCell::default() })
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Grants an evaluation used, in first-use order.
    pub fn used(&self) -> Vec<Id> {
        self.used.borrow().clone()
    }

    fn mark(&self, id: Id) {
        let mut used = self.used.borrow_mut();
        if !used.contains(&id) {
            used.push(id);
        }
    }

    fn of(&self, kind: ExceptionKind) -> impl Iterator<Item = &Grant> {
        self.list.iter().filter(move |grant| grant.kind == kind)
    }

    /// A worktree-relative path. A relative grant is a path or glob; an absolute one inside
    /// the worktree is the same thing spelled out.
    pub fn covers_path(&self, worktree: &Path, path: &Path) -> bool {
        let hit = self.of(ExceptionKind::Path).find(|grant| {
            let value = Path::new(grant.value.trim());
            if value.is_absolute() {
                value
                    .strip_prefix(worktree)
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .is_some_and(|relative| super::path_matches(&relative.to_string_lossy(), path))
            } else {
                super::path_matches(&grant.value, path)
            }
        });
        hit.map(|grant| self.mark(grant.id)).is_some()
    }

    /// An absolute path outside the worktree: covered by an absolute grant that is it, or a
    /// directory above it.
    pub fn covers_root(&self, candidate: &Path) -> bool {
        let hit = self.of(ExceptionKind::Path).find(|grant| {
            let value = Path::new(grant.value.trim());
            value.is_absolute() && candidate.starts_with(value)
        });
        hit.map(|grant| self.mark(grant.id)).is_some()
    }

    /// A commit of `files` / `lines` against `caps`: covered by a cap grant raising whichever
    /// limit it names, or by one that names neither, which lifts the caps outright.
    pub fn covers_caps(&self, files: u32, lines: u32, caps: &GuardrailCaps) -> bool {
        let hit = self.of(ExceptionKind::Cap).find(|grant| {
            let (granted_files, granted_lines) = parse_caps(&grant.value);
            if granted_files.is_none() && granted_lines.is_none() {
                return true;
            }
            files <= granted_files.unwrap_or(caps.files).max(caps.files)
                && lines <= granted_lines.unwrap_or(caps.lines).max(caps.lines)
        });
        hit.map(|grant| self.mark(grant.id)).is_some()
    }

    /// One command that runs a denied pattern, as its unquoted words, lowercased. A grant
    /// covers it when the grant is that exact command — `rm -rf target` lifts `rm -rf target`
    /// and not `rm -rf target /`. A grant ending in a lone `*` covers everything that starts
    /// with the words before it, and the person approving it reads that `*`.
    pub fn covers_command(&self, bare: &[String]) -> bool {
        let hit = self.of(ExceptionKind::Command).find(|grant| {
            super::command_words(&grant.value).iter().any(|words| match words.split_last() {
                Some((last, head)) if last == "*" => !head.is_empty() && bare.starts_with(head),
                Some(_) => bare == words.as_slice(),
                None => false,
            })
        });
        hit.map(|grant| self.mark(grant.id)).is_some()
    }
}

/// `files=60 lines=5000` (also `files:60`, commas, either order). Unknown words are ignored.
pub fn parse_caps(value: &str) -> (Option<u32>, Option<u32>) {
    let mut files = None;
    let mut lines = None;
    for part in value.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let Some((key, number)) = part.split_once(['=', ':']) else { continue };
        let Ok(number) = number.trim().parse::<u32>() else { continue };
        match key.trim().to_ascii_lowercase().as_str() {
            "files" => files = Some(number),
            "lines" => lines = Some(number),
            _ => {}
        }
    }
    (files, lines)
}

/// Count one use against each grant an allowed action leaned on. Returns the `once` grants
/// this used up, which are over now.
pub fn consume(conn: &Connection, ids: &[Id], now: &str) -> Result<Vec<Id>, BusError> {
    let mut spent = Vec::new();
    for id in ids {
        let scope: Option<String> = conn
            .prepare_cached(
                "UPDATE holds SET details = json_set(details,
                     '$.grant.uses', COALESCE(json_extract(details, '$.grant.uses'), 0) + 1,
                     '$.grant.used_at', ?1)
                 WHERE id = ?2 AND op = 'guardrail.request'
                 RETURNING json_extract(details, '$.grant.scope')",
            )
            .bus()?
            .query_row(params![now, id], |r| r.get(0))
            .optional()
            .bus()?
            .flatten();
        if scope.as_deref() == Some("once") {
            spent.push(*id);
        }
    }
    Ok(spent)
}

/// Check a request's shape before it reaches a person.
pub fn validate(kind: ExceptionKind, value: &str, reason: &str) -> Result<(), BusError> {
    if reason.trim().is_empty() {
        return Err(BusError::invalid("guardrail.request", "reason must say why you cannot progress without this"));
    }
    let value = value.trim();
    match kind {
        ExceptionKind::Command if value.is_empty() => {
            Err(BusError::invalid("guardrail.request", "value must be the exact command you need to run"))
        }
        ExceptionKind::Path if value.is_empty() => {
            Err(BusError::invalid("guardrail.request", "value must be the path you need to write"))
        }
        ExceptionKind::Path if Path::new(value).components().any(|c| matches!(c, Component::ParentDir)) => {
            Err(BusError::invalid("guardrail.request", format!("{value:?} must contain no ..")))
        }
        ExceptionKind::Cap if !value.is_empty() && parse_caps(value) == (None, None) => Err(BusError::invalid(
            "guardrail.request",
            "value for a cap is `files=N lines=M` (either one), or empty to lift the caps",
        )),
        _ => Ok(()),
    }
}

/// The stored details of a new request.
pub fn details(kind: ExceptionKind, value: &str, reason: &str, scope: GrantScope) -> Value {
    json!({
        "kind": kind, "value": value.trim(), "reason": reason.trim(),
        "requested_scope": scope, "grant": null,
    })
}

/// One line a person can read: what the agent wants to do.
pub fn describe(kind: ExceptionKind, value: &str) -> String {
    match kind {
        ExceptionKind::Command => format!("run `{value}`"),
        ExceptionKind::Path => format!("write `{value}`"),
        ExceptionKind::Cap if value.trim().is_empty() => "commit past the change caps".to_string(),
        ExceptionKind::Cap => format!("commit past the change caps ({value})"),
    }
}

/// What the stored details say, leniently.
struct Parsed {
    kind: Option<ExceptionKind>,
    value: String,
    reason: String,
    requested: GrantScope,
    scope: Option<GrantScope>,
    uses: u32,
    used_at: Option<String>,
    revoked_at: Option<String>,
    denial: Option<String>,
}

impl Parsed {
    fn from(details: &Value) -> Parsed {
        let grant = &details["grant"];
        Parsed {
            kind: serde_json::from_value(details["kind"].clone()).ok(),
            value: details["value"].as_str().unwrap_or_default().to_string(),
            reason: details["reason"].as_str().unwrap_or_default().to_string(),
            requested: serde_json::from_value(details["requested_scope"].clone()).unwrap_or(GrantScope::Once),
            scope: serde_json::from_value(grant["scope"].clone()).ok(),
            uses: grant["uses"].as_u64().unwrap_or(0) as u32,
            used_at: grant["used_at"].as_str().map(str::to_owned),
            revoked_at: grant["revoked_at"].as_str().map(str::to_owned),
            denial: details["rejection_reason"].as_str().map(str::to_owned),
        }
    }

    fn active(&self) -> bool {
        match self.scope {
            Some(GrantScope::Session) => self.revoked_at.is_none(),
            Some(GrantScope::Once) => self.revoked_at.is_none() && self.uses == 0,
            None => false,
        }
    }
}

/// A request hold, read as the exception it carries.
pub fn exception(hold: &Hold) -> GuardrailException {
    let parsed = Parsed::from(&hold.details);
    let active = hold.state == HoldState::Confirmed && parsed.active();
    GuardrailException {
        id: hold.id,
        project_id: hold.project_id,
        session: hold.session.clone(),
        kind: parsed.kind.unwrap_or(ExceptionKind::Command),
        value: parsed.value,
        reason: parsed.reason,
        requested_scope: parsed.requested,
        state: hold.state,
        scope: parsed.scope,
        active,
        uses: parsed.uses,
        used_at: parsed.used_at,
        revoked_at: parsed.revoked_at,
        denial_reason: parsed.denial,
        created_at: hold.created_at.clone(),
        resolved_at: hold.resolved_at.clone(),
        resolved_by: hold.resolved_by.clone(),
    }
}

/// The exception with this id, or a typed not-found when it is no request at all.
pub fn by_id(conn: &Connection, id: Id) -> Result<GuardrailException, BusError> {
    let hold = super::hold_by_id(conn, id)
        .map_err(|_| BusError::not_found("guardrail.request_not_found", format!("no exception request {id}")))?;
    if hold.op != OP {
        return Err(BusError::not_found(
            "guardrail.request_not_found",
            format!("hold {id} is not an exception request"),
        ));
    }
    Ok(exception(&hold))
}

/// The `guardrail.request_resolved` event a waiter would have seen had it been listening when
/// the person answered. `None` while the request is still open, or when `id` is no request.
/// `bus.wait` asks this first: a person who approves within a second of the ask answers
/// before the agent's wait subscribes, and that wait would otherwise sleep to its timeout.
pub fn resolved_event(conn: &Connection, id: Id) -> Option<relay_bus::envelope::Event> {
    let hold = super::hold_by_id(conn, id).ok().filter(|hold| hold.op == OP)?;
    let answered = exception(&hold);
    let payload = match hold.state {
        HoldState::Open => return None,
        HoldState::Confirmed if answered.revoked_at.is_some() => {
            json!({"request_id": id, "session": hold.session, "state": "revoked"})
        }
        HoldState::Confirmed => {
            json!({"request_id": id, "session": hold.session, "state": "confirmed", "scope": answered.scope})
        }
        HoldState::Rejected => json!({
            "request_id": id, "session": hold.session, "state": "rejected", "reason": answered.denial_reason,
        }),
        HoldState::Expired => json!({"request_id": id, "session": hold.session, "state": "expired"}),
    };
    let ts = answered.revoked_at.or(hold.resolved_at).unwrap_or(hold.created_at);
    let mut event = relay_bus::envelope::Event::new(
        RESOLVED_EVENT, ts, hold.resolved_by.unwrap_or(Actor::System), payload,
    );
    event.project_id = hold.project_id;
    Some(event)
}

/// The suggestion an agent gets with a refusal it could ask past: which `kind` and `value` a
/// `guardrail.request` should carry. `None` for a policy no grant lifts.
pub fn suggestion(policy: &str, details: &Value) -> Option<(ExceptionKind, String)> {
    match policy {
        "denied_command" => Some((ExceptionKind::Command, details["command"].as_str()?.to_string())),
        "protected_path" | "write_root" | "destructive_write" => {
            Some((ExceptionKind::Path, details["path"].as_str()?.to_string()))
        }
        "cap" => Some((
            ExceptionKind::Cap,
            format!("files={} lines={}", details["files"].as_u64()?, details["lines"].as_u64()?),
        )),
        _ => None,
    }
}

/// The hint text for a refusal an agent may ask past.
pub fn hint(kind: ExceptionKind, value: &str) -> String {
    let kind = serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    format!(
        "If you cannot progress without this, ask the user for an exception: call guardrail.request \
         {{\"kind\":\"{kind}\",\"value\":{},\"reason\":\"<why you need it>\"}}, then bus.wait \
         {{\"events\":[\"{RESOLVED_EVENT}\"],\"matching\":{{\"request_id\":<request.id>}}}} and retry once it is approved. \
         Do not try to approve it yourself.",
        Value::String(value.to_string())
    )
}

/// Who answered, for the message the agent gets.
pub fn by_line(actor: &Actor) -> String {
    match actor {
        Actor::User => "the user".to_string(),
        other => other.to_string(),
    }
}

/// Store a new request as an open hold, with the notification that puts it in front of a
/// person. The frozen envelope is kept for inspection; confirming never replays it.
#[allow(clippy::too_many_arguments)] // one hold row and its notification, named field by field
pub fn insert(
    conn: &Connection,
    request: &relay_bus::envelope::Request,
    project_id: Id,
    session_id: Id,
    session: &str,
    details: &Value,
    kind: ExceptionKind,
    value: &str,
    reason: &str,
    now: &str,
) -> Result<Id, BusError> {
    let mut frozen = request.clone();
    frozen.token = None; // never persist session secrets
    conn.execute(
        "INSERT INTO holds(project_id, session_id, session, actor, op, envelope, policy, details, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', ?9)",
        params![
            project_id, session_id, session, request.actor.to_string(), OP,
            serde_json::to_string(&frozen).bus()?, POLICY, serde_json::to_string(details).bus()?, now,
        ],
    )
    .bus()?;
    let hold_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO notifications(project_id, category, title, body, link, read, created_at)
         VALUES (?1, 'guardrail', 'Agent requests a guardrail exception', ?2, ?3, 0, ?4)",
        params![
            project_id,
            format!("{session} asks to {}: {}", describe(kind, value), reason.trim()),
            json!({"op": "guardrail.confirm", "payload": {"hold_id": hold_id, "session": session}}).to_string(),
            now,
        ],
    )
    .bus()?;
    Ok(hold_id)
}

/// The open or still-active request from this session for exactly this, if there is one.
pub fn existing(conn: &Connection, session_id: Id, kind: ExceptionKind, value: &str) -> Result<Option<GuardrailException>, BusError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT * FROM holds WHERE session_id = ?1 AND op = 'guardrail.request'
               AND state IN ('open', 'confirmed') ORDER BY id DESC",
        )
        .bus()?;
    let holds = stmt
        .query_map([session_id], super::hold_row)
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .bus()?;
    Ok(holds
        .iter()
        .map(exception)
        .find(|request| {
            request.kind == kind
                && request.value == value.trim()
                && (request.state == HoldState::Open || request.active)
        }))
}
