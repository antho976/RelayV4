//! `git.branch.switch` and the remote half of `git.branches`: what a person switching branches
//! from the Git panel runs into — untracked files, uncommitted edits, remote-only branches and
//! branches another checkout already holds.

mod common;

use common::{call, committed_repo, engine, err, git};
use relay_core::engine::Engine;
use serde_json::{json, Value};
use std::path::Path;

/// A repository with one commit on `main`, registered as project 1.
fn project(e: &Engine) -> (tempfile::TempDir, String) {
    let ws = tempfile::tempdir().unwrap();
    let repo = ws.path().join("repo");
    committed_repo(&repo, &[("README.md", "# Relay\n")]);
    let path = std::fs::canonicalize(&repo).unwrap().display().to_string();
    call(e, "workspace.create", json!({"path":std::fs::canonicalize(ws.path()).unwrap()})).into_result().unwrap();
    call(e, "project.add", json!({"workspace_id":1,"path":path})).into_result().unwrap();
    (ws, path)
}
fn branch(e: &Engine) -> Value {
    call(e, "git.status", json!({"project_id":1})).into_result().unwrap()["branch"].clone()
}

#[test]
fn untracked_files_do_not_block_a_switch_and_travel_with_it() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    git(root, &["branch", "feature/untracked"]);
    std::fs::write(root.join("notes.txt"), "scratch").unwrap();
    let out = call(&e, "git.branch.switch", json!({"project_id":1,"name":"feature/untracked"})).into_result().unwrap();
    assert_eq!(out["branch"], "feature/untracked");
    assert_eq!(out["created"], false);
    assert_eq!(branch(&e), "feature/untracked");
    assert_eq!(std::fs::read_to_string(root.join("notes.txt")).unwrap(), "scratch");
    // Switching to the branch already checked out is a no-op, not an error.
    let again = call(&e, "git.branch.switch", json!({"project_id":1,"name":"feature/untracked"})).into_result().unwrap();
    assert_eq!(again["branch"], "feature/untracked");
}

#[test]
fn tracked_changes_are_refused_unless_carried_and_never_discarded() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    git(root, &["branch", "feature/carry"]);
    std::fs::write(root.join("README.md"), "# Relay\nedited\n").unwrap();
    let refused = call(&e, "git.branch.switch", json!({"project_id":1,"name":"feature/carry"}));
    assert_eq!(err(&refused).code, "git.checkout_dirty");
    assert!(err(&refused).message.contains("README.md"), "{}", err(&refused).message);
    assert_eq!(branch(&e), "main");
    let carried = call(&e, "git.branch.switch", json!({"project_id":1,"name":"feature/carry","carry_changes":true}));
    assert_eq!(carried.into_result().unwrap()["branch"], "feature/carry");
    assert_eq!(branch(&e), "feature/carry");
    assert_eq!(std::fs::read_to_string(root.join("README.md")).unwrap(), "# Relay\nedited\n");
}

#[test]
fn carried_changes_that_the_target_would_overwrite_leave_the_checkout_alone() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    git(root, &["switch", "-c", "feature/other"]);
    std::fs::write(root.join("README.md"), "# Other\n").unwrap();
    git(root, &["commit", "-am", "Other readme"]);
    git(root, &["switch", "main"]);
    std::fs::write(root.join("README.md"), "# Mine\n").unwrap();
    let result = call(&e, "git.branch.switch", json!({"project_id":1,"name":"feature/other","carry_changes":true}));
    assert_eq!(err(&result).code, "git.checkout_conflict");
    assert!(err(&result).message.contains("README.md"), "{}", err(&result).message);
    assert_eq!(branch(&e), "main");
    assert_eq!(std::fs::read_to_string(root.join("README.md")).unwrap(), "# Mine\n");
}

#[test]
fn remote_branches_are_listed_and_switching_creates_a_tracking_branch() {
    let e = engine();
    let (ws, repo) = project(&e);
    let root = Path::new(&repo);
    let origin = ws.path().join("origin.git");
    git(ws.path(), &["init", "--bare", "-b", "main", origin.to_str().unwrap()]);
    git(root, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(root, &["push", "-u", "origin", "main"]);
    git(root, &["switch", "-c", "feature/remote"]);
    std::fs::write(root.join("remote.txt"), "from the remote").unwrap();
    git(root, &["add", "remote.txt"]);
    git(root, &["commit", "-m", "Remote work"]);
    git(root, &["push", "origin", "feature/remote"]);
    git(root, &["switch", "main"]);
    git(root, &["branch", "-D", "feature/remote"]);
    git(root, &["remote", "set-head", "origin", "main"]);

    let listed = call(&e, "git.branches", json!({"project_id":1})).into_result().unwrap();
    let remotes = listed["remote_branches"].as_array().unwrap();
    let names: Vec<_> = remotes.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["origin/feature/remote", "origin/main"], "origin/HEAD is not a branch");
    let feature = &remotes[0];
    assert_eq!((feature["remote"].as_str(), feature["branch"].as_str()), (Some("origin"), Some("feature/remote")));
    assert!(feature["local"].is_null());
    assert_eq!(remotes[1]["local"], "main");

    let out = call(&e, "git.branch.switch", json!({"project_id":1,"name":"origin/feature/remote"})).into_result().unwrap();
    assert_eq!((out["branch"].as_str(), out["created"].as_bool()), (Some("feature/remote"), Some(true)));
    let status = call(&e, "git.status", json!({"project_id":1})).into_result().unwrap();
    assert_eq!(status["branch"], "feature/remote");
    assert_eq!(status["upstream"], "origin/feature/remote");
    assert!(root.join("remote.txt").exists());

    // The remote of a branch that exists locally selects the local branch.
    let back = call(&e, "git.branch.switch", json!({"project_id":1,"name":"origin/main"})).into_result().unwrap();
    assert_eq!((back["branch"].as_str(), back["created"].as_bool()), (Some("main"), Some(false)));
    assert_eq!(branch(&e), "main");
}

#[test]
fn a_branch_held_by_another_checkout_or_a_session_owned_checkout_is_explained() {
    let e = engine();
    let (ws, repo) = project(&e);
    let root = Path::new(&repo);
    let other = ws.path().join("other");
    git(root, &["worktree", "add", "-b", "feature/elsewhere", other.to_str().unwrap()]);
    let held = call(&e, "git.branch.switch", json!({"project_id":1,"name":"feature/elsewhere"}));
    assert_eq!(err(&held).code, "git.branch_checked_out");
    assert!(err(&held).message.contains("other"), "{}", err(&held).message);
    assert_eq!(branch(&e), "main");

    let missing = call(&e, "git.branch.switch", json!({"project_id":1,"name":"nowhere"}));
    assert_eq!(err(&missing).code, "git.branch_not_found");

    git(root, &["branch", "feature/free"]);
    let session = call(&e, "session.create", json!({"project_id":1,"provider":"codex","worktree":"primary"})).into_result().unwrap();
    let owned = call(&e, "git.branch.switch", json!({"project_id":1,"name":"feature/free"}));
    assert_eq!(err(&owned).code, "git.checkout_session_owned");
    assert!(err(&owned).message.contains(session["name"].as_str().unwrap()), "{}", err(&owned).message);
    assert_eq!(branch(&e), "main");
}
