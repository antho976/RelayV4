//! Replies a client cannot bound itself are bounded by the engine (RA-210, 212, 214, 217, 227):
//! the desktop's control socket drops any line over 2 MiB, and with it the whole connection.

mod common;

use common::{call, committed_repo, engine, git, ok};
use relay_bus::{Actor, ErrorKind, Request};
use relay_core::engine::{Door, Engine};
use serde_json::{json, Value};
use std::sync::Arc;

/// A workspace holding one committed repository, added as project 1.
fn project() -> (Arc<Engine>, tempfile::TempDir, std::path::PathBuf) {
    let e = engine();
    let ws = tempfile::tempdir().unwrap();
    let repo = ws.path().join("repo");
    committed_repo(&repo, &[("README.md", "# Relay\n")]);
    let repo = std::fs::canonicalize(&repo).unwrap();
    ok(&e, "workspace.create", json!({"path": std::fs::canonicalize(ws.path()).unwrap()}));
    ok(&e, "project.add", json!({"workspace_id": 1, "path": repo}));
    (e, ws, repo)
}

/// RA-210: a directory is listed up to `limit` entries, folders first, and one cut short is
/// named with its full count, at every depth.
#[test]
fn file_tree_caps_entries_per_directory() {
    let (e, _ws, repo) = project();
    let icons = repo.join("icons");
    std::fs::create_dir_all(icons.join("sub")).unwrap();
    for i in 0..30 {
        std::fs::write(icons.join(format!("icon-{i:02}.svg")), "<svg/>").unwrap();
    }
    let tree = ok(&e, "file.tree", json!({"project_id": 1, "path": "icons", "limit": 10, "git_badges": false}));
    let entries = tree["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 10);
    assert_eq!(entries[0]["path"], "icons/sub", "folders come first");
    assert_eq!(entries[1]["path"], "icons/icon-00.svg");
    assert_eq!(tree["truncated"], json!({"icons": 31}));
    // Nested: the cut is recorded for the child directory, by its path.
    let tree = ok(&e, "file.tree", json!({"project_id": 1, "depth": 2, "limit": 5, "git_badges": false}));
    assert_eq!(tree["truncated"]["icons"], 31, "{tree}");
    assert!(tree["truncated"].get("").is_none(), "the root holds two entries: {tree}");
    // Under the cap nothing is said.
    let tree = ok(&e, "file.tree", json!({"project_id": 1, "git_badges": false}));
    assert!(tree.get("truncated").is_none(), "{tree}");
}

/// RA-212: a match in a long line comes back as a bounded window, and `col` still points into
/// the whole line.
#[test]
fn file_search_returns_a_window_of_a_long_line() {
    let (e, _ws, repo) = project();
    let line = format!("{}needle{}", "a".repeat(300_000), "b".repeat(300_000));
    std::fs::write(repo.join("bundle.min.js"), format!("short needle\n{line}\n")).unwrap();
    let hits = ok(&e, "file.search", json!({"project_id": 1, "query": "needle"}))["hits"].clone();
    let hits = hits.as_array().unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert_eq!(hits[0]["text"], "short needle");
    assert!(hits[0].get("text_offset").is_none());
    let long = &hits[1];
    let text = long["text"].as_str().unwrap();
    assert!(text.len() <= 240, "{}", text.len());
    let col = long["col"].as_u64().unwrap() as usize;
    assert_eq!(col, 300_001);
    let offset = long["text_offset"].as_u64().unwrap() as usize;
    assert_eq!(&text[col - 1 - offset..col - 1 - offset + 6], "needle");
}

/// RA-214: trashed files are listed until restored, and a file deleted in a session's worktree
/// is kept by the primary checkout, so removing the worktree does not take it along.
#[test]
fn trash_is_listed_and_kept_outside_the_worktree() {
    let (e, _ws, repo) = project();
    let pooled = repo.join(".relay/worktrees/w1");
    git(&repo, &["worktree", "add", "-b", "w1", pooled.to_str().unwrap()]);
    std::fs::write(pooled.join("draft.txt"), "keep me\n").unwrap();
    std::fs::write(repo.join("main.txt"), "main\n").unwrap();
    let wt = pooled.to_str().unwrap();
    let first = ok(&e, "file.delete", json!({"project_id": 1, "worktree": wt, "path": "draft.txt"}))["trash_id"].clone();
    let second = ok(&e, "file.delete", json!({"project_id": 1, "path": "main.txt"}))["trash_id"].clone();
    let payload = repo.join(".relay/trash").join(first.to_string()).join("payload");
    assert_eq!(std::fs::read_to_string(&payload).unwrap(), "keep me\n");

    let listed = ok(&e, "file.trash.list", json!({"project_id": 1}))["entries"].clone();
    assert_eq!(listed.as_array().unwrap().len(), 2, "{listed}");
    assert_eq!(listed[0]["id"], second, "newest first");
    assert_eq!(listed[1]["original_path"], "draft.txt");
    assert_eq!(listed[1]["worktree"], wt);
    assert_eq!(listed[1]["available"], true);
    assert_eq!(ok(&e, "file.trash.list", json!({"project_id": 1, "limit": 1}))["entries"].as_array().unwrap().len(), 1);
    // Agents are not shown the whole project's trash.
    let agent = e.dispatch(
        Request::new(Actor::parse("agent:calm-otter").unwrap(), "file.trash.list", json!({"project_id": 1})),
        Door::InProcess,
    );
    assert!(agent.error.is_some());

    ok(&e, "file.restore", json!({"project_id": 1, "trash_id": first}));
    assert_eq!(std::fs::read_to_string(pooled.join("draft.txt")).unwrap(), "keep me\n");
    let listed = ok(&e, "file.trash.list", json!({"project_id": 1}))["entries"].clone();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["original_path"], "main.txt");
}

/// RA-214 and RA-227: list previews, deleted notes on request, a body cap that counts the whole
/// note on append, and a `notes.changed` without the body.
#[test]
fn notes_are_bounded_and_deleted_ones_listable() {
    let (e, _ws, _repo) = project();
    let mut events = e.subscribe();
    let big = "x".repeat(10_000);
    let note = ok(&e, "notes.create", json!({"project_id": 1, "title": "Log", "body": big}));
    let changed = std::iter::from_fn(|| events.try_recv().ok())
        .find(|event| event.ev == "notes.changed")
        .expect("notes.changed");
    assert_eq!(changed.payload["id"], note["id"]);
    assert_eq!(changed.payload["body_bytes"], 10_000);
    assert!(changed.payload.get("body").is_none(), "{}", changed.payload);

    let summary = ok(&e, "notes.list", json!({"project_id": 1, "summary": true}));
    let listed = &summary["notes"][0];
    assert_eq!(listed["body"].as_str().unwrap().chars().count(), 240);
    let full = ok(&e, "notes.list", json!({"project_id": 1}));
    assert_eq!(full["notes"][0]["body"].as_str().unwrap().len(), 10_000);

    let gone = ok(&e, "notes.create", json!({"project_id": 1, "body": "short"}));
    ok(&e, "notes.delete", json!({"note_id": gone["id"]}));
    assert_eq!(ok(&e, "notes.list", json!({"project_id": 1}))["notes"].as_array().unwrap().len(), 1);
    let all = ok(&e, "notes.list", json!({"project_id": 1, "include_deleted": true}));
    let deleted: Vec<&Value> = all["notes"].as_array().unwrap().iter().filter(|n| !n["deleted_at"].is_null()).collect();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0]["id"], gone["id"]);

    let limit = 1024 * 1024;
    let refused = call(&e, "notes.create", json!({"project_id": 1, "body": "y".repeat(limit + 1)}));
    assert_eq!(refused.error.unwrap().kind, ErrorKind::Invalid);
    let refused = call(&e, "notes.update", json!({"note_id": note["id"], "body": "y".repeat(limit + 1)}));
    assert_eq!(refused.error.unwrap().kind, ErrorKind::Invalid);
    ok(&e, "notes.update", json!({"note_id": note["id"], "body": "y".repeat(limit - 10)}));
    // Each append is small; the note it would make is not.
    let refused = call(&e, "notes.append", json!({"note_id": note["id"], "text": "z".repeat(20)}));
    assert_eq!(refused.error.unwrap().kind, ErrorKind::Invalid);
    ok(&e, "notes.append", json!({"note_id": note["id"], "text": "z".repeat(5)}));
}

/// RA-217: a held write of a large file is shown cut, and confirming it still writes it whole.
/// `guardrail.holds.list` is a page, newest first.
#[test]
fn a_large_held_action_is_shown_elided_and_replayed_whole() {
    let (e, _ws, repo) = project();
    ok(&e, "project.update", json!({"project_id": 1, "protected_paths": ["protected-*.txt"]}));
    let text = "line\n".repeat(40_000);
    let mut holds = Vec::new();
    for i in 0..3 {
        let held = call(&e, "file.create", json!({"project_id": 1, "path": format!("protected-{i}.txt"), "kind": "file", "text": text}));
        let error = held.error.expect("held");
        assert_eq!(error.kind, ErrorKind::Held);
        holds.push(error.confirm.unwrap().payload["hold_id"].clone());
    }
    let page = ok(&e, "guardrail.holds.list", json!({"project_id": 1, "limit": 2}))["holds"].clone();
    assert_eq!(page.as_array().unwrap().len(), 2);
    assert_eq!(page[0]["id"], holds[2], "newest first");

    let shown = ok(&e, "guardrail.hold.get", json!({"hold_id": holds[0]}));
    assert_eq!(shown["elided"], json!(["/request/payload/text"]));
    let cut = shown["request"]["payload"]["text"].as_str().unwrap();
    assert!(cut.len() < 8 * 1024 && cut.starts_with("line\nline\n"), "{}", cut.len());
    let whole = ok(&e, "guardrail.hold.get", json!({"hold_id": holds[0], "full": true}));
    assert_eq!(whole["request"]["payload"]["text"].as_str().unwrap(), text);
    assert!(whole.get("elided").is_none());

    let confirmed = ok(&e, "guardrail.confirm", json!({"hold_id": holds[0]}));
    assert_eq!(confirmed["outcome"]["ok"], true, "{confirmed}");
    assert_eq!(std::fs::read_to_string(repo.join("protected-0.txt")).unwrap(), text);
}

/// RA-214: a file trashed in a worktree that has since been removed goes back into the primary
/// checkout, and the reply says so; a named checkout is honoured and confined for an agent, and
/// a restore never overwrites an existing path.
#[test]
fn restore_outlives_the_worktree_it_was_deleted_from() {
    let (e, _ws, repo) = project();
    let pooled = repo.join(".relay/worktrees/w2");
    git(&repo, &["worktree", "add", "-b", "w2", pooled.to_str().unwrap()]);
    let wt = pooled.to_str().unwrap();
    let mut trashed = Vec::new();
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(pooled.join(name), format!("{name}\n")).unwrap();
        trashed.push(ok(&e, "file.delete", json!({"project_id": 1, "worktree": wt, "path": name}))["trash_id"].clone());
    }
    let [a, b, c, d] = <[Value; 4]>::try_from(trashed).unwrap();
    let restore = |payload: Value| call(&e, "file.restore", payload);

    // While the worktree is there, that is where it goes back.
    let back = ok(&e, "file.restore", json!({"project_id": 1, "trash_id": a}));
    assert_eq!((back["path"].as_str(), back["worktree"].as_str(), back["fallback"].as_bool()), (Some("a.txt"), Some(wt), Some(false)));
    assert_eq!(std::fs::read_to_string(pooled.join("a.txt")).unwrap(), "a.txt\n");

    git(&repo, &["worktree", "remove", "--force", wt]);
    assert!(!pooled.exists());
    let primary = repo.display().to_string();
    // Never over an existing path.
    std::fs::write(repo.join("b.txt"), "mine\n").unwrap();
    let refused = restore(json!({"project_id": 1, "trash_id": b}));
    assert_eq!(refused.error.as_ref().map(|e| e.code.as_str()), Some("file.restore_conflict"), "{:?}", refused.error);
    assert_eq!(std::fs::read_to_string(repo.join("b.txt")).unwrap(), "mine\n");
    std::fs::remove_file(repo.join("b.txt")).unwrap();
    // Its worktree gone, it goes to the primary checkout.
    let back = ok(&e, "file.restore", json!({"project_id": 1, "trash_id": b}));
    assert_eq!((back["worktree"].as_str(), back["fallback"].as_bool()), (Some(primary.as_str()), Some(true)));
    assert_eq!(std::fs::read_to_string(repo.join("b.txt")).unwrap(), "b.txt\n");

    // A named checkout is where it goes.
    let other = repo.join(".relay/worktrees/w3");
    git(&repo, &["worktree", "add", "-b", "w3", other.to_str().unwrap()]);
    let back = ok(&e, "file.restore", json!({"project_id": 1, "trash_id": c, "worktree": other}));
    assert_eq!((back["worktree"].as_str(), back["fallback"].as_bool()), (other.to_str(), Some(false)));
    assert_eq!(std::fs::read_to_string(other.join("c.txt")).unwrap(), "c.txt\n");

    // An agent reaches only its own checkout, the fallback included.
    let builder = ok(&e, "session.create", json!({"project_id": 1, "provider": "claude", "role": "builder", "bus_writes": true}));
    let agent = |payload: Value| e.dispatch(
        Request::new(Actor::agent(builder["name"].as_str().unwrap()), "file.restore", payload), Door::InProcess);
    for payload in [json!({"project_id": 1, "trash_id": d}), json!({"project_id": 1, "trash_id": d, "worktree": "@project"})] {
        let refused = agent(payload);
        assert_eq!(refused.error.as_ref().map(|e| (e.kind, e.code.as_str())), Some((ErrorKind::Refused, "actor.scope")), "{:?}", refused.error);
    }
    let own = std::fs::canonicalize(builder["worktree"].as_str().unwrap()).unwrap();
    let back = agent(json!({"project_id": 1, "trash_id": d, "worktree": own})).into_result().unwrap();
    assert_eq!(back["fallback"], false);
    assert_eq!(std::fs::read_to_string(own.join("d.txt")).unwrap(), "d.txt\n");
}
