//! Worktrees (SPEC §1, §8): reads through gix, mutations through the `git` binary. The pool
//! lives at `<repo>/.relay/worktrees/<name>` on branch `relay/<name>`; `.relay/` is added to
//! `.git/info/exclude` so it never shows up in status.

use anyhow::{anyhow, Context, Result};
use relay_bus::types::{FileStatus, Worktree};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Directories that are build output and safe to purge before removing a worktree
/// (SPEC §8: "gradlew clean / target purge — the 73GB problem, closed").
pub const BUILD_DIRS: &[&str] = &["target", "build", ".gradle", "app/build", "node_modules/.cache", "dist"];

pub fn pool_dir(repo: &Path) -> PathBuf {
    repo.join(".relay").join("worktrees")
}

pub fn pooled_path(repo: &Path, name: &str) -> PathBuf {
    pool_dir(repo).join(name)
}

pub fn branch_for(name: &str) -> String {
    format!("relay/{name}")
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        return Err(anyhow!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Run a mutating system-git operation. Read paths stay in gix-backed helpers.
pub fn git_mutate(repo: &Path, args: &[&str]) -> Result<String> { git(repo, args) }

/// Make sure `.relay/` is excluded in this repo without touching its tracked `.gitignore`.
pub fn ensure_excluded(repo: &Path) -> Result<()> {
    exclude_paths(repo, &[".relay/", ".claude/settings.local.json", ".codex/hooks.json"])
}

/// Same, for paths Relay writes into a checkout on demand (materialized skill folders).
pub fn exclude_paths(repo: &Path, entries: &[&str]) -> Result<()> {
    let git_dir = git_dir_of(repo)?;
    let info = git_dir.join("info");
    std::fs::create_dir_all(&info)?;
    let exclude = info.join("exclude");
    let cur = std::fs::read_to_string(&exclude).unwrap_or_default();
    let mut missing = Vec::new();
    for entry in entries.iter().copied() {
        if !cur.lines().any(|line| line.trim() == entry.trim_end_matches('/'))
            && !cur.lines().any(|line| line.trim() == entry)
        {
            missing.push(entry);
        }
    }
    if !missing.is_empty() {
        let mut contents = cur;
        if !contents.is_empty() && !contents.ends_with('\n') { contents.push('\n'); }
        for entry in missing {
            contents.push_str(entry);
            contents.push('\n');
        }
        std::fs::write(&exclude, contents)?;
    }
    Ok(())
}

/// The common `.git` dir of a repo (works from the primary checkout).
pub fn git_dir_of(repo: &Path) -> Result<PathBuf> {
    let r = gix::open(repo).with_context(|| format!("opening {}", repo.display()))?;
    Ok(r.common_dir().to_path_buf())
}

/// Uncommitted changes of any kind — modified, staged, or untracked (an agent's new file is
/// a change too). `gix::Repository::is_dirty` ignores untracked files, hence this.
pub fn is_dirty(r: &gix::Repository) -> bool {
    // A failed scan cannot establish that deleting/switching a checkout is safe.
    r.workdir().is_none_or(|root| status_files(root).map_or(true, |files| !files.is_empty()))
}

/// Git persists refreshed stat data with its index locking. Dropping gix's status
/// outcome rehashes touched-but-unchanged LFS assets on every refresh; writing an
/// old index snapshot ourselves could lose concurrent staging.
pub fn status_files(root: &Path) -> Result<Vec<FileStatus>> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .env("GIT_OPTIONAL_LOCKS", "1");
    let output = crate::proc::output_with_timeout(&mut command, std::time::Duration::from_secs(10))?
        .ok_or_else(|| anyhow!("Git status timed out after 10 seconds; check repository filters and retry"))?;
    if !output.status.success() {
        return Err(anyhow!("Git status failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    parse_status(&output.stdout)
}

fn parse_status(output: &[u8]) -> Result<Vec<FileStatus>> {
    let mut records = output.split(|byte| *byte == 0).filter(|record| !record.is_empty());
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err(anyhow!("Malformed Git status record"));
        }
        let untracked = &record[..2] == b"??";
        let index = if untracked || record[0] == b' ' { String::new() } else { (record[0] as char).to_string() };
        let worktree = if record[1] == b' ' { String::new() } else { (record[1] as char).to_string() };
        let renamed_from = if record[..2].iter().any(|status| matches!(status, b'R' | b'C')) {
            Some(String::from_utf8_lossy(records.next().ok_or_else(|| anyhow!("Missing Git rename source"))?).into_owned())
        } else { None };
        files.push(FileStatus {
            path: String::from_utf8_lossy(&record[3..]).into_owned(),
            index, worktree, renamed_from,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn head_of(r: &gix::Repository) -> (String, String) {
    let branch = r.head_name().ok().flatten().map(|n| n.shorten().to_string()).unwrap_or_else(|| "HEAD".into());
    let head = r.head_id().map(|id| id.to_string()).unwrap_or_default();
    (branch, head)
}

/// All worktrees of a repo: the primary first, then linked ones. `session` is filled by the
/// caller (the store knows who owns what); `disk_mb` only when asked (it walks the tree).
pub fn list(repo: &Path) -> Result<Vec<Worktree>> {
    list_with_dirty(repo, true)
}

/// Worktree metadata with optional dirty scans. Hot UI queries skip untracked-file scans in every
/// linked checkout and project the selected checkout's dirty state from their one `git.status`.
pub fn list_with_dirty(repo: &Path, include_dirty: bool) -> Result<Vec<Worktree>> {
    let r = gix::open(repo).with_context(|| format!("opening {}", repo.display()))?;
    let mut out = Vec::new();
    let (branch, head) = head_of(&r);
    let dirty = include_dirty && is_dirty(&r);
    let primary = r.workdir().map(Path::to_path_buf).unwrap_or_else(|| repo.to_path_buf());
    out.push(Worktree { path: canon(&primary), branch, head, session: None, dirty, disk_mb: None });
    for proxy in r.worktrees().context("listing worktrees")? {
        let Ok(base) = proxy.base() else { continue };
        if !base.exists() { continue; }
        match proxy.into_repo() {
            Ok(wr) => {
                let (branch, head) = head_of(&wr);
                let dirty = include_dirty && is_dirty(&wr);
                out.push(Worktree { path: canon(&base), branch, head, session: None, dirty, disk_mb: None });
            }
            Err(_) => out.push(Worktree { path: canon(&base), branch: "?".into(), head: String::new(), session: None, dirty: false, disk_mb: None }),
        }
    }
    Ok(out)
}

/// Validate worktree membership without dirty-scanning every checkout.
pub fn contains(repo: &Path, candidate: &Path) -> Result<bool> {
    let r = gix::open(repo).with_context(|| format!("opening {}", repo.display()))?;
    let want = canon(candidate);
    let primary = r.workdir().map(Path::to_path_buf).unwrap_or_else(|| repo.to_path_buf());
    if canon(&primary) == want { return Ok(true); }
    for proxy in r.worktrees().context("listing worktrees")? {
        let Ok(base) = proxy.base() else { continue };
        if base.exists() && canon(&base) == want { return Ok(true); }
    }
    Ok(false)
}

fn canon(p: &Path) -> String {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).display().to_string()
}

/// `git worktree add -b <branch> <path> [<from>]`; if the branch exists already, check it out
/// instead of creating it.
pub fn create(repo: &Path, path: &Path, branch: &str, from: Option<&str>) -> Result<Worktree> {
    ensure_excluded(repo)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_s = path.display().to_string();
    let exists = git(repo, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
    if exists {
        git(repo, &["worktree", "add", &path_s, branch])?;
    } else {
        let mut args = vec!["worktree", "add", "-b", branch, &path_s];
        if let Some(f) = from { args.push(f); }
        git(repo, &args)?;
    }
    let all = list_with_dirty(repo, false)?;
    let want = canon(path);
    all.into_iter().find(|w| w.path == want).ok_or_else(|| anyhow!("worktree {} not listed after add", path.display()))
}

/// Bytes under `dir` (no symlink following).
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.file_type().is_symlink() { continue; }
            if md.is_dir() { stack.push(e.path()); } else { total += md.len(); }
        }
    }
    total
}

/// Size of the purge-able build dirs inside a worktree.
pub fn build_size(wt: &Path) -> u64 {
    BUILD_DIRS.iter().map(|d| dir_size(&wt.join(d))).sum()
}

/// Delete build output. Returns bytes freed.
pub fn purge_build(wt: &Path) -> u64 {
    let mut freed = 0;
    for d in BUILD_DIRS {
        let p = wt.join(d);
        if p.is_dir() {
            freed += dir_size(&p);
            let _ = std::fs::remove_dir_all(&p);
        }
    }
    freed
}

/// Remove a linked worktree (never the primary). Returns bytes freed. The branch is kept.
pub fn remove(repo: &Path, path: &Path, purge: bool) -> Result<u64> {
    let repo_c = canon(repo);
    let path_c = canon(path);
    if repo_c == path_c {
        return Err(anyhow!("refusing to remove the primary checkout"));
    }
    let mut freed = 0;
    if path.exists() {
        if purge { freed += purge_build(path); }
        freed += dir_size(path);
        git(repo, &["worktree", "remove", "--force", &path_c])
            .or_else(|e| {
                // a half-deleted worktree: remove the dir ourselves and prune
                std::fs::remove_dir_all(path).map(|_| String::new()).map_err(|io| anyhow!("{e}; and rm -rf failed: {io}"))
            })?;
    }
    let _ = git(repo, &["worktree", "prune"]);
    Ok(freed)
}

/// Rename the branch checked out by one worktree. Used only before a session's first spawn.
pub fn rename_branch(worktree: &Path, branch: &str) -> Result<()> {
    if branch.trim().is_empty() || branch != branch.trim() {
        return Err(anyhow!("branch name cannot be empty or padded"));
    }
    git(worktree, &["check-ref-format", "--branch", branch])?;
    git(worktree, &["branch", "-m", branch])?;
    Ok(())
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn porcelain_preserves_renames_conflicts_and_unusual_paths() {
        let files = parse_status(b"R  new name\0old\nname\0?? untracked\nfile\0UU conflict\0 M changed\0").unwrap();
        assert_eq!(files[0].worktree, "M");
        assert_eq!(files[1].index, "U");
        assert_eq!(files[1].worktree, "U");
        assert_eq!(files[2].renamed_from.as_deref(), Some("old\nname"));
        assert_eq!(files[3].path, "untracked\nfile");
        assert_eq!(files[3].index, "");
        assert_eq!(files[3].worktree, "?");
    }

    #[test]
    fn repeated_status_does_not_rerun_filters_for_unchanged_assets() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-q", "-b", "main"]).unwrap();
        git(root, &["config", "user.name", "Fixture"]).unwrap();
        git(root, &["config", "user.email", "fixture@example.invalid"]).unwrap();
        std::fs::write(root.join(".gitattributes"), "*.bin filter=fixture\n").unwrap();
        let filter = root.join(".git/filter.sh");
        std::fs::write(&filter, "#!/bin/sh\necho run >> .git/filter-calls\ncat\n").unwrap();
        git(root, &["config", "filter.fixture.clean", &format!("sh {}", filter.display())]).unwrap();
        let asset = root.join("asset.bin");
        std::fs::write(&asset, vec![42u8; 1024 * 1024]).unwrap();
        let initial_time = std::time::SystemTime::now() - std::time::Duration::from_secs(30);
        std::fs::File::open(&asset).unwrap().set_modified(initial_time).unwrap();
        git(root, &["add", "."]).unwrap();
        git(root, &["commit", "-qm", "Asset"]).unwrap();
        // Simulate an editor touching an identical LFS-managed asset.
        std::fs::File::open(&asset).unwrap()
            .set_modified(initial_time + std::time::Duration::from_secs(5)).unwrap();
        std::fs::write(root.join(".git/filter-calls"), "").unwrap();
        assert!(status_files(root).unwrap().is_empty());
        let first = std::fs::read_to_string(root.join(".git/filter-calls")).unwrap();
        assert!(!first.is_empty(), "Fixture must exercise the clean filter");
        for _ in 0..5 { assert!(status_files(root).unwrap().is_empty()); }
        assert_eq!(std::fs::read_to_string(root.join(".git/filter-calls")).unwrap(), first,
            "Unchanged assets must use the refreshed stat cache");
        // The optimization must still detect real edits and preserve staged content.
        std::fs::write(&asset, "changed asset").unwrap();
        assert_eq!(status_files(root).unwrap()[0].worktree, "M");
        git(root, &["add", "asset.bin"]).unwrap();
        assert_eq!(status_files(root).unwrap()[0].index, "M");
    }
}
