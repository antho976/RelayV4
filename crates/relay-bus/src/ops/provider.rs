//! `provider.*` / `usage.*` / `skill.*` / `plugin.*` — BUS.md §10.15.
use crate::registry::{Actors, Audit, OpMeta, Scope, Undo};
use crate::types::{GitHubRepo, GitHubStatus, Id, Provider, ProviderInfo, Skill, Usage};
use crate::{op, Empty};
use serde_json::Value;

result!(#[schemars(rename = "ProviderListOut")] ListOut { pub providers: Vec<ProviderInfo> });
op!(List, "provider.list", Empty => ListOut, OpMeta::query(Scope::Global, 6, "Installed / version / auth / spawn profile per provider"));
op!(Refresh, "provider.refresh", Empty => ListOut,
    OpMeta::mutation(Scope::Global, 6, "Re-detect providers now").audit(Audit::Never).actors(Actors::UserOnly).emits(&["provider.version"]));
payload!(#[schemars(rename = "ProviderUpdateIn")] UpdateIn { pub provider: Provider, pub automatic: Option<bool> });
result!(#[schemars(rename = "ProviderUpdateOut")] UpdateOut { pub started: bool, pub method: String, pub message: String });
op!(Update, "provider.update", UpdateIn => UpdateOut,
    OpMeta::mutation(Scope::Global, 6, "Update a supported user-installed provider in the background").actors(Actors::UserOnly).emits(&["provider.update.changed"]));
payload!(#[schemars(rename = "UsageGetIn")] UsageGetIn { pub provider: Option<Provider> });
result!(#[schemars(rename = "UsageGetOut")] UsageGetOut { pub usage: Vec<Usage> });
op!(UsageGet, "usage.get", UsageGetIn => UsageGetOut, OpMeta::query(Scope::Global, 9, "Per-provider usage in its own units"));
payload!(#[schemars(rename = "UsageReportIn")] UsageReportIn { pub session: String, pub provider: Provider, pub payload: Value });
op!(UsageReport, "usage.report", UsageReportIn => Empty,
    OpMeta::mutation(Scope::Session, 9, "Provider metering pushed by its statusLine/hook").audit(Audit::Never).actors(Actors::AgentOnly).emits(&["usage.changed"]));

payload!(#[schemars(rename = "SkillListIn")] SkillListIn { pub project_id: Option<Id>, pub enabled: Option<bool> });
result!(#[schemars(rename = "SkillListOut")] SkillListOut { pub skills: Vec<Skill> });
op!(SkillList, "skill.list", SkillListIn => SkillListOut, OpMeta::query(Scope::Global, 11, "Skills (markdown instruction files)"));
payload!(#[schemars(rename = "SkillCreateIn")] SkillCreateIn { pub name: String, pub body: String });
op!(SkillCreate, "skill.create", SkillCreateIn => Skill,
    OpMeta::mutation(Scope::Global, 11, "Create a skill").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["skill.changed"]));
payload!(#[schemars(rename = "SkillUpdateIn")] SkillUpdateIn { pub skill_id: Id, pub name: Option<String>, pub body: Option<String> });
op!(SkillUpdate, "skill.update", SkillUpdateIn => Skill,
    OpMeta::mutation(Scope::Global, 11, "Patch a skill").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["skill.changed"]));
payload!(#[schemars(rename = "SkillIdIn")] SkillIdIn { pub skill_id: Id });
op!(SkillDelete, "skill.delete", SkillIdIn => Empty,
    OpMeta::mutation(Scope::Global, 11, "Delete a skill").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["skill.deleted"]));
payload!(#[schemars(rename = "SkillEnableIn")] SkillEnableIn { pub skill_id: Id, pub project_id: Id, pub enabled: bool });
op!(SkillEnable, "skill.enable", SkillEnableIn => Skill,
    OpMeta::mutation(Scope::Project, 11, "Enable / disable a skill for a project").actors(Actors::UserOnly).undo(Undo::Inverse).emits(&["skill.changed"]));
payload!(#[schemars(rename = "SkillInstallIn")] SkillInstallIn {
    pub url: String,
    pub subdir: Option<String>,
    /// Explicit permission to replace one visible skill when its name conflicts with the
    /// downloaded SKILL.md. Hidden soft-deleted rows are reclaimed automatically.
    pub replace_skill_id: Option<Id>
});
result!(#[schemars(rename = "SkillInstallOut")] SkillInstallOut { pub skills: Vec<Skill> });
op!(SkillInstall, "skill.install", SkillInstallIn => SkillInstallOut,
    OpMeta::mutation(Scope::Global, 12, "Install or refresh SKILL.md files from a GitHub repository").actors(Actors::UserOnly).emits(&["skill.changed"]));

op!(GitHubStatusOp, "github.status", Empty => GitHubStatus,
    OpMeta::query(Scope::Global, 12, "GitHub CLI installation and authentication state"));
result!(#[schemars(rename = "GitHubConnectOut")] GitHubConnectOut { pub started: bool });
op!(GitHubConnect, "github.connect", Empty => GitHubConnectOut,
    OpMeta::mutation(Scope::Global, 12, "Start GitHub CLI browser authentication").audit(Audit::Never).actors(Actors::UserOnly).emits(&["github.changed"]));
result!(#[schemars(rename = "GitHubRepoListOut")] GitHubRepoListOut { pub repositories: Vec<GitHubRepo> });
op!(GitHubRepoList, "github.repo.list", Empty => GitHubRepoListOut,
    OpMeta::query(Scope::Global, 12, "All GitHub repositories available to the connected account"));
result!(#[schemars(rename = "PluginListOut")] PluginListOut { pub plugins: Vec<Value> });
op!(PluginList, "plugin.list", Empty => PluginListOut, OpMeta::query(Scope::Global, 11, "Stub in 4.0: always empty"));

entries!(List, Refresh, Update, UsageGet, UsageReport, SkillList, SkillCreate, SkillUpdate, SkillDelete, SkillEnable, SkillInstall, GitHubStatusOp, GitHubConnect, GitHubRepoList, PluginList);
