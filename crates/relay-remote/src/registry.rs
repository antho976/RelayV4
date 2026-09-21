//! `remote.json`: which phones may talk to this engine, the pairing codes that are open right
//! now, and where the rendezvous server is. One small file per instance next to the store,
//! mode 0600, re-read on every handshake so `relay remote pair` in a second terminal and the
//! serving process never have to talk to each other.

use anyhow::{Context, Result};
use jiff::{Timestamp, ToSpan};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A phone that completed pairing. The token is the whole credential: it is sent once, at
/// pairing, and afterwards only proven with a per-connection challenge (`wire.rs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub token: String,
    pub created_at: String,
    #[serde(default)]
    pub last_seen: Option<String>,
}

/// A pairing code somebody typed `relay remote pair` for. Short-lived and single use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pending {
    pub code: String,
    pub created_at: String,
    pub expires_at: String,
}

/// The self-hosted rendezvous this engine dials out to (`tunnel.rs`), if any.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rendezvous {
    /// `wss://relay.example.org` or `ws://192.0.2.1:7430`; paths are appended.
    pub url: String,
    /// What a phone asks the server for. Derived from the secret, never guessable.
    pub room: String,
    /// What the host proves to the server. Never leaves this machine except to that server.
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub v: u32,
    pub host_id: String,
    pub host_name: String,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub pending: Vec<Pending>,
    #[serde(default)]
    pub rendezvous: Option<Rendezvous>,
    /// The port `relay remote serve` last listened on, so `relay remote pair` can print a link
    /// that points at it without being told.
    #[serde(default)]
    pub direct_port: Option<u16>,
}

pub const DEFAULT_PORT: u16 = 7420;
pub const DEFAULT_RENDEZVOUS_PORT: u16 = 7430;
const PAIR_TTL_MINUTES: i64 = 10;

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    hex(&buf)
}

pub fn sha256_hex(input: &str) -> String {
    use sha2::Digest as _;
    hex(&sha2::Sha256::digest(input.as_bytes()))
}

/// The room a secret opens: the rendezvous server checks `sha256(secret)` against the room it
/// is asked to host, so it stores nothing and a restart forgets nothing.
pub fn room_for(secret: &str) -> String {
    sha256_hex(secret)[..32].to_string()
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn host_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "relay".to_string())
}

/// A pairing code a person can read aloud: eight characters from an alphabet without 0/O, 1/I.
fn pair_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; 8];
    rand::rng().fill_bytes(&mut buf);
    let chars: Vec<char> = buf
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!(
        "{}-{}",
        chars[..4].iter().collect::<String>(),
        chars[4..].iter().collect::<String>()
    )
}

/// Codes are compared without case, dashes or spaces: what a person types is never exact.
pub fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

impl Registry {
    pub fn path_for(instance: relay_core::Instance) -> PathBuf {
        instance.data_dir().join("remote.json")
    }

    pub fn fresh() -> Registry {
        Registry {
            v: 1,
            host_id: random_hex(8),
            host_name: host_name(),
            devices: Vec::new(),
            pending: Vec::new(),
            rendezvous: None,
            direct_port: None,
        }
    }

    /// Read the file, or start a fresh registry when there is none yet. A registry that cannot
    /// be parsed is an error, not a silent reset: it holds every phone's credential.
    pub fn load(path: &Path) -> Result<Registry> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut reg: Registry = serde_json::from_str(&text)
                    .with_context(|| format!("parsing {}", path.display()))?;
                reg.prune();
                Ok(reg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::fresh()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .with_context(|| format!("writing {}", tmp.display()))?;
            f.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
            f.write_all(b"\n")?;
        }
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Drop pairing codes that expired.
    pub fn prune(&mut self) {
        let now = now();
        self.pending.retain(|p| {
            p.expires_at
                .parse::<Timestamp>()
                .map(|t| t > now)
                .unwrap_or(false)
        });
    }

    /// Open a pairing window: one code, ten minutes, single use.
    pub fn begin_pair(&mut self) -> Pending {
        let created = now();
        let pending = Pending {
            code: pair_code(),
            created_at: created.to_string(),
            expires_at: (created + PAIR_TTL_MINUTES.minutes()).to_string(),
        };
        self.pending.push(pending.clone());
        pending
    }

    /// Redeem a code for a device credential. The code is consumed whether or not the rest of
    /// the handshake succeeds, so it cannot be tried twice.
    pub fn redeem(&mut self, code: &str, device_name: &str) -> Option<Device> {
        self.prune();
        let wanted = normalize_code(code);
        if wanted.is_empty() {
            return None;
        }
        let idx = self
            .pending
            .iter()
            .position(|p| normalize_code(&p.code) == wanted)?;
        self.pending.remove(idx);
        let name = device_name.trim();
        let device = Device {
            id: random_hex(6),
            name: if name.is_empty() { "phone".to_string() } else { name.chars().take(64).collect() },
            token: random_hex(32),
            created_at: now().to_string(),
            last_seen: None,
        };
        self.devices.push(device.clone());
        Some(device)
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn touch(&mut self, id: &str) {
        if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
            d.last_seen = Some(now().to_string());
        }
    }

    pub fn revoke(&mut self, id: &str) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| d.id != id);
        self.devices.len() != before
    }

    /// Point this engine at a rendezvous server, minting a room secret the first time.
    pub fn set_rendezvous(&mut self, url: &str) -> &Rendezvous {
        let url = url.trim().trim_end_matches('/').to_string();
        match self.rendezvous.as_mut() {
            Some(r) => r.url = url,
            None => {
                let secret = random_hex(32);
                self.rendezvous = Some(Rendezvous {
                    url,
                    room: room_for(&secret),
                    secret,
                });
            }
        }
        self.rendezvous.as_ref().expect("just set")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_pairs_once_and_then_is_gone() {
        let mut reg = Registry::fresh();
        let pending = reg.begin_pair();
        assert_eq!(pending.code.len(), 9);
        let device = reg.redeem(&pending.code.to_lowercase().replace('-', " "), "  Pixel 8 ").unwrap();
        assert_eq!(device.name, "Pixel 8");
        assert_eq!(device.token.len(), 64);
        assert!(reg.redeem(&pending.code, "again").is_none(), "codes are single use");
        assert_eq!(reg.device(&device.id).map(|d| &d.token), Some(&device.token));
        assert!(reg.revoke(&device.id));
        assert!(reg.device(&device.id).is_none());
    }

    #[test]
    fn expired_codes_are_pruned_on_load_and_redeem() {
        let mut reg = Registry::fresh();
        reg.pending.push(Pending {
            code: "AAAA-BBBB".into(),
            created_at: "2020-01-01T00:00:00Z".into(),
            expires_at: "2020-01-01T00:10:00Z".into(),
        });
        assert!(reg.redeem("AAAA-BBBB", "late").is_none());
        assert!(reg.pending.is_empty());
    }

    #[test]
    fn the_file_round_trips_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("remote.json");
        let mut reg = Registry::fresh();
        reg.begin_pair();
        reg.set_rendezvous("wss://relay.example.org/");
        reg.save(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let back = Registry::load(&path).unwrap();
        assert_eq!(back, reg);
        assert_eq!(back.rendezvous.as_ref().unwrap().url, "wss://relay.example.org");
        assert_eq!(
            back.rendezvous.as_ref().unwrap().room,
            room_for(&back.rendezvous.as_ref().unwrap().secret)
        );
    }

    #[test]
    fn a_missing_file_is_a_fresh_registry_but_a_broken_one_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.json");
        assert!(Registry::load(&path).unwrap().devices.is_empty());
        std::fs::write(&path, "{ not json").unwrap();
        assert!(Registry::load(&path).is_err());
    }
}
