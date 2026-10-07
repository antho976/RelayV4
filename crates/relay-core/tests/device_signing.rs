//! RA-136: a Relay signing profile that broke, or was left half-created, can be switched off
//! and created again, and a switched-off one never refuses a release build.

use relay_bus::{Actor, BusError, Request, Response};
use relay_core::engine::{Door, Engine};
use relay_core::{Instance, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn call(e: &Engine, op: &str, payload: Value) -> Response {
    e.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}

fn ok(e: &Engine, op: &str, payload: Value) -> Value {
    call(e, op, payload).into_result().unwrap_or_else(|error| panic!("{op} failed: {error:?}"))
}

fn refusal(response: Response) -> BusError {
    response.error.expect("expected a refusal")
}

fn create(e: &Engine, project_id: i64) -> PathBuf {
    let profile = ok(e, "device.signing.create", json!({"project_id": project_id, "key_alias": "upload", "password": "test-secret-123"}));
    assert_eq!(profile["configured"], true);
    PathBuf::from(profile["keystore"].as_str().unwrap()).parent().unwrap().to_path_buf()
}

/// Every directory this project's profiles left in the signing root, live or set aside.
fn profile_dirs(directory: &Path) -> Vec<PathBuf> {
    let name = directory.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::read_dir(directory.parent().unwrap()).unwrap().flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&name))
        .map(|entry| entry.path())
        .collect()
}

#[test]
fn a_broken_or_half_created_signing_profile_can_be_switched_off_and_recreated() {
    let e: Arc<Engine> = Engine::new(Instance::Test, Store::open_memory().unwrap());
    let workspace = tempfile::tempdir().unwrap();
    let repo = workspace.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/trunk\n").unwrap();
    let root = std::fs::canonicalize(workspace.path()).unwrap();
    let workspace_id = ok(&e, "workspace.create", json!({"path": root}))["id"].as_i64().unwrap();
    let project_id = ok(&e, "project.add", json!({"workspace_id": workspace_id, "path": root.join("repo")}))["id"].as_i64().unwrap();

    // A profile whose file broke refuses builds while it is on, with a way out.
    let directory = create(&e, project_id);
    std::fs::write(directory.join("profile.json"), "{ not json").unwrap();
    let broken = refusal(call(&e, "device.signing.get", json!({"project_id": project_id})));
    assert_eq!(broken.code, "device.signing_profile_invalid");
    assert!(broken.hint.is_some());
    let build = refusal(call(&e, "device.build", json!({"project_id": project_id})));
    assert_eq!(build.code, "device.signing_profile_invalid");
    assert!(build.hint.as_deref().unwrap().contains("switch Relay signing off"), "{build:?}");
    // Switching it on stays refused; switching it off sets it aside, key and all.
    assert_eq!(refusal(call(&e, "device.signing.set_enabled", json!({"project_id": project_id, "enabled": true}))).code, "device.signing_profile_invalid");
    let off = ok(&e, "device.signing.set_enabled", json!({"project_id": project_id, "enabled": false}));
    assert_eq!(off["configured"], false);
    assert!(!directory.exists());
    let aside = profile_dirs(&directory);
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert!(aside[0].join("release.p12").is_file(), "the key is kept");
    assert_eq!(ok(&e, "device.signing.get", json!({"project_id": project_id}))["configured"], false);

    // A fresh profile can be created in its place.
    assert_eq!(create(&e, project_id), directory);
    assert_eq!(refusal(call(&e, "device.signing.create", json!({"project_id": project_id, "key_alias": "upload", "password": "test-secret-123"}))).code, "device.signing_exists");

    // Switched off, a profile that then lost its keystore no longer stands in a build's way:
    // the build gets past signing to the next check (this fixture has no Gradle wrapper).
    ok(&e, "device.signing.set_enabled", json!({"project_id": project_id, "enabled": false}));
    std::fs::remove_file(directory.join("release.p12")).unwrap();
    assert_eq!(refusal(call(&e, "device.build", json!({"project_id": project_id}))).code, "device.gradle_missing");

    // Half-created (a crash before profile.json was written): not configured, and creatable.
    std::fs::remove_file(directory.join("profile.json")).unwrap();
    assert_eq!(ok(&e, "device.signing.get", json!({"project_id": project_id}))["configured"], false);
    assert_eq!(create(&e, project_id), directory);
    assert_eq!(ok(&e, "device.signing.get", json!({"project_id": project_id}))["enabled"], true);

    for path in profile_dirs(&directory) {
        std::fs::remove_dir_all(path).unwrap();
    }
}
