//! RA-102: a held write's text lives in a content-addressed file beside the store, not in the
//! hold's envelope; it is read back whole to confirm and to show in full, and removed once no
//! hold names it. A hold's payload hash is taken when it is frozen.

mod common;

use relay_bus::{Actor, ErrorKind, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::store::MIGRATIONS;
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn call(e: &Engine, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}
fn ok(e: &Engine, op: &str, payload: Value) -> Value {
    call(e, op, payload).into_result().unwrap_or_else(|error| panic!("{op}: {} {}", error.code, error.message))
}
fn git(repo: &Path, args: &[&str]) {
    let st = Command::new("git").arg("-C").arg(repo).args(args).status().unwrap();
    assert!(st.success(), "git {args:?}");
}
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// An on-disk store and a project whose `protected-*` files hold a person's own writes.
fn project() -> (Arc<Engine>, tempfile::TempDir, PathBuf, PathBuf) {
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
    let store_dir = root.join("store");
    let e = Engine::new(Instance::Test, Store::open(&store_dir.join("store.db"), false).unwrap());
    ok(&e, "workspace.create", json!({"path": ws}));
    ok(&e, "project.add", json!({"workspace_id": 1, "path": repo}));
    ok(&e, "project.update", json!({"project_id": 1, "protected_paths": ["protected-*"]}));
    (e, tmp, repo, store_dir)
}

fn held_create(e: &Engine, path: &str, text: &str) -> i64 {
    let error = call(e, "file.create", json!({"project_id": 1, "path": path, "kind": "file", "text": text}))
        .error.expect("held");
    assert_eq!(error.kind, ErrorKind::Held);
    error.confirm.unwrap().payload["hold_id"].as_i64().unwrap()
}

fn prune_all(e: &Engine) -> (usize, Vec<PathBuf>) {
    let mut conn = e.store.lock();
    let tx = conn.transaction().unwrap();
    let pruned = relay_core::guardrail::prune_holds(&tx, "9999-01-01T00:00:00Z").unwrap();
    tx.commit().unwrap();
    pruned
}

/// Multi-byte text, CRLF line ends, no final newline, a cut point inside a character: what a
/// replay must give back byte for byte.
fn awkward_text() -> String {
    let mut text = "é".repeat(2049);
    text.push_str(&"line ✓ with\r\nCRLF\ttabs and \"quotes\" \\ \u{1F600}\n".repeat(3000));
    text.push_str("no newline at the end");
    text
}

#[test]
fn a_held_write_keeps_its_text_beside_the_store_and_replays_it_byte_for_byte() {
    let (e, _tmp, repo, store_dir) = project();
    let text = awkward_text();
    let hold = held_create(&e, "protected-a.txt", &text);
    let blob = store_dir.join("hold-blobs").join(sha256(text.as_bytes()));
    assert_eq!(std::fs::read(&blob).unwrap(), text.as_bytes(), "the text is kept whole, under its hash");
    let (envelope, hash): (i64, Option<String>) = e.store.lock()
        .query_row("SELECT length(envelope), payload_hash FROM holds WHERE id=?1", [hold], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(envelope < 16 * 1024, "the envelope keeps only the cut copy ({envelope} bytes)");

    // Shown cut, as before, without reading the file; whole when asked for.
    let shown = ok(&e, "guardrail.hold.get", json!({"hold_id": hold}));
    assert_eq!(shown["elided"], json!(["/request/payload/text"]));
    let cut = shown["request"]["payload"]["text"].as_str().unwrap();
    assert!(cut.len() < 8 * 1024 && cut.starts_with("éé") && cut.contains("more bytes elided"), "{}", cut.len());
    let whole = ok(&e, "guardrail.hold.get", json!({"hold_id": hold, "full": true}));
    assert_eq!(whole["request"]["payload"]["text"].as_str().unwrap(), text);
    assert!(whole.get("elided").is_none());
    // The hash is the payload's, as it always was, and taken once at insert.
    let payload = serde_json::to_vec(&whole["request"]["payload"]).unwrap();
    assert_eq!(hash.as_deref(), Some(sha256(&payload).as_str()));
    assert_eq!(whole["hold"]["payload_hash"], json!(hash));
    let listed = ok(&e, "guardrail.holds.list", json!({"project_id": 1}));
    assert_eq!(listed["holds"][0]["payload_hash"], json!(hash));

    let confirmed = ok(&e, "guardrail.confirm", json!({"hold_id": hold}));
    assert_eq!(confirmed["outcome"]["ok"], true, "{confirmed}");
    assert_eq!(std::fs::read(repo.join("protected-a.txt")).unwrap(), text.as_bytes());
}

#[test]
fn a_held_text_file_goes_once_no_hold_names_it() {
    let (e, _tmp, _repo, store_dir) = project();
    let text = "same text\n".repeat(10_000);
    let first = held_create(&e, "protected-a.txt", &text);
    let second = held_create(&e, "protected-b.txt", &text);
    let blob = store_dir.join("hold-blobs").join(sha256(text.as_bytes()));
    assert_eq!(std::fs::read_dir(store_dir.join("hold-blobs")).unwrap().count(), 1, "one file for one text");

    ok(&e, "guardrail.reject", json!({"hold_id": first}));
    let (pruned, files) = prune_all(&e);
    assert_eq!((pruned, files.len()), (1, 0), "the open hold still names the text");
    ok(&e, "guardrail.reject", json!({"hold_id": second}));
    let (pruned, files) = prune_all(&e);
    assert_eq!((pruned, &files), (1, &vec![blob.clone()]));
    let rows: i64 = e.store.lock().query_row("SELECT COUNT(*) FROM hold_blobs", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0);
    relay_core::purge::remove_paths(&files);
    assert!(!blob.exists());
}

#[test]
fn a_missing_or_altered_text_is_never_replayed() {
    let (e, _tmp, repo, store_dir) = project();
    let text = "x".repeat(100_000);
    let hold = held_create(&e, "protected-a.txt", &text);
    let blob = store_dir.join("hold-blobs").join(sha256(text.as_bytes()));
    std::fs::write(&blob, "y".repeat(100_000)).unwrap();
    let refused = call(&e, "guardrail.confirm", json!({"hold_id": hold})).error.expect("an altered text is refused");
    assert_eq!(refused.code, "guardrail.hold_text_missing");
    assert!(!repo.join("protected-a.txt").exists());
    std::fs::remove_file(&blob).unwrap();
    assert_eq!(call(&e, "guardrail.hold.get", json!({"hold_id": hold, "full": true})).error.unwrap().code, "guardrail.hold_text_missing");
    // The cut copy is still there to look at, and the person can turn the hold down.
    ok(&e, "guardrail.hold.get", json!({"hold_id": hold}));
    ok(&e, "guardrail.reject", json!({"hold_id": hold}));
}

#[test]
fn a_small_held_text_stays_in_the_envelope() {
    let (e, _tmp, repo, store_dir) = project();
    let hold = held_create(&e, "protected-a.txt", "short\n");
    assert!(!store_dir.join("hold-blobs").exists());
    ok(&e, "guardrail.confirm", json!({"hold_id": hold}));
    assert_eq!(std::fs::read_to_string(repo.join("protected-a.txt")).unwrap(), "short\n");
}

#[test]
fn a_hold_from_before_v23_is_hashed_on_its_first_read() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("store.db");
    let payload = json!({"project_id": 1, "path": "a.txt", "text": "old"});
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        for m in &MIGRATIONS[..22] {
            c.execute_batch(m).unwrap();
        }
        c.pragma_update(None, "user_version", 22).unwrap();
        c.execute("INSERT INTO meta(key, value) VALUES ('created_at', 'then')", []).unwrap();
        let envelope = serde_json::to_string(&Request::new(Actor::User, "file.write", payload.clone())).unwrap();
        c.execute(
            "INSERT INTO holds(actor, op, envelope, policy, details, state, created_at) VALUES ('user', 'file.write', ?1, 'protected_path', '{}', 'open', 'then')",
            [envelope],
        ).unwrap();
    }
    let e = Engine::new(Instance::Test, Store::open(&path, false).unwrap());
    let stored = || -> Option<String> { e.store.lock().query_row("SELECT payload_hash FROM holds", [], |r| r.get(0)).unwrap() };
    assert_eq!(stored(), None);
    let listed = ok(&e, "guardrail.holds.list", json!({}));
    let expected = sha256(&serde_json::to_vec(&payload).unwrap());
    assert_eq!(listed["holds"][0]["payload_hash"], json!(expected));
    assert_eq!(stored(), Some(expected));
}
