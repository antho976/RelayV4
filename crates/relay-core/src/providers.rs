//! Provider profiles (SPEC §4): discovery/auth plus the only place CLI arguments are shaped.

use crate::engine::IntoBus;
use relay_bus::error::BusError;
use relay_bus::types::{Provider, ProviderInfo, Role, Session};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

pub struct Discovery {
    pub info: ProviderInfo,
    pub previous_version: Option<String>,
    pub version_changed: bool,
}

pub enum Launch<'a> {
    Fresh,
    Resume { provider_ref: Option<&'a str> },
}

pub fn role_instructions_relative(session: &str) -> String {
    format!(".relay/sessions/{session}/role-instructions.md")
}

pub const DISCOVERY_HINT: &str = "Discover operations with `bus.ops` and inspect the exact payload \
and result with `bus.schema {\"op\":\"<op>\"}`; do not guess fields. From a shell, use \
`$RELAY_BIN ops --mine` and `$RELAY_BIN schema <op>`. Relay fills your bound `project_id` and \
`session` when omitted. `session.done` always reports the current bootstrap task, so omit \
`task` and `task_id`.";

/// Every role is told the same three things, because every role needed all three: what
/// `session.bootstrap` answers, that peers are reached through the Relay mailbox rather than
/// the provider's own agent messaging, and where the brief is (D101).
const COMMON_INSTRUCTIONS: &str = "On your first turn, obtain the current assignment and queue before \
acting: call the session.bootstrap Relay MCP tool when available; otherwise run `$RELAY_BIN q \
session.bootstrap`. It also returns your live peers, every op you may call, and your \
guardrails. Coordinate with those peers through Relay `mailbox.send` — it reaches every \
provider including Codex, which your own cross-agent messaging tool does not; send \
`{\"to\":\"*\",\"text\":\"...\"}` to broadcast, and `mailbox.outbox` shows what you sent. Declare the files you are taking on with `session.claim` — it names any \
peer already holding them — and one line of what you are doing with `session.intent`. Before \
starting sizeable work, call `guardrail.explain` with structured paths, lines, and commands; \
`guardrail.check` tests one command or write. `bus.wait` blocks until something happens, so \
never poll. Android devices are shared between sessions: an `adb install`, `adb shell am …` or \
`gradlew install*` takes a lease on the device, and while a peer holds one yours is refused with \
`device.busy` naming who is using it, for what, and since when — `bus.wait` on \
`device.lease.released`, then retry, rather than retrying in a loop. `device.list` shows each \
device's holder; `device.claim` holds a device across several steps and `device.release` frees it. When a guardrail refuses something you cannot progress without, do not work around \
it: call `guardrail.request` with the `kind` and `value` its hint names and a `reason`, then \
`bus.wait` for `guardrail.request_resolved` matching your `request_id` and retry once approved; \
only the user can approve it. Work on `task` first; `tasks` is the remaining ordered queue. Report each current \
task through `session.done` with `status` `completed`, `blocked` or `partial`. All builders must \
finish before the task enters review. A shared-worktree group stays on that task until every \
reviewer finishes; do not begin queued work while peers are still building or reviewing. Relay \
then advances the group FIFO and sends the next assignment through mailbox. After reporting \
completion, wait for that assignment and read `session.bootstrap`; do not repeat `session.done`. \
When you stopped short, include \
the `blockers` that stopped you. Relay responses may carry `mail.priority`; when it is nonzero \
and you are not already handling mail, finish the current atomic action, call `mailbox.list` with \
`unread_only: true`, process the priority messages, and `mailbox.ack` each one. If \
`session.bootstrap` has a current task and you actually encountered Relay workflow friction while \
doing it, immediately before `session.done` append one concise observation with `notes.append` using \
`target: \"suggestions\"`. Name what you were \
doing and what got in the way. Do not record speculative suggestions or write them mid-task; Relay \
adds the session and task identity. Your full brief is at \
$RELAY_BRIEF.";

pub fn role_instructions(role: Role) -> String {
    let specific = match role {
        Role::Builder => "You are this Relay session's builder. If session.bootstrap has no assignment, query task.list for this project before suggesting board work; if no available tasks exist, say so instead of sending the user to an empty board. Implement only work the user assigned or authorized you to take in the current worktree, verify it, and report completion through session.done. When paired with a reviewer, send it a Relay mailbox message as soon as a file is ready for review; keep working while it reviews.",
        Role::Reviewer => "You are this Relay session's reviewer. Review the paired work against the task requirements. Review file-ready messages immediately. Do not modify files or commit; send the builder exact fixes through Relay mailbox tools, then verify its correction.",
        Role::Docs => "You are this Relay session's documentation agent. Change only documentation required by that assignment, verify relevant documentation checks, and report completion through session.done.",
    };
    format!("{specific}\n\n{COMMON_INSTRUCTIONS}\n\n{DISCOVERY_HINT}")
}

/// Codex takes no `--mcp-config`, so Relay's bus instructions travel through this provider-native
/// setting. Guardrail and lifecycle enforcement bind separately through project hooks (D132).
fn codex_developer_instructions(role: Role, brief: Option<&str>) -> String {
    let mut text = role_instructions(role);
    text.push_str(
        "\n\nRelay tools are not registered with this provider: reach the bus by running \
`$RELAY_BIN q <op> '<json>'` in a shell. `$RELAY_BIN q session.bootstrap` lists every op your \
role may call.",
    );
    if let Some(brief) = brief.map(str::trim).filter(|brief| !brief.is_empty()) {
        text.push_str("\n\n");
        text.push_str(brief);
    }
    let value = serde_json::to_string(&text).expect("Relay role instructions serialize");
    format!("developer_instructions={value}")
}

pub fn codex_notify_config(relay: &Path) -> String {
    let command = [relay.display().to_string(), "hook".into(), "codex-notify".into()];
    format!("notify={}", serde_json::to_string(&command).expect("Codex notify command serializes"))
}

/// `--config` overrides that register one plugin MCP server with Codex, which takes no MCP
/// config file (D159). Values are JSON strings and arrays, which TOML reads identically; a
/// name that is not a bare TOML key is refused rather than quoted into a different key.
pub fn codex_mcp_config(
    name: &str,
    command: &str,
    args: &[String],
    env: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let bare = |key: &str| !key.is_empty() && key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    if !bare(name) || !env.keys().all(|key| bare(key)) {
        return Vec::new();
    }
    let quote = |text: &str| serde_json::to_string(text).expect("a string serializes");
    let mut out = vec![
        "--config".into(),
        format!("mcp_servers.{name}.command={}", quote(command)),
        "--config".into(),
        format!("mcp_servers.{name}.args={}", serde_json::to_string(args).expect("strings serialize")),
    ];
    if !env.is_empty() {
        let pairs = env.iter().map(|(key, value)| format!("{key}={}", quote(value))).collect::<Vec<_>>().join(",");
        out.extend(["--config".into(), format!("mcp_servers.{name}.env={{{pairs}}}")]);
    }
    out
}

pub trait Driver: Sync {
    fn provider(&self) -> Provider;
    fn binary(&self) -> &'static str;
    /// `brief` is the compact session brief. Providers that read a prompt file ignore it;
    /// providers with no such channel inline it.
    fn args(&self, session: &Session, launch: Launch<'_>, brief: Option<&str>) -> Vec<String>;
    fn profile(&self) -> Value;
    /// Whether Relay's `PreToolUse` guardrail and lifecycle reports actually bind here.
    fn guarded(&self) -> bool;
    fn auth(&self, path: &Path) -> Option<String>;
}

struct Claude;
struct Codex;

static CLAUDE: Claude = Claude;
static CODEX: Codex = Codex;

pub fn driver(provider: Provider) -> &'static dyn Driver {
    match provider {
        Provider::Claude => &CLAUDE,
        Provider::Codex => &CODEX,
    }
}

fn model_effort_args(session: &Session, claude: bool) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(model) = &session.model {
        args.extend(["--model".to_string(), model.clone()]);
    }
    if let Some(effort) = &session.effort {
        if claude {
            args.extend(["--effort".to_string(), effort.clone()]);
        } else {
            args.extend(["--config".to_string(), format!("model_reasoning_effort=\"{effort}\"")]);
        }
    }
    args
}

impl Driver for Claude {
    fn provider(&self) -> Provider { Provider::Claude }
    fn binary(&self) -> &'static str { "claude" }
    fn guarded(&self) -> bool { true }
    fn args(&self, session: &Session, launch: Launch<'_>, _brief: Option<&str>) -> Vec<String> {
        let mut args = vec![
            "--mcp-config".into(), crate::hooks::MCP_CONFIG_RELATIVE.into(),
            "--settings".into(), crate::hooks::CLAUDE_SETTINGS_RELATIVE.into(),
            "--append-system-prompt-file".into(), role_instructions_relative(&session.name),
        ];
        match launch {
            Launch::Fresh => {
                args.extend(["--name".into(), session.name.clone()]);
                args.extend(model_effort_args(session, true));
            }
            Launch::Resume { provider_ref } => {
                match provider_ref {
                    Some(reference) => args.extend(["--resume".into(), reference.to_string()]),
                    None => args.push("--continue".into()),
                }
                args.extend(model_effort_args(session, true));
            }
        }
        args
    }
    fn profile(&self) -> Value {
        json!({
            "binary": "claude", "fresh": ["--mcp-config", ".relay/relay.mcp.json", "--settings", ".relay/relay.settings.json", "--append-system-prompt-file", ".relay/sessions/<session>/role-instructions.md", "--name", "<session>", "[--model]", "[--effort]"],
            "resume": ["--mcp-config", ".relay/relay.mcp.json", "--settings", ".relay/relay.settings.json", "--append-system-prompt-file", ".relay/sessions/<session>/role-instructions.md", "--resume", "<provider_ref>"], "fallback_resume": ["--mcp-config", ".relay/relay.mcp.json", "--settings", ".relay/relay.settings.json", "--append-system-prompt-file", ".relay/sessions/<session>/role-instructions.md", "--continue"],
            "auth": ["auth", "status"], "lifecycle_hooks": true, "guarded": true,
        })
    }
    fn auth(&self, path: &Path) -> Option<String> {
        let output = command_output(path, &["auth", "status"])?;
        if !output.success { return None; }
        let value: Value = serde_json::from_str(&output.stdout).ok()?;
        if value["loggedIn"].as_bool() != Some(true) { return None; }
        value["email"].as_str().or_else(|| value["orgName"].as_str()).map(str::to_string)
    }
}

impl Driver for Codex {
    fn provider(&self) -> Provider { Provider::Codex }
    fn binary(&self) -> &'static str { "codex" }
    fn guarded(&self) -> bool { true }
    fn args(&self, session: &Session, launch: Launch<'_>, brief: Option<&str>) -> Vec<String> {
        let mut args = Vec::new();
        match launch {
            Launch::Fresh => {
                args.push("--no-alt-screen".into());
                args.push("--approve-for-me".into());
                args.extend(model_effort_args(session, false));
                args.extend(["--config".into(), codex_developer_instructions(session.role, brief)]);
            }
            Launch::Resume { provider_ref } => {
                args.push("resume".into());
                args.push("--no-alt-screen".into());
                args.push("--approve-for-me".into());
                args.extend(model_effort_args(session, false));
                args.extend(["--config".into(), codex_developer_instructions(session.role, brief)]);
                match provider_ref {
                    Some(reference) => args.push(reference.to_string()),
                    None => args.push("--last".into()),
                }
            }
        }
        args
    }
    fn profile(&self) -> Value {
        json!({
            "binary": "codex", "fresh": ["--no-alt-screen", "--approve-for-me", "[--model]", "[--config model_reasoning_effort]", "--config", "<role developer_instructions>"],
            "resume": ["resume", "--no-alt-screen", "--approve-for-me", "--config", "<role developer_instructions>", "<provider_ref>"],
            "fallback_resume": ["resume", "--no-alt-screen", "--approve-for-me", "--config", "<role developer_instructions>", "--last"], "auth": ["login", "status"],
            "lifecycle_hooks": true, "guarded": true,
        })
    }
    fn auth(&self, path: &Path) -> Option<String> {
        let output = command_output(path, &["login", "status"])?;
        if !output.success { return None; }
        let line = output.stdout.lines().find(|line| !line.trim().is_empty())?.trim();
        line.strip_prefix("Logged in using ").unwrap_or(line).trim().to_string().into()
    }
}

pub fn validate_options(provider: Provider, model: Option<&str>, effort: Option<&str>) -> Result<(), BusError> {
    if model.is_some_and(|model| model.trim().is_empty()) {
        return Err(BusError::invalid("session.model", "model cannot be empty"));
    }
    let allowed: &[&str] = match provider {
        Provider::Claude => &["low", "medium", "high", "xhigh", "max"],
        Provider::Codex => &["minimal", "low", "medium", "high", "xhigh"],
    };
    if let Some(effort) = effort {
        if !allowed.contains(&effort) {
            return Err(BusError::invalid(
                "session.effort",
                format!("{} effort {effort:?} must be one of {}", driver(provider).binary(), allowed.join(", ")),
            ));
        }
    }
    Ok(())
}

fn configured_path(conn: &Connection, provider: Provider) -> Option<PathBuf> {
    let key = format!("providers.{}.path", driver(provider).binary());
    let configured: Option<String> = conn.query_row("SELECT value FROM settings WHERE path=?1", [key], |row| row.get(0)).optional().ok().flatten();
    configured.and_then(|raw| serde_json::from_str::<Option<String>>(&raw).ok().flatten()).map(PathBuf::from)
}

pub fn executable(conn: &Connection, provider: Provider) -> Result<PathBuf, BusError> {
    let drv = driver(provider);
    if let Some(path) = configured_path(conn, provider) {
        if path.is_file() { return Ok(path); }
        return Err(BusError::unavailable(
            "provider.not_installed",
            format!("providers.{}.path = {:?} does not exist", drv.binary(), path.display().to_string()),
        ));
    }
    which::which(drv.binary()).map_err(|_| {
        BusError::unavailable("provider.not_installed", format!("{} is not on PATH", drv.binary()))
            .with_hint(format!("install it, or set settings providers.{}.path", drv.binary()))
    })
}

type CacheRow = (Option<String>, Option<String>, Option<String>, Option<String>, Value);

fn cache_row(row: &Row) -> rusqlite::Result<CacheRow> {
    let profile: String = row.get("spawn_profile")?;
    Ok((row.get("path")?, row.get("version")?, row.get("signed_in_as")?, row.get("last_seen_version")?, serde_json::from_str(&profile).unwrap_or(Value::Null)))
}

pub fn list(conn: &Connection) -> Result<Vec<ProviderInfo>, BusError> {
    [Provider::Claude, Provider::Codex].into_iter().map(|provider| {
        let drv = driver(provider);
        let resolved = executable(conn, provider).ok();
        let cached = conn.query_row("SELECT * FROM provider_cache WHERE provider=?1", [drv.binary()], cache_row).optional().bus()?;
        let (path, version, signed_in_as, last_seen_version, profile) = cached.unwrap_or((None, None, None, None, drv.profile()));
        let path = resolved.as_ref().map(|p| p.display().to_string()).or(path);
        Ok(ProviderInfo { provider, installed: resolved.is_some(), path, version, signed_in_as, last_seen_version, spawn_profile: profile, guarded: drv.guarded() })
    }).collect()
}

pub fn refresh(tx: &Transaction, now: &str) -> Result<Vec<Discovery>, BusError> {
    let inputs = [Provider::Claude, Provider::Codex].into_iter().map(|provider| {
        let drv = driver(provider);
        let old: Option<(Option<String>, Option<String>)> = tx.query_row(
            "SELECT version, last_seen_version FROM provider_cache WHERE provider=?1",
            [drv.binary()], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().bus()?;
        let path = executable(tx, provider).ok();
        Ok((provider, path, old))
    }).collect::<Result<Vec<_>, BusError>>()?;
    let probed = thread::scope(|scope| {
        inputs.into_iter().map(|(provider, path, old)| scope.spawn(move || {
            let drv = driver(provider);
            let (version, signed_in_as) = thread::scope(|scope| {
                let version = scope.spawn(|| path.as_deref().and_then(|path| command_output(path, &["--version"]))
                    .filter(|output| output.success).map(|output| first_line(&output.stdout)));
                let auth = scope.spawn(|| path.as_deref().and_then(|path| drv.auth(path)));
                (version.join().expect("provider version probe panicked"), auth.join().expect("provider auth probe panicked"))
            });
            (provider, path, old, version, signed_in_as)
        })).collect::<Vec<_>>().into_iter().map(|handle| handle.join().expect("provider probe panicked")).collect::<Vec<_>>()
    });
    let mut out = Vec::new();
    for (provider, path, old, version, signed_in_as) in probed {
        let drv = driver(provider);
        let previous_version = old.as_ref().and_then(|old| old.0.clone());
        let version_changed = previous_version.is_some() && previous_version != version;
        let last_seen_version = if version_changed { previous_version.clone() } else { old.and_then(|old| old.1) };
        let profile = drv.profile();
        tx.execute(
            "INSERT INTO provider_cache(provider,path,version,signed_in_as,last_seen_version,spawn_profile,detected_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(provider) DO UPDATE SET path=excluded.path,version=excluded.version,
               signed_in_as=excluded.signed_in_as,last_seen_version=excluded.last_seen_version,
               spawn_profile=excluded.spawn_profile,detected_at=excluded.detected_at",
            params![drv.binary(), path.as_ref().map(|p| p.display().to_string()), version, signed_in_as, last_seen_version, serde_json::to_string(&profile).bus()?, now],
        ).bus()?;
        out.push(Discovery {
            info: ProviderInfo { provider, installed: path.is_some(), path: path.map(|p| p.display().to_string()), version, signed_in_as, last_seen_version, spawn_profile: profile, guarded: drv.guarded() },
            previous_version,
            version_changed,
        });
    }
    Ok(out)
}

struct Output { success: bool, stdout: String }

/// Provider probes (`<binary> --version`) run inside the request transaction, so the deadline is
/// what keeps a wedged binary from holding the store mutex indefinitely (D144).
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn command_output(path: &Path, args: &[&str]) -> Option<Output> {
    let mut command = Command::new(path);
    command.args(args);
    // A killed probe reports "not installed" rather than failing detection outright, which is the
    // same answer the old busy-wait produced on timeout.
    let Some(output) = crate::proc::output_with_timeout(&mut command, PROBE_TIMEOUT).ok()? else {
        return Some(Output { success: false, stdout: String::new() });
    };
    let stdout = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    Some(Output { success: output.status.success(), stdout })
}

fn first_line(raw: &str) -> String {
    raw.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().trim().chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use relay_bus::types::{Role, SessionState};

    fn session(provider: Provider) -> Session {
        Session { id: 1, name: "calm-otter".into(), intent: None, project_id: 1, provider, role: Role::Builder,
            model: Some("model-x".into()), effort: Some("high".into()), branch: "relay/calm-otter".into(), worktree: "/tmp/w".into(),
            task_id: None, module_id: None, pair_with: None, bus_writes: false, allow_ui: false, state: SessionState::Created,
            pid: None, exit_code: None, provider_ref: None, spawned_at: None, last_output_at: None, usage: None,
            created_at: "now".into(), updated_at: "now".into(), closed_at: None }
    }

    #[test]
    fn profiles_shape_fresh_and_resume_commands() {
        assert!(role_instructions(Role::Builder).contains("query task.list"));
        assert_eq!(codex_notify_config(Path::new("/opt/Relay 2/relay")), "notify=[\"/opt/Relay 2/relay\",\"hook\",\"codex-notify\"]");
        let claude = session(Provider::Claude);
        assert_eq!(driver(Provider::Claude).args(&claude, Launch::Fresh, None),
            ["--mcp-config", ".relay/relay.mcp.json", "--settings", ".relay/relay.settings.json", "--append-system-prompt-file", ".relay/sessions/calm-otter/role-instructions.md", "--name", "calm-otter", "--model", "model-x", "--effort", "high"]);
        assert_eq!(driver(Provider::Claude).args(&claude, Launch::Resume { provider_ref: Some("cc-id") }, None),
            ["--mcp-config", ".relay/relay.mcp.json", "--settings", ".relay/relay.settings.json", "--append-system-prompt-file", ".relay/sessions/calm-otter/role-instructions.md", "--resume", "cc-id", "--model", "model-x", "--effort", "high"]);
        let mut codex = session(Provider::Codex);
        codex.role = Role::Reviewer;
        let args = driver(Provider::Codex).args(&codex, Launch::Resume { provider_ref: Some("cx-id") }, None);
        assert_eq!(&args[..7], ["resume", "--no-alt-screen", "--approve-for-me", "--model", "model-x", "--config", "model_reasoning_effort=\"high\""]);
        assert_eq!(args[7], "--config");
        assert!(args[8].starts_with("developer_instructions=\"You are this Relay session's reviewer."));
        assert_eq!(args[9], "cx-id");
    }

    #[test]
    fn every_role_is_told_about_the_brief_and_the_mailbox() {
        for role in [Role::Builder, Role::Reviewer, Role::Docs] {
            let text = role_instructions(role);
            assert!(text.contains("$RELAY_BRIEF"), "{role:?} is never told where its brief is");
            assert!(text.contains("mailbox.send"), "{role:?} is never told the mailbox exists");
            assert!(text.contains("{\"to\":\"*\",\"text\":\"...\"}"), "{role:?} is never told the broadcast alias");
            assert!(text.contains("guardrail.check"), "{role:?} cannot find the dry run");
            assert!(text.contains("paths, lines, and commands"), "{role:?} is invited to send prose to guardrail.explain");
            assert!(text.contains("session.claim"), "{role:?} is never told it can claim files");
            assert!(text.contains("bus.schema"), "{role:?} is left to guess payload fields");
            assert!(text.contains("$RELAY_BIN schema <op>"), "{role:?} cannot inspect schemas from its shell");
            assert!(text.contains("omit `task` and `task_id`"), "{role:?} may guess a task field for session.done");
            assert!(text.contains("bus.wait"), "{role:?} is left to poll");
            assert!(text.contains("blockers"), "{role:?} can only report success");
            assert!(text.contains("mail.priority"), "{role:?} will miss priority mail");
            assert!(text.contains("has a current task"), "{role:?} will try to file an unattributed suggestion");
            assert!(text.contains("target: \"suggestions\""), "{role:?} cannot record observed friction");
        }
        // The common half is one string; keep the roles from drifting apart again.
        assert!(COMMON_INSTRUCTIONS.contains("mailbox.send"));
        for role in [Role::Builder, Role::Reviewer, Role::Docs] {
            assert!(role_instructions(role).contains(COMMON_INSTRUCTIONS));
        }
    }

    #[test]
    fn codex_carries_the_brief_and_the_shell_path_it_actually_has() {
        let codex = session(Provider::Codex);
        let args = driver(Provider::Codex).args(&codex, Launch::Fresh, Some("## Live peers\n- sly-egret"));
        assert_eq!(&args[..2], ["--no-alt-screen", "--approve-for-me"]);
        let config = args.iter().find(|arg| arg.starts_with("developer_instructions=")).expect("developer_instructions");
        assert!(config.contains("sly-egret"), "codex never receives its peers");
        assert!(config.contains("$RELAY_BIN q"), "codex is not told how to reach the bus");
        assert!(driver(Provider::Codex).guarded());
        assert!(driver(Provider::Claude).guarded());
    }
}
