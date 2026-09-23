//! Provider and VCS enforcement adapters (BUS.md §9.3). These files contain no policy:
//! they translate the host hook into a `guardrail.gate` call on the same bus as every other
//! client.

use crate::Instance;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CLAUDE_HOOK_MARKER: &str = "hook claude-";
const CLAUDE_PRE_TOOL: &str = "hook claude-pre-tool";
const CODEX_HOOK_MARKER: &str = "hook codex-";
const CODEX_PRE_TOOL: &str = "hook codex-pre-tool";
const PREVIOUS_HOOKS_PATH: &str = ".relay-previous-hooks-path";
/// First line after the shebang of every forwarder [`install_git`] writes, so a reinstall can
/// tell its own files from a hook someone else (`git lfs install`) put into the directory.
const FORWARDER_MARKER: &str = "# relay: forwards to the hook this directory replaced";
/// Every hook git looks up in `core.hooksPath`, except `pre-commit`, which carries the gate.
/// Pointing `core.hooksPath` at Relay's directory hides the previous directory from git, so
/// each of these gets a forwarder when the previous directory has it — Git LFS's `pre-push`,
/// `post-checkout`, `post-commit` and `post-merge` first among them. `fsmonitor-watchman` is
/// absent on purpose: git reaches it through `core.fsmonitor`, never through the hooks dir.
const FORWARDED_HOOKS: &[&str] = &[
    "applypatch-msg", "pre-applypatch", "post-applypatch", "pre-merge-commit",
    "prepare-commit-msg", "commit-msg", "post-commit", "pre-rebase", "post-checkout",
    "post-merge", "pre-push", "pre-receive", "update", "proc-receive", "post-receive",
    "post-update", "reference-transaction", "push-to-checkout", "pre-auto-gc", "post-rewrite",
    "sendemail-validate", "post-index-change", "p4-changelist", "p4-prepare-changelist",
    "p4-post-changelist", "p4-pre-submit",
];
/// A git that waits on a credential helper or an `index.lock` must not hold the bus forever
/// (these run under the store lock during spawn, resume and close).
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
pub const MCP_CONFIG_RELATIVE: &str = ".relay/relay.mcp.json";
pub const CLAUDE_SETTINGS_RELATIVE: &str = ".relay/relay.settings.json";
const CLAUDE_STATUSLINE_RELATIVE: &str = ".relay/agent-statusline.sh";

/// Prefer an explicit CLI, then the CLI shipped/built beside the current executable, then PATH.
/// Desktop launchers commonly have a reduced PATH, so hooks must persist an absolute sibling path
/// instead of assuming a terminal-installed `relay` is visible.
pub fn relay_bin() -> PathBuf {
    relay_bin_from(
        std::env::var_os("RELAY_BIN").map(PathBuf::from),
        std::env::current_exe().ok(),
    )
}

fn relay_bin_from(explicit: Option<PathBuf>, current_exe: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit.as_deref() {
        if let Ok(resolved) = which::which(path) {
            return resolved;
        }
    }
    if let Some(sibling) = current_exe
        .as_deref()
        .and_then(Path::parent)
        .map(|parent| parent.join("relay"))
        .filter(|path| is_executable(path))
    {
        return sibling;
    }
    which::which("relay")
        .ok()
        .or(explicit)
        .unwrap_or_else(|| PathBuf::from("relay"))
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// Give one worktree its own hook path, preserving any pre-existing pre-commit hook by
/// chaining it before Relay's gate, and every other hook the previous directory has through a
/// forwarder that `exec`s it with the same arguments and stdin.
pub fn install_git(
    repo: &Path,
    worktree: &Path,
    session: &str,
    instance: Instance,
    relay: &Path,
) -> Result<()> {
    // The hook directory and the provider files Relay writes live inside the checkout; the
    // primary checkout never goes through `worktree::create`, which used to be the only place
    // that kept them out of `git status` (and out of a `git add -A`).
    if let Err(error) = crate::worktree::ensure_excluded(repo) {
        tracing::warn!(repo = %repo.display(), %error, "excluding Relay's files from git");
    }
    git(repo, &["config", "extensions.worktreeConfig", "true"])?;

    let configured = git_optional(
        worktree,
        &["config", "--worktree", "--get", "core.hooksPath"],
    );
    let relay_hooks = repo.join(".relay").join("hooks");
    let previous_dir = configured
        .map(PathBuf::from)
        .and_then(|configured| {
            let absolute = if configured.is_absolute() {
                configured.clone()
            } else {
                worktree.join(&configured)
            };
            if absolute.starts_with(&relay_hooks) {
                fs::read_to_string(absolute.join(PREVIOUS_HOOKS_PATH))
                    .ok()
                    .map(|path| PathBuf::from(path.trim()))
            } else {
                Some(configured)
            }
        })
        .or_else(|| git_optional(worktree, &["rev-parse", "--git-path", "hooks"]).map(PathBuf::from));

    let hook_dir = relay_hooks.join(session);
    fs::create_dir_all(&hook_dir)
        .with_context(|| format!("creating {}", hook_dir.display()))?;
    let hook_path = hook_dir.join("pre-commit");
    let previous_hook = previous_dir
        .as_ref()
        .cloned()
        .map(|dir| if dir.is_absolute() { dir } else { worktree.join(dir) })
        .map(|dir| dir.join("pre-commit"))
        .filter(|path| path != &hook_path);

    let user_payload = serde_json::to_string(&json!({"session": session, "kind": "commit"}))?;
    let previous = previous_hook
        .as_deref()
        .map(shell_quote_path)
        .unwrap_or_else(|| "''".to_string());
    let script = format!(
        "#!/bin/sh\n\
         set -u\n\
         previous={previous}\n\
         if [ -n \"$previous\" ] && [ -x \"$previous\" ]; then\n\
           \"$previous\" \"$@\" || exit $?\n\
         fi\n\
         relay_bin={}\n\
         if [ -n \"${{RELAY_SESSION:-}}\" ]; then\n\
           if [ -z \"${{RELAY_TOKEN:-}}\" ]; then\n\
             echo 'RELAY guardrail: session identity has no token' >&2\n\
             exit 2\n\
           fi\n\
           payload=$(printf '{{\"session\":\"%s\",\"kind\":\"commit\"}}' \"$RELAY_SESSION\")\n\
           exec \"$relay_bin\" --instance {} cmd guardrail.gate \"$payload\"\n\
         fi\n\
         exec \"$relay_bin\" --instance {} --actor user cmd guardrail.gate {}\n",
        shell_quote_path(relay),
        shell_quote(instance.as_str()),
        shell_quote(instance.as_str()),
        shell_quote(&user_payload),
    );
    fs::write(&hook_path, script)
        .with_context(|| format!("writing {}", hook_path.display()))?;
    if let Some(previous_dir) = &previous_dir {
        fs::write(
            hook_dir.join(PREVIOUS_HOOKS_PATH),
            format!("{}\n", previous_dir.display()),
        )?;
    }
    let mut permissions = fs::metadata(&hook_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook_path, permissions)?;
    let previous_abs = previous_dir
        .map(|dir| if dir.is_absolute() { dir } else { worktree.join(dir) })
        .filter(|dir| dir != &hook_dir && !dir.starts_with(&relay_hooks));
    write_forwarders(&hook_dir, previous_abs.as_deref())?;

    let hook_dir_s = hook_dir.display().to_string();
    git(
        worktree,
        &["config", "--worktree", "core.hooksPath", &hook_dir_s],
    )?;
    Ok(())
}

/// One forwarder per hook the previous directory has now; a forwarder whose target has gone
/// is removed. Hooks that are not Relay forwarders (a tool may install straight into the
/// active `core.hooksPath`) are left alone.
fn write_forwarders(hook_dir: &Path, previous: Option<&Path>) -> Result<()> {
    for name in FORWARDED_HOOKS {
        let path = hook_dir.join(name);
        let target = previous.map(|dir| dir.join(name)).filter(|target| is_executable(target));
        let ours = fs::read_to_string(&path).is_ok_and(|text| text.contains(FORWARDER_MARKER));
        let Some(target) = target else {
            if ours { let _ = fs::remove_file(&path); }
            continue;
        };
        if path.exists() && !ours {
            continue;
        }
        // Checked again at run time: a hook deleted after install is a no-op, not a failure.
        let script = format!(
            "#!/bin/sh\n{FORWARDER_MARKER}\nprevious={}\nif [ -x \"$previous\" ]; then exec \"$previous\" \"$@\"; fi\nexit 0\n",
            shell_quote_path(&target),
        );
        fs::write(&path, script).with_context(|| format!("writing {}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// The session whose generated directory is this worktree's active `core.hooksPath`, if any.
pub fn active_hook_session(repo: &Path, worktree: &Path) -> Option<String> {
    let configured = PathBuf::from(git_optional(
        worktree,
        &["config", "--worktree", "--get", "core.hooksPath"],
    )?);
    let absolute = if configured.is_absolute() { configured } else { worktree.join(configured) };
    let relative = absolute.strip_prefix(repo.join(".relay").join("hooks")).ok()?;
    let mut components = relative.components();
    let Some(std::path::Component::Normal(session)) = components.next() else { return None };
    if components.next().is_some() {
        return None;
    }
    session.to_str().map(str::to_owned)
}

/// Rewrite the currently active Relay-owned hook with the current CLI path. Existing worktrees can
/// outlive the Relay process that created them, so a commit is also a repair point for stale hooks.
pub fn refresh_git(repo: &Path, worktree: &Path, instance: Instance, relay: &Path) -> Result<bool> {
    let Some(session) = active_hook_session(repo, worktree) else {
        return Ok(false);
    };
    install_git(repo, worktree, &session, instance, relay)?;
    Ok(true)
}

/// Run the user-owned pre-commit hook without re-entering Relay's generated guardrail hook.
/// `git.commit` already executes the guardrail in-process while its transaction is open; letting
/// the generated hook call the socket door here would wait on that same transaction forever.
pub fn run_user_pre_commit(repo: &Path, worktree: &Path) -> Result<()> {
    let configured = git_optional(
        worktree,
        &["config", "--worktree", "--get", "core.hooksPath"],
    );
    let relay_hooks = repo.join(".relay").join("hooks");
    let active_dir = configured
        .map(PathBuf::from)
        .or_else(|| git_optional(worktree, &["rev-parse", "--git-path", "hooks"]).map(PathBuf::from));
    let Some(active_dir) = active_dir else { return Ok(()) };
    let active_dir = if active_dir.is_absolute() { active_dir } else { worktree.join(active_dir) };
    let user_dir = if active_dir.starts_with(&relay_hooks) {
        let Ok(previous) = fs::read_to_string(active_dir.join(PREVIOUS_HOOKS_PATH)) else { return Ok(()) };
        let previous = PathBuf::from(previous.trim());
        if previous.is_absolute() { previous } else { worktree.join(previous) }
    } else {
        active_dir
    };
    let hook = user_dir.join("pre-commit");
    if !is_executable(&hook) { return Ok(()) }
    let output = Command::new(&hook)
        .current_dir(worktree)
        .output()
        .with_context(|| format!("running {}", hook.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
        return Err(anyhow!("{} failed with {}: {}", hook.display(), output.status, detail));
    }
    Ok(())
}

/// Restore the hook path that was active before Relay owned this worktree. If another Relay
/// session has since taken ownership, leave it alone.
pub fn uninstall_git(repo: &Path, worktree: &Path, session: &str) -> Result<()> {
    if !worktree.exists() {
        return Ok(());
    }
    let hook_dir = repo.join(".relay").join("hooks").join(session);
    let Some(current) = git_optional(
        worktree,
        &["config", "--worktree", "--get", "core.hooksPath"],
    ) else {
        return Ok(());
    };
    let current = PathBuf::from(current);
    let current_absolute = if current.is_absolute() { current } else { worktree.join(current) };
    if current_absolute != hook_dir {
        return Ok(());
    }
    match fs::read_to_string(hook_dir.join(PREVIOUS_HOOKS_PATH)) {
        Ok(previous) if !previous.trim().is_empty() => {
            git(
                worktree,
                &["config", "--worktree", "core.hooksPath", previous.trim()],
            )?;
        }
        _ => {
            let output = git_output(worktree, &["config", "--worktree", "--unset", "core.hooksPath"])?;
            if !output.status.success() && output.status.code() != Some(5) {
                return Err(anyhow!(
                    "git config --worktree --unset core.hooksPath failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
    }
    Ok(())
}

/// Drop one session's generated hook directory. Best-effort. The caller must first make sure no
/// worktree still points `core.hooksPath` at it — git reads a missing hooks directory as "no
/// hooks" and silently skips the gate and every user hook ([`uninstall_git`] when the session
/// was the last one in its worktree, [`install_git`] for a surviving session otherwise).
pub fn remove_hook_dir(repo: &Path, session: &str) {
    let _ = fs::remove_dir_all(repo.join(".relay").join("hooks").join(session));
}

/// Remove generated hook directories with no session behind them. Called from the launch
/// fsck, where the live set is already known — never on a timer (SPEC §15).
pub fn sweep_hook_dirs(repo: &Path, live: &[String]) -> usize {
    let dir = repo.join(".relay").join("hooks");
    let Ok(entries) = fs::read_dir(&dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if live.iter().any(|session| session == &name) {
            continue;
        }
        if fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Merge Relay's enforcement and lifecycle adapters into Claude's local, untracked settings.
pub fn install_claude(worktree: &Path, instance: Instance, relay: &Path) -> Result<()> {
    let dir = worktree.join(".claude");
    let path = dir.join("settings.local.json");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut root: Value = match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} must contain a JSON object", path.display()))?;
    let hooks = object.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow!("{}.hooks must be an object", path.display()))?;
    let pre = hooks.entry("PreToolUse").or_insert_with(|| json!([]));
    let pre = pre
        .as_array_mut()
        .ok_or_else(|| anyhow!("{}.hooks.PreToolUse must be an array", path.display()))?;

    let already_installed = pre.iter().any(|group| {
        group["hooks"].as_array().is_some_and(|handlers| {
            handlers.iter().any(|handler| {
                handler["command"]
                    .as_str()
                    .is_some_and(|command| command.contains(CLAUDE_PRE_TOOL))
            })
        })
    });
    if !already_installed {
        let command = format!(
            "{} --instance {} {}",
            shell_quote_path(relay),
            shell_quote(instance.as_str()),
            CLAUDE_PRE_TOOL,
        );
        pre.push(json!({
            "matcher": "Edit|Write|MultiEdit|Bash",
            "hooks": [{"type": "command", "command": command, "timeout": 30}]
        }));
    }

    for (event, kind) in [
        ("SessionStart", "session_start"),
        ("PostToolUse", "tool_use"),
        ("Stop", "stop"),
        ("Notification", "notification"),
    ] {
        let groups = hooks.entry(event).or_insert_with(|| json!([]));
        let groups = groups.as_array_mut()
            .ok_or_else(|| anyhow!("{}.hooks.{event} must be an array", path.display()))?;
        let installed = groups.iter().any(|group| group["hooks"].as_array().is_some_and(|handlers| {
            handlers.iter().any(|handler| handler["command"].as_str().is_some_and(|command| command.contains("hook claude-report")))
        }));
        if !installed {
            let command = format!(
                "{} --instance {} hook claude-report {}",
                shell_quote_path(relay), shell_quote(instance.as_str()), shell_quote(kind),
            );
            groups.push(json!({
                "hooks": [{"type": "command", "command": command, "timeout": 10}]
            }));
        }
    }

    let tmp = dir.join("settings.local.json.relay-tmp");
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(&root)?))
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("replacing {}", path.display()))?;

    // Claude already receives account rate limits on each normal turn. A silent status-line
    // command captures that payload for `usage.get`; it makes no provider or network request.
    let statusline_path = worktree.join(CLAUDE_STATUSLINE_RELATIVE);
    if let Some(parent) = statusline_path.parent() { fs::create_dir_all(parent)?; }
    let statusline_tmp = statusline_path.with_extension("sh.relay-tmp");
    fs::write(&statusline_tmp, claude_statusline_script())?;
    let mut permissions = fs::metadata(&statusline_tmp)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&statusline_tmp, permissions)?;
    fs::rename(&statusline_tmp, &statusline_path)?;
    let provider_settings = worktree.join(CLAUDE_SETTINGS_RELATIVE);
    let settings_tmp = provider_settings.with_extension("json.relay-tmp");
    fs::write(&settings_tmp, format!("{}\n", serde_json::to_string_pretty(&json!({
        "statusLine": {"type":"command", "command":statusline_path, "padding":0}
    }))?))?;
    fs::rename(&settings_tmp, &provider_settings)?;

    // Explicit per-launch MCP config: it inherits the session/token/project environment from
    // the Claude process, so no credentials are ever written to disk.
    let mcp_path = worktree.join(MCP_CONFIG_RELATIVE);
    if let Some(parent) = mcp_path.parent() { fs::create_dir_all(parent)?; }
    let config = json!({"mcpServers":{"relay":{
        "command":relay.display().to_string(),
        "args":["--instance",instance.as_str(),"mcp"]
    }}});
    let mcp_tmp = mcp_path.with_extension("json.relay-tmp");
    fs::write(&mcp_tmp, format!("{}\n", serde_json::to_string_pretty(&config)?))?;
    fs::rename(&mcp_tmp, &mcp_path)?;
    Ok(())
}

/// Add the MCP servers of the project's enabled plugins to the per-launch config written by
/// [`install_claude`] (D159). Relay's own `relay` entry is never replaced.
pub fn add_claude_mcp_servers(
    worktree: &Path,
    servers: &[crate::plugins::LaunchServer],
) -> Result<()> {
    if servers.is_empty() {
        return Ok(());
    }
    let mcp_path = worktree.join(MCP_CONFIG_RELATIVE);
    let raw = fs::read_to_string(&mcp_path).with_context(|| format!("reading {}", mcp_path.display()))?;
    let mut config: Value = serde_json::from_str(&raw).with_context(|| format!("parsing {}", mcp_path.display()))?;
    let Some(entries) = config.get_mut("mcpServers").and_then(Value::as_object_mut) else {
        anyhow::bail!("{} has no mcpServers", mcp_path.display());
    };
    for (name, command, args, env) in servers {
        if name == "relay" {
            continue;
        }
        let mut entry = json!({"command": command, "args": args});
        if !env.is_empty() {
            entry["env"] = json!(env);
        }
        entries.insert(name.clone(), entry);
    }
    let mcp_tmp = mcp_path.with_extension("json.relay-tmp");
    fs::write(&mcp_tmp, format!("{}\n", serde_json::to_string_pretty(&config)?))?;
    fs::rename(&mcp_tmp, &mcp_path)?;
    Ok(())
}

/// Remove only Relay's Claude handlers; all user/project-local settings survive.
pub fn uninstall_claude(worktree: &Path) -> Result<()> {
    let _ = fs::remove_file(worktree.join(MCP_CONFIG_RELATIVE));
    let _ = fs::remove_file(worktree.join(CLAUDE_SETTINGS_RELATIVE));
    let _ = fs::remove_file(worktree.join(CLAUDE_STATUSLINE_RELATIVE));
    let dir = worktree.join(".claude");
    let path = dir.join("settings.local.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = fs::remove_file(worktree.join(MCP_CONFIG_RELATIVE));
            let _ = fs::remove_file(worktree.join(CLAUDE_SETTINGS_RELATIVE));
            let _ = fs::remove_file(worktree.join(CLAUDE_STATUSLINE_RELATIVE));
            return Ok(());
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mut root: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", path.display()))?;
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    for groups in hooks.values_mut().filter_map(Value::as_array_mut) {
        for group in groups.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                handlers.retain(|handler| {
                    !handler["command"]
                        .as_str()
                        .is_some_and(|command| command.contains(CLAUDE_HOOK_MARKER))
                });
            }
        }
        groups.retain(|group| {
            group["hooks"]
                .as_array()
                .is_none_or(|handlers| !handlers.is_empty())
        });
    }
    let tmp = dir.join("settings.local.json.relay-tmp");
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(&root)?))?;
    fs::rename(&tmp, &path)?;
    let _ = fs::remove_file(worktree.join(MCP_CONFIG_RELATIVE));
    Ok(())
}

/// Merge Relay's adapters into Codex's project hook layer. Codex asks the user to trust the
/// exact generated hook once through `/hooks`; Relay never bypasses that provider boundary.
pub fn install_codex(worktree: &Path, instance: Instance, relay: &Path) -> Result<()> {
    let dir = worktree.join(".codex");
    let path = dir.join("hooks.json");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut root: Value = match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({
            "description": "Workspace hooks, including Relay guardrails. Review once with /hooks."
        }),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let object = root.as_object_mut()
        .ok_or_else(|| anyhow!("{} must contain a JSON object", path.display()))?;
    let hooks = object.entry("hooks").or_insert_with(|| json!({})).as_object_mut()
        .ok_or_else(|| anyhow!("{}.hooks must be an object", path.display()))?;

    let pre = hooks.entry("PreToolUse").or_insert_with(|| json!([])).as_array_mut()
        .ok_or_else(|| anyhow!("{}.hooks.PreToolUse must be an array", path.display()))?;
    let installed = pre.iter().any(|group| group["hooks"].as_array().is_some_and(|handlers| {
        handlers.iter().any(|handler| handler["command"].as_str().is_some_and(|command| command.contains(CODEX_PRE_TOOL)))
    }));
    if !installed {
        pre.push(json!({
            "matcher": "Bash|apply_patch|Edit|Write",
            "hooks": [{
                "type": "command",
                "command": format!("{} --instance {} {}", shell_quote_path(relay), shell_quote(instance.as_str()), CODEX_PRE_TOOL),
                "timeout": 30,
                "statusMessage": "Checking Relay guardrails"
            }]
        }));
    }

    for (event, kind) in [("SessionStart", "session_start"), ("PostToolUse", "tool_use"), ("Stop", "stop")] {
        let groups = hooks.entry(event).or_insert_with(|| json!([])).as_array_mut()
            .ok_or_else(|| anyhow!("{}.hooks.{event} must be an array", path.display()))?;
        let installed = groups.iter().any(|group| group["hooks"].as_array().is_some_and(|handlers| {
            handlers.iter().any(|handler| handler["command"].as_str().is_some_and(|command| command.contains("hook codex-report")))
        }));
        if !installed {
            groups.push(json!({
                "hooks": [{
                    "type": "command",
                    "command": format!("{} --instance {} hook codex-report {}", shell_quote_path(relay), shell_quote(instance.as_str()), shell_quote(kind)),
                    "timeout": 10
                }]
            }));
        }
    }

    let tmp = dir.join("hooks.json.relay-tmp");
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(&root)?))
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Remove only Relay's Codex handlers. User and project hook definitions remain byte-for-byte
/// equivalent as JSON values.
pub fn uninstall_codex(worktree: &Path) -> Result<()> {
    let path = worktree.join(".codex/hooks.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mut root: Value = serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else { return Ok(()) };
    for groups in hooks.values_mut().filter_map(Value::as_array_mut) {
        for group in groups.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                handlers.retain(|handler| !handler["command"].as_str().is_some_and(|command| command.contains(CODEX_HOOK_MARKER)));
            }
        }
        groups.retain(|group| group["hooks"].as_array().is_none_or(|handlers| !handlers.is_empty()));
    }
    let tmp = worktree.join(".codex/hooks.json.relay-tmp");
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(&root)?))?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn claude_statusline_script() -> &'static str {
    "#!/bin/sh\ninput=$(cat)\ncase \"$input\" in\n  *'\"rate_limits\"'*) ;;\n  *) exit 0 ;;\nesac\nout=\"${CLAUDE_CONFIG_DIR:-$HOME}/.claude/relay-usage.json\"\nmkdir -p \"${out%/*}\" 2>/dev/null\nprintf '%s\\n' \"$input\" > \"$out\" 2>/dev/null\nexit 0\n"
}

fn git_output(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args).env("GIT_TERMINAL_PROMPT", "0");
    crate::proc::output_with_timeout(&mut command, GIT_TIMEOUT)
        .with_context(|| format!("running git {}", args.join(" ")))?
        .ok_or_else(|| anyhow!("git {} timed out after {} seconds", args.join(" "), GIT_TIMEOUT.as_secs()))
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = git_output(repo, args)?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_optional(repo: &Path, args: &[&str]) -> Option<String> {
    git(repo, args).ok().filter(|value| !value.is_empty())
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.display().to_string())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::{claude_statusline_script, install_git, refresh_git, relay_bin_from,
        remove_hook_dir, shell_quote, sweep_hook_dirs};
    use crate::Instance;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn shell_quotes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn statusline_is_silent_and_captures_rate_limits() {
        let script = claude_statusline_script();
        assert!(script.contains("rate_limits"));
        assert!(script.contains("relay-usage.json"));
        assert!(!script.contains("curl"));
    }

    #[test]
    fn relay_bin_finds_cli_beside_desktop_executable() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("relay-app");
        let cli = dir.path().join("relay");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(relay_bin_from(None, Some(app)), cli);
    }

    #[test]
    fn stale_hook_directories_are_swept_and_live_ones_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        for session in ["live-otter", "dead-koala", "dead-ferret"] {
            std::fs::create_dir_all(repo.join(".relay/hooks").join(session)).unwrap();
        }
        std::fs::write(repo.join(".relay/hooks/not-a-dir"), "").unwrap();

        assert_eq!(sweep_hook_dirs(repo, &["live-otter".to_string()]), 2);
        assert!(repo.join(".relay/hooks/live-otter").is_dir());
        assert!(!repo.join(".relay/hooks/dead-koala").exists());
        assert!(repo.join(".relay/hooks/not-a-dir").is_file(), "only directories are swept");

        remove_hook_dir(repo, "live-otter");
        assert!(!repo.join(".relay/hooks/live-otter").exists());
        // A repository with no hook directory at all is not an error.
        assert_eq!(sweep_hook_dirs(&repo.join("nowhere"), &[]), 0);
    }

    #[test]
    fn refresh_git_rewrites_an_existing_relay_hook() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let init = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "-b", "main"])
            .output()
            .unwrap();
        assert!(init.status.success());

        install_git(&repo, &repo, "steady-fox", Instance::Test, Path::new("/bin/false")).unwrap();
        assert!(refresh_git(&repo, &repo, Instance::Test, Path::new("/bin/true")).unwrap());

        let hook = repo.join(".relay/hooks/steady-fox/pre-commit");
        let script = std::fs::read_to_string(&hook).unwrap();
        assert!(script.contains("relay_bin='/bin/true'"));
        assert!(Command::new(&hook).status().unwrap().success());
    }

    /// Git LFS lives in `pre-push`, `post-checkout`, `post-commit` and `post-merge`; pointing
    /// `core.hooksPath` at Relay's directory must not switch them off.
    #[test]
    fn hooks_other_than_pre_commit_still_run_through_forwarders() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let remote = dir.path().join("remote.git");
        std::fs::create_dir(&repo).unwrap();
        let git = |cwd: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(cwd).args(args)
                .env_remove("RELAY_SESSION").env_remove("RELAY_TOKEN").output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        git(dir.path(), &["init", "-q", "--bare", remote.to_str().unwrap()]);
        git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
        let log = repo.join(".git/hook-log");
        let hooks = repo.join(".git/hooks");
        for (name, body) in [
            ("pre-push", format!("#!/bin/sh\necho \"pre-push $1\" >> '{0}'\ncat >> '{0}'\n", log.display())),
            ("post-commit", format!("#!/bin/sh\necho post-commit >> '{}'\n", log.display())),
        ] {
            std::fs::write(hooks.join(name), body).unwrap();
            std::fs::set_permissions(hooks.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // A hook some tool installed straight into the active directory is not Relay's to touch.
        let own = repo.join(".relay/hooks/lfs-otter");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("post-merge"), "#!/bin/sh\nexit 0\n").unwrap();

        install_git(&repo, &repo, "lfs-otter", Instance::Test, Path::new("/bin/true")).unwrap();
        assert!(own.join("pre-push").is_file() && own.join("post-commit").is_file());
        assert!(!own.join("commit-msg").exists(), "a hook the previous directory lacks got a forwarder");
        assert_eq!(std::fs::read_to_string(own.join("post-merge")).unwrap(), "#!/bin/sh\nexit 0\n");

        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "a.txt"]);
        git(&repo, &["commit", "-qm", "a"]);
        git(&repo, &["push", "-q", "origin", "main"]);
        let seen = std::fs::read_to_string(&log).unwrap();
        assert!(seen.contains("post-commit"), "{seen}");
        assert!(seen.contains("pre-push origin"), "{seen}");
        assert!(seen.contains("refs/heads/main"), "pre-push lost its stdin: {seen}");

        // A reinstall drops a forwarder whose target has gone, and keeps the rest.
        std::fs::remove_file(hooks.join("post-commit")).unwrap();
        assert!(refresh_git(&repo, &repo, Instance::Test, Path::new("/bin/true")).unwrap());
        assert!(!own.join("post-commit").exists());
        assert!(own.join("pre-push").is_file());
        // Relay's own files stay out of `git status` in a primary checkout.
        let status = Command::new("git").arg("-C").arg(&repo).args(["status", "--porcelain"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&status.stdout).trim(), "", "Relay's hook directory is untracked noise");
    }
}
