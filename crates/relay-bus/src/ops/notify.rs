//! `notify.*` / `settings.*` / `dashboard.*` — BUS.md §10.16.
use crate::registry::{Actors, Audit, OpMeta, Scope, Undo};
use crate::types::{Hold, Id, Notification, NotifyCategory, Peer, Task};
use crate::{op, Empty};
use serde_json::Value;

payload!(#[schemars(rename = "NotifyListIn")] ListIn { pub project_id: Option<Id>, pub unread_only: Option<bool>, pub category: Option<NotifyCategory>, pub limit: Option<u32> });
result!(#[schemars(rename = "NotifyListOut")] ListOut { pub notifications: Vec<Notification> });
op!(List, "notify.list", ListIn => ListOut, OpMeta::query(Scope::Global, 9, "Notifications"));
payload!(#[schemars(rename = "NotifyAckIn")] AckIn { pub notification_id: Id });
op!(Ack, "notify.ack", AckIn => Empty, OpMeta::mutation(Scope::Global, 9, "Mark read").audit(Audit::Never).emits(&["notify.changed"]));
payload!(#[schemars(rename = "NotifyAckAllIn")] AckAllIn { pub category: Option<NotifyCategory> });
op!(AckAll, "notify.ack_all", AckAllIn => Empty, OpMeta::mutation(Scope::Global, 9, "Mark all read").audit(Audit::Never).emits(&["notify.changed"]));
op!(SettingsGetNotify, "notify.settings.get", Empty => Value, OpMeta::query(Scope::Global, 9, "Per-category toggles and sounds"));
payload!(#[schemars(rename = "NotifyPatchIn")] PatchIn { pub patch: Value });
op!(SettingsSetNotify, "notify.settings.set", PatchIn => Value,
    OpMeta::mutation(Scope::Global, 9, "Patch notification settings").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["settings.changed"]));

payload!(#[schemars(rename = "SettingsGetIn")] SettingsGetIn { pub path: Option<String> });
result!(#[schemars(rename = "NotifyValueOut")] ValueOut { pub value: Value });
op!(SettingsGet, "settings.get", SettingsGetIn => ValueOut, OpMeta::query(Scope::Global, 1, "Settings subtree by dotted path (whole tree if absent)"));
payload!(#[schemars(rename = "SettingsSetIn")] SettingsSetIn { pub path: String, pub value: Value });
op!(SettingsSet, "settings.set", SettingsSetIn => ValueOut,
    OpMeta::mutation(Scope::Global, 1, "Set a settings value").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["settings.changed"]));
payload!(#[schemars(rename = "SettingsResetIn")] SettingsResetIn { pub path: Option<String> });
op!(SettingsReset, "settings.reset", SettingsResetIn => ValueOut,
    OpMeta::mutation(Scope::Global, 1, "Reset a subtree to defaults").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["settings.changed"]));

result!(#[schemars(rename = "DashboardProject")] DashboardProject {
    pub project_id: Id, pub name: String, pub base_branch: String,
    pub tasks_open: i64, pub ready: i64, pub active: i64, pub in_review: i64,
    pub done_recent: i64, pub live_sessions: i64, pub blocked_sessions: i64,
});
result!(#[schemars(rename = "DashboardOut")] DashboardOut {
    pub projects: Vec<DashboardProject>,
    pub sessions_live: Vec<Peer>, pub in_review: Vec<Task>, pub holds_open: Vec<Hold>,
    pub notifications: Vec<Notification>, pub resources: crate::ops::app::ResourcesOut,
});
op!(DashboardGet, "dashboard.get", Empty => DashboardOut, OpMeta::query(Scope::Global, 9, "Cross-project overview in one round trip"));

entries!(List, Ack, AckAll, SettingsGetNotify, SettingsSetNotify, SettingsGet, SettingsSet, SettingsReset, DashboardGet);
