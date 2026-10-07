//! MCP stdio door (BUS.md §6.4). This is transport and argument shaping only: every tool
//! call is still a normal request through the socket door and the engine pipeline.

use anyhow::{Context, Result};
use relay_bus::registry::{OpEntry, OpKind, Registry};
use relay_bus::{Actor, MailHint, Request};
use relay_core::socket::Client;
use relay_core::Instance;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::OnceLock;

const FALLBACK_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: &[&str] = &["2026-07-28", "2025-11-25", FALLBACK_PROTOCOL];

pub async fn serve(instance: Instance, actor: Actor, token: Option<String>) -> Result<u8> {
    let stdin = std::io::stdin();
    let lines = stdin.lock().lines();
    let mut stdout = std::io::BufWriter::new(std::io::stdout());
    for line in lines {
        let line = line.context("reading MCP stdin")?;
        if line.trim().is_empty() { continue; }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(instance, &actor, token.as_deref(), message).await,
            Err(error) => Some(rpc_error(Value::Null, -32700, "Parse error", Some(json!({"message":error.to_string()})))),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut stdout, &response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(0)
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
}
