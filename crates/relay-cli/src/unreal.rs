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
/// Tools that act on the running editor. Each first checks the editor has this checkout's
/// project open: an agent in a worktree would otherwise edit assets in a different copy.
/// `ue_editor_lock` is not one: the lock is a file, and it must be readable and releasable
/// while the editor is down (after a crash, most of all).
const LIVE: [&str; 15] = ["ue_python", "ue_call", "ue_property", "ue_search_assets", "ue_level_actors", "ue_console",
    "ue_screenshot", "ue_anim_inspect", "ue_anim_preview", "ue_play", "ue_blueprint_info",
    "ue_asset_audit", "ue_asset_refs", "ue_data_table", "ue_profile"];
/// Live tools that change editor state, and so need the editor lock.
const MUTATING: [&str; 10] = ["ue_python", "ue_call", "ue_console", "ue_screenshot", "ue_anim_preview", "ue_property",
    "ue_play", "ue_profile", "ue_data_table", "ue_blueprint_info"];
/// A lock nobody has used for this long is free to take.
const LOCK_IDLE: Duration = Duration::from_secs(15 * 60);
/// A lock-file guard older than this was left by a process that died inside the few
/// microseconds it is held, and may be broken.
const GUARD_STALE: Duration = Duration::from_secs(10);
/// A tool result whose JSON is larger than this is written to a file; the reply carries the
/// path and a preview. MCP clients cap what they put in context well below what a Data Table
/// export or a folder of Blueprints can produce.
const MAX_INLINE_RESULT: usize = 200_000;
/// How much of a log `ue_log` reads back from its end at most, looking for enough lines.
const MAX_LOG_SCAN: u64 = 64 << 20;
/// Images one call may return; each is a full PNG in the agent's context.
const MAX_IMAGES: usize = 16;

const PY_COMMON: &str = include_str!("unreal_py/common.py");
const PY_PROJECT_CHECK: &str = include_str!("unreal_py/project_check.py");
const PY_CAPTURE: &str = include_str!("unreal_py/capture.py");
const PY_ANIM_INSPECT: &str = include_str!("unreal_py/anim_inspect.py");
const PY_ANIM_PREVIEW: &str = include_str!("unreal_py/anim_preview.py");
const PY_PLAY: &str = include_str!("unreal_py/play.py");
const PY_BLUEPRINT_INFO: &str = include_str!("unreal_py/blueprint_info.py");
const PY_ASSET_AUDIT: &str = include_str!("unreal_py/asset_audit.py");
const PY_ASSET_REFS: &str = include_str!("unreal_py/asset_refs.py");
const PY_DATA_TABLE: &str = include_str!("unreal_py/data_table.py");
const PY_IMPORT_FBX: &str = include_str!("unreal_py/import_fbx.py");

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

pub(crate) fn tool(name: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
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
                "timeout_s":{"type":"integer","minimum":5,"maximum":1800,"description":"Default 120"},
                "transaction":{"type":"boolean","description":"Wrap the whole call in one undo transaction (default true); pass false for read-only queries so they stay out of the editor's undo history"}
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
        tool("ue_screenshot",
            "Look at the level: render PNG images you can see. Frame one or more actors from named views (front/back/left/right/top/three_quarter, relative to the first actor's facing), or use an explicit camera, or the current editor viewport when neither is given. isolate=true renders only the framed actors on black, which makes silhouettes, hands and attachments easy to judge.",
            json!({
                "actors":{"type":"array","items":{"type":"string"},"description":"Actor labels or paths to frame"},
                "views":{"type":"array","items":{"type":"string","enum":["front","back","left","right","top","three_quarter","three_quarter_left"]},"description":"Default front, right, three_quarter"},
                "camera":{"type":"object","properties":{"location":{"type":"array","items":{"type":"number"}},"rotation":{"type":"array","items":{"type":"number"},"description":"[pitch, yaw, roll]"}}},
                "forward":{"type":"array","items":{"type":"number"},"description":"Override the facing used for named views"},
                "isolate":{"type":"boolean"},
                "width":{"type":"integer","minimum":64,"maximum":1920},
                "height":{"type":"integer","minimum":64,"maximum":1080},
                "fov":{"type":"number","minimum":10,"maximum":120}
            }), &[], false),
        tool("ue_anim_inspect",
            "Measure an animation instead of eyeballing it. Poses a skeletal mesh at sample times straight from the animation data and reports, in the character's own frame ([forward, right, up] cm, left/right detected from the skeleton's bone pairs): where tracked bones, sockets and attached items are and which side they are on; grip distances between item sockets and hands; clearance of attached items to the body and of hands, feet, head and items to a partner character; expected contacts (touch or stay apart); feet below ground. Works for any skeleton and any item (weapon, tool, shield, prop, bag, instrument). Returns problems[] and passed.",
            json!({
                "mesh":{"type":"string","description":"Skeletal mesh asset path"},
                "animation":{"type":"string","description":"Animation sequence path; omit for the reference pose"},
                "times":{"type":"array","items":{"type":"number"}},
                "samples":{"type":"integer","minimum":2,"maximum":60,"description":"Evenly spaced samples when times is omitted, default 9"},
                "track":{"type":"array","items":{"type":"string"},"description":"Points to report: bone or socket, partner:<bone>, item:<name>:<socket|end_a|end_b|origin|center>"},
                "attachments":{"type":"array","items":{"type":"object","properties":{
                    "name":{"type":"string"},"mesh":{"type":"string","description":"Static or skeletal mesh of the item"},
                    "socket":{"type":"string","description":"Character bone or socket it hangs from"},
                    "location":{"type":"array","items":{"type":"number"}},"rotation":{"type":"array","items":{"type":"number"},"description":"[pitch, yaw, roll] offset"},
                    "grips":{"type":"array","items":{"type":"object","properties":{"socket":{"type":"string"},"bone":{"type":"string"},"tolerance":{"type":"number"}}}}
                },"required":["name","mesh","socket"]}},
                "partner":{"type":"object","properties":{
                    "mesh":{"type":"string"},"animation":{"type":"string"},
                    "location":{"type":"array","items":{"type":"number"},"description":"In this character's mesh space, cm"},
                    "yaw":{"type":"number","description":"Degrees, default 180 (facing back)"},
                    "time_offset":{"type":"number"}
                },"required":["mesh"]},
                "contacts":{"type":"array","items":{"type":"object","properties":{
                    "a":{"type":"string"},"b":{"type":"string"},"expect":{"type":"string","enum":["touch","apart"]},
                    "distance":{"type":"number"},"window":{"type":"array","items":{"type":"number"},"description":"[start, end] seconds"}
                },"required":["a","b"]}},
                "body_radius":{"type":"number","description":"Body thickness around bones for clipping, default 8 cm"},
                "touch_distance":{"type":"number","description":"Default 5 cm"}
            }), &["mesh"], true),
        tool("ue_anim_preview",
            "See an animation: spawns a temporary copy of the character (with attached items and an optional partner, same arguments as ue_anim_inspect) in the open level, poses it at each sample time and returns images from the chosen views. Removes the preview actors afterwards; the level is left marked modified, so do not save the map because of it.",
            json!({
                "mesh":{"type":"string"},"animation":{"type":"string"},
                "times":{"type":"array","items":{"type":"number"}},
                "samples":{"type":"integer","minimum":1,"maximum":8,"description":"Default 4"},
                "attachments":{"type":"array","items":{"type":"object"}},
                "partner":{"type":"object"},
                "views":{"type":"array","items":{"type":"string"},"description":"Default front and right"},
                "location":{"type":"array","items":{"type":"number"},"description":"Where to spawn; default 6 m in front of the editor camera"},
                "isolate":{"type":"boolean","description":"Render only the preview actors (default true)"},
                "settle_ms":{"type":"integer","minimum":50,"maximum":5000,"description":"Wait for the editor to apply each pose, default 400"},
                "width":{"type":"integer","minimum":64,"maximum":1280},"height":{"type":"integer","minimum":64,"maximum":1080}
            }), &["mesh"], false),
        tool("ue_play",
            "Play the game in the editor and watch it: starts Play In Editor (or Simulate), waits for it to run, takes in-game screenshots and runs your Python probe (with `world` = the game world) at checkpoints, then stops and returns the screenshots, probe output and the log lines (errors, warnings, ensures, 'Accessed None') produced while playing. Use it to check gameplay, runtime IK and anything that only happens at runtime.",
            json!({
                "mode":{"type":"string","enum":["pie","simulate"],"description":"Default pie"},
                "seconds":{"type":"number","minimum":1,"maximum":300,"description":"How long to play, default 5"},
                "checkpoints":{"type":"array","items":{"type":"number"},"description":"Seconds after start to screenshot and probe; default the end"},
                "screenshots":{"type":"boolean","description":"Default true"},
                "width":{"type":"integer"},"height":{"type":"integer"},
                "console":{"type":"array","items":{"type":"string"},"description":"Console commands to run right after play starts, e.g. cheats or 'slomo 0.5'"},
                "probe":{"type":"string","description":"Python run at each checkpoint with `unreal` and `world` (the game world); print what you want to see"},
                "log_filter":{"type":"string","description":"Regex over new log lines; default errors, warnings, ensures and Blueprint runtime errors"},
                "stop":{"type":"boolean","description":"Stop at the end (default true)"},
                "stop_existing":{"type":"boolean","description":"End a session that is already running first"}
            }), &[], false),
        tool("ue_blueprint_info",
            "Read Blueprints as text: parent class, interfaces, variables with default values, functions and events, components, and graph nodes where this engine version exposes them. compile=true compiles them and returns the compiler's log lines. Give paths, or a folder to read every Blueprint in it.",
            json!({
                "paths":{"type":"array","items":{"type":"string"}},
                "folder":{"type":"string"},
                "compile":{"type":"boolean"},
                "limit":{"type":"integer","minimum":1,"maximum":500,"description":"Default 50"}
            }), &[], true),
        tool("ue_asset_audit",
            "Find asset problems with facts: textures (not power of two, oversized, normal maps with the wrong compression, masks in sRGB, Never Stream, no mips), static meshes (no collision, dense without Nanite or LODs), references to missing assets, and redirectors. Each finding says how to fix it.",
            json!({
                "path":{"type":"string","description":"Folder, default /Game"},
                "checks":{"type":"array","items":{"type":"string","enum":["textures","meshes","references","redirectors"]}},
                "limit":{"type":"integer","minimum":1,"maximum":5000,"description":"Assets loaded per class, default 400"}
            }), &[], true),
        tool("ue_asset_refs",
            "What an asset depends on and what depends on it (up to 4 levels). Run it before renaming, moving or deleting anything.",
            json!({"path":{"type":"string"},"depth":{"type":"integer","minimum":1,"maximum":4}}), &["path"], true),
        tool("ue_data_table",
            "Data Tables as text. export writes the table to a CSV or JSON file in the checkout (and returns it); import fills the table from such a file and saves the asset. Keeps balancing, dialogue and loot data diffable.",
            json!({
                "action":{"type":"string","enum":["export","import"]},
                "path":{"type":"string","description":"Data Table asset path"},
                "file":{"type":"string","description":"File in the checkout, relative to the project root"},
                "format":{"type":"string","enum":["csv","json"]}
            }), &["action","path"], false),
        tool("ue_profile",
            "Measure performance: plays (or uses the running editor world with play=false), records Unreal's CSV profiler for the given seconds, and returns average, 95th percentile and worst frame, game thread, render thread and GPU times, plus the slowest frames.",
            json!({
                "seconds":{"type":"number","minimum":2,"maximum":120,"description":"Default 10"},
                "warmup":{"type":"number","minimum":0,"maximum":60,"description":"Seconds before recording, default 3"},
                "play":{"type":"boolean","description":"Start Play In Editor first, default true"},
                "console":{"type":"array","items":{"type":"string"}}
            }), &[], false),
        tool("ue_run_tests",
            "Run automation tests and return pass/fail per test with error messages. Headless by default (a separate editor process with no window; works while your editor is open); in_editor=true runs them in the open editor instead.",
            json!({
                "filter":{"type":"string","description":"Test path prefix, e.g. Project. or MyGame.Inventory"},
                "in_editor":{"type":"boolean"},
                "timeout_s":{"type":"integer","minimum":30,"maximum":7200}
            }), &["filter"], false),
        tool("ue_crash",
            "The most recent crash: error message, call stack and the last log lines, from Saved/Crashes.",
            json!({"index":{"type":"integer","minimum":0,"description":"0 = most recent"}}), &[], true),
        tool("ue_editor_lock",
            "Who is driving the editor. Live tools that change the editor take this lock automatically, so two agents never script the one editor at once; it frees itself after 15 idle minutes. action=release gives it up when you are done.",
            json!({"action":{"type":"string","enum":["status","release"]}}), &[], false),
    ]
}

fn call(name: &str, args: &Value) -> Result<Value> {
    if LIVE.contains(&name) {
        let project = Project::find()?;
        guard_project(&project)?;
        let writes = MUTATING.contains(&name)
            && (name != "ue_property" || args.get("value").is_some())
            && (name != "ue_data_table" || args["action"] == "import")
            && (name != "ue_blueprint_info" || args["compile"] == true);
        if writes {
            acquire_lock(&project, &holder_id())?;
        }
    }
    match name {
        "ue_project_info" => project_info(&Project::find()?),
        "ue_setup_check" => setup_check(&Project::find()?, args["fix"].as_bool().unwrap_or(false)),
        "ue_build" => build(&Project::find()?, args),
        "ue_log" => log(&Project::find()?, args),
        "ue_editor_status" => editor_status(),
        "ue_python" => {
            let code = args["code"].as_str().ok_or_else(|| anyhow!("code is required"))?;
            python(code, secs(args, "timeout_s", 120), args["transaction"].as_bool().unwrap_or(true))
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
        "ue_search_assets" => python_json(&search_assets_script(args), Duration::from_secs(120), false),
        "ue_level_actors" => python_json(&level_actors_script(args), Duration::from_secs(60), false),
        "ue_console" => {
            let command = required(args, "command")?;
            let code = format!(
                "import unreal\nworld = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()\nunreal.SystemLibrary.execute_console_command(world, {})\nprint('ran', {})\n",
                py_str(&command), py_str(&command)
            );
            python(&code, Duration::from_secs(60), true)
        }
        "ue_screenshot" => {
            let project = Project::find()?;
            let dir = capture_dir(&project)?;
            let mut script_args = args.clone();
            script_args["out_dir"] = json!(dir);
            script_args["prefix"] = json!("shot");
            let result = python_json(&script(PY_CAPTURE, &script_args), Duration::from_secs(120), true);
            with_images(result, &dir)
        }
        "ue_anim_inspect" => python_json(&script(PY_ANIM_INSPECT, args), Duration::from_secs(300), false),
        "ue_anim_preview" => anim_preview(&Project::find()?, args),
        "ue_play" => play(&Project::find()?, args),
        "ue_blueprint_info" => {
            let project = Project::find()?;
            let start = log_len(&project);
            let compile = args["compile"] == true;
            let mut value = python_json(&script(PY_BLUEPRINT_INFO, args), Duration::from_secs(600), compile)?;
            if args["compile"] == true {
                value["compiler_log"] = json!(log_since(&project, start, Some(r"LogBlueprint|Error|Warning")));
            }
            Ok(value)
        }
        "ue_asset_audit" => python_json(&script(PY_ASSET_AUDIT, args), Duration::from_secs(1800), false),
        "ue_asset_refs" => python_json(&script(PY_ASSET_REFS, args), Duration::from_secs(120), false),
        "ue_data_table" => data_table(&Project::find()?, args),
        "ue_profile" => profile(&Project::find()?, args),
        "ue_run_tests" => run_tests(&Project::find()?, args),
        "ue_crash" => crash(&Project::find()?, args["index"].as_u64().unwrap_or(0) as usize),
        "ue_editor_lock" => {
            let project = Project::find()?;
            if args["action"].as_str() == Some("release") {
                release_lock(&project, &holder_id())?;
            }
            Ok(json!({"lock": read_lock(&project), "you": holder_id()}))
        }
        other => bail!("unknown tool {other}"),
    }
}

/// Import an FBX into the editor's project and measure what arrived (used by the Blender
/// plugin's `blender_to_unreal`). Same project guard and editor lock as every live tool.
/// `uproject` is the project the caller named (a `.uproject`, or a folder to search); without
/// one, `UE_PROJECT` and then the usual search from the checkout decide.
pub(crate) fn import_fbx(fbx: &Path, mut args: Value, uproject: Option<&Path>) -> Result<Value> {
    let project = Project::find_from(uproject)?;
    guard_project(&project)?;
    acquire_lock(&project, &holder_id())?;
    args["fbx"] = json!(fbx);
    let start = log_len(&project);
    match python_json(&script(PY_IMPORT_FBX, &args), Duration::from_secs(600), true) {
        Ok(mut value) => {
            value["import_log"] = json!(log_since(&project, start, Some(r"LogFbx|Interchange|Error|Warning")));
            Ok(value)
        }
        Err(error) => bail!("{error:#}\n{}", log_since(&project, start, Some(r"LogFbx|Interchange|Error|Warning")).join("\n")),
    }
}

/// The `.uproject` an import from another tool goes to: `explicit` (a `.uproject` or a folder
/// to search), then `UE_PROJECT`, then the checkout. Checked before any slow export runs.
pub(crate) fn project_file(explicit: Option<&Path>) -> Result<PathBuf> {
    Project::find_from(explicit).map(|p| p.uproject).context(
        "cannot find the Unreal project to import into. If it is in a different checkout from the Blender files, pass uproject (its .uproject or its folder) or set UE_PROJECT for the Blender plugin"
    )
}

/// `ARGS_JSON` first, then the shared helpers, then the tool's own script.
fn script(body: &str, args: &Value) -> String {
    format!("ARGS_JSON = {}\n{PY_COMMON}\n{body}", py_str(&args.to_string()))
}

// ---------------------------------------------------------------- which editor, and who drives it

/// Refuse live tools when the editor has another project open. The common case is an agent in
/// a git worktree while the human's editor is on the main checkout: its C++ would land in one
/// copy and its asset edits in the other.
fn guard_project(project: &Project) -> Result<()> {
    if std::env::var("UE_ALLOW_PROJECT_MISMATCH").is_ok_and(|v| v == "1") {
        return Ok(());
    }
    let open = editor_project()?;
    if same_file(&open, &project.uproject) {
        return Ok(());
    }
    bail!(
        "the editor has {} open, but this agent works in {}. Changes made through the editor would land in a different copy of the project than your files. Ask the human to start this agent in the project's main checkout (the Unreal plugin does that by default for new agents), or to open this checkout's .uproject in the editor. UE_ALLOW_PROJECT_MISMATCH=1 overrides this check.",
        open.display(), project.uproject.display()
    )
}

fn editor_project() -> Result<PathBuf> {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(std::time::Instant, PathBuf)>> = Mutex::new(None);
    if let Some((at, path)) = CACHE.lock().unwrap().clone() {
        if at.elapsed() < Duration::from_secs(30) {
            return Ok(path);
        }
    }
    let result = python_json(&script(PY_PROJECT_CHECK, &json!({})), Duration::from_secs(20), false)?;
    let path = PathBuf::from(result["project"].as_str().ok_or_else(|| anyhow!("the editor reported no project"))?);
    *CACHE.lock().unwrap() = Some((std::time::Instant::now(), path.clone()));
    Ok(path)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn holder_id() -> String {
    std::env::var("RELAY_SESSION").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("pid {}", std::os::unix::process::parent_id()))
}

fn lock_path(project: &Project) -> PathBuf {
    project.root.join("Saved/Relay/editor-lock.json")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_lock(project: &Project) -> Value {
    let Ok(raw) = std::fs::read_to_string(lock_path(project)) else { return Value::Null };
    let mut lock: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    if let Some(last) = lock["last_used"].as_u64() {
        let idle = now_secs().saturating_sub(last);
        lock["idle_s"] = json!(idle);
        lock["expired"] = json!(idle >= LOCK_IDLE.as_secs());
    }
    lock
}

/// Hold `<lock>.guard` across the read-decide-write of the lock file. The guard is created
/// with `create_new` (O_EXCL), so of two agents deciding at the same moment one waits for the
/// other and then sees its lock. It is held for microseconds; one older than `GUARD_STALE`
/// was left by a process that died holding it and is broken.
fn with_lock_file<T>(project: &Project, body: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let path = lock_path(project);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let guard = path.with_extension("json.guard");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&guard) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let age = std::fs::metadata(&guard).and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok());
                if age.is_some_and(|age| age > GUARD_STALE) {
                    let _ = std::fs::remove_file(&guard);
                    continue;
                }
                anyhow::ensure!(std::time::Instant::now() < deadline, "{} has been held for 5 s; if no agent is using the editor, delete it", guard.display());
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error).with_context(|| format!("creating {}", guard.display())),
        }
    }
    let result = body(&path);
    let _ = std::fs::remove_file(&guard);
    result
}

/// Replace the lock file in one step, so a reader never sees half of it.
fn write_lock(path: &Path, lock: &Value) -> Result<()> {
    let temp = path.with_extension(format!("json.{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&temp, lock.to_string())?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

/// One agent drives the editor at a time. The lock is a file in the project's `Saved/`, which
/// every agent sharing the checkout sees and git ignores.
fn acquire_lock(project: &Project, me: &str) -> Result<()> {
    with_lock_file(project, |path| {
        let lock = read_lock(project);
        if let Some(holder) = lock["holder"].as_str() {
            if holder != me && lock["expired"] != true {
                bail!(
                    "the editor is being driven by {holder} (idle {}s). Do offline work (C++, config, data files) or wait; the lock frees itself after {} idle minutes, or when {holder} runs ue_editor_lock with action=release.",
                    lock["idle_s"].as_u64().unwrap_or(0), LOCK_IDLE.as_secs() / 60
                );
            }
        }
        let since = if lock["holder"].as_str() == Some(me) { lock["since"].as_u64().unwrap_or(now_secs()) } else { now_secs() };
        write_lock(path, &json!({"holder": me, "since": since, "last_used": now_secs()}))
    })
}

fn release_lock(project: &Project, me: &str) -> Result<()> {
    with_lock_file(project, |path| {
        let lock = read_lock(project);
        match lock["holder"].as_str() {
            Some(holder) if holder != me && lock["expired"] != true => bail!("the lock belongs to {holder}, not to you"),
            Some(_) => Ok(std::fs::remove_file(path)?),
            None => Ok(()),
        }
    })
}

// ---------------------------------------------------------------- playing, testing, measuring

fn log_len(project: &Project) -> u64 {
    std::fs::metadata(project.log_path()).map(|m| m.len()).unwrap_or(0)
}

/// Log lines written since byte offset `start`, optionally filtered, last 300 kept. Reads only
/// the bytes after `start`; editor logs reach hundreds of MB.
fn log_since(project: &Project, start: u64, filter: Option<&str>) -> Vec<String> {
    let re = filter.and_then(|f| regex::Regex::new(f).ok());
    let mut cursor = LogCursor { path: project.log_path(), offset: start, partial: Vec::new() };
    let mut lines = cursor.read_new(re.as_ref());
    lines.extend(cursor.rest().filter(|l| re.as_ref().is_none_or(|re| re.is_match(l))));
    lines[lines.len().saturating_sub(300)..].to_vec()
}

/// Follows a log file: each read returns only the complete lines appended since the last one.
/// A file shorter than the offset was rotated or truncated (the editor restarted), and is
/// read again from its start.
struct LogCursor {
    path: PathBuf,
    offset: u64,
    /// The unterminated last line of the previous read.
    partial: Vec<u8>,
}

impl LogCursor {
    fn read_new(&mut self, filter: Option<&regex::Regex>) -> Vec<String> {
        use std::io::{Seek, SeekFrom};
        let Ok(mut file) = std::fs::File::open(&self.path) else { return Vec::new() };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
        }
        let mut bytes = std::mem::take(&mut self.partial);
        let before = bytes.len();
        if file.seek(SeekFrom::Start(self.offset)).is_err() || file.read_to_end(&mut bytes).is_err() {
            bytes.truncate(before);
            self.partial = bytes;
            return Vec::new();
        }
        self.offset += (bytes.len() - before) as u64;
        let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        self.partial = bytes.split_off(complete);
        String::from_utf8_lossy(&bytes).lines().filter(|l| filter.is_none_or(|re| re.is_match(l))).map(str::to_string).collect()
    }

    /// The unterminated line held back so far, for a final read.
    fn rest(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.partial);
        (!rest.is_empty()).then(|| String::from_utf8_lossy(&rest).trim_end().to_string())
    }
}

fn play_step(action: &str, extra: Value) -> Result<Value> {
    let mut args = extra;
    args["action"] = json!(action);
    python_json(&script(PY_PLAY, &args), Duration::from_secs(60), action != "status")
}

fn wait_for_play(on: bool) -> Result<()> {
    for _ in 0..120 {
        if play_step("status", json!({}))?["in_play"] == on {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    bail!("the play session did not {} within 30 s", if on { "start" } else { "stop" })
}

fn pngs_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(pngs_under(&path));
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
            out.push(path);
        }
    }
    out
}

const DEFAULT_PLAY_LOG: &str = r"Error|Warning|[Ee]nsure|Accessed None|Script Msg|LogBlueprintUserMessages|Assertion";

fn play(project: &Project, args: &Value) -> Result<Value> {
    let seconds = args["seconds"].as_f64().unwrap_or(5.0).clamp(1.0, 300.0);
    let mut checkpoints: Vec<f64> = args["checkpoints"].as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).filter(|t| *t >= 0.0 && *t <= seconds).collect())
        .unwrap_or_else(|| vec![seconds]);
    checkpoints.sort_by(|a, b| a.partial_cmp(b).unwrap());
    checkpoints.truncate(12);
    let screenshots = args["screenshots"].as_bool().unwrap_or(true);
    let shots_dir = project.root.join("Saved/Screenshots");
    let before: std::collections::HashSet<PathBuf> = pngs_under(&shots_dir).into_iter().collect();
    if args["stop_existing"] == true {
        play_step("stop", json!({}))?;
        wait_for_play(false)?;
    }
    let log_start = log_len(project);
    let started = play_step("start", json!({"mode": args["mode"].as_str().unwrap_or("pie")}))?;
    wait_for_play(true)?;
    let t0 = std::time::Instant::now();
    let outcome = (|| -> Result<Vec<Value>> {
        if let Some(commands) = args["console"].as_array() {
            play_step("console", json!({"commands": commands}))?;
        }
        let mut probes = Vec::new();
        for at in &checkpoints {
            let wait = Duration::from_secs_f64(*at).saturating_sub(t0.elapsed());
            std::thread::sleep(wait);
            if screenshots {
                play_step("shot", json!({"width": args["width"].as_u64().unwrap_or(1280), "height": args["height"].as_u64().unwrap_or(720)}))?;
            }
            if let Some(code) = args["probe"].as_str() {
                let out = python(&format!("ARGS_JSON = {}\n{PY_COMMON}\nworld = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_game_world()\n{code}", py_str("{}")), Duration::from_secs(60), true);
                probes.push(match out {
                    Ok(v) => json!({"at": at, "output": v["output"]}),
                    Err(e) => json!({"at": at, "error": format!("{e:#}")}),
                });
            }
        }
        std::thread::sleep(Duration::from_secs_f64(seconds).saturating_sub(t0.elapsed()));
        Ok(probes)
    })();
    let stopped = if args["stop"].as_bool().unwrap_or(true) {
        play_step("stop", json!({})).and_then(|_| wait_for_play(false)).err().map(|e| format!("{e:#}"))
    } else {
        None
    };
    let probes = outcome?;
    // High-resolution screenshots are written a frame or two after the request.
    let mut shots = Vec::new();
    for _ in 0..20 {
        shots = pngs_under(&shots_dir).into_iter().filter(|p| !before.contains(p)).collect::<Vec<_>>();
        if !screenshots || shots.len() >= checkpoints.len() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    shots.sort();
    let filter = args["log_filter"].as_str().unwrap_or(DEFAULT_PLAY_LOG);
    let log = log_since(project, log_start, Some(filter));
    let errors = log.iter().filter(|l| l.contains("Error") || l.contains("Accessed None") || l.to_lowercase().contains("ensure")).count();
    Ok(json!({
        "mode": started["requested"],
        "played_s": t0.elapsed().as_secs_f64().min(seconds + 5.0),
        "checkpoints": checkpoints,
        "probes": probes,
        "log": log,
        "error_lines": errors,
        "stop_error": stopped,
        "files": shots.iter().enumerate().map(|(i, p)| json!({"view": format!("checkpoint {}", checkpoints.get(i).map(|t| format!("{t}s")).unwrap_or_default()), "file": p})).collect::<Vec<_>>(),
        "_keep_files": true,
    }))
    .map(|mut v| {
        // Screenshots live in the project's own folder; show them but leave them in place.
        let files = v["files"].clone();
        v["_images"] = json!(files.as_array().unwrap().iter().map(|f| json!({"label": f["view"], "path": f["file"]})).collect::<Vec<_>>());
        v.as_object_mut().unwrap().remove("_keep_files");
        v
    })
}

fn data_table(project: &Project, args: &Value) -> Result<Value> {
    let format = args["format"].as_str().unwrap_or("csv");
    let file = args["file"].as_str().map(|f| {
        anyhow::ensure!(!f.contains("..") && !Path::new(f).is_absolute(), "file must be relative to the project root");
        Ok(project.root.join(f))
    }).transpose()?;
    match args["action"].as_str() {
        Some("export") => {
            let mut value = python_json(&script(PY_DATA_TABLE, &json!({"action":"export","path":args["path"],"format":format})), Duration::from_secs(120), false)?;
            if let Some(file) = &file {
                if let Some(parent) = file.parent() { std::fs::create_dir_all(parent)?; }
                std::fs::write(file, value["text"].as_str().unwrap_or(""))?;
                value["written"] = json!(file);
            }
            Ok(value)
        }
        Some("import") => {
            let file = file.ok_or_else(|| anyhow!("import needs file"))?;
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let start = log_len(project);
            let result = python_json(&script(PY_DATA_TABLE, &json!({"action":"import","path":args["path"],"format":format,"text":text})), Duration::from_secs(120), true);
            match result {
                Ok(v) => Ok(v),
                Err(e) => bail!("{e:#}\n{}", log_since(project, start, Some("LogDataTable|Error|Warning")).join("\n")),
            }
        }
        _ => bail!("action must be export or import"),
    }
}

/// Averages, 95th percentiles and maxima of the timing columns of an Unreal CSV profile.
fn summarize_csv(text: &str) -> Value {
    let mut lines = text.lines();
    let Some(header) = lines.next() else { return json!({"error":"empty profile"}) };
    let columns: Vec<&str> = header.split(',').map(str::trim).collect();
    let wanted: Vec<usize> = columns.iter().enumerate()
        .filter(|(_, c)| ["FrameTime", "GameThreadTime", "RenderThreadTime", "GPUTime", "RHIThreadTime"].contains(c) || c.starts_with("GPU/") && c.len() < 40)
        .map(|(i, _)| i).collect();
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        if cells.len() != columns.len() {
            continue;
        }
        let values: Option<Vec<f64>> = wanted.iter().map(|i| cells[*i].trim().parse().ok()).collect();
        if let Some(values) = values { rows.push(values); }
    }
    let mut stats = Map::new();
    for (k, i) in wanted.iter().enumerate() {
        let mut v: Vec<f64> = rows.iter().map(|r| r[k]).collect();
        if v.is_empty() { continue; }
        let avg = v.iter().sum::<f64>() / v.len() as f64;
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = v[((v.len() as f64 * 0.95) as usize).min(v.len() - 1)];
        stats.insert(columns[*i].to_string(), json!({"avg_ms": (avg * 100.0).round() / 100.0, "p95_ms": (p95 * 100.0).round() / 100.0, "max_ms": (v[v.len()-1] * 100.0).round() / 100.0}));
    }
    let frame = columns.iter().position(|c| *c == "FrameTime").and_then(|i| wanted.iter().position(|w| *w == i));
    let mut worst: Vec<(usize, f64)> = frame.map(|k| rows.iter().enumerate().map(|(n, r)| (n, r[k])).collect()).unwrap_or_default();
    worst.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let fps = stats.get("FrameTime").and_then(|f| f["avg_ms"].as_f64()).filter(|ms| *ms > 0.0).map(|ms| (1000.0 / ms * 10.0).round() / 10.0);
    json!({"frames": rows.len(), "avg_fps": fps, "stats": stats,
        "worst_frames": worst.iter().take(5).map(|(n, ms)| json!({"frame": n, "ms": ms})).collect::<Vec<_>>()})
}

fn profile(project: &Project, args: &Value) -> Result<Value> {
    let seconds = args["seconds"].as_f64().unwrap_or(10.0).clamp(2.0, 120.0);
    let warmup = args["warmup"].as_f64().unwrap_or(3.0).clamp(0.0, 60.0);
    let play = args["play"].as_bool().unwrap_or(true);
    let dir = project.root.join("Saved/Profiling/CSV");
    let before: std::collections::HashSet<PathBuf> = std::fs::read_dir(&dir).map(|e| e.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    if play {
        play_step("start", json!({"mode":"pie"}))?;
        wait_for_play(true)?;
    }
    let run = (|| -> Result<()> {
        if let Some(commands) = args["console"].as_array() {
            play_step("console", json!({"commands": commands}))?;
        }
        std::thread::sleep(Duration::from_secs_f64(warmup));
        play_step("console", json!({"commands": ["csvprofile start"]}))?;
        std::thread::sleep(Duration::from_secs_f64(seconds));
        play_step("console", json!({"commands": ["csvprofile stop"]}))?;
        Ok(())
    })();
    if play {
        let _ = play_step("stop", json!({})).and_then(|_| wait_for_play(false));
    }
    run?;
    let mut file = None;
    for _ in 0..40 {
        file = std::fs::read_dir(&dir).ok().and_then(|e| e.flatten().map(|e| e.path())
            .filter(|p| !before.contains(p) && p.extension().is_some_and(|x| x == "csv")).max());
        if file.is_some() { break; }
        std::thread::sleep(Duration::from_millis(250));
    }
    let file = file.ok_or_else(|| anyhow!("no CSV profile appeared in {} (csvprofile needs a Development or Test build of the editor)", dir.display()))?;
    // The profiler finishes writing on a worker thread.
    std::thread::sleep(Duration::from_millis(750));
    let text = std::fs::read_to_string(&file)?;
    let mut summary = summarize_csv(&text);
    summary["file"] = json!(file);
    summary["recorded_s"] = json!(seconds);
    summary["budget_hint"] = json!("60 fps is 16.7 ms per frame, 30 fps is 33.3 ms. The largest of game thread, render thread and GPU is what limits the frame.");
    Ok(summary)
}

fn editor_cmd(engine: &Path) -> PathBuf {
    let bin = engine.join("Engine/Binaries");
    if cfg!(target_os = "windows") {
        bin.join("Win64/UnrealEditor-Cmd.exe")
    } else if cfg!(target_os = "macos") {
        bin.join("Mac/UnrealEditor-Cmd")
    } else {
        bin.join("Linux/UnrealEditor-Cmd")
    }
}

fn run_tests(project: &Project, args: &Value) -> Result<Value> {
    let filter = required(args, "filter")?;
    anyhow::ensure!(!filter.contains(';') && !filter.contains('"'), "filter must be a test path prefix");
    if args["in_editor"] == true {
        guard_project(project)?;
        acquire_lock(project, &holder_id())?;
        let mut cursor = LogCursor { path: project.log_path(), offset: log_len(project), partial: Vec::new() };
        play_step("console", json!({"commands": [format!("Automation RunTests {filter}")]}))?;
        let deadline = std::time::Instant::now() + secs(args, "timeout_s", 1800);
        let automation = regex::Regex::new("LogAutomation").unwrap();
        let mut lines = Vec::new();
        loop {
            // Only what the editor appended since the last second's read.
            lines.extend(cursor.read_new(Some(&automation)));
            if lines.iter().any(|l| l.contains("Automation Test Queue Empty") || l.contains("No automation tests matched")) {
                return Ok(test_lines(&lines));
            }
            if std::time::Instant::now() > deadline {
                bail!("tests did not finish before the timeout; partial results:\n{}", lines.join("\n"));
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let engine = engine_root(project)?;
    let cmd = editor_cmd(&engine);
    anyhow::ensure!(cmd.is_file(), "{} does not exist (build the editor, or use in_editor=true)", cmd.display());
    let report = project.root.join("Saved/Relay/TestReport").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&report)?;
    let mut command = Command::new(&cmd);
    command
        .arg(&project.uproject)
        .arg(format!("-ExecCmds=Automation RunTests {filter};Quit"))
        .arg(format!("-ReportExportPath={}", report.display()))
        .args(["-unattended", "-nopause", "-nosplash", "-nullrhi", "-NoSound", "-stdout", "-FullStdOutLogOutput"]);
    let started = std::time::Instant::now();
    let output = relay_core::proc::output_with_timeout(&mut command, secs(args, "timeout_s", 1800))?
        .ok_or_else(|| anyhow!("the test run did not finish within its timeout and was stopped"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let index = std::fs::read_to_string(report.join("index.json")).ok()
        .and_then(|raw| serde_json::from_str::<Value>(raw.trim_start_matches('\u{feff}')).ok());
    let mut value = match index {
        Some(index) => {
            let tests: Vec<Value> = index["tests"].as_array().cloned().unwrap_or_default().iter().map(|t| {
                let errors: Vec<Value> = t["entries"].as_array().cloned().unwrap_or_default().iter()
                    .filter(|e| e["event"]["type"] == "Error" || e["event"]["type"] == "Warning")
                    .map(|e| json!(format!("{}: {}", e["event"]["type"].as_str().unwrap_or(""), e["event"]["message"].as_str().unwrap_or(""))))
                    .take(20).collect();
                json!({"test": t["fullTestPath"], "state": t["state"], "messages": errors})
            }).collect();
            json!({"succeeded": index["succeeded"], "failed": index["failed"], "tests": tests})
        }
        None => test_lines(&stdout.lines().filter(|l| l.contains("LogAutomation")).map(str::to_string).collect::<Vec<_>>()),
    };
    value["exit_code"] = json!(output.status.code());
    value["seconds"] = json!(started.elapsed().as_secs());
    let _ = std::fs::remove_dir_all(&report);
    Ok(value)
}

/// Results from `LogAutomationController` lines: `Test Completed. Result={Success} Name={..} Path={..}`.
fn test_lines(lines: &[String]) -> Value {
    let re = regex::Regex::new(r"Result=\{(\w+)\}\s+Name=\{([^}]*)\}\s+Path=\{([^}]*)\}").unwrap();
    let tests: Vec<Value> = lines.iter().filter_map(|l| re.captures(l)).map(|c| json!({"test": &c[3], "name": &c[2], "state": &c[1]})).collect();
    let failed = tests.iter().filter(|t| t["state"] != "Success").count();
    json!({"succeeded": tests.len() - failed, "failed": failed, "tests": tests,
        "errors": lines.iter().filter(|l| l.contains("Error")).take(50).collect::<Vec<_>>()})
}

fn crash(project: &Project, index: usize) -> Result<Value> {
    let dir = project.root.join("Saved/Crashes");
    let mut crashes: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .with_context(|| format!("no crash reports in {}", dir.display()))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    crashes.sort_by(|a, b| b.0.cmp(&a.0));
    let (_, folder) = crashes.get(index).cloned().ok_or_else(|| anyhow!("there are {} crash reports", crashes.len()))?;
    let context = std::fs::read_to_string(folder.join("CrashContext.runtime-xml")).unwrap_or_default();
    let field = |name: &str| -> Option<String> {
        let re = regex::Regex::new(&format!(r"(?s)<{name}>(.*?)</{name}>")).ok()?;
        re.captures(&context).map(|c| c[1].trim().replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&").replace("&quot;", "\"").replace("&apos;", "'"))
    };
    let log = std::fs::read_dir(&folder).ok().and_then(|e| e.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "log")));
    let log_tail = log.and_then(|p| std::fs::read_to_string(p).ok()).map(|t| tail(&t, 80));
    Ok(json!({
        "folder": folder,
        "of": crashes.len(),
        "error": field("ErrorMessage"),
        "crash_type": field("CrashType"),
        "callstack": field("CallStack").map(|c| tail(&c, 60)),
        "engine_version": field("EngineVersion"),
        "build_configuration": field("BuildConfiguration"),
        "log_tail": log_tail,
        "hint": "Read the first frames of the call stack that are in your project's modules; engine frames below them are usually the consequence.",
    }))
}

// ---------------------------------------------------------------- seeing

fn capture_dir(project: &Project) -> Result<PathBuf> {
    let dir = project.root.join("Saved/Relay/Captures").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

/// Attach the PNGs a capture script wrote as `_images`, which the MCP layer turns into image
/// content, then delete the folder.
fn with_images(result: Result<Value>, dir: &Path) -> Result<Value> {
    let mut value = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_dir_all(dir);
            return Err(error);
        }
    };
    let mut images = Vec::new();
    for file in value["files"].as_array().cloned().unwrap_or_default() {
        let path = PathBuf::from(file["file"].as_str().unwrap_or(""));
        // Some engine versions append the extension themselves.
        let found = [path.clone(), path.with_extension("png.png")].into_iter().find(|p| p.is_file());
        match found {
            Some(found) => images.push(json!({"label": file["view"], "path": found})),
            None => images.push(json!({"label": file["view"], "missing": path})),
        }
    }
    value["_images"] = json!(images);
    value["_cleanup"] = json!(dir);
    Ok(value)
}

fn anim_preview(project: &Project, args: &Value) -> Result<Value> {
    let dir = capture_dir(project)?;
    let mut base = args.clone();
    if base.get("samples").is_none() && base.get("times").is_none() {
        base["samples"] = json!(4);
    }
    let run = |action: &str, extra: Value| -> Result<Value> {
        let mut a = base.clone();
        a["action"] = json!(action);
        if let (Some(target), Some(extra)) = (a.as_object_mut(), extra.as_object()) {
            for (k, v) in extra {
                target.insert(k.clone(), v.clone());
            }
        }
        python_json(&script(PY_ANIM_PREVIEW, &a), Duration::from_secs(120), action != "check")
    };
    let outcome = (|| -> Result<Value> {
        let setup = run("setup", json!({}))?;
        let times: Vec<f64> = setup["times"].as_array().cloned().unwrap_or_default().iter().filter_map(Value::as_f64).take(8).collect();
        let settle = Duration::from_millis(args["settle_ms"].as_u64().unwrap_or(400).clamp(50, 5000));
        let mut files = Vec::new();
        let mut poses = Vec::new();
        for (i, t) in times.iter().enumerate() {
            run("pose", json!({"time": t}))?;
            // The editor applies a pose on its next tick; check before taking the picture.
            let mut check = Value::Null;
            for _ in 0..8 {
                std::thread::sleep(settle);
                check = run("check", json!({"time": t}))?;
                if check["off_by_cm"].as_f64().unwrap_or(f64::MAX) <= 1.5 {
                    break;
                }
            }
            poses.push(json!({"time": t, "pose_applied": check["off_by_cm"].as_f64().unwrap_or(f64::MAX) <= 1.5, "off_by_cm": check["off_by_cm"]}));
            let capture_args = json!({
                "actors": setup["actors"], "forward": setup["forward"],
                "views": args.get("views").cloned().unwrap_or(json!(["front", "right"])),
                "isolate": args["isolate"].as_bool().unwrap_or(true),
                "width": args.get("width").cloned().unwrap_or(json!(480)),
                "height": args.get("height").cloned().unwrap_or(json!(480)),
                "out_dir": dir, "prefix": format!("t{i}_{t:.3}s"),
            });
            let shot = python_json(&script(PY_CAPTURE, &capture_args), Duration::from_secs(120), true)?;
            files.extend(shot["files"].as_array().cloned().unwrap_or_default());
        }
        let stale = poses.iter().any(|p| p["pose_applied"] == false);
        Ok(json!({
            "files": files,
            "poses": poses,
            "warning": if stale { json!("Some poses had not been applied when captured: the editor may be throttled in the background (Editor Preferences > General > Performance > Use Less CPU when in Background). Raise settle_ms or keep the editor focused, and trust ue_anim_inspect's numbers over these images.") } else { Value::Null },
        }))
    })();
    let cleanup = run("cleanup", json!({}));
    let mut value = with_images(outcome, &dir)?;
    if let Err(error) = cleanup {
        value["cleanup_error"] = json!(format!("{error:#}"));
    }
    Ok(value)
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
        Project::find_from(None)
    }

    /// `explicit` (a `.uproject`, or a folder to search) first, then `UE_PROJECT`, then a
    /// search from the checkout.
    fn find_from(explicit: Option<&Path>) -> Result<Project> {
        let explicit = match explicit {
            Some(path) if path.is_dir() => Some(find_uproject(path).ok_or_else(|| anyhow!("no .uproject within three folders of {}", path.display()))?),
            Some(path) => Some(path.to_path_buf()),
            None => None,
        };
        let uproject = match explicit.or_else(|| std::env::var_os("UE_PROJECT").map(PathBuf::from)) {
            Some(path) => path,
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
        Ok(_) => python("print('relay-python-ok')", Duration::from_secs(20), false)
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
    let lines = args["lines"].as_u64().unwrap_or(200).clamp(1, 5000) as usize;
    let filter = match args["filter"].as_str().filter(|f| !f.is_empty()) {
        Some(pattern) => Some(regex::Regex::new(pattern).with_context(|| format!("bad filter regex {pattern:?}"))?),
        None => None,
    };
    let scan = tail_lines(&path, lines, filter.as_ref(), MAX_LOG_SCAN)
        .with_context(|| format!("reading {} — has the editor or game run yet?", path.display()))?;
    Ok(json!({
        "file": path,
        "matching_lines": scan.matching,
        "whole_file_scanned": scan.from == 0,
        "scanned_from_byte": scan.from,
        "file_bytes": scan.len,
        "lines": tail(&scan.kept.join("\n"), lines),
    }))
}

struct LogTail {
    /// Matching lines in the part of the file that was read.
    kept: Vec<String>,
    matching: usize,
    from: u64,
    len: u64,
}

/// The last `want` lines matching `filter`, reading back from the end of the file in growing
/// windows (1 MB, then 4x) instead of reading all of it, and never more than `max_scan` bytes.
fn tail_lines(path: &Path, want: usize, filter: Option<&regex::Regex>, max_scan: u64) -> Result<LogTail> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut window: u64 = 1 << 20;
    loop {
        let from = len.saturating_sub(window.min(max_scan));
        file.seek(SeekFrom::Start(from))?;
        let mut bytes = Vec::with_capacity((len - from) as usize);
        (&mut file).take(len - from).read_to_end(&mut bytes)?;
        // A window that starts mid-file starts mid-line: drop that fragment.
        let start = if from == 0 { 0 } else { bytes.iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| i + 1) };
        let text = String::from_utf8_lossy(&bytes[start..]);
        let kept: Vec<String> = text.lines().filter(|l| filter.is_none_or(|re| re.is_match(l))).map(str::to_string).collect();
        if kept.len() >= want || from == 0 || window >= max_scan {
            let matching = kept.len();
            return Ok(LogTail { kept: kept[kept.len().saturating_sub(want)..].to_vec(), matching, from: from + start as u64, len });
        }
        window = window.saturating_mul(4);
    }
}

// ---------------------------------------------------------------- the running editor

fn remote_base() -> String {
    std::env::var("UE_REMOTE_CONTROL_URL").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| DEFAULT_REMOTE.to_string())
}

fn editor_status() -> Result<Value> {
    match remote("GET", "/remote/info", None, Duration::from_secs(3)) {
        Ok(info) => {
            let routes = info["HttpRoutes"].as_array().map(|r| r.len());
            // Which project is open matters as much as whether the editor answers.
            let open = editor_project().ok();
            let ours = Project::find().ok().map(|p| p.uproject);
            let matches = match (&open, &ours) {
                (Some(open), Some(ours)) => Some(same_file(open, ours)),
                _ => None,
            };
            let lock = Project::find().map(|p| read_lock(&p)).unwrap_or(Value::Null);
            Ok(json!({"reachable": true, "url": remote_base(), "editor_project": open, "this_checkout": ours,
                "same_project": matches, "editor_lock": lock, "routes": routes, "info": info}))
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
/// alongside its result instead of only a success flag. `transaction` wraps the call in an
/// undo transaction; read-only queries pass false so they stay out of the undo history.
/// Returns the whole output, untrimmed, and the command result.
fn python_raw(code: &str, timeout: Duration, transaction: bool) -> Result<(String, Value)> {
    let body = json!({
        "objectPath": PYTHON_LIBRARY,
        "functionName": "ExecutePythonCommandEx",
        "parameters": {"PythonCommand": code, "ExecutionMode": "ExecuteFile", "FileExecutionScope": "Private"},
        "generateTransaction": transaction,
    });
    let mut response = remote("PUT", "/remote/object/call", Some(&body), timeout)?;
    let output: Vec<String> = response["LogOutput"]
        .as_array()
        .map(|entries| entries.iter().map(|e| {
            let kind = e["Type"].as_str().unwrap_or("Info");
            let text = e["Output"].as_str().unwrap_or("").trim_end();
            if kind == "Info" { text.to_string() } else { format!("[{kind}] {text}") }
        }).collect())
        .unwrap_or_default();
    let output = output.join("\n");
    if response["ReturnValue"].as_bool() != Some(true) {
        let shown = json!({"ok": false, "output": shown_output(&output), "result": response["CommandResult"]});
        bail!("Python failed:\n{}", serde_json::to_string_pretty(&shown)?);
    }
    Ok((output, response["CommandResult"].take()))
}

fn python(code: &str, timeout: Duration, transaction: bool) -> Result<Value> {
    let (output, result) = python_raw(code, timeout, transaction)?;
    Ok(json!({"ok": true, "output": shown_output(&output), "result": result}))
}

/// What an agent is shown of editor output: the last 2000 lines within `MAX_OUTPUT` bytes,
/// with any long `RELAY_JSON:` result line reduced to its size (the result is returned
/// parsed, and one such line alone can exceed the whole budget).
fn shown_output(output: &str) -> String {
    let lines: Vec<std::borrow::Cow<str>> = output.lines().map(|line| match line.trim().strip_prefix("RELAY_JSON:") {
        Some(json) if json.len() > 2000 => format!("RELAY_JSON:<{} bytes>", json.len()).into(),
        _ => line.into(),
    }).collect();
    tail(&lines.join("\n"), 2000)
}

/// Run a script that prints one line `RELAY_JSON:<json>` and return that JSON.
fn python_json(code: &str, timeout: Duration, transaction: bool) -> Result<Value> {
    let (output, _) = python_raw(code, timeout, transaction)?;
    relay_json(&output)
}

/// The last `RELAY_JSON:` line of the full, untrimmed output. It is searched before anything
/// is cut to size: a Data Table export or an audit of a large folder prints one line of
/// hundreds of KB, and trimming first would cut off its prefix.
fn relay_json(output: &str) -> Result<Value> {
    let line = output
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("RELAY_JSON:"))
        .ok_or_else(|| anyhow!("the editor script printed no result:\n{}", shown_output(output)))?;
    serde_json::from_str(line).with_context(|| format!("the editor script's result ({} bytes) is not valid JSON", line.len()))
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
    let raw = read_response(&mut stream).context("reading the editor's reply (a long editor operation may need a larger timeout)")?;
    let (status, body) = parse_response(&raw)?;
    let text = String::from_utf8_lossy(&body).into_owned();
    let value: Value = if text.trim().is_empty() { json!({}) } else { serde_json::from_str(&text).unwrap_or(json!({"body": text})) };
    if !(200..300).contains(&status) {
        bail!("Remote Control answered HTTP {status}: {}", serde_json::to_string(&value)?);
    }
    Ok(value)
}

/// Read one HTTP reply and stop when it is complete: at `Content-Length` bytes of body, at the
/// last chunk of a chunked body, or else at end of stream. A server that keeps the connection
/// open despite `Connection: close` would otherwise hold every call until the read timeout.
fn read_response(stream: &mut impl Read) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut expected: Option<Option<usize>> = None; // None: head not yet read; Some(None): no length
    let mut chunked = false;
    loop {
        if expected.is_none() {
            if let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
                chunked = head.lines().any(|l| l.starts_with("transfer-encoding:") && l.contains("chunked"));
                let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
                let status = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|c| c.parse::<u16>().ok());
                // No body: 1xx, 204 and 304 replies.
                let bodiless = status.is_some_and(|s| (100..200).contains(&s) || s == 204 || s == 304);
                expected = Some(if chunked { None } else if bodiless { Some(split + 4) } else { length.map(|n| split + 4 + n) });
            }
        }
        match expected {
            Some(Some(total)) if raw.len() >= total => {
                raw.truncate(total);
                return Ok(raw);
            }
            Some(None) if chunked => {
                let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0);
                if chunked_complete(&raw[split + 4..]) {
                    return Ok(raw);
                }
            }
            _ => {}
        }
        let n = match stream.read(&mut buffer) {
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if n == 0 {
            return Ok(raw);
        }
        raw.extend_from_slice(&buffer[..n]);
    }
}

/// Whether a chunked body has reached its terminating zero-size chunk and the blank line
/// after its (optional) trailers.
fn chunked_complete(mut raw: &[u8]) -> bool {
    loop {
        let Some(end) = raw.windows(2).position(|w| w == b"\r\n") else { return false };
        let size_text = String::from_utf8_lossy(&raw[..end]);
        let Ok(size) = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16) else { return false };
        raw = &raw[end + 2..];
        if size == 0 {
            // Trailers, if any, end with an empty line.
            return raw.starts_with(b"\r\n") || raw.windows(4).any(|w| w == b"\r\n\r\n");
        }
        if raw.len() < size + 2 {
            return false;
        }
        raw = &raw[size + 2..];
    }
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

pub(crate) fn tool_result(mut value: Value, is_error: bool) -> Value {
    let images = value.as_object_mut().and_then(|o| o.remove("_images")).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let cleanup = value.as_object_mut().and_then(|o| o.remove("_cleanup"));
    let mut content = Vec::new();
    let mut shown = Vec::new();
    for image in images.iter().take(MAX_IMAGES) {
        let Some(path) = image["path"].as_str() else { continue };
        if let Ok(bytes) = std::fs::read(path) {
            use base64::Engine as _;
            content.push(json!({"type":"text","text":format!("Image: {}", image["label"].as_str().unwrap_or(""))}));
            content.push(json!({"type":"image","data":base64::engine::general_purpose::STANDARD.encode(bytes),"mimeType":"image/png"}));
            shown.push(image["label"].clone());
        }
    }
    if !images.is_empty() {
        value["images"] = json!({"shown": shown, "requested": images.len(), "limit": MAX_IMAGES});
    }
    if let Some(dir) = cleanup.as_ref().and_then(Value::as_str) {
        let _ = std::fs::remove_dir_all(dir);
    }
    let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
    if text.len() > MAX_INLINE_RESULT {
        if let Ok(summary) = spill_result(&value, &text) {
            value = summary;
            text = serde_json::to_string_pretty(&value).unwrap_or_default();
        }
    }
    content.insert(0, json!({"type":"text","text":text}));
    let mut result = json!({"content":content,"isError":is_error});
    if !is_error {
        result["structuredContent"] = value;
    }
    result
}

/// Where results too large to return inline are written: the checkout's `.relay/` (which
/// Relay keeps out of git) when running under Relay, the system temp folder otherwise.
fn results_dir() -> PathBuf {
    match std::env::var_os("RELAY_WORKTREE").map(PathBuf::from).filter(|p| p.is_dir()) {
        Some(worktree) => worktree.join(".relay/tool-results"),
        None => std::env::temp_dir().join("relay-tool-results"),
    }
}

/// Write a large result to a file and return what the agent sees instead: the path, the size,
/// the top-level shape and the start of the JSON. Files older than a day are removed.
fn spill_result(value: &Value, text: &str) -> Result<Value> {
    let dir = results_dir();
    std::fs::create_dir_all(&dir)?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok());
            if old.is_some_and(|age| age > Duration::from_secs(24 * 3600)) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let file = dir.join(format!("{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&file, text)?;
    let shape: Map<String, Value> = value.as_object().map(|o| o.iter().map(|(k, v)| {
        let described = match v {
            Value::Array(a) => json!(format!("array of {}", a.len())),
            Value::Object(o) => json!(format!("object with {} keys", o.len())),
            Value::String(s) if s.len() > 200 => json!(format!("string of {} bytes", s.len())),
            other => other.clone(),
        };
        (k.clone(), described)
    }).collect()).unwrap_or_default();
    let mut cut = 20_000.min(text.len());
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    Ok(json!({
        "result_file": file,
        "result_bytes": text.len(),
        "note": format!("The result is {} KB, too large to return inline; the full JSON is in result_file. Read or search that file (e.g. with jq) for what you need.", text.len() / 1024),
        "shape": shape,
        "preview": &text[..cut],
    }))
}

pub(crate) fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
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
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 23);
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

    fn bare_project(root: &Path) -> Project {
        Project { uproject: root.join("Game.uproject"), root: root.to_path_buf(), name: "Game".into(), descriptor: json!({}) }
    }

    #[test]
    fn one_agent_drives_the_editor_at_a_time() {
        let root = tempfile::tempdir().unwrap();
        let project = bare_project(root.path());
        acquire_lock(&project, "calm-otter").unwrap();
        acquire_lock(&project, "calm-otter").expect("the holder keeps its own lock");
        let refused = acquire_lock(&project, "brisk-fox").unwrap_err().to_string();
        assert!(refused.contains("calm-otter"), "{refused}");
        assert!(release_lock(&project, "brisk-fox").is_err(), "only the holder releases");
        release_lock(&project, "calm-otter").unwrap();
        acquire_lock(&project, "brisk-fox").unwrap();

        // An abandoned lock frees itself.
        std::fs::write(lock_path(&project), json!({"holder":"brisk-fox","since":1,"last_used":1}).to_string()).unwrap();
        assert_eq!(read_lock(&project)["expired"], true);
        acquire_lock(&project, "calm-otter").unwrap();
        assert_eq!(read_lock(&project)["holder"], "calm-otter");
    }

    #[test]
    fn concurrent_acquirers_never_both_hold_the_lock() {
        for round in 0..20 {
            let root = tempfile::tempdir().unwrap();
            let project = std::sync::Arc::new(bare_project(root.path()));
            if round % 2 == 1 {
                // Half the rounds race to take over an abandoned lock rather than a free one.
                std::fs::create_dir_all(lock_path(&project).parent().unwrap()).unwrap();
                std::fs::write(lock_path(&project), json!({"holder":"gone","since":1,"last_used":1}).to_string()).unwrap();
            }
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
            let winners: Vec<String> = (0..8).map(|i| {
                let (project, barrier) = (project.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let me = format!("agent-{i}");
                    barrier.wait();
                    acquire_lock(&project, &me).ok().map(|_| me)
                })
            }).collect::<Vec<_>>().into_iter().filter_map(|t| t.join().unwrap()).collect();
            assert_eq!(winners.len(), 1, "round {round}: {winners:?}");
            assert_eq!(read_lock(&project)["holder"], winners[0].as_str());
            assert!(!lock_path(&project).with_extension("json.guard").exists(), "guard left behind");
        }
    }

    #[test]
    fn a_guard_left_by_a_dead_process_is_broken() {
        let root = tempfile::tempdir().unwrap();
        let project = bare_project(root.path());
        let guard = lock_path(&project).with_extension("json.guard");
        std::fs::create_dir_all(guard.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&guard).unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(60)).unwrap();
        acquire_lock(&project, "calm-otter").unwrap();
        assert_eq!(read_lock(&project)["holder"], "calm-otter");
    }

    #[test]
    fn the_lock_tool_works_with_no_editor() {
        assert!(!LIVE.contains(&"ue_editor_lock"));
        let root = tempfile::tempdir().unwrap();
        project(root.path(), "Game", json!({}));
        let project = bare_project(root.path());
        acquire_lock(&project, &holder_id()).unwrap();
        // SAFETY: only this test sets UE_PROJECT.
        unsafe { std::env::set_var("UE_PROJECT", root.path().join("Game.uproject")) };
        let status = call("ue_editor_lock", &json!({})).unwrap();
        assert_eq!(status["lock"]["holder"], holder_id());
        let released = call("ue_editor_lock", &json!({"action":"release"})).unwrap();
        unsafe { std::env::remove_var("UE_PROJECT") };
        assert_eq!(released["lock"], Value::Null);
    }

    #[test]
    fn an_import_can_name_a_project_in_another_checkout() {
        let game = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(game.path().join("Game")).unwrap();
        project(&game.path().join("Game"), "Shooter", json!({}));
        let wanted = game.path().join("Game/Shooter.uproject");
        assert_eq!(project_file(Some(game.path())).unwrap(), wanted, "a folder is searched");
        assert_eq!(project_file(Some(&wanted)).unwrap(), wanted);
        let art = tempfile::tempdir().unwrap();
        let error = format!("{:#}", project_file(Some(art.path())).unwrap_err());
        assert!(error.contains("pass uproject") && error.contains("UE_PROJECT"), "{error}");
    }

    #[test]
    fn a_long_result_line_is_found_in_the_untrimmed_output() {
        let rows: Vec<Value> = (0..2000).map(|i| json!({"row": i, "text": "x".repeat(40)})).collect();
        let result = json!({"rows": rows});
        let line = format!("RELAY_JSON:{result}");
        assert!(line.len() > 100_000);
        let before: Vec<String> = (0..500).map(|i| format!("LogPython: loading {i}")).collect();
        let output = format!("{}\n{line}\n[Warning] LogPython: after the result\nLogTemp: done", before.join("\n"));
        assert!(output.len() > MAX_OUTPUT);
        assert_eq!(relay_json(&output).unwrap(), result);
        // What is shown stays within budget and does not repeat the result.
        let shown = shown_output(&output);
        assert!(shown.len() <= MAX_OUTPUT);
        assert!(shown.contains(&format!("RELAY_JSON:<{} bytes>", line.len() - "RELAY_JSON:".len())), "{}", &shown[shown.len() - 200..]);
        assert!(shown.ends_with("LogTemp: done"));
        // The last result wins; a missing one is an error that shows the output.
        assert_eq!(relay_json("RELAY_JSON:{\"a\":1}\nRELAY_JSON:{\"a\":2}").unwrap(), json!({"a":2}));
        let missing = relay_json("Traceback: boom").unwrap_err().to_string();
        assert!(missing.contains("printed no result") && missing.contains("boom"), "{missing}");
    }

    #[test]
    fn oversized_results_are_written_to_a_file_with_a_preview() {
        let big = json!({"text": "row,value\n".repeat(40_000), "rows": 40_000});
        let result = tool_result(big.clone(), false);
        let summary = &result["structuredContent"];
        let file = summary["result_file"].as_str().expect("spilled to a file");
        let written: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(written, big);
        assert_eq!(summary["shape"]["rows"], 40_000);
        assert_eq!(summary["shape"]["text"], "string of 400000 bytes");
        assert!(result["content"][0]["text"].as_str().unwrap().len() < 60_000);
        let _ = std::fs::remove_file(file);
        let small = tool_result(json!({"ok": true}), false);
        assert_eq!(small["structuredContent"], json!({"ok": true}));
    }

    #[test]
    fn log_cursor_reads_only_what_was_appended_and_survives_rotation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Game.log");
        std::fs::write(&path, "LogInit: old line\n").unwrap();
        let mut cursor = LogCursor { path: path.clone(), offset: std::fs::metadata(&path).unwrap().len(), partial: Vec::new() };
        assert!(cursor.read_new(None).is_empty());
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"LogAutomation: one\nLogOther: skip\nLogAutomation: tw").unwrap();
        let re = regex::Regex::new("LogAutomation").unwrap();
        assert_eq!(cursor.read_new(Some(&re)), vec!["LogAutomation: one"], "a half-written line waits");
        file.write_all(b"o\n").unwrap();
        assert_eq!(cursor.read_new(Some(&re)), vec!["LogAutomation: two"]);
        // The editor restarted and began a new, shorter log.
        std::fs::write(&path, "LogAutomation: fresh\n").unwrap();
        assert_eq!(cursor.read_new(Some(&re)), vec!["LogAutomation: fresh"]);

        let project = bare_project(root.path());
        std::fs::create_dir_all(root.path().join("Saved/Logs")).unwrap();
        std::fs::write(project.log_path(), "before\nLogBlueprint: a\nLogBlueprint: b").unwrap();
        assert_eq!(log_since(&project, 7, Some("LogBlueprint")), vec!["LogBlueprint: a", "LogBlueprint: b"]);
        assert_eq!(log_since(&project, 10_000, None), vec!["before", "LogBlueprint: a", "LogBlueprint: b"], "a shorter file was rotated");
    }

    #[test]
    fn log_tail_reads_back_from_the_end() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Game.log");
        let mut text = String::new();
        for i in 0..100_000 {
            text.push_str(&format!("{} line {i}\n", if i % 1000 == 0 { "LogRare:" } else { "LogTemp:" }));
        }
        std::fs::write(&path, &text).unwrap();
        let last = tail_lines(&path, 3, None, MAX_LOG_SCAN).unwrap();
        assert_eq!(last.kept, vec!["LogTemp: line 99997", "LogTemp: line 99998", "LogTemp: line 99999"]);
        assert!(last.from > 0, "a 3-line tail must not read the whole file");
        let re = regex::Regex::new("LogRare").unwrap();
        let rare = tail_lines(&path, 50, Some(&re), MAX_LOG_SCAN).unwrap();
        assert_eq!(rare.kept.len(), 50);
        assert_eq!(rare.kept[49], "LogRare: line 99000");
        let everything = tail_lines(&path, 5000, Some(&re), MAX_LOG_SCAN).unwrap();
        assert_eq!((everything.from, everything.matching), (0, 100));
        let bounded = tail_lines(&path, 5000, Some(&re), 1 << 20).unwrap();
        assert!(bounded.from > 0 && bounded.matching < 100);
    }

    #[test]
    fn replies_end_at_content_length_on_a_kept_alive_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for chunked in [false, true] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = vec![0u8; 8192];
                let _ = stream.read(&mut buffer).unwrap();
                let body = r#"{"ReturnValue":true}"#;
                let reply = if chunked {
                    format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len())
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}", body.len())
                };
                stream.write_all(reply.as_bytes()).unwrap();
                // Keep the connection open until the client hangs up.
                while stream.read(&mut buffer).map(|n| n > 0).unwrap_or(false) {}
            }
        });
        let url = format!("http://127.0.0.1:{port}");
        for _ in 0..2 {
            let started = std::time::Instant::now();
            let result = remote_at(&url, "GET", "/remote/info", None, Duration::from_secs(10)).unwrap();
            assert_eq!(result["ReturnValue"], true);
            assert!(started.elapsed() < Duration::from_secs(3), "waited for the read timeout");
        }
        server.join().unwrap();
    }

    #[test]
    fn captured_images_become_image_content_and_the_folder_is_removed() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("cap");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot_front.png"), b"\x89PNG fake").unwrap();
        let value = with_images(Ok(json!({"files":[
            {"view":"front","file":dir.join("shot_front.png")},
            {"view":"right","file":dir.join("shot_right.png")}
        ]})), &dir).unwrap();
        let result = tool_result(value, false);
        let content = result["content"].as_array().unwrap();
        let images: Vec<&Value> = content.iter().filter(|c| c["type"] == "image").collect();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0]["mimeType"], "image/png");
        assert_eq!(result["structuredContent"]["images"]["shown"], json!(["front"]));
        assert!(result["structuredContent"].get("_images").is_none());
        assert!(!dir.exists(), "capture folder left behind");
    }

    #[test]
    fn scripts_carry_their_arguments_and_the_shared_helpers() {
        let code = script(PY_ANIM_INSPECT, &json!({"mesh":"/Game/It's"}));
        assert!(code.starts_with("ARGS_JSON = \"{\\\"mesh\\\":\\\"/Game/It's\\\"}\"\n"), "{}", &code[..80]);
        assert!(code.contains("def body_frame(skel):"));
        assert!(code.trim_end().ends_with("})"));
    }

    /// The pose maths is Python that runs inside the editor; `tests/run_inspect.py` runs it
    /// against a stand-in `unreal` module. Skipped only where no python3 exists.
    #[test]
    fn animation_checks_hold_against_a_stand_in_editor() {
        let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/unreal_py/tests/run_inspect.py");
        let Ok(output) = Command::new("python3").arg(&runner).output() else {
            eprintln!("python3 not found; skipping");
            return;
        };
        assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn profiles_summarize_timing_columns_and_skip_metadata_rows() {
        let csv = "FrameTime,GameThreadTime,RenderThreadTime,GPUTime,Other\n\
                   16.0,10.0,8.0,15.0,1\n\
                   18.0,12.0,8.0,17.0,1\n\
                   40.0,30.0,9.0,20.0,1\n\
                   [HasHeaderRowAtEnd],1\n\
                   FrameTime,GameThreadTime,RenderThreadTime,GPUTime,Other\n";
        let s = summarize_csv(csv);
        assert_eq!(s["frames"], 3);
        assert_eq!(s["stats"]["FrameTime"]["max_ms"], 40.0);
        assert_eq!(s["stats"]["GameThreadTime"]["avg_ms"], 17.33);
        assert!(s["stats"].get("Other").is_none());
        assert_eq!(s["worst_frames"][0]["ms"], 40.0);
    }

    #[test]
    fn in_editor_test_results_are_read_from_the_log() {
        let lines = vec![
            "LogAutomationController: Display: Test Completed. Result={Success} Name={Adds} Path={Project.Inventory.Adds}".to_string(),
            "LogAutomationController: Error: Test Completed. Result={Fail} Name={Stacks} Path={Project.Inventory.Stacks}".to_string(),
            "LogAutomationController: Display: ...Automation Test Queue Empty 2 tests performed.".to_string(),
        ];
        let v = test_lines(&lines);
        assert_eq!(v["succeeded"], 1);
        assert_eq!(v["failed"], 1);
        assert_eq!(v["tests"][1]["test"], "Project.Inventory.Stacks");
    }

    #[test]
    fn crashes_are_read_newest_first() {
        let root = tempfile::tempdir().unwrap();
        let project = bare_project(root.path());
        let old = root.path().join("Saved/Crashes/UECC-old");
        let new = root.path().join("Saved/Crashes/UECC-new");
        std::fs::create_dir_all(&old).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("CrashContext.runtime-xml"), "<Root><ErrorMessage>Assertion failed: Health &gt; 0</ErrorMessage><CallStack>MyGame!AHero::Tick()\nUnrealEditor-Engine!AActor::Tick()</CallStack></Root>").unwrap();
        std::fs::write(new.join("Game.log"), "line 1\nFatal error\n").unwrap();
        let c = crash(&project, 0).unwrap();
        assert_eq!(c["error"], "Assertion failed: Health > 0");
        assert!(c["callstack"].as_str().unwrap().starts_with("MyGame!AHero::Tick()"));
        assert!(c["log_tail"].as_str().unwrap().contains("Fatal error"));
        assert_eq!(c["of"], 2);
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
