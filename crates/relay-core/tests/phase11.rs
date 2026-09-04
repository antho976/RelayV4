//! Phase 11: durable project skills, plugin stub, and first-run workspace creation.

use relay_bus::{Actor, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::sync::Arc;

fn call(engine: &Engine, op: &str, payload: Value) -> Response {
    engine.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}

fn ok(engine: &Engine, op: &str, payload: Value) -> Value {
    call(engine, op, payload).into_result().unwrap_or_else(|error| panic!("{op}: {} {}", error.code, error.message))
}

fn fixture() -> (tempfile::TempDir, Arc<Engine>) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("store/store.db"), false).unwrap();
    (root, Engine::new(Instance::Test, store))
}

#[test]
fn skills_crud_filter_enable_delete_and_undo() {
    let (root, engine) = fixture();
    let first = ok(&engine, "app.first_run.state", json!({}));
    assert_eq!(first["needed"], true);
    assert_eq!(first["steps"]["workspace"], "todo");
    let workspace = root.path().join("created-by-relay");
    assert!(!workspace.exists());
    ok(&engine, "workspace.create", json!({"path":workspace}));
    assert!(workspace.is_dir(), "first-run may create the workspace directory");
    let repo = workspace.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    ok(&engine, "project.add", json!({"workspace_id":1,"path":repo}));
    let ready = ok(&engine, "app.first_run.state", json!({}));
    assert_eq!(ready["needed"], false);
    assert_eq!(ready["steps"]["import"], "skipped");

    let skill = ok(&engine, "skill.create", json!({"name":"  Verify  ","body":"Run cargo test."}));
    let id = skill["id"].as_i64().unwrap();
    assert_eq!(skill["name"], "Verify");
    // Skills belong to the app, not to one project (D147): a new one starts enabled everywhere.
    assert_eq!(skill["enabled_in"], json!([1]));

    let disabled = ok(&engine, "skill.enable", json!({"skill_id":id,"project_id":1,"enabled":false}));
    assert!(disabled["enabled_in"].as_array().unwrap().is_empty(), "a project can still opt out");
    assert!(ok(&engine, "skill.list", json!({"project_id":1,"enabled":true}))["skills"].as_array().unwrap().is_empty());
    let enabled = ok(&engine, "skill.enable", json!({"skill_id":id,"project_id":1,"enabled":true}));
    assert_eq!(enabled["enabled_in"], json!([1]));
    let listed = ok(&engine, "skill.list", json!({"project_id":1,"enabled":true}));
    assert_eq!(listed["skills"].as_array().unwrap().len(), 1);
    assert!(ok(&engine, "skill.list", json!({"project_id":1,"enabled":false}))["skills"].as_array().unwrap().is_empty());

    let updated = ok(&engine, "skill.update", json!({"skill_id":id,"body":"Run cargo test --all-targets."}));
    assert!(updated["body"].as_str().unwrap().contains("all-targets"));
    ok(&engine, "skill.delete", json!({"skill_id":id}));
    assert!(ok(&engine, "skill.list", json!({}))["skills"].as_array().unwrap().is_empty());

    let audit = ok(&engine, "audit.list", json!({"op_prefix":"skill.delete","limit":1}));
    let audit_id = audit["rows"][0]["id"].as_i64().unwrap();
    ok(&engine, "audit.undo", json!({"audit_id":audit_id}));
    let restored_list = ok(&engine, "skill.list", json!({}));
    let restored = &restored_list["skills"][0];
    assert_eq!(restored["id"], id);
    assert_eq!(restored["enabled_in"], json!([1]), "delete undo preserves project enables");

    assert!(ok(&engine, "plugin.list", json!({}))["plugins"].as_array().unwrap().is_empty());
}

#[test]
fn skills_validate_names_and_project_references() {
    let (_root, engine) = fixture();
    let empty = call(&engine, "skill.create", json!({"name":"  ","body":"x"}));
    assert_eq!(empty.error.unwrap().code, "skill.name");
    let skill = ok(&engine, "skill.create", json!({"name":"One","body":"x"}));
    let duplicate = call(&engine, "skill.create", json!({"name":"one","body":"y"}));
    assert_eq!(duplicate.error.unwrap().code, "skill.name_exists");
    let missing = call(&engine, "skill.enable", json!({"skill_id":skill["id"],"project_id":99,"enabled":true}));
    assert_eq!(missing.error.unwrap().code, "project.not_found");
}
