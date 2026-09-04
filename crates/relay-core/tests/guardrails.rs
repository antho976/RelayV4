//! Phase 4 adversarial suite: effective policy, durable decisions, confirmation,
//! authorization and the installed enforcement adapters. Policy calls go through the bus.

use relay_bus::{Actor, ErrorKind, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use uuid::Uuid;

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git").arg("-C").arg(repo).args(args).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

struct Fixture {
    _root: tempfile::TempDir,
    repo: PathBuf,
    engine: Arc<Engine>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        let repo = ws.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "phase4@relay.test"]);
        git(&repo, &["config", "user.name", "Phase Four"]);
        std::fs::write(repo.join("README.md"), "relay\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let store = Store::open(&root.path().join("store/store.db"), false).unwrap();
        let engine = Engine::new(Instance::Test, store);
        ok(&engine, Actor::User, "workspace.create", json!({"path": ws}));
        ok(&engine, Actor::User, "project.add", json!({"workspace_id": 1, "path": repo}));
        Self { _root: root, repo, engine }
    }

    fn session(&self, role: &str, provider: &str) -> Value {
        ok(
            &self.engine,
            Actor::User,
            "session.create",
            json!({"project_id": 1, "provider": provider, "role": role}),
        )
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

fn count(engine: &Engine, table: &str) -> i64 {
    engine
        .store
        .lock()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
        .unwrap()
}

fn error(response: &Response) -> &relay_bus::BusError {
    response.error.as_ref().expect("expected error response")
}

#[test]
fn effective_config_merges_without_erasing_project_overrides() {
    let fixture = Fixture::new();
    let engine = &fixture.engine;
    let defaults = ok(engine, Actor::User, "guardrail.config.get", json!({}));
    assert_eq!(defaults["caps"], json!({"files": 40, "lines": 2000}));

    let project = ok(
        engine,
        Actor::User,
        "guardrail.config.set",
        json!({"project_id": 1, "patch": {"caps": {"files": 3, "lines": 77}}}),
    );
    assert_eq!(project["caps"], json!({"files": 3, "lines": 77}));

    ok(
        engine,
        Actor::User,
        "guardrail.config.set",
        json!({"patch": {"destructive_write": {"min_removed_lines": 12}}}),
    );
    let effective = ok(
        engine,
        Actor::User,
        "guardrail.config.get",
        json!({"project_id": 1}),
    );
    assert_eq!(effective["caps"], json!({"files": 3, "lines": 77}));
    assert_eq!(effective["destructive_write"]["min_removed_lines"], 12);

    let invalid = call(
        engine,
        Actor::User,
        "guardrail.config.set",
        json!({"project_id": 1, "patch": {"caps": {"lines": 0}}}),
    );
    assert_eq!(error(&invalid).code, "guardrail.config");
    let after = ok(
        engine,
        Actor::User,
        "guardrail.config.get",
        json!({"project_id": 1}),
    );
    assert_eq!(after["caps"], json!({"files": 3, "lines": 77}), "invalid patch rolled back");
}

#[test]
fn refusals_holds_confirmation_replay_and_expiry_are_durable() {
    let fixture = Fixture::new();
    let engine = &fixture.engine;
    ok(
        engine,
        Actor::User,
        "guardrail.config.set",
        json!({"project_id": 1, "patch": {"protected_paths": ["secret/**"]}}),
    );
    let session = fixture.session("builder", "codex");
    let name = session["name"].as_str().unwrap();
    let actor = Actor::agent(name);
    let worktree = PathBuf::from(session["worktree"].as_str().unwrap());

    let refused = call(
        engine,
        actor.clone(),
        "guardrail.gate",
        json!({"session": name, "kind": "write", "path": "secret/key", "new_text": "x"}),
    );
    assert_eq!(error(&refused).kind, ErrorKind::Refused);
    assert_eq!(error(&refused).code, "guardrail.protected_path");
    assert_eq!(count(engine, "holds"), 0);
    assert_eq!(count(engine, "notifications"), 1);
    let audit = ok(engine, Actor::User, "audit.list", json!({"op_prefix": "guardrail.gate"}));
    assert!(audit["rows"].as_array().unwrap().iter().any(|row| {
        row["actor"] == format!("agent:{name}")
            && row["kind"] == "refused"
            && row["code"] == "guardrail.protected_path"
    }));

    let user_bypass = call(
        engine,
        Actor::User,
        "guardrail.gate",
        json!({"session": name, "kind": "write", "path": "secret/key", "new_text": "x"}),
    );
    assert_eq!(error(&user_bypass).kind, ErrorKind::Held);
    assert_eq!(error(&user_bypass).code, "guardrail.user_bypass");
    let bypass_id = error(&user_bypass).confirm.as_ref().unwrap().payload["hold_id"].as_i64().unwrap();
    let inspected = ok(engine, Actor::User, "guardrail.hold.get", json!({"hold_id": bypass_id}));
    assert_eq!(inspected["hold"]["state"], "open");
    assert_eq!(inspected["request"]["op"], "guardrail.gate");
    assert_eq!(inspected["request"]["payload"]["path"], "secret/key");
    assert_eq!(inspected["request"]["payload"]["new_text"], "x");
    assert!(inspected["request"].get("token").is_none());
    assert!(!call(engine, actor.clone(), "guardrail.hold.get", json!({"hold_id": bypass_id})).ok,
        "an agent must not inspect another actor's frozen action");
    let bypass = ok(
        engine,
        Actor::User,
        "guardrail.confirm",
        json!({"hold_id": bypass_id}),
    );
    assert_eq!(bypass["hold"]["state"], "confirmed");
    assert_eq!(bypass["outcome"]["ok"], true);

    let old = (0..100).map(|line| format!("line-{line}\n")).collect::<String>();
    let next = (0..10).map(|line| format!("line-{line}\n")).collect::<String>();
    std::fs::write(worktree.join("large.txt"), old).unwrap();

    let before_holds = count(engine, "holds");
    let before_notifications = count(engine, "notifications");
    // A diff is measured against the file on disk, not against itself. Three lines out of a
    // hundred is not a destructive write, and used to be held only because the missing old
    // size was inferred from the diff and therefore always came out at 100%.
    let small = ok(
        engine,
        actor.clone(),
        "guardrail.check",
        json!({"project_id": 1, "kind": "write", "path": "large.txt", "diff": "-a\n-b\n-c\n"}),
    );
    assert_eq!(small["verdict"], "allow", "a three-line diff is not a destructive write");

    let wholesale: String = (0..60).map(|_| "-gone\n").collect();
    let check = ok(
        engine,
        actor.clone(),
        "guardrail.check",
        json!({"project_id": 1, "kind": "write", "path": "large.txt", "diff": wholesale}),
    );
    assert_eq!(check["verdict"], "hold");
    assert_eq!(count(engine, "holds"), before_holds, "dry run created a hold");
    assert_eq!(count(engine, "notifications"), before_notifications, "dry run notified");

    let held = call(
        engine,
        actor.clone(),
        "guardrail.gate",
        json!({"session": name, "kind": "write", "path": "large.txt", "new_text": next}),
    );
    assert_eq!(error(&held).kind, ErrorKind::Held);
    assert_eq!(error(&held).code, "guardrail.destructive_write");
    let hold_id = error(&held).confirm.as_ref().unwrap().payload["hold_id"].as_i64().unwrap();
    assert_eq!(count(engine, "holds"), before_holds + 1);
    assert_eq!(count(engine, "notifications"), before_notifications + 1);

    let confirm_id = Uuid::new_v4();
    let confirm = Request::new(Actor::User, "guardrail.confirm", json!({"hold_id": hold_id}))
        .with_id(confirm_id);
    let first = engine.dispatch(confirm.clone(), Door::InProcess);
    assert!(first.ok, "{:?}", first.error);
    let result = first.result.as_ref().unwrap();
    assert_eq!(result["hold"]["state"], "confirmed");
    assert_eq!(result["outcome"]["ok"], true);
    let replay = engine.dispatch(confirm, Door::InProcess);
    assert_eq!(replay.replayed, Some(true));
    assert_eq!(replay.result, first.result);

    let inbox = call(
        engine,
        actor.clone(),
        "mailbox.list",
        json!({"unread_only": true}),
    );
    assert_eq!(inbox.mail.as_ref().map(|mail| mail.priority), Some(2));
    let messages = inbox.result.as_ref().unwrap()["messages"].as_array().unwrap();
    assert!(messages.iter().all(|message| message["priority"] == true));
    for message in messages {
        ok(
            engine,
            actor.clone(),
            "mailbox.ack",
            json!({"message_id": message["id"]}),
        );
    }
    let after_ack = call(
        engine,
        actor.clone(),
        "session.get",
        json!({"session": name}),
    );
    assert!(after_ack.mail.is_none());

    let audit = ok(engine, Actor::User, "audit.list", json!({"op_prefix": "guardrail.gate"}));
    let confirmed = audit["rows"].as_array().unwrap().iter()
        .find(|row| row["req_id"] == confirm_id.to_string()).unwrap();
    assert_eq!(confirmed["actor"], "user");
    assert_eq!(confirmed["on_behalf_of"], format!("agent:{name}"));
    assert_eq!(confirmed["kind"], "ok");

    let held_again = call(
        engine,
        actor,
        "guardrail.gate",
        json!({"session": name, "kind": "write", "path": "large.txt", "new_text": "tiny\n"}),
    );
    let expiring_id = error(&held_again).confirm.as_ref().unwrap().payload["hold_id"].as_i64().unwrap();
    ok(engine, Actor::User, "session.close", json!({"session": name}));
    let holds = ok(engine, Actor::User, "guardrail.holds.list", json!({"open_only": false}));
    let expired = holds["holds"].as_array().unwrap().iter()
        .find(|hold| hold["id"] == expiring_id).unwrap();
    assert_eq!(expired["state"], "expired");
    assert_eq!(expired["resolved_by"], "system");
}

#[test]
fn roles_scope_shape_exec_and_commit_caps_resist_bypass() {
    let fixture = Fixture::new();
    let engine = &fixture.engine;
    ok(
        engine,
        Actor::User,
        "guardrail.config.set",
        json!({"project_id": 1, "patch": {
            "caps": {"files": 1, "lines": 2},
            "shape_gates": [{"path": "manifest.json", "validator": "json_non_empty_object"}]
        }}),
    );
    let builder = fixture.session("builder", "codex");
    let reviewer = fixture.session("reviewer", "codex");
    let docs = fixture.session("docs", "codex");
    let builder_name = builder["name"].as_str().unwrap();
    let reviewer_name = reviewer["name"].as_str().unwrap();
    let docs_name = docs["name"].as_str().unwrap();

    let denied = call(
        engine,
        Actor::agent(reviewer_name),
        "guardrail.gate",
        json!({"session": reviewer_name, "kind": "exec", "command": "cargo test"}),
    );
    assert_eq!(error(&denied).code, "actor.allowlist");
    let query = call(
        engine,
        Actor::agent(reviewer_name),
        "guardrail.check",
        json!({"project_id": 1, "kind": "exec", "command": "cargo test"}),
    );
    assert!(query.ok, "all roles retain all queries");
    let future_write = call(
        engine,
        Actor::agent(reviewer_name),
        "file.write",
        json!({"project_id": 1, "path": "review.txt", "text": "no"}),
    );
    assert_eq!(
        error(&future_write).code,
        "actor.allowlist",
        "authorization runs before implementation availability"
    );

    let cross_session = call(
        engine,
        Actor::agent(builder_name),
        "guardrail.gate",
        json!({"session": docs_name, "kind": "exec", "command": "cargo test"}),
    );
    assert_eq!(error(&cross_session).code, "actor.scope");
    let docs_allowed = call(
        engine,
        Actor::agent(docs_name),
        "guardrail.gate",
        json!({"session": docs_name, "kind": "exec", "command": "cargo test"}),
    );
    assert!(docs_allowed.ok, "docs baseline includes guardrail.gate");

    let denied_command = call(
        engine,
        Actor::agent(builder_name),
        "guardrail.gate",
        json!({"session": builder_name, "kind": "exec", "command": "git reset --hard HEAD"}),
    );
    assert_eq!(error(&denied_command).code, "guardrail.command");
    assert_eq!(error(&denied_command).kind, ErrorKind::Refused);

    let shape = call(
        engine,
        Actor::agent(builder_name),
        "guardrail.gate",
        json!({"session": builder_name, "kind": "write", "path": "manifest.json", "new_text": "{}"}),
    );
    assert_eq!(error(&shape).code, "guardrail.shape_gate");
    assert_eq!(error(&shape).kind, ErrorKind::Held);
    let shape_id = error(&shape).confirm.as_ref().unwrap().payload["hold_id"].as_i64().unwrap();
    let rejected = ok(
        engine,
        Actor::User,
        "guardrail.reject",
        json!({"hold_id": shape_id, "reason": "manifest cannot be empty"}),
    );
    assert_eq!(rejected["hold"]["state"], "rejected");
    let duplicate_reject = call(
        engine,
        Actor::User,
        "guardrail.reject",
        json!({"hold_id": shape_id}),
    );
    assert_eq!(error(&duplicate_reject).code, "guardrail.hold_resolved");

    let worktree = PathBuf::from(builder["worktree"].as_str().unwrap());
    std::fs::write(worktree.join("one.txt"), "one\ntwo\n").unwrap();
    std::fs::write(worktree.join("two.txt"), "three\nfour\n").unwrap();
    git(&worktree, &["add", "one.txt", "two.txt"]);
    let capped = call(
        engine,
        Actor::agent(builder_name),
        "guardrail.gate",
        json!({"session": builder_name, "kind": "commit"}),
    );
    assert_eq!(error(&capped).code, "guardrail.cap");
    assert_eq!(error(&capped).kind, ErrorKind::Refused);
}

#[test]
fn sessions_install_git_and_claude_hooks_without_clobbering_local_settings() {
    let fixture = Fixture::new();
    let session = fixture.session("builder", "claude");
    let name = session["name"].as_str().unwrap();
    let worktree = PathBuf::from(session["worktree"].as_str().unwrap());
    let hook_dir = Command::new("git")
        .arg("-C")
        .arg(&worktree)
        .args(["config", "--worktree", "--get", "core.hooksPath"])
        .output()
        .unwrap();
    assert!(hook_dir.status.success());
    let hook = PathBuf::from(String::from_utf8(hook_dir.stdout).unwrap().trim()).join("pre-commit");
    let metadata = std::fs::metadata(&hook).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_ne!(metadata.permissions().mode() & 0o111, 0);
    let script = std::fs::read_to_string(&hook).unwrap();
    assert!(script.contains("guardrail.gate"));
    assert!(script.contains(name));
    assert!(script.contains("printf '{\"session\":\"%s\",\"kind\":\"commit\"}' \"$RELAY_SESSION\""));

    let claude_dir = worktree.join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("settings.local.json"),
        r#"{"permissions":{"allow":["Bash(cargo test)"]},"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"true"}]}]}}"#,
    ).unwrap();
    ok(
        &fixture.engine,
        Actor::User,
        "settings.set",
        json!({"path": "providers.claude.path", "value": "/bin/sh"}),
    );
    ok(
        &fixture.engine,
        Actor::User,
        "session.spawn",
        json!({"session": name}),
    );
    relay_core::hooks::install_claude(&worktree, Instance::Test, Path::new("relay")).unwrap();
    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(claude_dir.join("settings.local.json")).unwrap(),
    ).unwrap();
    assert_eq!(settings["permissions"]["allow"][0], "Bash(cargo test)");
    let groups = settings["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().any(|group| group["matcher"] == "Edit|Write|MultiEdit|Bash"));
    for event in ["SessionStart", "PostToolUse", "Stop", "Notification"] {
        let groups = settings["hooks"][event].as_array().unwrap();
        assert_eq!(groups.len(), 1, "Relay installed {event} reporting");
        assert!(groups[0]["hooks"][0]["command"].as_str().unwrap().contains("hook claude-report"));
    }
    let mcp: Value = serde_json::from_str(&std::fs::read_to_string(worktree.join(".relay/relay.mcp.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["relay"]["args"], json!(["--instance","test","mcp"]));

    let exclude = std::fs::read_to_string(fixture.repo.join(".git/info/exclude")).unwrap();
    assert!(exclude.lines().any(|line| line == ".claude/settings.local.json"));

    ok(
        &fixture.engine,
        Actor::User,
        "session.close",
        json!({"session": name, "remove_worktree": false}),
    );
    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(claude_dir.join("settings.local.json")).unwrap(),
    ).unwrap();
    let groups = settings["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "close removed only Relay's Claude hook");
    assert_eq!(groups[0]["matcher"], "Read");
    for event in ["SessionStart", "PostToolUse", "Stop", "Notification"] {
        assert!(settings["hooks"][event].as_array().unwrap().is_empty(), "close removed Relay's {event} hook");
    }
    assert!(!worktree.join(".relay/relay.mcp.json").exists(), "close removed Relay's MCP config");

    let primary = ok(
        &fixture.engine,
        Actor::User,
        "session.create",
        json!({"project_id": 1, "provider": "codex", "worktree": "primary"}),
    );
    let primary_name = primary["name"].as_str().unwrap();
    let configured = Command::new("git")
        .arg("-C")
        .arg(&fixture.repo)
        .args(["config", "--worktree", "--get", "core.hooksPath"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&configured.stdout).contains(".relay/hooks"));
    ok(
        &fixture.engine,
        Actor::User,
        "session.close",
        json!({"session": primary_name}),
    );
    let restored = Command::new("git")
        .arg("-C")
        .arg(&fixture.repo)
        .args(["config", "--worktree", "--get", "core.hooksPath"])
        .output()
        .unwrap();
    assert!(restored.status.success());
    assert!(!String::from_utf8_lossy(&restored.stdout).contains(".relay/hooks"));
}

#[test]
fn codex_hooks_add_guardrails_and_lifecycle_without_clobbering_project_hooks() {
    let root = tempfile::tempdir().unwrap();
    let codex_dir = root.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).unwrap();
    std::fs::write(
        codex_dir.join("hooks.json"),
        r#"{"description":"mine","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"true"}]}]}}"#,
    ).unwrap();

    relay_core::hooks::install_codex(root.path(), Instance::Test, Path::new("/opt/relay")).unwrap();
    let installed: Value = serde_json::from_str(&std::fs::read_to_string(codex_dir.join("hooks.json")).unwrap()).unwrap();
    assert_eq!(installed["description"], "mine");
    let pre = installed["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2);
    assert!(pre.iter().any(|group| group["hooks"][0]["command"].as_str().unwrap().contains("hook codex-pre-tool")));
    for event in ["SessionStart", "PostToolUse", "Stop"] {
        assert!(installed["hooks"][event][0]["hooks"][0]["command"].as_str().unwrap().contains("hook codex-report"));
    }

    relay_core::hooks::uninstall_codex(root.path()).unwrap();
    let removed: Value = serde_json::from_str(&std::fs::read_to_string(codex_dir.join("hooks.json")).unwrap()).unwrap();
    assert_eq!(removed["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(removed["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "true");
    for event in ["SessionStart", "PostToolUse", "Stop"] {
        assert!(removed["hooks"][event].as_array().unwrap().is_empty());
    }
}
