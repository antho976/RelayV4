//! Phase 12: current-directory onboarding, repository discovery/clone, and AVD management.

use relay_bus::{Actor, Request, Response};
use relay_core::{Door, Engine, Instance, Store};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn engine() -> Arc<Engine> { Engine::new(Instance::Test, Store::open_memory().unwrap()) }
fn call(engine: &Engine, op: &str, payload: Value) -> Response {
    engine.dispatch(Request::new(Actor::User, op, payload), Door::InProcess)
}
fn ok(engine: &Engine, op: &str, payload: Value) -> Value {
    call(engine, op, payload).into_result().unwrap_or_else(|error| panic!("{op}: {} {}", error.code, error.message))
}
fn command(dir: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(output.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr));
}
fn executable(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn workspace_defaults_to_cwd_and_local_clone_registers_project() {
    let engine = engine();
    let discovered = ok(&engine, "workspace.discover", json!({}));
    let cwd = fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    let expected = cwd.ancestors().find(|path| path.join(".git").exists())
        .and_then(Path::parent).unwrap_or(&cwd);
    assert_eq!(discovered["path"], expected.display().to_string());

    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    command(source.path(), &["init", "-b", "main"]);
    command(source.path(), &["config", "user.name", "Relay Test"]);
    command(source.path(), &["config", "user.email", "relay@example.test"]);
    fs::write(source.path().join("README.md"), "# source\n").unwrap();
    command(source.path(), &["add", "README.md"]);
    command(source.path(), &["commit", "-m", "Initial"]);

    let workspace = ok(&engine, "workspace.create", json!({"path":root.path()}));
    let cloned = ok(&engine, "project.clone", json!({"workspace_id":workspace["id"],"url":source.path(),"dest":"cloned"}));
    assert_eq!(cloned["project"]["name"], "cloned");
    assert!(root.path().join("cloned/.git").exists());
    assert_eq!(ok(&engine, "project.list", json!({}))["projects"].as_array().unwrap().len(), 1);
}

#[test]
fn avd_catalog_create_list_and_boot_use_configured_sdk_tools() {
    let engine = engine();
    let root = tempfile::tempdir().unwrap();
    let sdk = root.path().join("sdk");
    fs::create_dir_all(sdk.join("system-images/android-35/google_apis/x86_64")).unwrap();
    let adb = root.path().join("adb");
    let emulator = root.path().join("emulator");
    let avdmanager = root.path().join("avdmanager");
    executable(&adb, "if [ \"$1\" = \"devices\" ]; then echo 'List of devices attached'; fi");
    executable(&emulator, "if [ \"$1\" = \"-list-avds\" ]; then echo 'Pixel_9_API_35'; fi");
    executable(&avdmanager, "if [ \"$1\" = \"list\" ]; then echo 'pixel_9'; fi\nexit 0");
    for (path, value) in [
        ("device.sdk_path", sdk.display().to_string()),
        ("device.adb_path", adb.display().to_string()),
        ("device.emulator_path", emulator.display().to_string()),
        ("device.avdmanager_path", avdmanager.display().to_string()),
    ] { ok(&engine, "settings.set", json!({"path":path,"value":value})); }

    let catalog = ok(&engine, "avd.catalog", json!({}));
    assert_eq!(catalog["system_images"], json!(["system-images;android-35;google_apis;x86_64"]));
    assert_eq!(catalog["devices"], json!(["pixel_9"]));
    assert_eq!(ok(&engine, "avd.list", json!({}))["avds"][0]["name"], "Pixel_9_API_35");
    let created = ok(&engine, "avd.create", json!({"name":"New_API_35","package":"system-images;android-35;google_apis;x86_64","device":"pixel_9"}));
    assert_eq!(created["name"], "New_API_35");
    ok(&engine, "avd.boot", json!({"name":"Pixel_9_API_35","cold":true}));
}
