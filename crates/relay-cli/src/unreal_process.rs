//! The Unreal editor as a process: which engine a project builds with, whether an editor (or a
//! leftover crash reporter) is running, whether the Remote Control port is free, launching and
//! reading the editor log for bind failures, and the hot-reload module check after a build.
//!
//! Every lesson here came from a real session: a missing `UE_ROOT` stopped every build; a
//! crash reporter left behind made UnrealBuildTool build a numbered hot-reload module the
//! editor never loaded; and a quick relaunch lost the Remote Control port for a while.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A running process, as `/proc` shows it (Linux only; empty elsewhere).
#[derive(Debug, Clone)]
pub struct Proc {
    pub pid: u32,
    pub exe: PathBuf,
    pub cmdline: Vec<String>,
}

impl Proc {
    pub fn name(&self) -> String {
        self.exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

pub fn processes() -> Vec<Proc> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok())
        .filter_map(|pid| {
            let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
            let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
            let cmdline = raw.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
            Some(Proc { pid, exe, cmdline })
        })
        .collect()
}

fn is_editor(p: &Proc) -> bool {
    let name = p.name();
    (name == "UnrealEditor" || name == "UnrealEditor.exe" || name == "UE4Editor") && !p.cmdline.iter().any(|a| a.contains("-run=") || a == "-nullrhi")
}

fn is_crash_reporter(p: &Proc) -> bool {
    p.name().starts_with("CrashReportClient")
}

/// Editors that have this project open.
pub fn editors_for(uproject: &Path) -> Vec<Proc> {
    let want = uproject.canonicalize().unwrap_or_else(|_| uproject.to_path_buf());
    processes()
        .into_iter()
        .filter(is_editor)
        .filter(|p| p.cmdline.iter().any(|a| Path::new(a).canonicalize().map(|c| c == want).unwrap_or(false) || a.ends_with(&*uproject.file_name().unwrap_or_default().to_string_lossy())))
        .collect()
}

pub fn crash_reporters() -> Vec<Proc> {
    processes().into_iter().filter(is_crash_reporter).collect()
}

pub fn kill(pid: u32) -> bool {
    std::process::Command::new("kill").arg(pid.to_string()).status().is_ok_and(|s| s.success())
}

// ---------------------------------------------------------------- which engine

/// `(major, minor)` from an engine's `Engine/Build/Build.version`.
fn engine_version(root: &Path) -> Option<(u64, u64)> {
    let raw = std::fs::read_to_string(root.join("Engine/Build/Build.version")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    Some((v["MajorVersion"].as_u64()?, v["MinorVersion"].as_u64()?))
}

fn strip_braces(s: &str) -> String {
    s.trim().trim_start_matches('{').trim_end_matches('}').to_ascii_lowercase()
}

/// Engine folders registered in Epic's `Install.ini` (launcher and source builds on Linux and
/// macOS), as `(key, path)`.
fn registered_engines() -> Vec<(String, PathBuf)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let mut out = Vec::new();
    for ini in [home.join(".config/Epic/UnrealEngine/Install.ini"), home.join("Library/Application Support/Epic/UnrealEngine/Install.ini")] {
        let Ok(text) = std::fs::read_to_string(&ini) else { continue };
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') || line.starts_with(';') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let path = PathBuf::from(value.trim().trim_matches('"'));
                if !key.trim().is_empty() {
                    out.push((key.trim().to_string(), path));
                }
            }
        }
    }
    out
}

/// The engine directory (the one holding `Engine/`) for a project, trying in order: `UE_ROOT`;
/// an editor already running this project; the project's own log ("Base Directory"); a source
/// tree around the project; `Install.ini` by association key (braces optional) and then by
/// version; common install folders. The error lists everything tried.
pub fn engine_root(uproject: &Path, project_root: &Path, association: &str, log: &Path) -> Result<PathBuf> {
    let valid = |p: &Path| p.join("Engine/Build/BatchFiles").is_dir();
    let mut tried: Vec<String> = Vec::new();
    if let Some(root) = std::env::var_os("UE_ROOT").map(PathBuf::from) {
        if valid(&root) {
            return Ok(root);
        }
        bail!("UE_ROOT={} has no Engine/Build/BatchFiles", root.display());
    }
    // An editor that is running this project knows exactly which engine it is.
    for editor in editors_for(uproject) {
        if let Some(root) = editor.exe.ancestors().find(|a| valid(a)) {
            return Ok(root.to_path_buf());
        }
    }
    tried.push("a running editor for this project".into());
    // The last editor session logged its binary directory.
    if let Ok(text) = std::fs::read_to_string(log) {
        for line in text.lines().take(400) {
            if let Some(rest) = line.split("Base Directory:").nth(1) {
                let dir = PathBuf::from(rest.trim());
                if let Some(root) = dir.ancestors().find(|a| valid(a)) {
                    return Ok(root.to_path_buf());
                }
            }
        }
    }
    tried.push(format!("\"Base Directory\" in {}", log.display()));
    for dir in project_root.ancestors().skip(1) {
        if valid(dir) {
            return Ok(dir.to_path_buf());
        }
    }
    tried.push("a source engine tree around the project".into());
    let registered = registered_engines();
    let want = strip_braces(association);
    if let Some((_, path)) = registered.iter().find(|(key, path)| strip_braces(key) == want && valid(path)) {
        return Ok(path.clone());
    }
    // "5.8" in the .uproject but the engine registered under a GUID: match by version.
    let wanted_version = association.split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>()
        .filter(|v| v.len() >= 2).map(|v| (v[0], v[1]));
    if let Some(version) = wanted_version {
        if let Some((_, path)) = registered.iter().find(|(_, path)| valid(path) && engine_version(path) == Some(version)) {
            return Ok(path.clone());
        }
    }
    tried.push(format!("Install.ini entries: {}", if registered.is_empty() { "none".to_string() } else { registered.iter().map(|(k, p)| format!("{k}={}", p.display())).collect::<Vec<_>>().join(", ") }));
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for base in [home.clone(), home.join("dev"), home.join("Epic"), home.join("UnrealEngine")] {
            if let Ok(entries) = std::fs::read_dir(&base) {
                for e in entries.flatten() {
                    let n = e.file_name().to_string_lossy().to_lowercase();
                    if n.contains("unreal") || n.starts_with("ue_") || n.starts_with("ue5") || n.starts_with("ue-") {
                        candidates.push(e.path());
                    }
                }
            }
        }
        candidates.push(home.join("UnrealEngine"));
    }
    for base in ["/opt", "/Users/Shared/Epic Games", "C:/Program Files/Epic Games"] {
        if let Ok(entries) = std::fs::read_dir(base) {
            candidates.extend(entries.flatten().map(|e| e.path()));
        }
    }
    let mut found: Vec<PathBuf> = candidates.into_iter().filter(|c| valid(c)).collect();
    found.sort();
    found.dedup();
    if let Some(version) = wanted_version {
        if let Some(root) = found.iter().find(|p| engine_version(p) == Some(version)) {
            return Ok(root.clone());
        }
    }
    // A single installed engine is only a safe guess when the project names no version.
    if found.len() == 1 && wanted_version.is_none() {
        return Ok(found.remove(0));
    }
    tried.push(format!("common folders under ~, ~/dev, /opt ({} engines found{})", found.len(),
        if found.is_empty() { String::new() } else { format!(": {}", found.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")) }));
    Err(anyhow!(
        "cannot find the engine for EngineAssociation {association:?}. Tried: {}. Set UE_ROOT to the folder that contains Engine/ (in the environment Relay's engine starts from), or open the project in the editor once so its log records the engine path.",
        tried.join("; ")
    ))
}

pub fn editor_binary(engine: &Path) -> PathBuf {
    let bin = engine.join("Engine/Binaries");
    if cfg!(target_os = "windows") {
        bin.join("Win64/UnrealEditor.exe")
    } else if cfg!(target_os = "macos") {
        bin.join("Mac/UnrealEditor.app/Contents/MacOS/UnrealEditor")
    } else {
        bin.join("Linux/UnrealEditor")
    }
}

// ---------------------------------------------------------------- before and after a build

/// What stands between a build and an editor that runs the new code. A leftover crash reporter
/// makes UnrealBuildTool believe an editor is running and build a hot-reload module instead.
pub fn prebuild(uproject: &Path, kill_crash_reporters: bool) -> Value {
    let editors = editors_for(uproject);
    let reporters = crash_reporters();
    let mut killed = Vec::new();
    if kill_crash_reporters {
        for r in &reporters {
            if kill(r.pid) {
                killed.push(r.pid);
            }
        }
        if !killed.is_empty() {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    json!({
        "editor_running": editors.iter().map(|p| p.pid).collect::<Vec<_>>(),
        "crash_reporters": reporters.iter().map(|p| p.pid).collect::<Vec<_>>(),
        "killed_crash_reporters": killed,
    })
}

/// Modules the editor loads, from `Binaries/<Platform>/UnrealEditor.modules`, and whether any is
/// a numbered hot-reload build (`...-MyGame-0001.so`) the editor may not pick up on restart.
pub fn module_manifest(project_root: &Path, platform: &str) -> Value {
    let path = project_root.join("Binaries").join(platform).join("UnrealEditor.modules");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return json!({"manifest": path, "found": false});
    };
    let manifest: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let re = regex::Regex::new(r"-\d{4}(\.[A-Za-z]+)?$").unwrap();
    let modules = manifest["Modules"].as_object().cloned().unwrap_or_default();
    let numbered: Vec<String> = modules
        .iter()
        .filter_map(|(name, file)| {
            let f = file.as_str()?;
            let stem = Path::new(f).file_stem()?.to_string_lossy().into_owned();
            re.is_match(&stem).then(|| format!("{name} -> {f}"))
        })
        .collect();
    json!({"manifest": path, "found": true, "build_id": manifest["BuildId"], "modules": modules, "hot_reload_modules": numbered})
}

// ---------------------------------------------------------------- the Remote Control port

pub fn port_of(url: &str) -> u16 {
    url.rsplit(':').next().and_then(|p| p.trim_end_matches('/').parse().ok()).unwrap_or(30010)
}

/// Whether nothing is listening on (or still holding) the port.
pub fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Something accepts connections on the port: a live server, not just closed connections.
pub fn port_listening(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(300)).is_ok()
}

pub fn wait_port_free(port: u16, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if port_free(port) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Log lines saying the web server could not bind: the editor is up but unreachable, and
/// `WebControl.StartServer` alone does nothing until `WebControl.StopServer` runs first.
pub fn bind_failures(log_tail: &[String]) -> Vec<String> {
    let re = regex::Regex::new(r"(?i)(unable to bind|failed to bind|could not bind|address already in use|bind.*failed).*").unwrap();
    log_tail.iter().filter(|l| re.is_match(l) && (l.contains("Http") || l.contains("WebControl") || l.contains("RemoteControl") || l.contains("Socket"))).cloned().collect()
}

/// Start the editor on the project, detached, once the port is free.
pub fn launch(engine: &Path, uproject: &Path, port: u16, extra: &[String]) -> Result<u32> {
    let binary = editor_binary(engine);
    anyhow::ensure!(binary.is_file(), "{} does not exist (build the editor first)", binary.display());
    if !wait_port_free(port, Duration::from_secs(90)) {
        bail!("port {port} is still held (a previous editor, or another program). Wait, or free it, before launching: the new editor could not start its Remote Control server");
    }
    let mut command = std::process::Command::new(&binary);
    command
        .arg(uproject)
        // Starts the Remote Control web server even when auto-start is off in the project.
        .arg("-RCWebControlEnable")
        .args(extra)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        // Its own process group: the editor outlives this MCP server and the agent.
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn().with_context(|| format!("starting {}", binary.display()))?;
    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_reload_modules_are_flagged() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("Binaries/Linux");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("UnrealEditor.modules"), r#"{"BuildId":"abc","Modules":{"Shooter":"libUnrealEditor-Shooter-0003.so","ShooterUI":"libUnrealEditor-ShooterUI.so"}}"#).unwrap();
        let m = module_manifest(root.path(), "Linux");
        assert_eq!(m["hot_reload_modules"], json!(["Shooter -> libUnrealEditor-Shooter-0003.so"]));
        assert_eq!(module_manifest(root.path(), "Win64")["found"], false);
    }

    #[test]
    fn bind_failures_are_found_in_the_log() {
        let lines = vec![
            "LogHttpListener: Error: HttpListener unable to bind to 127.0.0.1:30010".to_string(),
            "LogTemp: Display: unable to bind a key".to_string(),
            "LogInit: Display: Engine started".to_string(),
        ];
        assert_eq!(bind_failures(&lines).len(), 1);
    }

    #[test]
    fn a_held_port_is_not_free() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!port_free(port));
        assert!(port_listening(port));
        drop(listener);
        assert!(!port_listening(port));
        assert!(wait_port_free(port, Duration::from_secs(5)));
        assert_eq!(port_of("http://127.0.0.1:30010"), 30010);
        assert_eq!(port_of("http://localhost:31000/"), 31000);
    }

    #[test]
    fn the_engine_is_found_by_version_when_registered_under_a_guid() {
        let home = tempfile::tempdir().unwrap();
        let engine = home.path().join("dev/UnrealEngine");
        std::fs::create_dir_all(engine.join("Engine/Build/BatchFiles")).unwrap();
        std::fs::write(engine.join("Engine/Build/Build.version"), r#"{"MajorVersion":5,"MinorVersion":8,"PatchVersion":0}"#).unwrap();
        let ini = home.path().join(".config/Epic/UnrealEngine");
        std::fs::create_dir_all(&ini).unwrap();
        std::fs::write(ini.join("Install.ini"), format!("[Installations]\n{{0A1B2C3D-0000}}={}\n", engine.display())).unwrap();
        let project = home.path().join("Game");
        std::fs::create_dir_all(&project).unwrap();
        // SAFETY: the only test in this binary that reads HOME or UE_ROOT.
        unsafe {
            std::env::set_var("HOME", home.path());
            std::env::remove_var("UE_ROOT");
        }
        let log = project.join("Saved/Logs/Game.log");
        let by_guid = engine_root(&project.join("Game.uproject"), &project, "{0A1B2C3D-0000}", &log).unwrap();
        assert_eq!(by_guid, engine);
        let by_guid_without_braces = engine_root(&project.join("Game.uproject"), &project, "0a1b2c3d-0000", &log).unwrap();
        assert_eq!(by_guid_without_braces, engine);
        let by_version = engine_root(&project.join("Game.uproject"), &project, "5.8", &log).unwrap();
        assert_eq!(by_version, engine);
        let wrong_version = engine_root(&project.join("Game.uproject"), &project, "4.27", &log).unwrap_err();
        assert!(format!("{wrong_version}").contains("Install.ini entries"), "{wrong_version}");
        // The last editor session's log names the engine outright.
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, format!("LogInit: Base Directory: {}/Engine/Binaries/Linux/\n", engine.display())).unwrap();
        assert_eq!(engine_root(&project.join("Game.uproject"), &project, "4.27", &log).unwrap(), engine);
    }
}
