//! `relay unreal-mcp` — the Unreal Engine plugin's MCP server (D159).
//!
//! Stdio JSON-RPC, like `relay mcp`, but it never touches the Relay bus: it works on the
//! Unreal project in the agent's checkout and on the Unreal editor running beside it. Offline
//! tools read the `.uproject`, run UnrealBuildTool and tail the editor log. Live tools talk to
//! the editor's Remote Control web server (plain HTTP on localhost, port 30010 by default) and
//! run editor Python through `PythonScriptLibrary`, which is how an agent edits binary assets
//! it cannot open as text.
//!
//! Environment: `UE_PROJECT` (a `.uproject` path) overrides discovery, `UE_ROOT` names the
//! engine directory (the one holding `Engine/`), `UE_REMOTE_CONTROL_URL` the editor endpoint
//! and `UE_REMOTE_CONTROL_PASSPHRASE` its passphrase when one is required.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use std::io::{BufRead, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const FALLBACK_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: &[&str] = &["2026-07-28", "2025-11-25", FALLBACK_PROTOCOL];
const DEFAULT_REMOTE: &str = "http://127.0.0.1:30010";
const PYTHON_LIBRARY: &str = "/Script/PythonScriptPlugin.Default__PythonScriptLibrary";
/// Editor plugins the live tools depend on, by `.uproject` plugin name.
const BRIDGE_PLUGINS: [&str; 3] = ["RemoteControl", "PythonScriptPlugin", "EditorScriptingUtilities"];
/// Folders a project scan never enters: generated, cached or version-control state.
const SKIP_DIRS: [&str; 7] = ["Binaries", "Intermediate", "Saved", "DerivedDataCache", ".git", "node_modules", ".relay"];
const MAX_OUTPUT: usize = 60_000;

pub fn serve() -> Result<u8> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::BufWriter::new(std::io::stdout());
    for line in stdin.lock().lines() {
        let line = line.context("reading MCP stdin")?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&message),
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

fn handle(message: &Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let result = match message.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            let requested = message.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or(FALLBACK_PROTOCOL);
            let protocol = SUPPORTED_PROTOCOLS.iter().copied().find(|v| *v == requested).unwrap_or(FALLBACK_PROTOCOL);
            json!({
                "protocolVersion": protocol,
                "capabilities": {"tools":{"listChanged":false}},
                "serverInfo": {"name":"unreal","title":"Unreal Engine (Relay plugin)","version":env!("CARGO_PKG_VERSION")},
                "instructions": "Unreal Engine bridge for the project in this checkout. Start with ue_project_info. ue_build and ue_log work without the editor; ue_python, ue_call, ue_property, ue_search_assets, ue_level_actors and ue_console need the editor open with its Remote Control web server running (check with ue_editor_status, diagnose with ue_setup_check). Load the unreal-editor-automation skill before editor scripting."
            })
        }
        Some("ping") => json!({}),
        Some("tools/list") => json!({"tools": tools()}),
        Some("tools/call") => {
            let Some(name) = message.pointer("/params/name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "Invalid params", Some(json!({"message":"tools/call requires params.name"}))));
            };
            let arguments = message.pointer("/params/arguments").cloned().unwrap_or_else(|| json!({}));
            if !tools().iter().any(|tool| tool["name"] == name) {
                return Some(rpc_error(id, -32602, "Unknown tool", Some(json!({"name":name}))));
            }
            match call(name, &arguments) {
                Ok(value) => tool_result(value, false),
                Err(error) => tool_result(json!({"error": format!("{error:#}")}), true),
            }
        }
        Some(other) => return Some(rpc_error(id, -32601, "Method not found", Some(json!({"method":other})))),
        None => return Some(rpc_error(id, -32600, "Invalid Request", None)),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type":"object","properties":properties,"required":required,"additionalProperties":false},
        "annotations": {"readOnlyHint": read_only, "destructiveHint": !read_only, "openWorldHint": false}
    })
}

fn tools() -> Vec<Value> {
    vec![
        tool("ue_project_info",
            "The Unreal project in this checkout: .uproject fields (engine association, modules, plugins), build targets, C++ modules, project plugins, config files, maps, asset counts, the resolved engine directory and log path. Works without the editor.",
            json!({}), &[], true),
        tool("ue_setup_check",
            "Whether the editor bridge is ready: the RemoteControl, PythonScriptPlugin and EditorScriptingUtilities plugins in the .uproject, whether the editor's Remote Control server answers, and whether remote Python runs. With fix=true, adds the missing plugins to the .uproject (takes effect after the editor restarts).",
            json!({"fix":{"type":"boolean","description":"Add missing bridge plugins to the .uproject"}}), &[], false),
        tool("ue_build",
            "Build with UnrealBuildTool through the engine's Build script. Defaults: the project's Editor target, the host platform, Development. Close the editor first (or use Live Coding in the editor instead). Returns success, the error lines and the tail of the output.",
            json!({
                "target":{"type":"string","description":"Target name, e.g. MyGameEditor, MyGame, MyGameServer"},
                "platform":{"type":"string","description":"Linux, Win64 or Mac; defaults to the host"},
                "configuration":{"type":"string","enum":["Debug","DebugGame","Development","Test","Shipping"]},
                "timeout_s":{"type":"integer","minimum":30,"maximum":7200,"description":"Default 3600"}
            }), &[], false),
        tool("ue_log",
            "Tail the project's editor/game log (Saved/Logs/<Project>.log), optionally keeping only lines that match a regex such as 'Error|Warning' or 'LogBlueprint'.",
            json!({
                "lines":{"type":"integer","minimum":1,"maximum":5000,"description":"Lines to return, default 200"},
                "filter":{"type":"string","description":"Regex applied before taking the tail"},
                "file":{"type":"string","description":"Another file under Saved/Logs, e.g. a backup log"}
            }), &[], true),
        tool("ue_editor_status",
            "Whether the Unreal editor answers on its Remote Control web server, with the server's route information.",
            json!({}), &[], true),
        tool("ue_python",
            "Run Python in the running editor (import unreal). Returns printed output, log lines and the result. Wrap asset or level changes in `with unreal.ScopedEditorTransaction(\"...\"):` and save what you change.",
            json!({
                "code":{"type":"string","description":"Python source; multiple lines are fine"},
                "timeout_s":{"type":"integer","minimum":5,"maximum":1800,"description":"Default 120"}
            }), &["code"], false),
        tool("ue_call",
            "Call a UFUNCTION on an object through Remote Control (PUT /remote/object/call). Object paths look like /Game/Maps/Main.Main:PersistentLevel.Door_2 or, for static functions, /Script/Module.Default__Class.",
            json!({
                "object_path":{"type":"string"},
                "function":{"type":"string"},
                "parameters":{"type":"object","description":"Function parameters by name"},
                "transaction":{"type":"boolean","description":"Record an undoable transaction (default true)"}
            }), &["object_path","function"], false),
        tool("ue_property",
            "Read a property of an object through Remote Control, or write it when value is given (as an undoable transaction).",
            json!({
                "object_path":{"type":"string"},
                "property":{"type":"string"},
                "value":{"description":"New value; omit to read"}
            }), &["object_path","property"], false),
        tool("ue_search_assets",
            "Search the Asset Registry in the running editor by name fragment, class and folder.",
            json!({
                "query":{"type":"string","description":"Case-insensitive fragment of the asset name"},
                "class_names":{"type":"array","items":{"type":"string"},"description":"e.g. StaticMesh, Blueprint, Material, MaterialInstanceConstant, World"},
                "package_paths":{"type":"array","items":{"type":"string"},"description":"Folders to search, default /Game"},
                "limit":{"type":"integer","minimum":1,"maximum":2000,"description":"Default 100"}
            }), &[], true),
        tool("ue_level_actors",
            "List the actors in the level open in the editor: label, class, object path, location and folder.",
            json!({
                "class_filter":{"type":"string","description":"Case-insensitive fragment of the class name"},
                "name_filter":{"type":"string","description":"Case-insensitive fragment of the actor label"},
                "selected_only":{"type":"boolean"},
                "limit":{"type":"integer","minimum":1,"maximum":5000,"description":"Default 200"}
            }), &[], true),
        tool("ue_console",
            "Run a console command in the editor world, e.g. 'stat unit', 'r.ScreenPercentage 75', 'obj list class=StaticMesh'. Output goes to the log; read it with ue_log.",
            json!({"command":{"type":"string"}}), &["command"], false),
    ]
}

fn call(name: &str, args: &Value) -> Result<Value> {
    match name {
        "ue_project_info" => project_info(&Project::find()?),
        "ue_setup_check" => setup_check(&Project::find()?, args["fix"].as_bool().unwrap_or(false)),
        "ue_build" => build(&Project::find()?, args),
        "ue_log" => log(&Project::find()?, args),
        "ue_editor_status" => editor_status(),
        "ue_python" => {
            let code = args["code"].as_str().ok_or_else(|| anyhow!("code is required"))?;
            python(code, secs(args, "timeout_s", 120))
        }
        "ue_call" => {
            let mut body = json!({
                "objectPath": required(args, "object_path")?,
                "functionName": required(args, "function")?,
                "generateTransaction": args["transaction"].as_bool().unwrap_or(true),
            });
            if let Some(parameters) = args.get("parameters").filter(|p| p.is_object()) {
                body["parameters"] = parameters.clone();
            }
            remote("PUT", "/remote/object/call", Some(&body), Duration::from_secs(120))
        }
        "ue_property" => {
            let property = required(args, "property")?;
            let mut body = json!({"objectPath": required(args, "object_path")?, "propertyName": property});
            match args.get("value") {
                Some(value) => {
                    body["access"] = json!("WRITE_TRANSACTION_ACCESS");
                    body["propertyValue"] = json!({ property: value });
                }
                None => body["access"] = json!("READ_ACCESS"),
            }
            remote("PUT", "/remote/object/property", Some(&body), Duration::from_secs(30))
        }
        "ue_search_assets" => python_json(&search_assets_script(args), Duration::from_secs(120)),
        "ue_level_actors" => python_json(&level_actors_script(args), Duration::from_secs(60)),
        "ue_console" => {
            let command = required(args, "command")?;
            let code = format!(
                "import unreal\nworld = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()\nunreal.SystemLibrary.execute_console_command(world, {})\nprint('ran', {})\n",
                py_str(&command), py_str(&command)
            );
            python(&code, Duration::from_secs(60))
        }
        other => bail!("unknown tool {other}"),
    }
}

fn required(args: &Value, key: &str) -> Result<String> {
    args[key].as_str().filter(|v| !v.trim().is_empty()).map(str::to_string).ok_or_else(|| anyhow!("{key} is required"))
}

fn secs(args: &Value, key: &str, default: u64) -> Duration {
    Duration::from_secs(args[key].as_u64().unwrap_or(default).clamp(5, 7200))
}

/// A JSON string literal is also a valid Python string literal.
fn py_str(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

// ---------------------------------------------------------------- the project on disk

struct Project {
    uproject: PathBuf,
    root: PathBuf,
    name: String,
    descriptor: Value,
}

impl Project {
    fn find() -> Result<Project> {
        let uproject = match std::env::var_os("UE_PROJECT") {
            Some(path) => PathBuf::from(path),
            None => {
                let start = std::env::var_os("RELAY_WORKTREE").map(PathBuf::from)
                    .filter(|p| p.is_dir())
                    .unwrap_or(std::env::current_dir()?);
                find_uproject(&start).ok_or_else(|| anyhow!(
                    "no .uproject within three folders of {} — set UE_PROJECT to the project file", start.display()
                ))?
            }
        };
        let raw = std::fs::read_to_string(&uproject).with_context(|| format!("reading {}", uproject.display()))?;
        let descriptor: Value = serde_json::from_str(&raw).with_context(|| format!("parsing {}", uproject.display()))?;
        let root = uproject.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = uproject.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Project { uproject, root, name, descriptor })
    }

    fn log_path(&self) -> PathBuf {
        self.root.join("Saved/Logs").join(format!("{}.log", self.name))
    }
}

fn find_uproject(start: &Path) -> Option<PathBuf> {
    let mut level = vec![start.to_path_buf()];
    for _ in 0..=3 {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in &entries {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "uproject") && path.is_file() {
                    return Some(path);
                }
            }
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() && !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    next.push(entry.path());
                }
            }
        }
        level = next;
    }
    None
}

/// The engine directory (the one holding `Engine/`) this project builds with.
fn engine_root(project: &Project) -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("UE_ROOT").map(PathBuf::from) {
        if root.join("Engine").is_dir() {
            return Ok(root);
        }
        bail!("UE_ROOT={} has no Engine folder", root.display());
    }
    let association = project.descriptor["EngineAssociation"].as_str().unwrap_or("").trim().to_string();
    // A project inside a source engine tree carries no association.
    if association.is_empty() {
        for dir in project.root.ancestors().skip(1) {
            if dir.join("Engine/Build/BatchFiles").is_dir() {
                return Ok(dir.to_path_buf());
            }
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    // Launcher and source builds register themselves here on Linux and macOS.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for ini in [home.join(".config/Epic/UnrealEngine/Install.ini"), home.join("Library/Application Support/Epic/UnrealEngine/Install.ini")] {
            if let Ok(text) = std::fs::read_to_string(&ini) {
                for line in text.lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        if key.trim().eq_ignore_ascii_case(&association) {
                            candidates.push(PathBuf::from(value.trim()));
                        }
                    }
                }
            }
        }
        if !association.is_empty() {
            candidates.push(home.join(format!("UnrealEngine-{association}")));
            candidates.push(home.join(format!("UE_{association}")));
        }
        candidates.push(home.join("UnrealEngine"));
    }
    if !association.is_empty() {
        candidates.push(PathBuf::from(format!("/opt/UnrealEngine-{association}")));
        candidates.push(PathBuf::from(format!("/Users/Shared/Epic Games/UE_{association}")));
        candidates.push(PathBuf::from(format!("C:/Program Files/Epic Games/UE_{association}")));
    }
    candidates.push(PathBuf::from("/opt/UnrealEngine"));
    candidates
        .into_iter()
        .find(|dir| dir.join("Engine/Build/BatchFiles").is_dir())
        .ok_or_else(|| anyhow!(
            "cannot find the engine for EngineAssociation {association:?} — set UE_ROOT to the folder that contains Engine/"
        ))
}

fn host_platform() -> &'static str {
    if cfg!(target_os = "windows") { "Win64" } else if cfg!(target_os = "macos") { "Mac" } else { "Linux" }
}

fn build_script(engine: &Path) -> PathBuf {
    let batch = engine.join("Engine/Build/BatchFiles");
    if cfg!(target_os = "windows") {
        batch.join("Build.bat")
    } else if cfg!(target_os = "macos") {
        batch.join("Mac/Build.sh")
    } else {
        batch.join("Linux/Build.sh")
    }
}

fn names_with_suffix(dir: &Path, suffix: &str, depth: usize) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() && depth > 0 && !SKIP_DIRS.contains(&name.as_str()) {
            out.extend(names_with_suffix(&path, suffix, depth - 1));
        } else if let Some(stem) = name.strip_suffix(suffix) {
            out.push(stem.to_string());
        }
    }
    out.sort();
    out
}

fn project_info(project: &Project) -> Result<Value> {
    let source = project.root.join("Source");
    let targets = names_with_suffix(&source, ".Target.cs", 0);
    let modules = names_with_suffix(&source, ".Build.cs", 2);
    let plugins = names_with_suffix(&project.root.join("Plugins"), ".uplugin", 2);
    let configs = names_with_suffix(&project.root.join("Config"), ".ini", 1);
    let mut counts = Map::new();
    let mut maps = Vec::new();
    let mut seen = 0usize;
    let content = project.root.join("Content");
    count_content(&content, &content, &mut counts, &mut maps, &mut seen);
    let engine = engine_root(project);
    Ok(json!({
        "name": project.name,
        "uproject": project.uproject,
        "root": project.root,
        "engine_association": project.descriptor["EngineAssociation"],
        "engine_root": engine.as_ref().ok(),
        "engine_problem": engine.as_ref().err().map(|e| e.to_string()),
        "descriptor_modules": project.descriptor["Modules"],
        "descriptor_plugins": project.descriptor["Plugins"],
        "target_platforms": project.descriptor["TargetPlatforms"],
        "targets": targets,
        "cpp_modules": modules,
        "blueprint_only": targets.is_empty(),
        "project_plugins": plugins,
        "config_files": configs,
        "content_files_by_extension": counts,
        "content_files_scanned": seen,
        "maps": maps,
        "log": project.log_path(),
        "log_exists": project.log_path().is_file(),
    }))
}

fn count_content(base: &Path, dir: &Path, counts: &mut Map<String, Value>, maps: &mut Vec<String>, seen: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *seen >= 200_000 {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            count_content(base, &path, counts, maps, seen);
            continue;
        }
        *seen += 1;
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let slot = counts.entry(ext.clone()).or_insert(json!(0));
        *slot = json!(slot.as_u64().unwrap_or(0) + 1);
        if ext == "umap" && maps.len() < 200 {
            if let Ok(relative) = path.with_extension("").strip_prefix(base) {
                maps.push(format!("/Game/{}", relative.display()));
            }
        }
    }
}

fn setup_check(project: &Project, fix: bool) -> Result<Value> {
    let listed = project.descriptor["Plugins"].as_array().cloned().unwrap_or_default();
    let state = |name: &str| listed.iter().find(|p| p["Name"] == name).map(|p| p["Enabled"].as_bool().unwrap_or(false));
    let missing: Vec<&str> = BRIDGE_PLUGINS.iter().copied().filter(|name| state(name) != Some(true)).collect();
    let mut fixed = Vec::new();
    if fix && !missing.is_empty() {
        let mut descriptor = project.descriptor.clone();
        if !descriptor["Plugins"].is_array() {
            descriptor["Plugins"] = json!([]);
        }
        let plugins = descriptor["Plugins"].as_array_mut().expect("just made an array");
        for name in &missing {
            match plugins.iter_mut().find(|p| p["Name"] == *name) {
                Some(entry) => entry["Enabled"] = json!(true),
                None => plugins.push(json!({"Name": name, "Enabled": true})),
            }
            fixed.push(*name);
        }
        // Unreal writes descriptors with tabs; keeping that keeps the diff to the added lines.
        let mut out = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
        let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
        serde::Serialize::serialize(&descriptor, &mut serializer)?;
        out.extend_from_slice(b"\n");
        std::fs::write(&project.uproject, out).with_context(|| format!("writing {}", project.uproject.display()))?;
    }
    let editor = remote("GET", "/remote/info", None, Duration::from_secs(3));
    let python_ok = match &editor {
        Ok(_) => python("print('relay-python-ok')", Duration::from_secs(20))
            .map(|v| v.to_string().contains("relay-python-ok"))
            .map_err(|e| e.to_string()),
        Err(_) => Err("editor not reachable".into()),
    };
    let mut advice = Vec::new();
    if !missing.is_empty() && fixed.is_empty() {
        advice.push(format!("Enable {} (Edit > Plugins, or run ue_setup_check with fix=true) and restart the editor.", missing.join(", ")));
    }
    if !fixed.is_empty() {
        advice.push("The .uproject now lists the bridge plugins; the editor must be restarted to load them.".to_string());
    }
    if editor.is_err() {
        advice.push("Open the project in the editor and start the Remote Control web server: run `WebControl.StartServer` in the editor console, or turn on auto-start under Project Settings > Plugins > Remote Control. Set UE_REMOTE_CONTROL_URL if it is not on 127.0.0.1:30010.".to_string());
    } else if let Err(error) = &python_ok {
        advice.push(format!("Remote Python failed ({error}). Make sure the Python Editor Script Plugin is enabled and that Project Settings > Plugins > Remote Control allows remote Python execution."));
    }
    Ok(json!({
        "uproject": project.uproject,
        "bridge_plugins": BRIDGE_PLUGINS.iter().map(|name| json!({"name": name, "enabled": state(name) == Some(true) || fixed.contains(name)})).collect::<Vec<_>>(),
        "added_to_uproject": fixed,
        "remote_control_url": remote_base(),
        "editor_reachable": editor.is_ok(),
        "editor_error": editor.err().map(|e| e.to_string()),
        "remote_python": python_ok.is_ok_and(|ok| ok),
        "ready": missing.is_empty() && advice.is_empty(),
        "advice": advice,
    }))
}

fn build(project: &Project, args: &Value) -> Result<Value> {
    let engine = engine_root(project)?;
    let script = build_script(&engine);
    anyhow::ensure!(script.is_file(), "{} does not exist", script.display());
    let targets = names_with_suffix(&project.root.join("Source"), ".Target.cs", 0);
    let target = match args["target"].as_str().filter(|t| !t.is_empty()) {
        Some(target) => target.to_string(),
        None => targets.iter().find(|t| t.ends_with("Editor")).cloned().unwrap_or_else(|| format!("{}Editor", project.name)),
    };
    let platform = args["platform"].as_str().filter(|p| !p.is_empty()).unwrap_or(host_platform()).to_string();
    let configuration = args["configuration"].as_str().filter(|c| !c.is_empty()).unwrap_or("Development").to_string();
    let mut command = Command::new(&script);
    command
        .arg(&target)
        .arg(&platform)
        .arg(&configuration)
        .arg(format!("-Project={}", project.uproject.display()))
        .arg("-WaitMutex")
        .arg("-FromMsBuild")
        .current_dir(&engine);
    let started = std::time::Instant::now();
    let output = relay_core::proc::output_with_timeout(&mut command, secs(args, "timeout_s", 3600))
        .with_context(|| format!("running {}", script.display()))?;
    let Some(output) = output else {
        bail!("the build did not finish within its timeout and was stopped");
    };
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let errors: Vec<&str> = text
        .lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.contains("error") && !lower.contains("0 error") || lower.contains("undefined reference") || lower.contains("fatal")
        })
        .take(200)
        .collect();
    Ok(json!({
        "success": output.status.success(),
        "exit_code": output.status.code(),
        "command": format!("{} {target} {platform} {configuration} -Project=\"{}\" -WaitMutex -FromMsBuild", script.display(), project.uproject.display()),
        "seconds": started.elapsed().as_secs(),
        "errors": errors,
        "tail": tail(&text, 150),
        "hint": if output.status.success() { Value::Null } else { json!("Fix the first error first; later ones often cascade. A 'Unable to build while Live Coding is active' error means the editor is open: close it or build from Live Coding.") },
    }))
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let mut out = all[all.len().saturating_sub(lines)..].join("\n");
    if out.len() > MAX_OUTPUT {
        let mut cut = out.len() - MAX_OUTPUT;
        while !out.is_char_boundary(cut) {
            cut += 1;
        }
        out = out[cut..].to_string();
    }
    out
}

fn log(project: &Project, args: &Value) -> Result<Value> {
    let path = match args["file"].as_str().filter(|f| !f.is_empty()) {
        Some(file) => {
            anyhow::ensure!(!file.contains("..") && !file.starts_with('/'), "file must be a name under Saved/Logs");
            project.root.join("Saved/Logs").join(file)
        }
        None => project.log_path(),
    };
    let bytes = std::fs::read(&path).with_context(|| format!("reading {} — has the editor or game run yet?", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let lines = args["lines"].as_u64().unwrap_or(200).clamp(1, 5000) as usize;
    let filter = match args["filter"].as_str().filter(|f| !f.is_empty()) {
        Some(pattern) => Some(regex::Regex::new(pattern).with_context(|| format!("bad filter regex {pattern:?}"))?),
        None => None,
    };
    let kept: Vec<&str> = text.lines().filter(|line| filter.as_ref().is_none_or(|re| re.is_match(line))).collect();
    let total = kept.len();
    Ok(json!({
        "file": path,
        "matching_lines": total,
        "lines": tail(&kept.join("\n"), lines),
    }))
}

// ---------------------------------------------------------------- the running editor

fn remote_base() -> String {
    std::env::var("UE_REMOTE_CONTROL_URL").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| DEFAULT_REMOTE.to_string())
}

fn editor_status() -> Result<Value> {
    match remote("GET", "/remote/info", None, Duration::from_secs(3)) {
        Ok(info) => {
            let routes = info["HttpRoutes"].as_array().map(|r| r.len());
            Ok(json!({"reachable": true, "url": remote_base(), "routes": routes, "info": info}))
        }
        Err(error) => Ok(json!({
            "reachable": false,
            "url": remote_base(),
            "error": format!("{error:#}"),
            "advice": "Open the project in the editor, then run `WebControl.StartServer` in its console (or enable auto-start in Project Settings > Plugins > Remote Control). Run ue_setup_check for the full picture."
        })),
    }
}

/// Run editor Python through `ExecutePythonCommandEx`, which returns the command's log output
/// alongside its result instead of only a success flag.
fn python(code: &str, timeout: Duration) -> Result<Value> {
    let body = json!({
        "objectPath": PYTHON_LIBRARY,
        "functionName": "ExecutePythonCommandEx",
        "parameters": {"PythonCommand": code, "ExecutionMode": "ExecuteFile", "FileExecutionScope": "Private"},
        "generateTransaction": true,
    });
    let response = remote("PUT", "/remote/object/call", Some(&body), timeout)?;
    let output: Vec<String> = response["LogOutput"]
        .as_array()
        .map(|entries| entries.iter().map(|e| {
            let kind = e["Type"].as_str().unwrap_or("Info");
            let text = e["Output"].as_str().unwrap_or("").trim_end();
            if kind == "Info" { text.to_string() } else { format!("[{kind}] {text}") }
        }).collect())
        .unwrap_or_default();
    let ok = response["ReturnValue"].as_bool().unwrap_or(false);
    let result = json!({
        "ok": ok,
        "output": tail(&output.join("\n"), 2000),
        "result": response["CommandResult"],
    });
    if !ok {
        bail!("Python failed:\n{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(result)
}

/// Run a script that prints one line `RELAY_JSON:<json>` and return that JSON.
fn python_json(code: &str, timeout: Duration) -> Result<Value> {
    let result = python(code, timeout)?;
    let output = result["output"].as_str().unwrap_or("");
    let line = output
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("RELAY_JSON:"))
        .ok_or_else(|| anyhow!("the editor script printed no result:\n{output}"))?;
    Ok(serde_json::from_str(line)?)
}

fn search_assets_script(args: &Value) -> String {
    let query = args["query"].as_str().unwrap_or("");
    let classes: Vec<String> = args["class_names"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let mut paths: Vec<String> = args["package_paths"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if paths.is_empty() {
        paths.push("/Game".into());
    }
    let limit = args["limit"].as_u64().unwrap_or(100).clamp(1, 2000);
    format!(r#"import unreal, json
query = {query}.lower()
classes = set(c.lower() for c in {classes})
registry = unreal.AssetRegistryHelpers.get_asset_registry()
found = []
for root in {paths}:
    for data in registry.get_assets_by_path(root, recursive=True):
        cls_path = getattr(data, "asset_class_path", None)
        cls = str(cls_path.asset_name) if cls_path is not None else str(data.asset_class)
        name = str(data.asset_name)
        if query and query not in name.lower():
            continue
        if classes and cls.lower() not in classes:
            continue
        found.append({{"name": name, "class": cls, "package": str(data.package_name), "object_path": str(data.package_name) + "." + name}})
        if len(found) >= {limit}:
            break
    if len(found) >= {limit}:
        break
print("RELAY_JSON:" + json.dumps({{"count": len(found), "limit": {limit}, "assets": found}}))
"#, query = py_str(query), classes = serde_json::to_string(&classes).unwrap(), paths = serde_json::to_string(&paths).unwrap())
}

fn level_actors_script(args: &Value) -> String {
    let class_filter = args["class_filter"].as_str().unwrap_or("");
    let name_filter = args["name_filter"].as_str().unwrap_or("");
    let selected = if args["selected_only"].as_bool().unwrap_or(false) { "True" } else { "False" };
    let limit = args["limit"].as_u64().unwrap_or(200).clamp(1, 5000);
    format!(r#"import unreal, json
actors_sub = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
actors = actors_sub.get_selected_level_actors() if {selected} else actors_sub.get_all_level_actors()
cf = {class_filter}.lower()
nf = {name_filter}.lower()
out = []
for actor in actors:
    cls = actor.get_class().get_name()
    label = actor.get_actor_label()
    if cf and cf not in cls.lower():
        continue
    if nf and nf not in label.lower():
        continue
    loc = actor.get_actor_location()
    out.append({{"label": label, "class": cls, "path": actor.get_path_name(), "folder": str(actor.get_folder_path()), "location": [round(loc.x, 2), round(loc.y, 2), round(loc.z, 2)]}})
    if len(out) >= {limit}:
        break
world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
print("RELAY_JSON:" + json.dumps({{"level": world.get_path_name() if world else None, "total_in_level": len(actors), "count": len(out), "actors": out}}))
"#, class_filter = py_str(class_filter), name_filter = py_str(name_filter))
}

/// One HTTP/1.1 request to the Remote Control server. It is plain HTTP on localhost, so a
/// small client is all this needs; the reply is JSON.
fn remote(method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value> {
    remote_at(&remote_base(), method, path, body, timeout)
}

fn remote_at(base: &str, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value> {
    let rest = base.strip_prefix("http://").ok_or_else(|| anyhow!("UE_REMOTE_CONTROL_URL must be an http:// URL, got {base}"))?;
    let host = rest.trim_end_matches('/').split('/').next().unwrap_or(rest).to_string();
    let address = if host.contains(':') { host.clone() } else { format!("{host}:80") };
    let socket = address
        .to_socket_addrs()
        .with_context(|| format!("resolving {address}"))?
        .next()
        .ok_or_else(|| anyhow!("{address} resolves to nothing"))?;
    let mut stream = TcpStream::connect_timeout(&socket, Duration::from_secs(3))
        .with_context(|| format!("the Unreal editor is not answering at {base} (Remote Control web server not running?)"))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let payload = body.map(serde_json::to_vec).transpose()?.unwrap_or_default();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n",
        payload.len()
    );
    if body.is_some() {
        request.push_str("Content-Type: application/json\r\n");
    }
    if let Ok(passphrase) = std::env::var("UE_REMOTE_CONTROL_PASSPHRASE") {
        request.push_str(&format!("Passphrase: {passphrase}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes())?;
    stream.write_all(&payload)?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).context("reading the editor's reply (a long editor operation may need a larger timeout)")?;
    let (status, body) = parse_response(&raw)?;
    let text = String::from_utf8_lossy(&body).into_owned();
    let value: Value = if text.trim().is_empty() { json!({}) } else { serde_json::from_str(&text).unwrap_or(json!({"body": text})) };
    if !(200..300).contains(&status) {
        bail!("Remote Control answered HTTP {status}: {}", serde_json::to_string(&value)?);
    }
    Ok(value)
}

fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>)> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| anyhow!("malformed HTTP reply"))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut body = raw[split + 4..].to_vec();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| anyhow!("malformed HTTP status line"))?;
    let chunked = head.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("transfer-encoding:") && lower.contains("chunked")
    });
    if chunked {
        body = dechunk(&body)?;
    }
    Ok((status, body))
}

fn dechunk(mut raw: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = raw.windows(2).position(|w| w == b"\r\n").ok_or_else(|| anyhow!("malformed chunk"))?;
        let size_text = String::from_utf8_lossy(&raw[..end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)?;
        raw = &raw[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        anyhow::ensure!(raw.len() >= size, "truncated chunk");
        out.extend_from_slice(&raw[..size]);
        raw = raw.get(size + 2..).unwrap_or(&[]);
    }
}

fn tool_result(value: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    let mut result = json!({"content":[{"type":"text","text":text}],"isError":is_error});
    if !is_error {
        result["structuredContent"] = value;
    }
    result
}

fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code":code,"message":message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc":"2.0","id":id,"error":error})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn project(dir: &Path, name: &str, descriptor: Value) {
        std::fs::write(dir.join(format!("{name}.uproject")), serde_json::to_string_pretty(&descriptor).unwrap()).unwrap();
    }

    #[test]
    fn every_tool_in_the_manifest_is_served_and_nothing_else() {
        let manifest: Value = serde_json::from_str(include_str!("../../../plugins/unreal-engine/plugin.json")).unwrap();
        let declared: Vec<&str> = manifest["mcp_servers"][0]["tools"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        let served: Vec<Value> = tools();
        let served: Vec<&str> = served.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(declared, served);
    }

    #[test]
    fn initialize_and_list_speak_mcp() {
        let init = handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "unreal");
        let listed = handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 11);
        assert!(handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        let unknown = handle(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nope"}})).unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
    }

    #[test]
    fn project_discovery_skips_generated_folders_and_reads_the_layout() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Intermediate/Stale")).unwrap();
        project(&root.path().join("Intermediate/Stale"), "Wrong", json!({}));
        let game = root.path().join("game");
        std::fs::create_dir_all(game.join("Source/Shooter")).unwrap();
        std::fs::create_dir_all(game.join("Content/Maps")).unwrap();
        std::fs::create_dir_all(game.join("Config")).unwrap();
        project(&game, "Shooter", json!({"FileVersion":3,"EngineAssociation":"5.4","Modules":[{"Name":"Shooter","Type":"Runtime"}]}));
        std::fs::write(game.join("Source/ShooterEditor.Target.cs"), "").unwrap();
        std::fs::write(game.join("Source/Shooter.Target.cs"), "").unwrap();
        std::fs::write(game.join("Source/Shooter/Shooter.Build.cs"), "").unwrap();
        std::fs::write(game.join("Content/Maps/Arena.umap"), "").unwrap();
        std::fs::write(game.join("Config/DefaultEngine.ini"), "").unwrap();
        assert_eq!(find_uproject(root.path()).unwrap(), game.join("Shooter.uproject"));

        let found = Project { uproject: game.join("Shooter.uproject"), root: game.clone(), name: "Shooter".into(), descriptor: json!({"EngineAssociation":"5.4"}) };
        let info = project_info(&found).unwrap();
        assert_eq!(info["targets"], json!(["Shooter", "ShooterEditor"]));
        assert_eq!(info["cpp_modules"], json!(["Shooter"]));
        assert_eq!(info["maps"], json!(["/Game/Maps/Arena"]));
        assert_eq!(info["config_files"], json!(["DefaultEngine"]));
        assert_eq!(info["blueprint_only"], false);
    }

    #[test]
    fn setup_fix_adds_only_the_missing_bridge_plugins() {
        let root = tempfile::tempdir().unwrap();
        project(root.path(), "Game", json!({"FileVersion":3,"Plugins":[{"Name":"PythonScriptPlugin","Enabled":true},{"Name":"RemoteControl","Enabled":false}]}));
        let found = Project {
            uproject: root.path().join("Game.uproject"),
            root: root.path().to_path_buf(),
            name: "Game".into(),
            descriptor: serde_json::from_str(&std::fs::read_to_string(root.path().join("Game.uproject")).unwrap()).unwrap(),
        };
        // SAFETY: the only test that reads this variable; it points the live check at a closed
        // port so the test never reaches a real editor.
        unsafe { std::env::set_var("UE_REMOTE_CONTROL_URL", "http://127.0.0.1:9") };
        let report = setup_check(&found, true).unwrap();
        assert_eq!(report["added_to_uproject"], json!(["RemoteControl", "EditorScriptingUtilities"]));
        assert_eq!(report["editor_reachable"], false);
        let written: Value = serde_json::from_str(&std::fs::read_to_string(root.path().join("Game.uproject")).unwrap()).unwrap();
        let plugins = written["Plugins"].as_array().unwrap();
        assert_eq!(plugins.len(), 3);
        assert!(plugins.iter().all(|p| p["Enabled"] == true));
        assert_eq!(written["FileVersion"], 3, "other fields survive");
    }

    #[test]
    fn remote_calls_reach_a_remote_control_server_and_decode_chunked_replies() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = vec![0u8; 8192];
            let mut request = Vec::new();
            loop {
                let n = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(split) = text.find("\r\n\r\n") {
                    let length: usize = text.lines().find_map(|l| l.strip_prefix("Content-Length: ")).unwrap().trim().parse().unwrap();
                    if request.len() >= split + 4 + length { break; }
                }
            }
            let body = r#"{"ReturnValue":true,"CommandResult":"None","LogOutput":[{"Type":"Info","Output":"RELAY_JSON:{\"count\":1}"}]}"#;
            let reply = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        let url = format!("http://127.0.0.1:{port}");
        let body = json!({"objectPath": PYTHON_LIBRARY, "functionName": "ExecutePythonCommandEx", "parameters": {"PythonCommand": "print(1)"}});
        let result = remote_at(&url, "PUT", "/remote/object/call", Some(&body), Duration::from_secs(5)).unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("PUT /remote/object/call HTTP/1.1\r\n"));
        assert!(request.contains("ExecutePythonCommandEx"));
        assert_eq!(result["ReturnValue"], true);
        assert_eq!(result["LogOutput"][0]["Output"], "RELAY_JSON:{\"count\":1}");
    }

    #[test]
    fn python_scripts_embed_arguments_as_literals() {
        let script = level_actors_script(&json!({"name_filter":"it's \"quoted\"\n","limit":5}));
        assert!(script.contains(r#"nf = "it's \"quoted\"\n".lower()"#));
        assert!(script.contains("if len(out) >= 5:"));
        let search = search_assets_script(&json!({"class_names":["StaticMesh"]}));
        assert!(search.contains(r#"for root in ["/Game"]:"#));
        assert!(search.contains(r#"set(c.lower() for c in ["StaticMesh"])"#));
    }
}
