//! MCP stdio door (BUS.md §6.4). This is transport and argument shaping only: every tool
//! call is still a normal request through the socket door and the engine pipeline.

use anyhow::{Context, Result};
use relay_bus::registry::{Doors, OpEntry, OpKind, Registry};
use relay_bus::{Actor, MailHint, Request};
use relay_core::socket::Client;
use relay_core::Instance;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use tokio::sync::mpsc;

const FALLBACK_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: &[&str] = &["2026-07-28", "2025-11-25", FALLBACK_PROTOCOL];

pub async fn serve(instance: Instance, actor: Actor, token: Option<String>) -> Result<u8> {
    dispatch(stdin_lines(), Output::stdout(), move |message| {
        let (actor, token) = (actor.clone(), token.clone());
        async move { handle(instance, &actor, token.as_deref(), message).await }
    })
    .await
}

/// The bus server's loop (RA-061). Each request runs as its own task, so one long `bus.wait`
/// no longer queues every later call behind it, and answers go out whole lines at a time in
/// whatever order they finish. `notifications/cancelled` aborts the call's task: the client
/// gets no answer, as MCP asks, and the dropped socket ends a wait on the engine side. The
/// engine itself serializes what needs it, so nothing here has to stay in order.
async fn dispatch<F, Fut>(mut lines: mpsc::UnboundedReceiver<std::io::Result<String>>, output: Output, handler: F) -> Result<u8>
where
    F: Fn(Value) -> Fut,
    Fut: Future<Output = Option<Value>> + Send + 'static,
{
    let running: Arc<Mutex<HashMap<String, tokio::task::AbortHandle>>> = Arc::default();
    while let Some(line) = lines.recv().await {
        let Some(message) = parse_line(&line.context("reading MCP stdin")?, &output)? else { continue };
        if let Some(id) = cancelled(&message) {
            if let Some(task) = lock(&running).remove(&id) {
                task.abort();
            }
            continue;
        }
        // Other notifications never receive a response.
        let Some(key) = message.get("id").map(Value::to_string) else { continue };
        let call = handler(message);
        let (out, table, mine) = (output.clone(), running.clone(), key.clone());
        // Held across the spawn, so a call that finishes at once still finds itself listed.
        let mut listed = lock(&running);
        let task = tokio::spawn(async move {
            let response = call.await;
            if lock(&table).remove(&mine).is_some() {
                if let Some(response) = response {
                    let _ = out.send(&response);
                }
            }
        });
        listed.insert(key, task.abort_handle());
    }
    // The client has gone; nobody is left to read an answer.
    for (_, task) in lock(&running).drain() {
        task.abort();
    }
    Ok(0)
}

/// Whether a plugin tool call may run beside others or must wait its turn.
pub(crate) enum Lane {
    Parallel,
    /// One at a time per server, e.g. everything that drives the one Unreal editor.
    Serial,
}

/// The loop of the synchronous plugin servers (`blender-mcp`, `unreal-mcp`; RA-061). Every
/// `tools/call` runs on its own thread, so a render or a build no longer blocks the server;
/// `lane` keeps the calls that share one external thing (an editor) in order. A cancelled call
/// that has not started is skipped; one already running is a subprocess or an editor request
/// this side cannot interrupt, so it runs to its own timeout and its answer is dropped.
pub(crate) fn serve_sync(handle: fn(&Value) -> Option<Value>, lane: fn(&str, &Value) -> Lane) -> Result<u8> {
    serve_sync_on(std::io::stdin().lock(), Output::stdout(), handle, lane)
}

fn serve_sync_on(input: impl BufRead, output: Output, handle: fn(&Value) -> Option<Value>, lane: fn(&str, &Value) -> Lane) -> Result<u8> {
    // Request id -> cancelled.
    let running: Arc<Mutex<HashMap<String, bool>>> = Arc::default();
    let serial = Arc::new(Mutex::new(()));
    let mut threads: Vec<std::thread::JoinHandle<()>> = Vec::new();
    for line in input.lines() {
        let Some(message) = parse_line(&line.context("reading MCP stdin")?, &output)? else { continue };
        if let Some(id) = cancelled(&message) {
            if let Some(flag) = lock(&running).get_mut(&id) {
                *flag = true;
            }
            continue;
        }
        let Some(key) = message.get("id").map(Value::to_string) else { continue };
        if message["method"] != "tools/call" {
            if let Some(response) = handle(&message) {
                output.send(&response)?;
            }
            continue;
        }
        let name = message.pointer("/params/name").and_then(Value::as_str).unwrap_or_default();
        let turn = matches!(lane(name, message.pointer("/params/arguments").unwrap_or(&Value::Null)), Lane::Serial)
            .then(|| serial.clone());
        lock(&running).insert(key.clone(), false);
        let (out, table) = (output.clone(), running.clone());
        threads.retain(|thread| !thread.is_finished());
        threads.push(std::thread::spawn(move || {
            let _turn = turn.as_deref().map(lock);
            // Its own statement: the guard must be gone before the call runs.
            let wanted = lock(&table).get(&key) == Some(&false);
            let response = if wanted { handle(&message) } else { None };
            if lock(&table).remove(&key) == Some(false) {
                if let Some(response) = response {
                    let _ = out.send(&response);
                }
            }
        }));
    }
    // Calls already running finish, as they did when the loop handled them one by one.
    for thread in threads {
        let _ = thread.join();
    }
    Ok(0)
}

fn stdin_lines() -> mpsc::UnboundedReceiver<std::io::Result<String>> {
    let (sender, receiver) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

/// A JSON-RPC message, or `None` for a blank line or one that was answered with a parse error.
fn parse_line(line: &str, output: &Output) -> Result<Option<Value>> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<Value>(line) {
        Ok(message) => Ok(Some(message)),
        Err(error) => {
            output.send(&rpc_error(Value::Null, -32700, "Parse error", Some(json!({"message":error.to_string()}))))?;
            Ok(None)
        }
    }
}

/// The request a `notifications/cancelled` gives up on, keyed like a request id.
fn cancelled(message: &Value) -> Option<String> {
    (message["method"] == "notifications/cancelled" && message.get("id").is_none())
        .then(|| message.pointer("/params/requestId").map(Value::to_string))
        .flatten()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Stdout shared by every call in flight; each answer is written and flushed as one line under
/// the lock, so concurrent answers never interleave.
#[derive(Clone)]
struct Output(Arc<Mutex<Box<dyn Write + Send>>>);

impl Output {
    fn stdout() -> Self {
        Self(Arc::new(Mutex::new(Box::new(std::io::BufWriter::new(std::io::stdout())))))
    }

    fn send(&self, message: &Value) -> Result<()> {
        let mut out = lock(&self.0);
        serde_json::to_writer(&mut *out, message)?;
        out.write_all(b"\n")?;
        out.flush()?;
        Ok(())
    }
}

static PLUGIN_INSTANCE: OnceLock<Instance> = OnceLock::new();

/// The engine instance a plugin server's `relay --instance …` names; its guardrail is the one
/// that counts for this session.
pub(crate) fn use_instance(instance: Instance) {
    let _ = PLUGIN_INSTANCE.set(instance);
}

fn plugin_instance() -> Result<Instance> {
    if let Some(instance) = PLUGIN_INSTANCE.get() {
        return Ok(*instance);
    }
    let instance = std::env::var("RELAY_INSTANCE").unwrap_or_else(|_| "stable".into());
    Instance::parse(&instance).ok_or_else(|| anyhow::anyhow!("bad RELAY_INSTANCE {instance:?}"))
}

/// Plugin tools that run agent-supplied code (`blender_python`, `ue_python`, `ue_console`, …)
/// meet `guardrail.gate` before they run, as a Bash call does through the PreToolUse hook
/// (RA-077): an `exec` gate naming the tool, so a denied-command rule such as `blender_python`
/// or `ue_console` refuses it, a grant lifts that, and every call is on the audit record; and a
/// `write` gate for each file the tool says it will write, so protected paths and write roots
/// apply to a save.
///
/// What this cannot do: the code is arbitrary Python running as the user. It can write, delete
/// or run anything without declaring it, and no gate sees inside it — matching its source
/// against command patterns would only catch `rm -rf` spelled as shell, never `shutil.rmtree`.
/// The gate decides whether the tool runs at all and checks what it declares; it is not a
/// sandbox.
///
/// Outside a Relay session (no `RELAY_SESSION`) there is no guardrail to ask and the plugin runs
/// as a plain MCP server. Inside one, an engine that does not answer blocks the call.
pub(crate) fn plugin_gate(tool: &str, file: Option<&Path>, writes: &[&Path]) -> Result<()> {
    let Some(session) = std::env::var("RELAY_SESSION").ok().filter(|s| !s.is_empty()) else { return Ok(()) };
    let payloads = plugin_gate_payloads(&session, crate::hook_root(&json!({})).as_deref(), tool, file, writes)?;
    if cfg!(test) {
        // Unit tests run inside agent sessions; they must not reach the live engine.
        return Ok(());
    }
    let instance = plugin_instance()?;
    let (actor, token) = crate::actor_from_env(None)?;
    // These servers are synchronous; the bus client is not. A thread of its own keeps the
    // short-lived runtime clear of any runtime the caller may already be inside.
    let outcome = std::thread::scope(|scope| {
        scope.spawn(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build()?
                .block_on(crate::first_refusal(connect(instance), actor, token, payloads, crate::GATE_DEADLINE))
        }).join()
    }).map_err(|_| anyhow::anyhow!("the guardrail check panicked"))?;
    match outcome {
        Ok(None) => Ok(()),
        Ok(Some(error)) => anyhow::bail!("RELAY blocked {tool}: {}", crate::refusal_text(error.as_ref())),
        Err(error) => anyhow::bail!("RELAY guardrail unavailable, so {tool} was not run: {error:#}"),
    }
}

/// The folders this session may write to, as the guardrail reckons them (`bus.whoami`'s
/// `write_roots`: the worktree first, then scratch and memory roots and configured extras).
/// The Blender sandbox lets a child write there and nowhere else.
pub(crate) fn session_write_roots() -> Result<Vec<std::path::PathBuf>> {
    let instance = plugin_instance()?;
    let (actor, token) = crate::actor_from_env(None)?;
    let query = async {
        let mut client = connect(instance).await?;
        let mut request = Request::new(actor, "bus.whoami", json!({}));
        if let Some(token) = token { request = request.with_token(token); }
        let response = client.call(&request, |_| {}).await?;
        response.into_result().map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))
    };
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build()?
                .block_on(async { tokio::time::timeout(crate::GATE_DEADLINE, query).await.map_err(|_| anyhow::anyhow!("bus.whoami did not answer"))? })
        }).join()
    }).map_err(|_| anyhow::anyhow!("bus.whoami panicked"))??;
    Ok(result["write_roots"].as_array().into_iter().flatten().filter_map(Value::as_str).map(std::path::PathBuf::from).collect())
}

fn plugin_gate_payloads(session: &str, root: Option<&Path>, tool: &str, file: Option<&Path>, writes: &[&Path]) -> Result<Vec<Value>> {
    let command = match file {
        Some(file) => format!("{tool} {}", crate::hook_relative_path(root, &file.to_string_lossy())?),
        None => tool.to_string(),
    };
    let mut payloads = vec![json!({"session": session, "kind": "exec", "command": command})];
    for path in writes {
        // Binary files: no text to judge, so the empty diff of an edit in place; the path rules
        // still meet it.
        payloads.push(crate::write_gate(root, session, path, "diff", json!(""))?);
    }
    Ok(payloads)
}

async fn handle(instance: Instance, actor: &Actor, token: Option<&str>, message: Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str);
    let Some(id) = message.get("id").cloned() else {
        // MCP notifications never receive a response.
        return None;
    };
    let result = match method {
        Some("initialize") => {
            let requested = message.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or(FALLBACK_PROTOCOL);
            let protocol = SUPPORTED_PROTOCOLS.iter().copied().find(|version| *version == requested).unwrap_or(FALLBACK_PROTOCOL);
            Ok(json!({
                "protocolVersion": protocol,
                "capabilities": {"tools":{"listChanged":false}},
                "serverInfo": {"name":"relay","title":"Relay command bus","version":env!("CARGO_PKG_VERSION")},
                "instructions":"Relay's typed command bus. Tool names are bus op names; typed refusals are returned as tool errors. The tool list is filtered to what this session may actually call, all three gating layers applied — if an op is missing, your role cannot call it. Result shapes are not inlined here: `bus.schema {op}` returns one op's full payload and result schema. `session.bootstrap` returns your peers, your callable ops and your guardrails."
            }))
        }
        Some("ping") => Ok(json!({})),
        Some("tools/list") => tools(instance, actor, token).await,
        Some("tools/call") => {
            let Some(name) = message.pointer("/params/name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "Invalid params", Some(json!({"message":"tools/call requires params.name"}))));
            };
            let Some(entry) = Registry::global().get(name) else {
                return Some(rpc_error(id, -32602, "Unknown tool", Some(json!({"name":name}))));
            };
            if !exposed(entry) {
                return Some(rpc_error(id, -32602, "Tool is not exposed over MCP stdio", Some(json!({"name":name}))));
            }
            call_tool(instance, actor, token, name, &message).await
        }
        Some(other) => return Some(rpc_error(id, -32601, "Method not found", Some(json!({"method":other})))),
        None => return Some(rpc_error(id, -32600, "Invalid Request", None)),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(error) => rpc_error(id, -32603, "Relay MCP transport error", Some(json!({"message":format!("{error:#}")}))),
    })
}

async fn tools(instance: Instance, actor: &Actor, token: Option<&str>) -> Result<Value> {
    let mut client = connect(instance).await?;
    let mut request = Request::new(actor.clone(), "bus.ops", json!({}));
    if let Some(token) = token { request = request.with_token(token); }
    let response = client.call(&request, |_| {}).await?;
    let result = response.into_result().map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?;
    let listed = result["ops"].as_array().cloned().unwrap_or_default();
    // Advertise only what this session can really call. Listing the whole registry meant
    // ~115 of the tools were ops the role allowlist refuses at runtime, and the payload was
    // large enough that a harness could drop the lot — so the working integration reached
    // nobody at all (D104).
    let selection = std::env::var("RELAY_MCP_OPS").ok();
    let selection: Option<Vec<String>> = selection.map(|raw| {
        raw.split(',').map(str::trim).filter(|part| !part.is_empty()).map(str::to_string).collect()
    });
    let tools = listed.into_iter().filter_map(|info| {
        if info["implemented"] != true { return None; }
        let name = info["name"].as_str()?;
        let entry = Registry::global().get(name)?;
        if !exposed(entry) { return None; }
        let admitted = match &selection {
            // An explicit RELAY_MCP_OPS is a deliberate override: it selects the set, and the
            // engine still refuses anything the role may not call.
            Some(patterns) => patterns.iter().any(|pattern| pattern_matches(pattern, name)),
            None => info["call"] != "no",
        };
        admitted.then(|| tool(entry))
    }).collect::<Vec<_>>();

    report_tool_count(&mut client, actor, token, tools.len()).await;
    Ok(json!({"tools":tools}))
}

fn pattern_matches(pattern: &str, op: &str) -> bool {
    pattern == op
        || pattern == "*"
        || pattern
            .strip_suffix(".*")
            .is_some_and(|prefix| op.starts_with(prefix) && op.as_bytes().get(prefix.len()) == Some(&b'.'))
}

/// A session that receives zero tools currently fails in silence — the integration looks
/// configured and simply is not there. Say so once, on the record.
async fn report_tool_count(client: &mut Client, actor: &Actor, token: Option<&str>, count: usize) {
    static REPORTED: OnceLock<()> = OnceLock::new();
    let first = REPORTED.set(()).is_ok();
    if !first && count > 0 {
        return;
    }
    if count == 0 {
        eprintln!("relay mcp: this session was offered 0 tools — check its role allowlist or RELAY_MCP_OPS");
    }
    let Some(session) = actor.session_name() else { return };
    let mut request = Request::new(actor.clone(), "session.report", json!({
        "session": session,
        "kind": "notification",
        "data": {"notification_type": "relay_mcp_tools", "tools": count},
    }));
    if let Some(token) = token { request = request.with_token(token); }
    let _ = client.call(&request, |_| {}).await;
}

async fn call_tool(instance: Instance, actor: &Actor, token: Option<&str>, name: &str, message: &Value) -> Result<Value> {
    let arguments = message.pointer("/params/arguments").cloned().unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Ok(tool_error(
            json!({"kind":"invalid","code":"bus.schema","message":"tool arguments must be an object"}),
            None,
        ));
    }
    let mut client = connect(instance).await?;
    let mut request = Request::new(actor.clone(), name, arguments);
    if let Some(token) = token { request = request.with_token(token); }
    let response = client.call(&request, |_| {}).await?;
    let mail = response.mail.clone();
    if response.ok {
        let value = response.result.unwrap_or_else(|| json!({}));
        Ok(tool_success(value, mail.as_ref())?)
    } else {
        Ok(tool_error(serde_json::to_value(response.error)?, mail.as_ref()))
    }
}

fn exposed(entry: &OpEntry) -> bool {
    entry.meta.stream.is_none()
        && entry.meta.doors != Doors::TauriOnly
        && !matches!(entry.name, "bus.subscribe" | "bus.unsubscribe")
}

fn tool(entry: &OpEntry) -> Value {
    let schema = relay_bus::schema::render_op(entry.name).expect("registry op has schema");
    // Result schemas are two thirds of this document and are only needed *after* choosing an
    // op. They stay one `bus.schema {op}` call away rather than in every session's context.
    json!({
        "name":entry.name,
        "title":entry.name,
        "description":entry.meta.summary,
        "inputSchema":schema["payload"],
        "annotations":{
            "readOnlyHint":entry.meta.kind == OpKind::Query,
            "destructiveHint":entry.meta.kind == OpKind::Mutation,
            "openWorldHint":false
        }
    })
}

fn mail_notice(mail: Option<&MailHint>) -> Option<Value> {
    mail.map(|mail| {
        json!({
            "type": "text",
            "text": format!(
                "Relay priority mail: {} unread. Finish the current atomic action, then call mailbox.list with unread_only true and acknowledge each message.",
                mail.priority
            )
        })
    })
}

fn tool_success(value: Value, mail: Option<&MailHint>) -> Result<Value> {
    let mut content = vec![json!({
        "type":"text",
        "text":serde_json::to_string_pretty(&value)?
    })];
    if let Some(notice) = mail_notice(mail) {
        content.push(notice);
    }
    Ok(json!({"content":content,"structuredContent":value,"isError":false}))
}

fn tool_error(error: Value, mail: Option<&MailHint>) -> Value {
    let text = serde_json::to_string_pretty(&error).unwrap_or_else(|_| "Relay tool error".to_string());
    let mut content = vec![json!({"type":"text","text":text})];
    if let Some(notice) = mail_notice(mail) {
        content.push(notice);
    }
    json!({"content":content,"isError":true})
}

fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code":code,"message":message});
    if let Some(data) = data { error["data"] = data; }
    json!({"jsonrpc":"2.0","id":id,"error":error})
}

async fn connect(instance: Instance) -> Result<Client> {
    let path = instance.socket_path();
    Client::connect(&path).await.with_context(|| format!("no Relay engine at {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_is_the_bus_schema_and_streams_are_not_exposed() {
        let task = Registry::global().get("task.create").unwrap();
        let value = tool(task);
        assert_eq!(value["name"], "task.create");
        assert_eq!(value["inputSchema"]["type"], "object");
        assert!(value["inputSchema"]["required"].as_array().unwrap().iter().any(|item| item == "project_id"));
        assert_eq!(value["annotations"]["readOnlyHint"], false);
        assert!(value["outputSchema"].is_null(), "result schemas belong in bus.schema, not in every tool list");
        assert!(!exposed(Registry::global().get("session.attach").unwrap()));
        assert!(!exposed(Registry::global().get("device.mirror.start").unwrap()));
        assert!(exposed(Registry::global().get("session.brief").unwrap()));
        assert!(exposed(
            Registry::global().get("session.bootstrap").unwrap()
        ));
    }

    #[test]
    fn selection_patterns_are_exact_or_one_namespace() {
        assert!(pattern_matches("mailbox.send", "mailbox.send"));
        assert!(pattern_matches("mailbox.*", "mailbox.send"));
        assert!(pattern_matches("*", "anything.at_all"));
        assert!(!pattern_matches("mailbox.*", "mailboxes.send"));
        assert!(!pattern_matches("task.get", "task.list"));
    }

    #[test]
    fn priority_mail_is_visible_without_changing_the_typed_result() {
        let mail = MailHint { priority: 2 };
        let success = tool_success(json!({"task_id": 7}), Some(&mail)).unwrap();
        assert_eq!(success["structuredContent"], json!({"task_id": 7}));
        assert_eq!(success["content"].as_array().unwrap().len(), 2);
        assert!(success["content"][1]["text"].as_str().unwrap().contains("2 unread"));

        let error = tool_error(json!({"code":"task.not_found"}), Some(&mail));
        assert_eq!(error["isError"], true);
        assert!(error["content"][1]["text"].as_str().unwrap().contains("mailbox.list"));
    }

    /// A writer the test can read back, standing in for stdout.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Captured {
        fn output(&self) -> Output {
            Output(Arc::new(Mutex::new(Box::new(self.clone()))))
        }
        /// The ids answered, in the order the answers were written.
        fn ids(&self) -> Vec<Value> {
            String::from_utf8(lock(&self.0).clone()).unwrap().lines()
                .map(|line| serde_json::from_str::<Value>(line).expect("every answer is one whole JSON line")["id"].clone())
                .collect()
        }
    }

    fn call(id: u64, name: &str) -> String {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":{}}}).to_string()
    }

    fn cancel(id: u64) -> String {
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"user"}}).to_string()
    }

    /// `slow` takes 300 ms, `fast` none; `serial_*` tools share one lane.
    fn plugin_handle(message: &Value) -> Option<Value> {
        let name = message.pointer("/params/name").and_then(Value::as_str).unwrap_or_default();
        if name.ends_with("slow") {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        if name == "serial_marker" {
            assert!(!std::mem::replace(&mut *lock(&SERIAL_BUSY), true), "two serial calls ran at once");
            std::thread::sleep(std::time::Duration::from_millis(50));
            *lock(&SERIAL_BUSY) = false;
        }
        Some(json!({"jsonrpc":"2.0","id":message["id"],"result":{}}))
    }
    static SERIAL_BUSY: Mutex<bool> = Mutex::new(false);

    fn plugin_lane(name: &str, _args: &Value) -> Lane {
        if name.starts_with("serial") { Lane::Serial } else { Lane::Parallel }
    }

    #[test]
    fn plugin_calls_run_side_by_side_and_cancelled_ones_go_unanswered() {
        let captured = Captured::default();
        let input = [
            json!({"jsonrpc":"2.0","id":0,"method":"ping"}).to_string(),
            call(1, "slow"),
            call(2, "fast"),
            "not json".to_string(),
            call(3, "slow"),
            cancel(3),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
            call(4, "serial_slow"),
            call(5, "serial_slow"),
            cancel(5),
        ].join("\n");
        let started = std::time::Instant::now();
        serve_sync_on(std::io::Cursor::new(input), captured.output(), plugin_handle, plugin_lane).unwrap();
        let ids = captured.ids();
        // The fast call is answered while the slow one is still running.
        assert!(ids.iter().position(|id| id == 2) < ids.iter().position(|id| id == 1), "{ids:?}");
        assert!(ids.contains(&Value::Null), "the parse error is answered");
        assert!(!ids.contains(&json!(3)), "a cancelled call gets no answer: {ids:?}");
        assert!(!ids.contains(&json!(5)), "a cancelled call waiting for its lane gets no answer: {ids:?}");
        assert_eq!(ids.len(), 5, "{ids:?}");
        // 1, 3 and 4 overlap; 5 never ran.
        assert!(started.elapsed() < std::time::Duration::from_millis(900), "{:?}", started.elapsed());
    }

    #[test]
    fn serial_plugin_calls_wait_their_turn() {
        let captured = Captured::default();
        let input = (1..=4).map(|id| call(id, "serial_marker")).chain([call(9, "fast")]).collect::<Vec<_>>().join("\n");
        serve_sync_on(std::io::Cursor::new(input), captured.output(), plugin_handle, plugin_lane).unwrap();
        assert_eq!(captured.ids().len(), 5);
    }

    #[tokio::test]
    async fn bus_calls_run_side_by_side_and_cancelling_aborts_the_call() {
        let captured = Captured::default();
        let (sender, lines) = mpsc::unbounded_channel();
        let aborted = Arc::new(Mutex::new(true));
        let flag = aborted.clone();
        let server = tokio::spawn(dispatch(lines, captured.output(), move |message: Value| {
            let flag = flag.clone();
            async move {
                let name = message.pointer("/params/name").and_then(Value::as_str).unwrap_or_default().to_string();
                if name == "bus.wait" {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    *lock(&flag) = false;
                }
                message.get("id").map(|id| json!({"jsonrpc":"2.0","id":id,"result":{}}))
            }
        }));
        for line in [call(1, "bus.wait"), call(2, "bus.ping"), cancel(1), call(3, "bus.ping")] {
            sender.send(Ok(line)).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        drop(sender);
        server.await.unwrap().unwrap();
        assert_eq!(captured.ids(), [json!(2), json!(3)], "the wait neither blocks later calls nor answers once cancelled");
        assert!(*lock(&aborted), "the cancelled call was stopped, not left to finish");
    }

    #[test]
    fn plugin_code_meets_an_exec_gate_and_its_saves_meet_write_gates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("art/hero.blend");
        let copy = root.join("art/hero_v2.blend");
        let gates = plugin_gate_payloads("calm-otter", Some(&root), "blender_python", Some(&file), &[&copy]).unwrap();
        assert_eq!(gates, [
            json!({"session": "calm-otter", "kind": "exec", "command": "blender_python art/hero.blend"}),
            json!({"session": "calm-otter", "kind": "write", "path": "art/hero_v2.blend", "diff": ""}),
        ]);
        assert_eq!(plugin_gate_payloads("calm-otter", Some(&root), "ue_python", None, &[]).unwrap(),
            [json!({"session": "calm-otter", "kind": "exec", "command": "ue_python"})]);
    }

    #[test]
    fn cancellations_name_the_request_they_cancel() {
        assert_eq!(cancelled(&serde_json::from_str(&cancel(7)).unwrap()), Some("7".to_string()));
        let by_string = json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"7"}});
        assert_eq!(cancelled(&by_string), Some("\"7\"".to_string()), "string and number ids stay distinct");
        assert_eq!(cancelled(&serde_json::from_str(&call(7, "x")).unwrap()), None);
    }
}
