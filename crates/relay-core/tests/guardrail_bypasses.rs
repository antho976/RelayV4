//! Guardrail bypasses from the RA-093..RA-105 audit, each driven through the bus: what used to
//! slip past a gate, and the ordinary work that must still pass it.

use relay_bus::{Actor, ErrorKind, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git").arg("-C").arg(repo).args(args).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

struct Fixture {
    _root: tempfile::TempDir,
    engine: Arc<Engine>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        let repo = ws.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "bypass@relay.test"]);
        git(&repo, &["config", "user.name", "Bypass"]);
        std::fs::write(repo.join("README.md"), "relay\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let store = Store::open(&root.path().join("store/store.db"), false).unwrap();
        let engine = Engine::new(Instance::Test, store);
        ok(&engine, Actor::User, "workspace.create", json!({"path": ws}));
        ok(&engine, Actor::User, "project.add", json!({"workspace_id": 1, "path": repo}));
        Self { _root: root, engine }
    }

    /// A builder session: its name, its agent actor and its worktree.
    fn agent(&self) -> (String, Actor, PathBuf) {
        let session = ok(&self.engine, Actor::User, "session.create",
            json!({"project_id": 1, "provider": "codex", "role": "builder"}));
        let name = session["name"].as_str().unwrap().to_string();
        let worktree = PathBuf::from(session["worktree"].as_str().unwrap());
        (name.clone(), Actor::agent(&name), worktree)
    }

    fn protect(&self, paths: &[&str]) {
        ok(&self.engine, Actor::User, "guardrail.config.set", json!({"project_id": 1, "patch": {"protected_paths": paths}}));
    }
}

fn call(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Response {
    engine.dispatch(Request::new(actor, op, payload), Door::InProcess)
}

fn ok(engine: &Engine, actor: Actor, op: &str, payload: Value) -> Value {
    call(engine, actor, op, payload)
        .into_result()
        .unwrap_or_else(|error| panic!("{op} failed: {} {}", error.code, error.message))
}

fn code(response: &Response) -> String {
    response.error.as_ref().map(|error| error.code.clone()).unwrap_or_else(|| "ok".into())
}

fn long_text() -> String {
    (0..200).map(|line| format!("line-{line}\n")).collect()
}

/// RA-093: a file that is not UTF-8 (or not readable) used to measure as empty, so replacing
/// it never counted as destructive. Git can still restore a committed one.
#[test]
fn replacing_a_file_that_is_not_text_is_a_destructive_write() {
    let f = Fixture::new();
    let (name, agent, worktree) = f.agent();
    let write = |path: &str| call(&f.engine, agent.clone(), "guardrail.gate",
        json!({"session": name, "kind": "write", "path": path, "new_text": "x\n"}));
    let blob: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8 | 0x80).collect();
    std::fs::write(worktree.join("model.bin"), &blob).unwrap();
    let held = write("model.bin");
    assert_eq!(code(&held), "guardrail.destructive_write");
    assert_eq!(held.error.as_ref().unwrap().kind, ErrorKind::Held);

    git(&worktree, &["add", "model.bin"]);
    git(&worktree, &["commit", "--no-verify", "-qm", "model"]);
    assert!(write("model.bin").ok, "git can restore a committed binary");
}

/// RA-094: git cannot restore an ignored file, so gutting one is not "recoverable".
#[test]
fn an_ignored_file_is_not_recoverable() {
    let f = Fixture::new();
    let (name, agent, worktree) = f.agent();
    std::fs::write(worktree.join(".gitignore"), ".env\n").unwrap();
    std::fs::write(worktree.join(".env"), long_text()).unwrap();
    let held = call(&f.engine, agent, "guardrail.gate",
        json!({"session": name, "kind": "write", "path": ".env", "new_text": "KEY=1\n"}));
    assert_eq!(code(&held), "guardrail.destructive_write");
}

/// RA-095: a staged rename into a protected directory, and a protected path git would quote.
#[test]
fn the_commit_gate_sees_renamed_and_non_ascii_protected_paths() {
    let f = Fixture::new();
    f.protect(&["secret/**", "café/**"]);
    let (name, agent, worktree) = f.agent();
    let commit = || call(&f.engine, agent.clone(), "guardrail.gate", json!({"session": name, "kind": "commit"}));

    std::fs::write(worktree.join("notes.txt"), "a\nb\n").unwrap();
    git(&worktree, &["add", "notes.txt"]);
    git(&worktree, &["commit", "--no-verify", "-qm", "notes"]);
    std::fs::create_dir_all(worktree.join("secret")).unwrap();
    git(&worktree, &["mv", "notes.txt", "secret/notes.txt"]);
    assert_eq!(code(&commit()), "guardrail.protected_path", "a rename into secret/ touches secret/");
    git(&worktree, &["reset", "-q"]);
    std::fs::rename(worktree.join("secret/notes.txt"), worktree.join("notes.txt")).unwrap();

    std::fs::create_dir_all(worktree.join("café")).unwrap();
    std::fs::write(worktree.join("café/key"), "k\n").unwrap();
    git(&worktree, &["add", "café/key"]);
    assert_eq!(code(&commit()), "guardrail.protected_path", "a quoted path is still the path");
}

/// RA-101: protected paths and write roots judge where a write lands, however it is spelled.
#[test]
fn spellings_and_symlinks_do_not_reach_around_protected_paths_or_write_roots() {
    let f = Fixture::new();
    f.protect(&["secret/**"]);
    let (name, agent, worktree) = f.agent();
    let write = |path: &str| call(&f.engine, agent.clone(), "guardrail.gate",
        json!({"session": name, "kind": "write", "path": path, "new_text": "x\n"}));
    std::fs::create_dir_all(worktree.join("secret")).unwrap();
    for path in ["secret/key", "./secret/key", "secret//key", "secret/./key"] {
        assert_eq!(code(&write(path)), "guardrail.protected_path", "{path}");
    }
    let absolute = format!("{}/./secret//key", worktree.display());
    assert_eq!(code(&write(&absolute)), "guardrail.protected_path");

    // A link inside the worktree to the protected directory, and one to a protected file
    // that does not exist yet.
    std::os::unix::fs::symlink("secret", worktree.join("docs")).unwrap();
    std::os::unix::fs::symlink("secret/new.key", worktree.join("plain.txt")).unwrap();
    assert_eq!(code(&write("docs/key")), "guardrail.protected_path");
    assert_eq!(code(&write("plain.txt")), "guardrail.protected_path");
    // A link in scratch space that leads back into the repository.
    let scratch = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(worktree.join("secret"), scratch.path().join("in")).unwrap();
    assert_eq!(code(&write(&format!("{}/in/key", scratch.path().display()))), "guardrail.protected_path");
    // A link inside the worktree that leads out of every write root.
    std::os::unix::fs::symlink("/opt/relay-bypass-test", worktree.join("out")).unwrap();
    assert_eq!(code(&write("out/x")), "guardrail.write_root");

    assert!(write("src/./lib.rs").ok, "an ordinary path still passes");
    assert!(write(&format!("{}/plain", scratch.path().display())).ok, "scratch space still passes");
}

/// RA-103 / RA-104 end to end: the suggested `cd dist && rm -rf *` grants that line, not
/// `rm -rf /`; a quoted target is part of the grant.
#[test]
fn command_grants_cover_the_command_that_was_approved_and_nothing_wider() {
    let f = Fixture::new();
    let (name, agent, _) = f.agent();
    let exec = |command: &str| call(&f.engine, agent.clone(), "guardrail.gate",
        json!({"session": name, "kind": "exec", "command": command}));
    let grant = |value: &str| {
        let id = ok(&f.engine, agent.clone(), "guardrail.request", json!({
            "kind": "command", "value": value, "reason": "clean the build output", "scope": "session",
        }))["request"]["id"].as_i64().unwrap();
        ok(&f.engine, Actor::User, "guardrail.confirm", json!({"hold_id": id}));
    };
    let suggested = exec("cd dist && rm -rf *");
    assert_eq!(code(&suggested), "guardrail.command");
    let value = suggested.error.as_ref().unwrap().details.as_ref().unwrap()["exception"]["value"]
        .as_str().unwrap().to_string();
    grant(&value);
    assert!(exec("cd dist && rm -rf *").ok);
    for wider in ["rm -rf /", "rm -rf * /", "cd / && rm -rf *", "rm -rf ~"] {
        assert_eq!(code(&exec(wider)), "guardrail.command", "{wider}");
    }

    grant("rm -rf \"$TMPDIR/x\"");
    assert!(exec("rm -rf \"$TMPDIR/x\"").ok);
    assert_eq!(code(&exec("rm -rf \"/\"")), "guardrail.command");
}

/// RA-096 / RA-097 / RA-098 end to end, and a search for the op names still passes.
#[test]
fn the_exec_gate_sees_envelopes_wrappers_and_not_searches() {
    let f = Fixture::new();
    let (name, agent, _) = f.agent();
    let exec = |command: &str| call(&f.engine, agent.clone(), "guardrail.gate",
        json!({"session": name, "kind": "exec", "command": command}));
    for command in [
        "relay cmd '{\"actor\":\"user\",\"op\":\"guardrail.confirm\",\"payload\":{\"hold_id\":1}}'",
        "env -i relay q task.list",
        "bash -c 'relay --actor user q task.list'",
        "python3 -c \"import os; os.system('relay --actor user q guardrail.confirm {}')\"",
    ] {
        assert_eq!(code(&exec(command)), "guardrail.self_approval", "{command}");
    }
    for command in ["echo 'git reset --hard' | sh", "python3 -c \"import os; os.system('rm -rf build')\""] {
        assert_eq!(code(&exec(command)), "guardrail.command", "{command}");
    }
    for command in [
        "rg -n guardrail.confirm crates/relay-core", "echo rm -rf build", "ls # rm -rf /",
        "git commit -m \"$(cat <<'EOF'\nNo more rm -rf build by hand\nEOF\n)\"",
    ] {
        assert!(exec(command).ok, "{command}: {:?}", exec(command).error);
    }
}

/// RA-105: a grant ends with its session.
#[test]
fn a_closed_sessions_grants_are_no_longer_active() {
    let f = Fixture::new();
    f.protect(&["secret/**"]);
    let (name, agent, _) = f.agent();
    let id = ok(&f.engine, agent, "guardrail.request", json!({
        "kind": "path", "value": "secret/key", "reason": "rotate the key", "scope": "session",
    }))["request"]["id"].as_i64().unwrap();
    ok(&f.engine, Actor::User, "guardrail.confirm", json!({"hold_id": id}));
    let active = ok(&f.engine, Actor::User, "guardrail.requests.list", json!({"state": "active"}));
    assert_eq!(active["requests"].as_array().unwrap().len(), 1);

    ok(&f.engine, Actor::User, "session.close", json!({"session": name, "remove_worktree": false}));
    let active = ok(&f.engine, Actor::User, "guardrail.requests.list", json!({"state": "active"}));
    assert_eq!(active["requests"], json!([]));
    let all = ok(&f.engine, Actor::User, "guardrail.requests.list", json!({}));
    assert_eq!(all["requests"][0]["active"], false);
    assert_eq!(ok(&f.engine, Actor::User, "guardrail.request.get", json!({"request_id": id}))["active"], false);
}

/// RA-102: answered holds can be pruned; open ones and a live session's grants stay.
#[test]
fn prune_holds_keeps_open_holds_and_live_grants() {
    let f = Fixture::new();
    ok(&f.engine, Actor::User, "guardrail.config.set", json!({"project_id": 1, "patch": {
        "shape_gates": [{"path": "manifest.json", "validator": "json_non_empty_object"}],
    }}));
    let (name, agent, _) = f.agent();
    let shape = || call(&f.engine, agent.clone(), "guardrail.gate",
        json!({"session": name, "kind": "write", "path": "manifest.json", "new_text": "{}"}));
    let hold_id = |response: Response| response.error.unwrap().confirm.unwrap().payload["hold_id"].as_i64().unwrap();
    let rejected = hold_id(shape());
    ok(&f.engine, Actor::User, "guardrail.reject", json!({"hold_id": rejected}));
    let open = hold_id(call(&f.engine, agent.clone(), "guardrail.gate",
        json!({"session": name, "kind": "write", "path": "manifest.json", "new_text": "[]"})));
    let grant = ok(&f.engine, agent, "guardrail.request", json!({
        "kind": "command", "value": "rm -rf build", "reason": "clean", "scope": "session",
    }))["request"]["id"].as_i64().unwrap();
    ok(&f.engine, Actor::User, "guardrail.confirm", json!({"hold_id": grant}));

    let pruned = {
        let mut conn = f.engine.store.lock();
        let tx = conn.transaction().unwrap();
        let (pruned, _) = relay_core::guardrail::prune_holds(&tx, "9999-01-01T00:00:00Z").unwrap();
        tx.commit().unwrap();
        pruned
    };
    assert_eq!(pruned, 1);
    let left: Vec<i64> = ok(&f.engine, Actor::User, "guardrail.holds.list", json!({"open_only": false}))["holds"]
        .as_array().unwrap().iter().map(|hold| hold["id"].as_i64().unwrap()).collect();
    assert!(!left.contains(&rejected) && left.contains(&open) && left.contains(&grant), "{left:?}");
}

/// RA-100: the dry runs read files and run git with no transaction open.
#[test]
fn the_dry_runs_run_unlocked() {
    let f = Fixture::new();
    assert!(f.engine.runs_unlocked("guardrail.check"));
    assert!(f.engine.runs_unlocked("guardrail.explain"));
}
