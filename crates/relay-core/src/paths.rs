//! Where an instance lives on disk (BUS.md §6.2): store under XDG data, socket + lock under
//! XDG runtime, one set per instance so dev and stable never cross-corrupt (SPEC §1).

use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Instance {
    Stable,
    Dev,
    Test,
}

impl Instance {
    pub fn from_env() -> Self {
        match std::env::var("RELAY_INSTANCE").as_deref() {
            Ok("dev") => Instance::Dev,
            Ok("test") => Instance::Test,
            _ => Instance::Stable,
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "stable" => Some(Instance::Stable),
            "dev" => Some(Instance::Dev),
            "test" => Some(Instance::Test),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Instance::Stable => "stable",
            Instance::Dev => "dev",
            Instance::Test => "test",
        }
    }
    /// The Tauri bundle identifier this instance corresponds to.
    pub fn bundle_id(self) -> &'static str {
        match self {
            Instance::Stable => "com.quietsoftware.relay",
            Instance::Dev => "com.quietsoftware.relay.dev",
            Instance::Test => "com.quietsoftware.relay.test",
        }
    }
    /// `test` actors are only accepted by dev/test engines (BUS.md §4.1).
    pub fn accepts_test_actor(self) -> bool {
        !matches!(self, Instance::Stable)
    }
    /// V4 is isolated from the Relay-2/V3 engine and store.
    /// `~/.local/share/relay-v4/<instance>/`
    pub fn data_dir(self) -> PathBuf {
        let base = directories::BaseDirs::new()
            .map(|b| b.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("relay-v4").join(self.as_str())
    }
    pub fn store_path(self) -> PathBuf {
        self.data_dir().join("store.db")
    }
    pub fn backup_dir(self) -> PathBuf {
        self.data_dir().join("backups")
    }
    pub fn log_dir(self) -> PathBuf {
        self.data_dir().join("logs")
    }
    /// `$XDG_RUNTIME_DIR/relay-v4/`. Without that variable (cron, `su`, ssh without
    /// pam_systemd): `/run/user/<uid>/relay-v4/` when that directory is ours and private, else
    /// `~/.cache/relay-v4-runtime/relay-v4/`. Never a predictable name in a shared `/tmp`: any
    /// local user could create it first and own the directory the socket and lock live in
    /// (RA-013). The engine checks the directory with [`check_private_dir`] before using it.
    pub fn runtime_dir(self) -> PathBuf {
        let base = directories::BaseDirs::new()
            .and_then(|b| b.runtime_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| {
                let uid = unsafe { libc::getuid() };
                let run = PathBuf::from(format!("/run/user/{uid}"));
                if check_private_dir(&run).is_ok() {
                    return run;
                }
                directories::BaseDirs::new()
                    .map(|b| b.cache_dir().join("relay-v4-runtime"))
                    .unwrap_or_else(|| std::env::temp_dir().join(format!("relay-v4-{uid}")))
            });
        base.join("relay-v4")
    }
    pub fn socket_path(self) -> PathBuf {
        self.runtime_dir().join(format!("{}.sock", self.as_str()))
    }
    pub fn lock_path(self) -> PathBuf {
        self.runtime_dir().join(format!("{}.lock", self.as_str()))
    }
}

/// Refuse a directory another user could tamper with: it must be a real directory (not a
/// symlink), owned by this user, with no group or other permission bits.
pub fn check_private_dir(dir: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(dir)?;
    let uid = unsafe { libc::getuid() };
    let problem = if meta.file_type().is_symlink() || !meta.is_dir() {
        "is not a directory"
    } else if meta.uid() != uid {
        "is owned by another user"
    } else if meta.mode() & 0o077 != 0 {
        "is open to other users"
    } else {
        return Ok(());
    };
    Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{} {problem}", dir.display())))
}

/// Refuse a parent another user could use to swap `dir` out from under us: it must be ours or
/// root's, and if it is not ours, nobody else may write to it except under the sticky bit
/// (`/tmp`).
pub fn check_parent_dir(dir: &Path) -> std::io::Result<()> {
    let Some(parent) = dir.parent().filter(|parent| !parent.as_os_str().is_empty()) else { return Ok(()) };
    let meta = std::fs::metadata(parent)?;
    let uid = unsafe { libc::getuid() };
    let shared_writable = meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0;
    if meta.uid() == uid || meta.uid() == 0 && !shared_writable {
        return Ok(());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("{} belongs to another user", parent.display()),
    ))
}

impl fmt::Display for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
