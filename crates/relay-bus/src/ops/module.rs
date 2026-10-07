//! `module.*` — BUS.md §10.6.
use crate::registry::{OpMeta, Scope, Undo};
use crate::types::{Column, Id, Module, ModuleHeader, ModuleSummary, Priority, Task};
use crate::{op, Empty};
use std::collections::BTreeMap;

payload!(#[schemars(rename = "ModuleCreateIn")] CreateIn { pub project_id: Id, pub name: String, pub icon: Option<String>, pub priority: Option<Priority> });
op!(Create, "module.create", CreateIn => Module,
    OpMeta::mutation(Scope::Project, 7, "Create a module").undo(Undo::Inverse).emits(&["module.changed"]));

payload!(#[schemars(rename = "ModuleIdIn")] IdIn { pub module_id: Id });
result!(#[schemars(rename = "ModuleGetOut")] GetOut { #[serde(flatten)] pub module: Module, pub tasks_by_state: BTreeMap<Column, Vec<Task>> });
op!(Get, "module.get", IdIn => GetOut, OpMeta::query(Scope::Project, 7, "Module detail with its tasks grouped by column"));

payload!(#[schemars(rename = "ModuleListIn")] ListIn { pub project_id: Id, pub include_archived: Option<bool> });
result!(#[schemars(rename = "ModuleListOut")] ListOut { pub modules: Vec<ModuleSummary>, pub header: ModuleHeader });
op!(List, "module.list", ListIn => ListOut, OpMeta::query(Scope::Project, 7, "Modules index: list + header stats"));

payload!(#[schemars(rename = "ModuleUpdateIn")] UpdateIn {
    pub module_id: Id, pub name: Option<String>,
    #[serde(default, deserialize_with = "crate::nullable")] pub icon: Option<Option<String>>,
    pub priority: Option<Priority>, pub order: Option<i64>,
    /// Original editable field values; every supplied value must still match atomically.
    pub expected: Option<serde_json::Map<String, serde_json::Value>>,
});
op!(Update, "module.update", UpdateIn => Module,
    OpMeta::mutation(Scope::Project, 7, "Patch a module").undo(Undo::Inverse).emits(&["module.changed"]));
op!(Complete, "module.complete", IdIn => Module,
    OpMeta::mutation(Scope::Project, 7, "Archive a module and stamp the date").undo(Undo::Inverse).emits(&["module.changed"]));
op!(Reopen, "module.reopen", IdIn => Module,
    OpMeta::mutation(Scope::Project, 7, "Un-archive a module").undo(Undo::Inverse).emits(&["module.changed"]));
op!(Delete, "module.delete", IdIn => Empty,
    OpMeta::mutation(Scope::Project, 7, "Soft-delete a module; its tasks keep existing unlinked").undo(Undo::Inverse).emits(&["module.deleted", "task.changed"]));
op!(Restore, "module.restore", IdIn => Module,
    OpMeta::mutation(Scope::Project, 7, "Restore a soft-deleted module").undo(Undo::Inverse).emits(&["module.changed", "task.changed"]));

payload!(#[schemars(rename = "ModuleStatsIn")] StatsIn { pub project_id: Id });
op!(Stats, "module.stats", StatsIn => ModuleHeader, OpMeta::query(Scope::Project, 7, "Header stats only"));

payload!(#[schemars(rename = "ModuleChangelogDraftIn")] ChangelogDraftIn { pub module_id: Id, pub group_by: Option<String> });
result!(#[schemars(rename = "ModuleChangelogDraftOut")] ChangelogDraftOut { pub markdown: String, pub tasks: Vec<Id> });
op!(ChangelogDraft, "module.changelog.draft", ChangelogDraftIn => ChangelogDraftOut, OpMeta::query(Scope::Project, 7, "Draft patch notes from done tasks' changelog fields"));

entries!(
    Create,
    Get,
    List,
    Update,
    Complete,
    Reopen,
    Delete,
    Restore,
    Stats,
    ChangelogDraft
);
