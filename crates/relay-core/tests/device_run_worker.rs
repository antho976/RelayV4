//! RA-141: a device run follows its app, not one pid forever. The run ends when the app exits,
//! and an app that dies during startup is reported as a crash, with its stack.

mod common;

use common::ok;
use relay_core::engine::Engine;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn executable(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake adb whose `pidof` answers from `pids` (one line per call, then nothing), and whose
/// crash buffer holds `crash`.
fn fake_adb(dir: &Path, pids: &str, crash: &str) -> String {
    std::fs::write(dir.join("pids"), pids).unwrap();
    std::fs::write(dir.join("crash"), crash).unwrap();
    let path = dir.join("adb");
    executable(&path, &format!(r#"#!/bin/sh
dir='{dir}'
if [ "$1" = "devices" ]; then
  printf 'List of devices attached\nrelay-phone device product:relay model:Pixel_9_Pro\n'
  exit 0
fi
if [ "$3" = "shell" ] && [ "$4" = "cmd" ]; then printf 'com.example.app/.MainActivity\n'; exit 0; fi
if [ "$3" = "shell" ] && [ "$4" = "am" ]; then printf 'Status: ok\n'; exit 0; fi
if [ "$3" = "shell" ] && [ "$4" = "pidof" ]; then
  head -n 1 "$dir/pids"
  tail -n +2 "$dir/pids" > "$dir/pids.next"; mv "$dir/pids.next" "$dir/pids"
  exit 0
fi
if [ "$3" = "logcat" ] && [ "$4" = "-c" ]; then exit 0; fi
if [ "$3" = "logcat" ] && [ "$4" = "-d" ]; then cat "$dir/crash"; exit 0; fi
if [ "$3" = "logcat" ]; then
  printf 'I/RelayTest( 1000): app ready\n'
  exec sleep 30
fi
exit 1
"#, dir = dir.display()));
    path.display().to_string()
}

fn fixture(pids: &str, crash: &str) -> (Arc<Engine>, tempfile::TempDir, i64) {
    let e = common::engine();
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let adb = fake_adb(&root, pids, crash);
    ok(&e, "settings.set", json!({"path": "device.adb_path", "value": adb}));
    ok(&e, "settings.set", json!({"path": "device.sdk_path", "value": root}));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/trunk\n").unwrap();
    executable(&repo.join("gradlew"), "#!/bin/sh\nprintf 'installed\\n'\n");
    let metadata = repo.join("app/build/outputs/apk/debug");
    std::fs::create_dir_all(&metadata).unwrap();
    std::fs::write(metadata.join("output-metadata.json"), r#"{"variantName": "debug", "applicationId": "com.example.app"}"#).unwrap();
    let workspace_id = ok(&e, "workspace.create", json!({"path": root}))["id"].as_i64().unwrap();
    let project_id = ok(&e, "project.add", json!({"workspace_id": workspace_id, "path": repo}))["id"].as_i64().unwrap();
    (e, dir, project_id)
}

fn wait_for_end(e: &Engine, project_id: i64, within: Duration) -> Value {
    let deadline = Instant::now() + within;
    loop {
        let run = ok(e, "device.run.list", json!({"project_id": project_id}))["runs"][0].clone();
        if matches!(run["state"].as_str(), Some("finished" | "failed" | "stopped")) {
            return run;
        }
        assert!(Instant::now() < deadline, "the run never ended: {run}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_run_ends_when_its_app_exits() {
    // Found once after launch, then gone at the first liveness check.
    let (e, _dir, project_id) = fixture("1000\n", "");
    let run = ok(&e, "device.run", json!({"project_id": project_id, "device": "relay-phone"}));
    let runtime = relay_core::handlers::device::run_by_id(&e, run["id"].as_i64().unwrap()).unwrap();
    let run = wait_for_end(&e, project_id, Duration::from_secs(15));
    assert_eq!(run["state"], "finished", "{run}");
    let (_, lines, _) = runtime.attach();
    assert!(lines.iter().any(|line| line.line.contains("app ready")));
    assert!(lines.iter().any(|line| line.line.contains("app exited (pid 1000)")), "{lines:?}");
    assert!(ok(&e, "device.leases", json!({}))["leases"].as_array().unwrap().is_empty(), "the lease went with the run");
}

#[test]
fn a_startup_crash_fails_the_run_with_its_stack() {
    let crash = "\
E/AndroidRuntime( 1234): FATAL EXCEPTION: main
E/AndroidRuntime( 1234): Process: com.example.app, PID: 1234
E/AndroidRuntime( 1234): java.lang.IllegalStateException: boom
";
    let (e, _dir, project_id) = fixture("", crash);
    let mut events = e.subscribe();
    let run = ok(&e, "device.run", json!({"project_id": project_id, "device": "relay-phone"}));
    let runtime = relay_core::handlers::device::run_by_id(&e, run["id"].as_i64().unwrap()).unwrap();
    let run = wait_for_end(&e, project_id, Duration::from_secs(20));
    assert_eq!(run["state"], "failed", "{run}");
    let (_, lines, _) = runtime.attach();
    assert!(lines.iter().any(|line| line.line.contains("IllegalStateException: boom")), "{lines:?}");
    assert!(lines.iter().any(|line| line.line.starts_with("device.app_crashed")), "{lines:?}");
    let mut crashed = false;
    while let Ok(event) = events.try_recv() {
        crashed |= event.ev == "run.crash" && event.payload["line"].as_str().is_some_and(|line| line.contains("FATAL EXCEPTION"));
    }
    assert!(crashed, "the crash is announced like one seen in a live stream");
}
