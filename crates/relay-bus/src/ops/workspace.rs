//! `workspace.*` / `project.*` — BUS.md §10.4. All mutations user-only.
use crate::registry::{Actors, OpMeta, Scope, Undo};
use crate::types::{Column, Id, LocalRepo, Project, Workspace};
use crate::{op, Empty};
use std::collections::BTreeMap;

payload!(#[schemars(rename = "WorkspaceCreateIn")] WsCreateIn { pub path: Option<String>, pub name: Option<String> });
op!(WsCreate, "workspace.create", WsCreateIn => Workspace,
    OpMeta::mutation(Scope::Global, 1, "Register a directory as a workspace").actors(Actors::UserOnly).emits(&["workspace.changed"]));
payload!(#[schemars(rename = "WorkspaceDiscoverIn")] WsDiscoverIn { pub path: Option<String> });
result!(#[schemars(rename = "WorkspaceDiscoverOut")] WsDiscoverOut { pub path: String, pub repositories: Vec<LocalRepo> });
op!(WsDiscover, "workspace.discover", WsDiscoverIn => WsDiscoverOut,
    OpMeta::query(Scope::Global, 12, "Resolve a workspace path and discover Git repositories inside it"));
result!(#[schemars(rename = "WorkspaceListOut")] WsListOut { pub workspaces: Vec<Workspace> });
op!(WsList, "workspace.list", Empty => WsListOut, OpMeta::query(Scope::Global, 1, "All workspaces"));
payload!(#[schemars(rename = "WorkspaceUpdateIn")] WsUpdateIn { pub workspace_id: Id, pub name: Option<String>, pub order: Option<i64> });
op!(WsUpdate, "workspace.update", WsUpdateIn => Workspace,
    OpMeta::mutation(Scope::Global, 1, "Rename / reorder a workspace").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["workspace.changed"]));
payload!(#[schemars(rename = "WorkspaceRemoveIn")] WsRemoveIn {
    pub workspace_id: Id,
    /// Also remove every project in it, closing their open sessions (`project.remove` with `force`).
    /// Without it the op refuses `workspace.has_projects`.
    pub force: Option<bool>,
    /// With `force`: also delete the Relay-pool worktrees of the sessions it closes. Branches
    /// are always kept; off by default, so every checkout stays on disk.
    pub remove_worktrees: Option<bool>,
});
result!(#[schemars(rename = "WorkspaceRemoveOut")] WsRemoveOut { pub projects_removed: i64, pub sessions_closed: i64 });
op!(WsRemove, "workspace.remove", WsRemoveIn => WsRemoveOut,
    OpMeta::mutation(Scope::Global, 1, "Forget a workspace; conflict if it still has projects unless force removes them too").actors(Actors::UserOnly).emits(&["workspace.deleted", "project.deleted", "session.changed"]));

payload!(#[schemars(rename = "ProjectAddIn")] ProjectAddIn { pub workspace_id: Id, pub path: String, pub name: Option<String> });
op!(ProjectAdd, "project.add", ProjectAddIn => Project,
    OpMeta::mutation(Scope::Global, 1, "Register a git repo inside a workspace as a project").actors(Actors::UserOnly).emits(&["project.changed"]));
payload!(#[schemars(rename = "ProjectCloneIn")] ProjectCloneIn { pub workspace_id: Id, pub url: String, pub dest: Option<String> });
result!(#[schemars(rename = "ProjectCloneOut")] ProjectCloneOut { pub project: Project });
op!(ProjectClone, "project.clone", ProjectCloneIn => ProjectCloneOut,
    OpMeta::mutation(Scope::Global, 8, "Clone a repo into the workspace and register it (async; project.changed when done)").actors(Actors::UserOnly).emits(&["project.changed"]));
payload!(#[schemars(rename = "ProjectListIn")] ProjectListIn { pub workspace_id: Option<Id> });
result!(#[schemars(rename = "ProjectListOut")] ProjectListOut { pub projects: Vec<Project> });
op!(ProjectList, "project.list", ProjectListIn => ProjectListOut, OpMeta::query(Scope::Global, 1, "Projects, optionally within one workspace"));
payload!(#[schemars(rename = "ProjectGetIn")] ProjectGetIn { pub project_id: Id });
op!(ProjectGet, "project.get", ProjectGetIn => Project, OpMeta::query(Scope::Project, 1, "One project"));
payload!(#[schemars(rename = "ProjectUpdateIn")] ProjectUpdateIn {
    pub project_id: Id, pub name: Option<String>, #[serde(default, deserialize_with = "crate::nullable")] pub build_cmd: Option<Option<String>>, #[serde(default, deserialize_with = "crate::nullable")] pub run_cmd: Option<Option<String>>,
    pub base_branch: Option<String>, pub protected_paths: Option<Vec<String>>, pub critical_files: Option<Vec<String>>,
    pub order: Option<i64>, pub pinned: Option<bool>,
});
op!(ProjectUpdate, "project.update", ProjectUpdateIn => Project,
    OpMeta::mutation(Scope::Project, 1, "Patch project settings").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["project.changed"]));
payload!(#[schemars(rename = "ProjectRemoveIn")] ProjectRemoveIn {
    pub project_id: Id,
    /// Close its open sessions (`session.close`, worktree kept) and stop its device runs first.
    /// Without it the op refuses `project.sessions_live` / `project.activity_live`.
    pub force: Option<bool>,
    /// With `force`: also delete the Relay-pool worktrees of the sessions it closes. Branches
    /// are always kept; off by default, so every checkout stays on disk.
    pub remove_worktrees: Option<bool>,
});
result!(#[schemars(rename = "ProjectRemoveOut")] ProjectRemoveOut { pub sessions_closed: i64, pub runs_stopped: i64 });
op!(ProjectRemove, "project.remove", ProjectRemoveIn => ProjectRemoveOut,
    OpMeta::mutation(Scope::Project, 1, "Forget a project (repository untouched); conflict if live sessions unless force closes them").actors(Actors::UserOnly).emits(&["project.deleted", "session.changed", "worktree.changed", "run.changed"]));
result!(#[schemars(rename = "ProjectStatsOut")] ProjectStatsOut { pub tasks_by_column: BTreeMap<Column, i64>, pub sessions_live: i64, pub sessions_idle: i64, pub worktrees: i64, pub disk_mb: f64 });
op!(ProjectStats, "project.stats", ProjectGetIn => ProjectStatsOut, OpMeta::query(Scope::Project, 9, "Sidebar numbers for one project"));

entries!(WsCreate, WsDiscover, WsList, WsUpdate, WsRemove, ProjectAdd, ProjectClone, ProjectList, ProjectGet, ProjectUpdate, ProjectRemove, ProjectStats);
