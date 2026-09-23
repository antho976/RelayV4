//! Reply-size and lock-time limits on `file.*` / `git.*` (audit B1, B5, B6, C1-C5): replies the
//! desktop client can always read, and walks and reads that stay bounded on an Unreal-sized tree.

use relay_bus::{Actor, BusError, Event, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn engine() -> Arc<Engine> {
    Engine::new(Instance::Test, Store::open_memory().unwrap())
}
fn call(e: &Engine, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}
fn ok(e: &Engine, op: &str, payload: Value) -> Value {
    call(e, op, payload).into_result().unwrap()
}
fn err(r: &Response) -> &BusError {
    r.error.as_ref().expect("expected an error response")
}
fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
}
/// A committed repository registered as project 1. Returns the tempdir guard and the repo root.
fn project(e: &Engine) -> (tempfile::TempDir, std::path::PathBuf) {
    let ws = tempfile::tempdir().unwrap();
    let repo = ws.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Relay Test"]);
    git(&repo, &["config", "user.email", "relay@example.test"]);
    std::fs::write(repo.join("README.md"), "# Relay\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "Initial"]);
    let repo = std::fs::canonicalize(&repo).unwrap();
    ok(e, "workspace.create", json!({"path": std::fs::canonicalize(ws.path()).unwrap()}));
    ok(e, "project.add", json!({"workspace_id": 1, "path": repo}));
    (ws, repo)
}
fn next_file_changed(events: &mut tokio::sync::broadcast::Receiver<Event>, within: Duration) -> Option<Event> {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        match events.try_recv() {
            Ok(event) if event.ev == "file.changed" => return Some(event),
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    None
}

#[test]
fn diff_file_sends_no_content_for_binary_or_oversized_files() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let mut asset = b"uasset\0".to_vec();
    asset.extend(std::iter::repeat_n(7u8, 4096));
    std::fs::write(repo.join("Hero.uasset"), &asset).unwrap();
    let binary = ok(&e, "git.diff.file", json!({"project_id": 1, "path": "Hero.uasset"}));
    assert_eq!((binary["binary"].clone(), binary["too_large"].clone()), (json!(true), json!(false)));
    assert_eq!((binary["old"].as_str(), binary["new"].as_str()), (Some(""), Some("")));
    assert_eq!(binary["hunks"], json!([]));

    std::fs::write(repo.join("big.txt"), "line of text\n".repeat(100_000)).unwrap();
    let large = ok(&e, "git.diff.file", json!({"project_id": 1, "path": "big.txt"}));
    assert_eq!((large["binary"].clone(), large["too_large"].clone()), (json!(false), json!(true)));
    assert_eq!(large["new"], "");

    std::fs::write(repo.join("README.md"), "# Relay\nmore\n").unwrap();
    let text = ok(&e, "git.diff.file", json!({"project_id": 1, "path": "README.md"}));
    assert_eq!((text["binary"].clone(), text["too_large"].clone()), (json!(false), json!(false)));
    assert_eq!(text["new"], "# Relay\nmore\n");
    assert_eq!(text["hunks"].as_array().unwrap().len(), 1);

    // The list view counts a binary file as 0/0 and flags it, as `git diff --numstat` prints `-`.
    let diff = ok(&e, "git.diff", json!({"project_id": 1}));
    let row = diff["files"].as_array().unwrap().iter().find(|f| f["path"] == "Hero.uasset").unwrap();
    assert_eq!((row["binary"].clone(), row["added"].clone()), (json!(true), json!(0)));
}

#[test]
fn status_reply_is_capped_and_says_so() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let few = ok(&e, "git.status", json!({"project_id": 1}));
    assert_eq!(few["truncated"], false);
    let many = repo.join("Content");
    std::fs::create_dir_all(&many).unwrap();
    for i in 0..5001 {
        std::fs::write(many.join(format!("asset_{i}.txt")), "x").unwrap();
    }
    let status = ok(&e, "git.status", json!({"project_id": 1}));
    assert_eq!(status["truncated"], true);
    assert_eq!(status["files"].as_array().unwrap().len(), 5000);
}

#[test]
fn search_clips_long_lines_and_skips_what_is_not_worth_reading() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let long = format!("{}needle{}", "a".repeat(100_000), "é".repeat(50_000));
    std::fs::write(repo.join("bundle.min.js"), &long).unwrap();
    std::fs::write(repo.join("asset.bin"), b"needle\0binary").unwrap();
    std::fs::write(repo.join(".gitignore"), "ignored/\n").unwrap();
    for dir in ["ignored", "Intermediate/Build", "Saved/Logs", "src"] {
        std::fs::create_dir_all(repo.join(dir)).unwrap();
        std::fs::write(repo.join(dir).join("hit.txt"), "needle\n").unwrap();
    }
    std::fs::write(repo.join("huge.txt"), format!("needle\n{}", "x".repeat(5 * 1024 * 1024))).unwrap();
    assert!(Command::new("mkfifo").arg(repo.join("pipe")).status().unwrap().success());

    let hits = ok(&e, "file.search", json!({"project_id": 1, "query": "needle"}));
    let hits = hits["hits"].as_array().unwrap();
    let mut paths: Vec<&str> = hits.iter().map(|h| h["path"].as_str().unwrap()).collect();
    paths.sort();
    assert_eq!(paths, ["Saved/Logs/hit.txt", "bundle.min.js", "src/hit.txt"]);
    let clipped = hits.iter().find(|h| h["path"] == "bundle.min.js").unwrap();
    let text = clipped["text"].as_str().unwrap();
    assert!(text.len() <= 400 + 2 * '…'.len_utf8(), "{} bytes", text.len());
    assert!(text.contains("needle") && text.starts_with('…') && text.ends_with('…'));
    assert_eq!(clipped["col"], 100_001);
}

#[test]
fn tree_lists_generated_folders_marked_and_lets_them_be_browsed() {
    let e = engine();
    let (_ws, repo) = project(&e);
    std::fs::create_dir_all(repo.join("Saved/Logs")).unwrap();
    std::fs::write(repo.join("Saved/Logs/Game.log"), "crash\n").unwrap();
    std::fs::write(repo.join("build.sh"), "#!/bin/sh\n").unwrap();
    let tree = ok(&e, "file.tree", json!({"project_id": 1, "depth": 3}));
    let entries = tree["entries"].as_array().unwrap();
    let saved = entries.iter().find(|v| v["path"] == "Saved").unwrap();
    assert_eq!(saved["generated"], true);
    assert!(saved["children"].is_null(), "generated folders are not walked unasked");
    assert_eq!(entries.iter().find(|v| v["path"] == "build.sh").unwrap()["generated"], false);
    let logs = ok(&e, "file.tree", json!({"project_id": 1, "path": "Saved/Logs"}));
    assert_eq!(logs["entries"][0]["path"], "Saved/Logs/Game.log");
    assert_eq!(logs["entries"][0]["generated"], true);
}

#[test]
fn watcher_events_carry_the_project_and_the_changed_paths() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let mut events = e.subscribe();
    ok(&e, "file.tree", json!({"project_id": 1}));
    let registered = next_file_changed(&mut events, Duration::from_secs(5)).expect("watcher did not register");
    assert_eq!(registered.project_id, Some(1));
    assert!(registered.payload.get("paths").is_none(), "registration may have missed anything");
    std::thread::sleep(Duration::from_millis(250));
    while events.try_recv().is_ok() {}

    // A folder created after registration is watched too.
    std::fs::create_dir_all(repo.join("Source/Game")).unwrap();
    next_file_changed(&mut events, Duration::from_secs(3)).expect("new folder not seen");
    std::thread::sleep(Duration::from_millis(250));
    while events.try_recv().is_ok() {}
    std::fs::write(repo.join("Source/Game/Hero.cpp"), "int x;\n").unwrap();
    std::fs::write(repo.join("README.md"), "changed\n").unwrap();
    let mut seen = std::collections::BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !(seen.contains("Source/Game/Hero.cpp") && seen.contains("README.md")) {
        let event = next_file_changed(&mut events, deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|| panic!("changed paths not reported: {seen:?}"));
        assert_eq!(event.project_id, Some(1));
        seen.extend(event.payload["paths"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_string()));
    }

    // Badges come from a cached status, and the watcher's event invalidates it.
    let before = ok(&e, "file.tree", json!({"project_id": 1}));
    assert!(before["entries"].as_array().unwrap().iter().all(|v| v["path"] != "NEW.md"));
    std::fs::write(repo.join("NEW.md"), "new\n").unwrap();
    next_file_changed(&mut events, Duration::from_secs(3)).expect("new file not seen");
    let after = ok(&e, "file.tree", json!({"project_id": 1}));
    let new = after["entries"].as_array().unwrap().iter().find(|v| v["path"] == "NEW.md").unwrap();
    assert_eq!(new["badge"], "?");
}

#[test]
fn large_files_move_without_being_read_and_import_copies_before_locking() {
    let e = engine();
    let (_ws, repo) = project(&e);
    let mut level = vec![0u8; 2 * 1024 * 1024];
    level[10] = 1;
    std::fs::write(repo.join("Level.umap"), &level).unwrap();
    std::fs::write(repo.join("Notes.txt"), "a\n".repeat(700_000)).unwrap();
    std::fs::create_dir_all(repo.join("Maps")).unwrap();
    ok(&e, "file.rename", json!({"project_id": 1, "path": "Level.umap", "new_name": "Arena.umap"}));
    ok(&e, "file.move", json!({"project_id": 1, "path": "Arena.umap", "into": "Maps"}));
    ok(&e, "file.move", json!({"project_id": 1, "path": "Notes.txt", "into": "Maps"}));
    ok(&e, "file.delete", json!({"project_id": 1, "path": "Maps/Notes.txt"}));
    assert_eq!(std::fs::read(repo.join("Maps/Arena.umap")).unwrap(), level);

    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(outside.path().join("Pack/Textures")).unwrap();
    std::fs::write(outside.path().join("Pack/Textures/rock.png"), b"\x89PNG\0").unwrap();
    std::fs::write(outside.path().join("readme.txt"), "hello\n").unwrap();
    let sources = [outside.path().join("Pack"), outside.path().join("readme.txt")];
    let imported = ok(&e, "file.import", json!({"project_id": 1, "into": "Maps", "sources": sources}));
    assert_eq!(imported["entries"].as_array().unwrap().len(), 2);
    assert_eq!(std::fs::read(repo.join("Maps/Pack/Textures/rock.png")).unwrap(), b"\x89PNG\0");
    assert_eq!(std::fs::read_to_string(repo.join("Maps/readme.txt")).unwrap(), "hello\n");
    let again = call(&e, "file.import", json!({"project_id": 1, "into": "Maps", "sources": [outside.path().join("readme.txt")]}));
    assert_eq!(err(&again).code, "file.exists");
    // Nothing is left behind in the staging area, on success or on refusal.
    let tmp = repo.join(".relay/tmp");
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::read_dir(&tmp).map(|d| d.count()).unwrap_or(0) > 0 {
        assert!(Instant::now() < deadline, "import staging left behind");
        std::thread::sleep(Duration::from_millis(10));
    }

    // A protected destination is still held, and the held import still lands once confirmed.
    ok(&e, "project.update", json!({"project_id": 1, "protected_paths": ["Locked/**"]}));
    std::fs::create_dir_all(repo.join("Locked")).unwrap();
    let held = call(&e, "file.import", json!({"project_id": 1, "into": "Locked", "sources": [outside.path().join("readme.txt")]}));
    let hold_id = err(&held).confirm.as_ref().expect("held").payload["hold_id"].as_i64().unwrap();
    assert!(!repo.join("Locked/readme.txt").exists());
    ok(&e, "guardrail.confirm", json!({"hold_id": hold_id}));
    assert_eq!(std::fs::read_to_string(repo.join("Locked/readme.txt")).unwrap(), "hello\n");
}
