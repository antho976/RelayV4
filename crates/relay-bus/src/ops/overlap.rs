//! `overlap.*` — BUS.md §10.10.
use crate::registry::{OpMeta, Scope};
use crate::types::{Id, Overlap};
use crate::op;

payload!(#[schemars(rename = "OverlapProjectIn")] ProjectIn { pub project_id: Id });
result!(#[schemars(rename = "OverlapListOut")] ListOut { pub overlaps: Vec<Overlap> });
op!(List, "overlap.list", ProjectIn => ListOut, OpMeta::query(Scope::Project, 5, "Session pairs sharing changed files / symbols"));
payload!(#[schemars(rename = "OverlapFlagIn")] FlagIn { pub project_id: Id, pub path: String, pub symbol: Option<String>, pub note: Option<String> });
op!(Flag, "overlap.flag", FlagIn => Overlap,
    OpMeta::mutation(Scope::Project, 5, "An agent claiming 'I am touching this'").emits(&["overlap.changed"]));
payload!(#[schemars(rename = "OverlapAckIn")] AckIn { pub overlap_id: Id });
op!(Ack, "overlap.ack", AckIn => Overlap,
    OpMeta::mutation(Scope::Project, 5, "Acknowledge an overlap").emits(&["overlap.changed"]));
op!(Scan, "overlap.scan", ProjectIn => ListOut,
    OpMeta::mutation(Scope::Project, 5, "Force a scan now (tests / reconcile)").emits(&["overlap.changed"]));

entries!(List, Flag, Ack, Scan);
