//! `relay blender-mcp` — the Blender plugin's MCP server (D161).
//!
//! Every tool runs Blender in background mode (`blender -b`) on a `.blend` in the agent's
//! checkout: no window, no running Blender, nothing to connect. Each call is one Blender process
//! with a Python script from `blender_py/`, arguments in a JSON file, and one `RELAY_JSON:` line
//! back. Renders come back as images the agent can see. `blender_to_unreal` hands the export to
//! the Unreal plugin's editor bridge and measures what arrived.
//!
//! Environment: `BLENDER_BIN` names the Blender executable when it is not on `PATH`.

use crate::unreal::{rpc_error, tool, tool_result};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const FALLBACK_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: &[&str] = &["2026-07-28", "2025-11-25", FALLBACK_PROTOCOL];
/// Folders a file listing never enters.
const SKIP_DIRS: [&str; 9] = [".git", "node_modules", ".relay", "Binaries", "Intermediate", "Saved", "DerivedDataCache", "target", ".venv"];
const ASSET_EXTENSIONS: [&str; 7] = ["blend", "fbx", "obj", "glb", "gltf", "abc", "usd"];

const PY_COMMON: &str = include_str!("blender_py/common.py");
const PY_INFO: &str = include_str!("blender_py/info.py");
const PY_RUN: &str = include_str!("blender_py/run.py");
const PY_RENDER: &str = include_str!("blender_py/render.py");
const PY_RIG_CHECK: &str = include_str!("blender_py/rig_check.py");
const PY_ANIM_INSPECT: &str = include_str!("blender_py/anim_inspect.py");
const PY_EXPORT: &str = include_str!("blender_py/export.py");

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
                "serverInfo": {"name":"blender","title":"Blender (Relay plugin)","version":env!("CARGO_PKG_VERSION")},
                "instructions": "Blender in background mode on .blend files in this checkout; no Blender window is needed. Start with blender_info (no file lists the art files). Look with blender_render, check rigs with blender_rig_check, measure animations with blender_anim_inspect, export with blender_export, and send to a running Unreal editor with blender_to_unreal. Load the blender-fundamentals skill first."
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

fn file_arg() -> Value {
    json!({"type":"string","description":".blend file, relative to the checkout root"})
}

fn attachments_schema() -> Value {
    json!({"type":"array","items":{"type":"object","properties":{
        "object":{"type":"string","description":"An object already attached to the rig (bone parent or Child Of constraint)"},
        "name":{"type":"string"},
        "grips":{"type":"array","items":{"type":"object","properties":{
            "point":{"type":"string","description":"obj:<Empty> (a socket), item:<name>:<end_a|end_b|center|origin>, or a bone"},
            "bone":{"type":"string","description":"Bone, or bone:tail for the bone's tail (a hand bone's tail is usually the palm end)"},
            "tolerance":{"type":"number"}}}}
    },"required":["object"]}})
}

fn tools() -> Vec<Value> {
    vec![
        tool("blender_info",
            "What a .blend holds: units and frame rate, every object with type, parent and parent bone, world location and dimensions in cm, unapplied scale/rotation, modifiers; meshes (vertices, triangles, UV maps, vertex groups, materials, shape keys, Unreal collision/socket naming); armatures (bones, deform bones, roots, left/right pairs, current action); actions with frame ranges; images. Without file, lists the art files (.blend, .fbx, .obj, .glb, .gltf, .abc, .usd) in the checkout.",
            json!({"file": file_arg()}), &[], true),
        tool("blender_python",
            "Run Python (bpy) in Blender on a file (or an empty scene without file). save=true saves the file, save_as writes a new one (relative path). Print what you need to see. addons enables bundled add-ons first (e.g. rigify).",
            json!({
                "file": file_arg(),
                "code":{"type":"string"},
                "save":{"type":"boolean"},
                "save_as":{"type":"string"},
                "addons":{"type":"array","items":{"type":"string"}},
                "timeout_s":{"type":"integer","minimum":10,"maximum":3600,"description":"Default 300"}
            }), &["code"], false),
        tool("blender_render",
            "See a model or a pose: renders objects (an armature brings its skinned meshes) from named views relative to the character's facing - front, back, left, right, top, three_quarter, three_quarter_left - at chosen frames of an action. Returns images. Workbench (default, fast, shows shape and cavities), eevee or cycles (CPU, no denoiser). Other meshes are hidden unless isolate=false. The file is not changed.",
            json!({
                "file": file_arg(),
                "objects":{"type":"array","items":{"type":"string"}},
                "action":{"type":"string"},
                "frames":{"type":"array","items":{"type":"integer"}},
                "views":{"type":"array","items":{"type":"string"}},
                "engine":{"type":"string","enum":["workbench","eevee","cycles"]},
                "color":{"type":"string","enum":["MATERIAL","OBJECT","SINGLE","RANDOM","TEXTURE","VERTEX"],"description":"Workbench colouring, default MATERIAL"},
                "isolate":{"type":"boolean"},
                "width":{"type":"integer","minimum":64,"maximum":1920},"height":{"type":"integer","minimum":64,"maximum":1080},
                "timeout_s":{"type":"integer","minimum":10,"maximum":3600}
            }), &["file"], true),
        tool("blender_rig_check",
            "Check a rig and its skin before export: applied transforms, a single root bone, deform bones, leftover _end bones, left/right symmetry and naming, facing (-Y is Blender's front), skinned meshes with unweighted vertices, too many influences, weights on non-deform bones, stray vertex groups, missing UVs, non-manifold or inside-out geometry, and height in cm. Returns problems (must fix), warnings and passed.",
            json!({"file": file_arg(), "armature":{"type":"string"}, "meshes":{"type":"array","items":{"type":"string"}}}), &["file"], true),
        tool("blender_anim_inspect",
            "Measure an action before export, like ue_anim_inspect: at sampled frames, where tracked bones and objects are in the character's frame ([forward, right, up] cm, sides from the rig's .L/.R pairs); attached items (objects parented to a bone) with grip distances and clearance to the body; a partner armature's clearance; expected contacts (touch or apart, with a frame window); feet below the ground. Returns problems and passed.",
            json!({
                "file": file_arg(),
                "armature":{"type":"string"},
                "action":{"type":"string"},
                "frames":{"type":"array","items":{"type":"integer"}},
                "samples":{"type":"integer","minimum":2,"maximum":60},
                "track":{"type":"array","items":{"type":"string"},"description":"bone, bone:tail, obj:<name>, item:<name>:<end_a|end_b|center|origin>, partner:<bone>"},
                "attachments": attachments_schema(),
                "partner":{"type":"object","properties":{"armature":{"type":"string"},"action":{"type":"string"}},"required":["armature"]},
                "contacts":{"type":"array","items":{"type":"object","properties":{
                    "a":{"type":"string"},"b":{"type":"string"},"expect":{"type":"string","enum":["touch","apart"]},
                    "distance":{"type":"number"},"window":{"type":"array","items":{"type":"integer"}}},"required":["a","b"]}},
                "body_radius":{"type":"number","description":"cm, default 8"},
                "touch_distance":{"type":"number","description":"cm, default 5"}
            }), &["file"], true),
        tool("blender_export",
            "Export FBX for Unreal. kind static (a mesh with its SOCKET_ empties, UCX_/UBX_/USP_ collision and _LODn children), skeletal (an armature and the meshes skinned to it; deform bones only, no leaf bones; animations=true bakes the action) or animation (the armature's action only). Settings are Unreal's usual ones; fbx_options overrides any exporter option. Reports the size in cm to compare after import.",
            json!({
                "file": file_arg(),
                "objects":{"type":"array","items":{"type":"string"}},
                "path":{"type":"string","description":"Output .fbx, relative to the checkout root"},
                "kind":{"type":"string","enum":["auto","static","skeletal","animation"]},
                "action":{"type":"string"},
                "animations":{"type":"boolean"},
                "all_actions":{"type":"boolean","description":"One take per action (fake-user actions included)"},
                "fbx_options":{"type":"object"}
            }), &["file","path"], false),
        tool("blender_to_unreal",
            "Export from Blender and import into the running Unreal editor in one step, then measure the result: imported assets, their size against the Blender size (a 100x difference means a unit problem), a skeletal mesh's root bone scale and which way it faces, and which hand ends up on which side. Needs the Unreal plugin on and the editor open.",
            json!({
                "file": file_arg(),
                "objects":{"type":"array","items":{"type":"string"}},
                "kind":{"type":"string","enum":["static","skeletal","animation"]},
                "action":{"type":"string"},
                "animations":{"type":"boolean"},
                "destination":{"type":"string","description":"Content folder, e.g. /Game/Characters/Hero"},
                "name":{"type":"string","description":"Asset name in Unreal"},
                "skeleton":{"type":"string","description":"Existing Skeleton asset, for animations and shared rigs"},
                "fbx_path":{"type":"string","description":"Where to keep the FBX, relative to the checkout; default Saved/Relay/Exports/<name>.fbx"},
                "materials":{"type":"boolean"},
                "uproject":{"type":"string","description":"The Unreal project to import into, when it is not in this checkout: its .uproject or its folder (absolute, or relative to the checkout). Default: UE_PROJECT, then a .uproject found in this checkout"}
            }), &["file","kind","destination"], false),
    ]
}

fn call(name: &str, args: &Value) -> Result<Value> {
    let root = checkout_root()?;
    match name {
        "blender_info" => match args["file"].as_str().filter(|f| !f.is_empty()) {
            None => Ok(list_files(&root)),
            Some(file) => run(PY_INFO, args, Some(&resolve(&root, file)?), secs(args, 120)),
        },
        "blender_python" => {
            let mut a = args.clone();
            if let Some(save_as) = args["save_as"].as_str() {
                a["save_as"] = json!(resolve_new(&root, save_as, "blend")?);
            }
            let file = args["file"].as_str().filter(|f| !f.is_empty()).map(|f| resolve(&root, f)).transpose()?;
            run(PY_RUN, &a, file.as_deref(), secs(args, 300))
        }
        "blender_render" => {
            let file = resolve(&root, required(args, "file")?)?;
            let dir = std::env::temp_dir().join(format!("relay-blender-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir)?;
            let mut a = args.clone();
            a["out_dir"] = json!(dir);
            let result = run(PY_RENDER, &a, Some(&file), secs(args, 600));
            match result {
                Ok(mut value) => {
                    let files = value["files"].as_array().cloned().unwrap_or_default();
                    value["_images"] = json!(files.iter().map(|f| json!({"label": f["view"], "path": f["file"]})).collect::<Vec<_>>());
                    value["_cleanup"] = json!(dir);
                    value.as_object_mut().unwrap().remove("files");
                    Ok(value)
                }
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    Err(error)
                }
            }
        }
        "blender_rig_check" => run(PY_RIG_CHECK, args, Some(&resolve(&root, required(args, "file")?)?), secs(args, 300)),
        "blender_anim_inspect" => run(PY_ANIM_INSPECT, args, Some(&resolve(&root, required(args, "file")?)?), secs(args, 300)),
        "blender_export" => {
            let file = resolve(&root, required(args, "file")?)?;
            let mut a = args.clone();
            a["path"] = json!(resolve_new(&root, required(args, "path")?, "fbx")?);
            run(PY_EXPORT, &a, Some(&file), secs(args, 600))
        }
        "blender_to_unreal" => to_unreal(&root, args),
        other => bail!("unknown tool {other}"),
    }
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args[key].as_str().filter(|v| !v.trim().is_empty()).ok_or_else(|| anyhow!("{key} is required"))
}

fn secs(args: &Value, default: u64) -> Duration {
    Duration::from_secs(args["timeout_s"].as_u64().unwrap_or(default).clamp(10, 3600))
}

fn checkout_root() -> Result<PathBuf> {
    Ok(std::env::var_os("RELAY_WORKTREE").map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or(std::env::current_dir()?))
}

/// An existing file inside the checkout. Absolute paths are accepted when they are inside it.
fn resolve(root: &Path, file: &str) -> Result<PathBuf> {
    let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { root.join(file) };
    let real = path.canonicalize().with_context(|| format!("{} does not exist; list the art files with blender_info and no file", path.display()))?;
    let root = root.canonicalize()?;
    anyhow::ensure!(real.starts_with(&root), "{} is outside the checkout {}", real.display(), root.display());
    Ok(real)
}

/// A file to be written inside the checkout, with the extension it must have.
fn resolve_new(root: &Path, file: &str, extension: &str) -> Result<PathBuf> {
    anyhow::ensure!(!Path::new(file).is_absolute() && !file.split(['/', '\\']).any(|part| part == ".."), "{file} must be a path inside the checkout, relative to its root");
    anyhow::ensure!(Path::new(file).extension().is_some_and(|e| e.eq_ignore_ascii_case(extension)), "{file} must end in .{extension}");
    Ok(root.join(file))
}

fn list_files(root: &Path) -> Value {
    fn walk(dir: &Path, root: &Path, depth: usize, out: &mut Vec<Value>) {
        if depth > 8 || out.len() >= 500 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    walk(&path, root, depth + 1, out);
                }
            } else if path.extension().is_some_and(|e| ASSET_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str())) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                out.push(json!({"file": path.strip_prefix(root).unwrap_or(&path), "bytes": size}));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, 0, &mut files);
    let blender = find_blender().map(|p| p.display().to_string()).map_err(|e| e.to_string());
    json!({"root": root, "files": files, "blender": blender.as_ref().ok(), "blender_problem": blender.err()})
}

fn find_blender() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("BLENDER_BIN").map(PathBuf::from) {
        anyhow::ensure!(path.is_file(), "BLENDER_BIN={} is not a file", path.display());
        return Ok(path);
    }
    let exe = if cfg!(target_os = "windows") { "blender.exe" } else { "blender" };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    let mut candidates = vec![
        PathBuf::from("/Applications/Blender.app/Contents/MacOS/Blender"),
        PathBuf::from("/snap/bin/blender"),
        PathBuf::from("/usr/local/bin/blender"),
    ];
    // Windows installs one folder per version; take the newest.
    if let Ok(entries) = std::fs::read_dir("C:/Program Files/Blender Foundation") {
        let mut versions: Vec<PathBuf> = entries.flatten().map(|e| e.path().join("blender.exe")).filter(|p| p.is_file()).collect();
        versions.sort();
        candidates.extend(versions.into_iter().rev());
    }
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| anyhow!("Blender not found: install it, put `blender` on PATH, or set BLENDER_BIN to the executable"))
}

/// One background Blender run: the shared helpers and the tool's script, the arguments as a
/// JSON file, the last `RELAY_JSON:` line as the result. A Python error comes back with its
/// traceback.
fn run(body: &str, args: &Value, file: Option<&Path>, timeout: Duration) -> Result<Value> {
    let blender = find_blender()?;
    let dir = std::env::temp_dir().join(format!("relay-blender-job-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let script = dir.join("job.py");
    let args_file = dir.join("args.json");
    std::fs::write(&script, format!("{PY_COMMON}\n{body}"))?;
    std::fs::write(&args_file, args.to_string())?;
    let mut command = Command::new(&blender);
    command.arg("-b");
    if let Some(file) = file {
        command.arg(file);
    }
    command
        .args(["--factory-startup", "--python-exit-code", "1", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&args_file);
    if let Some(file) = file.and_then(Path::parent) {
        command.current_dir(file);
    }
    let output = relay_core::proc::output_with_timeout(&mut command, timeout);
    let _ = std::fs::remove_dir_all(&dir);
    let output = output.with_context(|| format!("running {}", blender.display()))?
        .ok_or_else(|| anyhow!("Blender did not finish within {} s and was stopped", timeout.as_secs()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The agent's script prints between markers; Blender's own start-up lines stay out.
    let printed: Vec<&str> = stdout.lines()
        .skip_while(|l| *l != "RELAY_OUT_BEGIN").skip(1)
        .take_while(|l| *l != "RELAY_OUT_END")
        .collect();
    let result = stdout.lines().rev().find_map(|l| l.strip_prefix("RELAY_JSON:"));
    if !output.status.success() || (result.is_none() && body != PY_RUN) {
        bail!("Blender failed:\n{}", failure_text(&stdout, &stderr));
    }
    let mut value = match result {
        Some(json_text) => serde_json::from_str(json_text)?,
        None => json!({}),
    };
    if body == PY_RUN {
        // The agent's own script: its printed output is the result.
        value["output"] = json!(tail(&printed.join("\n"), 300));
        value["saved"] = stdout.lines().find_map(|l| l.strip_prefix("RELAY_SAVED:")).and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
    }
    Ok(value)
}

/// The traceback and error lines of a failed run, without Blender's start-up noise.
fn failure_text(stdout: &str, stderr: &str) -> String {
    let all: Vec<&str> = stdout.lines().chain(stderr.lines()).collect();
    let start = all.iter().position(|l| l.starts_with("Traceback")).unwrap_or(all.len().saturating_sub(40));
    let lines: Vec<&str> = all[start..].iter().copied().filter(|l| !l.starts_with("EGL Error") && !l.trim().is_empty()).collect();
    tail(&lines.join("\n"), 60)
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn to_unreal(root: &Path, args: &Value) -> Result<Value> {
    let file = resolve(root, required(args, "file")?)?;
    let kind = required(args, "kind")?;
    // Find the Unreal project before the (slow) export. It may be a separate checkout.
    let explicit = args["uproject"].as_str().filter(|p| !p.trim().is_empty())
        .map(|p| if Path::new(p).is_absolute() { PathBuf::from(p) } else { root.join(p) });
    let uproject = crate::unreal::project_file(explicit.as_deref())?;
    let stem = args["name"].as_str().map(str::to_string)
        .or_else(|| args["objects"][0].as_str().map(str::to_string))
        .unwrap_or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Export".into()));
    let fbx_rel = args["fbx_path"].as_str().map(str::to_string).unwrap_or_else(|| format!("Saved/Relay/Exports/{stem}.fbx"));
    let fbx = resolve_new(root, &fbx_rel, "fbx")?;
    let exported = run(PY_EXPORT, &json!({
        "objects": args["objects"], "path": fbx, "kind": kind, "action": args["action"],
        "animations": args["animations"].as_bool().unwrap_or(kind == "animation"),
    }), Some(&file), Duration::from_secs(600))?;
    let imported = crate::unreal::import_fbx(&fbx, json!({
        "kind": kind, "destination": required(args, "destination")?, "name": args["name"],
        "skeleton": args["skeleton"], "animations": args["animations"], "materials": args["materials"],
    }), Some(&uproject))?;
    let checks = compare(&exported, &imported);
    Ok(json!({"fbx": fbx, "exported": exported, "imported": imported["imported"], "import_log": imported["import_log"],
        "problems": checks, "passed": checks.is_empty()}))
}

/// What should survive the trip: height (Z is up on both sides), a root bone scale of 1, and the
/// character's facing. Sides are worked out from bone names on both sides, so a mirrored rig
/// would still look right-handed; the facing is what changes. Blender's (x, y, z) arrives in
/// Unreal as (x, -y, z) with the export's default axes.
fn compare(exported: &Value, imported: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let blender_height = exported["size_cm"][2].as_f64();
    let expected_forward = exported["forward_world"].as_array()
        .and_then(|f| Some([f.first()?.as_f64()?, -f.get(1)?.as_f64()?, f.get(2)?.as_f64()?]));
    for asset in imported["imported"].as_array().cloned().unwrap_or_default() {
        let path = asset["path"].as_str().unwrap_or("?");
        if let (Some(b), Some(u)) = (blender_height, asset["size_cm"][2].as_f64()) {
            if b > 0.1 {
                let ratio = u / b;
                if !(0.9..=1.1).contains(&ratio) {
                    problems.push(format!(
                        "{path} is {u:.1} cm tall in Unreal but {b:.1} cm in Blender (x{ratio:.3}); {}",
                        if (80.0..120.0).contains(&ratio) || (0.008..0.012).contains(&ratio) {
                            "a unit mismatch: check the scene's unit scale and the export's apply_scale_options"
                        } else {
                            "check the object's scale and the import's scale settings"
                        }
                    ));
                }
            }
        }
        if let Some(scale) = asset["root_bone_scale"].as_array() {
            if scale.iter().filter_map(Value::as_f64).any(|s| (s - 1.0).abs() > 0.01) {
                problems.push(format!("{path}: root bone scale is {scale:?}, not 1; animations and attachments will be scaled. Apply scale in Blender and export with apply_scale_options FBX_SCALE_ALL"));
            }
        }
        let actual = asset["forward_axis_in_mesh_space"].as_array()
            .and_then(|f| Some([f.first()?.as_f64()?, f.get(1)?.as_f64()?, f.get(2)?.as_f64()?]));
        if let (Some(e), Some(a)) = (expected_forward, actual) {
            let dot = e[0] * a[0] + e[1] * a[1] + e[2] * a[2];
            if dot < 0.9 {
                problems.push(format!(
                    "{path}: the character faces {a:?} in Unreal's mesh space, expected {e:?} from Blender; {}",
                    if dot < -0.9 { "it is mirrored or turned around - check negative scale, swapped .L/.R names and the export axes" } else { "it is rotated - apply rotation in Blender and check the export axes" }
                ));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_in_the_manifest_is_served() {
        let manifest: Value = serde_json::from_str(include_str!("../../../plugins/blender/plugin.json")).unwrap();
        let declared: Vec<&str> = manifest["mcp_servers"][0]["tools"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        let served = tools();
        let served: Vec<&str> = served.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(declared, served);
    }

    #[test]
    fn paths_stay_inside_the_checkout() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("hero.blend"), "").unwrap();
        assert!(resolve(root.path(), "hero.blend").is_ok());
        assert!(resolve(root.path(), "/etc/hostname").is_err());
        assert!(resolve_new(root.path(), "../out.fbx", "fbx").is_err());
        assert!(resolve_new(root.path(), "Exports/out.obj", "fbx").is_err());
        assert!(resolve_new(root.path(), "Exports/out.fbx", "fbx").is_ok());
    }

    #[test]
    fn a_round_trip_flags_units_root_scale_and_facing() {
        let exported = json!({"size_cm": [60.0, 20.0, 178.0], "forward_world": [0.0, -1.0, 0.0]});
        let good = json!({"imported": [{"path": "/Game/Hero", "size_cm": [60.0, 20.0, 178.2], "root_bone_scale": [1.0, 1.0, 1.0], "forward_axis_in_mesh_space": [0.0, 1.0, 0.0]}]});
        assert!(compare(&exported, &good).is_empty(), "{:?}", compare(&exported, &good));
        let bad = json!({"imported": [{"path": "/Game/Hero", "size_cm": [6000.0, 2000.0, 17800.0], "root_bone_scale": [100.0, 100.0, 100.0], "forward_axis_in_mesh_space": [0.0, -1.0, 0.0]}]});
        let problems = compare(&exported, &bad);
        assert!(problems[0].contains("unit mismatch"), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("root bone scale")));
        assert!(problems.iter().any(|p| p.contains("mirrored or turned around")));
        let turned = json!({"imported": [{"path": "/Game/Hero", "size_cm": [60.0, 20.0, 178.0], "forward_axis_in_mesh_space": [1.0, 0.0, 0.0]}]});
        assert!(compare(&exported, &turned)[0].contains("rotated"));
    }

    /// Runs every Blender script against a fixture built by `blender_py/tests/make_fixture.py`,
    /// with real Blender. Skipped where Blender is not installed (CI runners do not carry it).
    #[test]
    fn the_blender_tools_work_on_a_real_rig() {
        let Ok(blender) = find_blender() else {
            eprintln!("Blender not installed; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture.blend");
        let maker = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/blender_py/tests/make_fixture.py");
        let status = Command::new(&blender).args(["-b", "--factory-startup", "--python"]).arg(&maker).arg("--").arg(&fixture)
            .output().unwrap();
        assert!(fixture.is_file(), "fixture not built: {}", String::from_utf8_lossy(&status.stdout));
        let t = Duration::from_secs(300);

        let info = run(PY_INFO, &json!({}), Some(&fixture), t).unwrap();
        assert!(info["objects"].as_array().unwrap().iter().any(|o| o["name"] == "Hero" && o["armature"]["left_right_pairs"] == 6));

        let rig = run(PY_RIG_CHECK, &json!({"armature": "Hero"}), Some(&fixture), t).unwrap();
        assert_eq!(rig["passed"], true, "{rig}");
        assert_eq!(rig["faces"], "-Y (Blender front)");
        assert_eq!(rig["height_cm"], 178.0);

        let anim = run(PY_ANIM_INSPECT, &json!({
            "armature": "Hero", "action": "Swing", "samples": 3, "track": ["hand.R", "hand.L"],
            "attachments": [{"object": "Sword", "grips": [{"point": "obj:Grip", "bone": "hand.R:tail"}]}]
        }), Some(&fixture), t).unwrap();
        assert_eq!(anim["attachments"]["Sword"]["side"], "right", "{anim}");
        assert_eq!(anim["attachments"]["Sword"]["held_by"], "hand.R");
        assert_eq!(anim["samples"][0]["points"]["hand.L"]["side"], "left");
        assert_eq!(anim["samples"][0]["checks"][0]["distance"], 0.0);

        let out = dir.path().join("Hero.fbx");
        let exported = run(PY_EXPORT, &json!({"path": out, "objects": ["Hero"], "animations": true, "action": "Swing"}), Some(&fixture), t);
        match exported {
            Ok(exported) => {
                assert_eq!(exported["kind"], "skeletal");
                assert!(!exported["objects"].as_array().unwrap().iter().any(|o| o == "Sword"), "a bone-parented prop went into the character");
                assert!(out.is_file());
            }
            // Distribution builds of Blender without numpy cannot run the FBX exporter.
            Err(error) => assert!(format!("{error:#}").contains("numpy"), "{error:#}"),
        }

        let rendered = dir.path().join("renders");
        std::fs::create_dir_all(&rendered).unwrap();
        let shots = run(PY_RENDER, &json!({"out_dir": rendered, "objects": ["Hero"], "views": ["front"], "width": 96, "height": 96}), Some(&fixture), t).unwrap();
        assert!(Path::new(shots["files"][0]["file"].as_str().unwrap()).is_file());

        let script = run(PY_RUN, &json!({"code": "print(len(bpy.data.objects))"}), Some(&fixture), t).unwrap();
        assert_eq!(script["output"].as_str().unwrap().lines().last(), Some("6"));

        let broken = run(PY_RUN, &json!({"code": "raise ValueError('nope')"}), None, t).unwrap_err();
        assert!(format!("{broken:#}").contains("ValueError: nope"), "{broken:#}");
    }
}
