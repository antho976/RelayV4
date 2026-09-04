//! `guardrail.*` — BUS.md §9, §10.11.
use crate::envelope::Response;
use crate::error::BusError;
use crate::registry::{Actors, OpMeta, Scope, Undo};
use crate::types::{GateKind, GuardrailConfig, Hold, Id, Verdict};
use crate::op;
use serde_json::Value;

payload!(#[schemars(rename = "GuardrailGateIn")] GateIn {
    pub session: String, pub kind: GateKind, pub path: Option<String>, pub new_text: Option<String>,
    pub diff: Option<String>, pub command: Option<String>,
});
result!(#[schemars(rename = "GuardrailGateOut")] GateOut { pub verdict: Verdict, pub error: Option<BusError>, pub hold_id: Option<Id> });
op!(Gate, "guardrail.gate", GateIn => GateOut,
    OpMeta::mutation(Scope::Session, 4, "The enforcement door: hooks call it before a write/commit/exec; may create a hold").actors(Actors::AgentOnly).emits(&["guardrail.held", "notify.new"]));

payload!(#[schemars(rename = "GuardrailHoldsListIn")] HoldsListIn { pub project_id: Option<Id>, pub session: Option<String>, pub open_only: Option<bool> });
result!(#[schemars(rename = "GuardrailHoldsListOut")] HoldsListOut { pub holds: Vec<Hold> });
op!(HoldsList, "guardrail.holds.list", HoldsListIn => HoldsListOut, OpMeta::query(Scope::Global, 4, "Holds, open by default"));

payload!(#[schemars(rename = "GuardrailHoldGetIn")] HoldGetIn { pub hold_id: Id });
result!(#[schemars(rename = "GuardrailHoldGetOut")] HoldGetOut {
    pub hold: Hold,
    /// The exact frozen action, with authentication material removed.
    pub request: crate::envelope::Request,
});
op!(HoldGet, "guardrail.hold.get", HoldGetIn => HoldGetOut,
    OpMeta::query(Scope::Global, 4, "Inspect the frozen action before deciding a hold").actors(Actors::UserOnly));

payload!(#[schemars(rename = "GuardrailConfirmIn")] ConfirmIn { pub hold_id: Id });
result!(#[schemars(rename = "GuardrailConfirmOut")] ConfirmOut { pub hold: Hold, pub outcome: Response });
op!(Confirm, "guardrail.confirm", ConfirmIn => ConfirmOut,
    OpMeta::mutation(Scope::Global, 4, "Lift a hold: re-execute the held envelope as the confirmer (§9.4)").actors(Actors::UserOnly).emits(&["guardrail.resolved", "mailbox.new"]));
payload!(#[schemars(rename = "GuardrailRejectIn")] RejectIn { pub hold_id: Id, pub reason: Option<String> });
result!(#[schemars(rename = "GuardrailRejectOut")] RejectOut { pub hold: Hold });
op!(Reject, "guardrail.reject", RejectIn => RejectOut,
    OpMeta::mutation(Scope::Global, 4, "Close a hold without executing").actors(Actors::UserOnly).emits(&["guardrail.resolved", "mailbox.new"]));

payload!(#[schemars(rename = "GuardrailConfigGetIn")] ConfigGetIn { pub project_id: Option<Id> });
op!(ConfigGet, "guardrail.config.get", ConfigGetIn => GuardrailConfig, OpMeta::query(Scope::Global, 4, "Guardrail config: global merged with project overrides"));
payload!(#[schemars(rename = "GuardrailConfigSetIn")] ConfigSetIn { pub project_id: Option<Id>, pub patch: Value });
op!(ConfigSet, "guardrail.config.set", ConfigSetIn => GuardrailConfig,
    OpMeta::mutation(Scope::Global, 4, "Patch guardrail config (caps, protected paths, thresholds, roles)").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["guardrail.config_changed"]));

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

entries!(Gate, HoldsList, HoldGet, Confirm, Reject, ConfigGet, ConfigSet, Check, Explain);
