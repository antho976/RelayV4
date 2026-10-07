//! Phase 8 bus-driven coverage: files, git, guardrail replay, and integrations.

mod common;

use common::{call, committed_repo, err, git, git_command};
use relay_bus::ErrorKind;
use relay_core::engine::Engine;
use relay_core::Instance;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::Arc;

/// An engine whose git reads no global or system config (RA-672). The fixture's own git
/// (`git_command`) already does not, but the engine's runs as this process does, and a
/// developer's global `core.hooksPath` would chain `git.commit` to their real pre-commit hook.
/// Pinning `core.hooksPath` in the fixture instead would move the hook-chaining test off git's
/// default hooks directory, which is what it covers. The engine itself must keep reading the
/// user's config, so only this test process drops it. Every test here calls this first, so the
/// one `set_var` runs before any test has started git.
fn engine() -> Arc<Engine> {
    static HERMETIC: std::sync::Once = std::sync::Once::new();
    HERMETIC.call_once(|| {
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    });
    common::engine()
}

fn real_repo() -> (tempfile::TempDir, String) {
    let ws = tempfile::tempdir().unwrap();
    let repo = ws.path().join("repo");
    committed_repo(&repo, &[("README.md", "# Relay\n")]);
    let path = std::fs::canonicalize(&repo).unwrap().display().to_string();
    (ws, path)
}
fn add_project(e: &Engine, ws: &tempfile::TempDir, repo: &str) {
    call(
        e,
        "workspace.create",
        json!({"path":std::fs::canonicalize(ws.path()).unwrap()}),
    )
    .into_result()
    .unwrap();
    call(e, "project.add", json!({"workspace_id":1,"path":repo}))
        .into_result()
        .unwrap();
}

#[test]
fn file_lifecycle_search_and_confirmation_replay() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    call(
        &e,
        "file.create",
        json!({"project_id":1,"path":"src","kind":"dir"}),
    )
    .into_result()
    .unwrap();
    call(&e,"file.create",json!({"project_id":1,"path":"src/main.rs","kind":"file","text":"fn main() { println!(\"relay\"); }\n"})).into_result().unwrap();
    std::fs::create_dir_all(std::path::Path::new(&repo).join("target/debug")).unwrap();
    std::fs::write(
        std::path::Path::new(&repo).join("target/debug/generated"),
        "cache",
    )
    .unwrap();
    let read = call(
        &e,
        "file.read",
        json!({"project_id":1,"path":"src/main.rs"}),
    )
    .into_result()
    .unwrap();
    assert!(read["text"].as_str().unwrap().contains("relay"));
    let tree = call(&e, "file.tree", json!({"project_id":1,"depth":3}))
        .into_result()
        .unwrap();
    assert!(tree["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["path"] == "src"));
    assert!(!tree["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["path"] == "target"));
    let fast_tree = call(
        &e,
        "file.tree",
        json!({"project_id":1,"depth":3,"git_badges":false}),
    )
    .into_result()
    .unwrap();
    let src = fast_tree["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == "src")
        .unwrap();
    let main = src["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == "src/main.rs")
        .unwrap();
    assert!(main["badge"].is_null());
    let fast_worktrees = call(
        &e,
        "worktree.list",
        json!({"project_id":1,"include_dirty":false}),
    )
    .into_result()
    .unwrap();
    assert_eq!(fast_worktrees["worktrees"][0]["dirty"], false);
    let hits = call(
        &e,
        "file.search",
        json!({"project_id":1,"query":"println","glob":"src/**"}),
    )
    .into_result()
    .unwrap();
    assert_eq!(hits["hits"][0]["line"], 1);
    // Binaries and non-regular files are skipped, not read: a FIFO would block the walk.
    let src = std::path::Path::new(&repo).join("src");
    std::fs::write(src.join("asset.bin"), b"println\0").unwrap();
    let fifo = src.join("pipe");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let hits = call(&e, "file.search", json!({"project_id":1,"query":"println","glob":"src/**"}))
        .into_result()
        .unwrap();
    assert_eq!(hits["hits"].as_array().unwrap().len(), 1, "{hits}");
    std::fs::remove_file(src.join("asset.bin")).unwrap();
    std::fs::remove_file(fifo).unwrap();
    let deleted = call(
        &e,
        "file.delete",
        json!({"project_id":1,"path":"src/main.rs"}),
    )
    .into_result()
    .unwrap();
    assert!(!std::path::Path::new(&repo).join("src/main.rs").exists());
    call(
        &e,
        "file.restore",
        json!({"project_id":1,"trash_id":deleted["trash_id"]}),
    )
    .into_result()
    .unwrap();
    assert!(std::path::Path::new(&repo).join("src/main.rs").exists());
    call(
        &e,
        "project.update",
        json!({"project_id":1,"protected_paths":["protected.txt"]}),
    )
    .into_result()
    .unwrap();
    let held = call(
        &e,
        "file.create",
        json!({"project_id":1,"path":"protected.txt","kind":"file","text":"confirmed\n"}),
    );
    assert_eq!(err(&held).kind, ErrorKind::Held);
    let hold_id = err(&held).confirm.as_ref().unwrap().payload["hold_id"]
        .as_i64()
        .unwrap();
    let confirmed = call(&e, "guardrail.confirm", json!({"hold_id":hold_id}))
        .into_result()
        .unwrap();
    assert_eq!(confirmed["outcome"]["ok"], true);
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&repo).join("protected.txt")).unwrap(),
        "confirmed\n"
    );
    assert_eq!(
        err(&call(
            &e,
            "file.read",
            json!({"project_id":1,"path":"../secret"})
        ))
        .code,
        "file.path"
    );
}

#[test]
fn git_status_diff_commit_log_show_and_branches() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    relay_core::worktree::ensure_excluded(std::path::Path::new(&repo)).unwrap();
    let user_hook = std::path::Path::new(&repo).join(".git/hooks/pre-commit");
    std::fs::write(&user_hook, "#!/bin/sh\ntouch .git/previous-hook-ran\n").unwrap();
    std::fs::set_permissions(&user_hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    relay_core::hooks::install_git(
        std::path::Path::new(&repo),
        std::path::Path::new(&repo),
        "phase-eight",
        Instance::Test,
        std::path::Path::new("/bin/false"),
    )
    .unwrap();
    call(
        &e,
        "file.create",
        json!({"project_id":1,"path":"phase8.txt","kind":"file","text":"one\ntwo\n"}),
    )
    .into_result()
    .unwrap();
    let status = call(&e, "git.status", json!({"project_id":1}))
        .into_result()
        .unwrap();
    assert!(status["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["path"] == "phase8.txt"));
    let diff = call(
        &e,
        "git.diff.file",
        json!({"project_id":1,"path":"phase8.txt"}),
    )
    .into_result()
    .unwrap();
    assert_eq!(diff["old"], "");
    assert!(diff["new"].as_str().unwrap().contains("two"));
    // A binary or oversized file is refused before its reply outgrows a client's line.
    let root = std::path::Path::new(&repo);
    std::fs::write(root.join("blob.bin"), b"head\0tail").unwrap();
    std::fs::write(root.join("huge.txt"), "line\n".repeat(300_000)).unwrap();
    assert_eq!(err(&call(&e, "git.diff.file", json!({"project_id":1,"path":"blob.bin"}))).code, "git.diff_binary");
    assert_eq!(err(&call(&e, "git.diff.file", json!({"project_id":1,"path":"huge.txt"}))).code, "git.diff_too_large");
    std::fs::remove_file(root.join("blob.bin")).unwrap();
    std::fs::remove_file(root.join("huge.txt")).unwrap();
    call(
        &e,
        "git.stage",
        json!({"project_id":1,"paths":["phase8.txt"]}),
    )
    .into_result()
    .unwrap();
    let staged = call(&e, "git.diff", json!({"project_id":1,"staged":true}))
        .into_result()
        .unwrap();
    assert_eq!(staged["files"][0]["added"], 2);
    let commit = call(
        &e,
        "git.commit",
        json!({"project_id":1,"message":"Add phase 8 fixture"}),
    )
    .into_result()
    .unwrap();
    assert!(std::path::Path::new(&repo).join(".git/previous-hook-ran").is_file());
    assert_eq!(commit["sha"].as_str().unwrap().len(), 40);
    let shown = call(&e, "git.show", json!({"project_id":1,"sha":commit["sha"]}))
        .into_result()
        .unwrap();
    assert_eq!(shown["files"][0]["path"], "phase8.txt");
    let base_diff = call(&e, "git.diff", json!({"project_id":1,"base":"HEAD~1"}))
        .into_result()
        .unwrap();
    assert_eq!(base_diff["files"][0]["path"], "phase8.txt");
    assert_eq!(base_diff["files"][0]["added"], 2);
    let log = call(&e, "git.log", json!({"project_id":1,"limit":5}))
        .into_result()
        .unwrap();
    assert_eq!(log["commits"][0]["subject"], "Add phase 8 fixture");
    let branches = call(&e, "git.branches", json!({"project_id":1}))
        .into_result()
        .unwrap();
    assert_eq!(branches["current"], "main");
}

#[test]
fn git_push_sets_upstream_on_first_push_and_reuses_it_afterward() {
    let e = engine();
    let (ws, repo) = real_repo();
    let remote = ws.path().join("remote.git");
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--bare"]);
    git(
        std::path::Path::new(&repo),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(
        std::path::Path::new(&repo),
        &["checkout", "-b", "relay/spry-gecko"],
    );
    add_project(&e, &ws, &repo);

    call(&e, "git.push", json!({"project_id":1}))
        .into_result()
        .unwrap();
    let first_status = call(&e, "git.status", json!({"project_id":1}))
        .into_result()
        .unwrap();
    assert_eq!(first_status["upstream"], "origin/relay/spry-gecko");
    assert_eq!(first_status["ahead"], 0);
    assert_eq!(first_status["behind"], 0);

    std::fs::write(
        std::path::Path::new(&repo).join("second.txt"),
        "second push\n",
    )
    .unwrap();
    git(std::path::Path::new(&repo), &["add", "second.txt"]);
    git(
        std::path::Path::new(&repo),
        &["commit", "-m", "Second push"],
    );
    call(&e, "git.push", json!({"project_id":1}))
        .into_result()
        .unwrap();
    let second_status = call(&e, "git.status", json!({"project_id":1}))
        .into_result()
        .unwrap();
    assert_eq!(second_status["upstream"], "origin/relay/spry-gecko");
    assert_eq!(second_status["ahead"], 0);
    assert_eq!(second_status["behind"], 0);
}

#[test]
fn branch_create_validates_names_reports_conflicts_and_survives_repeated_dispatch() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let created = call(&e, "git.branch.create", json!({"project_id":1,"name":"feature/typed-create"})).into_result().unwrap();
    assert_eq!(created["name"], "feature/typed-create");
    assert_eq!(created["worktree"], repo);
    assert_eq!(created["head"].as_str().unwrap().len(), 40);
    assert_eq!(call(&e, "git.status", json!({"project_id":1})).into_result().unwrap()["branch"], "feature/typed-create");

    let duplicate = call(&e, "git.branch.create", json!({"project_id":1,"name":"feature/typed-create"}));
    assert_eq!(err(&duplicate).code, "git.branch_exists");
    let invalid = call(&e, "git.branch.create", json!({"project_id":1,"name":"bad name"}));
    assert_eq!(err(&invalid).kind, ErrorKind::Invalid);
    assert_eq!(err(&invalid).code, "git.branch_name");

    for index in 0..24 {
        call(&e, "git.branch.create", json!({"project_id":1,"name":format!("fixture/{index}"),"checkout":false})).into_result().unwrap();
    }
    let branches = call(&e, "git.branches", json!({"project_id":1})).into_result().unwrap();
    assert_eq!(branches["branches"].as_array().unwrap().len(), 26);
}

#[test]
fn branch_switch_keeps_dirty_and_session_owned_checkouts() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let root = std::path::Path::new(&repo);
    git(root, &["branch", "feature/switch"]);
    call(&e, "git.branch.switch", json!({"project_id":1,"name":"feature/switch"})).into_result().unwrap();
    assert_eq!(call(&e,"git.status",json!({"project_id":1})).into_result().unwrap()["branch"],"feature/switch");
    std::fs::write(root.join("README.md"), "keep dirty text").unwrap();
    assert_eq!(err(&call(&e,"git.branch.switch",json!({"project_id":1,"name":"main"}))).code,"git.checkout_dirty");
    assert_eq!(std::fs::read_to_string(root.join("README.md")).unwrap(),"keep dirty text");
    git(root, &["add", "README.md"]);
    git(root, &["commit", "-m", "save"]);
    let session = call(&e,"session.create",json!({"project_id":1,"provider":"codex","worktree":"primary"})).into_result().unwrap();
    assert_eq!(session["worktree"], repo);
    assert_eq!(err(&call(&e,"git.branch.switch",json!({"project_id":1,"name":"main"}))).code,"git.checkout_session_owned");
}

#[test]
fn branch_switch_preserves_ignored_files_that_target_would_overwrite() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let root=std::path::Path::new(&repo);
    std::fs::write(root.join(".gitignore"), "private.bin\n").unwrap();
    git(root, &["add", ".gitignore"]);
    git(root, &["commit", "-m", "Ignore local file"]);
    git(root, &["switch", "-c", "feature/tracked"]);
    std::fs::write(root.join("private.bin"), "tracked on target").unwrap();
    git(root, &["add", "-f", "private.bin"]);
    git(root, &["commit", "-m", "Target file"]);
    git(root, &["switch", "main"]);
    std::fs::write(root.join("private.bin"), "personal ignored data").unwrap();
    let result=call(&e,"git.branch.switch",json!({"project_id":1,"name":"feature/tracked"}));
    assert_eq!(err(&result).code,"git.branch_switch_failed");
    assert_eq!(std::fs::read_to_string(root.join("private.bin")).unwrap(),"personal ignored data");
    assert_eq!(call(&e,"git.status",json!({"project_id":1})).into_result().unwrap()["branch"],"main");
}

#[test]
fn branch_listing_follows_the_selected_worktree_and_delete_keeps_unsafe_branches() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let repo_path = std::path::Path::new(&repo);
    let linked = ws.path().join("secondary");
    git(
        repo_path,
        &[
            "worktree",
            "add",
            "-b",
            "feature/secondary",
            linked.to_str().unwrap(),
        ],
    );
    let linked = std::fs::canonicalize(&linked).unwrap();

    let branches = call(
        &e,
        "git.branches",
        json!({"project_id":1,"worktree":linked}),
    )
    .into_result()
    .unwrap();
    assert_eq!(branches["current"], "feature/secondary");
    assert!(branches["branches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|branch| branch["name"] == "feature/secondary" && branch["current"] == true));

    let checked_out = call(
        &e,
        "git.branch.delete",
        json!({"project_id":1,"name":"feature/secondary"}),
    );
    assert_eq!(err(&checked_out).code, "git.branch_checked_out");
    for dry_run in [true, false] {
        let cleaned = call(&e, "git.branch.clean_merged", json!({"project_id":1,"dry_run":dry_run}))
            .into_result().unwrap();
        assert!(!cleaned["deleted"].as_array().unwrap().iter().any(|name| name == "feature/secondary"));
    }
    assert!(linked.exists());

    git(repo_path, &["worktree", "remove", linked.to_str().unwrap()]);
    call(
        &e,
        "git.branch.delete",
        json!({"project_id":1,"name":"feature/secondary"}),
    )
    .into_result()
    .unwrap();
    let branches = call(&e, "git.branches", json!({"project_id":1}))
        .into_result()
        .unwrap();
    assert!(!branches["branches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|branch| branch["name"] == "feature/secondary"));

    git(repo_path, &["checkout", "-b", "feature/unmerged"]);
    std::fs::write(repo_path.join("unmerged.txt"), "keep me\n").unwrap();
    git(repo_path, &["add", "unmerged.txt"]);
    git(repo_path, &["commit", "-m", "Unmerged work"]);
    git(repo_path, &["checkout", "main"]);
    let unmerged = call(
        &e,
        "git.branch.delete",
        json!({"project_id":1,"name":"feature/unmerged"}),
    );
    assert_eq!(err(&unmerged).code, "git.branch_unmerged");
    let base = call(
        &e,
        "git.branch.delete",
        json!({"project_id":1,"name":"main"}),
    );
    assert_eq!(err(&base).code, "git.branch_protected");

    let session = call(
        &e,
        "session.create",
        json!({"project_id":1,"provider":"codex"}),
    )
    .into_result()
    .unwrap();
    let owned = call(
        &e,
        "git.branch.delete",
        json!({"project_id":1,"name":session["branch"]}),
    );
    assert_eq!(err(&owned).code, "git.branch_session_owned");
}

#[test]
fn integration_merges_two_branches_and_reports_result() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let path = std::path::Path::new(&repo);
    git(path, &["checkout", "-b", "feature-a"]);
    std::fs::write(path.join("a.txt"), "a\n").unwrap();
    git(path, &["add", "a.txt"]);
    git(path, &["commit", "-m", "A"]);
    git(path, &["checkout", "main"]);
    git(path, &["checkout", "-b", "feature-b"]);
    std::fs::write(path.join("b.txt"), "b\n").unwrap();
    git(path, &["add", "b.txt"]);
    git(path, &["commit", "-m", "B"]);
    git(path, &["checkout", "main"]);
    let queued = call(
        &e,
        "integration.request",
        json!({"project_id":1,"branches":["feature-a","feature-b"],"build":false}),
    )
    .into_result()
    .unwrap();
    let id = queued["id"].as_i64().unwrap();
    // A worktree add and an octopus merge on a background thread, while the other tests run git
    // too: allow it the time a loaded machine needs, and stop as soon as it settles.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let state = loop {
        let value = call(&e, "integration.get", json!({"integration_id":id}))
            .into_result()
            .unwrap();
        let state = value["state"].as_str().unwrap().to_string();
        if matches!(state.as_str(), "passed" | "failed" | "conflict")
            || std::time::Instant::now() >= deadline
        {
            break state;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    assert_eq!(state, "passed");
    let value = call(&e, "integration.get", json!({"integration_id":id}))
        .into_result()
        .unwrap();
    let wt = value["worktree"].as_str().unwrap();
    assert!(
        std::path::Path::new(wt).join("a.txt").exists()
            && std::path::Path::new(wt).join("b.txt").exists()
    );
    call(&e, "integration.discard", json!({"integration_id":id}))
        .into_result()
        .unwrap();
    assert!(!std::path::Path::new(wt).exists());
}

/// D144/D149: `git.commit` does its hooks and staging before the transaction opens, but a held
/// commit is replayed from inside `guardrail.confirm`'s transaction. That path has to reach the
/// same handler and produce the same commit.
#[test]
fn a_held_commit_still_commits_when_confirmed() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    call(&e, "settings.set", json!({"path":"guardrails.caps","value":{"files":1,"lines":2}}))
        .into_result()
        .unwrap();

    let root = std::path::Path::new(&repo);
    std::fs::write(root.join("wide.txt"), "a\nb\nc\nd\ne\nf\n").unwrap();
    call(&e, "git.stage", json!({"project_id":1,"paths":["wide.txt"]}))
        .into_result()
        .unwrap();

    let held = call(&e, "git.commit", json!({"project_id":1,"message":"Too big"}));
    // A user commit over the caps is held for confirmation rather than refused (the bypass path).
    assert_eq!(err(&held).kind, ErrorKind::Held);
    assert_eq!(err(&held).code, "guardrail.user_bypass");
    let hold_id = err(&held).confirm.as_ref().unwrap().payload["hold_id"]
        .as_i64()
        .unwrap();

    // The replay's slow half — here a pre-commit hook that takes a second — runs before the
    // confirm takes the store, so the rest of the bus keeps moving meanwhile.
    let hook = root.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nsleep 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let confirming = {
        let e = e.clone();
        std::thread::spawn(move || call(&e, "guardrail.confirm", json!({"hold_id": hold_id})))
    };
    std::thread::sleep(std::time::Duration::from_millis(300));
    let started = std::time::Instant::now();
    call(&e, "settings.set", json!({"path":"appearance.panel_alpha","value":0.9})).into_result().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_millis(500), "a write waited {:?} behind the confirm", started.elapsed());
    let confirmed = confirming.join().unwrap().into_result().unwrap();
    assert_eq!(confirmed["hold"]["state"], "confirmed");
    assert_eq!(confirmed["outcome"]["ok"], true);
    let sha = confirmed["outcome"]["result"]["sha"].as_str().unwrap();
    assert_eq!(sha.len(), 40, "the replayed commit produced a real sha");
    let log = call(&e, "git.log", json!({"project_id":1,"limit":2}))
        .into_result()
        .unwrap();
    assert_eq!(log["commits"][0]["subject"], "Too big");
}
#[test]
fn file_save_expectation_rejects_stale_content_and_deleted_files() {
    use sha2::{Digest, Sha256};
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let expected = Sha256::digest(b"# Relay\n")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let payload =
        json!({"project_id":1,"path":"README.md","text":"my draft","expected_sha256":expected});
    std::fs::write(
        std::path::Path::new(&repo).join("README.md"),
        "newer agent edit",
    )
    .unwrap();
    assert_eq!(
        err(&call(&e, "file.write", payload.clone())).code,
        "file.edit_conflict"
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&repo).join("README.md")).unwrap(),
        "newer agent edit"
    );
    std::fs::remove_file(std::path::Path::new(&repo).join("README.md")).unwrap();
    assert_eq!(
        err(&call(&e, "file.write", payload)).code,
        "file.edit_conflict"
    );
    std::fs::write(std::path::Path::new(&repo).join("README.md"), "# Relay\n").unwrap();
    call(
        &e,
        "file.write",
        json!({"project_id":1,"path":"README.md","text":"saved","expected_sha256":expected}),
    )
    .into_result()
    .unwrap();
}

#[test]
fn filesystem_reads_do_not_retrigger_refresh_but_writes_do() {
    use std::time::{Duration, Instant};
    // The watcher emits from a thread that sleeps `watch::DEBOUNCE` after the first event of a
    // burst. A "nothing arrived" check only means something if its window is well past that,
    // or a delayed thread lets it pass without testing anything.
    const QUIET: Duration = relay_core::watch::DEBOUNCE.saturating_mul(8);
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let mut events = e.subscribe();
    call(&e, "file.tree", json!({"project_id":1,"depth":1}))
        .into_result()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if events.try_recv().is_ok_and(|event| event.ev == "file.changed") {
            break;
        }
        assert!(Instant::now() < deadline, "watcher did not register");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(QUIET);
    while events.try_recv().is_ok() {}
    for _ in 0..3 {
        call(&e, "file.tree", json!({"project_id":1,"depth":1}))
            .into_result().unwrap();
        call(&e, "file.read", json!({"project_id":1,"path":"README.md"}))
            .into_result().unwrap();
        call(&e, "git.status", json!({"project_id":1}))
            .into_result().unwrap();
    }
    std::thread::sleep(QUIET);
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.ev, "file.changed", "read-only refresh retriggered watcher");
    }
    let root = std::path::Path::new(&repo);
    for path in [".git/lfs/tmp/filter-output", "Saved/Logs/Unreal.log"] {
        std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        std::fs::write(root.join(path), "generated output").unwrap();
    }
    std::thread::sleep(QUIET);
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.ev, "file.changed", "LFS or Unreal output retriggered watcher");
    }
    std::fs::write(std::path::Path::new(&repo).join("README.md"), "changed\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if events.try_recv().is_ok_and(|event| event.ev == "file.changed") {
            break;
        }
        assert!(Instant::now() < deadline, "real write did not invalidate files");
        std::thread::sleep(Duration::from_millis(10));
    }
    // Index metadata refreshes are ignored, but actual staging still refreshes Git.
    assert!(git_command(&repo)
        .args(["add", "README.md"]).status().unwrap().success());
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if events.try_recv().is_ok_and(|event| event.ev == "file.changed") { break; }
        assert!(Instant::now() < deadline, "staging did not invalidate Git");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn search_reads_only_bounded_regular_text_and_path_ops_never_read_content() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let root = std::path::Path::new(&repo);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "// needle in source\n").unwrap();
    // A FIFO would block open() forever; a symlink out of the worktree must not be followed.
    assert!(Command::new("mkfifo").arg(root.join("src/pipe.rs")).status().unwrap().success());
    let outside = ws.path().join("outside.txt");
    std::fs::write(&outside, "needle outside\n").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("src/escape.txt")).unwrap();
    std::os::unix::fs::symlink(root.join("src/lib.rs"), root.join("src/alias.rs")).unwrap();
    // Generated trees, binaries and oversized files are not searched.
    for dir in ["build", "Intermediate", "dist"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
        std::fs::write(root.join(dir).join("gen.txt"), "needle generated\n").unwrap();
    }
    let mut binary = b"needle".to_vec();
    binary.extend([0u8; 16]);
    std::fs::write(root.join("src/blob.bin"), binary).unwrap();
    let big = std::fs::File::create(root.join("src/huge.txt")).unwrap();
    big.set_len(65 * 1024 * 1024).unwrap();
    drop(big);

    let started = std::time::Instant::now();
    let hits = call(&e, "file.search", json!({"project_id":1,"query":"needle"})).into_result().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "the search waited on something");
    let mut paths: Vec<&str> = hits["hits"].as_array().unwrap().iter().map(|h| h["path"].as_str().unwrap()).collect();
    paths.sort();
    assert_eq!(paths, ["src/alias.rs", "src/lib.rs"]);

    // Renaming, moving and deleting a file past the 64 MiB comparison cap is a path operation:
    // it is not held as a destructive write and its content is never read.
    let renamed = call(&e, "file.rename", json!({"project_id":1,"path":"src/huge.txt","new_name":"huge2.txt"}));
    assert!(renamed.error.is_none(), "rename held: {:?}", renamed.error);
    call(&e, "file.move", json!({"project_id":1,"path":"src/huge2.txt","into":""})).into_result().unwrap();
    call(&e, "file.delete", json!({"project_id":1,"path":"huge2.txt"})).into_result().unwrap();
    std::fs::remove_file(root.join("src/pipe.rs")).unwrap();

    // Protected paths still hold a rename, on either end.
    call(&e, "project.update", json!({"project_id":1,"protected_paths":["keep.txt"]})).into_result().unwrap();
    std::fs::write(root.join("keep.txt"), "x\n").unwrap();
    let held = call(&e, "file.rename", json!({"project_id":1,"path":"keep.txt","new_name":"moved.txt"}));
    assert_eq!(err(&held).kind, ErrorKind::Held);
    let held = call(&e, "file.rename", json!({"project_id":1,"path":"README.md","new_name":"keep.txt"}));
    assert_eq!(err(&held).kind, ErrorKind::Held);
}

#[test]
fn a_confirmed_write_over_the_comparison_cap_goes_through() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let big = std::fs::File::create(std::path::Path::new(&repo).join("big.dat")).unwrap();
    big.set_len(65 * 1024 * 1024).unwrap();
    drop(big);
    let held = call(&e, "file.write", json!({"project_id":1,"path":"big.dat","text":"small now\n"}));
    assert_eq!(err(&held).kind, ErrorKind::Held);
    let hold_id = err(&held).confirm.as_ref().unwrap().payload["hold_id"].as_i64().unwrap();
    let confirmed = call(&e, "guardrail.confirm", json!({"hold_id":hold_id})).into_result().unwrap();
    assert_eq!(confirmed["outcome"]["ok"], true, "{confirmed}");
    assert_eq!(std::fs::read_to_string(std::path::Path::new(&repo).join("big.dat")).unwrap(), "small now\n");
}

#[test]
fn integrations_run_one_at_a_time_and_only_the_newest_keep_their_checkouts() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let path = std::path::Path::new(&repo);
    for branch in ["feature-a", "feature-b"] {
        git(path, &["checkout", "-b", branch]);
        std::fs::write(path.join(format!("{branch}.txt")), "x\n").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", branch]);
        git(path, &["checkout", "main"]);
    }
    // Five clicks in a row: they queue behind each other instead of five concurrent checkouts.
    let ids: Vec<i64> = (0..5).map(|_| {
        call(&e, "integration.request", json!({"project_id":1,"branches":["feature-a","feature-b"],"build":false}))
            .into_result().unwrap()["id"].as_i64().unwrap()
    }).collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let list = call(&e, "integration.list", json!({"project_id":1})).into_result().unwrap();
        let states: Vec<String> = list["integrations"].as_array().unwrap().iter().map(|i| i["state"].as_str().unwrap().to_string()).collect();
        let live = states.iter().filter(|s| matches!(s.as_str(), "merging" | "building" | "deploying")).count();
        assert!(live <= 1, "integrations ran concurrently: {states:?}");
        // Pruning follows each finish, so wait for it as well as for the last result.
        if states.iter().all(|s| matches!(s.as_str(), "passed" | "discarded"))
            && states.iter().filter(|s| *s == "discarded").count() == 2 { break; }
        assert!(std::time::Instant::now() < deadline, "integrations never finished: {states:?}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Retention: the newest three keep their checkout; the older two are discarded, branch and all.
    for (n, id) in ids.iter().enumerate() {
        let value = call(&e, "integration.get", json!({"integration_id":id})).into_result().unwrap();
        let wt = std::path::Path::new(value["worktree"].as_str().unwrap());
        let branch = format!("refs/heads/relay/integration-{id}");
        let has_branch = git_command(path).args(["rev-parse", "--verify", "--quiet", &branch]).status().unwrap().success();
        if n < 2 {
            assert_eq!(value["state"], "discarded", "integration {id}");
            assert!(!wt.exists() && !has_branch, "integration {id} kept its checkout");
        } else {
            assert_eq!(value["state"], "passed", "integration {id}");
            assert!(wt.exists() && has_branch, "integration {id} lost its checkout");
        }
    }
}

/// `git.commit` makes the commit object (and any signature) before the store is locked, then
/// gates and publishes exactly that object. It still behaves like `git commit -m`.
#[test]
fn commit_publishes_the_gated_object_signs_when_asked_and_concludes_merges() {
    let e = engine();
    let (ws, repo) = real_repo();
    add_project(&e, &ws, &repo);
    let path = std::path::Path::new(&repo);
    let head = |rev: &str| String::from_utf8(git_command(path).args(["rev-parse", rev]).output().unwrap().stdout).unwrap().trim().to_string();

    // Nothing staged is refused, as `git commit` refuses it.
    let empty = call(&e, "git.commit", json!({"project_id":1,"message":"nothing"}));
    assert_eq!(err(&empty).code, "git.commit_failed");

    std::fs::write(path.join("a.txt"), "a\n").unwrap();
    let before = head("HEAD");
    let out = call(&e, "git.commit", json!({"project_id":1,"message":"\nAdd a  \n\n\nbody\n","all":true})).into_result().unwrap();
    assert_eq!(out["sha"].as_str().unwrap(), head("main"), "the branch moved to the new commit");
    assert_eq!(head("HEAD~1"), before);
    let message = String::from_utf8(git_command(path).args(["log", "-1", "--format=%B"]).output().unwrap().stdout).unwrap();
    assert_eq!(message.trim_end(), "Add a\n\nbody");
    let reflog = String::from_utf8(git_command(path).args(["reflog", "-1", "--format=%gs", "main"]).output().unwrap().stdout).unwrap();
    assert_eq!(reflog.trim(), "commit: Add a");

    // commit.gpgSign is honoured: with a signer that always fails, the commit fails and the
    // branch stays where it was.
    git(path, &["config", "commit.gpgsign", "true"]);
    git(path, &["config", "gpg.program", "false"]);
    std::fs::write(path.join("b.txt"), "b\n").unwrap();
    let unsigned = call(&e, "git.commit", json!({"project_id":1,"message":"Add b","all":true}));
    assert_eq!(err(&unsigned).code, "git.commit_failed");
    assert_eq!(head("main"), out["sha"].as_str().unwrap());
    git(path, &["config", "commit.gpgsign", "false"]);
    git(path, &["config", "--unset", "gpg.program"]);
    git(path, &["reset", "-q"]);
    std::fs::remove_file(path.join("b.txt")).unwrap();

    // A merge stopped on a conflict is concluded as a merge, with both parents.
    git(path, &["checkout", "-q", "-b", "side"]);
    std::fs::write(path.join("a.txt"), "side\n").unwrap();
    git(path, &["commit", "-qam", "side"]);
    git(path, &["checkout", "-q", "main"]);
    std::fs::write(path.join("a.txt"), "main\n").unwrap();
    git(path, &["commit", "-qam", "main"]);
    assert!(!git_command(path).args(["merge", "side"]).output().unwrap().status.success());
    std::fs::write(path.join("a.txt"), "both\n").unwrap();
    // Not until the resolution is staged: `all` would stage markers just the same (RA-204).
    assert_eq!(err(&call(&e, "git.commit", json!({"project_id":1,"message":"Merge side","all":true}))).code, "git.unmerged");
    git(path, &["add", "a.txt"]);
    call(&e, "git.commit", json!({"project_id":1,"message":"Merge side","all":true})).into_result().unwrap();
    assert_eq!(head("HEAD^2"), head("side"), "the merge kept its second parent");
}
