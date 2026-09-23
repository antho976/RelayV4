# The `unreal` MCP server

Served by `relay unreal-mcp` over stdio. Relay registers it for every agent of a project the
Unreal Engine plugin is on for. It works on the `.uproject` found in the agent's checkout (searched
up to three folders deep, skipping `Binaries`, `Intermediate`, `Saved`, `DerivedDataCache`).

Every tool returns JSON. A failure comes back as a tool error with a message that says what to do.

## Offline tools

### `ue_project_info` — `{}`
Project name and paths, `EngineAssociation`, the resolved engine folder (or why it was not found),
the `.uproject` `Modules`, `Plugins` and `TargetPlatforms`, build targets (`Source/*.Target.cs`),
C++ modules (`*.Build.cs`), project plugins (`Plugins/**/*.uplugin`), config files, maps under
`Content/` as `/Game/...` paths, file counts by extension, whether the project is Blueprint-only,
and the log path. **Call it first in every task.**

### `ue_setup_check` — `{ fix?: bool }`
With `fix`, also writes `Config/DefaultRemoteControl.ini` (web server at start-up, remote Python,
console commands, remote function calls), checking each key against the engine's
`RemoteControlSettings.h` when the engine source is present.

Whether *RemoteControl*, *PythonScriptPlugin* and *EditorScriptingUtilities* are enabled in the
`.uproject`, whether the editor answers, whether remote Python runs, and a list of `advice` steps.
`fix: true` adds the missing plugins to the `.uproject` (tab-indented, other fields untouched).

### `ue_build` — `{ target?, platform?, configuration?, timeout_s?, restart_editor?, allow_editor_open?, keep_crash_reporters? }`
`restart_editor` saves and quits the editor, builds, relaunches it and waits until it answers
(the C++ loop on Linux, which has no Live Coding). Before building it stops leftover
CrashReportClient processes and refuses while the editor runs; afterwards it warns when
`UnrealEditor.modules` names a numbered hot-reload module. The engine is found without `UE_ROOT`
from a running editor, the project log, a surrounding source tree, `Install.ini` or common folders.

Runs `Engine/Build/BatchFiles/<Linux|Mac>/Build.sh` (or `Build.bat`) with
`<target> <platform> <configuration> -Project=<uproject> -WaitMutex -FromMsBuild`.
Defaults: the `*Editor` target, the host platform, `Development`, one hour. Returns `success`,
`exit_code`, extracted `errors`, the last 150 lines and the exact command.

### `ue_log` — `{ lines?, filter?, file? }`
The tail of `Saved/Logs/<Project>.log` (or another file in `Saved/Logs`), optionally filtered by a
regex first. Typical filters: `Error|Warning`, `LogBlueprint`, `LogPython`, `LogTemp`.

## Live editor tools

All need the editor running with the Remote Control web server (see [setup](setup.md)).

**Same project.** Before acting, every live tool asks the editor which `.uproject` it has open and
refuses if it is not the one in the agent's checkout (for example, the agent is in a git worktree
while the editor is on the main checkout). `UE_ALLOW_PROJECT_MISMATCH=1` overrides this.

**One driver.** Live tools that change the editor (`ue_python`, `ue_call`, `ue_property` writes,
`ue_console`, `ue_screenshot`, `ue_anim_preview`) take a lock in `Saved/Relay/editor-lock.json`,
held by the agent's session name. Another agent gets a clear refusal naming the holder. The lock
frees itself after 15 idle minutes.

### `ue_editor_status` — `{}`
`reachable`, the endpoint and the server's route list.

### `ue_python` — `{ code, timeout_s? }`
Runs the code as a file through `PythonScriptLibrary.ExecutePythonCommandEx`. Returns `ok`, the
printed `output` (warnings and errors are prefixed with their type) and the command `result`.
A Python exception is a tool error carrying the traceback.

### `ue_call` — `{ object_path, function, parameters?, transaction? }`
`PUT /remote/object/call`. For a static `BlueprintCallable` function, call it on the class default
object: `/Script/<Module>.Default__<Class>`, e.g.
`/Script/EditorScriptingUtilities.Default__EditorAssetLibrary` with `ListAssets`.

### `ue_property` — `{ object_path, property, value? }`
`PUT /remote/object/property`: a read, or with `value` an undoable write.

### `ue_search_assets` — `{ query?, class_names?, package_paths?, limit? }`
Asset Registry search by name fragment, class short name (`StaticMesh`, `Blueprint`, `Material`,
`MaterialInstanceConstant`, `World`, `SoundWave`, `AnimSequence`, …) and folders (default `/Game`).
Returns name, class, package and object path.

### `ue_level_actors` — `{ class_filter?, name_filter?, selected_only?, limit? }`
Actors of the open level with label, class, object path, outliner folder and location.

### `ue_console` — `{ command }`
Runs a console command in the editor world. Output lands in the log; read it with `ue_log`.

### `ue_editor_launch` — `{ timeout_s?, extra_args? }` and `ue_editor_quit` — `{ save? }`
Launch waits for the Remote Control port to be free, starts the editor with `-RCWebControlEnable`
(and with "Use Less CPU when in Background" overridden off unless `keep_background_throttle`),
and waits until Remote Control answers, failing early on a bind error in the new log. Quit saves
dirty packages (unless `save: false`), asks the editor to quit, sends a terminate signal if it is
still running 20 s later, waits for the port to be released, and reports the timing of each step.

When the editor is unreachable, `ue_editor_status` says which case it is: editor running but no
server, a failed bind (run `WebControl.StopServer` then `WebControl.StartServer`), or the port
still held with no editor.

### `ue_editor_status` also reports
`editor_project`, `this_checkout`, `same_project` and the current `editor_lock`.

### `ue_editor_lock` — `{ action?: "status" | "release" }`
Who holds the editor lock; `release` gives up your own.

## Playing, testing and profiling

### `ue_play` — `{ mode?, seconds?, checkpoints?, screenshots?, width?, height?, console?, probe?, log_filter?, stop?, stop_existing? }`
Starts Play In Editor (`mode: "simulate"` for Simulate), waits until it runs, runs `console`
commands, and at each checkpoint (seconds after start; default the end) takes an in-game
screenshot (`HighResShot`) and runs `probe` — Python with `unreal` and `world` (the game world) in
scope, whose printed output is returned. Then it stops the session and returns the screenshots as
images, the probe output, and the log lines written while playing (default filter: errors,
warnings, ensures, "Accessed None", Blueprint user messages). On engine versions whose Python has
no play-in-editor request it falls back to Simulate and says so.

### `ue_run_tests` — `{ filter, in_editor?, timeout_s? }`
Headless by default: runs `UnrealEditor-Cmd` on the project with
`-ExecCmds="Automation RunTests <filter>;Quit" -nullrhi -unattended` and a report folder, and
returns each test's state and error/warning messages. `in_editor: true` runs the tests in the open
editor and reads the results from the log.

### `ue_profile` — `{ seconds?, warmup?, play?, console? }`
Starts Play In Editor (unless `play: false`), waits `warmup` seconds, records `csvprofile` for
`seconds`, stops, and summarises the newest CSV in `Saved/Profiling/CSV`: frames, average fps, and
average / 95th percentile / worst for FrameTime, GameThreadTime, RenderThreadTime, GPUTime (and
RHIThreadTime when present), plus the five worst frames.

### `ue_crash` — `{ index? }`
The newest (or `index`-th newest) folder in `Saved/Crashes`: error message, crash type, call stack,
engine version, build configuration and the tail of its log. Works without the editor.

## Assets and data

### `ue_blueprint_info` — `{ paths? | folder?, compile?, limit? }`
Parent class and Asset Registry tags, variables (with default values) and functions/events that
the Blueprint adds over its native parent, components, and event-graph nodes where the engine
exposes them to Python (the `notes` say what could not be read). `compile: true` compiles and adds
the compiler's log lines.

### `ue_asset_audit` — `{ path?, checks?, limit? }`
Textures (non-power-of-two, over 4096, normal maps without Normalmap compression, masks in sRGB,
Never Stream on large textures, world textures without mips), static meshes (no simple collision,
over 50k vertices with one LOD and no Nanite), references to missing assets, and redirectors. Each
finding has a fix. Loads assets, bounded by `limit` per class.

### `ue_asset_refs` — `{ path, depth? }`
Dependencies and referencers, level by level.

### `ue_data_table` — `{ action, path, file?, format? }`
`export` returns the Data Table as CSV (default) or JSON and writes it to `file` in the checkout;
`import` fills the table from `file` and saves it (the engine's error for a bad row or column is
returned with the log lines).

## Seeing and measuring

### `ue_screenshot` — `{ actors?, views?, camera?, forward?, isolate?, width?, height?, fov? }`
Renders PNGs with a temporary scene capture and returns them as images the agent can see.
Frame actors from named views (`front`, `back`, `left`, `right`, `top`, `three_quarter`,
`three_quarter_left`, relative to the first actor's facing), or give an explicit `camera`
(`location`, `rotation` as `[pitch, yaw, roll]`), or omit both for the editor viewport.
`isolate` renders only the framed actors on black.

### `ue_anim_inspect` — `{ mesh, animation?, times?, samples?, track?, attachments?, partner?, contacts?, body_radius?, touch_distance? }`
Poses the skeleton at sample times directly from the animation data (no level, no ticking) and
measures, in the character's own frame (`[forward, right, up]` cm; left and right found from the
skeleton's `_l`/`_r`-style bone pairs, so any skeleton and mesh orientation works):
- where tracked bones, sockets and item points are, and which side they are on;
- for each attached item (any static or skeletal mesh on any bone or socket, with an optional
  offset): its side, long axis and ends, grip distances to hands or palm sockets, and clearance of
  its ends to the character's own body;
- for a `partner` character: clearance of hands, feet, head and items to the partner's body;
- `contacts` that must touch or stay apart, optionally inside a time window;
- feet below the reference ground.

Returns `problems`, `passed`, `closest_approach` and per-sample details. It reads the animation
asset, not the Animation Blueprint, so runtime IK is not included.

### `ue_anim_preview` — `{ mesh, animation?, times?, samples?, attachments?, partner?, views?, location?, isolate?, settle_ms?, width?, height? }`
Spawns temporary `RelayPreview` actors (character, attached items, partner), poses them at each
time (up to 8), confirms the editor applied the pose, and returns images from the chosen views
(default `front` and `right`, isolated). The actors are removed afterwards; the level stays marked
modified, so do not save it for this. If poses lag, the editor is throttled in the background:
see [setup](setup.md).
