//! Phase 2: migrations at every prior version, upgrade backups, backup ops, and the v3 importer.

use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::store::{MIGRATIONS, SCHEMA_VERSION};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::sync::Arc;

fn call(e: &Engine, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}

/// Build a DB at schema version `k` the way a build of that era would have, then open it with
/// the current build: it must migrate to the latest version and pass integrity_check.
#[test]
fn every_prior_version_migrates_forward() {
    assert_eq!(MIGRATIONS.len() as i64, SCHEMA_VERSION);
    for k in 1..=MIGRATIONS.len() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        {
            let c = rusqlite::Connection::open(&path).unwrap();
            for m in &MIGRATIONS[..k] {
                c.execute_batch(m).unwrap();
            }
            c.pragma_update(None, "user_version", k as i64).unwrap();
            c.execute("INSERT INTO meta(key, value) VALUES ('created_at', 'then')", []).unwrap();
        }
        let s = Store::open(&path, false).unwrap_or_else(|e| panic!("opening a v{k} store: {e}"));
        assert_eq!(s.version().unwrap(), SCHEMA_VERSION, "v{k} did not migrate to latest");
        let ok: String = s.lock().query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(ok, "ok");
        // every table has its FK indexes: no FK column without an index (SPEC §1)
        let conn = s.lock();
        let mut st = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'").unwrap();
        let tables: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        for t in tables {
            let mut fk = conn.prepare(&format!("PRAGMA foreign_key_list({t})")).unwrap();
            let cols: Vec<String> = fk.query_map([], |r| r.get::<_, String>(3)).unwrap().map(|r| r.unwrap()).collect();
            for col in cols {
                let mut il = conn.prepare(&format!("PRAGMA index_list({t})")).unwrap();
                let idx: Vec<String> = il.query_map([], |r| r.get::<_, String>(1)).unwrap().map(|r| r.unwrap()).collect();
                let covered = idx.iter().any(|i| {
                    let mut ii = conn.prepare(&format!("PRAGMA index_info({i})")).unwrap();
                    let first: Option<String> = ii.query_map([], |r| r.get::<_, String>(2)).unwrap().next().map(|r| r.unwrap());
                    first.as_deref() == Some(col.as_str())
                });
                assert!(covered, "{t}.{col} is a foreign key with no index leading on it (v{k})");
            }
        }
        if k < MIGRATIONS.len() {
            // upgrading an existing file made a backup first
            let backups = s.list_backups().unwrap();
            assert_eq!(backups.len(), 1, "one upgrade backup for v{k}");
            assert_eq!(backups[0].reason, "upgrade");
        }
    }
}

#[test]
fn backups_via_bus_keep_last_five() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.db"), false).unwrap();
    let e = Engine::new(Instance::Test, store);
    for _ in 0..7 {
        let r = call(&e, "app.backup.now", json!({})).into_result().unwrap();
        assert!(r["bytes"].as_u64().unwrap() > 0);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let l = call(&e, "app.backup.list", json!({})).into_result().unwrap();
    let b = l["backups"].as_array().unwrap();
    assert_eq!(b.len(), 5);
    assert!(b[0]["created_at"].as_str().unwrap() >= b[4]["created_at"].as_str().unwrap(), "newest first");
    assert_eq!(b[0]["reason"], "manual");
    // an agent may not back up
    let r = e.dispatch(Request::new(Actor::agent("x"), "app.backup.now", json!({})), Door::InProcess);
    assert_eq!(r.error.unwrap().code, "actor.allowlist");
}

/// A v3 `.relay/` dir the way v3 wrote it (schema copied from a real relay.db).
fn make_v3(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("attachments")).unwrap();
    let c = rusqlite::Connection::open(dir.join("relay.db")).unwrap();
    // v3 never turned foreign_keys on, so dangling module refs are possible in the wild
    c.execute_batch(r#"
        PRAGMA foreign_keys = OFF;
        CREATE TABLE columns (id INTEGER PRIMARY KEY, name TEXT NOT NULL, position INTEGER NOT NULL);
        CREATE TABLE modules (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL);
        CREATE TABLE tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
            acceptance_criteria TEXT NOT NULL DEFAULT '', target_files TEXT NOT NULL DEFAULT '', size_hint TEXT,
            status TEXT NOT NULL DEFAULT 'idle', column_id INTEGER NOT NULL REFERENCES columns(id),
            module_id INTEGER REFERENCES modules(id), priority TEXT NOT NULL DEFAULT 'medium',
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
        CREATE TABLE runs (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL, branch TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'running',
            session_id TEXT, cost_usd REAL, log_path TEXT, note TEXT, started_at INTEGER NOT NULL, finished_at INTEGER);
        CREATE TABLE attachments (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL, path TEXT NOT NULL, original_name TEXT NOT NULL, created_at INTEGER NOT NULL);
        CREATE TABLE comments (id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL);
        INSERT INTO columns VALUES (1,'Backlog',0),(2,'Queue',3),(3,'Ready',2),(4,'Done',4),(5,'In review',1),(6,'Someday',5);
        INSERT INTO modules VALUES (7,'Wear',1786000000),(9,'Academy',1786000100);
        INSERT INTO tasks (id,title,description,acceptance_criteria,target_files,size_hint,status,column_id,module_id,priority,created_at,updated_at)
          VALUES (10,'First','Do the thing','It works','a.kt, b.kt','large','approved',4,7,'medium',1786000000,1786000001),
                 (11,'Second','','','',NULL,'idle',1,NULL,'urgent',1786000002,1786000003),
                 (12,'Third','Body','','','small','running',2,9,'low',1786000004,1786000005),
                 (13,'Fourth','','','','medium','idle',6,42,'high',1786000006,1786000007);
        INSERT INTO comments VALUES (1,10,'looks good',1786000010),(2,10,'shipped',1786000011);
        INSERT INTO attachments VALUES (1,10,'attachments/shot.png','shot.png',1786000012),(2,11,'attachments/missing.png','missing.png',1786000013);
        INSERT INTO runs VALUES (1,10,'relay/t10','done','s1',0.5,NULL,NULL,1786000000,1786000100);
    "#).unwrap();
    std::fs::write(dir.join("attachments/shot.png"), b"\x89PNGfake").unwrap();
    std::fs::write(dir.join("notes.json"), json!([
        {"id": 1, "title": "Academy foundational rewrite", "body": "long text", "tags": [], "pinned": true, "created_at": 1786798899, "updated_at": 1786798899},
        {"id": 2, "title": null, "body": "", "tags": [], "pinned": false, "created_at": 1786838884, "updated_at": 1786838884}
    ]).to_string()).unwrap();
}

fn project_with_v3() -> (Arc<Engine>, tempfile::TempDir, String) {
    let root = tempfile::tempdir().unwrap();
    let ws = root.path().join("ws");
    let repo = ws.join("app");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    make_v3(&repo.join(".relay"));
    let sessions = root.path().join("sessions.json");
    std::fs::write(&sessions, json!([
        {"id": "s1", "name": "Avex", "repo": std::fs::canonicalize(&repo).unwrap(), "max_files": 20, "max_lines": 1500, "agent": "claude"},
        {"id": "s2", "name": "Other", "repo": "/nowhere", "max_files": 5, "max_lines": 10}
    ]).to_string()).unwrap();
    std::env::set_var("RELAY_V3_SESSIONS_JSON", &sessions);
    let store_dir = root.path().join("store");
    let e = Engine::new(Instance::Test, Store::open(&store_dir.join("store.db"), false).unwrap());
    call(&e, "workspace.create", json!({"path": ws})).into_result().unwrap();
    call(&e, "project.add", json!({"workspace_id": 1, "path": repo})).into_result().unwrap();
    let src = repo.join(".relay").display().to_string();
    (e, root, src)
}

fn count(e: &Engine, sql: &str) -> i64 {
    e.store.lock().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn v3_import_dry_run_then_real() {
    let (e, _root, src) = project_with_v3();
    // dry run: full report, nothing written
    let r = call(&e, "app.import.v3", json!({"source": src, "project_id": 1, "dry_run": true})).into_result().unwrap();
    assert_eq!(r["counts"]["tasks"], 4);
    assert_eq!(r["counts"]["modules"], 2);
    assert_eq!(r["counts"]["notes"], 2);
    assert_eq!(r["counts"]["sessions"], 1);
    assert_eq!(count(&e, "SELECT COUNT(*) FROM tasks"), 0);
    let warnings = r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect::<Vec<_>>().join("\n");
    assert!(warnings.contains("Someday"), "{warnings}");
    assert!(warnings.contains("missing module"), "{warnings}");
    assert!(warnings.contains("missing.png"), "{warnings}");
    assert!(warnings.contains("1 v3 runs"), "{warnings}");

    // real
    let r = call(&e, "app.import.v3", json!({"source": src, "project_id": 1})).into_result().unwrap();
    assert_eq!(r["counts"]["tasks"], 4);
    let map = &r["id_map"];
    let t10 = map["tasks"]["10"].as_i64().unwrap();
    let m7 = map["modules"]["7"].as_i64().unwrap();
    assert_eq!(count(&e, "SELECT COUNT(*) FROM tasks"), 4);
    assert_eq!(count(&e, "SELECT COUNT(*) FROM modules"), 2);
    assert_eq!(count(&e, "SELECT COUNT(*) FROM notes"), 2);
    assert_eq!(count(&e, "SELECT COUNT(*) FROM attachments"), 1);
    let conn = e.store.lock();
    let (col, state, prio, size, module_id, body): (String, String, String, Option<String>, Option<i64>, String) = conn
        .query_row("SELECT col, state, priority, size, module_id, body FROM tasks WHERE id = ?1", [t10],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).unwrap();
    assert_eq!((col.as_str(), state.as_str(), prio.as_str(), size.as_deref(), module_id), ("done", "none", "medium", Some("L"), Some(m7)));
    assert!(body.starts_with("Do the thing"), "{body}");
    assert!(body.contains("## Acceptance criteria\n\nIt works"), "{body}");
    assert!(body.contains("## Target files\n\na.kt, b.kt"), "{body}");
    assert!(body.contains("--- comments ---"), "{body}");
    assert!(body.contains("looks good") && body.contains("shipped"), "{body}");
    // column mapping: Queue → ready, Someday → backlog; priority urgent/low/high; state running
    let t12 = map["tasks"]["12"].as_i64().unwrap();
    let (col, state, prio, size): (String, String, String, Option<String>) = conn
        .query_row("SELECT col, state, priority, size FROM tasks WHERE id = ?1", [t12], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
    assert_eq!((col.as_str(), state.as_str(), prio.as_str(), size.as_deref()), ("ready", "running", "low", Some("S")));
    let t13 = map["tasks"]["13"].as_i64().unwrap();
    let (col, module_id, prio): (String, Option<i64>, String) = conn
        .query_row("SELECT col, module_id, priority FROM tasks WHERE id = ?1", [t13], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!((col.as_str(), module_id, prio.as_str()), ("backlog", None, "high"));
    // timestamps converted
    let created: String = conn.query_row("SELECT created_at FROM tasks WHERE id = ?1", [t10], |r| r.get(0)).unwrap();
    assert!(created.starts_with("2026-08-0"), "{created}");
    // attachment copied under the store
    let path: String = conn.query_row("SELECT path FROM attachments WHERE task_id = ?1", [t10], |r| r.get(0)).unwrap();
    assert!(std::path::Path::new(&path).is_file());
    assert!(path.contains("/attachments/"));
    // notes
    let (title, pinned): (Option<String>, i64) = conn.query_row("SELECT title, pinned FROM notes ORDER BY id LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(title.as_deref(), Some("Academy foundational rewrite"));
    assert_eq!(pinned, 1);
    drop(conn);
    // guardrail caps + project name from sessions.json
    let caps = call(&e, "settings.get", json!({"path": "guardrails.projects.1.caps"})).into_result().unwrap();
    assert_eq!(caps["value"]["files"], 20);
    assert_eq!(caps["value"]["lines"], 1500);
    let pr = call(&e, "project.get", json!({"project_id": 1})).into_result().unwrap();
    assert_eq!(pr["name"], "Avex");
    // audited with the id map in result_summary
    let rows = call(&e, "audit.list", json!({"op_prefix": "app.import"})).into_result().unwrap();
    let row = &rows["rows"][0];
    assert_eq!(row["kind"], "ok");
    assert_eq!(row["result_summary"]["id_map"]["tasks"]["10"], t10);
    // one-time
    let r = call(&e, "app.import.v3", json!({"source": src, "project_id": 1}));
    assert_eq!(r.error.unwrap().code, "import.already_done");
    // bad source / bad project
    let r = call(&e, "app.import.v3", json!({"source": "/nope", "project_id": 1}));
    assert_eq!(r.error.unwrap().code, "import.source");
    let r = call(&e, "app.import.v3", json!({"source": src, "project_id": 9}));
    assert_eq!(r.error.unwrap().code, "project.not_found");
}
