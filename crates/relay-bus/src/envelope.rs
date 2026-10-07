//! Request / Response / Event envelopes (BUS.md §1).

use crate::error::BusError;
use crate::types::{Id, Ts};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use uuid::Uuid;

/// Envelope schema major. Bumped only for breaking changes (BUS.md §12).
pub const ENVELOPE_V: u32 = 1;

/// Who is asking (BUS.md §4). `System` is internal-only: doors reject it (`bus.actor`);
/// the engine uses it for reconcile passes and post-commit continuations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Actor {
    User,
    Agent(String),
    Test,
    System,
}

impl Actor {
    pub fn agent(name: impl Into<String>) -> Self {
        Actor::Agent(name.into())
    }
    pub fn is_agent(&self) -> bool {
        matches!(self, Actor::Agent(_))
    }
    pub fn session_name(&self) -> Option<&str> {
        match self {
            Actor::Agent(n) => Some(n),
            _ => None,
        }
    }
    /// `user` and `test` bypass allowlists (BUS.md §9.1); `system` too.
    pub fn is_privileged(&self) -> bool {
        !self.is_agent()
    }
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "user" => Ok(Actor::User),
            "test" => Ok(Actor::Test),
            "system" => Ok(Actor::System),
            _ => match s.strip_prefix("agent:") {
                // The same set the schema's `pattern` publishes: a session name, never a path or
                // anything with spaces, so what the type accepts is what the contract says.
                Some(name)
                    if !name.is_empty()
                        && name.len() <= 64
                        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
                {
                    Ok(Actor::Agent(name.to_string()))
                }
                _ => Err(format!(
                    "invalid actor {s:?}: expected user | agent:<name> | test"
                )),
            },
        }
    }
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Actor::User => f.write_str("user"),
            Actor::Test => f.write_str("test"),
            Actor::System => f.write_str("system"),
            Actor::Agent(n) => write!(f, "agent:{n}"),
        }
    }
}

impl Serialize for Actor {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Actor {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Actor::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Actor {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Actor".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "\"user\" | \"agent:<session-name>\" | \"test\" | \"system\". \"system\" is the engine itself: it appears on events, audit rows and holds, and every door rejects a request that claims it",
            "pattern": "^(user|test|system|agent:[A-Za-z0-9_-]{1,64})$"
        })
    }
}

/// A request envelope (BUS.md §1.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Envelope schema major; must equal [`ENVELOPE_V`].
    pub v: u32,
    /// UUID v4 minted by the caller; the idempotency key. An audited request's id stays
    /// deduplicated for as long as its audit row is kept (`audit.retention_days`, BUS.md §5.3).
    pub id: Uuid,
    pub actor: Actor,
    /// `noun.verb` or `noun.sub.verb`.
    pub op: String,
    /// Op-specific; `{}` when the op takes nothing.
    #[serde(default = "empty_object")]
    pub payload: Value,
    /// Session token binding an `agent:<name>` actor on the socket door.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

impl Request {
    pub fn new(actor: Actor, op: impl Into<String>, payload: Value) -> Self {
        Request {
            v: ENVELOPE_V,
            id: Uuid::new_v4(),
            actor,
            op: op.into(),
            payload,
            token: None,
        }
    }
    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }
}

// Server-to-client shapes (this, `Response`, `Event`, `Frame`, `BusError`, `Confirm`) accept
// unknown fields: an additive change does not bump `v` (BUS.md §12), and a strict reader would
// drop the connection on the first line from a newer engine. Requests and payloads stay strict.

/// The `mail` sideband on a response (BUS.md §1.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MailHint {
    /// Number of unread priority messages for the authenticated agent session.
    pub priority: u32,
}

/// A response envelope (BUS.md §1.2). Exactly one of `result` / `error` is present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    pub v: u32,
    /// `null` only when the request could not be parsed far enough to read one (`bus.parse`).
    pub id: Option<Uuid>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BusError>,
    /// Present (true) when the id was seen before and the recorded result was returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed: Option<bool>,
    /// Live, unaudited sideband. Present only when an authenticated agent has priority mail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<MailHint>,
}

impl Response {
    pub fn ok(id: Uuid, result: Value) -> Self {
        Response {
            v: ENVELOPE_V,
            id: Some(id),
            ok: true,
            result: Some(result),
            error: None,
            replayed: None,
            mail: None,
        }
    }
    pub fn err(id: Uuid, error: BusError) -> Self {
        Response {
            v: ENVELOPE_V,
            id: Some(id),
            ok: false,
            result: None,
            error: Some(error),
            replayed: None,
            mail: None,
        }
    }
    /// A response to something that never became a request (`invalid` / `bus.parse`).
    pub fn unparsed(error: BusError) -> Self {
        Response {
            v: ENVELOPE_V,
            id: None,
            ok: false,
            result: None,
            error: Some(error),
            replayed: None,
            mail: None,
        }
    }
    pub fn replayed(mut self) -> Self {
        self.replayed = Some(true);
        self
    }
    pub fn into_result(self) -> Result<Value, BusError> {
        if self.ok {
            Ok(self.result.unwrap_or(Value::Null))
        } else {
            Err(self
                .error
                .unwrap_or_else(|| BusError::internal("response had ok=false and no error")))
        }
    }
}

/// A fact about a mutation that already happened (BUS.md §1.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub v: u32,
    /// `noun.changed`, `noun.deleted`, or a domain event.
    pub ev: String,
    /// RFC 3339 UTC.
    pub ts: Ts,
    pub actor: Actor,
    /// Request id that caused it; absent for system-originated events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    pub payload: Value,
}

impl Event {
    pub fn new(ev: impl Into<String>, ts: Ts, actor: Actor, payload: Value) -> Self {
        Event {
            v: ENVELOPE_V,
            ev: ev.into(),
            ts,
            actor,
            cause: None,
            project_id: None,
            payload,
        }
    }
    pub fn caused_by(mut self, id: Uuid) -> Self {
        self.cause = Some(id);
        self
    }
}

/// A data-plane frame (BUS.md §7) as carried on the socket door.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Frame {
    pub v: u32,
    /// `pty` | `logcat` | `mirror`, all on the socket door (BUS.md §7). `app.log.tail` declares
    /// a `log` stream but is not built: it sends no frames.
    pub stream: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<Id>,
    /// `mirror` only: which `device.mirror.start` this frame belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_id: Option<Id>,
    /// `pty` only: increments on every spawn/wake; `seq` restarts at 0 within it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    pub seq: u64,
    /// base64 for `pty`; a line for `logcat`; for `mirror`, a base64 video packet or a status
    /// object (state, picture size, device, and the typed reason on the last one).
    pub data: Value,
}

#[cfg(test)]
mod wire_prefix_tests {
    use super::*;

    /// Readers tell the three line shapes apart from the key that follows `v`, which lets a
    /// socket line be parsed once instead of being staged through a `serde_json::Value` first.
    /// That only holds while each envelope declares that key second, so it is asserted here
    /// rather than assumed at the reader. A reader that stops recognizing a prefix falls back
    /// to the general path, so this failing means "the fast path went quiet", not "it broke".
    #[test]
    fn each_envelope_leads_with_the_key_that_identifies_it() {
        let event = Event::new("session.changed", "2026-01-01T00:00:00Z".into(), Actor::User, serde_json::json!({}));
        assert!(serde_json::to_string(&event).unwrap().starts_with(r#"{"v":1,"ev""#));

        let frame = Frame {
            v: ENVELOPE_V,
            stream: "pty".into(),
            session: Some("brisk-otter".into()),
            run_id: None,
            mirror_id: None,
            epoch: Some(1),
            seq: 2,
            data: serde_json::Value::String("aGk=".into()),
        };
        assert!(serde_json::to_string(&frame).unwrap().starts_with(r#"{"v":1,"stream""#));

        // Both the answered and the unparseable response, whose id is null rather than absent.
        let answered = Response::ok(Uuid::nil(), serde_json::json!({}));
        assert!(serde_json::to_string(&answered).unwrap().starts_with(r#"{"v":1,"id""#));
        let unparsed = Response::unparsed(BusError::internal("x"));
        assert!(serde_json::to_string(&unparsed).unwrap().starts_with(r#"{"v":1,"id""#));
    }
}
