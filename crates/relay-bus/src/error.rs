//! Typed errors (BUS.md §1.2). Agents react to `kind` and `code`, never to `message`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Invalid,
    NotFound,
    Conflict,
    Refused,
    Held,
    Unavailable,
    Internal,
}

impl ErrorKind {
    /// CLI exit code (BUS.md §8.2).
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::Invalid | ErrorKind::NotFound | ErrorKind::Conflict | ErrorKind::Internal => 1,
            ErrorKind::Refused => 2,
            ErrorKind::Held => 3,
            ErrorKind::Unavailable => 4,
        }
    }
    /// The audit `kind` column for a request that ended with this error.
    pub fn audit_kind(self) -> &'static str {
        match self {
            ErrorKind::Held => "held",
            ErrorKind::Refused => "refused",
            _ => "error",
        }
    }
}

/// The op that lifts a hold (present iff `kind == held`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Confirm {
    pub op: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("{kind:?} {code}: {message}")]
pub struct BusError {
    pub kind: ErrorKind,
    /// Stable, dotted, e.g. `task.not_found`, `guardrail.destructive_write`.
    pub code: String,
    /// Human sentence; may change; never parse it.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
}

impl BusError {
    pub fn new(kind: ErrorKind, code: impl Into<String>, message: impl Into<String>) -> Self {
        BusError { kind, code: code.into(), message: message.into(), details: None, hint: None, confirm: None }
    }
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn with_confirm(mut self, op: impl Into<String>, payload: Value) -> Self {
        self.confirm = Some(Confirm { op: op.into(), payload });
        self
    }

    // ---- constructors for the codes every door and handler shares ----

    pub fn invalid(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, code, message)
    }
    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, code, message)
    }
    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Conflict, code, message)
    }
    pub fn refused(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Refused, code, message)
    }
    pub fn held(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Held, code, message)
    }
    pub fn unavailable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unavailable, code, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, "internal", message)
    }

    /// `invalid` / `bus.unknown_op`.
    pub fn unknown_op(op: &str) -> Self {
        Self::invalid("bus.unknown_op", format!("unknown op {op:?}"))
            .with_hint("call bus.ops to list what exists")
    }
    /// `invalid` / `bus.schema` — payload failed validation.
    pub fn schema(op: &str, why: impl std::fmt::Display) -> Self {
        Self::invalid("bus.schema", format!("payload for {op} does not match its schema: {why}"))
            .with_hint(format!("relay schema {op}"))
    }
    /// `invalid` / `bus.actor`.
    pub fn actor(why: impl Into<String>) -> Self {
        Self::invalid("bus.actor", why)
    }
    /// `invalid` / `bus.parse` — not even JSON, or JSON with no readable `id`.
    pub fn parse(why: impl std::fmt::Display) -> Self {
        Self::invalid("bus.parse", format!("could not parse request: {why}"))
    }
    /// `invalid` / `bus.envelope`.
    pub fn envelope(why: impl std::fmt::Display) -> Self {
        Self::invalid("bus.envelope", format!("malformed envelope: {why}"))
    }
    /// `unavailable` / `bus.not_implemented` — registered, not yet built (BUS.md §15).
    pub fn not_implemented(op: &str, phase: u8) -> Self {
        Self::unavailable("bus.not_implemented", format!("{op} is registered but not implemented yet"))
            .with_details(serde_json::json!({ "phase": phase }))
    }
    /// `refused` / `actor.allowlist`.
    pub fn allowlist(op: &str, role: &str) -> Self {
        Self::refused("actor.allowlist", format!("role {role:?} may not call {op}"))
            .with_details(serde_json::json!({ "op": op, "role": role }))
    }
    /// `refused` / `actor.scope` — an agent reaching for something that isn't its own.
    pub fn not_own(what: &str) -> Self {
        Self::refused("actor.scope", format!("agents may only act on their own {what}"))
    }
}
