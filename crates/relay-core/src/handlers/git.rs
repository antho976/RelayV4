//! `worktree.*` (phase 3). `git.*` and `integration.*` land in phase 8.

use crate::engine::{Ctx, Engine, IntoBus, Unlocked};
use crate::handlers::workspace::get_project;
use crate::worktree;
use relay_bus::error::BusError;
use relay_bus::ops::app::WorktreeDisk as WorktreeDiskRow;
use relay_bus::ops::git::*;
use relay_bus::types::{Branch, Commit, DiffFile, FileStatus, GateKind, Hunk, Worktree};
use rusqlite::{Connection, Transaction};
use serde::Deserialize;
use serde_json::json;
use similar::{ChangeTag, TextDiff};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Live sessions of a project as (branch, name), for `git.branches`.
fn worktree_owners_by_branch(
    conn: &Connection,
    project_id: relay_bus::types::Id,
) -> Result<Vec<(String, String)>, BusError> {
    let mut stmt = conn
        .prepare_cached("SELECT branch,name FROM sessions WHERE project_id=?1 AND state!='closed'")
        .bus()?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>();
    rows.bus()
}

/// Live sessions of a project as (name, worktree). The store half of [`list_with_owners`].
pub fn worktree_owners(
    conn: &Connection,
    project_id: relay_bus::types::Id,
) -> Result<Vec<(String, String)>, BusError> {
    let mut st = conn
        .prepare_cached("SELECT name, worktree FROM sessions WHERE project_id = ?1 AND state != 'closed'")
        .bus()?;
    let owners = st
        .query_map([project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .bus()?
        .collect::<rusqlite::Result<Vec<_>>>();
    owners.bus()
}

/// `git worktree list` plus the owning session names already read out of the store. The
/// subprocess belongs outside the lock, so callers read `owners` first and pass it in (D144).
pub fn worktrees_owned_by(
    owners: &[(String, String)],
    repo: &Path,
    include_dirty: bool,
) -> Result<Vec<Worktree>, BusError> {
    let mut wts = worktree::list_with_dirty(repo, include_dirty)
        .map_err(|e| BusError::unavailable("worktree.list_failed", e.to_string()))?;
    for w in &mut wts {
        w.session = owners
            .iter()
            .find(|(_, path)| *path == w.path)
            .map(|(n, _)| n.clone());
    }
    Ok(wts)
}

/// Worktrees of a project with the owning session filled in from the store.
pub fn list_with_owners(
    tx: &Transaction,
    project_id: relay_bus::types::Id,
    repo: &Path,
    include_dirty: bool,
) -> Result<Vec<Worktree>, BusError> {
    let owners = worktree_owners(tx, project_id)?;
    worktrees_owned_by(&owners, repo, include_dirty)
}

/// How long `gh pr list` may take before Relay stops waiting on the network.
const PR_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub fn register(e: &mut Engine) {
    e.register_unlocked::<WorktreeList>(|ctx, p| {
        let (project, owners) = ctx.read(|conn| {
            let project = get_project(conn, p.project_id)?;
            let owners = worktree_owners(conn, project.id)?;
            Ok((project, owners))
        })?;
        Ok(WorktreeListOut {
            worktrees: worktrees_owned_by(
                &owners,
                Path::new(&project.path),
                p.include_dirty.unwrap_or(true),
            )?,
        })
    });
    // `git worktree add` checks a tree out; the store has nothing to do while it runs (D144).
    e.register_staged::<WorktreeCreate, _>(
        |ctx, p| {
            let project = ctx.read(|conn| get_project(conn, p.project_id))?;
            let repo = Path::new(&project.path);
            let name = p.branch.trim_start_matches("relay/").replace('/', "-");
            let path = worktree::pooled_path(repo, &name);
            if path.exists() {
                return Err(BusError::conflict(
                    "worktree.exists",
                    format!("{} already exists", path.display()),
                ));
            }
            let wt = worktree::create(repo, &path, &p.branch, p.from.as_deref())
                .map_err(|e| BusError::conflict("worktree.create_failed", e.to_string()))?;
            Ok((project, wt))
        },
        |ctx: &mut Ctx, _p, (project, wt)| {
            ctx.set_project(project.id);
            ctx.emit("worktree.changed", json!({ "project_id": project.id }));
            Ok(wt)
        },
    );
    e.register::<WorktreeRemove>(|ctx: &mut Ctx, p| {
        let project = get_project(ctx.tx(), p.project_id)?;
        let repo = Path::new(&project.path);
        let want = std::fs::canonicalize(&p.path).map(|c| c.display().to_string()).unwrap_or(p.path.clone());
        let owner: Option<String> = ctx.tx().query_row(
            "SELECT name FROM sessions WHERE project_id = ?1 AND worktree = ?2 AND state != 'closed'",
            rusqlite::params![project.id, want], |r| r.get(0)).ok();
        if let Some(name) = owner {
            return Err(BusError::conflict("worktree.owned", format!("session {name} owns {want}")).with_hint("session.close it first"));
        }
        let freed = worktree::remove(repo, Path::new(&want), p.purge_build.unwrap_or(true))
            .map_err(|e| BusError::conflict("worktree.remove_failed", e.to_string()))?;
        ctx.set_project(project.id);
        ctx.emit("worktree.changed", json!({ "project_id": project.id }));
        Ok(FreedOut { freed_mb: freed as f64 / (1024.0 * 1024.0) })
    });
    e.register_unlocked::<WorktreeDisk>(|ctx, p| {
        let project = ctx.read(|conn| get_project(conn, p.project_id))?;
        let wts = worktree::list(Path::new(&project.path))
            .map_err(|e| BusError::unavailable("worktree.list_failed", e.to_string()))?;
        let worktrees = wts
            .iter()
            .map(|w| {
                let path = Path::new(&w.path);
                WorktreeDiskRow {
                    path: w.path.clone(),
                    disk_mb: worktree::dir_size(path) as f64 / (1024.0 * 1024.0),
                    build_mb: Some(worktree::build_size(path) as f64 / (1024.0 * 1024.0)),
                }
            })
            .collect();
        Ok(WorktreeDiskOut { worktrees })
    });
    e.register_unlocked::<Status>(|ctx, p| {
        let (project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        crate::watch::ensure_after_commit(ctx, root.clone(), project.id);
        let repo = gix::open(&root).map_err(gix_err("git.open_failed"))?;
        let branch = repo
            .head_name()
            .ok()
            .flatten()
            .map(|n| n.shorten().to_string())
            .unwrap_or_else(|| "HEAD".into());
        let (upstream, ahead, behind) = upstream_metrics(&repo, &branch);
        Ok(StatusOut {
            branch,
            upstream,
            ahead,
            behind,
            files: status_files(&root)?,
        })
    });
    e.register_unlocked::<Diff>(|ctx, p| {
        let (_project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        let mut files = if let Some(base) = p.base.as_deref() {
            let repo = gix::open(&root).map_err(gix_err("git.open_failed"))?;
            let old_tree = repo
                .rev_parse_single(base)
                .map_err(gix_err("git.revision"))?
                .object()
                .map_err(gix_err("git.diff_failed"))?
                .peel_to_commit()
                .map_err(gix_err("git.diff_failed"))?
                .tree()
                .map_err(gix_err("git.diff_failed"))?;
            let new_tree = repo
                .head_commit()
                .map_err(gix_err("git.diff_failed"))?
                .tree()
                .map_err(gix_err("git.diff_failed"))?;
            tree_diff_files(&repo, &old_tree, &new_tree, "git.diff_failed")?
        } else {
            Vec::new()
        };
        for status in status_files(&root)? {
            if p.staged == Some(true) && status.index.is_empty() {
                continue;
            }
            let old = revision_text(&root, p.base.as_deref().unwrap_or("HEAD"), &status.path)?;
            let new = if p.staged == Some(true) {
                index_text(&root, &status.path)?
            } else {
                working_text(&root, &status.path)?
            };
            let (added, removed) = counts(&old, &new);
            let file = DiffFile {
                path: status.path,
                old_path: status.renamed_from,
                status: if !status.index.is_empty() {
                    status.index
                } else {
                    status.worktree
                },
                added,
                removed,
                binary: old.contains('\0') || new.contains('\0'),
            };
            if let Some(existing) = files.iter_mut().find(|existing| existing.path == file.path) {
                *existing = file;
            } else {
                files.push(file);
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(DiffOut { files })
    });
    e.register_unlocked::<DiffFileOp>(|ctx, p| {
        let (_project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        validate_path(&p.path)?;
        let old = revision_text(&root, p.base.as_deref().unwrap_or("HEAD"), &p.path)?;
        let new = working_text(&root, &p.path)?;
        let hunks = if old == new {
            Vec::new()
        } else {
            vec![Hunk {
                old_start: 1,
                old_lines: old.lines().count() as i64,
                new_start: 1,
                new_lines: new.lines().count() as i64,
                text: TextDiff::from_lines(&old, &new)
                    .unified_diff()
                    .context_radius(3)
                    .to_string(),
            }]
        };
        Ok(DiffFileOut { old, new, hunks })
    });
    e.register_unlocked::<Log>(|ctx, p| {
        let (_project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        let repo = gix::open(&root).map_err(gix_err("git.open_failed"))?;
        let tip = repo
            .rev_parse_single(p.branch.as_deref().unwrap_or("HEAD"))
            .map_err(gix_err("git.revision"))?;
        let walk = repo
            .rev_walk([tip.detach()])
            .all()
            .map_err(gix_err("git.log_failed"))?;
        let mut commits = Vec::new();
        for info in walk.take(p.limit.unwrap_or(50).min(500) as usize) {
            let info = info.map_err(gix_err("git.log_failed"))?;
            commits.push(commit_from_gix(
                info.object().map_err(gix_err("git.log_failed"))?,
            )?);
        }
        Ok(LogOut { commits })
    });
    e.register_unlocked::<Show>(|ctx, p| {
        let project = ctx.read(|conn| get_project(conn, p.project_id))?;
        let repo = gix::open(&project.path).map_err(gix_err("git.open_failed"))?;
        let id = repo
            .rev_parse_single(p.sha.as_str())
            .map_err(gix_err("git.revision"))?;
        let object = id
            .object()
            .map_err(gix_err("git.show_failed"))?
            .peel_to_commit()
            .map_err(gix_err("git.show_failed"))?;
        let files = commit_diff_files(&repo, &object)?;
        let commit = commit_from_gix(object)?;
        Ok(ShowOut { commit, files })
    });
    e.register_unlocked::<Branches>(|ctx, p| {
        let (project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        let owner_rows = ctx.read(|conn| {
            let owners = worktree_owners_by_branch(conn, project.id)?;
            Ok(owners)
        })?;
        let repo = gix::open(&root).map_err(gix_err("git.open_failed"))?;
        let current = repo
            .head_name()
            .ok()
            .flatten()
            .map(|n| n.shorten().to_string())
            .unwrap_or_default();
        let head = repo
            .rev_parse_single(project.base_branch.as_str())
            .ok()
            .map(|id| id.detach());
        let owners: HashMap<String, String> = owner_rows.into_iter().collect();
        let mut branches = Vec::new();
        let platform = repo.references().map_err(gix_err("git.branches_failed"))?;
        let refs = platform
            .local_branches()
            .map_err(gix_err("git.branches_failed"))?;
        for reference in refs {
            let reference = reference.map_err(gix_err("git.branches_failed"))?;
            let name = reference
                .name()
                .as_bstr()
                .to_string()
                .trim_start_matches("refs/heads/")
                .to_string();
            let Some(id) = reference.try_id() else {
                continue;
            };
            let merged = head
                .as_ref()
                .and_then(|h| repo.merge_base(*h, id.detach()).ok())
                .map(|base| base.detach() == id.detach())
                .unwrap_or(false);
            let (upstream, ahead, behind) = upstream_metrics(&repo, &name);
            branches.push(Branch {
                name: name.clone(),
                head: id.to_string(),
                upstream,
                ahead,
                behind,
                merged,
                session: owners.get(&name).cloned(),
                current: name == current,
            });
        }
        branches.sort_by_key(|b| (!b.current, b.name.clone()));
        Ok(BranchesOut { current, branches })
    });
    e.register::<BranchCreate>(|ctx: &mut Ctx, p| {
        let (project, root) = resolve_root(ctx, p.project_id, p.worktree.as_deref())?;
        let name = validate_branch_name(&root, &p.name)?;
        let repo = gix::open(&root).map_err(gix_err("git.open_failed"))?;
        let full_name = format!("refs/heads/{name}");
        if repo.find_reference(full_name.as_str()).is_ok() {
            return Err(BusError::conflict(
                "git.branch_exists",
                format!("branch {name} already exists"),
            ));
        }
        let start = p.start_point.as_deref().map(str::trim).filter(|value| !value.is_empty());
        if let Some(revision) = start {
            repo.rev_parse_single(revision)
                .map_err(|_| BusError::invalid("git.start_point", format!("unknown start point {revision}")))?;
        }
        let mut args = if p.checkout.unwrap_or(true) {
            vec!["switch", "-c", name.as_str()]
        } else {
            vec!["branch", name.as_str()]
        };
        if let Some(revision) = start {
            args.push(revision);
        }
        worktree::git_mutate(&root, &args).map_err(git_mutation("git.branch_create_failed"))?;
        let repo = gix::open(&root).map_err(gix_err("git.open_failed"))?;
        let head = repo.head_id().map_err(gix_err("git.head"))?.to_string();
        changed(ctx, project.id, &root);
        Ok(BranchCreateOut { name, head, worktree: root.display().to_string() })
    });
    e.register::<BranchDelete>(|ctx: &mut Ctx, p| {
        let project = get_project(ctx.tx(), p.project_id)?;
        let root = Path::new(&project.path);
        let name = validate_branch_name(root, &p.name)?;
        if name == project.base_branch {
            return Err(BusError::conflict(
                "git.branch_protected",
                format!("cannot delete the base branch {name}"),
            ));
        }
        let branches = branches_for(ctx.tx(), &project)?;
        let branch = branches
            .branches
            .iter()
            .find(|branch| branch.name == name)
            .ok_or_else(|| BusError::not_found("git.branch_not_found", format!("no local branch {name}")))?;
        if let Some(session) = &branch.session {
            return Err(BusError::conflict(
                "git.branch_session_owned",
                format!("branch {name} belongs to session {session}"),
            ));
        }
        if !branch.merged {
            return Err(BusError::conflict(
                "git.branch_unmerged",
                format!("branch {name} is not merged into {}", project.base_branch),
            ));
        }
        if worktree::list(root)
            .map_err(|error| BusError::unavailable("worktree.list_failed", error.to_string()))?
            .iter()
            .any(|worktree| worktree.branch == name)
        {
            return Err(BusError::conflict(
                "git.branch_checked_out",
                format!("branch {name} is checked out in a worktree"),
            ));
        }
        worktree::git_mutate(root, &["branch", "-d", &name])
            .map_err(git_mutation("git.branch_delete_failed"))?;
        changed(ctx, project.id, root);
        Ok(relay_bus::Empty {})
    });
    e.register::<Stage>(|ctx: &mut Ctx, p| {
        let (project, root) = resolve_root(ctx, p.project_id, p.worktree.as_deref())?;
        if p.paths.is_empty() {
            return Err(BusError::invalid(
                "git.paths",
                "at least one path is required",
            ));
        }
        for path in &p.paths {
            validate_path(path)?;
        }
        let mut args = vec!["add", "--"];
        args.extend(p.paths.iter().map(String::as_str));
        worktree::git_mutate(&root, &args).map_err(git_mutation("git.stage_failed"))?;
        changed(ctx, project.id, &root);
        Ok(relay_bus::Empty {})
    });
    e.register::<Unstage>(|ctx: &mut Ctx, p| {
        let (project, root) = resolve_root(ctx, p.project_id, p.worktree.as_deref())?;
        if p.paths.is_empty() {
            return Err(BusError::invalid(
                "git.paths",
                "at least one path is required",
            ));
        }
        for path in &p.paths {
            validate_path(path)?;
        }
        let unborn = gix::open(&root)
            .map(|repo| repo.head_id().is_err())
            .unwrap_or(true);
        let mut args = if unborn {
            vec!["rm", "--cached", "--"]
        } else {
            vec!["reset", "HEAD", "--"]
        };
        args.extend(p.paths.iter().map(String::as_str));
        worktree::git_mutate(&root, &args).map_err(git_mutation("git.unstage_failed"))?;
        changed(ctx, project.id, &root);
        Ok(relay_bus::Empty {})
    });
    // Staging, hook refresh and the user's own pre-commit hook are all subprocesses of unbounded
    // length — a slow `pre-commit` used to hold the one store connection for its whole run, so
    // every keystroke in every terminal waited on it. They happen before anything is locked; the
    // transaction only sees the gate and the commit itself (D144).
    e.register_staged::<CommitOp, _>(
        |ctx, p| {
            if p.message.trim().is_empty() {
                return Err(BusError::invalid(
                    "git.message",
                    "commit message cannot be empty",
                ));
            }
            let (project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
            if p.all.unwrap_or(false) {
                worktree::git_mutate(&root, &["add", "-A"])
                    .map_err(git_mutation("git.stage_failed"))?;
            }
            crate::hooks::refresh_git(
                Path::new(&project.path),
                &root,
                ctx.instance(),
                &crate::hooks::relay_bin(),
            )
            .map_err(|error| BusError::unavailable("git.hook_refresh_failed", error.to_string()))?;
            crate::hooks::run_user_pre_commit(Path::new(&project.path), &root)
                .map_err(|error| BusError::conflict("git.pre_commit_failed", error.to_string()))?;
            let numstat = staged_numstat(&root)?;
            Ok((project, root, numstat))
        },
        |ctx: &mut Ctx, p, (project, root, numstat)| {
            super::guardrail::enforce(
                ctx,
                project.id,
                &root,
                GateKind::Commit,
                None,
                None,
                Some(&numstat),
                None,
            )?;
            worktree::git_mutate(&root, &["commit", "--no-verify", "-m", &p.message])
                .map_err(git_mutation("git.commit_failed"))?;
            let repo = gix::open(&root).map_err(gix_err("git.open_failed"))?;
            let sha = repo.head_id().map_err(gix_err("git.head"))?.to_string();
            changed(ctx, project.id, &root);
            Ok(CommitOut { sha })
        },
    );
    // `git fetch` is the network; it must not be holding the store while it waits (D144).
    e.register_staged::<Fetch, _>(
        |ctx, p| {
            let project = ctx.read(|conn| get_project(conn, p.project_id))?;
            worktree::git_mutate(Path::new(&project.path), &["fetch", "--all", "--prune"])
                .map_err(git_mutation("git.fetch_failed"))?;
            let repo = gix::open(&project.path).map_err(gix_err("git.open_failed"))?;
            let branch = repo
                .head_name()
                .ok()
                .flatten()
                .map(|n| n.shorten().to_string())
                .unwrap_or_default();
            let (_, ahead, behind) = upstream_metrics(&repo, &branch);
            Ok((project, ahead, behind))
        },
        |ctx: &mut Ctx, _p, (project, ahead, behind)| {
            changed(ctx, project.id, Path::new(&project.path));
            Ok(FetchOut { ahead, behind })
        },
    );
    // Same for `git push`: the whole op is network, and none of it needs the store.
    e.register_staged::<Push, _>(
        |ctx, p| {
            let (project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
            let repo = gix::open(&root).map_err(gix_err("git.open_failed"))?;
            let branch = repo
                .head_name()
                .ok()
                .flatten()
                .map(|n| n.shorten().to_string())
                .ok_or_else(|| BusError::conflict("git.detached", "cannot push a detached HEAD"))?;
            let has_upstream = upstream_metrics(&repo, &branch).0.is_some();
            let set_upstream = p.set_upstream.unwrap_or(!has_upstream);
            let args = if set_upstream {
                vec!["push", "-u", "origin", branch.as_str()]
            } else {
                vec!["push"]
            };
            worktree::git_mutate(&root, &args).map_err(git_mutation("git.push_failed"))?;
            Ok((project, root))
        },
        |ctx: &mut Ctx, _p, (project, root)| {
            changed(ctx, project.id, &root);
            Ok(relay_bus::Empty {})
        },
    );
    e.register_unlocked::<PrList>(|ctx, p| {
        let project = ctx.read(|conn| get_project(conn, p.project_id))?;
        let gh = crate::github::gh_path()?;
        let mut command = std::process::Command::new(gh);
        command.current_dir(&project.path).args([
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,headRefName,isDraft,url,title",
        ]);
        // `gh` talks to github.com. An unbounded wait on a flaky network used to be an unbounded
        // wait for every other bus op behind it; the lock is gone now, but the caller still gets
        // an answer either way (D144).
        let out = crate::proc::output_with_timeout(&mut command, PR_LIST_TIMEOUT)
            .map_err(|error| BusError::unavailable("git.pr_list_failed", error.to_string()))?
            .ok_or_else(|| {
                BusError::unavailable(
                    "git.pr_list_timeout",
                    format!("gh did not answer within {}s", PR_LIST_TIMEOUT.as_secs()),
                )
            })?;
        if !out.status.success() {
            return Err(BusError::unavailable(
                "git.pr_list_failed",
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        Ok(PrListOut {
            pull_requests: decode_pull_requests(&out.stdout)?,
        })
    });
    e.register::<PrOpen>(|ctx: &mut Ctx, p| {
        let (project, root) = resolve_root(ctx, p.project_id, p.worktree.as_deref())?;
        let gh = crate::github::gh_path()?;
        let mut cmd = std::process::Command::new(gh);
        cmd.current_dir(&root).args(["pr", "create"]);
        if let Some(title) = &p.title {
            cmd.args(["--title", title]);
        }
        if let Some(body) = &p.body {
            cmd.args(["--body", body]);
        } else {
            cmd.arg("--fill");
        }
        let out = cmd
            .output()
            .map_err(|e| BusError::unavailable("git.gh_unavailable", e.to_string()))?;
        if !out.status.success() {
            return Err(BusError::conflict(
                "git.pr_failed",
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        changed(ctx, project.id, &root);
        Ok(PrOpenOut {
            url: String::from_utf8_lossy(&out.stdout).trim().to_string(),
        })
    });
    e.register::<CleanMerged>(|ctx: &mut Ctx, p| {
        let project = get_project(ctx.tx(), p.project_id)?;
        let branches = branches_for(ctx.tx(), &project)?;
        let candidates: Vec<String> = branches
            .branches
            .into_iter()
            .filter(|b| {
                b.merged && !b.current && b.session.is_none() && b.name != project.base_branch
            })
            .map(|b| b.name)
            .collect();
        if !p.dry_run.unwrap_or(false) {
            for branch in &candidates {
                worktree::git_mutate(Path::new(&project.path), &["branch", "-d", branch])
                    .map_err(git_mutation("git.branch_delete_failed"))?;
            }
        }
        changed(ctx, project.id, Path::new(&project.path));
        Ok(CleanMergedOut {
            deleted: candidates,
        })
    });
    e.register_unlocked::<SuggestMessage>(|ctx, p| {
        let (_project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
        let files = status_files(&root)?;
        let message = match files.as_slice() {
            [] => "No changes".into(),
            [one] => format!("Update {}", one.path),
            many => format!("Update {} files", many.len()),
        };
        Ok(SuggestOut { message })
    });
}

pub fn status_badges(root: &Path) -> Result<HashMap<String, String>, BusError> {
    Ok(status_files(root)?
        .into_iter()
        .map(|s| {
            let badge = if !s.worktree.is_empty() {
                s.worktree.clone()
            } else {
                s.index.clone()
            };
            (s.path, badge)
        })
        .collect())
}

fn status_files(root: &Path) -> Result<Vec<FileStatus>, BusError> {
    let repo = gix::open(root).map_err(gix_err("git.open_failed"))?;
    let platform = repo
        .status(gix::progress::Discard)
        .map_err(gix_err("git.status_failed"))?
        .untracked_files(gix::status::UntrackedFiles::Files);
    let iter = platform
        .into_iter(Vec::<gix::bstr::BString>::new())
        .map_err(gix_err("git.status_failed"))?;
    let mut files: BTreeMap<String, FileStatus> = BTreeMap::new();
    for item in iter {
        let item = item.map_err(gix_err("git.status_failed"))?;
        match item {
            gix::status::Item::IndexWorktree(change) => {
                let path = change.rela_path().to_string();
                let summary = format!("{:?}", change.summary());
                let wt = if summary.contains("Added") {
                    "?"
                } else if summary.contains("Removed") {
                    "D"
                } else if summary.contains("Conflict") {
                    "U"
                } else if summary.contains("Renamed") {
                    "R"
                } else {
                    "M"
                };
                let renamed_from = match &change {
                    gix::status::index_worktree::Item::Rewrite { source, .. } => {
                        Some(source.rela_path().to_string())
                    }
                    _ => None,
                };
                let row = files.entry(path.clone()).or_insert(FileStatus {
                    path,
                    index: String::new(),
                    worktree: String::new(),
                    renamed_from: None,
                });
                row.worktree = wt.into();
                row.renamed_from = renamed_from;
            }
            gix::status::Item::TreeIndex(change) => {
                let path = change.location().to_string();
                let (idx, renamed_from) = match &change {
                    gix::diff::index::Change::Addition { .. } => ("A", None),
                    gix::diff::index::Change::Deletion { .. } => ("D", None),
                    gix::diff::index::Change::Modification { .. } => ("M", None),
                    gix::diff::index::Change::Rewrite {
                        source_location, ..
                    } => ("R", Some(source_location.to_string())),
                };
                let row = files.entry(path.clone()).or_insert(FileStatus {
                    path,
                    index: String::new(),
                    worktree: String::new(),
                    renamed_from: None,
                });
                row.index = idx.into();
                row.renamed_from = renamed_from;
            }
        }
    }
    Ok(files.into_values().collect())
}

fn resolve_root(
    ctx: &Ctx,
    project_id: relay_bus::types::Id,
    requested: Option<&str>,
) -> Result<(relay_bus::types::Project, PathBuf), BusError> {
    let (project, chosen) = crate::handlers::file::root_choice(
        ctx.tx(),
        ctx.actor_session_id(),
        project_id,
        requested,
    )?;
    let chosen = crate::handlers::file::root_verify(&project, chosen, "git.worktree")?;
    Ok((project, chosen))
}

/// [`resolve_root`] for the unlocked query context: one short read for the project row and the
/// session's worktree, then `git worktree list` with the store lock released (D144).
fn resolve_root_unlocked(
    ctx: &Unlocked,
    project_id: relay_bus::types::Id,
    requested: Option<&str>,
) -> Result<(relay_bus::types::Project, PathBuf), BusError> {
    let session_id = ctx.actor_session_id();
    let (project, chosen) = ctx.read(|conn| {
        crate::handlers::file::root_choice(conn, session_id, project_id, requested)
    })?;
    let chosen = crate::handlers::file::root_verify(&project, chosen, "git.worktree")?;
    Ok((project, chosen))
}

fn validate_path(path: &str) -> Result<(), BusError> {
    let p = Path::new(path);
    if path.is_empty()
        || p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        Err(BusError::invalid(
            "git.path",
            "path must be worktree-relative and contain no ..",
        ))
    } else {
        Ok(())
    }
}

fn revision_text(root: &Path, revision: &str, path: &str) -> Result<String, BusError> {
    let repo = gix::open(root).map_err(gix_err("git.open_failed"))?;
    let id = match repo.rev_parse_single(revision) {
        Ok(id) => id,
        Err(_) if revision == "HEAD" => return Ok(String::new()),
        Err(error) => return Err(BusError::unavailable("git.revision", error.to_string())),
    };
    let tree = id
        .object()
        .map_err(gix_err("git.object"))?
        .peel_to_commit()
        .map_err(gix_err("git.object"))?
        .tree()
        .map_err(gix_err("git.object"))?;
    let Some(entry) = tree
        .lookup_entry_by_path(path)
        .map_err(gix_err("git.object"))?
    else {
        return Ok(String::new());
    };
    let object = entry.object().map_err(gix_err("git.object"))?;
    let Ok(blob) = object.try_into_blob() else {
        return Ok(String::new());
    };
    Ok(String::from_utf8_lossy(&blob.data).to_string())
}

fn index_text(root: &Path, path: &str) -> Result<String, BusError> {
    use gix::bstr::ByteSlice;
    let repo = gix::open(root).map_err(gix_err("git.open_failed"))?;
    let index = repo.index_or_empty().map_err(gix_err("git.index"))?;
    let Some(entry) = index.entry_by_path(path.as_bytes().as_bstr()) else {
        return Ok(String::new());
    };
    let obj = repo.find_object(entry.id).map_err(gix_err("git.object"))?;
    let Ok(blob) = obj.try_into_blob() else {
        return Ok(String::new());
    };
    Ok(String::from_utf8_lossy(&blob.data).to_string())
}
fn working_text(root: &Path, path: &str) -> Result<String, BusError> {
    validate_path(path)?;
    match std::fs::read(root.join(path)) {
        Ok(v) => Ok(String::from_utf8_lossy(&v).to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(BusError::unavailable("git.read_failed", e.to_string())),
    }
}
fn counts(old: &str, new: &str) -> (i64, i64) {
    let (mut a, mut r) = (0, 0);
    for c in TextDiff::from_lines(old, new).iter_all_changes() {
        match c.tag() {
            ChangeTag::Insert => a += 1,
            ChangeTag::Delete => r += 1,
            _ => {}
        }
    }
    (a, r)
}
fn staged_numstat(root: &Path) -> Result<String, BusError> {
    let mut out = String::new();
    for s in status_files(root)? {
        if s.index.is_empty() {
            continue;
        }
        let old = revision_text(root, "HEAD", &s.path)?;
        let new = index_text(root, &s.path)?;
        let (a, r) = counts(&old, &new);
        out.push_str(&format!("{a}\t{r}\t{}\n", s.path));
    }
    Ok(out)
}

fn commit_from_gix(c: gix::Commit<'_>) -> Result<Commit, BusError> {
    let author = c.author().map_err(gix_err("git.commit_decode"))?;
    let message = c.message().map_err(gix_err("git.commit_decode"))?;
    let title = message.title.to_string().trim_end().to_string();
    let body = message
        .body
        .map(|b| b.to_string().trim_end().to_string())
        .unwrap_or_default();
    let at = jiff::Timestamp::from_second(author.seconds())
        .map_err(|e| BusError::internal(e.to_string()))?
        .to_string();
    Ok(Commit {
        sha: c.id.to_string(),
        parents: c.parent_ids().map(|p| p.to_string()).collect(),
        author: author.name.to_string(),
        email: author.email.to_string(),
        at,
        subject: title,
        body,
        refs: Vec::new(),
    })
}
fn upstream_metrics(repo: &gix::Repository, branch: &str) -> (Option<String>, i64, i64) {
    let Ok(reference) = repo.find_reference(branch) else {
        return (None, 0, 0);
    };
    let Some(Ok(name)) = reference.remote_tracking_ref_name(gix::remote::Direction::Fetch) else {
        return (None, 0, 0);
    };
    let display = name
        .as_bstr()
        .to_string()
        .trim_start_matches("refs/remotes/")
        .to_string();
    let Ok(upstream) = repo.find_reference(name.as_bstr()) else {
        return (Some(display), 0, 0);
    };
    let local = reference.id().detach();
    let remote = upstream.id().detach();
    let ahead = repo
        .rev_walk([local])
        .with_hidden([remote])
        .all()
        .map(|w| w.filter(Result::is_ok).count() as i64)
        .unwrap_or(0);
    let behind = repo
        .rev_walk([remote])
        .with_hidden([local])
        .all()
        .map(|w| w.filter(Result::is_ok).count() as i64)
        .unwrap_or(0);
    (Some(display), ahead, behind)
}

#[derive(Deserialize)]
struct GhPullRequest {
    number: u64,
    #[serde(rename = "headRefName")]
    branch: String,
    #[serde(rename = "isDraft")]
    draft: bool,
    url: String,
    title: String,
}

fn decode_pull_requests(bytes: &[u8]) -> Result<Vec<PullRequest>, BusError> {
    let rows: Vec<GhPullRequest> = serde_json::from_slice(bytes)
        .map_err(|error| BusError::internal(format!("decoding GitHub pull requests: {error}")))?;
    Ok(rows
        .into_iter()
        .map(|row| PullRequest {
            number: row.number,
            branch: row.branch,
            draft: row.draft,
            url: row.url,
            title: row.title,
        })
        .collect())
}

fn commit_diff_files(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
) -> Result<Vec<DiffFile>, BusError> {
    let new_tree = commit.tree().map_err(gix_err("git.show_failed"))?;
    let old_tree = match commit.parent_ids().next() {
        Some(parent) => parent
            .object()
            .map_err(gix_err("git.show_failed"))?
            .into_commit()
            .tree()
            .map_err(gix_err("git.show_failed"))?,
        None => repo.empty_tree(),
    };
    tree_diff_files(repo, &old_tree, &new_tree, "git.show_failed")
}

fn tree_diff_files(
    repo: &gix::Repository,
    old_tree: &gix::Tree<'_>,
    new_tree: &gix::Tree<'_>,
    code: &'static str,
) -> Result<Vec<DiffFile>, BusError> {
    let mut files = Vec::new();
    old_tree
        .changes()
        .map_err(gix_err(code))?
        .for_each_to_obtain_tree(new_tree, |change| {
            use gix::object::tree::diff::Change;
            let (path, old_path, status, old, new) = match change {
                Change::Addition { location, id, .. } => (
                    location.to_string(),
                    None,
                    "A",
                    String::new(),
                    repo.find_object(id.detach())
                        .ok()
                        .and_then(|o| o.try_into_blob().ok())
                        .map(|b| String::from_utf8_lossy(&b.data).to_string())
                        .unwrap_or_default(),
                ),
                Change::Deletion { location, id, .. } => (
                    location.to_string(),
                    None,
                    "D",
                    repo.find_object(id.detach())
                        .ok()
                        .and_then(|o| o.try_into_blob().ok())
                        .map(|b| String::from_utf8_lossy(&b.data).to_string())
                        .unwrap_or_default(),
                    String::new(),
                ),
                Change::Modification {
                    location,
                    previous_id,
                    id,
                    ..
                } => (
                    location.to_string(),
                    None,
                    "M",
                    repo.find_object(previous_id.detach())
                        .ok()
                        .and_then(|o| o.try_into_blob().ok())
                        .map(|b| String::from_utf8_lossy(&b.data).to_string())
                        .unwrap_or_default(),
                    repo.find_object(id.detach())
                        .ok()
                        .and_then(|o| o.try_into_blob().ok())
                        .map(|b| String::from_utf8_lossy(&b.data).to_string())
                        .unwrap_or_default(),
                ),
                Change::Rewrite {
                    source_location,
                    location,
                    source_id,
                    id,
                    ..
                } => (
                    location.to_string(),
                    Some(source_location.to_string()),
                    "R",
                    repo.find_object(source_id.detach())
                        .ok()
                        .and_then(|o| o.try_into_blob().ok())
                        .map(|b| String::from_utf8_lossy(&b.data).to_string())
                        .unwrap_or_default(),
                    repo.find_object(id.detach())
                        .ok()
                        .and_then(|o| o.try_into_blob().ok())
                        .map(|b| String::from_utf8_lossy(&b.data).to_string())
                        .unwrap_or_default(),
                ),
            };
            if !path.is_empty() {
                let (added, removed) = counts(&old, &new);
                files.push(DiffFile {
                    path,
                    old_path,
                    status: status.into(),
                    added,
                    removed,
                    binary: old.contains('\0') || new.contains('\0'),
                });
            }
            Ok::<_, std::io::Error>(std::ops::ControlFlow::Continue(()))
        })
        .map_err(gix_err(code))?;
    Ok(files)
}
fn changed(ctx: &mut Ctx, project_id: relay_bus::types::Id, root: &Path) {
    ctx.set_project(project_id);
    ctx.emit(
        "git.changed",
        json!({"project_id":project_id,"worktree":root}),
    );
}
fn validate_branch_name(root: &Path, value: &str) -> Result<String, BusError> {
    let name = value.trim();
    if name.is_empty() {
        return Err(BusError::invalid("git.branch_name", "branch name cannot be empty"));
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ref-format", "--branch", name])
        .output()
        .map_err(|e| BusError::unavailable("git.unavailable", e.to_string()))?;
    if !out.status.success() {
        return Err(BusError::invalid(
            "git.branch_name",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(name.to_string())
}
fn git_mutation(code: &'static str) -> impl Fn(anyhow::Error) -> BusError {
    move |e| BusError::conflict(code, e.to_string())
}
fn gix_err<E: std::fmt::Display>(code: &'static str) -> impl Fn(E) -> BusError {
    move |e| BusError::unavailable(code, e.to_string())
}

fn branches_for(
    tx: &Transaction,
    project: &relay_bus::types::Project,
) -> Result<BranchesOut, BusError> {
    let repo = gix::open(&project.path).map_err(gix_err("git.open_failed"))?;
    let current = repo
        .head_name()
        .ok()
        .flatten()
        .map(|n| n.shorten().to_string())
        .unwrap_or_default();
    let head = repo
        .rev_parse_single(project.base_branch.as_str())
        .ok()
        .map(|id| id.detach());
    let mut owners = HashMap::new();
    let mut st = tx
        .prepare_cached("SELECT branch,name FROM sessions WHERE project_id=?1 AND state!='closed'")
        .bus()?;
    for row in st
        .query_map([project.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .bus()?
    {
        let (b, n) = row.bus()?;
        owners.insert(b, n);
    }
    let mut branches = Vec::new();
    let platform = repo.references().map_err(gix_err("git.branches_failed"))?;
    let refs = platform
        .local_branches()
        .map_err(gix_err("git.branches_failed"))?;
    for reference in refs {
        let reference = reference.map_err(gix_err("git.branches_failed"))?;
        let name = reference
            .name()
            .as_bstr()
            .to_string()
            .trim_start_matches("refs/heads/")
            .to_string();
        let Some(id) = reference.try_id() else {
            continue;
        };
        let merged = head
            .as_ref()
            .and_then(|h| repo.merge_base(*h, id.detach()).ok())
            .map(|base| base.detach() == id.detach())
            .unwrap_or(false);
        let (upstream, ahead, behind) = upstream_metrics(&repo, &name);
        branches.push(Branch {
            name: name.clone(),
            head: id.to_string(),
            upstream,
            ahead,
            behind,
            merged,
            session: owners.get(&name).cloned(),
            current: name == current,
        });
    }
    Ok(BranchesOut { current, branches })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_pull_request_rows_decode_to_bus_shape() {
        let rows = decode_pull_requests(
            br#"[{"number":8,"headRefName":"relay/spry-gecko","isDraft":false,"url":"https://github.com/antho976/Relay-2/pull/8","title":"Launch sheet"}]"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].number, 8);
        assert_eq!(rows[0].branch, "relay/spry-gecko");
        assert!(!rows[0].draft);
    }
}
