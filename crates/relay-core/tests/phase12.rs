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
    let boot_log = root.path().join("emulator-boot.log");
    executable(&emulator, &format!("if [ \"$1\" = \"-list-avds\" ]; then echo 'Pixel_9_API_35'; else echo \"$@\" > '{}'; fi", boot_log.display()));
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
    // Booted headless, so the mirror is the only window the emulator gets.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while fs::read_to_string(&boot_log).map_or(true, |text| !text.ends_with('\n')) && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(20)); }
    assert_eq!(fs::read_to_string(&boot_log).unwrap().trim(), "@Pixel_9_API_35 -no-window -no-snapshot-load");

    // With no emulator window, avd.stop is how a running AVD shuts down.
    let err = call(&engine, "avd.stop", json!({"name":"Pixel_9_API_35"})).into_result().unwrap_err();
    assert_eq!(err.code, "avd.not_running");
    let kill_log = root.path().join("adb-kill.log");
    executable(&adb, &format!(
        "case \"$*\" in\n  'devices -l') printf 'List of devices attached\\nemulator-5554 device product:sdk model:sdk_gphone64\\n' ;;\n  '-s emulator-5554 emu avd name') printf 'Pixel_9_API_35\\nOK\\n' ;;\n  '-s emulator-5554 emu kill') echo \"$*\" > '{}' ;;\nesac",
        kill_log.display()
    ));
    assert_eq!(ok(&engine, "avd.list", json!({}))["avds"][0]["running_serial"], "emulator-5554");
    ok(&engine, "avd.stop", json!({"name":"Pixel_9_API_35"}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while fs::read_to_string(&kill_log).map_or(true, |text| !text.ends_with('\n')) && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(20)); }
    assert_eq!(fs::read_to_string(&kill_log).unwrap().trim(), "-s emulator-5554 emu kill");
}

#[test]
fn clone_uses_workspace_when_engine_directory_was_removed() {
    // CWD is process-wide. Reproduce the removed launcher worktree in a child.
    if std::env::var_os("RELAY_TEST_DELETED_CWD").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "clone_uses_workspace_when_engine_directory_was_removed", "--nocapture"])
            .env("RELAY_TEST_DELETED_CWD", "1")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "protocol.ext.allow")
            .env("GIT_CONFIG_VALUE_0", "always")
            .output().unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        return;
    }
    let engine = engine();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let workspace = root.path().join("workspace");
    let removed = root.path().join("removed-launch-worktree");
    for path in [&source, &workspace, &removed] { fs::create_dir(path).unwrap(); }
    command(&source, &["init", "-b", "main"]);
    command(&source, &["-c", "user.name=Fixture", "-c", "user.email=fixture@relay.test", "commit", "--allow-empty", "-m", "Initial"]);
    let helper = root.path().join("upload");
    // A local transport reproduces remote helpers needing a readable CWD, without GitHub/network.
    fs::write(&helper, format!("#!/usr/bin/env python3\nimport os\nassert os.getcwd() == {}\nos.execvp('git', ['git', 'upload-pack', {}])\n",
        serde_json::to_string(&workspace).unwrap(), serde_json::to_string(&source).unwrap())).unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    let ws = ok(&engine, "workspace.create", json!({"path":workspace}));
    std::env::set_current_dir(&removed).unwrap();
    fs::remove_dir(&removed).unwrap();
    assert!(std::env::current_dir().is_err());
    let result = call(&engine, "project.clone", json!({"workspace_id":ws["id"],"url":format!("ext::{}",helper.display()),"dest":"cloned"}));
    std::env::set_current_dir(root.path()).unwrap();
    let project = result.into_result().unwrap();
    assert!(workspace.join("cloned/.git").is_dir());
    assert_eq!(project["project"]["workspace_id"], ws["id"]);
}

#[test]
fn a_settings_write_over_the_audit_limit_keeps_no_inverse() {
    let engine = engine();
    let image = format!("data:image/png;base64,{}", "A".repeat(80 * 1024));
    ok(&engine, "settings.set", json!({"path":"appearance.wallpaper","value":image}));
    ok(&engine, "settings.set", json!({"path":"appearance.wallpaper","value":"data:image/png;base64,small"}));
    let rows = ok(&engine, "audit.list", json!({"op_prefix":"settings.set","limit":1}));
    let id = rows["rows"][0]["id"].as_i64().unwrap();
    // The previous image is not copied into the audit row, so this write cannot be undone.
    let undo = call(&engine, "audit.undo", json!({"audit_id":id})).into_result().unwrap_err();
    assert_eq!(undo.code, "audit.not_undoable");
    // A small previous value still has its inverse.
    ok(&engine, "settings.set", json!({"path":"appearance.wallpaper","value":"data:image/png;base64,other"}));
    let rows = ok(&engine, "audit.list", json!({"op_prefix":"settings.set","limit":1}));
    ok(&engine, "audit.undo", json!({"audit_id":rows["rows"][0]["id"]}));
    assert_eq!(ok(&engine, "settings.get", json!({"path":"appearance.wallpaper"}))["value"], "data:image/png;base64,small");
}
