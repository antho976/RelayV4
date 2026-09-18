//! Opt-in self-updates for known user-owned installations, outside the store lock.
use crate::engine::{Ctx, Engine, IntoBus};
use relay_bus::ops::provider::{Update, UpdateOut};
use relay_bus::types::Provider;
use relay_bus::BusError;
use rusqlite::params;
use serde_json::json;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn installation(provider: Provider, path: &Path, home: &Path) -> Option<&'static str> {
    match provider {
        Provider::Claude if path.starts_with(home.join(".local/share/claude/versions")) => {
            Some("native")
        }
        Provider::Codex if path.starts_with(home.join(".codex/packages/standalone/releases")) => {
            Some("standalone")
        }
        _ => None,
    }
}

pub(crate) fn register(engine: &mut Engine) {
    engine.register::<Update>(|ctx: &mut Ctx, payload| {
        let provider = payload.provider;
        let name = crate::sessions::provider_str(provider);
        if payload.automatic == Some(true)
            && crate::handlers::settings::get(ctx.tx(), Some(&format!("providers.{name}.auto_update")))? != true
        {
            return Ok(UpdateOut { started: false, method: "disabled".into(), message: "Startup updates are disabled for this provider.".into() });
        }
        let path = crate::providers::executable(ctx.tx(), provider)?;
        let path = std::fs::canonicalize(&path).map_err(|e| BusError::unavailable("provider.update_path", e.to_string()))?;
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let method = installation(provider, &path, &home);
        if home.as_os_str().is_empty() || method.is_none() || path.metadata().map(|m| m.uid()).ok() != Some(unsafe { libc::geteuid() }) {
            return Ok(UpdateOut { started: false, method: "managed_or_unknown".into(), message: "Update this installation with its package manager or installer. Relay only self-updates known user-owned native installations.".into() });
        }
        let active: bool = ctx.tx().query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE provider=?1 AND (pid IS NOT NULL OR state='spawning') AND state NOT IN ('closed','exited'))",
            [name], |row| row.get(0),
        ).bus()?;
        if active {
            return Ok(UpdateOut { started: false, method: method.unwrap().into(), message: "Update skipped while this provider has a live session. Existing sessions are unchanged.".into() });
        }
        if !ctx.engine().provider_updates.lock().unwrap().insert(name.into()) {
            return Ok(UpdateOut { started: false, method: method.unwrap().into(), message: "An update is already running for this provider.".into() });
        }
        ctx.emit("provider.update.changed", json!({"provider":name,"state":"updating","message":"Checking for an update…"}));
        ctx.after_commit(move |engine| {
            std::thread::spawn(move || {
                let result = run(&path, Duration::from_secs(120));
                let (state, message) = match &result {
                    Ok(message) => ("complete", message.clone()),
                    Err(message) => ("failed", message.clone()),
                };
                let _ = engine.system_write("provider.update.finished", None, None, None, json!({"provider":name,"state":state}), |tx, now| {
                    if result.is_ok() { let _ = crate::providers::refresh(tx, now); }
                    tx.execute("INSERT INTO notifications(project_id,category,title,body,link,read,created_at) VALUES (NULL,'provider',?1,?2,NULL,0,?3)",
                        params![format!("{name} update {state}"),message,now]).bus()?;
                    Ok(((), vec![
                        ("provider.update.changed".into(), json!({"provider":name,"state":state,"message":message})),
                        ("notify.new".into(), json!({"category":"provider"})),
                    ]))
                });
                engine.provider_updates.lock().unwrap().remove(name);
            });
        });
        Ok(UpdateOut { started: true, method: method.unwrap().into(), message: format!("Updating {name} in the background. New sessions use the updated version.") })
    });
}

fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut saved = Vec::new();
        let mut bytes = [0; 4096];
        while let Ok(count) = pipe.read(&mut bytes) {
            if count == 0 {
                break;
            }
            let keep = count.min(65536usize.saturating_sub(saved.len()));
            saved.extend_from_slice(&bytes[..keep]);
        }
        saved
    })
}

fn run(path: &Path, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(path)
        .arg("update")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start update: {e}"))?;
    let group = child.id() as i32;
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                break Err(
                    "Update timed out. Check the provider version before retrying.".to_string(),
                )
            }
            Err(e) => break Err(format!("Could not read update status: {e}")),
        }
    };
    // Self-updaters may spawn helpers inheriting output pipes. Close the whole group before
    // joining readers so neither a timeout nor a completed launcher can leave an orphan.
    unsafe {
        libc::kill(-group, libc::SIGKILL);
    }
    let _ = child.wait();
    let mut output = stdout.join().unwrap_or_default();
    output.extend(stderr.join().unwrap_or_default());
    let output = String::from_utf8_lossy(&output);
    let message = output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Update command completed.")
        .chars()
        .take(1000)
        .collect::<String>();
    if status?.success() {
        Ok(message)
    } else {
        Err(format!("Update failed: {message}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn script(root: &Path, body: &str) -> PathBuf {
        let path = root.join("provider");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    #[test]
    fn installation_detection_does_not_treat_npm_or_system_paths_as_native() {
        let home = Path::new("/home/fixture");
        assert_eq!(
            installation(
                Provider::Claude,
                &home.join(".local/share/claude/versions/1.2.3"),
                home
            ),
            Some("native")
        );
        assert_eq!(
            installation(
                Provider::Codex,
                &home.join(".codex/packages/standalone/releases/v1/bin/codex"),
                home
            ),
            Some("standalone")
        );
        for path in [
            "/usr/bin/codex",
            "/home/fixture/.npm/bin/codex",
            "/home/fixture/.local/share/claude/versions-other/custom",
        ] {
            assert!(installation(Provider::Codex, Path::new(path), home).is_none());
            assert!(installation(Provider::Claude, Path::new(path), home).is_none());
        }
    }
    #[test]
    fn updater_executes_only_update_and_captures_failures() {
        let root = tempfile::tempdir().unwrap();
        let path = script(
            root.path(),
            "[ \"$1\" = update ] || exit 4\n[ \"$#\" = 1 ] || exit 5\necho updated",
        );
        assert_eq!(run(&path, Duration::from_secs(2)).unwrap(), "updated");
        script(root.path(), "echo download-failed >&2\nexit 9");
        assert!(run(&path, Duration::from_secs(2))
            .unwrap_err()
            .contains("download-failed"));
    }
    #[test]
    fn updater_kills_a_hung_process_group_and_bounds_output() {
        // Both bounds below are deliberately far looser than the work they wrap. What this
        // test proves is that a hung group is killed rather than waited on, and that output is
        // truncated — neither claim is about how fast the machine is. Tight wall-clock budgets
        // made this the one test that failed at random when the suite runs in parallel, and a
        // test that fails at random teaches people to ignore the suite.
        let root = tempfile::tempdir().unwrap();
        let path = script(root.path(), "sleep 60 &\nwait");
        let started = Instant::now();
        assert!(run(&path, Duration::from_millis(100))
            .unwrap_err()
            .contains("timed out"));
        // The child sleeps for 60s; returning in under 10 proves it was not waited on.
        assert!(started.elapsed() < Duration::from_secs(10));
        script(root.path(), "head -c 200000 /dev/zero | tr '\\000' x");
        // Milliseconds of work. The timeout only has to be long enough not to fire.
        assert!(run(&path, Duration::from_secs(30)).unwrap().len() <= 1000);
    }

    #[test]
    fn registered_update_defaults_to_disabled_and_never_runs_an_unknown_install() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::Store::open(&root.path().join("store.db"), false).unwrap();
        let engine = Engine::new(crate::Instance::Test, store);
        let call = |op, payload| {
            engine
                .dispatch(
                    relay_bus::Request::new(relay_bus::Actor::User, op, payload),
                    crate::Door::InProcess,
                )
                .into_result()
                .unwrap()
        };
        assert_eq!(
            call(
                "provider.update",
                json!({"provider":"codex","automatic":true})
            )["method"],
            "disabled"
        );
        let marker = root.path().join("was-run");
        let path = script(root.path(), &format!("touch '{}'", marker.display()));
        call(
            "settings.set",
            json!({"path":"providers.codex.path","value":path}),
        );
        let result = call("provider.update", json!({"provider":"codex"}));
        assert_eq!(result["started"], false);
        assert_eq!(result["method"], "managed_or_unknown");
        assert!(
            !marker.exists(),
            "Unsupported installations must never execute an updater"
        );
    }
}
