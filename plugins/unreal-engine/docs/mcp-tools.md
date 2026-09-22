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
Whether *RemoteControl*, *PythonScriptPlugin* and *EditorScriptingUtilities* are enabled in the
`.uproject`, whether the editor answers, whether remote Python runs, and a list of `advice` steps.
`fix: true` adds the missing plugins to the `.uproject` (tab-indented, other fields untouched).

### `ue_build` — `{ target?, platform?, configuration?, timeout_s? }`
Runs `Engine/Build/BatchFiles/<Linux|Mac>/Build.sh` (or `Build.bat`) with
`<target> <platform> <configuration> -Project=<uproject> -WaitMutex -FromMsBuild`.
Defaults: the `*Editor` target, the host platform, `Development`, one hour. Returns `success`,
`exit_code`, extracted `errors`, the last 150 lines and the exact command.

### `ue_log` — `{ lines?, filter?, file? }`
The tail of `Saved/Logs/<Project>.log` (or another file in `Saved/Logs`), optionally filtered by a
regex first. Typical filters: `Error|Warning`, `LogBlueprint`, `LogPython`, `LogTemp`.

## Live editor tools

All need the editor running with the Remote Control web server (see [setup](setup.md)).

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
