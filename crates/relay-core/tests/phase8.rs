//! Phase 8 bus-driven coverage: files, git, guardrail replay, and integrations.

use relay_bus::{Actor, BusError, ErrorKind, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::Arc;

fn engine() -> Arc<Engine> {
    Engine::new(Instance::Test, Store::open_memory().unwrap())
}
fn call(e: &Engine, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}
fn err(r: &Response) -> &BusError {
    r.error.as_ref().expect("expected an error response")
}
fn git(repo: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}
fn real_repo() -> (tempfile::TempDir, String) {
    let ws = tempfile::tempdir().unwrap();
    let repo = ws.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Relay Test"]);
    git(&repo, &["config", "user.email", "relay@example.test"]);
    std::fs::write(repo.join("README.md"), "# Relay\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "Initial"]);
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
    let mut state = String::new();
    for _ in 0..100 {
        let value = call(&e, "integration.get", json!({"integration_id":id}))
            .into_result()
            .unwrap();
        state = value["state"].as_str().unwrap().to_string();
        if matches!(state.as_str(), "passed" | "failed" | "conflict") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
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

    let confirmed = call(&e, "guardrail.confirm", json!({"hold_id": hold_id}))
        .into_result()
        .unwrap();
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
    std::thread::sleep(Duration::from_millis(250));
    while events.try_recv().is_ok() {}
    for _ in 0..3 {
        call(&e, "file.tree", json!({"project_id":1,"depth":1}))
            .into_result().unwrap();
        call(&e, "file.read", json!({"project_id":1,"path":"README.md"}))
            .into_result().unwrap();
        call(&e, "git.status", json!({"project_id":1}))
            .into_result().unwrap();
    }
    std::thread::sleep(Duration::from_millis(350));
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.ev, "file.changed", "read-only refresh retriggered watcher");
    }
    let root = std::path::Path::new(&repo);
    for path in [".git/lfs/tmp/filter-output", "Saved/Logs/Unreal.log"] {
        std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        std::fs::write(root.join(path), "generated output").unwrap();
    }
    std::thread::sleep(Duration::from_millis(350));
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
    assert!(std::process::Command::new("git").arg("-C").arg(&repo)
        .args(["add", "README.md"]).status().unwrap().success());
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if events.try_recv().is_ok_and(|event| event.ev == "file.changed") { break; }
        assert!(Instant::now() < deadline, "staging did not invalidate Git");
        std::thread::sleep(Duration::from_millis(10));
    }
}
