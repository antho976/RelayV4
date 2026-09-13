//! Phase 3: worktrees, PTY sessions, teardown (SPEC §16 "spawn/kill N sessions, assert zero
//! orphans"), token binding, crash recovery. All through the bus.

use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::socket::{Client, Line, SocketServer};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn call(e: &Engine, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}
fn ok(e: &Engine, op: &str, payload: Value) -> Value {
    let r = call(e, op, payload);
    match r.into_result() {
        Ok(v) => v,
        Err(err) => panic!("{op} failed: {} {}", err.code, err.message),
    }
}
fn code(r: Response) -> String {
    r.error.expect("expected error").code
}

fn git(repo: &Path, args: &[&str]) {
    let st = Command::new("git").arg("-C").arg(repo).args(args).status().unwrap();
    assert!(st.success(), "git {args:?}");
}

/// A workspace with one real git repo (one commit on `main`) and a store beside it.
struct Fixture {
    _root: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    engine: Arc<Engine>,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let ws = root.join("ws");
    let repo = ws.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README.md"), "hi\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Store::open(&root.join("store").join("store.db"), false).unwrap();
    let engine = Engine::new(Instance::Test, store);
    ok(&engine, "workspace.create", json!({"path": ws}));
    ok(&engine, "project.add", json!({"workspace_id": 1, "path": repo}));
    Fixture { _root: tmp, root, repo, engine }
}

fn head(repo: &Path, reference: &str) -> String {
    let out = Command::new("git").arg("-C").arg(repo)
        .args(["rev-parse", reference]).output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn advancing_remote(f: &Fixture, tracking: bool) -> PathBuf {
    let origin = f.root.join("remote");
    git(&f.root, &["clone", f.repo.to_str().unwrap(), origin.to_str().unwrap()]);
    git(&origin, &["config", "user.email", "t@t"]);
    git(&origin, &["config", "user.name", "t"]);
    git(&f.repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&f.repo, &["fetch", "origin"]);
    if tracking {
        git(&f.repo, &["branch", "--set-upstream-to=origin/main", "main"]);
    }
    std::fs::write(origin.join("latest.txt"), "remote change\n").unwrap();
    git(&origin, &["add", "latest.txt"]);
    git(&origin, &["commit", "-qm", "remote advance"]);
    origin
}

#[test]
fn new_sessions_fetch_remote_commits_without_touching_primary() {
    for tracking in [true, false] {
        let f = fixture();
        let origin = advancing_remote(&f, tracking);
        let original = head(&f.repo, "HEAD");
        std::fs::write(f.repo.join("README.md"), "uncommitted primary work\n").unwrap();
        for round in 0..2 {
            git(&origin, &["commit", "--allow-empty", "-qm", &format!("advance {round}")]);
            let session = ok(&f.engine, "session.create", json!({"project_id":1,"provider":"codex"}));
            let checkout = Path::new(session["worktree"].as_str().unwrap());
            assert_eq!(head(checkout, "HEAD"), head(&origin, "HEAD"));
            assert!(checkout.join("latest.txt").exists());
            assert_eq!(head(&f.repo, "HEAD"), original);
            assert_eq!(std::fs::read_to_string(f.repo.join("README.md")).unwrap(), "uncommitted primary work\n");
        }
    }
}

#[test]
fn new_worktree_defaults_fetch_but_explicit_revisions_and_existing_branches_are_preserved() {
    let f = fixture();
    let origin = advancing_remote(&f, true);
    let original = head(&f.repo, "HEAD");
    let worktree = ok(&f.engine, "worktree.create", json!({"project_id":1,"branch":"relay/fresh"}));
    assert_eq!(head(Path::new(worktree["path"].as_str().unwrap()), "HEAD"), head(&origin, "HEAD"));
    git(&f.repo, &["remote", "set-url", "origin", f.root.join("missing").to_str().unwrap()]);
    let pinned = ok(&f.engine, "worktree.create", json!({"project_id":1,"branch":"relay/pinned","from":original}));
    assert_eq!(head(Path::new(pinned["path"].as_str().unwrap()), "HEAD"), original);
    git(&f.repo, &["branch", "relay/existing", &original]);
    let existing = ok(&f.engine, "session.create", json!({"project_id":1,"provider":"codex","branch":"relay/existing"}));
    assert_eq!(head(Path::new(existing["worktree"].as_str().unwrap()), "HEAD"), original);
    let pair = ok(&f.engine, "session.create", json!({"project_id":1,"provider":"claude","role":"reviewer","pair_with":existing["name"]}));
    assert_eq!(pair["worktree"], existing["worktree"]);
}

#[test]
fn new_sessions_refuse_failed_fetch_and_divergence_without_stale_fallback() {
    let f = fixture();
    advancing_remote(&f, true);
    git(&f.repo, &["commit", "--allow-empty", "-qm", "local advance"]);
    let original = head(&f.repo, "HEAD");
    assert_eq!(code(call(&f.engine, "session.create", json!({"project_id":1,"provider":"codex"}))), "git.base_diverged");
    git(&f.repo, &["remote", "set-url", "origin", f.root.join("missing").to_str().unwrap()]);
    assert_eq!(code(call(&f.engine, "session.create", json!({"project_id":1,"provider":"codex"}))), "git.fetch_failed");
    assert_eq!(head(&f.repo, "HEAD"), original);
    assert_eq!(ok(&f.engine, "worktree.list", json!({"project_id":1}))["worktrees"].as_array().unwrap().len(), 1);
    assert!(ok(&f.engine, "session.list", json!({"project_id":1}))["sessions"].as_array().unwrap().is_empty());
}

#[test]
fn new_sessions_preserve_local_ahead_base_and_reject_missing_base() {
    let f = fixture();
    let origin = advancing_remote(&f, true);
    git(&f.repo, &["fetch", "origin"]);
    git(&f.repo, &["merge", "--ff-only", "origin/main"]);
    git(&f.repo, &["commit", "--allow-empty", "-qm", "local only"]);
    let local = head(&f.repo, "HEAD");
    assert_ne!(local, head(&origin, "HEAD"));
    let session = ok(&f.engine, "session.create", json!({"project_id":1,"provider":"codex"}));
    assert_eq!(head(Path::new(session["worktree"].as_str().unwrap()), "HEAD"), local);
    ok(&f.engine, "project.update", json!({"project_id":1,"base_branch":"missing"}));
    assert_eq!(code(call(&f.engine, "session.create", json!({"project_id":1,"provider":"codex"}))), "git.base_missing");
}

/// A stand-in provider CLI: greets, echoes lines, exits 3 on "exit". Its `sleep` child is
/// what the teardown test looks for.
fn fake_provider(dir: &Path, with_child: bool) -> PathBuf {
    let p = dir.join("fake-claude.sh");
    let body = if with_child {
        "#!/bin/sh\necho hello-from-pty\nsleep 300 &\necho $! > \"$PWD/child.pid\"\nwhile IFS= read -r line; do echo \"echo:$line\"; [ \"$line\" = exit ] && exit 3; done\n"
    } else {
        "#!/bin/sh\necho hello-from-pty\nwhile IFS= read -r line; do echo \"echo:$line\"; [ \"$line\" = exit ] && exit 3; done\n"
    };
    std::fs::write(&p, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn fake_discovery_provider(dir: &Path, binary: &str, version: &str) -> PathBuf {
    let path = dir.join(binary);
    let auth = if binary == "claude" {
        "echo '{\"loggedIn\":true,\"email\":\"agent@example.test\"}'"
    } else {
        "echo 'Logged in using ChatGPT'"
    };
    let body = format!(
        "#!/bin/sh\ncase \"${{1:-}}\" in\n  --version) echo '{version}'; exit 0;;\n  auth|login) {auth}; exit 0;;\nesac\nprintf '%s\\n' \"$@\" > \"$PWD/.relay/provider-args.txt\"\necho hello-from-pty\nwhile IFS= read -r line; do echo \"echo:$line\"; done\n"
    );
    std::fs::write(&path, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn alive(pid: i64) -> bool {
    relay_core::pty::pid_alive(pid as u32) && std::fs::read_to_string(format!("/proc/{pid}/stat")).map(|s| !s.contains(") Z ")).unwrap_or(false)
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_pid(path: &Path) -> i64 {
    let mut pid = None;
    wait_until("child pid written", || {
        pid = std::fs::read_to_string(path).ok().and_then(|value| value.trim().parse().ok());
        pid.is_some()
    });
    pid.expect("wait completed with a parsed pid")
}

#[test]
fn worktree_ops() {
    let f = fixture();
    let e = &f.engine;
    let l = ok(e, "worktree.list", json!({"project_id": 1}));
    let wts = l["worktrees"].as_array().unwrap();
    assert_eq!(wts.len(), 1);
    assert_eq!(wts[0]["branch"], "main");
    assert_eq!(wts[0]["path"], f.repo.display().to_string());
    assert_eq!(wts[0]["dirty"], false);
    // create
    let w = ok(e, "worktree.create", json!({"project_id": 1, "branch": "relay/feature-x"}));
    assert_eq!(w["branch"], "relay/feature-x");
    let path = PathBuf::from(w["path"].as_str().unwrap());
    assert!(path.starts_with(f.repo.join(".relay/worktrees")));
    assert!(path.join("README.md").is_file());
    let excl = std::fs::read_to_string(f.repo.join(".git/info/exclude")).unwrap();
    assert!(excl.lines().any(|l| l == ".relay/"));
    assert_eq!(code(call(e, "worktree.create", json!({"project_id": 1, "branch": "relay/feature-x"}))), "worktree.exists");
    assert_eq!(ok(e, "worktree.list", json!({"project_id": 1}))["worktrees"].as_array().unwrap().len(), 2);
    // dirty detection through gix, no spawn
    std::fs::write(path.join("scratch.txt"), "x").unwrap();
    let l = ok(e, "worktree.list", json!({"project_id": 1}));
    let mine = l["worktrees"].as_array().unwrap().iter().find(|w| w["path"] == path.display().to_string()).unwrap();
    assert_eq!(mine["dirty"], true);
    // build output shows in disk, is purged on remove
    std::fs::create_dir_all(path.join("build")).unwrap();
    std::fs::write(path.join("build/big.bin"), vec![0u8; 512 * 1024]).unwrap();
    let d = ok(e, "worktree.disk", json!({"project_id": 1}));
    let mine = d["worktrees"].as_array().unwrap().iter().find(|w| w["path"] == path.display().to_string()).unwrap();
    assert!(mine["build_mb"].as_f64().unwrap() > 0.4, "{mine}");
    let r = ok(e, "worktree.remove", json!({"project_id": 1, "path": path}));
    assert!(r["freed_mb"].as_f64().unwrap() > 0.4);
    assert!(!path.exists());
    assert_eq!(ok(e, "worktree.list", json!({"project_id": 1}))["worktrees"].as_array().unwrap().len(), 1);
    // the branch survives removal (SPEC §8)
    let out = Command::new("git").arg("-C").arg(&f.repo).args(["branch", "--list", "relay/feature-x"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("relay/feature-x"));
    // agents may not create/remove worktrees
    let r = e.dispatch(Request::new(Actor::agent("x"), "worktree.create", json!({"project_id": 1, "branch": "relay/y"})), Door::InProcess);
    assert_eq!(code(r), "actor.allowlist");
}

#[test]
fn launch_passes_configured_and_memory_write_roots_to_both_providers() {
    for provider in ["claude", "codex"] {
        let f = fixture();
        let binary = fake_discovery_provider(&f.root, provider, "fixture 1.0");
        ok(&f.engine, "settings.set", json!({"path":format!("providers.{provider}.path"),"value":binary}));
        let extra = f.root.join("agent notes");
        std::fs::create_dir_all(&extra).unwrap();
        ok(&f.engine, "settings.set", json!({"path":"guardrails.allowed_write_roots","value":[extra]}));
        let session = ok(&f.engine, "session.create", json!({"project_id":1,"provider":provider}));
        let worktree = PathBuf::from(session["worktree"].as_str().unwrap());
        ok(&f.engine, "session.spawn", json!({"session":session["name"]}));
        let captured = worktree.join(".relay/provider-args.txt");
        wait_until("provider argv", || captured.is_file());
        let args = std::fs::read_to_string(captured).unwrap();
        let args: Vec<_> = args.lines().collect();
        let cfg = relay_core::guardrail::config(&f.engine.store.lock(), Some(1)).unwrap();
        let roots = relay_core::guardrail::write_roots(&cfg, &worktree);
        assert!(roots.iter().any(|path| path.ends_with("memories")));
        assert!(roots.iter().any(|path| path.ends_with("memory")));
        for root in roots.iter().skip(1) {
            assert!(args.windows(2).any(|pair| pair == ["--add-dir", root.to_str().unwrap()]), "{provider}: missing {root:?}");
            let verdict = f.engine.dispatch(Request::new(Actor::agent(session["name"].as_str().unwrap()), "guardrail.check", json!({
                "project_id":1,"kind":"write","path":root.join("note.md"),"new_text":"note"
            })), Door::InProcess).into_result().unwrap();
            assert_eq!(verdict["verdict"], "allow");
        }
        ok(&f.engine, "session.close", json!({"session":session["name"]}));
    }
}

#[test]
fn closing_a_provider_that_ignores_term_is_bounded_and_preserves_its_worktree() {
    let f = fixture();
    let provider = fake_provider(&f.root, false);
    std::fs::write(&provider, "#!/bin/sh\ntrap '' TERM\necho ready\nwhile IFS= read -r line; do :; done\n").unwrap();
    ok(&f.engine, "settings.set", json!({"path":"providers.claude.path","value":provider}));
    let session = ok(&f.engine, "session.create", json!({"project_id":1,"provider":"claude"}));
    let spawned = ok(&f.engine, "session.spawn", json!({"session":session["name"]}));
    wait_until("ignoring TERM", || ok(&f.engine,"session.scrollback",json!({"session":session["name"]}))["text"].as_str().unwrap().contains("ready"));
    let start = Instant::now();
    ok(&f.engine, "session.close", json!({"session":session["name"],"remove_worktree":false}));
    assert!(start.elapsed() < Duration::from_secs(1), "close took {:?}", start.elapsed());
    assert!(!alive(spawned["pid"].as_i64().unwrap()));
    assert!(Path::new(session["worktree"].as_str().unwrap()).join("README.md").is_file());
}

#[test]
fn session_create_and_worktree_ownership() {
    let f = fixture();
    let e = &f.engine;
    let s = ok(e, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let name = s["name"].as_str().unwrap().to_string();
    assert!(name.split('-').count() == 2, "{name}");
    assert_eq!(s["state"], "created");
    assert_eq!(s["branch"], format!("relay/{name}"));
    let wt = PathBuf::from(s["worktree"].as_str().unwrap());
    assert!(wt.join("README.md").is_file());
    assert_eq!(s["role"], "builder");
    // the worktree is listed as owned; cannot be removed while owned
    let l = ok(e, "worktree.list", json!({"project_id": 1}));
    let mine = l["worktrees"].as_array().unwrap().iter().find(|w| w["path"] == wt.display().to_string()).unwrap();
    assert_eq!(mine["session"], name);
    assert_eq!(code(call(e, "worktree.remove", json!({"project_id": 1, "path": wt}))), "worktree.owned");
    // primary checkout
    let s2 = ok(e, "session.create", json!({"project_id": 1, "provider": "codex", "worktree": "primary", "role": "reviewer"}));
    assert_eq!(s2["worktree"], f.repo.display().to_string());
    assert_eq!(s2["branch"], "main");
    assert_ne!(s2["name"], name);
    // list / get
    assert_eq!(ok(e, "session.list", json!({})).as_object().unwrap()["sessions"].as_array().unwrap().len(), 2);
    assert_eq!(ok(e, "session.get", json!({"session": name}))["id"], 1);
    assert_eq!(code(call(e, "session.get", json!({"session": "nope-nope"}))), "session.not_found");
    // close: worktree gone, state closed, name free again for listing purposes
    let r = ok(e, "session.close", json!({"session": name}));
    assert!(r["freed_mb"].as_f64().unwrap() >= 0.0);
    assert!(!wt.exists());
    assert_eq!(code(call(e, "session.get", json!({"session": name}))), "session.not_found");
    let all = ok(e, "session.list", json!({"include_closed": true}));
    assert_eq!(all["sessions"].as_array().unwrap().len(), 2);
    // closing the primary-checkout session never removes the repo
    ok(e, "session.close", json!({"session": s2["name"]}));
    assert!(f.repo.join("README.md").is_file());
    // spawn without a provider on PATH / configured is a typed unavailable
    let s3 = ok(e, "session.create", json!({"project_id": 1, "provider": "codex"}));
    ok(e, "settings.set", json!({"path": "providers.codex.path", "value": "/nonexistent/codex"}));
    assert_eq!(code(call(e, "session.spawn", json!({"session": s3["name"]}))), "provider.not_installed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_lifecycle_over_socket() {
    let f = fixture();
    let e = f.engine.clone();
    let prov = fake_provider(&f.root, false);
    ok(&e, "settings.set", json!({"path": "providers.claude.path", "value": prov}));
    let sockdir = f.root.join("run");
    let server = SocketServer::start_in(e.clone(), sockdir.clone()).await.unwrap();

    let s = ok(&e, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let name = s["name"].as_str().unwrap().to_string();
    // events subscriber
    let mut sub = Client::connect(&server.path).await.unwrap();
    sub.call(&Request::new(Actor::User, "bus.subscribe", json!({"events": ["session.*"]})), |_| {}).await.unwrap();

    let spawn_req = Request::new(Actor::User, "session.spawn", json!({"session": name}));
    let spawn_id = spawn_req.id;
    let s = e.dispatch(spawn_req, Door::InProcess).into_result().unwrap();
    assert_eq!(s["state"], "running");
    let pid = s["pid"].as_i64().unwrap();
    assert!(alive(pid));
    assert_eq!(code(call(&e, "session.spawn", json!({"session": name}))), "session.already_spawned");

    // token binding on the socket door
    let token: String = e.store.lock().query_row("SELECT token FROM sessions WHERE name = ?1", [&name], |r| r.get(0)).unwrap();
    let mut c = Client::connect(&server.path).await.unwrap();
    let r = c.call(&Request::new(Actor::agent(&name), "session.get", json!({"session": name})).with_token("wrong"), |_| {}).await.unwrap();
    assert_eq!(code(r), "bus.actor");
    let r = c.call(&Request::new(Actor::agent(&name), "session.get", json!({"session": name})).with_token(&token), |_| {}).await.unwrap();
    assert!(r.ok, "{:?}", r.error);
    // an agent sees the metadata of a peer in its own project (D106) but not its private
    // surfaces, and nothing at all outside the project
    let other = ok(&e, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let r = c.call(&Request::new(Actor::agent(&name), "session.get", json!({"session": other["name"]})).with_token(&token), |_| {}).await.unwrap();
    assert!(r.ok, "{:?}", r.error);
    let r = c.call(&Request::new(Actor::agent(&name), "session.scrollback", json!({"session": other["name"]})).with_token(&token), |_| {}).await.unwrap();
    assert_eq!(code(r), "actor.scope", "a peer's scrollback stays private");
    let r = c.call(&Request::new(Actor::agent(&name), "session.brief", json!({"session": other["name"]})).with_token(&token), |_| {}).await.unwrap();
    assert_eq!(code(r), "actor.scope", "a peer's brief stays private");
    // ...nor close one (user-only)
    let r = c.call(&Request::new(Actor::agent(&name), "session.close", json!({"session": name})).with_token(&token), |_| {}).await.unwrap();
    assert_eq!(code(r), "actor.allowlist");

    // attach: catch-up + live frames
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut term = Client::connect(&server.path).await.unwrap();
    let r = term.call(&Request::new(Actor::User, "session.attach", json!({"session": name})), |_| {}).await.unwrap();
    assert!(r.ok, "{:?}", r.error);
    let mut seen = Vec::new();
    let mut last_seq = 0u64;
    let mut epoch = 0u64;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !String::from_utf8_lossy(&seen).contains("hello-from-pty") {
        assert!(Instant::now() < deadline, "no greeting; got {:?}", String::from_utf8_lossy(&seen));
        if let Ok(Ok(Some(Line::Frame(fr)))) = tokio::time::timeout(Duration::from_secs(2), term.next()).await {
            assert_eq!(fr.stream, "pty");
            assert_eq!(fr.session.as_deref(), Some(name.as_str()));
            epoch = fr.epoch.unwrap();
            assert!(fr.seq >= last_seq);
            last_seq = fr.seq;
            seen.extend(b64.decode(fr.data.as_str().unwrap()).unwrap());
        }
    }
    assert_eq!(epoch, 1);
    // input → echo
    ok(&e, "session.input", json!({"session": name, "data": "ping\n"}));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !String::from_utf8_lossy(&seen).contains("echo:ping") {
        assert!(Instant::now() < deadline, "no echo; got {:?}", String::from_utf8_lossy(&seen));
        if let Ok(Ok(Some(Line::Frame(fr)))) = tokio::time::timeout(Duration::from_secs(2), term.next()).await {
            last_seq = fr.seq;
            seen.extend(b64.decode(fr.data.as_str().unwrap()).unwrap());
        }
    }
    // resize + scrollback
    ok(&e, "session.resize", json!({"session": name, "cols": 200, "rows": 50}));
    let sb = ok(&e, "session.scrollback", json!({"session": name}));
    assert!(sb["text"].as_str().unwrap().contains("hello-from-pty"));
    assert!(sb["text"].as_str().unwrap().contains("echo:ping"));
    assert_eq!(sb["epoch"], 1);
    assert!(sb["seq"].as_u64().unwrap() >= last_seq);
    // a second attach from (epoch, seq) replays only what came after
    let mut term2 = Client::connect(&server.path).await.unwrap();
    term2.call(&Request::new(Actor::User, "session.attach", json!({"session": name, "epoch": epoch, "from_seq": sb["seq"]})), |_| {}).await.unwrap();
    ok(&e, "session.input", json!({"session": name, "data": "again\n"}));
    let mut seen2 = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !String::from_utf8_lossy(&seen2).contains("echo:again") {
        assert!(Instant::now() < deadline, "no again; got {:?}", String::from_utf8_lossy(&seen2));
        if let Ok(Ok(Some(Line::Frame(fr)))) = tokio::time::timeout(Duration::from_secs(2), term2.next()).await {
            seen2.extend(b64.decode(fr.data.as_str().unwrap()).unwrap());
        }
    }
    assert!(!String::from_utf8_lossy(&seen2).contains("hello-from-pty"), "catch-up replayed history it should have skipped: {:?}", String::from_utf8_lossy(&seen2));
    // detach stops frames on term2
    term2.call(&Request::new(Actor::User, "session.detach", json!({"session": name})), |_| {}).await.unwrap();

    // process exit → session.changed(exited, 3) event + completion audit row
    ok(&e, "session.input", json!({"session": name, "data": "exit\n"}));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut exited = false;
    while !exited {
        assert!(Instant::now() < deadline, "no exit event");
        if let Ok(Ok(Some(Line::Event(ev)))) = tokio::time::timeout(Duration::from_secs(2), sub.next()).await {
            if ev.ev == "session.changed" && ev.payload["name"] == name && ev.payload["state"] == "exited" {
                assert_eq!(ev.payload["exit_code"], 3);
                assert_eq!(ev.actor, Actor::System);
                exited = true;
            }
        }
    }
    wait_until("pid gone", || !alive(pid));
    let rows = ok(&e, "audit.list", json!({"op_prefix": "session.spawn.completed"}));
    let row = &rows["rows"][0];
    assert_eq!(row["parent_req"], spawn_id.to_string());
    assert_eq!(row["actor"], "system");
    assert_eq!(row["result_summary"]["exit_code"], 3);
    assert_eq!(code(call(&e, "session.input", json!({"session": name, "data": "x"}))), "session.exited");
    // scrollback survives exit; close tears it all down
    assert!(ok(&e, "session.scrollback", json!({"session": name}))["text"].as_str().unwrap().contains("echo:again"));
    let wt = PathBuf::from(s["worktree"].as_str().unwrap());
    ok(&e, "session.close", json!({"session": name}));
    assert!(!wt.exists());
    let r = c.call(&Request::new(Actor::agent(&name), "session.get", json!({"session": name})).with_token(&token), |_| {}).await.unwrap();
    assert_eq!(code(r), "bus.actor", "token revoked with the session");
    drop(server);
}

/// D148: input reaches the PTY without a transaction, and still carries the one state change
/// that a keystroke owes the store — the idle→running edge — on a worker.
#[test]
fn input_bypasses_the_store_but_still_wakes_an_idle_session() {
    let f = fixture();
    let e = &f.engine;
    let prov = fake_provider(&f.root, false);
    ok(e, "settings.set", json!({"path": "providers.claude.path", "value": prov}));
    let s = ok(e, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let name = s["name"].as_str().unwrap().to_string();

    // An unspawned session has no PTY to resolve from memory, so the request must still fall
    // through to the handler and get the exact typed refusal.
    assert_eq!(
        code(call(e, "session.input", json!({"session": name, "data": "x"}))),
        "session.not_spawned"
    );
    assert_eq!(
        code(call(e, "session.input", json!({"session": "no-such-session", "data": "x"}))),
        "session.not_found"
    );

    ok(e, "session.spawn", json!({"session": &name}));
    let session_id: i64 = e.store.lock()
        .query_row("SELECT id FROM sessions WHERE name = ?1", [&name], |r| r.get(0))
        .unwrap();

    let state = || -> String {
        e.store.lock()
            .query_row("SELECT state FROM sessions WHERE id = ?1", [session_id], |r| r.get(0))
            .unwrap()
    };
    // The provider says it went idle; the live PTY learns it too.
    e.store.lock()
        .execute("UPDATE sessions SET state='idle' WHERE id=?1", [session_id])
        .unwrap();
    e.pty(session_id).expect("spawned session has a PTY").set_idle(true);

    ok(e, "session.input", json!({"session": &name, "data": "wake\n"}));
    wait_until("idle session back to running", || state() == "running");

    // Every keystroke after the edge is pure memory: no further writes, and the echo still lands.
    ok(e, "session.resize", json!({"session": &name, "cols": 120, "rows": 40}));
    ok(e, "session.input", json!({"session": &name, "data": "again\n"}));
    wait_until("echo reaches the scrollback", || {
        ok(e, "session.scrollback", json!({"session": &name}))["text"]
            .as_str()
            .unwrap()
            .contains("echo:again")
    });
    assert_eq!(state(), "running");
    ok(e, "session.close", json!({"session": &name}));
}

/// SPEC §16: spawn/kill N sessions, assert zero orphans — including the children's children.
#[test]
fn teardown_leaves_no_orphans() {
    let f = fixture();
    let e = &f.engine;
    let prov = fake_provider(&f.root, true);
    ok(e, "settings.set", json!({"path": "providers.claude.path", "value": prov}));
    let mut pids = Vec::new();
    let mut names = Vec::new();
    for _ in 0..5 {
        let s = ok(e, "session.create", json!({"project_id": 1, "provider": "claude"}));
        let name = s["name"].as_str().unwrap().to_string();
        let s = ok(e, "session.spawn", json!({"session": name}));
        let pid = s["pid"].as_i64().unwrap();
        let wt = PathBuf::from(s["worktree"].as_str().unwrap());
        let child = wait_for_pid(&wt.join("child.pid"));
        assert!(alive(pid) && alive(child));
        pids.push((pid, child));
        names.push(name);
    }
    assert_eq!(e.live_pty_count(), 5);
    // close three through the bus, then shut the engine down for the rest
    for name in &names[..3] {
        ok(e, "session.close", json!({"session": name}));
    }
    for (pid, child) in &pids[..3] {
        wait_until("closed session's processes gone", || !alive(*pid) && !alive(*child));
    }
    assert_eq!(e.live_pty_count(), 2);
    e.shutdown();
    for (pid, child) in &pids[3..] {
        wait_until("shut-down session's processes gone", || !alive(*pid) && !alive(*child));
    }
    assert_eq!(e.live_pty_count(), 0);
    let store = e.store.path().display().to_string();
    assert!(relay_core::pty::relay_children("test", &store).is_empty(), "orphans left behind");
    // the two shut-down sessions are restorable, the closed three closed
    let l = ok(e, "session.list", json!({"include_closed": true}));
    let mut states: Vec<String> = l["sessions"].as_array().unwrap().iter().map(|s| s["state"].as_str().unwrap().to_string()).collect();
    states.sort();
    assert_eq!(states, vec!["closed", "closed", "closed", "restorable", "restorable"]);
}

#[test]
fn recovery_reaps_orphans_and_fscks() {
    let f = fixture();
    let prov = fake_provider(&f.root, true);
    ok(&f.engine, "settings.set", json!({"path": "providers.claude.path", "value": prov}));
    let s = ok(&f.engine, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let name = s["name"].as_str().unwrap().to_string();
    let s = ok(&f.engine, "session.spawn", json!({"session": name}));
    let pid = s["pid"].as_i64().unwrap();
    let wt = PathBuf::from(s["worktree"].as_str().unwrap());
    let child = wait_for_pid(&wt.join("child.pid"));
    // a session row that claims to be running with a dead pid, and a task stuck in active
    f.engine.store.lock().execute_batch(
        "INSERT INTO sessions(name, project_id, provider, role, branch, worktree, state, pid, token, epoch, created_at, updated_at)
         VALUES ('ghost-heron', 1, 'claude', 'builder', 'x', '/tmp', 'running', 999999, 't', 1, 'now', 'now');
         INSERT INTO tasks(project_id, title, col, created_at, updated_at) VALUES (1, 'stuck', 'active', 'now', 'now');").unwrap();
    // "crash": leak the PTY handle so the child outlives the engine, then drop the engine
    let leaked = f.engine.take_pty(1).unwrap();
    std::mem::forget(leaked);
    let store_path = f.engine.store.path().to_path_buf();
    drop(f.engine);
    assert!(alive(pid) && alive(child), "the orphan should have survived the engine");

    // a new engine on the same store
    let e2 = Engine::new(Instance::Test, Store::open(&store_path, false).unwrap());
    let report = relay_core::recovery::run(&e2).unwrap();
    assert!(report.reaped_pids.contains(&pid), "{report:?}");
    wait_until("orphan and its child gone", || !alive(pid) && !alive(child));
    assert!(report.fsck_fixes.iter().any(|x| x.contains(&name) && x.contains("restorable")), "{report:?}");
    assert!(report.fsck_fixes.iter().any(|x| x.contains("ghost-heron") && x.contains("dead")), "{report:?}");
    assert_eq!(report.tasks_reset_offered, vec![1]);
    let s = ok(&e2, "session.get", json!({"session": name}));
    assert_eq!(s["state"], "restorable");
    assert!(s["pid"].is_null());
    let last = ok(&e2, "app.recovery.last", json!({}));
    assert_eq!(last["reaped_pids"], json!(report.reaped_pids));
    // dirty pooled worktree gets flagged (child.pid is untracked)
    assert!(report.dirty_worktrees.iter().any(|d| d == &wt.display().to_string()), "{report:?}");
}

#[test]
fn recovery_releases_legacy_claims_but_preserves_new_work() {
    let f = fixture();
    let create = || ok(&f.engine, "session.create", json!({"project_id":1,"provider":"codex"}));
    let done = create();
    let closed = create();
    let working = create();
    let archived = create();
    f.engine.dispatch(Request::new(Actor::agent(done["name"].as_str().unwrap()),
        "session.done", json!({"summary":"Finished before upgrade"})), Door::InProcess).into_result().unwrap();
    ok(&f.engine, "session.close", json!({"session":closed["name"],"remove_worktree":false}));
    let task = ok(&f.engine,"task.create",json!({"project_id":1,"title":"Completed before audit retention"}));
    ok(&f.engine,"task.dispatch",json!({"task_id":task["id"],"session":archived["name"],"start":false}));
    f.engine.dispatch(Request::new(Actor::agent(archived["name"].as_str().unwrap()),
        "session.done", json!({"summary":"Archived completion"})), Door::InProcess).into_result().unwrap();
    {
        let conn = f.engine.store.lock();
        conn.execute("DELETE FROM audit WHERE op='session.done' AND session_id=?1", [archived["id"].as_i64().unwrap()]).unwrap();
        // Reconstruct rows the old build failed to delete, plus a renewed claim
        // and unfinished work whose claims must remain reserved.
        for (session, path, updated) in [
            (&done,"old.rs","2000-01-01T00:00:00Z"),
            (&done,"renewed.rs","2999-01-01T00:00:00Z"),
            (&closed,"closed.rs","2000-01-01T00:00:00Z"),
            (&working,"working.rs","2000-01-01T00:00:00Z"),
            (&archived,"archived.rs","2000-01-01T00:00:00Z"),
        ] {
            conn.execute("INSERT INTO claims(project_id,session_id,session,path,created_at,updated_at) VALUES(1,?1,?2,?3,'2000-01-01T00:00:00Z',?4)",
                rusqlite::params![session["id"].as_i64().unwrap(),session["name"].as_str().unwrap(),path,updated]).unwrap();
            conn.execute("INSERT INTO overlaps(project_id,fingerprint,sessions,path,kind,first_seen,last_seen) VALUES(1,?1,?2,?1,'claim','2000-01-01T00:00:00Z','2000-01-01T00:00:00Z')",
                rusqlite::params![path,json!([session["name"]]).to_string()]).unwrap();
        }
    }
    let report = relay_core::recovery::run(&f.engine).unwrap();
    assert!(report.fsck_fixes.iter().any(|fix| fix == "released 3 stale file claim(s)"), "{report:?}");
    {
        let conn = f.engine.store.lock();
        let remaining: Vec<String> = conn.prepare("SELECT path FROM claims ORDER BY path").unwrap()
            .query_map([], |row| row.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(remaining, ["renewed.rs", "working.rs"]);
        let overlaps: Vec<String> = conn.prepare("SELECT path FROM overlaps WHERE active=1 ORDER BY path").unwrap()
            .query_map([], |row| row.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(overlaps, remaining);
    }
    let again = relay_core::recovery::run(&f.engine).unwrap();
    assert!(!again.fsck_fixes.iter().any(|fix| fix.contains("stale file claim")));
}

#[test]
fn provider_discovery_is_explicit_cached_and_reports_version_changes() {
    let f = fixture();
    let claude = fake_discovery_provider(&f.root, "claude", "claude 1.0.0");
    let codex = fake_discovery_provider(&f.root, "codex", "codex-cli 2.0.0");
    ok(&f.engine, "settings.set", json!({"path": "providers.claude.path", "value": claude}));
    ok(&f.engine, "settings.set", json!({"path": "providers.codex.path", "value": codex}));

    let before = ok(&f.engine, "provider.list", json!({}));
    assert!(before["providers"].as_array().unwrap().iter().all(|provider| provider["installed"] == true && provider["version"].is_null()));
    let refreshed = ok(&f.engine, "provider.refresh", json!({}));
    let providers = refreshed["providers"].as_array().unwrap();
    assert_eq!(providers[0]["version"], "claude 1.0.0");
    assert_eq!(providers[0]["signed_in_as"], "agent@example.test");
    assert_eq!(providers[1]["version"], "codex-cli 2.0.0");
    assert_eq!(providers[1]["signed_in_as"], "ChatGPT");
    assert!(providers[0]["spawn_profile"]["fresh"].is_array());

    fake_discovery_provider(&f.root, "claude", "claude 1.1.0");
    let changed = ok(&f.engine, "provider.refresh", json!({}));
    let claude = &changed["providers"][0];
    assert_eq!(claude["version"], "claude 1.1.0");
    assert_eq!(claude["last_seen_version"], "claude 1.0.0");
    let notifications: i64 = f.engine.store.lock().query_row(
        "SELECT COUNT(*) FROM notifications WHERE category='provider' AND title='claude version changed'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(notifications, 1);
    assert_eq!(ok(&f.engine, "app.status", json!({}))["providers"][0]["version"], "claude 1.1.0");
}

#[test]
fn codex_launch_uses_reviewer_instructions_and_types_the_private_start_prompt() {
    let f = fixture();
    let provider = fake_discovery_provider(&f.root, "codex", "codex-cli 1.0.0");
    ok(&f.engine, "settings.set", json!({"path": "providers.codex.path", "value": provider}));
    let created = ok(&f.engine, "session.create", json!({
        "project_id": 1, "provider": "codex", "role": "reviewer"
    }));
    let name = created["name"].as_str().unwrap().to_string();
    let running = ok(&f.engine, "session.spawn", json!({
        "session": name, "prompt": "review the private assignment"
    }));
    wait_until("provider greeting", || ok(&f.engine, "session.scrollback", json!({"session": name}))["text"].as_str().unwrap().contains("hello-from-pty"));
    wait_until("provider start prompt", || ok(&f.engine, "session.scrollback", json!({"session": name}))["text"].as_str().unwrap().contains("echo:Call session.bootstrap first, then begin this assignment: review the private assignment"));
    let worktree = PathBuf::from(running["worktree"].as_str().unwrap());
    let args = std::fs::read_to_string(worktree.join(".relay/provider-args.txt")).unwrap();
    assert!(args.contains("developer_instructions=\"You are this Relay session's reviewer."));
    assert!(args.contains("session.bootstrap"));
    assert!(args.contains("notify=[\""));
    assert!(args.contains("\",\"hook\",\"codex-notify\"]"));
    // F7: Codex gets no MCP config and files no lifecycle report, so the one channel it does
    // have has to carry the brief and the shell path to the bus…
    assert!(args.contains("$RELAY_BIN q"), "codex is never told how to reach the bus");
    assert!(args.contains("Live peers"), "codex never receives the peer table");
    // …and Relay still sees it working, because the PTY is the signal (D108).
    let observed = ok(&f.engine, "session.get", json!({"session": name}));
    assert!(
        observed["last_output_at"].is_string(),
        "an unhooked provider must not look like a session that has never done anything",
    );
    assert!(!args.contains("review the private assignment"));
    assert!(!args.contains("Start by reading"));
    let bootstrap = f.engine.dispatch(
        Request::new(Actor::agent(&name), "session.bootstrap", json!({})),
        Door::InProcess,
    ).into_result().unwrap();
    assert_eq!(bootstrap["assignment"], "review the private assignment");
    ok(&f.engine, "session.close", json!({"session": name}));
}

#[test]
fn skills_are_app_wide_folders_in_every_checkout() {
    let f = fixture();
    let provider = fake_discovery_provider(&f.root, "claude", "claude 1.0.0");
    ok(&f.engine, "settings.set", json!({"path": "providers.claude.path", "value": provider}));
    // A skill the repository checks in itself is the project's, not Relay's: same name, and
    // the checked-in copy still wins in the folder it occupies.
    let owned = f.repo.join(".claude/skills/impeccable");
    std::fs::create_dir_all(&owned).unwrap();
    std::fs::write(owned.join("SKILL.md"), "checked in").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-q", "-m", "vendor a skill"]);

    let body = "---\nname: Impeccable\n---\nDesign well.\n";
    let skill = ok(&f.engine, "skill.create", json!({"name": "Impeccable", "body": body}));
    // Installing once covers the project that already existed (D147).
    assert_eq!(skill["enabled_in"], json!([1]));

    // What `skill.install` stores beside the store: the folder, not just the SKILL.md body.
    let id = skill["id"].as_i64().unwrap();
    let library = f.root.join("store/skills").join(id.to_string());
    std::fs::create_dir_all(library.join("reference")).unwrap();
    std::fs::write(library.join("SKILL.md"), body).unwrap();
    std::fs::write(library.join("reference/polish.md"), "polish pass").unwrap();

    let created = ok(&f.engine, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let name = created["name"].as_str().unwrap().to_string();
    let running = ok(&f.engine, "session.spawn", json!({"session": name}));
    let worktree = PathBuf::from(running["worktree"].as_str().unwrap());
    let relay_owned = worktree.join(".agents/skills/impeccable");
    assert_eq!(
        std::fs::read_to_string(relay_owned.join("reference/polish.md")).unwrap(),
        "polish pass",
        "the reference documents a SKILL.md points at never travelled with it",
    );
    assert_eq!(
        std::fs::read_to_string(relay_owned.join("SKILL.md")).unwrap(),
        body,
        "the worktree never received the skill as a real provider folder",
    );
    assert!(relay_owned.join(relay_core::skills::MARKER).is_file(), "the folder is unmarked");
    assert_eq!(
        std::fs::read_to_string(worktree.join(".claude/skills/impeccable/SKILL.md")).unwrap(),
        "checked in",
        "Relay overwrote a skill the repository checks in",
    );
    let status = Command::new("git").arg("-C").arg(&worktree).args(["status", "--porcelain"]).output().unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(status.trim().is_empty(), "materialized skills dirty the worktree: {status}");

    // Disabling removes the folders Relay owns again on the next launch.
    ok(&f.engine, "skill.enable", json!({"skill_id": id, "project_id": 1, "enabled": false}));
    ok(&f.engine, "session.park", json!({"session": name}));
    ok(&f.engine, "session.wake", json!({"session": name}));
    assert!(!relay_owned.exists(), "the worktree kept a skill nobody enabled");
    assert!(worktree.join(".claude/skills/impeccable/SKILL.md").is_file(), "pruning reached a checked-in skill");
    ok(&f.engine, "session.close", json!({"session": name}));
}

#[test]
fn multi_task_launch_stages_the_queue_and_prompts_each_current_task() {
    let f = fixture();
    let provider = fake_discovery_provider(&f.root, "claude", "claude 1.0.0");
    ok(&f.engine, "settings.set", json!({"path": "providers.claude.path", "value": provider}));
    let first = ok(&f.engine, "task.create", json!({"project_id":1,"title":"First queued task"}));
    let second = ok(&f.engine, "task.create", json!({"project_id":1,"title":"Second current task"}));
    let created = ok(&f.engine, "session.create", json!({
        "project_id":1,"provider":"claude","role":"builder"
    }));
    let name = created["name"].as_str().unwrap().to_string();

    let staged_first = ok(&f.engine, "task.dispatch", json!({
        "task_id":first["id"],"session":name,"start":false
    }));
    let staged_second = ok(&f.engine, "task.dispatch", json!({
        "task_id":second["id"],"session":name,"start":false
    }));
    assert_eq!(staged_first["session"]["state"], "created");
    assert_eq!(staged_second["session"]["state"], "created");
    assert_eq!(staged_second["session"]["task_id"], first["id"]);

    ok(&f.engine, "session.spawn", json!({"session":name}));
    wait_until("current-task start prompt", || {
        ok(&f.engine, "session.scrollback", json!({"session":name}))["text"]
            .as_str().unwrap().contains(&format!("echo:Call session.bootstrap first, then begin current Task #{}", first["id"]))
    });
    let brief = ok(&f.engine, "session.brief", json!({"session":name}));
    assert!(brief["text"].as_str().unwrap().contains("assigned_tasks: 2"));
    assert!(brief["text"].as_str().unwrap().contains(&format!("Task #{} [CURRENT]", first["id"])));
    assert!(brief["text"].as_str().unwrap().contains(&format!("Task #{} [QUEUED]", second["id"])));

    let advanced = f.engine.dispatch(
        Request::new(Actor::agent(&name), "session.done", json!({
            "session":name,"status":"completed","summary":"current complete"
        })),
        Door::InProcess,
    ).into_result().unwrap();
    assert_eq!(advanced["task_id"], second["id"]);
    assert_eq!(ok(&f.engine, "task.get", json!({"task_id":first["id"]}))["column"], "in_review");
    assert_eq!(ok(&f.engine, "task.get", json!({"task_id":second["id"]}))["column"], "active");
    let next_prompt = format!("echo:Your current assignment is Task #{}.", second["id"]);
    assert!(!ok(&f.engine, "session.scrollback", json!({"session":name}))["text"].as_str().unwrap().contains(&next_prompt), "Done must not inject into its own unfinished provider turn");
    // Providers can emit another tool event while finishing the Done turn. It must
    // not clear the marker or make that turn's Stop complete the next assignment.
    ok(&f.engine, "session.report", json!({"session":name,"kind":"tool_use"}));
    ok(&f.engine, "session.report", json!({"session":name,"kind":"stop"}));
    assert_eq!(f.engine.store.lock().query_row(
        "SELECT COUNT(*) FROM notifications WHERE category='agent_done'", [], |row| row.get::<_, i64>(0),
    ).unwrap(), 1, "the trailing Stop belongs to the first task");
    assert_eq!(ok(&f.engine, "session.get", json!({"session":name}))["state"], "running");
    wait_until("next-task prompt", || {
        ok(&f.engine, "session.scrollback", json!({"session":name}))["text"]
            .as_str().unwrap().contains(&format!(
                "echo:Your current assignment is Task #{}.",
                second["id"],
            ))
    });
    let mail_count: i64 = f.engine.store.lock().query_row(
        "SELECT COUNT(*) FROM messages WHERE re_task=?1 AND text LIKE 'Your current assignment is Task #%';", [second["id"].as_i64().unwrap()], |row| row.get(0),
    ).unwrap();
    assert_eq!(mail_count, 1, "deferred delivery must not duplicate assignment mail");
    ok(&f.engine, "session.report", json!({"session":name,"kind":"tool_use"}));
    ok(&f.engine, "session.report", json!({"session":name,"kind":"stop","data":{"message":"Second work finished"}}));
    let (count, body): (i64, String) = f.engine.store.lock().query_row(
        "SELECT COUNT(*),MAX(body) FROM notifications WHERE category='agent_done' AND json_extract(link,'$.payload.task_id')=?1", [second["id"].as_i64().unwrap()], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!((count,body),(1,"Second work finished".into()));
    ok(&f.engine, "session.close", json!({"session":name}));
}

#[test]
fn unassigned_done_stop_marker_is_consumed_once_and_reset_on_session_start() {
    let f = fixture();
    let created = ok(&f.engine, "session.create", json!({"project_id":1,"provider":"claude"}));
    let name = created["name"].as_str().unwrap();
    let report = |kind: &str| ok(&f.engine, "session.report", json!({"session":name,"kind":kind}));
    let complete = || f.engine.dispatch(Request::new(Actor::agent(name), "session.done", json!({"session":name,"summary":"Finished"})), Door::InProcess).into_result().unwrap();
    let unread = || ok(&f.engine,"notify.list",json!({"unread_only":true}))["notifications"].as_array().unwrap().clone();
    let ack = || { for notification in unread() { ok(&f.engine,"notify.ack",json!({"notification_id":notification["id"]})); } };
    report("session_start");
    complete();
    ack();
    report("tool_use");
    report("stop");
    assert!(unread().is_empty(), "acknowledged Done must not be resent by its trailing Stop");
    report("tool_use");
    report("stop");
    assert_eq!(unread().len(),1,"marker must be consumed, allowing a later real turn completion");
    ack();
    complete();
    ack();
    report("session_start");
    report("stop");
    assert_eq!(unread().len(),1,"a fresh provider session must not inherit an old pending Stop");
    ok(&f.engine,"session.close",json!({"session":name}));
}

#[test]
fn phase_six_park_wake_restore_and_immutable_launch_options() {
    let f = fixture();
    let provider = fake_discovery_provider(&f.root, "claude", "claude 1.0.0");
    ok(&f.engine, "settings.set", json!({"path": "providers.claude.path", "value": provider}));
    let created = ok(&f.engine, "session.create", json!({
        "project_id": 1, "provider": "claude", "model": "model-x", "effort": "high"
    }));
    let name = created["name"].as_str().unwrap().to_string();
    let updated = ok(&f.engine, "session.update", json!({"session": name, "branch": "relay/custom", "allow_ui": true}));
    assert_eq!(updated["branch"], "relay/custom");
    assert_eq!(updated["allow_ui"], true);
    let branch = Command::new("git").arg("-C").arg(updated["worktree"].as_str().unwrap())
        .args(["branch", "--show-current"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&branch.stdout).trim(), "relay/custom");

    let running = ok(&f.engine, "session.spawn", json!({"session": name, "prompt": "start here"}));
    let first_pid = running["pid"].as_i64().unwrap();
    wait_until("provider greeting", || ok(&f.engine, "session.scrollback", json!({"session": name}))["text"].as_str().unwrap().contains("hello-from-pty"));
    wait_until("provider start prompt", || ok(&f.engine, "session.scrollback", json!({"session": name}))["text"].as_str().unwrap().contains("echo:Call session.bootstrap first, then begin this assignment: start here"));
    let worktree = PathBuf::from(running["worktree"].as_str().unwrap());
    let brief_path = format!(".relay/sessions/{name}/session-brief.md");
    let role_path = format!(".relay/sessions/{name}/role-instructions.md");
    let brief = std::fs::read_to_string(worktree.join(&brief_path)).unwrap();
    assert!(brief.contains("## Current state"));
    assert!(brief.contains(&format!("session: {name}")));
    assert!(brief.contains("Launch assignment:\nstart here"));
    let role = std::fs::read_to_string(worktree.join(&role_path)).unwrap();
    assert!(role.contains("Relay session's builder"));
    assert!(role.contains("session.bootstrap"));
    assert!(role.contains("query task.list"));
    // F1/F9: the injected prompt carries the brief and names the cross-provider channel, and
    // the skill bodies stay in their own file rather than in every system prompt.
    assert!(role.contains("## Live peers"), "the brief is written but never delivered");
    assert!(role.contains("mailbox.send"), "a builder is never told the mailbox exists");
    assert!(role.contains("$RELAY_BRIEF"));
    assert!(!role.contains("start here"), "the launch assignment stays out of the prompt file");
    assert!(worktree.join(".relay/session-skills.md").is_file(), "skills are split out");
    let provider_args = std::fs::read_to_string(worktree.join(".relay/provider-args.txt")).unwrap();
    assert!(provider_args.contains(&format!("--append-system-prompt-file\n{role_path}")));
    assert!(!provider_args.contains("Start by reading"));
    assert!(!provider_args.contains("start here"));
    assert!(!provider_args.contains("## Current state"));
    let bootstrap = f.engine.dispatch(
        Request::new(Actor::agent(&name), "session.bootstrap", json!({})),
        Door::InProcess,
    ).into_result().unwrap();
    assert_eq!(bootstrap["session"], name);
    assert_eq!(bootstrap["role"], "builder");
    assert_eq!(bootstrap["project_id"], 1);
    assert_eq!(bootstrap["assignment"], "start here");
    assert_eq!(bootstrap["brief_path"], brief_path);
    assert!(bootstrap["task"].is_null());
    assert_eq!(code(call(&f.engine, "session.bootstrap", json!({}))), "bus.actor");
    assert_eq!(code(call(&f.engine, "session.update", json!({"session": name, "model": "other"}))), "session.already_spawned");

    let parked = ok(&f.engine, "session.park", json!({"session": name}));
    assert_eq!(parked["state"], "parked");
    wait_until("parked process gone", || !alive(first_pid));
    let saved = ok(&f.engine, "session.scrollback", json!({"session": name}));
    assert!(saved["text"].as_str().unwrap().contains("hello-from-pty"));

    let woken = ok(&f.engine, "session.wake", json!({"session": name}));
    assert_eq!(woken["state"], "running");
    assert_ne!(woken["pid"].as_i64().unwrap(), first_pid);
    assert!(ok(&f.engine, "session.scrollback", json!({"session": name}))["text"].as_str().unwrap().contains("hello-from-pty"));

    f.engine.shutdown();
    let recoverable = ok(&f.engine, "session.restorable", json!({}));
    assert_eq!(recoverable["sessions"][0]["session"]["name"], name);
    assert_eq!(recoverable["sessions"][0]["reason"], "app_restart");
    let resumed = ok(&f.engine, "session.resume", json!({"session": name}));
    assert_eq!(resumed["state"], "running");
    ok(&f.engine, "session.close", json!({"session": name}));
}

#[test]
fn clear_restorable_fresh_starts_the_same_terminal_without_saved_context() {
    let f = fixture();
    let provider = fake_discovery_provider(&f.root, "claude", "claude 1.0.0");
    ok(&f.engine, "settings.set", json!({"path": "providers.claude.path", "value": provider}));
    let created = ok(&f.engine, "session.create", json!({"project_id": 1, "provider": "claude"}));
    let name = created["name"].as_str().unwrap().to_string();
    let id = created["id"].as_i64().unwrap();
    let worktree = created["worktree"].clone();
    let branch = created["branch"].clone();

    ok(&f.engine, "session.spawn", json!({"session": name, "prompt": "keep this assignment"}));
    ok(&f.engine, "session.input", json!({"session": name, "data": "old-context-marker\n"}));
    wait_until("old context in scrollback", || {
        ok(&f.engine, "session.scrollback", json!({"session": name}))["text"]
            .as_str().unwrap().contains("echo:old-context-marker")
    });
    f.engine.store.lock().execute(
        "UPDATE sessions SET provider_ref='saved-provider-context' WHERE id=?1",
        [id],
    ).unwrap();
    f.engine.shutdown();
    assert_eq!(ok(&f.engine, "session.get", json!({"session": name}))["state"], "restorable");

    let cleared = ok(&f.engine, "session.clear_restorable", json!({"session": name}));
    assert_eq!(cleared["state"], "running");
    assert_eq!(cleared["id"], id);
    assert_eq!(cleared["worktree"], worktree);
    assert_eq!(cleared["branch"], branch);
    assert!(cleared["provider_ref"].is_null());
    let args = std::fs::read_to_string(
        PathBuf::from(cleared["worktree"].as_str().unwrap()).join(".relay/provider-args.txt"),
    ).unwrap();
    assert!(!args.contains("--resume"), "Clear resumed the old provider context: {args}");
    assert!(!args.contains("saved-provider-context"), "Clear passed the old provider handle: {args}");
    let scrollback = ok(&f.engine, "session.scrollback", json!({"session": name}));
    assert!(!scrollback["text"].as_str().unwrap().contains("old-context-marker"));
    assert_eq!(
        code(call(&f.engine, "session.clear_restorable", json!({"session": name}))),
        "session.state",
    );
    ok(&f.engine, "session.close", json!({"session": name}));
}

#[test]
fn allocated_assignments_survive_partial_launch_and_omitted_spawn_prompt() {
    let f = fixture();
    let provider = fake_discovery_provider(&f.root, "claude", "claude 1.0.0");
    ok(
        &f.engine,
        "settings.set",
        json!({"path":"providers.claude.path","value":provider}),
    );
    for (override_prompt, expected) in [
        (None, Some("preserved assignment")),
        (
            Some("replacement assignment"),
            Some("replacement assignment"),
        ),
        (Some(""), None),
    ] {
        let session = ok(
            &f.engine,
            "session.create",
            json!({"project_id":1,"provider":"claude","prompt":"preserved assignment"}),
        );
        let id = session["id"].as_i64().unwrap();
        let stored = || {
            f.engine
                .store
                .lock()
                .query_row(
                    "SELECT launch_prompt FROM sessions WHERE id=?1",
                    [id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap()
        };
        assert_eq!(stored().as_deref(), Some("preserved assignment"));
        // A later allocation can fail without losing the already allocated assignment.
        assert!(
            !call(
                &f.engine,
                "session.create",
                json!({"project_id":1,"provider":"claude","worktree":"invalid-relative-worktree"})
            )
            .ok
        );
        let mut spawn = json!({"session":session["name"]});
        if let Some(prompt) = override_prompt {
            spawn["prompt"] = json!(prompt);
        }
        ok(&f.engine, "session.spawn", spawn);
        assert_eq!(stored().as_deref(), expected);
        if let Some(prompt) = expected {
            wait_until("allocated assignment reaches fake provider", || {
                ok(
                    &f.engine,
                    "session.scrollback",
                    json!({"session":session["name"]}),
                )["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!(
                        "echo:Call session.bootstrap first, then begin this assignment: {prompt}"
                    ))
            });
        }
        ok(
            &f.engine,
            "session.close",
            json!({"session":session["name"]}),
        );
    }
}

#[test]
fn pair_sessions_share_checkout_and_teardown_safely() {
    let f = fixture();
    let provider = fake_discovery_provider(&f.root, "claude", "claude 1.0.0");
    ok(&f.engine, "settings.set", json!({"path": "providers.claude.path", "value": provider}));
    let builder = ok(&f.engine, "session.create", json!({"project_id": 1, "provider": "claude", "role": "builder"}));
    let builder_name = builder["name"].as_str().unwrap().to_string();
    let reviewer = ok(&f.engine, "session.create", json!({
        "project_id": 1, "provider": "claude", "role": "reviewer", "pair_with": builder_name
    }));
    let reviewer_name = reviewer["name"].as_str().unwrap().to_string();
    assert_eq!(builder["worktree"], reviewer["worktree"]);
    assert_eq!(builder["branch"], reviewer["branch"]);
    assert_eq!(ok(&f.engine, "session.get", json!({"session": builder_name}))["pair_with"], reviewer_name);
    let second_builder = ok(&f.engine, "session.create", json!({
        "project_id": 1, "provider": "claude", "role": "builder", "pair_with": reviewer_name
    }));
    let second_builder_name = second_builder["name"].as_str().unwrap().to_string();
    assert_eq!(builder["worktree"], second_builder["worktree"]);
    assert_eq!(second_builder["pair_with"], reviewer_name);
    assert_eq!(ok(&f.engine, "session.get", json!({"session": reviewer_name}))["pair_with"], builder_name);
    assert_eq!(code(call(&f.engine, "session.create", json!({
        "project_id": 1, "provider": "claude", "role": "builder", "pair_with": reviewer_name
    }))), "session.review_group_full");
    let hook_dir = Command::new("git").arg("-C").arg(reviewer["worktree"].as_str().unwrap())
        .args(["config", "--worktree", "--get", "core.hooksPath"]).output().unwrap();
    let hook = PathBuf::from(String::from_utf8(hook_dir.stdout).unwrap().trim()).join("pre-commit");
    assert!(std::fs::read_to_string(hook).unwrap().contains("$RELAY_SESSION"));

    let task = ok(&f.engine, "task.create", json!({"project_id": 1, "title": "Review together"}));
    ok(&f.engine, "task.dispatch", json!({"task_id": task["id"], "session": reviewer_name}));
    assert_eq!(ok(&f.engine, "session.get", json!({"session": builder_name}))["task_id"], task["id"]);
    assert_eq!(ok(&f.engine, "session.get", json!({"session": second_builder_name}))["task_id"], task["id"]);
    let worktree = PathBuf::from(builder["worktree"].as_str().unwrap());
    let reviewer_role = worktree.join(format!(".relay/sessions/{reviewer_name}/role-instructions.md"));
    let reviewer_brief = worktree.join(format!(".relay/sessions/{reviewer_name}/session-brief.md"));
    assert!(std::fs::read_to_string(&reviewer_role).unwrap().contains("Relay session's reviewer"));
    assert!(std::fs::read_to_string(&reviewer_brief).unwrap().contains(&format!("session: {reviewer_name}")));
    ok(&f.engine, "session.spawn", json!({"session": builder_name}));
    let builder_role = worktree.join(format!(".relay/sessions/{builder_name}/role-instructions.md"));
    let builder_brief = worktree.join(format!(".relay/sessions/{builder_name}/session-brief.md"));
    assert!(std::fs::read_to_string(&builder_role).unwrap().contains("Relay session's builder"));
    assert!(std::fs::read_to_string(&builder_brief).unwrap().contains(&format!("session: {builder_name}")));
    assert!(std::fs::read_to_string(&reviewer_role).unwrap().contains("Relay session's reviewer"),
        "the paired builder overwrote the reviewer's role instructions");
    assert!(std::fs::read_to_string(&reviewer_brief).unwrap().contains(&format!("session: {reviewer_name}")),
        "the paired builder overwrote the reviewer's brief");
    ok(&f.engine, "session.spawn", json!({"session": second_builder_name}));
    let bootstrap = f.engine.dispatch(
        Request::new(Actor::agent(&reviewer_name), "session.bootstrap", json!({})),
        Door::InProcess,
    ).into_result().unwrap();
    assert_eq!(bootstrap["role"], "reviewer");
    assert_eq!(bootstrap["pair"]["session"], builder_name);
    assert_eq!(bootstrap["pair"]["branch"], reviewer["branch"]);
    assert_eq!(code(call(&f.engine, "session.close", json!({"session": builder_name}))), "session.pair_live");
    ok(&f.engine, "session.close", json!({"session": builder_name, "remove_worktree": false}));
    let worktree = PathBuf::from(reviewer["worktree"].as_str().unwrap());
    assert!(worktree.exists());
    assert!(ok(&f.engine, "session.get", json!({"session": reviewer_name}))["pair_with"].is_null());
    assert_eq!(code(call(&f.engine, "session.close", json!({"session": reviewer_name}))), "session.pair_live");
    ok(&f.engine, "session.close", json!({"session": reviewer_name, "remove_worktree": false}));
    assert!(ok(&f.engine, "session.get", json!({"session": second_builder_name}))["pair_with"].is_null());
    ok(&f.engine, "session.close", json!({"session": second_builder_name}));
    assert!(!worktree.exists());
}

#[test]
fn review_group_keeps_shared_worktree_until_every_participant_finishes_then_advances_fifo() {
    let f = fixture();
    let first = ok(&f.engine,"task.create",json!({"project_id":1,"title":"First group task"}));
    let second = ok(&f.engine,"task.create",json!({"project_id":1,"title":"Second group task"}));
    let b1 = ok(&f.engine,"session.create",json!({"project_id":1,"provider":"codex","role":"builder"}));
    let reviewer = ok(&f.engine,"session.create",json!({"project_id":1,"provider":"codex","role":"reviewer","pair_with":b1["name"]}));
    let b2 = ok(&f.engine,"session.create",json!({"project_id":1,"provider":"codex","role":"builder","pair_with":reviewer["name"]}));
    let names: Vec<_> = [&b1,&b2,&reviewer].into_iter().map(|s|s["name"].as_str().unwrap().to_string()).collect();
    for task in [&first,&second] { ok(&f.engine,"task.dispatch",json!({"task_id":task["id"],"session":names[0],"start":false})); }
    let current = |name: &str| ok(&f.engine,"session.get",json!({"session":name}))["task_id"].clone();
    let done = |name: &str,status: &str| f.engine.dispatch(Request::new(Actor::agent(name),"session.done",json!({"session":name,"status":status,"summary":"fixture report","blockers":if status == "completed" {vec![]} else {vec!["fixture blocker"]}})),Door::InProcess);
    for name in &names { assert_eq!(current(name),first["id"],"staging must preserve FIFO current"); }
    assert_eq!(code(done(&names[2],"completed")),"session.review_not_ready");
    done(&names[0],"completed").into_result().unwrap();
    // A repeated report before the barrier cannot count as the other builder's work.
    done(&names[0],"completed").into_result().unwrap();
    assert_eq!(ok(&f.engine,"task.get",json!({"task_id":first["id"]}))["column"],"active");
    for name in &names { assert_eq!(current(name),first["id"]); }
    done(&names[1],"partial").into_result().unwrap();
    assert_eq!(ok(&f.engine,"task.get",json!({"task_id":first["id"]}))["state"],"blocked");
    done(&names[1],"completed").into_result().unwrap();
    assert_eq!(ok(&f.engine,"task.get",json!({"task_id":first["id"]}))["column"],"in_review");
    for name in &names { assert_eq!(current(name),first["id"],"review must retain a stable shared worktree"); }
    let mail = ok(&f.engine,"mailbox.list",json!({"project_id":1,"session":names[2]}));
    assert_eq!(mail["messages"].as_array().unwrap().iter().filter(|m|m["re_task"] == first["id"] && m["text"].as_str().unwrap().starts_with("All builders finished")).count(),1);
    done(&names[2],"blocked").into_result().unwrap();
    for name in &names { assert_eq!(current(name),first["id"]); }
    done(&names[2],"completed").into_result().unwrap();
    for name in &names {
        assert_eq!(current(name),second["id"]);
        let mail = ok(&f.engine,"mailbox.list",json!({"project_id":1,"session":name}));
        assert_eq!(mail["messages"].as_array().unwrap().iter().filter(|m|m["re_task"] == second["id"] && m["text"].as_str().unwrap().starts_with("Your current assignment")).count(),1,"each participant must get a durable next assignment");
    }
    // A new taskless Done after this point names the new current task by contract.
    // Only reusing the original req_id is replay-idempotent across an advancement.
}

#[test]
fn slow_checkout_does_not_hold_the_store_lock() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let marker = f.root.join("checkout-started");
    let filter = f.root.join("slow-filter");
    std::fs::write(&filter, format!(
        "#!/bin/sh\ntouch '{}'\nsleep 2\ncat\n", marker.display(),
    )).unwrap();
    std::fs::set_permissions(&filter, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(f.repo.join(".gitattributes"), "README.md filter=slow\n").unwrap();
    git(&f.repo, &["config", "filter.slow.smudge", filter.to_str().unwrap()]);
    git(&f.repo, &["config", "filter.slow.clean", "cat"]);
    git(&f.repo, &["config", "filter.slow.required", "true"]);
    git(&f.repo, &["add", ".gitattributes"]);
    git(&f.repo, &["commit", "-qm", "slow checkout fixture"]);
    let engine = f.engine.clone();
    let creating = std::thread::spawn(move || ok(&engine, "session.create",
        json!({"project_id":1, "provider":"codex", "worktree":"new"})));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(marker.exists(), "Git did not start the slow checkout filter");
    let started = Instant::now();
    ok(&f.engine, "app.status", json!({}));
    let elapsed = started.elapsed();
    let session = creating.join().unwrap();
    assert_eq!(session["state"], "created");
    assert_eq!(std::fs::read_to_string(Path::new(session["worktree"].as_str().unwrap()).join("README.md")).unwrap(), "hi\n");
    assert!(elapsed < Duration::from_millis(500), "app.status blocked behind checkout for {elapsed:?}");
}
