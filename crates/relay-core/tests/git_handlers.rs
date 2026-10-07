//! `git.*` over the bus where a repository is less tidy than a fixture: merges in history,
//! bracketed file names, submodules, symlinks, renames out of protected paths.

use relay_bus::{Actor, BusError, ErrorKind, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn engine() -> Arc<Engine> {
    Engine::new(Instance::Test, Store::open_memory().unwrap())
}
fn call(e: &Engine, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}
fn ok(e: &Engine, op: &str, payload: Value) -> Value {
    call(e, op, payload).into_result().unwrap_or_else(|error| panic!("{op}: {error:?}"))
}
fn err(r: &Response) -> &BusError {
    r.error.as_ref().expect("expected an error response")
}
fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
fn init(repo: &Path) {
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.name", "Relay Test"]);
    git(repo, &["config", "user.email", "relay@example.test"]);
}
/// A repository with one commit on `main`, registered as project 1.
fn project(e: &Engine) -> (tempfile::TempDir, String) {
    let ws = tempfile::tempdir().unwrap();
    let repo = ws.path().join("repo");
    init(&repo);
    std::fs::write(repo.join("README.md"), "# Relay\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "Initial"]);
    let path = std::fs::canonicalize(&repo).unwrap().display().to_string();
    ok(e, "workspace.create", json!({"path":std::fs::canonicalize(ws.path()).unwrap()}));
    ok(e, "project.add", json!({"workspace_id":1,"path":path}));
    (ws, path)
}
fn commit(repo: &Path, message: &str) -> String {
    git(repo, &["commit", "-q", "--allow-empty", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}
fn staged(repo: &Path) -> Vec<String> {
    let names = git(repo, &["diff", "--cached", "--name-only"]);
    names.lines().map(str::to_owned).collect()
}

/// RA-150: a parent is never listed before one of its children, even when a breadth-first
/// walk would reach it first through the shorter side of a merge.
#[test]
fn log_lists_every_commit_before_its_parents() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    git(root, &["checkout", "-q", "-b", "side"]);
    for n in 1..=3 {
        commit(root, &format!("side {n}"));
    }
    git(root, &["checkout", "-q", "main"]);
    commit(root, "main 1");
    git(root, &["merge", "-q", "--no-ff", "-m", "Merge side", "side"]);
    let log = ok(&e, "git.log", json!({"project_id":1,"limit":50}));
    let commits = log["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 6);
    assert_eq!(commits[0]["subject"], "Merge side");
    let position = |sha: &Value| commits.iter().position(|c| c["sha"] == *sha);
    for (at, c) in commits.iter().enumerate() {
        for parent in c["parents"].as_array().unwrap() {
            let parent_at = position(parent).expect("every parent is in the listing");
            assert!(parent_at > at, "{} is listed before its child {}", parent, c["sha"]);
        }
    }
}

/// RA-151: `git.branches` decides "merged" with the same single walk as branch.delete.
#[test]
fn branches_mark_merged_and_unmerged_tips() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    git(root, &["branch", "landed"]);
    git(root, &["checkout", "-q", "-b", "open"]);
    commit(root, "not on main");
    git(root, &["checkout", "-q", "main"]);
    commit(root, "main moves on");
    let out = ok(&e, "git.branches", json!({"project_id":1}));
    let merged = |name: &str| {
        out["branches"].as_array().unwrap().iter().find(|b| b["name"] == name).unwrap()["merged"].clone()
    };
    assert_eq!(out["current"], "main");
    assert_eq!(out["branches"][0]["name"], "main", "the current branch is listed first");
    assert_eq!(merged("landed"), true);
    assert_eq!(merged("open"), false);
    assert_eq!(merged("main"), true);
}

/// RA-152: creating a branch with checkout moves a live session's checkout no more than
/// git.branch.switch may; creating it without checkout is still fine.
#[test]
fn branch_create_does_not_check_out_in_a_session_owned_checkout() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let session = ok(&e, "session.create", json!({"project_id":1,"provider":"codex","worktree":"primary"}));
    assert_eq!(session["worktree"], repo);
    let refused = call(&e, "git.branch.create", json!({"project_id":1,"name":"feature/taken"}));
    assert_eq!(err(&refused).code, "git.checkout_session_owned");
    assert_eq!(git(Path::new(&repo), &["branch", "--show-current"]), "main");
    assert!(git(Path::new(&repo), &["branch", "--list", "feature/taken"]).is_empty(), "nothing was created");
    ok(&e, "git.branch.create", json!({"project_id":1,"name":"feature/taken","checkout":false}));
    assert_eq!(git(Path::new(&repo), &["branch", "--show-current"]), "main");
}

/// RA-153: a path is a name, never a glob.
#[test]
fn stage_and_unstage_take_paths_literally() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    std::fs::write(root.join("a[1].txt"), "bracketed\n").unwrap();
    std::fs::write(root.join("a1.txt"), "plain\n").unwrap();
    ok(&e, "git.stage", json!({"project_id":1,"paths":["a[1].txt"]}));
    assert_eq!(staged(root), ["a[1].txt"]);
    git(root, &["add", "a1.txt"]);
    ok(&e, "git.unstage", json!({"project_id":1,"paths":["a[1].txt"]}));
    assert_eq!(staged(root), ["a1.txt"]);
}

/// RA-155: a submodule bump and an untracked nested repository are diffed as git does — one
/// `Subproject commit` line — and committed, instead of failing the whole op.
#[test]
fn submodules_and_nested_repositories_diff_and_commit() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    let sub = root.join("sub");
    init(&sub);
    commit(&sub, "sub one");
    git(root, &["-c", "advice.addEmbeddedRepo=false", "add", "sub"]);
    git(root, &["commit", "-q", "-m", "Add sub"]);
    commit(&sub, "sub two");
    init(&root.join("nested"));
    commit(&root.join("nested"), "nested one");

    let unstaged = ok(&e, "git.diff", json!({"project_id":1}));
    let files = unstaged["files"].as_array().unwrap();
    let sub_row = files.iter().find(|f| f["path"] == "sub").expect("the bumped submodule is listed");
    assert_eq!((sub_row["added"].as_i64(), sub_row["removed"].as_i64()), (Some(1), Some(1)));
    assert!(files.iter().any(|f| f["path"].as_str().unwrap().starts_with("nested")));

    ok(&e, "git.stage", json!({"project_id":1,"paths":["sub"]}));
    let staged_diff = ok(&e, "git.diff", json!({"project_id":1,"staged":true}));
    assert_eq!(staged_diff["files"][0]["path"], "sub");
    assert_eq!(staged_diff["files"][0]["added"], 1);
    let out = ok(&e, "git.commit", json!({"project_id":1,"message":"Bump sub"}));
    let shown = ok(&e, "git.show", json!({"project_id":1,"sha":out["sha"]}));
    assert_eq!(shown["files"][0]["path"], "sub");
    assert_eq!(shown["files"][0]["status"], "M");
}

/// RA-156: a symlink is diffed as the name it holds, as git stores it, and a path through a
/// symlinked directory is not read at all — even when it points at something endless.
#[test]
fn diffs_never_follow_symlinks_out_of_the_checkout() {
    let e = engine();
    let (ws, repo) = project(&e);
    let root = Path::new(&repo);
    let outside = ws.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "not in the checkout\n").unwrap();
    std::os::unix::fs::symlink("/dev/zero", root.join("zero")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("out")).unwrap();

    let diff = ok(&e, "git.diff", json!({"project_id":1}));
    let zero = diff["files"].as_array().unwrap().iter().find(|f| f["path"] == "zero").unwrap();
    assert_eq!(zero["added"], 1);
    assert_eq!(zero["binary"], false);
    let file = ok(&e, "git.diff.file", json!({"project_id":1,"path":"zero"}));
    assert_eq!(file["new"], "/dev/zero");
    let through = ok(&e, "git.diff.file", json!({"project_id":1,"path":"out/secret.txt"}));
    assert_eq!(through["new"], "");
}

/// RA-158: moving a protected file out is a change to the protected path, and a pure rename
/// counts no lines.
#[test]
fn a_staged_rename_out_of_a_protected_path_is_gated_on_its_source() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    std::fs::create_dir_all(root.join("secret")).unwrap();
    std::fs::write(root.join("secret/key.txt"), "one\ntwo\nthree\n").unwrap();
    git(root, &["add", "secret/key.txt"]);
    git(root, &["commit", "-q", "-m", "Add key"]);
    ok(&e, "guardrail.config.set", json!({"project_id":1,"patch":{"protected_paths":["secret/**"]}}));
    std::fs::create_dir_all(root.join("public")).unwrap();
    git(root, &["mv", "secret/key.txt", "public/key.txt"]);

    let diff = ok(&e, "git.diff", json!({"project_id":1,"staged":true}));
    let moved = &diff["files"][0];
    assert_eq!((moved["path"].as_str(), moved["old_path"].as_str()), (Some("public/key.txt"), Some("secret/key.txt")));
    assert_eq!((moved["added"].as_i64(), moved["removed"].as_i64()), (Some(0), Some(0)));

    let held = call(&e, "git.commit", json!({"project_id":1,"message":"Move the key"}));
    assert_eq!(err(&held).kind, ErrorKind::Held, "{:?}", err(&held));
    assert!(err(&held).message.contains("secret/key.txt"), "{}", err(&held).message);
    assert_eq!(git(root, &["log", "-1", "--format=%s"]), "Add key", "nothing was committed");
}

/// RA-159: a commit that adds a file in a new directory lists the file, not the directory.
#[test]
fn show_and_base_diff_list_files_not_directories() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::write(root.join("src/deep/lib.rs"), "fn main() {}\n").unwrap();
    git(root, &["add", "src"]);
    let sha = {
        git(root, &["commit", "-q", "-m", "Add lib"]);
        git(root, &["rev-parse", "HEAD"])
    };
    let shown = ok(&e, "git.show", json!({"project_id":1,"sha":sha}));
    let paths: Vec<_> = shown["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap().to_owned()).collect();
    assert_eq!(paths, ["src/deep/lib.rs"]);
    let base = ok(&e, "git.diff", json!({"project_id":1,"base":"HEAD~1"}));
    let paths: Vec<_> = base["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap().to_owned()).collect();
    assert_eq!(paths, ["src/deep/lib.rs"]);
    assert_eq!(base["files"][0]["added"], 1);
}

/// RA-157: a big or binary file is flagged rather than diffed line by line.
#[test]
fn big_and_binary_files_are_flagged_without_counts() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    std::fs::write(root.join("blob.bin"), b"head\0tail").unwrap();
    std::fs::write(root.join("big.txt"), "line\n".repeat(2 * 1024 * 1024)).unwrap();
    let diff = ok(&e, "git.diff", json!({"project_id":1}));
    for name in ["blob.bin", "big.txt"] {
        let row = diff["files"].as_array().unwrap().iter().find(|f| f["path"] == name).unwrap();
        assert_eq!(row["binary"], true, "{name}");
        assert_eq!(row["added"], 0, "{name}");
    }
}

/// RA-206: a staged row diffs HEAD against the index, and a rename against its source.
#[test]
fn diff_file_shows_the_staged_side_and_a_rename_source() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-q", "-m", "Add a"]);
    std::fs::write(root.join("a.txt"), "one\nstaged\n").unwrap();
    git(root, &["add", "a.txt"]);
    std::fs::write(root.join("a.txt"), "one\nstaged\nlater\n").unwrap();

    let staged = ok(&e, "git.diff.file", json!({"project_id":1,"path":"a.txt","staged":true}));
    assert_eq!((staged["old"].as_str(), staged["new"].as_str()), (Some("one\ntwo\n"), Some("one\nstaged\n")));
    // Without `staged`, as before: HEAD against the working tree.
    let work = ok(&e, "git.diff.file", json!({"project_id":1,"path":"a.txt"}));
    assert_eq!(work["new"], "one\nstaged\nlater\n");

    git(root, &["commit", "-q", "-am", "Edit a"]);
    git(root, &["mv", "a.txt", "b.txt"]);
    std::fs::write(root.join("b.txt"), "one\nstaged\nlater\nrenamed\n").unwrap();
    git(root, &["add", "b.txt"]);
    let status = ok(&e, "git.status", json!({"project_id":1}));
    assert_eq!(status["files"][0]["renamed_from"], "a.txt", "{status}");
    let renamed = ok(&e, "git.diff.file", json!({"project_id":1,"path":"b.txt","staged":true,"old_path":"a.txt"}));
    assert_eq!(renamed["old"], "one\nstaged\nlater\n");
    assert_eq!(renamed["new"], "one\nstaged\nlater\nrenamed\n");
    let text = renamed["hunks"][0]["text"].as_str().unwrap();
    assert!(text.contains("+renamed") && !text.contains("-one"), "{text}");
    let escape = call(&e, "git.diff.file", json!({"project_id":1,"path":"b.txt","old_path":"../a.txt"}));
    assert_eq!(err(&escape).kind, ErrorKind::Invalid);
}

/// RA-204: no commit, `all` or not, while a conflict is unresolved in the index.
#[test]
fn commit_is_refused_while_the_index_is_unmerged() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    git(root, &["switch", "-q", "-c", "side"]);
    std::fs::write(root.join("README.md"), "# Side\n").unwrap();
    git(root, &["commit", "-q", "-am", "Side"]);
    git(root, &["switch", "-q", "main"]);
    std::fs::write(root.join("README.md"), "# Main\n").unwrap();
    git(root, &["commit", "-q", "-am", "Main"]);
    let merge = Command::new("git").arg("-C").arg(root).args(["merge", "-q", "side"]).output().unwrap();
    assert!(!merge.status.success(), "the fixture must conflict");
    let head = git(root, &["rev-parse", "HEAD"]);

    for all in [true, false] {
        let refused = call(&e, "git.commit", json!({"project_id":1,"message":"Merge","all":all}));
        let error = err(&refused);
        assert_eq!((error.kind, error.code.as_str()), (ErrorKind::Conflict, "git.unmerged"), "{error:?}");
        assert!(error.message.contains("README.md"), "{}", error.message);
    }
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head, "nothing was committed");
    assert!(!git(root, &["diff", "--name-only", "--diff-filter=U"]).is_empty(), "the conflict is still unresolved");

    std::fs::write(root.join("README.md"), "# Both\n").unwrap();
    ok(&e, "git.stage", json!({"project_id":1,"paths":["README.md"]}));
    ok(&e, "git.commit", json!({"project_id":1,"message":"Merge side"}));
    assert_eq!(git(root, &["log", "-1", "--format=%p"]).split(' ').count(), 2, "the merge was concluded");
}

/// RA-205: an untracked directory is one status entry, and the dirty check still sees it.
#[test]
fn status_lists_an_untracked_directory_once() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let root = Path::new(&repo);
    std::fs::create_dir_all(root.join("venv/lib")).unwrap();
    for n in 0..50 {
        std::fs::write(root.join(format!("venv/lib/m{n}.py")), "x\n").unwrap();
    }
    let status = ok(&e, "git.status", json!({"project_id":1}));
    let paths: Vec<&str> = status["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["venv/"]);
    assert_eq!((status["truncated"].as_bool(), status["total"].as_u64()), (Some(false), Some(1)));
    let listed = ok(&e, "worktree.list", json!({"project_id":1}));
    assert_eq!(listed["worktrees"][0]["dirty"], true, "{listed}");
}

/// RA-110: a bus commit runs the user's prepare-commit-msg hook with git's arguments, before
/// commit-msg, and keeps what it adds. Concluding a merge, `git commit --no-verify` runs it
/// itself, so it runs exactly once there too, and so does commit-msg.
#[test]
fn prepare_commit_msg_hook_runs_once_on_every_bus_commit() {
    use std::os::unix::fs::PermissionsExt;
    let e = engine();
    let (ws, repo) = project(&e);
    let root = Path::new(&repo);
    git(root, &["switch", "-q", "-c", "side"]);
    std::fs::write(root.join("side.txt"), "side\n").unwrap();
    git(root, &["add", "side.txt"]);
    git(root, &["commit", "-q", "-m", "Side"]);
    git(root, &["switch", "-q", "main"]);
    let log = ws.path().join("hooks.log");
    let hook = |name: &str, body: String| {
        let path = root.join(".git/hooks").join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    hook("prepare-commit-msg", format!(
        "#!/bin/sh\necho \"prepare-commit-msg $2 $3\" >> '{}'\nprintf '\\nPrepared-by: hook\\n' >> \"$1\"\n", log.display()));
    hook("commit-msg", format!("#!/bin/sh\necho commit-msg >> '{}'\n", log.display()));
    let ran = || std::fs::read_to_string(&log).unwrap_or_default();

    std::fs::write(root.join("notes.txt"), "notes\n").unwrap();
    ok(&e, "git.commit", json!({"project_id":1,"message":"Add notes","all":true}));
    assert_eq!(ran(), "prepare-commit-msg message \ncommit-msg\n");
    assert_eq!(git(root, &["log", "-1", "--format=%B"]), "Add notes\n\nPrepared-by: hook");

    std::fs::remove_file(&log).unwrap();
    git(root, &["merge", "-q", "--no-commit", "--no-ff", "side"]);
    ok(&e, "git.commit", json!({"project_id":1,"message":"Merge side"}));
    assert_eq!(git(root, &["log", "-1", "--format=%p"]).split(' ').count(), 2, "the merge was concluded");
    let ran = ran();
    assert_eq!(ran.lines().filter(|line| line.starts_with("prepare-commit-msg message")).count(), 1, "{ran}");
    assert_eq!(ran.lines().filter(|line| *line == "commit-msg").count(), 1, "{ran}");
    assert_eq!(git(root, &["log", "-1", "--format=%B"]).matches("Prepared-by: hook").count(), 1);
}
