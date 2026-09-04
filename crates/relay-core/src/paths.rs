//! Where an instance lives on disk (BUS.md §6.2): store under XDG data, socket + lock under
//! XDG runtime, one set per instance so dev and stable never cross-corrupt (SPEC §1).

use std::fmt;
use std::path::PathBuf;

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
    /// `$XDG_RUNTIME_DIR/relay-v4/` (falls back to `/tmp/relay-v4-<uid>/`).
    pub fn runtime_dir(self) -> PathBuf {
        let base = directories::BaseDirs::new()
            .and_then(|b| b.runtime_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| {
                let uid = unsafe { libc::getuid() };
                std::env::temp_dir().join(format!("relay-v4-{uid}"))
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

impl fmt::Display for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
