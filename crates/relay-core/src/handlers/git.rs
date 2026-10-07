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
use std::collections::HashMap;
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
        .prepare_cached(
            "SELECT name, worktree FROM sessions WHERE project_id = ?1 AND state != 'closed'",
        )
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

/// How long the paginated GitHub lookup may take before Relay stops waiting on the network.
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
            let from = if p.from.is_none() && !existing_worktree_branch(repo, Some(&p.branch))? {
                refresh_new_worktree(repo, Some(&p.branch))?;
                new_worktree_base(repo, &project.base_branch)?
            } else {
                p.from.clone()
            };
            let wt = worktree::create(repo, &path, &p.branch, from.as_deref())
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
        // A relative path would resolve against the engine's cwd ($HOME under `relay serve`).
        if !Path::new(&p.path).is_absolute() {
            return Err(BusError::invalid("worktree.path", format!("{:?} must be an absolute path", p.path)));
        }
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
        let wts = worktree::list_with_dirty(Path::new(&project.path), false)
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
        let remote_branches = remote_branches(&repo, &platform, &branches)?;
        Ok(BranchesOut { current, branches, remote_branches })
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
        let start = p
            .start_point
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(revision) = start {
            repo.rev_parse_single(revision).map_err(|_| {
                BusError::invalid("git.start_point", format!("unknown start point {revision}"))
            })?;
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
        Ok(BranchCreateOut {
            name,
            head,
            worktree: root.display().to_string(),
        })
    });
    // `git status` and `git switch` both walk the checkout; neither needs the store (D144).
    e.register_staged::<BranchSwitch, _>(
        |ctx, p| {
            let (project, root) = resolve_root_unlocked(ctx, p.project_id, p.worktree.as_deref())?;
            let checkout = root.to_string_lossy().to_string();
            let owner = ctx.read(|conn| {
                let mut st = conn
                    .prepare_cached("SELECT name FROM sessions WHERE project_id=?1 AND worktree=?2 AND state!='closed' LIMIT 1")
                    .bus()?;
                let mut rows = st.query(rusqlite::params![project.id, checkout]).bus()?;
                rows.next().bus()?.map(|row| row.get::<_, String>(0)).transpose().bus()
            })?;
            if let Some(session) = owner {
                return Err(BusError::conflict(
                    "git.checkout_session_owned",
                    format!("This checkout belongs to the live session {session}; its agent decides its branch. Select another checkout, or end the session first."),
                ));
            }
            let target = switch_target(&root, &p.name)?;
            let current = gix::open(&root)
                .ok()
                .and_then(|repo| repo.head_name().ok().flatten().map(|n| n.shorten().to_string()));
            if current.as_deref() == Some(target.local.as_str()) {
                return Ok((project, root, target.local, false));
            }
            // Untracked files travel with any switch and git refuses one that would overwrite
            // them, so only tracked changes count as a dirty checkout.
            let tracked: Vec<_> = status_files(&root)?
                .into_iter()
                .filter(|file| !(file.index.is_empty() && file.worktree == "?"))
                .collect();
            if tracked.iter().any(|file| file.index == "U" || file.worktree == "U") {
                return Err(BusError::conflict(
                    "git.checkout_unmerged",
                    "This checkout is in the middle of a merge or rebase with unresolved conflicts; resolve them before switching branches",
                ));
            }
            if !tracked.is_empty() && !p.carry_changes.unwrap_or(false) {
                return Err(BusError::conflict(
                    "git.checkout_dirty",
                    format!(
                        "{} uncommitted change{} ({}). Commit them first, or bring them along to {}.",
                        tracked.len(),
                        if tracked.len() == 1 { "" } else { "s" },
                        sample_paths(tracked.iter().map(|file| file.path.as_str())),
                        target.local
                    ),
                ));
            }
            let args: Vec<&str> = match &target.remote {
                Some(remote) => vec!["switch", "--no-overwrite-ignore", "--track", "-c", &target.local, remote],
                None => vec!["switch", "--no-overwrite-ignore", "--", &target.local],
            };
            run_switch(&root, &args, &target.local)?;
            Ok((project, root, target.local, target.remote.is_some()))
        },
        |ctx: &mut Ctx, _p, (project, root, branch, created)| {
            changed(ctx, project.id, &root);
            Ok(BranchSwitchOut { branch, created })
        },
    );
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
            .ok_or_else(|| {
                BusError::not_found("git.branch_not_found", format!("no local branch {name}"))
            })?;
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
        if worktree::list_with_dirty(root, false)
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
            fetch_remote(Path::new(&project.path))?;
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
        let listed = list_pull_requests(&gh, Path::new(&project.path))?;
        // A PR merged for a branch a closed session left behind: clean that branch up now
        // rather than at the next sweep (branch_cleanup).
        let merged: Vec<String> = listed.pull_requests.iter()
            .filter(|pr| pr.state == "merged" && pr.same_repository).map(|pr| pr.branch.clone()).collect();
        if !merged.is_empty() {
            let leftover: Vec<String> = ctx.read(|conn| {
                let (candidates, _) = crate::branch_cleanup::candidates(conn, Some(project.id), Some(&merged))?;
                Ok(candidates.into_iter().map(|candidate| candidate.branch).collect())
            })?;
            if !leftover.is_empty() && ctx.engine().instance != crate::Instance::Test {
                let project_id = project.id;
                ctx.after_commit(move |engine| crate::branch_cleanup::after_merged_prs(engine, project_id, leftover));
            }
        }
        Ok(listed)
    });
    // Every git and gh call is a subprocess; the transaction only attributes the result (D149).
    e.register_staged::<BranchCleanup, _>(
        |ctx, p| {
            let project = ctx.read(|conn| get_project(conn, p.project_id))?;
            let options = crate::branch_cleanup::Options {
                dry_run: p.dry_run.unwrap_or(false),
                gh: crate::branch_cleanup::gh(),
                audit_kept: false,
                use_gh_cache: false,
            };
            let rows = crate::branch_cleanup::run(ctx.engine(), Some(project.id), None, &options)?;
            Ok((project, rows))
        },
        |ctx: &mut Ctx, _p, (project, rows)| {
            changed(ctx, project.id, Path::new(&project.path));
            if rows.iter().any(|row| row.removed_worktree) {
                ctx.emit("worktree.changed", json!({ "project_id": project.id }));
            }
            Ok(BranchCleanupOut { branches: rows })
        },
    );
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
        let checked_out = worktree::list_with_dirty(Path::new(&project.path), false)
            .map_err(|error| BusError::unavailable("worktree.list_failed", error.to_string()))?;
        let candidates: Vec<String> = branches
            .branches
            .into_iter()
            .filter(|b| {
                b.merged
                    && !b.current
                    && b.session.is_none()
                    && b.name != project.base_branch
                    && !checked_out.iter().any(|worktree| worktree.branch == b.name)
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
    worktree::status_files(root).map_err(git_mutation("git.status_failed"))
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
    let (project, chosen) = ctx
        .read(|conn| crate::handlers::file::root_choice(conn, session_id, project_id, requested))?;
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
fn upstream_metrics(
    repo: &gix::Repository,
    branch: &str,
) -> (Option<String>, Option<i64>, Option<i64>) {
    let Ok(reference) = repo.find_reference(branch) else {
        return (None, None, None);
    };
    let Some(Ok(name)) = reference.remote_tracking_ref_name(gix::remote::Direction::Fetch) else {
        return (None, None, None);
    };
    let display = name
        .as_bstr()
        .to_string()
        .trim_start_matches("refs/remotes/")
        .to_string();
    let Ok(upstream) = repo.find_reference(name.as_bstr()) else {
        return (Some(display), None, None);
    };
    let (Some(local), Some(remote)) = (reference.try_id(), upstream.try_id()) else {
        return (Some(display), None, None);
    };
    let counts = divergence(repo, local.detach(), remote.detach());
    (Some(display), counts.map(|v| v.0), counts.map(|v| v.1))
}

fn divergence(
    repo: &gix::Repository,
    local: gix::ObjectId,
    remote: gix::ObjectId,
) -> Option<(i64, i64)> {
    let count = |from, hidden| {
        repo.rev_walk([from])
            .with_hidden([hidden])
            .all()
            .ok()?
            .try_fold(0_i64, |count, item| item.ok().map(|_| count + 1))
    };
    Some((count(local, remote)?, count(remote, local)?))
}

/// Bounded network preflight, called outside the store transaction.
pub fn fetch_remote(root: &Path) -> Result<(), BusError> {
    let mut command = std::process::Command::new("git");
    command
        .current_dir(root)
        .args(["fetch", "--all", "--prune"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    let output = crate::proc::output_with_timeout(&mut command, std::time::Duration::from_secs(20))
        .map_err(|error| BusError::unavailable("git.fetch_failed", error.to_string()))?
        .ok_or_else(|| {
            BusError::unavailable(
                "git.fetch_timeout",
                "Fetch timed out; cached refs are still available",
            )
        })?;
    if !output.status.success() {
        return Err(BusError::unavailable(
            "git.fetch_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

pub(super) fn existing_worktree_branch(root: &Path, branch: Option<&str>) -> Result<bool, BusError> {
    let repo = gix::open(root).map_err(gix_err("git.open_failed"))?;
    Ok(branch.is_some_and(|branch| repo.find_reference(format!("refs/heads/{branch}").as_str()).is_ok()))
}

/// Only new branches need refreshing. Reattaching an existing branch preserves its work,
/// including offline sessions. A failed fetch must never silently launch from stale refs.
pub(super) fn refresh_new_worktree(root: &Path, branch: Option<&str>) -> Result<(), BusError> {
    if !existing_worktree_branch(root, branch)? {
        fetch_remote(root)?;
    }
    Ok(())
}

/// Pin the freshest base commit without moving the primary checkout. Preserve local-only
/// commits; refuse divergent histories instead of silently omitting remote changes.
pub fn new_worktree_base(root: &Path, base: &str) -> Result<Option<String>, BusError> {
    let repo = gix::open(root).map_err(gix_err("git.open_failed"))?;
    let local_name = format!("refs/heads/{}", base.trim_start_matches("refs/heads/"));
    let local = repo.find_reference(local_name.as_str()).ok();
    let upstream = local.as_ref().and_then(|reference| {
        reference.remote_tracking_ref_name(gix::remote::Direction::Fetch)
    }).transpose().map_err(gix_err("git.base_invalid"))?;
    let remote_name = upstream.map(|name| name.as_bstr().to_string()).or_else(|| {
        let name = format!("refs/remotes/origin/{}", base.trim_start_matches("refs/heads/"));
        repo.find_reference(name.as_str()).ok().map(|_| name)
    });
    let remote = remote_name.as_ref().map(|name| {
        repo.rev_parse_single(name.as_str()).map(|id| id.detach())
            .map_err(gix_err("git.base_upstream_missing"))
    }).transpose()?;
    let local = local.and_then(|reference| reference.try_id().map(|id| id.detach()));
    // A repository before its first commit has a valid unborn base, but no commit to pin.
    if local.is_none() && remote.is_none()
        && repo.head().is_ok_and(|head| head.is_unborn())
        && repo.head_name().ok().flatten().is_some_and(|name| *name.as_bstr() == local_name)
    {
        return Ok(None);
    }
    let selected = match (local, remote) {
        (Some(local), Some(remote)) if local != remote => {
            match repo.merge_base(local, remote).ok().map(|id| id.detach()) {
                Some(common) if common == local => remote,
                Some(common) if common == remote => local,
                _ => return Err(BusError::conflict("git.base_diverged", format!(
                    "Base branch {base} and its upstream have diverged; reconcile them before creating a new agent branch"
                ))),
            }
        }
        (Some(id), _) | (_, Some(id)) => id,
        _ => return Err(BusError::conflict("git.base_missing", format!("Base branch {base:?} does not exist"))),
    };
    Ok(Some(selected.to_string()))
}

/// Cached Git context only: generating a brief must never make a network request.
pub fn briefing_state(root: &Path, branch: &str, base: &str) -> String {
    let Ok(repo) = gix::open(root) else {
        return "git: unavailable".into();
    };
    let (upstream, ahead, behind) = upstream_metrics(&repo, branch);
    let (base_upstream, base_ahead, base_behind) = upstream_metrics(&repo, base);
    let count = |value: Option<i64>| {
        value
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unknown".into())
    };
    let base_counts = repo.rev_parse_single(branch).ok().and_then(|local| {
        repo.rev_parse_single(base)
            .ok()
            .and_then(|remote| divergence(&repo, local.detach(), remote.detach()))
    });
    let fetched = std::fs::metadata(repo.common_dir().join("FETCH_HEAD"))
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.elapsed().ok())
        .map(|age| {
            format!(
                "FETCH_HEAD updated {}s ago; cached snapshot, not proof of a successful fetch",
                age.as_secs()
            )
        })
        .unwrap_or_else(|| "unknown; offline cached refs only".into());
    format!(
        "git_upstream: {}\ngit_ahead: {}\ngit_behind: {}\ngit_base: {}\ngit_base_ahead: {}\ngit_base_behind: {}\ngit_base_upstream: {}\ngit_base_upstream_ahead: {}\ngit_base_upstream_behind: {}\ngit_last_fetch: {}",
        upstream.as_deref().unwrap_or("none"),
        count(ahead),
        count(behind),
        base,
        count(base_counts.map(|v| v.0)),
        count(base_counts.map(|v| v.1)),
        base_upstream.as_deref().unwrap_or("none"),
        count(base_ahead),
        count(base_behind),
        fetched
    )
}

#[derive(Deserialize)]
struct GhPullRequest {
    number: u64,
    head: GhBranch,
    base: GhBranch,
    #[serde(default)]
    draft: bool,
    html_url: String,
    title: String,
    state: String,
    merged_at: Option<String>,
}
#[derive(Deserialize)]
struct GhBranch {
    #[serde(rename = "ref")]
    branch: String,
    repo: Option<GhRepo>,
}
#[derive(Deserialize)]
struct GhRepo {
    full_name: String,
}

fn list_pull_requests(gh: &Path, root: &Path) -> Result<PrListOut, BusError> {
    let mut command = std::process::Command::new(gh);
    command.current_dir(root).args([
        "api",
        "--paginate",
        "--slurp",
        "repos/{owner}/{repo}/pulls?state=all&per_page=100&sort=updated&direction=desc",
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
        complete: true,
    })
}

fn decode_pull_requests(bytes: &[u8]) -> Result<Vec<PullRequest>, BusError> {
    let pages: Vec<Vec<GhPullRequest>> = serde_json::from_slice(bytes)
        .map_err(|error| BusError::internal(format!("decoding GitHub pull requests: {error}")))?;
    Ok(pages
        .into_iter()
        .flatten()
        .map(|row| {
            let same_repository = row
                .head
                .repo
                .as_ref()
                .zip(row.base.repo.as_ref())
                .is_some_and(|(head, base)| head.full_name == base.full_name);
            PullRequest {
                number: row.number,
                branch: row.head.branch,
                draft: row.draft,
                url: row.html_url,
                title: row.title,
                state: if row.merged_at.is_some() {
                    "merged".into()
                } else {
                    row.state
                },
                same_repository,
            }
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
        return Err(BusError::invalid(
            "git.branch_name",
            "branch name cannot be empty",
        ));
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
/// The tips among `tips` that the base's ancestry reaches, from a single walk that ends as soon
/// as the last of them is found — or when history runs out, which is the cost `git branch
/// --merged` pays too.
fn merged_tips(
    repo: &gix::Repository,
    head: Option<gix::ObjectId>,
    tips: impl IntoIterator<Item = gix::ObjectId>,
) -> std::collections::HashSet<gix::ObjectId> {
    let mut pending: std::collections::HashSet<gix::ObjectId> = tips.into_iter().collect();
    let mut merged = std::collections::HashSet::new();
    let Some(head) = head else { return merged };
    let Ok(walk) = repo.rev_walk([head]).all() else { return merged };
    for info in walk {
        let Ok(info) = info else { break };
        if pending.remove(&info.id) {
            merged.insert(info.id);
        }
        if pending.is_empty() {
            break;
        }
    }
    merged
}

/// What `git.branch.switch` checks out: a local branch, or a new local tracking branch for a
/// remote one.
struct SwitchTarget {
    local: String,
    remote: Option<String>,
}

/// A local branch of that name wins; otherwise `origin/feature` names a remote-tracking branch,
/// checked out as `feature` — or as the existing local `feature`, if there is one.
fn switch_target(root: &Path, requested: &str) -> Result<SwitchTarget, BusError> {
    let name = requested.trim();
    let repo = gix::open(root).map_err(gix_err("git.open_failed"))?;
    if repo.find_reference(format!("refs/heads/{name}").as_str()).is_ok() {
        return Ok(SwitchTarget { local: validate_branch_name(root, name)?, remote: None });
    }
    if repo.find_reference(format!("refs/remotes/{name}").as_str()).is_ok() {
        let branch = repo
            .remote_names()
            .iter()
            .map(|remote| remote.to_string())
            .filter_map(|remote| name.strip_prefix(&format!("{remote}/")).map(str::to_owned))
            .min_by_key(String::len)
            .or_else(|| name.split_once('/').map(|(_, rest)| rest.to_owned()))
            .filter(|branch| !branch.is_empty() && branch != "HEAD")
            .ok_or_else(|| BusError::invalid("git.branch_name", format!("{name} is not a remote branch")))?;
        let local = validate_branch_name(root, &branch)?;
        if repo.find_reference(format!("refs/heads/{local}").as_str()).is_ok() {
            return Ok(SwitchTarget { local, remote: None });
        }
        return Ok(SwitchTarget { local, remote: Some(name.to_owned()) });
    }
    validate_branch_name(root, name)?;
    Err(BusError::not_found("git.branch_not_found", format!("no local or remote branch {name}")))
}

/// `git switch`, bounded, with its refusals turned into errors a person can act on. Git never
/// discards anything on these paths: every refusal leaves the checkout as it was.
fn run_switch(root: &Path, args: &[&str], branch: &str) -> Result<(), BusError> {
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(root).args(args).env("GIT_TERMINAL_PROMPT", "0");
    let output = crate::proc::output_with_timeout(&mut command, std::time::Duration::from_secs(120))
        .map_err(|error| BusError::unavailable("git.branch_switch_failed", error.to_string()))?
        .ok_or_else(|| BusError::unavailable("git.branch_switch_failed", "git switch did not finish within 2 minutes"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let listed = || {
        sample_paths(
            stderr
                .lines()
                .filter(|line| line.starts_with('\t'))
                .map(str::trim),
        )
    };
    if let Some(at) = stderr
        .split("is already checked out at ")
        .nth(1)
        .or_else(|| stderr.split("is already used by worktree at ").nth(1))
    {
        let path = at.lines().next().unwrap_or("").trim().trim_matches(['\'', '"']);
        return Err(BusError::conflict(
            "git.branch_checked_out",
            format!("{branch} is already checked out in {path}. Select that checkout to work on it."),
        ));
    }
    if stderr.contains("local changes to the following files would be overwritten") {
        return Err(BusError::conflict(
            "git.checkout_conflict",
            format!("Your uncommitted changes to {} would be overwritten by {branch}. Commit them first; nothing was changed.", listed()),
        ));
    }
    if stderr.contains("untracked working tree files would be overwritten") {
        return Err(BusError::conflict(
            "git.branch_switch_failed",
            format!("{branch} has its own copy of {}, which is untracked or ignored here. Move it aside first; nothing was changed.", listed()),
        ));
    }
    Err(BusError::conflict("git.branch_switch_failed", format!("git {} failed: {stderr}", args.join(" "))))
}

/// Up to three paths, then a count, for one-line messages.
fn sample_paths<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    let paths: Vec<&str> = paths.collect();
    let shown = paths.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    match paths.len() {
        0 => String::from("some files"),
        n if n > 3 => format!("{shown} and {} more", n - 3),
        _ => shown,
    }
}

/// `refs/remotes/*` as remote branches, each linked to the local branch of the same name.
fn remote_branches(
    repo: &gix::Repository,
    platform: &gix::reference::iter::Platform<'_>,
    local: &[Branch],
) -> Result<Vec<RemoteBranch>, BusError> {
    let remotes: Vec<String> = repo.remote_names().iter().map(|r| r.to_string()).collect();
    let mut out = Vec::new();
    for reference in platform.remote_branches().map_err(gix_err("git.branches_failed"))? {
        let reference = reference.map_err(gix_err("git.branches_failed"))?;
        // Symbolic refs — `origin/HEAD` — point at a branch already listed.
        let Some(id) = reference.try_id() else {
            continue;
        };
        let name = reference.name().as_bstr().to_string();
        let Some(name) = name.strip_prefix("refs/remotes/").map(str::to_owned) else {
            continue;
        };
        let (remote, branch) = remotes
            .iter()
            .filter_map(|remote| name.strip_prefix(&format!("{remote}/")).map(|b| (remote.clone(), b.to_owned())))
            .min_by_key(|(_, branch)| branch.len())
            .or_else(|| name.split_once('/').map(|(r, b)| (r.to_owned(), b.to_owned())))
            .unwrap_or_else(|| (String::new(), name.clone()));
        if branch == "HEAD" {
            continue;
        }
        out.push(RemoteBranch {
            local: local.iter().find(|b| b.name == branch).map(|b| b.name.clone()),
            head: id.to_string(),
            name,
            remote,
            branch,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn gix_err<E: std::fmt::Display>(code: &'static str) -> impl Fn(E) -> BusError {
    move |e| BusError::unavailable(code, e.to_string())
}

fn branches_for(
    tx: &Transaction,
    project: &relay_bus::types::Project,
) -> Result<BranchesOut, BusError> {
    let mut repo = gix::open(&project.path).map_err(gix_err("git.open_failed"))?;
    // Every walk below decompresses commits; without a cache each branch paid for its own.
    repo.object_cache_size_if_unset(16 * 1024 * 1024);
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
    let mut tips = Vec::new();
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
        if let Some(id) = reference.try_id() {
            tips.push((name, id.detach()));
        }
    }
    // One walk from the base, stopping once every tip has been seen, decides "merged" for all
    // branches. A merge-base per branch was branches × distance to the base, with every
    // commit on the way inflated once per branch: 520 ms and 545 MB for ~150 branches and 65
    // commits (PERF §1.3).
    let merged_tips = merged_tips(&repo, head, tips.iter().map(|(_, id)| *id));
    let mut branches = Vec::new();
    for (name, id) in tips {
        let merged = merged_tips.contains(&id);
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
    Ok(BranchesOut { current, branches, remote_branches: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_pull_requests_cover_all_pages_and_lifecycle_states() {
        let row = |number, state: &str, merged_at: serde_json::Value, repository: &str| {
            json!({
                "number":number,"head":{"ref":"feature","repo":{"full_name":repository}},
                "base":{"ref":"main","repo":{"full_name":"owner/repo"}},
                "draft":false,"html_url":"https://example.test/pr","title":"Changes",
                "state":state,"merged_at":merged_at
            })
        };
        let fixture = json!([
            [row(1, "open", serde_json::Value::Null, "owner/repo")],
            [
                row(2, "closed", json!("2026-09-05T00:00:00Z"), "owner/repo"),
                row(3, "closed", serde_json::Value::Null, "other/fork")
            ]
        ]);
        let rows = decode_pull_requests(&serde_json::to_vec(&fixture).unwrap()).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter().map(|pr| pr.state.as_str()).collect::<Vec<_>>(),
            ["open", "merged", "closed"]
        );
        assert!(rows[1].same_repository);
        assert!(!rows[2].same_repository);
        assert!(decode_pull_requests(b"[[]").is_err());
    }

    #[test]
    fn github_cli_paginates_and_failure_is_not_an_empty_list() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let gh = dir.path().join("gh");
        std::fs::write(&gh, "#!/usr/bin/env python3\nimport sys\nassert sys.argv[1:4] == ['api','--paginate','--slurp']\nassert 'state=all' in sys.argv[4]\nprint('[[], []]')\n").unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let result = list_pull_requests(&gh, dir.path()).unwrap();
        assert!(result.complete && result.pull_requests.is_empty());
        std::fs::write(
            &gh,
            "#!/usr/bin/env python3\nimport sys\nprint('[[]]')\nsys.exit(1)\n",
        )
        .unwrap();
        assert!(list_pull_requests(&gh, dir.path()).is_err());
    }

    fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn local_remote_fetch_refreshes_new_worktree_without_moving_primary() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        let checkout = dir.path().join("checkout");
        std::fs::create_dir(&origin).unwrap();
        git(&origin, &["init", "-b", "main"]);
        git(&origin, &["config", "user.name", "Fixture"]);
        git(&origin, &["config", "user.email", "fixture@example.test"]);
        git(&origin, &["commit", "--allow-empty", "-m", "initial"]);
        git(
            dir.path(),
            &[
                "clone",
                origin.to_str().unwrap(),
                checkout.to_str().unwrap(),
            ],
        );
        let original = git(&checkout, &["rev-parse", "HEAD"]);
        git(
            &origin,
            &["commit", "--allow-empty", "-m", "remote advance"],
        );
        let advanced = git(&origin, &["rev-parse", "HEAD"]);
        fetch_remote(&checkout).unwrap();
        let from = new_worktree_base(&checkout, "main").unwrap().unwrap();
        let new_path = dir.path().join("new-session");
        worktree::create(&checkout, &new_path, "relay/fixture", Some(&from)).unwrap();
        assert_eq!(git(&new_path, &["rev-parse", "HEAD"]), advanced);
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), original);
        let brief = briefing_state(&new_path, "relay/fixture", "main");
        assert!(brief.contains("git_base_upstream_behind: 1"));
        assert!(brief.contains("cached snapshot"));
    }

    #[test]
    fn missing_upstream_is_unknown_and_fetched_base_preserves_local_commits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Fixture"]);
        git(root, &["config", "user.email", "fixture@example.test"]);
        git(root, &["commit", "--allow-empty", "-m", "base"]);
        let base = git(root, &["rev-parse", "HEAD"]);
        let metrics = || upstream_metrics(&gix::open(root).unwrap(), "main");
        assert_eq!(metrics(), (None, None, None));
        git(root, &["config", "branch.main.remote", "origin"]);
        git(root, &["config", "branch.main.merge", "refs/heads/main"]);
        git(
            root,
            &["config", "remote.origin.url", "/nonexistent/relay-fixture"],
        );
        git(
            root,
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
        );
        assert_eq!(metrics(), (Some("origin/main".into()), None, None));
        assert!(briefing_state(root, "main", "main").contains("git_ahead: unknown"));
        git(root, &["update-ref", "refs/remotes/origin/main", &base]);
        assert_eq!(metrics(), (Some("origin/main".into()), Some(0), Some(0)));
        git(root, &["checkout", "-b", "remote-fixture"]);
        git(root, &["commit", "--allow-empty", "-m", "remote"]);
        let remote = git(root, &["rev-parse", "HEAD"]);
        git(root, &["update-ref", "refs/remotes/origin/main", &remote]);
        git(root, &["checkout", "main"]);
        assert_eq!(
            new_worktree_base(root, "main").unwrap(),
            Some(remote)
        );
        assert_eq!(git(root, &["rev-parse", "main"]), base);
        git(root, &["commit", "--allow-empty", "-m", "local"]);
        assert_eq!(metrics(), (Some("origin/main".into()), Some(1), Some(1)));
        assert_eq!(new_worktree_base(root, "main").unwrap_err().code, "git.base_diverged");
        let local = git(root, &["rev-parse", "main"]);
        assert!(fetch_remote(root).is_err());
        assert_eq!(git(root, &["rev-parse", "main"]), local);
        assert_eq!(new_worktree_base(root, "main").unwrap_err().code, "git.base_diverged");
    }
}
