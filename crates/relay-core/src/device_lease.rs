//! Device leases: one holder per Android device at a time.
//!
//! Several agent sessions working on one app used to install over each other on the same phone —
//! one `gradlew installDebug` replacing the APK another session was halfway through testing, with
//! neither knowing the other existed. A lease makes the phone's current user visible and turns
//! the clobber into a typed refusal (`device.busy`) that names the holder, what it is doing and
//! since when, and says how to wait (`bus.wait` on `device.lease.released`).
//!
//! Three things take a lease:
//! - `device.run` — for as long as the run is live (building, installing, streaming logcat);
//! - an agent's own device-writing shell command (`adb install`, `adb shell am start`,
//!   `gradlew installDebug`, …), seen by the guardrail hook before it runs, and kept for a short
//!   grace period after it finishes so the agent can look at what it installed;
//! - `device.claim`, an explicit hold across several steps.
//!
//! Leases live in memory: a restart is a release, which is exactly right for a crash. A lease
//! also lapses with its run, its session's exit or close, or its own expiry. The map sits behind
//! its own small mutex and never touches SQLite, so taking one costs nothing in the store lock.

use relay_bus::error::BusError;
use relay_bus::types::{DeviceLease, Id};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The key of a lease whose command named no device. `adb` without `-s` uses the only device
/// and Gradle's `install*` tasks install on every device, so it conflicts with all of them.
pub const ANY_DEVICE: &str = "*";
/// How long a shell lease lasts while its command may still be running. Claude's Bash tool
/// gives up on a command after ten minutes; the hook that ends the command shortens this.
pub const SHELL_RUNNING: Duration = Duration::from_secs(15 * 60);
/// How long a shell lease outlives the command that took it: enough to look at the screen or
/// the log of what was just installed before someone else installs over it.
pub const SHELL_GRACE: Duration = Duration::from_secs(90);
pub const CLAIM_DEFAULT_MINUTES: u32 = 10;
pub const CLAIM_MAX_MINUTES: u32 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    Session { id: Id, name: String },
    User,
}

impl Holder {
    pub fn session_id(&self) -> Option<Id> {
        match self {
            Holder::Session { id, .. } => Some(*id),
            Holder::User => None,
        }
    }
    fn describe(&self) -> String {
        match self {
            Holder::Session { name, .. } => format!("session {name}"),
            Holder::User => "you (the Relay user)".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Lasts exactly as long as the `device.run` it belongs to.
    Run(Id),
    Claim,
    Shell,
}

impl Kind {
    fn rank(self) -> u8 {
        match self {
            Kind::Run(_) => 2,
            Kind::Claim => 1,
            Kind::Shell => 0,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Kind::Run(_) => "run",
            Kind::Claim => "claim",
            Kind::Shell => "shell",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Lease {
    pub device: String,
    pub holder: Holder,
    pub kind: Kind,
    pub action: String,
    pub since: String,
    started: Instant,
    expires: Option<Instant>,
}

impl Lease {
    pub fn new(device: &str, holder: Holder, kind: Kind, action: &str, ttl: Option<Duration>) -> Lease {
        let now = Instant::now();
        Lease {
            device: if device.trim().is_empty() { ANY_DEVICE.into() } else { device.trim().to_string() },
            holder,
            kind,
            action: clip(action),
            since: crate::time::now(),
            started: now,
            expires: ttl.map(|ttl| now + ttl),
        }
    }

    pub fn view(&self) -> DeviceLease {
        DeviceLease {
            device: self.device.clone(),
            session: match &self.holder {
                Holder::Session { name, .. } => Some(name.clone()),
                Holder::User => None,
            },
            kind: self.kind.as_str().into(),
            action: self.action.clone(),
            since: self.since.clone(),
            run_id: match self.kind { Kind::Run(id) => Some(id), _ => None },
            expires_in_s: self.expires.map(|at| at.saturating_duration_since(Instant::now()).as_secs()),
        }
    }

    /// The payload of `device.lease.acquired` / `device.lease.released`.
    pub fn event(&self) -> Value {
        serde_json::to_value(self.view()).unwrap_or_else(|_| json!({"device": self.device}))
    }

    fn overlaps(&self, device: &str) -> bool {
        self.device == device || self.device == ANY_DEVICE || device == ANY_DEVICE
    }
}

/// The refusal a conflicting caller gets: who, doing what, since when, and how to wait.
pub fn busy(lease: &Lease) -> BusError {
    let device = if lease.device == ANY_DEVICE { "every connected device".to_string() } else { format!("device {}", lease.device) };
    let mut message = format!(
        "{device} is in use by {}: {} (since {}, {} ago)",
        lease.holder.describe(), lease.action, clock(&lease.since), ago(lease.started.elapsed()),
    );
    match lease.kind {
        Kind::Run(id) => message.push_str(&format!("; it is device run {id}, which holds the device until it stops")),
        _ => if let Some(at) = lease.expires {
            message.push_str(&format!("; the lease lapses in {} unless renewed", ago(at.saturating_duration_since(Instant::now()))));
        },
    }
    let mut hint = String::from(
        "wait for it with bus.wait {\"events\":[\"device.lease.released\"]} and retry; device.list and device.leases show every holder",
    );
    if let Holder::Session { name, .. } = &lease.holder {
        hint.push_str(&format!("; to coordinate, mailbox.send {{\"to\":\"{name}\",...}}"));
    }
    BusError::conflict("device.busy", message)
        .with_hint(hint)
        .with_details(json!({ "lease": lease.event() }))
}

#[derive(Default)]
pub struct Leases {
    inner: Mutex<HashMap<String, Lease>>,
}

impl Leases {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Lease>> {
        self.inner.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// Take (or renew) a lease on `lease.device`. Refuses with [`busy`] when someone else holds
    /// that device, or holds every device, or — for `*` — holds any device at all. The same
    /// holder never conflicts with itself; a weaker lease never replaces a stronger one it holds
    /// (a shell command during the holder's own run leaves the run's lease alone).
    ///
    /// Returns whether a new lease was taken (the caller emits `device.lease.acquired`).
    pub fn acquire(&self, lease: Lease, pruned: &mut Vec<Lease>) -> Result<bool, BusError> {
        let mut map = self.map();
        pruned.extend(expire(&mut map));
        if let Some(other) = map.values().filter(|held| held.overlaps(&lease.device) && held.holder != lease.holder)
            .min_by_key(|held| held.started)
        {
            return Err(busy(other));
        }
        match map.get_mut(&lease.device) {
            Some(held) if held.kind.rank() > lease.kind.rank() => Ok(false),
            Some(held) if held.kind == lease.kind => {
                // A renewal: keep when it started, move what it is doing and when it lapses.
                held.action = lease.action;
                held.expires = match (held.expires, lease.expires) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
                Ok(false)
            }
            _ => {
                map.insert(lease.device.clone(), lease);
                Ok(true)
            }
        }
    }

    /// Would `holder` be refused on `device`? Never takes anything.
    pub fn check(&self, device: &str, holder: &Holder) -> Result<(), BusError> {
        let map = self.map();
        let now = Instant::now();
        match map.values().filter(|held| held.overlaps(device) && &held.holder != holder)
            .filter(|held| held.expires.is_none_or(|at| at > now))
            .min_by_key(|held| held.started)
        {
            Some(other) => Err(busy(other)),
            None => Ok(()),
        }
    }

    pub fn list(&self) -> Vec<Lease> {
        let map = self.map();
        let now = Instant::now();
        let mut out: Vec<Lease> = map.values().filter(|held| held.expires.is_none_or(|at| at > now)).cloned().collect();
        out.sort_by(|a, b| a.device.cmp(&b.device));
        out
    }

    /// The lease that covers `device` (its own, or a `*` one), for `device.list`.
    pub fn holder_of(&self, device: &str) -> Option<Lease> {
        let now = Instant::now();
        let map = self.map();
        map.get(device).or_else(|| map.get(ANY_DEVICE))
            .filter(|held| held.expires.is_none_or(|at| at > now))
            .cloned()
    }

    /// Drop every lease the predicate selects; return what went, for the release events.
    pub fn release_where(&self, mut select: impl FnMut(&Lease) -> bool) -> Vec<Lease> {
        let mut map = self.map();
        let mut gone = expire(&mut map);
        let keys: Vec<String> = map.iter().filter(|(_, lease)| select(lease)).map(|(key, _)| key.clone()).collect();
        for key in keys {
            if let Some(lease) = map.remove(&key) { gone.push(lease); }
        }
        gone
    }

    pub fn release_run(&self, run_id: Id) -> Vec<Lease> {
        self.release_where(|lease| lease.kind == Kind::Run(run_id))
    }

    /// A session that exits or closes gives back its shell leases and claims. A run it asked
    /// for stays with the run: the run, not the session, is what is using the device.
    pub fn release_session(&self, session_id: Id) -> Vec<Lease> {
        self.release_where(|lease| lease.holder.session_id() == Some(session_id) && !matches!(lease.kind, Kind::Run(_)))
    }

    /// The shell command that took a lease has finished: keep it only for the grace period.
    pub fn shell_finished(&self, session_id: Id) {
        let mut map = self.map();
        let until = Instant::now() + SHELL_GRACE;
        for lease in map.values_mut() {
            if lease.kind == Kind::Shell && lease.holder.session_id() == Some(session_id) {
                lease.expires = Some(lease.expires.map_or(until, |at| at.min(until)));
            }
        }
    }

    /// Drop what has lapsed, plus anything `stale` says is held by something that is gone (a
    /// run no longer live, a session no longer running). Returns what went.
    pub fn prune(&self, mut stale: impl FnMut(&Lease) -> bool) -> Vec<Lease> {
        self.release_where(|lease| stale(lease))
    }

    pub fn is_empty(&self) -> bool {
        self.map().is_empty()
    }
}

fn expire(map: &mut HashMap<String, Lease>) -> Vec<Lease> {
    let now = Instant::now();
    let keys: Vec<String> = map.iter().filter(|(_, lease)| lease.expires.is_some_and(|at| at <= now)).map(|(key, _)| key.clone()).collect();
    keys.into_iter().filter_map(|key| map.remove(&key)).collect()
}

fn clip(action: &str) -> String {
    let one_line = action.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= 120 { return one_line; }
    let mut cut: String = one_line.chars().take(119).collect();
    cut.push('…');
    cut
}

/// `2026-10-05T14:02:11.123Z` → `14:02:11 UTC`. Anything else is shown as given.
fn clock(ts: &str) -> String {
    ts.split_once('T')
        .map(|(_, time)| format!("{} UTC", time.get(..8).unwrap_or(time)))
        .unwrap_or_else(|| ts.to_string())
}

fn ago(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// What an agent's shell command does to a device, when it does anything that could clobber
/// another session's work there: installs, uninstalls, launches, input, pushes, reboots. Reads
/// (`logcat`, `devices`, `getprop`, `dumpsys`, `screencap`) are free for everyone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCommand {
    /// The serial it targets, or [`ANY_DEVICE`].
    pub device: String,
    /// A few words for the lease (`adb install app-debug.apk`).
    pub action: String,
}

/// Recognize device-writing commands in a shell line. Conservative by design: it splits on the
/// shell's sequencing operators and looks at each simple command's program and verb; it does not
/// try to be a shell. Whatever it misses runs unleased, as everything did before.
pub fn device_command(line: &str) -> Option<DeviceCommand> {
    let mut exported_serial: Option<String> = None;
    // The same reader the denied-command guardrail uses: `( )`, `$( )`, backticks and
    // `sh -c '…'` each start a command, and a quoted argument stays one word.
    for segment in crate::shell::commands(line) {
        let tokens: Vec<String> = segment.into_iter().map(|word| word.text).collect();
        let mut index = 0;
        let mut serial = exported_serial.clone();
        // Leading assignments and wrappers: `ANDROID_SERIAL=x env timeout 300 ./gradlew …`.
        while index < tokens.len() {
            let token = tokens[index].as_str();
            if let Some(value) = token.strip_prefix("ANDROID_SERIAL=") {
                serial = Some(value.to_string());
                index += 1;
            } else if token == "export" {
                if let Some(value) = tokens.get(index + 1).and_then(|t| t.strip_prefix("ANDROID_SERIAL=")) {
                    exported_serial = Some(value.to_string());
                }
                break;
            } else if is_assignment(token) || matches!(token, "env" | "sudo" | "time" | "nice" | "nohup" | "command" | "exec" | "stdbuf") {
                index += 1;
            } else if token == "timeout" {
                index += 1;
                while tokens.get(index).is_some_and(|t| t.starts_with('-')) { index += 1; }
                index += 1; // the duration
            } else if token == "cd" || token == "pushd" {
                index = tokens.len();
            } else {
                break;
            }
        }
        let Some(program) = tokens.get(index) else { continue };
        let args = &tokens[index + 1..];
        let base = program.rsplit('/').next().unwrap_or(program);
        let found = match base {
            "adb" | "adb.exe" => adb_command(args, serial.as_deref()),
            "gradlew" | "gradlew.bat" | "gradle" => gradle_command(base, args, serial.as_deref()),
            "npx" | "bunx" | "pnpx" | "yarn" | "pnpm" | "npm" | "expo" | "react-native" | "flutter" => {
                js_command(base, args, serial.as_deref())
            }
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

fn is_assignment(token: &str) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Shell verbs (after `adb shell`) that change what is on the device or on its screen.
const SHELL_WRITES: &[&str] = &["am", "pm", "cmd", "input", "monkey", "reboot", "svc", "settings", "wm", "setprop", "rm", "mv", "cp", "uiautomator", "su"];
/// `pm` / `cmd package` verbs that only read.
const PM_READS: &[&str] = &["list", "path", "dump", "resolve-activity", "query-activities", "get-max-users", "has-feature"];
/// `am` verbs that only read.
const AM_READS: &[&str] = &["get-config", "get-current-user", "stack", "task", "monitor", "dumpheap", "profile"];

fn adb_command(args: &[String], serial: Option<&str>) -> Option<DeviceCommand> {
    let mut serial = serial.map(str::to_string);
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-s" | "-t" => {
                if args[index].as_str() == "-s" { serial = args.get(index + 1).cloned(); }
                index += 2;
            }
            "-H" | "-P" | "-L" => index += 2,
            flag if flag.starts_with('-') => index += 1,
            _ => break,
        }
    }
    let verb = args.get(index)?.as_str();
    let rest = &args[index + 1..];
    let writes = match verb {
        "install" | "install-multiple" | "install-multi-package" | "uninstall" | "push" | "sync" | "reboot" | "root"
        | "unroot" | "remount" | "sideload" | "disable-verity" | "enable-verity" | "restore" | "emu" => true,
        "shell" | "exec-out" => {
            let mut words = rest.iter().map(String::as_str).filter(|word| !word.starts_with('-'));
            // `adb shell "am start …"` arrives as one quoted word; look inside it.
            let first = words.next().unwrap_or("");
            let mut inner = first.split_whitespace();
            let command = inner.next().unwrap_or("");
            let sub = inner.next().or_else(|| words.next()).unwrap_or("");
            match command {
                "pm" => !PM_READS.contains(&sub),
                "cmd" => sub == "package" || sub == "activity",
                "am" => !AM_READS.contains(&sub),
                other => SHELL_WRITES.contains(&other),
            }
        }
        _ => false,
    };
    if !writes {
        return None;
    }
    let summary = std::iter::once("adb").chain(std::iter::once(verb))
        .chain(rest.iter().map(String::as_str).take(3).map(|word| word.rsplit('/').next().unwrap_or(word)))
        .collect::<Vec<_>>().join(" ");
    Some(DeviceCommand { device: serial.unwrap_or_else(|| ANY_DEVICE.into()), action: summary })
}

fn gradle_command(program: &str, args: &[String], serial: Option<&str>) -> Option<DeviceCommand> {
    let tasks: Vec<&str> = args.iter().map(String::as_str).filter(|arg| !arg.starts_with('-')).filter(|task| {
        let name = task.rsplit(':').next().unwrap_or(task);
        (name.starts_with("install") && name.len() > "install".len())
            || name.starts_with("uninstall")
            || name.starts_with("connected")
            || name.starts_with("deviceAndroidTest")
    }).collect();
    if tasks.is_empty() {
        return None;
    }
    Some(DeviceCommand {
        device: serial.map(str::to_string).unwrap_or_else(|| ANY_DEVICE.into()),
        action: format!("{program} {}", tasks.join(" ")),
    })
}

fn js_command(program: &str, args: &[String], serial: Option<&str>) -> Option<DeviceCommand> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let android = words.iter().any(|word| matches!(*word, "run:android" | "run-android"))
        || (program == "flutter" && matches!(words.first(), Some(&"run" | &"install")));
    if !android {
        return None;
    }
    let device = words.iter().position(|word| matches!(*word, "--device" | "--deviceId" | "-d"))
        .and_then(|at| words.get(at + 1))
        .map(|value| value.to_string())
        .or_else(|| serial.map(str::to_string))
        .unwrap_or_else(|| ANY_DEVICE.into());
    let shown: Vec<&str> = std::iter::once(program).chain(words.iter().copied().take(3)).collect();
    Some(DeviceCommand { device, action: shown.join(" ") })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(line: &str) -> Option<(String, String)> {
        device_command(line).map(|found| (found.device, found.action))
    }

    #[test]
    fn device_writes_are_recognized_and_reads_are_not() {
        assert_eq!(device("adb install -r app/build/outputs/apk/debug/app-debug.apk").unwrap().0, ANY_DEVICE);
        assert_eq!(device("adb -s R5CT123 install -r app-debug.apk").unwrap(), ("R5CT123".into(), "adb install -r app-debug.apk".into()));
        assert_eq!(device("cd android && ./gradlew :app:installDebug --offline").unwrap(), (ANY_DEVICE.into(), "gradlew :app:installDebug".into()));
        assert_eq!(device("ANDROID_SERIAL=emulator-5554 ./gradlew installRelease").unwrap().0, "emulator-5554");
        assert_eq!(device("export ANDROID_SERIAL=abc; ./gradlew installDebug").unwrap().0, "abc");
        assert!(device("adb shell am start -n com.example/.Main").is_some());
        assert!(device("adb -s X shell 'am force-stop com.example'").is_some());
        assert!(device("adb shell input tap 100 200").is_some());
        assert!(device("adb shell pm clear com.example").is_some());
        assert!(device("timeout 600 npx expo run:android --device R5CT").unwrap().0 == "R5CT");
        assert!(device("adb uninstall com.example").is_some());
        // Reads are free.
        assert!(device("adb devices -l").is_none());
        assert!(device("adb logcat -d | grep FATAL").is_none());
        assert!(device("adb shell pm list packages | grep example").is_none());
        assert!(device("adb shell getprop ro.build.version.sdk").is_none());
        assert!(device("adb exec-out screencap -p > shot.png").is_none());
        assert!(device("./gradlew assembleDebug test lint").is_none());
        assert!(device("./gradlew install").is_none());
        assert!(device("cargo test && git commit -m 'adb install fix'").is_none());
        assert!(device("echo adb install").is_none());
    }

    #[test]
    fn a_lease_refuses_other_holders_and_renews_for_its_own() {
        let leases = Leases::default();
        let a = Holder::Session { id: 1, name: "brisk-otter".into() };
        let b = Holder::Session { id: 2, name: "calm-heron".into() };
        let mut pruned = Vec::new();
        assert!(leases.acquire(Lease::new("R5", a.clone(), Kind::Shell, "adb install app.apk", Some(SHELL_RUNNING)), &mut pruned).unwrap());
        assert!(!leases.acquire(Lease::new("R5", a.clone(), Kind::Shell, "adb shell am start", Some(SHELL_RUNNING)), &mut pruned).unwrap());
        let refused = leases.acquire(Lease::new("R5", b.clone(), Kind::Shell, "gradlew installDebug", Some(SHELL_RUNNING)), &mut pruned).unwrap_err();
        assert_eq!(refused.code, "device.busy");
        assert!(refused.message.contains("brisk-otter"), "{}", refused.message);
        assert!(refused.message.contains("adb shell am start"), "{}", refused.message);
        assert!(refused.hint.as_deref().unwrap().contains("device.lease.released"));
        // `*` conflicts with every device, both ways.
        assert_eq!(leases.acquire(Lease::new(ANY_DEVICE, b.clone(), Kind::Shell, "adb install x", None), &mut pruned).unwrap_err().code, "device.busy");
        assert!(leases.acquire(Lease::new("OTHER", b.clone(), Kind::Shell, "adb install x", None), &mut pruned).unwrap());
        // A run outranks the holder's own shell lease and is not downgraded by it.
        assert!(leases.acquire(Lease::new("R5", a.clone(), Kind::Run(7), "device.run", None), &mut pruned).unwrap());
        assert!(!leases.acquire(Lease::new("R5", a.clone(), Kind::Shell, "adb install", Some(SHELL_RUNNING)), &mut pruned).unwrap());
        assert_eq!(leases.holder_of("R5").unwrap().kind, Kind::Run(7));
        // Closing the session leaves its run's lease; the run ending releases it.
        assert!(leases.release_session(1).is_empty());
        assert_eq!(leases.release_run(7).len(), 1);
        assert!(leases.check("R5", &b).is_ok());
    }

    #[test]
    fn a_finished_shell_command_keeps_its_lease_only_for_the_grace_period() {
        let leases = Leases::default();
        let a = Holder::Session { id: 1, name: "a".into() };
        let mut pruned = Vec::new();
        leases.acquire(Lease::new("R5", a.clone(), Kind::Shell, "adb install", Some(SHELL_RUNNING)), &mut pruned).unwrap();
        leases.shell_finished(1);
        let left = leases.holder_of("R5").unwrap().view().expires_in_s.unwrap();
        assert!(left <= SHELL_GRACE.as_secs(), "{left}");
        // An already-lapsed lease is dropped by the next acquire and reported for its event.
        let lapsed = Lease::new("R6", a.clone(), Kind::Shell, "adb install", Some(Duration::ZERO));
        leases.acquire(lapsed, &mut pruned).unwrap();
        leases.acquire(Lease::new("R7", Holder::User, Kind::Claim, "testing", None), &mut pruned).unwrap();
        assert!(pruned.iter().any(|lease| lease.device == "R6"));
    }
}
