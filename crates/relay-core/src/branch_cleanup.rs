//! Branch cleanup after a session's work lands.
//!
//! `session.close` removes a session's pooled worktree but always kept its branch, so every
//! finished session left a `relay/<name>` branch behind — locally, and on GitHub once its PR was
//! merged. This module deletes those branches once their work is provably merged, and never
//! otherwise:
//!
//! - the branch tip is an ancestor of the base branch (local or `origin/<base>`), or every one of
//!   its commits is already upstream by patch id (`git cherry`: a rebase merge); or
//! - GitHub (`gh`) reports a merged pull request whose head is the branch tip, or contains it (a
//!   squash merge, which no local git check can prove).
//!
//! A branch with commits that neither covers is kept, and the reason is recorded. A worktree
//! still holding the branch is removed first only when it is a pooled checkout that no live
//! session owns and that has no uncommitted changes. The remote branch is deleted only when a
//! merged PR proves it, and only while it still points at that PR's head.
//!
//! It runs after a session closes, when `git.pr.list` sees a PR merged for a closed session's
//! branch, on a slow background sweep, and on demand (`git.branch.cleanup`). All of it is
//! subprocess work, so none of it holds the store: it reads the candidates in one short lock,
//! runs git and gh bounded by `proc::output_with_timeout`, and records each deletion as a
//! `system` audit row.

use crate::engine::Engine;
use crate::worktree;
use relay_bus::error::BusError;
use relay_bus::ops::git::BranchCleanupRow;
use relay_bus::types::Id;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

const GIT_TIMEOUT: Duration = Duration::from_secs(20);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
/// First sweep after start, then the interval between sweeps.
const SWEEP_FIRST: Duration = Duration::from_secs(90);
const SWEEP_EVERY: Duration = Duration::from_secs(20 * 60);
/// A sweep does not ask GitHub again about an unchanged, unmerged branch more often than this.
const GH_RECHECK: Duration = Duration::from_secs(60 * 60);

/// One closed session's branch, as the store knows it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub project_id: Id,
    pub repo: PathBuf,
    pub base: String,
    pub branch: String,
    pub session: String,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub dry_run: bool,
    /// `gh`, when GitHub can be asked about merged PRs.
    pub gh: Option<PathBuf>,
    /// Write a `system` audit row for branches kept too, not only for deletions. Set after a
    /// close, where the user is waiting to learn why a branch stayed; a periodic sweep would
    /// write the same row every time.
    pub audit_kept: bool,
    /// Skip the GitHub query for branches it answered recently (the periodic sweep).
    pub use_gh_cache: bool,
}

/// Closed sessions' branches that no other open session uses, newest session first, one per
/// (project, branch). `only` narrows to those branch names.
/// Worktree paths still owned by open sessions, per project.
pub type LiveWorktrees = HashMap<Id, Vec<String>>;

pub fn candidates(conn: &Connection, project_id: Option<Id>, only: Option<&[String]>) -> Result<(Vec<Candidate>, LiveWorktrees), BusError> {
    let mut statement = conn.prepare_cached(
        "SELECT s.project_id, p.path, p.base_branch, s.branch, s.name FROM sessions s JOIN projects p ON p.id = s.project_id
         WHERE s.state = 'closed' AND s.branch != '' AND s.branch != p.base_branch AND (?1 IS NULL OR s.project_id = ?1)
           AND NOT EXISTS (SELECT 1 FROM sessions o WHERE o.project_id = s.project_id AND o.branch = s.branch AND o.state != 'closed')
         ORDER BY s.id DESC",
    ).map_err(crate::engine::internal)?;
    let rows = statement.query_map([project_id], |row| Ok(Candidate {
        project_id: row.get(0)?,
        repo: PathBuf::from(row.get::<_, String>(1)?),
        base: row.get(2)?,
        branch: row.get(3)?,
        session: row.get(4)?,
    })).map_err(crate::engine::internal)?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        let row = row.map_err(crate::engine::internal)?;
        if only.is_some_and(|only| !only.contains(&row.branch)) { continue; }
        if seen.insert((row.project_id, row.branch.clone())) { out.push(row); }
    }
    // Worktrees still owned by open sessions, per project: never removed from under them.
    let mut live = LiveWorktrees::new();
    let mut statement = conn.prepare_cached("SELECT project_id, worktree FROM sessions WHERE state != 'closed'")
        .map_err(crate::engine::internal)?;
    for row in statement.query_map([], |row| Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?))).map_err(crate::engine::internal)? {
        let (project, path) = row.map_err(crate::engine::internal)?;
        live.entry(project).or_default().push(path);
    }
    Ok((out, live))
}

struct Git<'a> { repo: &'a Path }

struct Ran { ok: bool, out: String, err: String }

impl Git<'_> {
    fn run(&self, args: &[&str], timeout: Duration) -> Result<Ran, String> {
        let mut command = Command::new("git");
        command.arg("-C").arg(self.repo).args(args)
            .env("GIT_TERMINAL_PROMPT", "0").env("GIT_OPTIONAL_LOCKS", "0");
        let output = crate::proc::output_with_timeout(&mut command, timeout)
            .map_err(|error| format!("git {}: {error}", args.join(" ")))?
            .ok_or_else(|| format!("git {} timed out after {}s", args.join(" "), timeout.as_secs()))?;
        Ok(Ran {
            ok: output.status.success(),
            out: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            err: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
    fn ok(&self, args: &[&str]) -> bool {
        self.run(args, GIT_TIMEOUT).is_ok_and(|ran| ran.ok)
    }
    fn value(&self, args: &[&str]) -> Option<String> {
        self.run(args, GIT_TIMEOUT).ok().filter(|ran| ran.ok && !ran.out.is_empty()).map(|ran| ran.out)
    }
    fn resolve(&self, reference: &str) -> Option<String> {
        self.value(&["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")])
    }
    fn is_ancestor(&self, commit: &str, of: &str) -> bool {
        self.ok(&["merge-base", "--is-ancestor", commit, of])
    }
}

/// Where a branch is checked out, from `git worktree list --porcelain`.
fn checkouts(git: &Git, branch: &str) -> Vec<PathBuf> {
    let Ok(ran) = git.run(&["worktree", "list", "--porcelain"], GIT_TIMEOUT) else { return Vec::new() };
    let mut out = Vec::new();
    let mut path: Option<PathBuf> = None;
    let full = format!("refs/heads/{branch}");
    for line in ran.out.lines() {
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if line.strip_prefix("branch ") == Some(full.as_str()) {
            if let Some(path) = path.take() { out.push(path); }
        }
    }
    out
}

fn canon(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[derive(Deserialize)]
struct GhPr {
    number: u64,
    #[serde(rename = "headRefOid")]
    head: String,
}

/// Merged PRs whose head branch is `branch`, newest first. `None` when GitHub could not be asked
/// (no `gh`, not signed in, offline): the caller then decides on local evidence alone.
fn merged_prs(gh: &Path, repo: &Path, branch: &str) -> Option<Vec<GhPr>> {
    let mut command = Command::new(gh);
    command.current_dir(repo)
        .args(["pr", "list", "--head", branch, "--state", "merged", "--json", "number,headRefOid", "--limit", "20"])
        .env("GH_PROMPT_DISABLED", "1");
    let output = crate::proc::output_with_timeout(&mut command, NETWORK_TIMEOUT).ok()??;
    if !output.status.success() {
        tracing::debug!(branch, stderr = %String::from_utf8_lossy(&output.stderr).trim(), "gh pr list failed");
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// (repo, branch) → (tip, when) of the last GitHub answer that found no merged PR.
type GhCache = HashMap<(PathBuf, String), (String, Instant)>;
static GH_CACHE: Mutex<Option<GhCache>> = Mutex::new(None);
static SWEEPING: Mutex<()> = Mutex::new(());

fn gh_recently_kept(repo: &Path, branch: &str, tip: &str) -> bool {
    let cache = GH_CACHE.lock().unwrap_or_else(|poison| poison.into_inner());
    cache.as_ref().and_then(|cache| cache.get(&(repo.to_path_buf(), branch.to_string())))
        .is_some_and(|(seen_tip, at)| seen_tip == tip && at.elapsed() < GH_RECHECK)
}

fn remember_gh(repo: &Path, branch: &str, tip: &str) {
    let mut cache = GH_CACHE.lock().unwrap_or_else(|poison| poison.into_inner());
    cache.get_or_insert_with(HashMap::new).insert((repo.to_path_buf(), branch.to_string()), (tip.to_string(), Instant::now()));
}

/// How a branch's work was found merged.
struct Merged { reason: String, pr: Option<GhPr> }

/// Decide one candidate and, unless `dry_run`, act on it.
pub fn evaluate(candidate: &Candidate, live_worktrees: &[String], options: &Options) -> BranchCleanupRow {
    let git = Git { repo: &candidate.repo };
    let branch = candidate.branch.as_str();
    let mut row = BranchCleanupRow {
        branch: branch.to_string(),
        session: Some(candidate.session.clone()),
        outcome: "kept".into(),
        reason: String::new(),
        pr: None,
        removed_worktree: false,
        deleted_remote: false,
    };
    let kept = |mut row: BranchCleanupRow, reason: String| { row.reason = reason; row };
    let Some(tip) = git.resolve(&format!("refs/heads/{branch}")) else {
        row.outcome = "gone".into();
        row.reason = "the branch no longer exists".into();
        return row;
    };

    // Where it is checked out decides whether it can go at all.
    let primary = canon(&candidate.repo);
    let pool = canon(&worktree::pool_dir(&candidate.repo));
    let mut removable = None;
    for path in checkouts(&git, branch) {
        let path = canon(&path);
        if path == primary {
            return kept(row, "checked out in the primary checkout".into());
        }
        if live_worktrees.iter().any(|live| canon(Path::new(live)) == path) {
            return kept(row, format!("checked out at {} by an open session", path.display()));
        }
        if !path.starts_with(&pool) {
            return kept(row, format!("checked out at {}, outside Relay's worktree pool", path.display()));
        }
        removable = Some(path);
    }

    let remote = git.value(&["config", "--get", &format!("branch.{branch}.remote")]).unwrap_or_else(|| "origin".into());
    let remote_tip = git.resolve(&format!("refs/remotes/{remote}/{branch}"));
    let pushed = remote_tip.is_some() || git.value(&["config", "--get", &format!("branch.{branch}.merge")]).is_some();

    let mut merged: Option<Merged> = None;
    let bases: Vec<String> = [format!("refs/heads/{}", candidate.base), format!("refs/remotes/{remote}/{}", candidate.base)]
        .into_iter().filter(|reference| git.resolve(reference).is_some()).collect();
    for base in &bases {
        let shown = base.trim_start_matches("refs/heads/").trim_start_matches("refs/remotes/");
        if git.is_ancestor(&tip, base) {
            merged = Some(Merged { reason: format!("merged into {shown}"), pr: None });
            break;
        }
        if let Ok(ran) = git.run(&["cherry", base, &tip], GIT_TIMEOUT) {
            if ran.ok && !ran.out.is_empty() && ran.out.lines().all(|line| line.starts_with('-')) {
                merged = Some(Merged { reason: format!("every commit is already in {shown} (rebased)"), pr: None });
                break;
            }
        }
    }

    // GitHub: the only proof of a squash merge, and the only licence to delete the remote branch.
    let mut unmerged_pr_note = None;
    if pushed {
        if let Some(gh) = options.gh.as_deref().filter(|_| !(options.use_gh_cache && merged.is_none() && gh_recently_kept(&candidate.repo, branch, &tip))) {
            if let Some(prs) = merged_prs(gh, &candidate.repo, branch) {
                let covering = prs.into_iter().find(|pr| {
                    pr.head == tip || git.is_ancestor(&tip, &pr.head)
                });
                match covering {
                    Some(pr) => {
                        let reason = match &merged {
                            Some(found) => format!("{}; PR #{} merged", found.reason, pr.number),
                            None => format!("PR #{} merged on GitHub", pr.number),
                        };
                        merged = Some(Merged { reason, pr: Some(pr) });
                    }
                    None if merged.is_none() => {
                        remember_gh(&candidate.repo, branch, &tip);
                        unmerged_pr_note = Some("no merged PR contains its tip".to_string());
                    }
                    None => {}
                }
            }
        }
    }

    let Some(merged) = merged else {
        let base = bases.first().cloned().unwrap_or_else(|| candidate.base.clone());
        let ahead = git.value(&["rev-list", "--count", &format!("{base}..{tip}")]).unwrap_or_else(|| "some".into());
        let mut reason = format!("{ahead} commit(s) not in {}", candidate.base);
        if let Some(note) = unmerged_pr_note { reason.push_str(&format!("; {note}")); }
        else if !pushed { reason.push_str("; never pushed"); }
        return kept(row, reason);
    };
    row.pr = merged.pr.as_ref().map(|pr| pr.number);
    row.reason = merged.reason;

    if let Some(path) = &removable {
        match worktree::status_files(path) {
            Ok(files) if files.is_empty() => {}
            Ok(files) => return kept(row.clone(), format!("{}, but {} has {} uncommitted change(s)", row.reason, path.display(), files.len())),
            Err(error) => return kept(row.clone(), format!("{}, but {} could not be checked: {error}", row.reason, path.display())),
        }
    }
    if options.dry_run {
        row.outcome = "would_delete".into();
        return row;
    }
    if let Some(path) = &removable {
        if let Err(error) = worktree::remove(&candidate.repo, path, true) {
            return kept(row.clone(), format!("{}, but removing {} failed: {error}", row.reason, path.display()));
        }
        row.removed_worktree = true;
    }
    // Compare-and-delete: if anything committed to the branch since it was judged, this fails
    // and the new work stays.
    match git.run(&["update-ref", "-d", &format!("refs/heads/{branch}"), &tip], GIT_TIMEOUT) {
        Ok(ran) if ran.ok => {}
        Ok(ran) => return kept(row.clone(), format!("{}, but deleting it failed: {}", row.reason, ran.err)),
        Err(error) => return kept(row.clone(), format!("{}, but deleting it failed: {error}", row.reason)),
    }
    let _ = git.run(&["config", "--remove-section", &format!("branch.{branch}")], GIT_TIMEOUT);
    row.outcome = "deleted".into();

    if let Some(pr) = &merged.pr {
        // Only the branch the PR merged, and only if nobody pushed to it since.
        match git.run(&["ls-remote", "--heads", &remote, &format!("refs/heads/{branch}")], NETWORK_TIMEOUT) {
            Ok(ran) if ran.ok => {
                let live_tip = ran.out.split_whitespace().next().map(str::to_string);
                match live_tip {
                    Some(sha) if sha == pr.head || sha == tip => {
                        match git.run(&["push", &remote, "--delete", branch], NETWORK_TIMEOUT) {
                            Ok(ran) if ran.ok => {
                                row.deleted_remote = true;
                                let _ = git.run(&["update-ref", "-d", &format!("refs/remotes/{remote}/{branch}")], GIT_TIMEOUT);
                            }
                            Ok(ran) => row.reason.push_str(&format!("; remote branch kept: {}", ran.err)),
                            Err(error) => row.reason.push_str(&format!("; remote branch kept: {error}")),
                        }
                    }
                    Some(_) => row.reason.push_str(&format!("; {remote}/{branch} has commits after the PR, kept")),
                    None => {
                        // GitHub already deleted it; drop the stale tracking ref.
                        let _ = git.run(&["update-ref", "-d", &format!("refs/remotes/{remote}/{branch}")], GIT_TIMEOUT);
                    }
                }
            }
            Ok(ran) => row.reason.push_str(&format!("; remote branch not checked: {}", ran.err)),
            Err(error) => row.reason.push_str(&format!("; remote branch not checked: {error}")),
        }
    }
    row
}

/// Clean up every candidate of `project_id` (all projects when `None`), narrowed to `only`.
/// Takes the store only to read the candidates and to append audit rows.
pub fn run(engine: &Engine, project_id: Option<Id>, only: Option<&[String]>, options: &Options) -> Result<Vec<BranchCleanupRow>, BusError> {
    let _one_at_a_time = SWEEPING.lock().unwrap_or_else(|poison| poison.into_inner());
    let (candidates, live) = {
        let conn = engine.store.lock();
        candidates(&conn, project_id, only)?
    };
    let mut out = Vec::new();
    for candidate in &candidates {
        if engine.is_quitting() { break; }
        if !candidate.repo.is_dir() { continue; }
        let row = evaluate(candidate, live.get(&candidate.project_id).map(Vec::as_slice).unwrap_or_default(), options);
        if row.outcome == "gone" { continue; }
        record(engine, candidate, &row, options);
        out.push(row);
    }
    Ok(out)
}

fn record(engine: &Engine, candidate: &Candidate, row: &BranchCleanupRow, options: &Options) {
    let summary = json!({
        "branch": row.branch, "session": row.session, "outcome": row.outcome, "reason": row.reason,
        "pr": row.pr, "removed_worktree": row.removed_worktree, "deleted_remote": row.deleted_remote,
    });
    match row.outcome.as_str() {
        "deleted" => tracing::info!(branch = %row.branch, reason = %row.reason, remote = row.deleted_remote, "deleted merged session branch"),
        "kept" => tracing::info!(branch = %row.branch, reason = %row.reason, "kept session branch"),
        _ => {}
    }
    let audited = row.outcome == "deleted" || (row.outcome == "kept" && options.audit_kept);
    if !audited || options.dry_run { return; }
    let repo = candidate.repo.display().to_string();
    let deleted = row.outcome == "deleted";
    let removed = row.removed_worktree;
    let project_id = candidate.project_id;
    let _ = engine.system_write("git.branch.cleanup", None, Some(project_id), None, summary.clone(), move |_, _| {
        let mut events = Vec::new();
        if deleted {
            events.push(("git.changed".to_string(), json!({"project_id": project_id, "worktree": repo, "branch_cleanup": summary})));
        }
        if removed {
            events.push(("worktree.changed".to_string(), json!({"project_id": project_id})));
        }
        Ok(((), events))
    });
}

pub fn gh() -> Option<PathBuf> {
    which::which("gh").ok()
}

/// After `session.close`: settle that one branch now, off the request thread.
pub fn after_close(engine: Arc<Engine>, project_id: Id, branch: String) {
    std::thread::Builder::new().name("branch-cleanup".into()).spawn(move || {
        crate::background_priority();
        let options = Options { gh: gh(), audit_kept: true, ..Options::default() };
        if let Err(error) = run(&engine, Some(project_id), Some(std::slice::from_ref(&branch)), &options) {
            tracing::warn!(branch, error = %error.message, "branch cleanup after close failed");
        }
    }).ok();
}

/// `git.pr.list` saw these branches merged on GitHub: clean up the ones closed sessions left.
pub fn after_merged_prs(engine: Arc<Engine>, project_id: Id, branches: Vec<String>) {
    if branches.is_empty() { return; }
    std::thread::Builder::new().name("branch-cleanup".into()).spawn(move || {
        crate::background_priority();
        let options = Options { gh: gh(), ..Options::default() };
        if let Err(error) = run(&engine, Some(project_id), Some(&branches), &options) {
            tracing::warn!(error = %error.message, "branch cleanup after merged PRs failed");
        }
    }).ok();
}

/// The slow sweep for branches whose PR merged after their session closed. Holds only a weak
/// reference, so it never keeps an engine alive.
pub fn spawn_sweeper(engine: &Arc<Engine>) {
    let weak: Weak<Engine> = Arc::downgrade(engine);
    std::thread::Builder::new().name("branch-sweep".into()).spawn(move || {
        crate::background_priority();
        let mut wait = SWEEP_FIRST;
        loop {
            let deadline = Instant::now() + wait;
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_secs(1));
                match weak.upgrade() {
                    Some(engine) if !engine.is_quitting() => {}
                    _ => return,
                }
            }
            let Some(engine) = weak.upgrade() else { return };
            let options = Options { gh: gh(), use_gh_cache: true, ..Options::default() };
            match run(&engine, None, None, &options) {
                Ok(rows) => {
                    let deleted = rows.iter().filter(|row| row.outcome == "deleted").count();
                    if deleted > 0 { tracing::info!(deleted, "branch sweep deleted merged session branches"); }
                }
                Err(error) => tracing::warn!(error = %error.message, "branch sweep failed"),
            }
            drop(engine);
            wait = SWEEP_EVERY;
        }
    }).ok();
}
