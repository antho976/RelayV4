//! `audit.*` — BUS.md §10.3.
use crate::envelope::Actor;
use crate::registry::{Actors, OpMeta, Scope};
use crate::types::{AuditRow, Id, Ts};
use crate::op;

payload!(#[schemars(rename = "AuditListIn")] ListIn {
    pub project_id: Option<Id>, pub actor: Option<Actor>, pub session_id: Option<Id>,
    pub op_prefix: Option<String>, pub parent_req: Option<String>,
    pub since: Option<Ts>, pub until: Option<Ts>, pub limit: Option<u32>,
});
result!(#[schemars(rename = "AuditListOut")] ListOut { pub rows: Vec<AuditRow> });
op!(List, "audit.list", ListIn => ListOut, OpMeta::query(Scope::Global, 1, "Audit rows, filtered; limit ≤ 1000"));

payload!(#[schemars(rename = "AuditGetIn")] GetIn { pub audit_id: Id });
op!(Get, "audit.get", GetIn => AuditRow, OpMeta::query(Scope::Global, 1, "One audit row with its stored payload"));

payload!(#[schemars(rename = "AuditUndoIn")] UndoIn { pub audit_id: Id, pub force: Option<bool> });
result!(#[schemars(rename = "AuditUndoOut")] UndoOut { pub undone: Id, pub by: Id });
op!(Undo, "audit.undo", UndoIn => UndoOut,
    OpMeta::mutation(Scope::Global, 7, "Replay the inverse of an audited op; conflict audit.stale if the entity changed since").actors(Actors::UserOnly));

entries!(List, Get, Undo);
