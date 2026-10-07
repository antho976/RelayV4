//! The Blender plugin's children under bubblewrap (RA-077).
//!
//! `blender_python` runs whatever Python the agent sends, as the user. The guardrail gate
//! decides whether it runs at all (mcp::plugin_gate); this decides what it can touch once it
//! does. Inside a Relay session on Linux with a working `bwrap`, every Blender child sees the
//! whole file system read-only and may write only to the session's write roots (the guardrail's
//! own list, `bus.whoami`), the job's folder and Blender's sandbox config/cache. The network is
//! off: no tool, add-ons included, needs it. GPU devices stay reachable for EEVEE and Cycles.
//!
//! Outside a session (a standalone plugin) or without bwrap (macOS, Windows, a kernel without
//! user namespaces) Blender runs as before, and every result says so in `sandbox`: the sandbox
//! is never silently off.
//!
//! What it does not stop: reading anything the user can read, and anything written inside the
//! write roots, which is the worktree the agent may edit anyway.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

pub(crate) enum Sandbox {
    /// Children run under bwrap and may write to these folders, besides their own job's.
    On { bwrap: PathBuf, writable: Vec<PathBuf>, from: String },
    Off(String),
}

impl Sandbox {
    /// The sandbox for a Blender child of this server. `checkout` is the fallback write root
    /// when the session's own list cannot be read.
    pub(crate) fn current(checkout: &Path) -> Sandbox {
        #[cfg(test)]
        if let Some(writable) = TEST_ROOTS.with(|roots| roots.borrow().clone()) {
            return match bwrap() {
                Ok(bwrap) => Sandbox::On { bwrap, writable, from: "test roots".into() },
                Err(why) => Sandbox::Off(why),
            };
        }
        if std::env::var("RELAY_SESSION").map_or(true, |s| s.is_empty()) {
            return Sandbox::Off("not a Relay session; Blender runs unconfined".into());
        }
        if cfg!(test) {
            // Unit tests run inside agent sessions; they must not reach the live engine.
            return Sandbox::Off("unit test".into());
        }
        let bwrap = match bwrap() {
            Ok(bwrap) => bwrap,
            Err(why) => {
                eprintln!("relay blender-mcp: {why}");
                return Sandbox::Off(why);
            }
        };
        match crate::mcp::session_write_roots() {
            Ok(roots) if !roots.is_empty() => Sandbox::On { bwrap, writable: roots, from: "the session's guardrail write roots".into() },
            Ok(_) => Sandbox::On { bwrap, writable: vec![checkout.to_path_buf()], from: "the worktree (the session reported no write roots)".into() },
            Err(error) => Sandbox::On {
                bwrap,
                writable: vec![checkout.to_path_buf()],
                from: format!("the worktree only (write roots unavailable: {error:#})"),
            },
        }
    }

    /// `command` (a Blender invocation) as this sandbox runs it. `job` holds the script and is
    /// the child's TMPDIR; `extra` are further folders this one run writes (a render's output).
    pub(crate) fn wrap(&self, command: Command, job: &Path, extra: &[PathBuf]) -> Result<Command> {
        let Sandbox::On { bwrap, writable, .. } = self else { return Ok(command) };
        let state = state_dir()?;
        let (config, cache) = (state.join("config"), state.join("cache"));
        for dir in [&config, &cache] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let mut sandboxed = Command::new(bwrap);
        sandboxed.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-net", "--unshare-pid", "--die-with-parent"]);
        for device in gpu_devices() {
            sandboxed.arg("--dev-bind-try").arg(&device).arg(&device);
        }
        let mut bound = Vec::new();
        for dir in writable.iter().chain(extra).chain([&job.to_path_buf(), &state]) {
            // A root that does not exist yet has nothing to write into; binding it would fail.
            let Ok(real) = std::fs::canonicalize(dir) else { continue };
            if !bound.contains(&real) {
                sandboxed.arg("--bind").arg(&real).arg(&real);
                bound.push(real);
            }
        }
        if let Some(cwd) = command.get_current_dir() {
            sandboxed.arg("--chdir").arg(cwd);
        }
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => sandboxed.env(key, value),
                None => sandboxed.env_remove(key),
            };
        }
        // Blender's config and caches (its own, Mesa's, NVIDIA's) go to the sandbox's folder, and
        // its temp files to the job's.
        sandboxed.env("XDG_CONFIG_HOME", &config).env("XDG_CACHE_HOME", &cache).env("TMPDIR", job);
        sandboxed.arg("--").arg(command.get_program()).args(command.get_args());
        Ok(sandboxed)
    }

    /// One line for the tool result: whether the child was confined, and to what.
    pub(crate) fn describe(&self) -> String {
        match self {
            Sandbox::On { writable, from, .. } => format!(
                "bwrap: read-only file system, no network; writes allowed only in {} ({from}) and the job's own folders",
                writable.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            ),
            Sandbox::Off(why) => format!("off: {why}"),
        }
    }
}

/// A working bwrap, found and tried once per server. Installed is not enough: a kernel or
/// distribution that forbids unprivileged user namespaces makes every run fail.
fn bwrap() -> Result<PathBuf, String> {
    static FOUND: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    FOUND.get_or_init(|| {
        if !cfg!(target_os = "linux") {
            return Err("the Blender sandbox needs bubblewrap, which is Linux-only; Blender runs unconfined".into());
        }
        let bwrap = std::env::var_os("PATH")
            .and_then(|paths| std::env::split_paths(&paths).map(|dir| dir.join("bwrap")).find(|p| p.is_file()))
            .or_else(|| Some(PathBuf::from("/usr/bin/bwrap")).filter(|p| p.is_file()))
            .ok_or("bubblewrap (bwrap) is not installed; Blender runs unconfined")?;
        let tried = relay_core::proc::output_with_timeout(
            Command::new(&bwrap).args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-net", "--unshare-pid", "--die-with-parent", "true"]),
            Duration::from_secs(10),
        );
        match tried {
            Ok(Some(output)) if output.status.success() => Ok(bwrap),
            Ok(Some(output)) => Err(format!("bwrap does not work here ({}); Blender runs unconfined", String::from_utf8_lossy(&output.stderr).trim())),
            Ok(None) => Err("bwrap did not start within 10 s; Blender runs unconfined".into()),
            Err(error) => Err(format!("bwrap could not run ({error}); Blender runs unconfined")),
        }
    }).clone()
}

/// Kept across runs, so shader caches survive; never the user's own Blender config.
fn state_dir() -> Result<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .context("no cache directory for the Blender sandbox (HOME is unset)")?;
    Ok(cache.join("relay").join("blender-sandbox"))
}

/// Render devices: DRM nodes for EEVEE and Mesa, the NVIDIA nodes for CUDA/OptiX and its EGL.
fn gpu_devices() -> Vec<PathBuf> {
    let mut devices = vec![PathBuf::from("/dev/dri")];
    if let Ok(entries) = std::fs::read_dir("/dev") {
        devices.extend(entries.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("nvidia"))));
    }
    devices.sort();
    devices
}

#[cfg(test)]
thread_local! {
    static TEST_ROOTS: std::cell::RefCell<Option<Vec<PathBuf>>> = const { std::cell::RefCell::new(None) };
}

/// Sandbox the Blender runs this test thread makes, with `roots` writable, until dropped.
#[cfg(test)]
pub(crate) fn test_roots(roots: &[&Path]) -> impl Drop {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ROOTS.with(|r| *r.borrow_mut() = None);
        }
    }
    TEST_ROOTS.with(|r| *r.borrow_mut() = Some(roots.iter().map(|p| p.to_path_buf()).collect()));
    Reset
}

#[cfg(test)]
pub(crate) fn available() -> bool {
    bwrap().is_ok()
}
