//! `guardrail.*` — BUS.md §9, §10.11.
use crate::envelope::Response;
use crate::error::BusError;
use crate::registry::{Actors, OpMeta, Scope, Undo};
use crate::types::{
    ExceptionKind, GateKind, GrantScope, GuardrailConfig, GuardrailException, GuardrailLayer, Hold, Id, Verdict,
};
use crate::op;
use serde_json::Value;

payload!(#[schemars(rename = "GuardrailGateIn")] GateIn {
    pub session: String, pub kind: GateKind, pub path: Option<String>, pub new_text: Option<String>,
    pub diff: Option<String>, pub command: Option<String>,
});
result!(#[schemars(rename = "GuardrailGateOut")] GateOut { pub verdict: Verdict, pub error: Option<BusError>, pub hold_id: Option<Id> });
op!(Gate, "guardrail.gate", GateIn => GateOut,
    OpMeta::mutation(Scope::Session, 4, "The enforcement door: hooks call it before a write/commit/exec; may create a hold").actors(Actors::AgentOnly).emits(&["guardrail.held", "guardrail.refused", "guardrail.grant_used", "guardrail.resolved", "notify.new"]));

payload!(#[schemars(rename = "GuardrailHoldsListIn")] HoldsListIn {
    pub project_id: Option<Id>, pub session: Option<String>, pub open_only: Option<bool>,
    /// Newest first; default 200, at most 1000.
    pub limit: Option<u32>,
});
result!(#[schemars(rename = "GuardrailHoldsListOut")] HoldsListOut { pub holds: Vec<Hold> });
op!(HoldsList, "guardrail.holds.list", HoldsListIn => HoldsListOut, OpMeta::query(Scope::Global, 4, "Holds, open by default"));

payload!(#[schemars(rename = "GuardrailHoldGetIn")] HoldGetIn {
    pub hold_id: Id,
    /// Return every string whole, however large. By default a string over 64 KiB in the
    /// payload or details is cut to its first 4 KiB and named in `elided`.
    pub full: Option<bool>,
});
result!(#[schemars(rename = "GuardrailHoldGetOut")] HoldGetOut {
    pub hold: Hold,
    /// The exact frozen action, with authentication material removed. `guardrail.confirm`
    /// replays the stored action whole, whatever was elided here.
    pub request: crate::envelope::Request,
    /// JSON pointers (`/request/payload/new_text`, `/hold/details/reason`) of the strings cut short.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elided: Vec<String>,
});
op!(HoldGet, "guardrail.hold.get", HoldGetIn => HoldGetOut,
    OpMeta::query(Scope::Global, 4, "Inspect the frozen action before deciding a hold").actors(Actors::UserOnly));

payload!(#[schemars(rename = "GuardrailConfirmIn")] ConfirmIn {
    pub hold_id: Id,
    /// Exception requests only: how long the grant lasts. Defaults to what the agent asked for.
    pub scope: Option<GrantScope>,
});
result!(#[schemars(rename = "GuardrailConfirmOut")] ConfirmOut { pub hold: Hold, pub outcome: Response });
op!(Confirm, "guardrail.confirm", ConfirmIn => ConfirmOut,
    OpMeta::mutation(Scope::Global, 4, "Lift a hold: re-execute the held envelope as the confirmer (§9.4)").actors(Actors::UserOnly).emits(&["guardrail.resolved", "guardrail.request_resolved", "mailbox.new"]));
payload!(#[schemars(rename = "GuardrailRejectIn")] RejectIn { pub hold_id: Id, pub reason: Option<String> });
result!(#[schemars(rename = "GuardrailRejectOut")] RejectOut { pub hold: Hold });
op!(Reject, "guardrail.reject", RejectIn => RejectOut,
    OpMeta::mutation(Scope::Global, 4, "Close a hold without executing").actors(Actors::UserOnly).emits(&["guardrail.resolved", "guardrail.request_resolved", "mailbox.new"]));

payload!(#[schemars(rename = "GuardrailConfigGetIn")] ConfigGetIn {
    /// One of the two, or neither for the global layer.
    pub workspace_id: Option<Id>,
    pub project_id: Option<Id>,
});
op!(ConfigGet, "guardrail.config.get", ConfigGetIn => GuardrailConfig, OpMeta::query(Scope::Global, 4, "Effective guardrail config: defaults, then global, workspace and project overrides"));
payload!(#[schemars(rename = "GuardrailConfigSetIn")] ConfigSetIn {
    /// Patch this workspace's layer. At most one of `workspace_id` / `project_id`.
    pub workspace_id: Option<Id>,
    pub project_id: Option<Id>,
    /// Merged into the layer's stored overrides; `null` clears a key back to the inherited value.
    pub patch: Value,
});
op!(ConfigSet, "guardrail.config.set", ConfigSetIn => GuardrailConfig,
    OpMeta::mutation(Scope::Global, 4, "Patch one layer of guardrail config (caps, protected paths, thresholds, roles)").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["guardrail.config_changed"]));

payload!(#[schemars(rename = "GuardrailConfigLayersIn")] ConfigLayersIn {
    /// At most one of the two; neither reads the global layer.
    pub workspace_id: Option<Id>,
    pub project_id: Option<Id>,
});
result!(#[schemars(rename = "GuardrailConfigLayersOut")] ConfigLayersOut {
    /// The layer asked about.
    pub scope: GuardrailLayer,
    pub workspace_id: Option<Id>,
    pub project_id: Option<Id>,
    /// What applies at this layer.
    pub effective: GuardrailConfig,
    /// What this layer would get with no overrides of its own: the parent layer's effective config.
    pub inherited: GuardrailConfig,
    /// This layer's stored overrides, exactly as stored.
    pub overrides: Value,
    /// Leaf path (`caps.files`, `protected_paths`) to the layer that decided its value.
    pub sources: std::collections::BTreeMap<String, GuardrailLayer>,
});
op!(ConfigLayers, "guardrail.config.layers", ConfigLayersIn => ConfigLayersOut,
    OpMeta::query(Scope::Global, 4, "One guardrail config layer: effective, inherited, its own overrides, and which layer set each value"));

payload!(#[schemars(rename = "GuardrailRequestIn")] ExceptionRequestIn {
    pub session: String,
    /// `command` for a denied command, `path` for a protected path / write root / large rewrite,
    /// `cap` for the commit caps.
    pub kind: ExceptionKind,
    /// The exact command line; the path (worktree-relative or absolute); or `files=N lines=M`.
    pub value: String,
    /// Why you cannot progress without it. The user reads this before deciding.
    pub reason: String,
    /// What you are asking for; the user may grant less. Default `once`.
    pub scope: Option<GrantScope>,
});
result!(#[schemars(rename = "GuardrailRequestOut")] ExceptionRequestOut {
    pub request: GuardrailException,
    /// False when an identical request from this session was already open.
    pub created: bool,
    /// The event announcing the answer; wait with `bus.wait {events:[wait_for], matching:{request_id}}`.
    pub wait_for: String,
});
op!(ExceptionRequest, "guardrail.request", ExceptionRequestIn => ExceptionRequestOut,
    OpMeta::mutation(Scope::Session, 4, "Ask the user for a guardrail exception when you cannot progress without one; every agent may call it").actors(Actors::AgentOnly).emits(&["guardrail.requested", "guardrail.held", "notify.new"]));

payload!(#[schemars(rename = "GuardrailRequestGetIn")] ExceptionGetIn { pub request_id: Id });
op!(ExceptionGet, "guardrail.request.get", ExceptionGetIn => GuardrailException,
    OpMeta::query(Scope::Global, 4, "One exception request and its grant: open, approved (and whether still active), denied"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "GuardrailRequestsFilter")]
pub enum ExceptionFilter {
    /// Waiting for an answer.
    Open,
    /// Approved and still usable.
    Active,
    All,
}
payload!(#[schemars(rename = "GuardrailRequestsListIn")] ExceptionsListIn {
    pub project_id: Option<Id>,
    pub session: Option<String>,
    /// Default `all`.
    pub state: Option<ExceptionFilter>,
});
result!(#[schemars(rename = "GuardrailRequestsListOut")] ExceptionsListOut { pub requests: Vec<GuardrailException> });
op!(ExceptionsList, "guardrail.requests.list", ExceptionsListIn => ExceptionsListOut,
    OpMeta::query(Scope::Global, 4, "Exception requests, newest first"));

payload!(#[schemars(rename = "GuardrailGrantRevokeIn")] GrantRevokeIn { pub request_id: Id });
op!(GrantRevoke, "guardrail.grant.revoke", GrantRevokeIn => GuardrailException,
    OpMeta::mutation(Scope::Global, 4, "End an approved exception before it would expire").actors(Actors::UserOnly).emits(&["guardrail.resolved", "guardrail.request_resolved", "mailbox.new"]));

payload!(#[schemars(rename = "GuardrailCheckIn")] CheckIn {
    pub project_id: Id, pub kind: GateKind, pub path: Option<String>, pub new_text: Option<String>,
    pub diff: Option<String>, pub command: Option<String>,
});
result!(#[schemars(rename = "GuardrailCheckOut")] CheckOut { pub verdict: Verdict, pub error: Option<BusError> });
op!(Check, "guardrail.check", CheckIn => CheckOut, OpMeta::query(Scope::Project, 4, "Pure dry run of gate: nothing created, nothing audited"));

payload!(#[schemars(rename = "GuardrailExplainIn")] ExplainIn {
    pub project_id: Id,
    /// Worktree-relative paths the plan intends to touch.
    pub paths: Option<Vec<String>>,
    /// Lines the plan expects to change in total, if known.
    pub lines: Option<u32>,
    /// Commands the plan intends to run.
    pub commands: Option<Vec<String>>,
});
result!(#[schemars(rename = "GuardrailExplainItem")] ExplainItem {
    pub subject: String,
    pub verdict: Verdict,
    /// The policy that decided it, when one did.
    pub policy: Option<String>,
    pub message: Option<String>,
});
result!(#[schemars(rename = "GuardrailExplainOut")] ExplainOut {
    /// The worst verdict across everything below.
    pub verdict: Verdict,
    pub paths: Vec<ExplainItem>,
    pub commands: Vec<ExplainItem>,
    /// What the plan costs against the commit caps.
    pub files: u32,
    pub lines: u32,
    pub caps: crate::types::GuardrailCaps,
    pub over_caps: bool,
    /// Absolute roots the caller may write to.
    pub write_roots: Vec<String>,
});
op!(Explain, "guardrail.explain", ExplainIn => ExplainOut,
    OpMeta::query(Scope::Project, 4, "Preflight a whole plan: which paths, commands and totals would be allowed, held or refused"));

entries!(
    Gate, HoldsList, HoldGet, Confirm, Reject, ConfigGet, ConfigSet, ConfigLayers, Check, Explain,
    ExceptionRequest, ExceptionGet, ExceptionsList, GrantRevoke,
);
