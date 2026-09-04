//! `relay` — the CLI door (BUS.md §8). A thin client of the socket door: argument shaping,
//! defaults from the environment, exit codes. No logic of its own.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use relay_bus::{Actor, Registry, Request, Response};
use relay_core::socket::Client;
use relay_core::Instance;
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

mod mcp;

#[derive(Parser)]
#[command(name = "relay", version, about = "RELAY v4 command bus CLI", long_about = None)]
struct Cli {
    /// Instance to talk to: stable | dev | test (env RELAY_INSTANCE)
    #[arg(long, global = true, env = "RELAY_INSTANCE", default_value = "stable")]
    instance: String,
    /// Actor override: user | test (agents come from RELAY_SESSION + RELAY_TOKEN)
    #[arg(long, global = true)]
    actor: Option<String>,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one op: `relay cmd <op> [<json-payload>|-]` or `relay cmd '<json-envelope>'`
    Cmd { op: String, payload: Option<String> },
    /// Like cmd, but prints the result only, pretty
    Q { op: String, payload: Option<String> },
    /// Subscribe and print Event lines until killed
    Events {
        /// Filter: exact name, `prefix.*`, repeatable
        #[arg(long)]
        filter: Vec<String>,
    },
    /// Print bus.v1.json, or one op's payload+result schema
    Schema { op: Option<String> },
    /// List ops
    Ops {
        /// Only ops my actor may really call, all gating layers applied (needs a running engine)
        #[arg(long)]
        mine: bool,
        /// Substring filter on op name and summary
        #[arg(long)]
        grep: Option<String>,
    },
    /// Liveness check
    Ping,
    /// Attach to a session's PTY and print its output (decoded) until killed
    Attach { session: String },
    /// Run a headless engine (owns the socket for the instance)
    Serve {
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Serve callable bus ops as MCP tools over stdio
    Mcp,
    /// Internal provider hook adapters. Enforcement fails closed; lifecycle reports are best-effort.
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        hook: HookCommand,
    },
}

#[derive(Subcommand)]
enum HookCommand {
    /// Translate Claude Code PreToolUse JSON on stdin into `guardrail.gate`.
    ClaudePreTool,
    /// Forward a Claude lifecycle hook payload to `session.report`.
    ClaudeReport { kind: String },
    /// Translate Codex PreToolUse JSON on stdin into `guardrail.gate`.
    CodexPreTool,
    /// Forward a Codex lifecycle hook payload to `session.report`.
    CodexReport { kind: String },
    /// Forward a Codex `notify` payload to `session.report`.
    CodexNotify { payload: String },
}

const EXIT_UNREACHABLE: u8 = 5;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("relay: {e:#}");
            ExitCode::from(EXIT_UNREACHABLE)
        }
    }
}

fn actor_from_env(override_: Option<&str>) -> Result<(Actor, Option<String>)> {
    if let Some(a) = override_ {
        return Ok((Actor::parse(a).map_err(|e| anyhow!(e))?, std::env::var("RELAY_TOKEN").ok()));
    }
    if let Ok(name) = std::env::var("RELAY_SESSION") {
        let token = std::env::var("RELAY_TOKEN").ok();
        return Ok((Actor::Agent(name), token));
    }
    if let Ok(a) = std::env::var("RELAY_ACTOR") {
        return Ok((Actor::parse(&a).map_err(|e| anyhow!(e))?, None));
    }
    Ok((Actor::User, None))
}

fn read_payload(arg: Option<String>) -> Result<Value> {
    match arg.as_deref() {
        None => Ok(json!({})),
        Some("-") => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            Ok(serde_json::from_str(&s).context("payload on stdin is not JSON")?)
        }
        Some(s) => Ok(serde_json::from_str(s).context("payload is not JSON")?),
    }
}

/// Does this op's payload schema require `project_id`?
fn requires_project(op: &str) -> bool {
    relay_bus::schema::render_op(op)
        .and_then(|s| s["payload"]["required"].as_array().map(|r| r.iter().any(|v| v == "project_id")))
        .unwrap_or(false)
}

/// Repo root of the cwd, by walking up to a `.git`.
fn cwd_repo_root() -> Option<PathBuf> {
    let mut p = std::env::current_dir().ok()?;
    loop {
        if p.join(".git").exists() {
            return std::fs::canonicalize(&p).ok();
        }
        if !p.pop() {
            return None;
        }
    }
}

async fn fill_project(client: &mut Client, actor: &Actor, token: &Option<String>, op: &str, payload: &mut Value) -> Result<()> {
    if !requires_project(op) || payload.get("project_id").is_some() {
        return Ok(());
    }
    if let Ok(p) = std::env::var("RELAY_PROJECT") {
        let id: i64 = p.parse().context("RELAY_PROJECT must be a project id")?;
        payload["project_id"] = json!(id);
        return Ok(());
    }
    if let Some(root) = cwd_repo_root() {
        let mut req = Request::new(actor.clone(), "project.list", json!({}));
        if let Some(t) = token { req = req.with_token(t.clone()); }
        let resp = client.call(&req, |_| {}).await?;
        if let Ok(v) = resp.into_result() {
            let root_s = root.display().to_string();
            if let Some(p) = v["projects"].as_array().and_then(|ps| ps.iter().find(|p| p["path"] == root_s)) {
                payload["project_id"] = p["id"].clone();
                return Ok(());
            }
        }
    }
    anyhow::bail!("{op} needs project_id: pass it in the payload, set RELAY_PROJECT, or run from inside a registered project (bus.project_required)");
}

fn exit_for(resp: &Response) -> u8 {
    match &resp.error {
        None => 0,
        Some(e) => e.kind.exit_code() as u8,
    }
}

async fn connect(instance: Instance) -> Result<Client> {
    let path = instance.socket_path();
    Client::connect(&path).await.with_context(|| format!("no engine for instance {instance} at {}", path.display()))
}

async fn run(cli: Cli) -> Result<u8> {
    let instance = Instance::parse(&cli.instance).ok_or_else(|| anyhow!("bad --instance {:?}", cli.instance))?;
    match cli.cmd {
        Command::Serve { store } => {
            tracing_subscriber::fmt()
                .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("RELAY_LOG").unwrap_or_else(|_| "info".into()))
                .init();
            match relay_core::serve::serve(instance, store).await {
                Ok(()) => Ok(0),
                Err(relay_core::socket::BindError::AlreadyRunning { instance, pid, socket }) => {
                    eprintln!("relay: engine already running for instance {instance} (pid {}) at {}", pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into()), socket.display());
                    Ok(EXIT_UNREACHABLE)
                }
                Err(relay_core::socket::BindError::Other(e)) => Err(e),
            }
        }
        Command::Mcp => {
            let (actor, token) = actor_from_env(cli.actor.as_deref())?;
            mcp::serve(instance, actor, token).await
        }
        Command::Schema { op } => {
            match op {
                None => print!("{}", relay_bus::schema::render_pretty()),
                Some(op) => {
                    let s = relay_bus::schema::render_op(&op).ok_or_else(|| anyhow!("unknown op {op:?}"))?;
                    println!("{}", serde_json::to_string_pretty(&s)?);
                }
            }
            Ok(0)
        }
        Command::Attach { session } => {
            use base64::Engine as _;
            let (actor, token) = actor_from_env(cli.actor.as_deref())?;
            let mut c = connect(instance).await?;
            let mut req = Request::new(actor, "session.attach", json!({ "session": session }));
            if let Some(t) = token { req = req.with_token(t); }
            let resp = c.call(&req, |_| {}).await?;
            if !resp.ok {
                println!("{}", serde_json::to_string(&resp)?);
                return Ok(exit_for(&resp));
            }
            use std::io::Write;
            let mut out = std::io::stdout();
            loop {
                match c.next().await? {
                    None => return Ok(0),
                    Some(relay_core::socket::Line::Frame(f)) => {
                        if let Some(b) = f.data.as_str().and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok()) {
                            out.write_all(&b)?;
                            out.flush()?;
                        }
                    }
                    Some(_) => {}
                }
            }
        }
        Command::Ping => {
            let mut c = connect(instance).await?;
            let req = Request::new(Actor::User, "bus.ping", json!({}));
            let resp = c.call(&req, |_| {}).await?;
            println!("{}", serde_json::to_string(&resp)?);
            Ok(exit_for(&resp))
        }
        Command::Ops { mine, grep } => {
            let (actor, token) = actor_from_env(cli.actor.as_deref())?;
            let ops: Vec<Value> = match connect(instance).await {
                Ok(mut c) => {
                    let mut req = Request::new(actor.clone(), "bus.ops", if mine { json!({}) } else { json!({"actor": "user"}) });
                    if let Some(t) = &token { req = req.with_token(t.clone()); }
                    let resp = c.call(&req, |_| {}).await?;
                    match resp.into_result() {
                        Ok(v) => v["ops"].as_array().cloned().unwrap_or_default(),
                        Err(e) => anyhow::bail!("bus.ops failed: {} {}", e.code, e.message),
                    }
                }
                Err(_) => {
                    if mine { anyhow::bail!("--mine needs a running engine"); }
                    Registry::global().entries().iter().map(|e| serde_json::to_value(relay_bus::registry::OpInfo::from_entry(e, false)).unwrap()).collect()
                }
            };
            let needle = grep.map(|g| g.to_ascii_lowercase());
            let matches = |o: &Value| match &needle {
                None => true,
                Some(needle) => {
                    let name = o["name"].as_str().unwrap_or("").to_ascii_lowercase();
                    let summary = o["summary"].as_str().unwrap_or("").to_ascii_lowercase();
                    name.contains(needle) || summary.contains(needle)
                }
            };
            // `--mine` used to answer with layer 1 only, which overstated a builder's write
            // surface tenfold. Every row now says whether it is really callable and why.
            println!("{:<32} {:<9} {:>3}  {:<5} {:<5} {:<34} summary", "op", "kind", "ph", "impl", "call", "why");
            let mut shown = 0;
            for o in ops.iter().filter(|o| matches(o)) {
                shown += 1;
                let call = match o["call"].as_str() {
                    Some("no") => "NO",
                    Some("self_only") => "SELF",
                    Some("yes") => "YES",
                    _ => "-",
                };
                println!(
                    "{:<32} {:<9} {:>3}  {:<5} {:<5} {:<34} {}",
                    o["name"].as_str().unwrap_or(""),
                    o["kind"].as_str().unwrap_or(""),
                    o["phase"],
                    if o["implemented"].as_bool().unwrap_or(false) { "yes" } else { "-" },
                    call,
                    o["why"].as_str().unwrap_or(""),
                    o["summary"].as_str().unwrap_or(""),
                );
            }
            if mine {
                let callable = ops.iter().filter(|o| matches(o) && o["call"] != "no").count();
                println!("\n{shown} listed, {callable} callable (SELF = own session only)");
            }
            Ok(0)
        }
        Command::Events { filter } => {
            let mut c = connect(instance).await?;
            let req = Request::new(Actor::User, "bus.subscribe", json!({ "events": filter }));
            let resp = c.call(&req, |_| {}).await?;
            if !resp.ok {
                println!("{}", serde_json::to_string(&resp)?);
                return Ok(exit_for(&resp));
            }
            loop {
                match c.next().await? {
                    None => return Ok(0),
                    Some(relay_core::socket::Line::Event(e)) => println!("{}", serde_json::to_string(&e)?),
                    Some(relay_core::socket::Line::Response(r)) => println!("{}", serde_json::to_string(&r)?),
                    Some(relay_core::socket::Line::Frame(f)) => println!("{}", serde_json::to_string(&f)?),
                }
            }
        }
        Command::Hook { hook: HookCommand::ClaudePreTool } => {
            match claude_pre_tool(instance, cli.actor.as_deref()).await {
                Ok(code) => Ok(code),
                Err(error) => {
                    // Claude Code only treats exit 2 as blocking. Infrastructure and malformed
                    // hook input must therefore fail closed too, not use the CLI's normal 5.
                    eprintln!("RELAY guardrail unavailable: {error:#}");
                    Ok(2)
                }
            }
        }
        Command::Hook { hook: HookCommand::ClaudeReport { kind } } => {
            if let Err(error) = claude_report(instance, cli.actor.as_deref(), &kind).await {
                // Lifecycle reporting is observability, not enforcement. A down Relay must
                // never trap Claude in a Stop-hook retry loop.
                eprintln!("RELAY lifecycle report unavailable: {error:#}");
            }
            Ok(0)
        }
        Command::Hook { hook: HookCommand::CodexPreTool } => {
            match codex_pre_tool(instance, cli.actor.as_deref()).await {
                Ok(code) => Ok(code),
                Err(error) => {
                    eprintln!("RELAY guardrail unavailable: {error:#}");
                    Ok(2)
                }
            }
        }
        Command::Hook { hook: HookCommand::CodexReport { kind } } => {
            if let Err(error) = lifecycle_hook_report(instance, cli.actor.as_deref(), &kind, "Codex").await {
                eprintln!("RELAY lifecycle report unavailable: {error:#}");
            }
            Ok(0)
        }
        Command::Hook { hook: HookCommand::CodexNotify { payload } } => {
            if let Err(error) = codex_notify(instance, cli.actor.as_deref(), &payload).await {
                eprintln!("RELAY lifecycle report unavailable: {error:#}");
            }
            Ok(0)
        }
        Command::Cmd { op, payload } => do_cmd(instance, cli.actor.as_deref(), op, payload, false).await,
        Command::Q { op, payload } => do_cmd(instance, cli.actor.as_deref(), op, payload, true).await,
    }
}

async fn claude_pre_tool(instance: Instance, actor_override: Option<&str>) -> Result<u8> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: Value = serde_json::from_str(&raw).context("Claude hook input is not JSON")?;
    let tool = input["tool_name"]
        .as_str()
        .ok_or_else(|| anyhow!("Claude hook input has no tool_name"))?;
    let tool_input = input["tool_input"]
        .as_object()
        .ok_or_else(|| anyhow!("Claude hook input has no tool_input object"))?;
    let session = std::env::var("RELAY_SESSION").context("RELAY_SESSION is missing")?;

    let payload = match tool {
        "Bash" => json!({
            "session": session,
            "kind": "exec",
            "command": required_string(tool_input, "command")?,
        }),
        "Write" => json!({
            "session": session,
            "kind": "write",
            "path": hook_relative_path(&input, required_string(tool_input, "file_path")?)?,
            "new_text": required_string(tool_input, "content")?,
        }),
        "Edit" => {
            let file_path = required_string(tool_input, "file_path")?;
            let old = required_string(tool_input, "old_string")?;
            let new = required_string(tool_input, "new_string")?;
            let replace_all = tool_input.get("replace_all").and_then(Value::as_bool).unwrap_or(false);
            let current = std::fs::read_to_string(file_path)
                .with_context(|| format!("reading edit target {file_path}"))?;
            if !current.contains(old) {
                anyhow::bail!("Edit old_string does not occur in {file_path}");
            }
            let next = if replace_all { current.replace(old, new) } else { current.replacen(old, new, 1) };
            json!({
                "session": session,
                "kind": "write",
                "path": hook_relative_path(&input, file_path)?,
                "new_text": next,
            })
        }
        // Kept for Claude versions that still expose the former batched edit tool.
        "MultiEdit" => {
            let file_path = required_string(tool_input, "file_path")?;
            let mut next = std::fs::read_to_string(file_path)
                .with_context(|| format!("reading multi-edit target {file_path}"))?;
            let edits = tool_input.get("edits").and_then(Value::as_array)
                .ok_or_else(|| anyhow!("MultiEdit input has no edits array"))?;
            for edit in edits {
                let object = edit.as_object().ok_or_else(|| anyhow!("MultiEdit edit is not an object"))?;
                let old = required_string(object, "old_string")?;
                let new = required_string(object, "new_string")?;
                if !next.contains(old) {
                    anyhow::bail!("MultiEdit old_string does not occur in {file_path}");
                }
                next = if object.get("replace_all").and_then(Value::as_bool).unwrap_or(false) {
                    next.replace(old, new)
                } else {
                    next.replacen(old, new, 1)
                };
            }
            json!({
                "session": session,
                "kind": "write",
                "path": hook_relative_path(&input, file_path)?,
                "new_text": next,
            })
        }
        other => anyhow::bail!("unsupported Claude PreToolUse tool {other:?}"),
    };

    let (actor, token) = actor_from_env(actor_override)?;
    let mut client = connect(instance).await?;
    let mut request = Request::new(actor, "guardrail.gate", payload);
    if let Some(token) = token {
        request = request.with_token(token);
    }
    let response = client.call(&request, |_| {}).await?;
    if response.ok {
        return Ok(0);
    }
    if let Some(error) = response.error {
        eprintln!("RELAY blocked {tool}: {} ({})", error.message, error.code);
    } else {
        eprintln!("RELAY blocked {tool}: guardrail returned no result");
    }
    Ok(2)
}

async fn claude_report(instance: Instance, actor_override: Option<&str>, kind: &str) -> Result<()> {
    lifecycle_hook_report(instance, actor_override, kind, "Claude").await
}

async fn lifecycle_hook_report(instance: Instance, actor_override: Option<&str>, kind: &str, provider: &str) -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let data: Value = if raw.trim().is_empty() { json!({}) } else {
        serde_json::from_str(&raw).with_context(|| format!("{provider} lifecycle hook input is not JSON"))?
    };
    lifecycle_report(instance, actor_override, kind, data).await
}

async fn codex_pre_tool(instance: Instance, actor_override: Option<&str>) -> Result<u8> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: Value = serde_json::from_str(&raw).context("Codex hook input is not JSON")?;
    let tool = input["tool_name"].as_str().ok_or_else(|| anyhow!("Codex hook input has no tool_name"))?;
    let tool_input = input["tool_input"].as_object().ok_or_else(|| anyhow!("Codex hook input has no tool_input object"))?;
    let session = std::env::var("RELAY_SESSION").context("RELAY_SESSION is missing")?;
    let command = required_string(tool_input, "command")?;
    let payload = match tool {
        "Bash" | "apply_patch" | "Edit" | "Write" => json!({
            "session": session,
            "kind": "exec",
            "command": command,
        }),
        other => anyhow::bail!("unsupported Codex PreToolUse tool {other:?}"),
    };

    let (actor, token) = actor_from_env(actor_override)?;
    let mut client = connect(instance).await?;
    let mut request = Request::new(actor, "guardrail.gate", payload);
    if let Some(token) = token { request = request.with_token(token); }
    let response = client.call(&request, |_| {}).await?;
    if response.ok { return Ok(0); }
    if let Some(error) = response.error {
        eprintln!("RELAY blocked {tool}: {} ({})", error.message, error.code);
    } else {
        eprintln!("RELAY blocked {tool}: guardrail returned no result");
    }
    Ok(2)
}

async fn codex_notify(instance: Instance, actor_override: Option<&str>, raw: &str) -> Result<()> {
    let data: Value = serde_json::from_str(raw).context("Codex notification payload is not JSON")?;
    if data.get("type").and_then(Value::as_str) != Some("agent-turn-complete") {
        return Ok(());
    }
    lifecycle_report(instance, actor_override, "stop", data).await
}

async fn lifecycle_report(instance: Instance, actor_override: Option<&str>, kind: &str, data: Value) -> Result<()> {
    let session = std::env::var("RELAY_SESSION").context("RELAY_SESSION is missing")?;
    let (actor, token) = actor_from_env(actor_override)?;
    let mut client = connect(instance).await?;
    let mut request = Request::new(actor, "session.report", json!({
        "session": session,
        "kind": kind,
        "data": data,
    }));
    if let Some(token) = token { request = request.with_token(token); }
    let response = client.call(&request, |_| {}).await?;
    response.into_result().map(|_| ()).map_err(|error| anyhow!("{}: {}", error.code, error.message))
}

fn required_string<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Result<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("hook input field {key:?} is missing or not a string"))
}

/// The path `guardrail.gate` should judge. Inside the worktree it is worktree-relative, as
/// the gate expects. Outside, the absolute path goes through unchanged: whether a write to
/// scratch space is allowed is a policy question, and policy lives on the bus, not in this
/// adapter (BUS.md §9.3). Refusing here produced "guardrail unavailable" for what was really
/// a deliberate decision, and left the agent with nowhere legal to put a temporary file.
fn hook_relative_path(input: &Value, raw_path: &str) -> Result<String> {
    let path = PathBuf::from(raw_path);
    let root = std::env::var_os("RELAY_WORKTREE")
        .map(PathBuf::from)
        .or_else(|| input["cwd"].as_str().map(PathBuf::from))
        .or_else(cwd_repo_root)
        .ok_or_else(|| anyhow!("cannot determine the Relay worktree"))?;
    match path.strip_prefix(&root) {
        Ok(relative) if relative.as_os_str().is_empty() => {
            anyhow::bail!("write target is the worktree directory")
        }
        Ok(relative) => Ok(relative.display().to_string()),
        Err(_) => Ok(path.display().to_string()),
    }
}

async fn do_cmd(instance: Instance, actor_override: Option<&str>, op: String, payload: Option<String>, pretty: bool) -> Result<u8> {
    let (actor, token) = actor_from_env(actor_override)?;
    let mut c = connect(instance).await?;
    let req = if op.trim_start().starts_with('{') {
        // full envelope
        let mut v: Value = serde_json::from_str(&op).context("envelope is not JSON")?;
        if v.get("v").is_none() { v["v"] = json!(relay_bus::ENVELOPE_V); }
        if v.get("id").is_none() { v["id"] = json!(uuid::Uuid::new_v4()); }
        if v.get("actor").is_none() { v["actor"] = json!(actor.to_string()); }
        if v.get("token").is_none() { if let Some(t) = &token { v["token"] = json!(t); } }
        serde_json::from_value::<Request>(v).context("bad envelope")?
    } else {
        let mut p = read_payload(payload)?;
        if !p.is_object() { anyhow::bail!("payload must be a JSON object"); }
        fill_project(&mut c, &actor, &token, &op, &mut p).await?;
        let mut r = Request::new(actor, op, p);
        if let Some(t) = token { r = r.with_token(t); }
        r
    };
    let resp = c.call(&req, |_| {}).await?;
    if pretty {
        match &resp.error {
            None => println!("{}", serde_json::to_string_pretty(resp.result.as_ref().unwrap_or(&Value::Null))?),
            // A typed refusal printed on stdout beside real results reads as a result — every
            // caller then has to special-case an error that arrived looking like success.
            Some(e) => eprintln!("{}", serde_json::to_string_pretty(e)?),
        }
    } else {
        println!("{}", serde_json::to_string(&resp)?);
    }
    if pretty {
        if let Some(mail) = &resp.mail {
            eprintln!(
                "RELAY priority mail: {} unread. Finish the current atomic action, then run mailbox.list with unread_only true and acknowledge each message.",
                mail.priority
            );
        }
    }
    Ok(exit_for(&resp))
}
